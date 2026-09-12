#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import subprocess
import tempfile
import time
from pathlib import Path

from fod_mount import FODMount

BLOCK_SIZE = 4096
FAIL_FAST_LIMIT_SECONDS = 1.0
DF_TIMEOUT_SECONDS = 3.0
FILE_NAME = "stale-writer-fence.bin"


def sql_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def psql_scalar(sql: str) -> str:
    env = os.environ.copy()
    host = env.get("POSTGRES_HOST", env.get("FOD_PG_HOST", "127.0.0.1"))
    port = env.get("POSTGRES_PORT", env.get("FOD_PG_PORT", "5432"))
    dbname = env.get("POSTGRES_DB", env.get("FOD_PG_DBNAME", "foddbname_test"))
    user = env.get("POSTGRES_USER", env.get("FOD_PG_USER", "foduser"))
    password = env.get("POSTGRES_PASSWORD", env.get("FOD_PG_PASSWORD", "cichosza"))
    env["PGPASSWORD"] = password
    result = subprocess.run(
        ["psql", "-h", host, "-p", port, "-U", user, "-d", dbname,
         "-At", "-v", "ON_ERROR_STOP=1", "-c", sql],
        env=env, check=True, capture_output=True, text=True,
    )
    return result.stdout.strip()


def session_for_mount(mountpoint: Path) -> int:
    raw = psql_scalar(
        f"""
        SELECT session_id
        FROM fod.client_sessions
        WHERE mountpoint = {sql_quote(str(mountpoint))}
        ORDER BY started_at DESC
        LIMIT 1
        """
    )
    if not raw:
        raise AssertionError(f"client session missing for mount {mountpoint}")
    return int(raw)


def file_lease_snapshot() -> tuple[int, int, int]:
    raw = psql_scalar(
        f"""
        SELECT
            fwl.fencing_token::text || '|' ||
            fwl.session_id::text || '|' ||
            fwl.file_id::text
        FROM fod.file_write_leases fwl
        JOIN fod.files f ON f.id_file = fwl.file_id
        WHERE f.id_directory IS NULL
          AND f.name = {sql_quote(FILE_NAME)}
        ORDER BY fwl.fencing_token DESC
        LIMIT 1
        """
    )
    if not raw:
        raise AssertionError("active file write lease missing")
    token_text, session_text, file_id_text = raw.split("|", 2)
    return int(token_text), int(session_text), int(file_id_text)


def revoke_write_ownership(session_id: int, file_id: int) -> None:
    raw = psql_scalar(
        f"""
        WITH
        deleted_file AS (
            DELETE FROM fod.file_write_leases
            WHERE session_id = {session_id}
              AND file_id = {file_id}
            RETURNING 1
        ),
        deleted_destination AS (
            DELETE FROM fod.destination_write_leases
            WHERE session_id = {session_id}
              AND parent_key = 0
              AND name = {sql_quote(FILE_NAME)}
            RETURNING 1
        )
        SELECT
            (SELECT COUNT(*) FROM deleted_file)::text || '|' ||
            (SELECT COUNT(*) FROM deleted_destination)::text
        """
    )
    if raw != "1|1":
        raise AssertionError(
            f"expected one file and destination ownership row removed, got {raw!r}"
        )


def assert_no_ownership_for_session(session_id: int, file_id: int) -> None:
    raw = psql_scalar(
        f"""
        SELECT
            (SELECT COUNT(*)
             FROM fod.file_write_leases
             WHERE session_id = {session_id}
               AND file_id = {file_id})::text || '|' ||
            (SELECT COUNT(*)
             FROM fod.destination_write_leases
             WHERE session_id = {session_id}
               AND parent_key = 0
               AND name = {sql_quote(FILE_NAME)})::text
        """
    )
    if raw != "0|0":
        raise AssertionError(f"revoked ownership unexpectedly returned: {raw}")


