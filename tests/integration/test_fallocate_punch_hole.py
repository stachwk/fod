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
from tests.integration.test_fallocate_contract import (
    FALLOC_FL_KEEP_SIZE,
    FALLOC_FL_PUNCH_HOLE,
    db_snapshot,
    file_snapshot,
    raw_fallocate,
)


def connect(launcher: FODMount):
    connection = psycopg2.connect(
        host=os.environ.get("POSTGRES_HOST", "127.0.0.1"),
        port=os.environ.get("POSTGRES_PORT", "5432"),
        dbname=launcher.postgres_db,
        user=launcher.postgres_user,
        password=launcher.postgres_password,
    )
    connection.autocommit = True
    return connection


def configured_block_size(connection) -> int:
    with connection.cursor() as cursor:
        cursor.execute("SELECT value FROM fod.config WHERE key = 'block_size'")
        row = cursor.fetchone()
    if row is None:
        raise AssertionError("missing fod.config block_size")
    value = int(row[0])
    if value <= 0:
        raise AssertionError(f"invalid FOD block_size={value}")
    return value


def zero_range(data: bytes, start: int, length: int) -> bytes:
    output = bytearray(data)
    end = min(len(output), start + length)
    if start < end:
        output[start:end] = b"\x00" * (end - start)
    return bytes(output)


