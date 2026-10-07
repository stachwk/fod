#!/usr/bin/env python
# -*- coding: utf-8 -*-
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


def storage_snapshot(connection) -> tuple[int, int]:
    # Quota jest liczona w skonfigurowanych blokach FOD. Test nie moze
    # zakladac historycznego 4 KiB, bo produkcyjny block_size moze byc inny.
    with connection.cursor() as cursor:
        cursor.execute(
            """
            SELECT
                (SELECT COUNT(*)::bigint FROM fod.data_blocks)
                    * (SELECT value FROM fod.config WHERE key = 'block_size'),
                (SELECT value FROM fod.config WHERE key = 'block_size')
            """
        )
        payload_bytes, block_size = cursor.fetchone()
        return int(payload_bytes), int(block_size)


def write_and_sync(
    path: Path,
    marker: bytes,
    write_size: int,
    barrier: threading.Barrier,
) -> int | None:
    descriptor = os.open(path, os.O_WRONLY)
    error_number = None
    try:
        barrier.wait()
        written = os.write(descriptor, marker * write_size)
        if written != write_size:
            raise AssertionError(
                f"short write for {path}: expected={write_size} actual={written}"
            )
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

            baseline, block_size = storage_snapshot(observer)
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
                    (baseline + block_size,),
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
                        paths[index],
                        bytes([65 + index]),
                        block_size,
                        barrier,
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

            after, observed_block_size = storage_snapshot(observer)
            if observed_block_size != block_size:
                raise AssertionError(
                    f"block_size changed during test: before={block_size} after={observed_block_size}"
                )
            if after != baseline + block_size:
                raise AssertionError(
                    f"payload changed by {after - baseline}, expected {block_size}"
                )

            states = file_storage_state(observer, names)
            expected_states = {
                names[winners[0]]: (block_size, 1),
                names[rejected[0]]: (0, 0),
            }
            if states != expected_states:
                raise AssertionError(
                    f"unexpected winner/rejected storage state: {states}"
                )

            print(
                "OK two-mount quota "
                f"waiters={waiting} winner={names[winners[0]]} "
                f"rejected={names[rejected[0]]} payload_delta={after - baseline} "
                f"block_size={block_size}"
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
