#!/usr/bin/env python3
from __future__ import annotations

import errno
import os
import subprocess
import tempfile
import threading
import time
import uuid
from pathlib import Path

from fod_mount import FODMount

FAIL_FAST_SECONDS = 1.0
BARRIER_TIMEOUT_SECONDS = 10.0
LEASE_TIMEOUT_SECONDS = 10.0
BARRIER_ENV = "FOD_TEST_CREATE_BEFORE_OWNERSHIP_BARRIER_DIR"


def sql_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def psql_scalar(launcher: FODMount, sql: str) -> str:
    env = os.environ.copy()
    env["PGPASSWORD"] = launcher.postgres_password
    host = env.get("FOD_PG_HOST") or env.get("POSTGRES_HOST") or "127.0.0.1"
    port = env.get("FOD_PG_PORT") or env.get("POSTGRES_PORT") or "5432"

    result = subprocess.run(
        [
            "psql",
            "-h", host,
            "-p", port,
            "-U", launcher.postgres_user,
            "-d", launcher.postgres_db,
            "-At",
            "-v", "ON_ERROR_STOP=1",
            "-c", sql,
        ],
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def ownership_counts(launcher: FODMount, name: str) -> tuple[int, int]:
    raw = psql_scalar(
        launcher,
        f"""
        SELECT
          (SELECT COUNT(*)
             FROM fod.destination_write_leases
            WHERE parent_key = 0
              AND name = {sql_quote(name)})::text || '|' ||
          (SELECT COUNT(*)
             FROM fod.file_write_leases fwl
             JOIN fod.files f ON f.id_file = fwl.file_id
            WHERE f.id_directory IS NULL
              AND f.name = {sql_quote(name)})::text
        """,
    )
    destination_count, file_count = raw.split("|", 1)
    return int(destination_count), int(file_count)


def wait_for_ownership_cleanup(launcher: FODMount, name: str) -> None:
    deadline = time.monotonic() + LEASE_TIMEOUT_SECONDS
    last = (-1, -1)

    while time.monotonic() < deadline:
        last = ownership_counts(launcher, name)
        if last == (0, 0):
            return
        time.sleep(0.05)

    raise AssertionError(
        f"ownership leak name={name} destination={last[0]} file={last[1]}"
    )


def db_file_size(launcher: FODMount, name: str) -> int:
    raw = psql_scalar(
        launcher,
        f"""
        SELECT size
          FROM fod.files
         WHERE id_directory IS NULL
           AND name = {sql_quote(name)}
         LIMIT 1
        """,
    )
    if not raw:
        raise AssertionError(f"missing database file row: {name}")
    return int(raw)


def arm_barrier(barrier_dir: Path, path: str) -> None:
    for name in ("target", "ready", "release"):
        (barrier_dir / name).unlink(missing_ok=True)
    (barrier_dir / "target").write_text(path, encoding="utf-8")


def wait_barrier_ready(barrier_dir: Path) -> None:
    deadline = time.monotonic() + BARRIER_TIMEOUT_SECONDS
    ready = barrier_dir / "ready"

    while time.monotonic() < deadline:
        if ready.exists():
            return
        time.sleep(0.01)

    raise AssertionError("create barrier was not reached")


def release_barrier(barrier_dir: Path) -> None:
    (barrier_dir / "release").write_text("release\n", encoding="utf-8")


def start_open(path: Path, flags: int) -> tuple[threading.Thread, dict[str, object]]:
    result: dict[str, object] = {"fd": None, "errno": None}

    def worker() -> None:
        try:
            result["fd"] = os.open(path, flags, 0o644)
        except OSError as exc:
            result["errno"] = exc.errno

    thread = threading.Thread(target=worker, daemon=True)
    thread.start()
    return thread, result


def finish_open(
    thread: threading.Thread,
    result: dict[str, object],
    label: str,
) -> tuple[int | None, int | None]:
    thread.join(BARRIER_TIMEOUT_SECONDS)
    if thread.is_alive():
        raise AssertionError(f"{label}: open thread did not finish")

    fd = result["fd"]
    err = result["errno"]

    return (
        int(fd) if fd is not None else None,
        int(err) if err is not None else None,
    )


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    with tempfile.TemporaryDirectory(prefix="fod-create-ownership-") as temp_dir:
        temp = Path(temp_dir)
        mount_a_dir = temp / "mount-a"
        mount_b_dir = temp / "mount-b"

        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        launcher_a.init_schema()

        fd_a = None
        fd_b = None

        try:
            barrier_dir = temp / "create-barrier"
            barrier_dir.mkdir()

            launcher_a.start(str(mount_a_dir), log_prefix="/tmp/fod-create-own-a")

            previous_barrier = os.environ.get(BARRIER_ENV)
            os.environ[BARRIER_ENV] = str(barrier_dir)
            try:
                launcher_b.start(
                    str(mount_b_dir),
                    log_prefix="/tmp/fod-create-own-b",
                )
            finally:
                if previous_barrier is None:
                    os.environ.pop(BARRIER_ENV, None)
                else:
                    os.environ[BARRIER_ENV] = previous_barrier

            file_name = f"create-write-ownership-{uuid.uuid4().hex}.bin"
            path_a = mount_a_dir / file_name
            path_b = mount_b_dir / file_name

            fd_a = os.open(
                path_a,
                os.O_CREAT | os.O_EXCL | os.O_WRONLY,
                0o644,
            )
            os.write(fd_a, b"A")

            started = time.monotonic()
            loser_errno = None
            try:
                fd_b = os.open(path_b, os.O_WRONLY | os.O_TRUNC)
            except OSError as exc:
                loser_errno = exc.errno
            elapsed = time.monotonic() - started

            if elapsed >= FAIL_FAST_SECONDS:
                raise AssertionError(
                    f"create ownership loser did not fail fast: {elapsed:.6f}s"
                )
            if loser_errno != errno.EBUSY:
                raise AssertionError(
                    "create handle did not own destination: "
                    f"loser_errno={loser_errno}, expected EBUSY"
                )

            print(
                "OK create-write-ownership "
                f"loser_errno={loser_errno} "
                f"loser_elapsed_ms={elapsed * 1000.0:.3f}"
            )

            if fd_a is not None:
                os.close(fd_a)
                fd_a = None

            wait_for_ownership_cleanup(launcher_a, file_name)

            # ----------------------------------------------------------
            # Recovered existing + O_TRUNC
            # ----------------------------------------------------------

            truncate_name = (
                f"create-recovered-truncate-{uuid.uuid4().hex}.bin"
            )
            truncate_a = mount_a_dir / truncate_name
            truncate_b = mount_b_dir / truncate_name
            truncate_payload = b"FOD recovered existing truncate payload"

            arm_barrier(barrier_dir, f"/{truncate_name}")

            open_thread, open_result = start_open(
                truncate_b,
                os.O_CREAT | os.O_WRONLY | os.O_TRUNC,
            )

            wait_barrier_ready(barrier_dir)

            creator_fd = os.open(
                truncate_a,
                os.O_CREAT | os.O_EXCL | os.O_WRONLY,
                0o644,
            )
            try:
                os.write(creator_fd, truncate_payload)
                os.fsync(creator_fd)
            finally:
                os.close(creator_fd)

            wait_for_ownership_cleanup(launcher_a, truncate_name)

            if db_file_size(launcher_a, truncate_name) != len(truncate_payload):
                raise AssertionError("truncate setup size mismatch")

            release_barrier(barrier_dir)

            recovered_fd, recovered_errno = finish_open(
                open_thread,
                open_result,
                "recovered O_TRUNC",
            )

            if recovered_errno is not None:
                raise AssertionError(
                    f"recovered O_TRUNC failed errno={recovered_errno}"
                )
            if recovered_fd is None:
                raise AssertionError("recovered O_TRUNC returned no fd")

            os.close(recovered_fd)

            if db_file_size(launcher_a, truncate_name) != 0:
                raise AssertionError(
                    "recovered O_TRUNC did not truncate existing file"
                )

            wait_for_ownership_cleanup(launcher_a, truncate_name)

            print(
                "OK create-recovered-existing-truncate "
                "size=0 ownership_leaks=0"
            )

            # ----------------------------------------------------------
            # Recovered existing + O_EXCL
            # ----------------------------------------------------------

            excl_name = f"create-recovered-excl-{uuid.uuid4().hex}.bin"
            excl_a = mount_a_dir / excl_name
            excl_b = mount_b_dir / excl_name
            excl_payload = b"FOD recovered existing O_EXCL payload"

            arm_barrier(barrier_dir, f"/{excl_name}")

            excl_thread, excl_result = start_open(
                excl_b,
                os.O_CREAT | os.O_EXCL | os.O_WRONLY,
            )

            wait_barrier_ready(barrier_dir)

            creator_fd = os.open(
                excl_a,
                os.O_CREAT | os.O_EXCL | os.O_WRONLY,
                0o644,
            )
            try:
                os.write(creator_fd, excl_payload)
                os.fsync(creator_fd)
            finally:
                os.close(creator_fd)

            wait_for_ownership_cleanup(launcher_a, excl_name)

            if db_file_size(launcher_a, excl_name) != len(excl_payload):
                raise AssertionError("O_EXCL setup size mismatch")

            release_barrier(barrier_dir)

            excl_fd, excl_errno = finish_open(
                excl_thread,
                excl_result,
                "recovered O_EXCL",
            )

            if excl_fd is not None:
                os.close(excl_fd)
                raise AssertionError(
                    "recovered O_EXCL unexpectedly returned an fd"
                )

            if excl_errno != errno.EEXIST:
                raise AssertionError(
                    f"recovered O_EXCL expected EEXIST, got {excl_errno}"
                )

            if db_file_size(launcher_a, excl_name) != len(excl_payload):
                raise AssertionError(
                    "recovered O_EXCL changed existing file size"
                )

            wait_for_ownership_cleanup(launcher_a, excl_name)

            print(
                "OK create-recovered-existing-excl "
                f"errno={excl_errno} "
                f"size={len(excl_payload)} ownership_leaks=0"
            )
        except Exception:
            launcher_a._dump_log()
            launcher_b._dump_log()
            raise
        finally:
            if fd_b is not None:
                try:
                    os.close(fd_b)
                except OSError:
                    pass
            if fd_a is not None:
                try:
                    os.close(fd_a)
                except OSError:
                    pass
            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
