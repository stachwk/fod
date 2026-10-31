#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

"""C9: whole-object copy serializes against writes to its source inode."""

from __future__ import annotations

import os
import sys
import tempfile
import threading
import time
import uuid
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def file_snapshot(connection, name: str) -> tuple[int, int, int]:
    with connection.cursor() as cursor:
        cursor.execute(
            """
            SELECT id_file, data_object_id, size
              FROM fod.files
             WHERE id_directory IS NULL AND name = %s
            """,
            (name,),
        )
        rows = cursor.fetchall()
    if len(rows) != 1:
        raise AssertionError(f"expected one file {name!r}, got {rows!r}")
    return tuple(int(value) for value in rows[0])


def wait_for_blocker(connection, blocker_pid: int, timeout: float = 10.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        with connection.cursor() as cursor:
            cursor.execute(
                """
                SELECT EXISTS (
                    SELECT 1
                      FROM pg_stat_activity
                     WHERE %s = ANY(pg_blocking_pids(pid))
                       AND wait_event_type = 'Lock'
                       AND wait_event = 'advisory'
                )
                """,
                (blocker_pid,),
            )
            if cursor.fetchone()[0]:
                return
        time.sleep(0.02)
    raise AssertionError(
        "copy_file_range did not wait for the source inode write-order lock"
    )


def main() -> None:
    suffix = uuid.uuid4().hex[:12]
    src_name = f"c9-copy-source-{suffix}.bin"
    dst_name = f"c9-copy-destination-{suffix}.bin"
    launcher_a = FODMount(str(ROOT))
    launcher_b = FODMount(str(ROOT))
    database = launcher_a.postgres_db
    if database == "foddbname" or "test" not in database.lower():
        raise RuntimeError(
            "C9 modifies files: select isolated FOD_PG_DBNAME containing 'test'"
        )
    if database != launcher_b.postgres_db:
        raise RuntimeError("C9 requires both mounts to use the same database")

    launcher_a.init_schema()
    with tempfile.TemporaryDirectory(prefix=f"fod-c9-copy-{suffix}-") as tmp:
        mount_a = Path(tmp) / "mount-a"
        mount_b = Path(tmp) / "mount-b"
        source_fd = destination_fd = None
        copy_thread = None
        observer = blocker = None
        blocker_active = False
        copy_result: dict[str, int | BaseException] = {}
        try:
            launcher_a.start(str(mount_a), log_prefix="fod-c9-copy-a")
            launcher_b.start(str(mount_b), log_prefix="fod-c9-copy-b")

            connection_args = {
                "host": os.environ.get("FOD_PG_HOST")
                or os.environ.get("POSTGRES_HOST", "127.0.0.1"),
                "port": os.environ.get("FOD_PG_PORT")
                or os.environ.get("POSTGRES_PORT", "5432"),
                "dbname": database,
                "user": launcher_a.postgres_user,
                "password": launcher_a.postgres_password,
            }
            observer = psycopg2.connect(**connection_args)
            observer.autocommit = True
            blocker = psycopg2.connect(**connection_args)
            blocker.autocommit = False

            with observer.cursor() as cursor:
                cursor.execute("SELECT value FROM fod.config WHERE key = 'block_size'")
                block_size = int(cursor.fetchone()[0])
            source_payload = b"R" * (2 * block_size)
            replacement_payload = b"W" * len(source_payload)
            source_a, source_b = mount_a / src_name, mount_b / src_name
            destination_a, destination_b = mount_a / dst_name, mount_b / dst_name
            source_a.write_bytes(source_payload)
            destination_a.write_bytes(b"")
            source_before = file_snapshot(observer, src_name)
            destination_before = file_snapshot(observer, dst_name)
            if source_before[2] != len(source_payload) or destination_before[2] != 0:
                raise AssertionError(
                    f"invalid initial file sizes: {source_before}, {destination_before}"
                )
            if source_before[1] == destination_before[1]:
                raise AssertionError("source and destination unexpectedly share an object")

            source_fd = os.open(source_b, os.O_RDONLY)
            destination_fd = os.open(destination_b, os.O_RDWR)
            with blocker.cursor() as cursor:
                cursor.execute(
                    "SELECT pg_advisory_xact_lock(hashtextextended(%s, 0))",
                    (f"fod:write:file:{source_before[0]}",),
                )
            blocker_active = True
            blocker_pid = blocker.get_backend_pid()

            def copy_source_to_destination() -> None:
                try:
                    copy_result["bytes"] = os.copy_file_range(
                        source_fd,
                        destination_fd,
                        len(source_payload),
                        offset_src=0,
                        offset_dst=0,
                    )
                    os.fsync(destination_fd)
                except BaseException as error:
                    copy_result["error"] = error

            copy_thread = threading.Thread(
                target=copy_source_to_destination,
                name="fod-c9-copy-file-range",
                daemon=True,
            )
            copy_thread.start()
            wait_for_blocker(observer, blocker_pid)

            blocker.rollback()
            blocker_active = False
            copy_thread.join(timeout=10)
            if copy_thread.is_alive():
                raise AssertionError("copy_file_range stayed blocked after lock release")
            if "error" in copy_result:
                raise AssertionError("copy_file_range failed") from copy_result["error"]
            if copy_result.get("bytes") != len(source_payload):
                raise AssertionError(
                    f"short copy: {copy_result.get('bytes')} != {len(source_payload)}"
                )

            copied_destination = file_snapshot(observer, dst_name)
            if copied_destination[1] != source_before[1]:
                raise AssertionError(
                    "whole-object adoption did not share the source data object"
                )

            with source_a.open("r+b") as source_update:
                source_update.write(replacement_payload)
                source_update.flush()
                os.fsync(source_update.fileno())

            source_after = file_snapshot(observer, src_name)
            destination_after = file_snapshot(observer, dst_name)
            if source_after[1] == destination_after[1]:
                raise AssertionError("source write did not detach the shared data object")
            if destination_after[1] != source_before[1]:
                raise AssertionError("source write changed the destination data object")
            if destination_a.read_bytes() != source_payload:
                raise AssertionError("source write changed copied destination bytes")
            if source_a.read_bytes() != replacement_payload:
                raise AssertionError("source write did not persist replacement bytes")

            print(
                "C9_COPY_SOURCE_WRITE_FENCING=PASS "
                f"source_file_id={source_before[0]} "
                f"destination_file_id={destination_before[0]}"
            )
        finally:
            if blocker is not None and blocker_active:
                blocker.rollback()
            if copy_thread is not None:
                copy_thread.join(timeout=10)
            if source_fd is not None:
                os.close(source_fd)
            if destination_fd is not None:
                os.close(destination_fd)
            if observer is not None:
                observer.close()
            if blocker is not None:
                blocker.close()
            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