def assert_df_responsive(mountpoint: Path, stage: str) -> float:
    started = time.monotonic()
    try:
        result = subprocess.run(
            ["df", "-P", str(mountpoint)],
            check=False,
            capture_output=True,
            text=True,
            timeout=DF_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as exc:
        raise AssertionError(
            f"df hung stage={stage} timeout={DF_TIMEOUT_SECONDS:.1f}s"
        ) from exc

    elapsed = time.monotonic() - started
    if result.returncode != 0:
        raise AssertionError(
            f"df failed stage={stage} rc={result.returncode} "
            f"stderr={result.stderr.strip()!r}"
        )
    return elapsed


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    with tempfile.TemporaryDirectory(prefix="fod-stale-writer-fence-") as temp_dir:
        temp = Path(temp_dir)
        mount_a_dir = temp / "mount-a"
        mount_b_dir = temp / "mount-b"

        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        launcher_a.init_schema()

        fd_a: int | None = None
        fd_b: int | None = None

        try:
            launcher_a.start(str(mount_a_dir), log_prefix="/tmp/fod-stale-fence-a")
            launcher_b.start(str(mount_b_dir), log_prefix="/tmp/fod-stale-fence-b")

            path_a = mount_a_dir / FILE_NAME
            path_b = mount_b_dir / FILE_NAME
            path_a.write_bytes(b"S" * BLOCK_SIZE)

            fd_a = os.open(path_a, os.O_WRONLY)
            session_a = session_for_mount(mount_a_dir)
            token_a, lease_session_a, file_id = file_lease_snapshot()
            if lease_session_a != session_a:
                raise AssertionError(
                    f"lease session mismatch: mount_a={session_a} lease={lease_session_a}"
                )

            df_before = assert_df_responsive(mount_a_dir, "before_revoke")

            # Test kontrolowany: odbierz tylko ownership, daemon FUSE pozostaje zywy.
            revoke_write_ownership(session_a, file_id)
            assert_no_ownership_for_session(session_a, file_id)
            df_after_revoke = assert_df_responsive(mount_a_dir, "after_revoke")

            takeover_started = time.monotonic()
            fd_b = os.open(path_b, os.O_WRONLY | os.O_TRUNC)
            takeover_elapsed = time.monotonic() - takeover_started
            if takeover_elapsed >= FAIL_FAST_LIMIT_SECONDS:
                raise AssertionError(
                    f"takeover did not complete fail-fast: elapsed={takeover_elapsed:.6f}s"
                )

            token_b, lease_session_b, file_id_b = file_lease_snapshot()
            if file_id_b != file_id:
                raise AssertionError(f"file identity changed: {file_id}->{file_id_b}")
            if token_b <= token_a:
                raise AssertionError(
                    f"takeover fencing token did not advance: {token_a}->{token_b}"
                )
            if lease_session_b == session_a:
                raise AssertionError("takeover still belongs to stale session A")

            os.write(fd_b, b"B" * BLOCK_SIZE)
            os.fsync(fd_b)

            winner_before = path_b.read_bytes()
            if winner_before != b"B" * BLOCK_SIZE:
                raise AssertionError(
                    f"writer B payload missing before stale fsync: len={len(winner_before)}"
                )

            stale_write_errno: int | None = None
            try:
                os.pwrite(fd_a, b"A", 0)
            except OSError as exc:
                stale_write_errno = exc.errno
                if stale_write_errno != errno.EBUSY:
                    raise

            stale_fsync_errno: int | None = None
            try:
                os.fsync(fd_a)
            except OSError as exc:
                stale_fsync_errno = exc.errno

            if stale_fsync_errno != errno.EBUSY:
                raise AssertionError(
                    f"stale writer fsync errno={stale_fsync_errno}, expected EBUSY"
                )

            token_after, session_after, file_id_after = file_lease_snapshot()
            if (token_after, session_after, file_id_after) != (
                token_b, lease_session_b, file_id_b
            ):
                raise AssertionError(
                    "stale writer changed active ownership after rejected fsync: "
                    f"before={(token_b, lease_session_b, file_id_b)} "
                    f"after={(token_after, session_after, file_id_after)}"
                )

            winner_after = path_b.read_bytes()
            if winner_after != b"B" * BLOCK_SIZE:
                raise AssertionError(
                    "stale writer mutated winner B payload: "
                    f"len={len(winner_after)} prefix={winner_after[:16]!r}"
                )

            df_after_fence = assert_df_responsive(mount_a_dir, "after_stale_fsync")

            print(
                "OK stale-writer-fencing "
                f"session_a={session_a} "
                f"token_a={token_a} token_b={token_b} "
                f"takeover_elapsed_ms={takeover_elapsed * 1000.0:.3f} "
                f"stale_write_errno={stale_write_errno} "
                f"stale_fsync_errno={stale_fsync_errno} "
                f"winner_size={len(winner_after)} "
                f"df_ms={df_before * 1000.0:.3f}/"
                f"{df_after_revoke * 1000.0:.3f}/"
                f"{df_after_fence * 1000.0:.3f}"
            )
        except Exception:
            launcher_a._dump_log()
            launcher_b._dump_log()
            raise
        finally:
            if fd_a is not None:
                try:
                    os.close(fd_a)
                except OSError:
                    pass
            if fd_b is not None:
                try:
                    os.close(fd_b)
                except OSError:
                    pass

            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
