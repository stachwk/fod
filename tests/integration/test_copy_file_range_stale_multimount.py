#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

"""C8: a stale FUSE handle must not adopt a data object after lease takeover."""

from __future__ import annotations

import errno
import os
import sys
import tempfile
import time
import uuid
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount

FAIL_FAST_SECONDS = 1.0


def file_snapshot(conn, name: str) -> tuple[int, int, int]:
    with conn.cursor() as cursor:
        cursor.execute(
            """
            SELECT id_file, data_object_id, size
              FROM fod.files
             WHERE id_directory IS NULL AND name = %s
            """,
            (name,),
        )
        values = cursor.fetchall()
    if len(values) != 1:
        raise AssertionError(f"expected one root-level file {name!r}, found {values!r}")
    return tuple(map(int, values[0]))


def lease_snapshot(conn, file_id: int) -> tuple[int, int]:
    with conn.cursor() as cursor:
        cursor.execute(
            """
            SELECT fencing_token, session_id
              FROM fod.file_write_leases
             WHERE file_id = %s
            """,
            (file_id,),
        )
        values = cursor.fetchall()
    if len(values) != 1:
        raise AssertionError(f"expected one file lease for {file_id}, got {values!r}")
    return tuple(map(int, values[0]))


def mount_session(conn, path: Path) -> int:
    with conn.cursor() as cursor:
        cursor.execute(
            """
            SELECT session_id
              FROM fod.client_sessions
             WHERE mountpoint = %s
             ORDER BY started_at DESC LIMIT 1
            """,
            (str(path),),
        )
        value = cursor.fetchone()
    if value is None:
        raise AssertionError(f"mount session missing for {path}")
    return int(value[0])


def revoke_only_test_lease(
    conn, name: str, file_id: int, session_id: int
) -> None:
    with conn.cursor() as cursor:
        cursor.execute(
            """
            WITH
            removed_file AS (
                DELETE FROM fod.file_write_leases
                 WHERE file_id = %s AND session_id = %s
                 RETURNING 1
            ),
            removed_destination AS (
                DELETE FROM fod.destination_write_leases
                 WHERE parent_key = 0 AND name = %s AND session_id = %s
                 RETURNING 1
            )
            SELECT
                (SELECT COUNT(*) FROM removed_file),
                (SELECT COUNT(*) FROM removed_destination)
            """,
            (file_id, session_id, name, session_id),
        )
        result = tuple(map(int, cursor.fetchone()))
    if result != (1, 1):
        raise AssertionError(f"C8 lease revocation was incomplete: {result!r}")


