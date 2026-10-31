#!/usr/bin/env python3
from __future__ import annotations

import errno
import os
import stat
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
POST_CREATE_BARRIER_ENV = "FOD_TEST_CREATE_AFTER_COMMIT_BARRIER_DIR"
OPEN_INODE_BARRIER_ENV = "FOD_TEST_OPEN_AFTER_INODE_VALIDATION_BARRIER_DIR"


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


def start_operation(operation):
    result: dict[str, object] = {"errno": None}

    def worker() -> None:
        try:
            operation()
        except OSError as exc:
            result["errno"] = exc.errno

    thread = threading.Thread(target=worker, daemon=True)
    thread.start()
    return thread, result


def finish_operation(
    thread: threading.Thread,
    result: dict[str, object],
    label: str,
) -> int | None:
    thread.join(BARRIER_TIMEOUT_SECONDS)
    if thread.is_alive():
        raise AssertionError(f"{label}: operation thread did not finish")
    error = result["errno"]
    return int(error) if error is not None else None


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
            post_create_barrier_dir = temp / "post-create-barrier"
            post_create_barrier_dir.mkdir()
            open_inode_barrier_dir = temp / "open-inode-barrier"
            open_inode_barrier_dir.mkdir()

            previous_post_create_barrier = os.environ.get(POST_CREATE_BARRIER_ENV)
            previous_open_inode_barrier = os.environ.get(OPEN_INODE_BARRIER_ENV)
            os.environ[POST_CREATE_BARRIER_ENV] = str(post_create_barrier_dir)
            os.environ[OPEN_INODE_BARRIER_ENV] = str(open_inode_barrier_dir)
            try:
                launcher_a.start(str(mount_a_dir), log_prefix="/tmp/fod-create-own-a")
            finally:
                if previous_post_create_barrier is None:
                    os.environ.pop(POST_CREATE_BARRIER_ENV, None)
                else:
                    os.environ[POST_CREATE_BARRIER_ENV] = previous_post_create_barrier
                if previous_open_inode_barrier is None:
                    os.environ.pop(OPEN_INODE_BARRIER_ENV, None)
                else:
                    os.environ[OPEN_INODE_BARRIER_ENV] = previous_open_inode_barrier

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

            # Pause CREATE after its PostgreSQL commit but before its FUSE
            # reply. A second mount removes and replaces the name in that
            # window. The original fd must keep the created inode identity.
            race_name = f"create-reply-replacement-{uuid.uuid4().hex}.bin"
            race_path_a = mount_a_dir / race_name
            race_path_b = mount_b_dir / race_name
            (post_create_barrier_dir / "ready").unlink(missing_ok=True)
            (post_create_barrier_dir / "release").unlink(missing_ok=True)
            (post_create_barrier_dir / "target").write_text(
                f"/{race_name}", encoding="utf-8"
            )
            create_thread, create_result = start_open(
                race_path_a,
                os.O_CREAT | os.O_EXCL | os.O_RDWR,
            )
            wait_barrier_ready(post_create_barrier_dir)
            race_fd_a = None
            race_fd_b = None
            try:
                race_path_b.unlink()
                race_fd_b = os.open(
                    race_path_b,
                    os.O_CREAT | os.O_EXCL | os.O_RDWR,
                    0o644,
                )
                os.write(race_fd_b, b"replacement")
                os.fsync(race_fd_b)
            finally:
                release_barrier(post_create_barrier_dir)
            race_fd_a, create_errno = finish_open(
                create_thread,
                create_result,
                "create reply replacement race",
            )
            if create_errno is not None or race_fd_a is None:
                raise AssertionError(
                    "CREATE failed after concurrent pathname replacement: "
                    f"errno={create_errno}"
                )
            original_inode = os.fstat(race_fd_a).st_ino
            assert race_fd_b is not None
            replacement_inode = os.fstat(race_fd_b).st_ino
            if original_inode == replacement_inode:
                raise AssertionError(
                    "CREATE reply bound the original handle to the replacement inode: "
                    f"inode={original_inode}"
                )
            os.write(race_fd_a, b"original")
            os.fsync(race_fd_a)
            if os.pread(race_fd_a, 8, 0) != b"original":
                raise AssertionError("original CREATE handle lost its file identity")
            if race_path_b.read_bytes() != b"replacement":
                raise AssertionError("original CREATE handle modified the replacement")
            print(
                "OK create-reply-path-replacement "
                f"original_inode={original_inode} replacement_inode={replacement_inode}"
            )
            os.close(race_fd_a)
            os.close(race_fd_b)
            fd_b = None
            wait_for_ownership_cleanup(launcher_a, race_name)
            wait_for_ownership_cleanup(launcher_b, race_name)

            # The other namespace-creation callbacks also used to resolve
            # their destination path after commit. If that object was already
            # removed, they must not return the replacement's attributes.
            def replacement_race(
                label: str,
                race_name: str,
                operation,
                replace,
                cleanup,
            ) -> None:
                race_path_a = mount_a_dir / race_name
                race_path_b = mount_b_dir / race_name
                for control_file in ("ready", "release"):
                    (post_create_barrier_dir / control_file).unlink(missing_ok=True)
                (post_create_barrier_dir / "target").write_text(
                    f"/{race_name}", encoding="utf-8"
                )
                thread, result = start_operation(lambda: operation(race_path_a))
                wait_barrier_ready(post_create_barrier_dir)
                try:
                    replace(race_path_b)
                except OSError:
                    for launcher in (launcher_a, launcher_b):
                        if launcher.config is not None:
                            print(
                                launcher.config.log_file.read_text(
                                    encoding="utf-8"
                                ),
                                end="",
                            )
                    raise
                finally:
                    release_barrier(post_create_barrier_dir)
                error = finish_operation(thread, result, label)
                if error != errno.EIO:
                    raise AssertionError(
                        f"{label}: expected EIO after original entry was removed, "
                        f"got errno={error}"
                    )
                cleanup(race_path_b)
                print(f"OK {label} replacement_not_returned errno={error}")

            replacement_race(
                "mkdir-reply-path-replacement",
                f"mkdir-reply-replacement-{uuid.uuid4().hex}",
                lambda path: os.mkdir(path, 0o755),
                lambda path: (os.rmdir(path), os.mkdir(path, 0o755)),
                lambda path: os.rmdir(path),
            )
            replacement_race(
                "mknod-reply-path-replacement",
                f"mknod-reply-replacement-{uuid.uuid4().hex}.bin",
                lambda path: os.mknod(path, stat.S_IFREG | 0o600),
                lambda path: (path.unlink(), path.write_bytes(b"replacement")),
                lambda path: path.unlink(),
            )
            replacement_race(
                "symlink-reply-path-replacement",
                f"symlink-reply-replacement-{uuid.uuid4().hex}",
                lambda path: os.symlink("original-target", path),
                lambda path: (path.unlink(), os.symlink("replacement-target", path)),
                lambda path: path.unlink(),
            )

            # A hardlink reply must keep the source inode even if another
            # mount removes and replaces the new alias before FUSE replies.
            source_name = f"link-reply-source-{uuid.uuid4().hex}.bin"
            alias_name = f"link-reply-alias-{uuid.uuid4().hex}.bin"
            source_path_a = mount_a_dir / source_name
            source_path_b = mount_b_dir / source_name
            alias_path_a = mount_a_dir / alias_name
            alias_path_b = mount_b_dir / alias_name
            source_path_a.write_bytes(b"source")
            for control_file in ("ready", "release"):
                (post_create_barrier_dir / control_file).unlink(missing_ok=True)
            (post_create_barrier_dir / "target").write_text(
                f"/{alias_name}", encoding="utf-8"
            )
            link_thread, link_result = start_operation(
                lambda: os.link(source_path_a, alias_path_a)
            )
            wait_barrier_ready(post_create_barrier_dir)
            try:
                alias_path_b.unlink()
                alias_path_b.write_bytes(b"replacement")
            finally:
                release_barrier(post_create_barrier_dir)
            link_errno = finish_operation(
                link_thread,
                link_result,
                "link reply path replacement",
            )
            if link_errno is not None:
                raise AssertionError(f"link reply failed with errno={link_errno}")
            source_inode = source_path_a.stat().st_ino
            alias_inode = alias_path_a.stat().st_ino
            replacement_inode = alias_path_b.stat().st_ino
            if alias_inode != source_inode or alias_inode == replacement_inode:
                raise AssertionError(
                    "link reply used replacement inode: "
                    f"source={source_inode} alias={alias_inode} "
                    f"replacement={replacement_inode}"
                )
            print(
                "OK link-reply-path-replacement "
                f"source_inode={source_inode} replacement_inode={replacement_inode}"
            )
            alias_path_b.unlink()
            source_path_a.unlink()

            # An OPEN request carrying the old kernel inode must not resolve
            # its backing file by pathname after another mount replaces it.
            open_race_name = f"open-inode-replacement-{uuid.uuid4().hex}.bin"
            open_race_a = mount_a_dir / open_race_name
            open_race_b = mount_b_dir / open_race_name
            open_race_a.write_bytes(b"original")
            open_race_a.stat()
            arm_barrier(open_inode_barrier_dir, f"/{open_race_name}")
            open_thread, open_result = start_open(open_race_a, os.O_RDONLY)
            wait_barrier_ready(open_inode_barrier_dir)
            replacement_fd = None
            try:
                open_race_b.unlink()
                replacement_fd = os.open(
                    open_race_b,
                    os.O_CREAT | os.O_EXCL | os.O_WRONLY,
                    0o644,
                )
                os.write(replacement_fd, b"replacement")
                os.fsync(replacement_fd)
            finally:
                release_barrier(open_inode_barrier_dir)
            open_fd, open_errno = finish_open(
                open_thread,
                open_result,
                "open inode path replacement",
            )
            if replacement_fd is not None:
                os.close(replacement_fd)
            if open_fd is not None:
                os.close(open_fd)
                raise AssertionError(
                    "OPEN bound the validated old inode to the replacement path"
                )
            if open_errno != errno.ENOENT:
                raise AssertionError(
                    "OPEN did not fail for the removed validated inode: "
                    f"errno={open_errno}, expected={errno.ENOENT}"
                )
            if open_race_b.read_bytes() != b"replacement":
                raise AssertionError("OPEN race altered replacement contents")
            open_race_b.unlink()
            print("OK open-inode-path-replacement errno=ENOENT")

            if fd_a is not None:
                os.close(fd_a)
                fd_a = None

            wait_for_ownership_cleanup(launcher_a, file_name)

            # Existing-file O_APPEND follows the same cross-mount exclusive
            # write-ownership contract as create/truncate opens.
            append_fd_a = os.open(path_a, os.O_WRONLY | os.O_APPEND)
            try:
                os.write(append_fd_a, b"-A")
                os.fsync(append_fd_a)
                started = time.monotonic()
                try:
                    fd_b = os.open(path_b, os.O_WRONLY | os.O_APPEND)
                except OSError as exc:
                    append_loser_errno = exc.errno
                else:
                    os.close(fd_b)
                    fd_b = None
                    raise AssertionError(
                        "second mount acquired O_APPEND while the first held "
                        "the write lease"
                    )
                elapsed = time.monotonic() - started
            finally:
                os.close(append_fd_a)

            if append_loser_errno != errno.EBUSY or elapsed >= FAIL_FAST_SECONDS:
                raise AssertionError(
                    "cross-mount O_APPEND ownership refusal "
                    f"errno={append_loser_errno} elapsed={elapsed:.6f}s "
                    f"expected EBUSY before {FAIL_FAST_SECONDS}s"
                )
            if db_file_size(launcher_a, file_name) != 3:
                raise AssertionError("first mount O_APPEND did not extend the file")
            wait_for_ownership_cleanup(launcher_a, file_name)

            append_fd_b = os.open(path_b, os.O_WRONLY | os.O_APPEND)
            os.close(append_fd_b)
            wait_for_ownership_cleanup(launcher_a, file_name)

            print(
                "OK cross-mount-append-ownership "
                f"loser_errno={append_loser_errno} "
                f"loser_elapsed_ms={elapsed * 1000.0:.3f} "
                "first_append_size=3 lease_reacquired=true"
            )

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
