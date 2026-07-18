#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
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

BLOCK_SIZE = 4096
QUOTA_LOCK_KEY = (4607812, 1)


def database_connection(launcher: FODMount, *, autocommit: bool = True):
    connection = psycopg2.connect(
        host=os.environ.get("POSTGRES_HOST", "127.0.0.1"),
        port=os.environ.get("POSTGRES_PORT", "5432"),
        dbname=launcher.postgres_db,
        user=launcher.postgres_user,
        password=launcher.postgres_password,
    )
    connection.autocommit = autocommit
    return connection


def payload_bytes(connection) -> int:
    with connection.cursor() as cursor:
        cursor.execute(
            """
            SELECT
                (SELECT COUNT(*)::bigint FROM fod.data_blocks)
                    * (SELECT value FROM fod.config WHERE key = 'block_size')
                + COALESCE(
                    (SELECT SUM(used_bytes)::bigint FROM fod.data_extents),
                    0
                )
            """
        )
        return int(cursor.fetchone()[0])


def write_and_sync(path: Path, marker: bytes, barrier: threading.Barrier) -> int | None:
    descriptor = os.open(path, os.O_WRONLY)
    error_number = None
    try:
        barrier.wait()
        written = os.write(descriptor, marker * BLOCK_SIZE)
        if written != BLOCK_SIZE:
            raise AssertionError(f"short write for {path}: {written}")
        os.fsync(descriptor)
    except OSError as error:
        error_number = error.errno
    finally:
        try:
            os.close(descriptor)
        except OSError as error:
            if error_number is None:
                error_number = error.errno
    return error_number


def wait_for_advisory_waiters(connection, expected: int) -> int:
    deadline = time.monotonic() + 10
    waiting = 0
    while time.monotonic() < deadline:
        with connection.cursor() as cursor:
            cursor.execute(
                """
                SELECT COUNT(*)
                FROM pg_stat_activity
                WHERE datname = current_database()
                  AND wait_event_type = 'Lock'
                  AND wait_event = 'advisory'
                """
            )
            waiting = int(cursor.fetchone()[0])
        if waiting >= expected:
            return waiting
        time.sleep(0.05)
    raise AssertionError(
        f"expected {expected} PostgreSQL advisory-lock waiters, observed {waiting}"
    )


def file_storage_state(connection, names: list[str]) -> dict[str, tuple[int, int]]:
    with connection.cursor() as cursor:
        cursor.execute(
            """
            SELECT
                files.name,
                files.size,
                (SELECT COUNT(*) FROM fod.data_blocks
                 WHERE data_object_id = files.data_object_id)
                    + (SELECT COUNT(*) FROM fod.data_extents
                       WHERE data_object_id = files.data_object_id)
            FROM fod.files
            WHERE files.name = ANY(%s)
            """,
            (names,),
        )
        return {
            str(name): (int(size), int(payload_rows))
            for name, size, payload_rows in cursor.fetchall()
        }


def main() -> None:
    suffix = uuid.uuid4().hex[:12]
    names = [f"quota-a-{suffix}.bin", f"quota-b-{suffix}.bin"]
    launcher_a = FODMount(str(ROOT))
    launcher_b = FODMount(str(ROOT))
    launcher_a.init_schema()

    with (
        tempfile.TemporaryDirectory(prefix=f"/tmp/fod-quota-a-{suffix}.") as mount_a_dir,
        tempfile.TemporaryDirectory(prefix=f"/tmp/fod-quota-b-{suffix}.") as mount_b_dir,
    ):
        mount_a = Path(mount_a_dir)
        mount_b = Path(mount_b_dir)
        launcher_a.start(str(mount_a), log_prefix="fod-quota-a")
        launcher_b.start(str(mount_b), log_prefix="fod-quota-b")

        observer = database_connection(launcher_a)
        blocker = database_connection(launcher_a, autocommit=False)
        original_limit = None
        paths = [mount_a / names[0], mount_b / names[1]]
        threads: list[threading.Thread] = []
        try:
            for path in paths:
                path.touch()

            baseline = payload_bytes(observer)
            with observer.cursor() as cursor:
                cursor.execute(
                    "SELECT value FROM fod.config WHERE key = 'max_fs_size_bytes'"
                )
                original_limit = int(cursor.fetchone()[0])
                cursor.execute(
                    """
                    UPDATE fod.config
                    SET value = %s
                    WHERE key = 'max_fs_size_bytes'
                    """,
                    (baseline + BLOCK_SIZE,),
                )

            with blocker.cursor() as cursor:
                cursor.execute(
                    "SELECT pg_advisory_xact_lock(%s, %s)", QUOTA_LOCK_KEY
                )

            barrier = threading.Barrier(2)
            results: list[int | BaseException | None] = [None, None]

            def run_writer(index: int) -> None:
                try:
                    results[index] = write_and_sync(
                        paths[index], bytes([65 + index]), barrier
                    )
                except BaseException as error:
                    results[index] = error

            threads = [
                threading.Thread(
                    target=run_writer,
                    args=(index,),
                    daemon=True,
                )
                for index in range(2)
            ]
            for thread in threads:
                thread.start()

            waiting = wait_for_advisory_waiters(observer, 2)
            blocker.commit()

            for thread in threads:
                thread.join(timeout=15)
            if any(thread.is_alive() for thread in threads):
                raise AssertionError("quota writer did not finish after releasing advisory lock")

            winners = [index for index, result in enumerate(results) if result is None]
            rejected = [
                index for index, result in enumerate(results) if result == errno.ENOSPC
            ]
            if len(winners) != 1 or len(rejected) != 1:
                launcher_a._dump_log()
                launcher_b._dump_log()
                raise AssertionError(
                    f"expected one success and one ENOSPC, got {results}"
                )

            after = payload_bytes(observer)
            if after != baseline + BLOCK_SIZE:
                raise AssertionError(
                    f"payload changed by {after - baseline}, expected {BLOCK_SIZE}"
                )

            states = file_storage_state(observer, names)
            expected_states = {
                names[winners[0]]: (BLOCK_SIZE, 1),
                names[rejected[0]]: (0, 0),
            }
            if states != expected_states:
                raise AssertionError(
                    f"unexpected winner/rejected storage state: {states}"
                )

            print(
                "OK two-mount quota "
                f"waiters={waiting} winner={names[winners[0]]} "
                f"rejected={names[rejected[0]]} payload_delta={after - baseline}"
            )
        finally:
            try:
                blocker.rollback()
            except Exception:
                pass
            for thread in threads:
                thread.join(timeout=5)
            for path in paths:
                try:
                    path.unlink()
                except OSError:
                    pass
            if original_limit is not None:
                with observer.cursor() as cursor:
                    cursor.execute(
                        """
                        UPDATE fod.config
                        SET value = %s
                        WHERE key = 'max_fs_size_bytes'
                        """,
                        (original_limit,),
                    )
            blocker.close()
            observer.close()
            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
