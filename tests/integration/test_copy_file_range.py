#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import errno
import os
import sys
import tempfile
import uuid
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def database_connection(launcher: FODMount):
    connection = psycopg2.connect(
        host=os.environ.get("POSTGRES_HOST", "127.0.0.1"),
        port=os.environ.get("POSTGRES_PORT", "5432"),
        dbname=launcher.postgres_db,
        user=launcher.postgres_user,
        password=launcher.postgres_password,
    )
    connection.autocommit = True
    return connection


def payload_usage_and_limit(connection):
    with connection.cursor() as cursor:
        cursor.execute(
            """
            SELECT
                (
                    (SELECT COUNT(*)::bigint FROM fod.data_blocks)
                        * (SELECT value FROM fod.config WHERE key = 'block_size')
                    + COALESCE((SELECT SUM(used_bytes)::bigint FROM fod.data_extents), 0)
                ),
                (SELECT value FROM fod.config WHERE key = 'max_fs_size_bytes')
            """
        )
        return cursor.fetchone()


def main():
    suffix = uuid.uuid4().hex[:8]
    src_payload = b"ABCDEFGHIJ"

    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-copy-{suffix}.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            dir_path = mountpoint / f"copy_{suffix}"
            src_path = dir_path / "source.txt"
            dst_path = dir_path / "dest.txt"

            dir_path.mkdir()
            src_path.write_bytes(src_payload)
            dst_path.write_bytes(b"")

            with src_path.open("rb") as src_fh, dst_path.open("r+b") as dst_fh:
                copied = os.copy_file_range(src_fh.fileno(), dst_fh.fileno(), 5, offset_src=2, offset_dst=4)
            assert copied == 5, copied

            assert dst_path.stat().st_size == 9, dst_path.stat()
            data = dst_path.read_bytes()
            assert data == b"\x00\x00\x00\x00CDEFG", data
            assert src_path.read_bytes() == src_payload, src_path.read_bytes()

            reserve_src_path = dir_path / "reserve-source.bin"
            reserve_dst_path = dir_path / "reserve-dest.bin"
            reserve_src_path.write_bytes(b"R" * 4096)
            reserve_dst_path.write_bytes(b"X")

            connection = database_connection(launcher)
            try:
                used_bytes, original_limit = payload_usage_and_limit(connection)
                with connection.cursor() as cursor:
                    cursor.execute(
                        "SELECT COUNT(*) FROM fod.payload_capacity_reservations"
                    )
                    reservations_before = cursor.fetchone()[0]
                    cursor.execute(
                        """
                        UPDATE fod.config
                        SET value = %s
                        WHERE key = 'max_fs_size_bytes'
                        """,
                        (used_bytes,),
                    )
                    cursor.execute(
                        """
                        SELECT value
                        FROM fod.config
                        WHERE key = 'max_fs_size_bytes'
                        """
                    )
                    assert cursor.fetchone()[0] == used_bytes

                try:
                    with (
                        reserve_src_path.open("rb") as src_fh,
                        reserve_dst_path.open("r+b") as dst_fh,
                    ):
                        try:
                            os.copy_file_range(
                                src_fh.fileno(),
                                dst_fh.fileno(),
                                4096,
                                offset_src=0,
                                offset_dst=1,
                            )
                        except OSError as error:
                            assert error.errno == errno.ENOSPC, error
                        else:
                            launcher._dump_log()
                            raise AssertionError(
                                "copy_file_range should fail before writing without reserved capacity"
                            )

                    assert reserve_dst_path.read_bytes() == b"X"
                    with connection.cursor() as cursor:
                        cursor.execute(
                            "SELECT COUNT(*) FROM fod.payload_capacity_reservations"
                        )
                        assert cursor.fetchone()[0] == reservations_before
                finally:
                    with connection.cursor() as cursor:
                        cursor.execute(
                            """
                            UPDATE fod.config
                            SET value = %s
                            WHERE key = 'max_fs_size_bytes'
                            """,
                            (original_limit,),
                        )
            finally:
                connection.close()
            print("OK copy_file_range")
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