def wait_for_path(path: Path, timeout: float = 10.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists():
            return
        time.sleep(0.05)
    raise AssertionError(f"C8 remote mount did not observe {path}")


def main() -> None:
    suffix = uuid.uuid4().hex[:12]
    src_name = f"c8-source-{suffix}.bin"
    dst_name = f"c8-destination-{suffix}.bin"
    launcher_a = FODMount(str(ROOT))
    launcher_b = FODMount(str(ROOT))
    db = launcher_a.postgres_db

    if db == "foddbname" or "test" not in db.lower():
        raise RuntimeError(
            "C8 requires a dedicated FOD_PG_DBNAME containing 'test'; "
            "the production database must not be used"
        )
    if db != launcher_b.postgres_db:
        raise RuntimeError("C8 requires both mounts on the same database")

    launcher_a.init_schema()
    with tempfile.TemporaryDirectory(prefix=f"fod-c8-copy-{suffix}-") as tmp:
        mount_a = Path(tmp) / "mount-a"
        mount_b = Path(tmp) / "mount-b"
        stale_fd: int | None = None
        winner_fd: int | None = None
        conn = None

        try:
            launcher_a.start(str(mount_a), log_prefix="fod-c8-a")
            launcher_b.start(str(mount_b), log_prefix="fod-c8-b")
            conn = psycopg2.connect(
                host=os.environ.get("FOD_PG_HOST") or os.environ.get("POSTGRES_HOST", "127.0.0.1"),
                port=os.environ.get("FOD_PG_PORT") or os.environ.get("POSTGRES_PORT", "5432"),
                dbname=db,
                user=launcher_a.postgres_user,
                password=launcher_a.postgres_password,
            )
            conn.autocommit = True
            with conn.cursor() as cursor:
                cursor.execute("SELECT value FROM fod.config WHERE key = 'block_size'")
                block_size = int(cursor.fetchone()[0])
            payload = b"S" * (2 * block_size)

            # The shared destination must stay empty so copy_file_range uses
            # the whole-data-object adoption path, not a block-copy fallback.
            (mount_a / src_name).write_bytes(payload)
            (mount_a / dst_name).write_bytes(b"")
            src_a, src_b = mount_a / src_name, mount_b / src_name
            dst_a, dst_b = mount_a / dst_name, mount_b / dst_name
            wait_for_path(src_b)
            wait_for_path(dst_b)
            src_before = file_snapshot(conn, src_name)
            dst_before = file_snapshot(conn, dst_name)
            if src_before[2] != len(payload) or dst_before[2] != 0:
                raise AssertionError("C8 invalid initial source/destination sizes")
            if src_before[1] == dst_before[1]:
                raise AssertionError("C8 source and destination already share data object")

            stale_fd = os.open(dst_a, os.O_RDWR)
            session_a = mount_session(conn, mount_a)
            token_a, lease_session_a = lease_snapshot(conn, dst_before[0])
            if lease_session_a != session_a:
                raise AssertionError("C8 stale handle lease does not belong to mount A")

            # Simulate lease loss without terminating daemon A or closing fh A.
            revoke_only_test_lease(conn, dst_name, dst_before[0], session_a)

            started = time.monotonic()
            winner_fd = os.open(dst_b, os.O_RDWR)
            takeover_ms = (time.monotonic() - started) * 1000.0
            if takeover_ms >= FAIL_FAST_SECONDS * 1000:
                raise AssertionError(f"C8 takeover took too long: {takeover_ms:.3f}ms")
            token_b, session_b = lease_snapshot(conn, dst_before[0])
            if token_b <= token_a or session_b == session_a:
                raise AssertionError(
                    f"C8 invalid fencing transition: {token_a}/{session_a} -> "
                    f"{token_b}/{session_b}"
                )

            # A remains open with its stale fencing token and may still issue
            # copy_file_range. PG must reject its adoption transaction.
            started = time.monotonic()
            with src_a.open("rb") as src_fd:
                try:
                    copied = os.copy_file_range(
                        src_fd.fileno(),
                        stale_fd,
                        len(payload),
                        offset_src=0,
                        offset_dst=0,
                    )
                except OSError as error:
                    stale_errno = error.errno
                else:
                    raise AssertionError(
                        f"C8 stale handle unexpectedly copied {copied} bytes"
                    )
            stale_ms = (time.monotonic() - started) * 1000.0
            # FUSE maps a fenced adoption failure to EIO in fs.rs.
            if stale_errno != errno.EIO or stale_ms >= FAIL_FAST_SECONDS * 1000:
                raise AssertionError(
                    f"C8 stale adoption error={stale_errno} in {stale_ms:.3f}ms, "
                    f"expected EIO ({errno.EIO}) within 1s"
                )
            if file_snapshot(conn, dst_name) != dst_before:
                raise AssertionError("C8 stale adoption modified destination metadata")
            if dst_b.read_bytes() != b"":
                raise AssertionError("C8 stale adoption modified destination bytes")
            if lease_snapshot(conn, dst_before[0]) != (token_b, session_b):
                raise AssertionError("C8 stale adoption changed B's active lease")

            # B is the current owner. B must be able to adopt the same source
            # while A's now-stale descriptor remains open.
            with src_b.open("rb") as src_fd:
                copied = os.copy_file_range(
                    src_fd.fileno(),
                    winner_fd,
                    len(payload),
                    offset_src=0,
                    offset_dst=0,
                )
            if copied != len(payload):
                raise AssertionError(f"C8 current owner copied only {copied} bytes")
            os.fsync(winner_fd)
            src_after, dst_after = (
                file_snapshot(conn, src_name),
                file_snapshot(conn, dst_name),
            )
            if src_after != src_before:
                raise AssertionError("C8 source changed during valid B adoption")
            if (
                dst_after[0] != dst_before[0]
                or dst_after[1] != src_after[1]
                or dst_after[2] != len(payload)
            ):
                raise AssertionError(
                    f"C8 valid B adoption failed: src={src_after}, dst={dst_after}"
                )
            if dst_b.read_bytes() != payload:
                raise AssertionError("C8 adopted destination has wrong bytes")
            if lease_snapshot(conn, dst_before[0]) != (token_b, session_b):
                raise AssertionError("C8 B lost lease after valid adoption")

            # Closing the stale A handle must not roll back B's successful
            # adoption or clear B's ownership. The kernel may report the
            # failed stale-handle state during close; only these errors are OK.
            try:
                os.close(stale_fd)
            except OSError as error:
                if error.errno not in (errno.EIO, errno.EBUSY):
                    raise
            stale_fd = None
            if file_snapshot(conn, dst_name) != dst_after:
                raise AssertionError("C8 stale handle close reverted valid adoption")
            if dst_b.read_bytes() != payload:
                raise AssertionError("C8 stale handle close changed B's payload")
            if lease_snapshot(conn, dst_before[0]) != (token_b, session_b):
                raise AssertionError("C8 stale handle close changed B's lease")

            print(
                "C8_FUSE_STALE_COPY=PASS "
                f"token={token_a}->{token_b} stale_errno={stale_errno} "
                f"takeover_ms={takeover_ms:.3f} stale_ms={stale_ms:.3f} "
                f"bytes={len(payload)} mounts=2"
            )
        except BaseException:
            print("\n=== C8 MOUNT A LOG ===")
            launcher_a._dump_log()
            print("\n=== C8 MOUNT B LOG ===")
            launcher_b._dump_log()
            raise
        finally:
            for fd in (stale_fd, winner_fd):
                if fd is not None:
                    try:
                        os.close(fd)
                    except OSError:
                        pass
            if conn is not None:
                conn.close()
            try:
                (mount_b / dst_name).unlink(missing_ok=True)
                (mount_b / src_name).unlink(missing_ok=True)
            except OSError:
                pass
            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
