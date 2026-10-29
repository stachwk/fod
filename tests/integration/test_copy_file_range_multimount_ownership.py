#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

"""C7: copy_file_range adoption observes ownership across two FOD mounts."""

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


def file_snapshot(connection, name: str) -> tuple[int, int, int]:
    with connection.cursor() as cursor:
        cursor.execute(
            """
            SELECT id_file, data_object_id, size
              FROM fod.files
             WHERE id_directory IS NULL
               AND name = %s
            """,
            (name,),
        )
        rows = cursor.fetchall()
    if len(rows) != 1:
        raise AssertionError(f"expected one file {name!r}, got {rows!r}")
    return tuple(int(value) for value in rows[0])


def assert_snapshot(connection, name: str, expected: tuple[int, int, int]) -> None:
    observed = file_snapshot(connection, name)
    if observed != expected:
        raise AssertionError(
            f"file {name!r} changed unexpectedly: before={expected} after={observed}"
        )


def wait_for_path(path: Path, timeout: float = 10.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists():
            return
        time.sleep(0.05)
    raise AssertionError(f"remote mount did not observe file {path}")


def main() -> None:
    suffix = uuid.uuid4().hex[:12]
    src_name = f"c7-copy-source-{suffix}.bin"
    dst_name = f"c7-copy-destination-{suffix}.bin"
    launcher_a = FODMount(str(ROOT))
    launcher_b = FODMount(str(ROOT))

    if (
        launcher_a.postgres_db == "foddbname"
        or "test" not in launcher_a.postgres_db.lower()
    ):
        raise RuntimeError(
            "C7 modifies files: select isolated FOD_PG_DBNAME containing 'test'"
        )
    if launcher_a.postgres_db != launcher_b.postgres_db:
        raise RuntimeError("C7 requires both mounts to use the same PostgreSQL database")

    launcher_a.init_schema()
    with tempfile.TemporaryDirectory(prefix=f"fod-c7-copy-{suffix}-") as tmp:
        mount_a = Path(tmp) / "mount-a"
        mount_b = Path(tmp) / "mount-b"
        fd_a: int | None = None
        connection = None
        try:
            launcher_a.start(str(mount_a), log_prefix="fod-c7-copy-a")
            launcher_b.start(str(mount_b), log_prefix="fod-c7-copy-b")

            connection = psycopg2.connect(
                host=os.environ.get("FOD_PG_HOST") or os.environ.get("POSTGRES_HOST", "127.0.0.1"),
                port=os.environ.get("FOD_PG_PORT") or os.environ.get("POSTGRES_PORT", "5432"),
                dbname=launcher_a.postgres_db,
                user=launcher_a.postgres_user,
                password=launcher_a.postgres_password,
            )
            connection.autocommit = True
            with connection.cursor() as cursor:
                cursor.execute("SELECT value FROM fod.config WHERE key = 'block_size'")
                block_size = int(cursor.fetchone()[0])
            payload = b"C" * (2 * block_size)

            src_b = mount_b / src_name
            dst_b = mount_b / dst_name
            dst_a = mount_a / dst_name

            src_b.write_bytes(payload)
            dst_b.write_bytes(b"")
            wait_for_path(dst_a)
            src_before = file_snapshot(connection, src_name)
            dst_before = file_snapshot(connection, dst_name)
            if src_before[2] != len(payload) or dst_before[2] != 0:
                raise AssertionError(
                    f"unexpected precondition: source={src_before} destination={dst_before}"
                )
            if src_before[1] == dst_before[1]:
                raise AssertionError("source and empty destination already share data object")

            # A and B are distinct daemon processes. A's writable descriptor
            # holds the destination lease while B tries to open it for copying.
            fd_a = os.open(dst_a, os.O_WRONLY)
            started = time.monotonic()
            try:
                fd_b = os.open(dst_b, os.O_RDWR)
            except OSError as exc:
                loser_errno = exc.errno
            else:
                os.close(fd_b)
                raise AssertionError("B acquired destination while A held the write lease")
            elapsed = time.monotonic() - started
            if loser_errno != errno.EBUSY or elapsed >= FAIL_FAST_SECONDS:
                raise AssertionError(
                    f"copy destination ownership refusal errno={loser_errno} "
                    f"elapsed={elapsed:.6f}s expected EBUSY before {FAIL_FAST_SECONDS}s"
                )
            assert_snapshot(connection, dst_name, dst_before)
            if dst_b.read_bytes() != b"":
                raise AssertionError("refused writer changed destination bytes")

            os.close(fd_a)
            fd_a = None

            # A has released the destination lease. B may now enter FUSE's
            # full-object copy_file_range fast path with a valid write fence.
            with src_b.open("rb") as src_fd, dst_b.open("r+b") as dst_fd:
                src_fd_size = os.fstat(src_fd.fileno()).st_size
                dst_fd_size = os.fstat(dst_fd.fileno()).st_size
                if src_fd_size != len(payload) or dst_fd_size != 0:
                    raise AssertionError(
                        "unexpected kernel-visible sizes before copy: "
                        f"source={src_fd_size} destination={dst_fd_size}"
                    )
                copied = os.copy_file_range(
                    src_fd.fileno(),
                    dst_fd.fileno(),
                    len(payload),
                    offset_src=0,
                    offset_dst=0,
                )
                if copied != len(payload):
                    raise AssertionError(f"short copy: {copied} != {len(payload)}")
                os.fsync(dst_fd.fileno())

            src_after = file_snapshot(connection, src_name)
            dst_after = file_snapshot(connection, dst_name)
            if src_after[0] != src_before[0] or src_after[1] != src_before[1]:
                raise AssertionError("source data object changed during adoption")
            if dst_after[0] != dst_before[0]:
                raise AssertionError("destination identity changed during adoption")
            if dst_after[1] != src_after[1] or dst_after[2] != len(payload):
                raise AssertionError(
                    f"full-object adoption did not occur: source={src_after} "
                    f"destination={dst_after}"
                )
            if dst_b.read_bytes() != payload or src_b.read_bytes() != payload:
                raise AssertionError("data changed or became unreadable after adoption")

            print(
                "C7_FUSE_COPY_ADOPTION=PASS "
                f"loser_errno={loser_errno} loser_elapsed_ms={elapsed * 1000:.3f} "
                f"bytes={len(payload)} shared_object={dst_after[1]} "
                "mounts=2"
            )
        except BaseException:
            print("\n=== C7 MOUNT A LOG ===")
            launcher_a._dump_log()
            print("\n=== C7 MOUNT B LOG ===")
            launcher_b._dump_log()
            raise
        finally:
            if fd_a is not None:
                os.close(fd_a)
            if connection is not None:
                connection.close()
            # Avoid unlink while FUSE daemons are still serving open handles.
            try:
                (mount_b / dst_name).unlink(missing_ok=True)
                (mount_b / src_name).unlink(missing_ok=True)
            except OSError:
                pass
            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
