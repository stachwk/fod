#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount
from tests.integration.test_fallocate_contract import (
    FALLOC_FL_KEEP_SIZE,
    FALLOC_FL_PUNCH_HOLE,
    raw_fallocate,
)

FAIL_FAST_LIMIT_SECONDS = 1.0
FILE_NAME = "fallocate-stale-writer-fence.bin"


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
        [
            "psql", "-h", host, "-p", port, "-U", user, "-d", dbname,
            "-At", "-v", "ON_ERROR_STOP=1", "-c", sql,
        ],
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def block_size() -> int:
    raw = psql_scalar("SELECT value FROM fod.config WHERE key = 'block_size'")
    value = int(raw)
    if value <= 0:
        raise AssertionError(f"invalid block_size={value}")
    return value


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
    token, session_id, file_id = raw.split("|", 2)
    return int(token), int(session_id), int(file_id)


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
            f"expected one file and destination lease removed, got {raw!r}"
        )


def payload_state() -> str:
    return psql_scalar(
        f"""
        SELECT
            f.size::text || '|' ||
            f.data_object_id::text || '|' ||
            COUNT(b.id_block)::text || '|' ||
            COALESCE(SUM(octet_length(b.data)), 0)::text
        FROM fod.files f
        LEFT JOIN fod.data_blocks b
          ON b.data_object_id = f.data_object_id
        WHERE f.id_directory IS NULL
          AND f.name = {sql_quote(FILE_NAME)}
        GROUP BY f.size, f.data_object_id
        """
    )


def require_ebusy(result, label: str) -> None:
    if result.rc == 0 or result.errno != errno.EBUSY:
        raise AssertionError(f"{label}: expected EBUSY, got {result}")


def main() -> int:
    template = FODMount(str(ROOT))
    template.init_schema()
    bs = block_size()
    mode = FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE

    with tempfile.TemporaryDirectory(prefix="/tmp/fod-fallocate-stale-fence.") as temp_dir:
        temp = Path(temp_dir)
        mount_a = temp / "mount-a"
        mount_b = temp / "mount-b"

        launcher_a = FODMount(str(ROOT))
        launcher_b = FODMount(str(ROOT))
        launcher_a.postgres_db = template.postgres_db
        launcher_a.postgres_user = template.postgres_user
        launcher_a.postgres_password = template.postgres_password
        launcher_b.postgres_db = template.postgres_db
        launcher_b.postgres_user = template.postgres_user
        launcher_b.postgres_password = template.postgres_password

        fd_a = None
        fd_b = None
        try:
            launcher_a.start(str(mount_a), log_prefix="/tmp/fod-fallocate-stale-a")
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-fallocate-stale-b")

            path_a = mount_a / FILE_NAME
            path_b = mount_b / FILE_NAME
            baseline = bytes(((index * 31) + 7) % 251 for index in range(bs * 4))
            path_a.write_bytes(baseline)

            fd_a = os.open(path_a, os.O_RDWR)
            session_a = session_for_mount(mount_a)
            token_a, lease_session_a, file_id = file_lease_snapshot()
            if lease_session_a != session_a:
                raise AssertionError(
                    f"lease session mismatch A={session_a} lease={lease_session_a}"
                )

            revoke_write_ownership(session_a, file_id)

            takeover_started = time.monotonic()
            fd_b = os.open(path_b, os.O_RDWR)
            takeover_elapsed = time.monotonic() - takeover_started
            if takeover_elapsed >= FAIL_FAST_LIMIT_SECONDS:
                raise AssertionError(
                    f"writer B takeover too slow: {takeover_elapsed:.6f}s"
                )

            token_b, session_b, file_id_b = file_lease_snapshot()
            if file_id_b != file_id:
                raise AssertionError(f"file identity changed {file_id}->{file_id_b}")
            if token_b <= token_a:
                raise AssertionError(
                    f"fencing token did not advance {token_a}->{token_b}"
                )
            if session_b == session_a:
                raise AssertionError("writer B takeover still belongs to session A")

            winner_before = path_b.read_bytes()
            db_before = payload_state()

            stale_in_range = raw_fallocate(fd_a, mode, bs, bs)
            require_ebusy(stale_in_range, "stale in-range punch")
            if path_b.read_bytes() != winner_before:
                raise AssertionError("stale in-range punch mutated winner payload")
            if payload_state() != db_before:
                raise AssertionError("stale in-range punch mutated database payload")

            stale_beyond_eof = raw_fallocate(fd_a, mode, len(baseline) + bs, bs)
            require_ebusy(stale_beyond_eof, "stale beyond-EOF punch")
            if path_b.read_bytes() != winner_before:
                raise AssertionError("stale beyond-EOF punch mutated winner payload")
            if payload_state() != db_before:
                raise AssertionError("stale beyond-EOF punch mutated database payload")

            winner_beyond_eof = raw_fallocate(fd_b, mode, len(baseline) + bs, bs)
            if winner_beyond_eof.rc != 0:
                raise AssertionError(
                    f"current owner beyond-EOF no-op failed: {winner_beyond_eof}"
                )

            winner_punch = raw_fallocate(fd_b, mode, 2 * bs, bs)
            if winner_punch.rc != 0:
                raise AssertionError(f"current owner punch failed: {winner_punch}")
            os.fsync(fd_b)

            expected = bytearray(baseline)
            expected[2 * bs : 3 * bs] = b"\x00" * bs
            winner_after = path_b.read_bytes()
            if winner_after != bytes(expected):
                raise AssertionError("current owner punch produced wrong payload")

            token_after, session_after, file_id_after = file_lease_snapshot()
            if (token_after, session_after, file_id_after) != (
                token_b, session_b, file_id_b,
            ):
                raise AssertionError(
                    "stale fallocate changed active ownership: "
                    f"before={(token_b, session_b, file_id_b)} "
                    f"after={(token_after, session_after, file_id_after)}"
                )

            print(
                "OK fallocate-stale-writer-fencing "
                f"token_a={token_a} token_b={token_b} "
                f"takeover_ms={takeover_elapsed * 1000.0:.3f} "
                f"stale_in_range_errno={stale_in_range.errno} "
                f"stale_beyond_eof_errno={stale_beyond_eof.errno} "
                "winner_beyond_eof=1 winner_punch=1 payload_preserved=1"
            )
            return 0
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
    raise SystemExit(main())