def main() -> int:
    template = FODMount(str(ROOT))
    template.init_schema()
    suffix = uuid.uuid4().hex[:12]
    name = f"fallocate-punch-{suffix}.bin"
    link_name = f"fallocate-punch-link-{suffix}.bin"

    launcher = FODMount(str(ROOT))
    launcher.postgres_db = template.postgres_db
    launcher.postgres_user = template.postgres_user
    launcher.postgres_password = template.postgres_password

    expected = b""
    final_db = None
    block_size = 0

    with tempfile.TemporaryDirectory(prefix="/tmp/fod-fallocate-punch.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(tmpdir, log_prefix="/tmp/fod-fallocate-punch")
        try:
            connection = connect(launcher)
            try:
                block_size = configured_block_size(connection)
                path = mountpoint / name
                link_path = mountpoint / link_name
                initial = bytes(((index * 29) + 11) % 251 for index in range(block_size * 4))

                fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_TRUNC, 0o644)
                try:
                    written = os.write(fd, initial)
                    if written != len(initial):
                        raise AssertionError(f"short initial write {written}/{len(initial)}")
                    os.fsync(fd)
                finally:
                    os.close(fd)

                os.link(path, link_path)
                before = file_snapshot(path)
                before_db = db_snapshot(connection, name)

                fd = os.open(path, os.O_RDWR)
                try:
                    result = raw_fallocate(
                        fd,
                        FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
                        block_size,
                        block_size,
                    )
                    if result.rc != 0:
                        raise AssertionError(f"aligned punch failed: {result}")
                    os.fsync(fd)
                finally:
                    os.close(fd)

                expected = zero_range(initial, block_size, block_size)
                if path.read_bytes() != expected or link_path.read_bytes() != expected:
                    raise AssertionError("aligned punch produced incorrect hardlink-visible contents")
                aligned = file_snapshot(path)
                aligned_db = db_snapshot(connection, name)
                if aligned.size != before.size:
                    raise AssertionError("aligned punch changed file size")
                if aligned_db.payload_bytes != before_db.payload_bytes - block_size:
                    raise AssertionError("aligned punch did not release one storage block")
                if aligned_db.block_rows != before_db.block_rows - 1:
                    raise AssertionError("aligned punch did not remove one data_blocks row")
                if aligned.blocks_512 >= before.blocks_512:
                    raise AssertionError("aligned punch did not reduce st_blocks")

                partial_start = (2 * block_size) + (block_size // 4)
                partial_len = block_size // 2
                fd = os.open(path, os.O_RDWR)
                try:
                    result = raw_fallocate(
                        fd,
                        FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
                        partial_start,
                        partial_len,
                    )
                    if result.rc != 0:
                        raise AssertionError(f"partial punch failed: {result}")
                    os.fsync(fd)
                finally:
                    os.close(fd)

                expected = zero_range(expected, partial_start, partial_len)
                if path.read_bytes() != expected:
                    raise AssertionError("partial punch corrupted boundary bytes")
                partial_db = db_snapshot(connection, name)
                if partial_db.block_rows != aligned_db.block_rows:
                    raise AssertionError("partial punch unexpectedly removed boundary block")
                if partial_db.payload_bytes != aligned_db.payload_bytes:
                    raise AssertionError("partial punch unexpectedly changed allocated block bytes")

                fd = os.open(path, os.O_RDWR)
                try:
                    result = raw_fallocate(
                        fd,
                        FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
                        block_size,
                        block_size,
                    )
                    if result.rc != 0:
                        raise AssertionError(f"idempotent sparse punch failed: {result}")
                    os.fsync(fd)
                finally:
                    os.close(fd)
                sparse_again = db_snapshot(connection, name)
                if sparse_again.block_rows != partial_db.block_rows or sparse_again.payload_bytes != partial_db.payload_bytes:
                    raise AssertionError("re-punching sparse range changed payload accounting")

                beyond_before = file_snapshot(path)
                beyond_db_before = db_snapshot(connection, name)
                fd = os.open(path, os.O_RDWR)
                try:
                    result = raw_fallocate(
                        fd,
                        FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
                        len(expected) + block_size,
                        block_size,
                    )
                    if result.rc != 0:
                        raise AssertionError(f"beyond-EOF punch failed: {result}")
                finally:
                    os.close(fd)
                beyond_after = file_snapshot(path)
                beyond_db_after = db_snapshot(connection, name)
                if (beyond_after.size, beyond_after.blocks_512, beyond_after.sha256) != (
                    beyond_before.size,
                    beyond_before.blocks_512,
                    beyond_before.sha256,
                ):
                    raise AssertionError("beyond-EOF punch mutated file state")
                if beyond_db_after != beyond_db_before:
                    raise AssertionError("beyond-EOF punch mutated database state")

                fd = os.open(path, os.O_RDWR)
                try:
                    unsupported = raw_fallocate(fd, 0, 0, block_size)
                finally:
                    os.close(fd)
                if unsupported.rc == 0 or unsupported.errno not in {errno.ENOTSUP, errno.EOPNOTSUPP}:
                    raise AssertionError(f"mode=0 must remain unsupported, got {unsupported}")

                if os.stat(path).st_nlink != 2 or os.stat(link_path).st_nlink != 2:
                    raise AssertionError("hardlink count changed during punch")
                final_db = db_snapshot(connection, name)
            finally:
                connection.close()
        finally:
            launcher.stop()

    remount = FODMount(str(ROOT))
    remount.postgres_db = template.postgres_db
    remount.postgres_user = template.postgres_user
    remount.postgres_password = template.postgres_password
    with tempfile.TemporaryDirectory(prefix="/tmp/fod-fallocate-punch-remount.") as tmpdir:
        mountpoint = Path(tmpdir)
        remount.start(tmpdir, log_prefix="/tmp/fod-fallocate-punch-remount")
        try:
            connection = connect(remount)
            try:
                path = mountpoint / name
                link_path = mountpoint / link_name
                if path.read_bytes() != expected or link_path.read_bytes() != expected:
                    raise AssertionError("punch contents changed after remount")
                if db_snapshot(connection, name) != final_db:
                    raise AssertionError("database state changed after remount")
                link_path.unlink()
                path.unlink()
            finally:
                connection.close()
        finally:
            remount.stop()

    print(
        "OK fallocate-punch-hole "
        f"block_size={block_size} aligned=1 partial=1 sparse_idempotent=1 "
        "beyond_eof_noop=1 hardlink=1 unsupported_mode0=1 remount=1"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
