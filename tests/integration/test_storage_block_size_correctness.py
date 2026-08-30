#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import hashlib
import os
import sys
import tempfile
import threading
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def deterministic_bytes(length: int, label: str) -> bytes:
    chunks: list[bytes] = []
    remaining = length
    counter = 0
    while remaining > 0:
        chunk = hashlib.sha256(f"{label}:{counter}".encode("utf-8")).digest()
        take = min(remaining, len(chunk))
        chunks.append(chunk[:take])
        remaining -= take
        counter += 1
    return b"".join(chunks)


def write_fsync(path: Path, payload: bytes) -> None:
    with path.open("wb") as handle:
        handle.write(payload)
        handle.flush()
        os.fsync(handle.fileno())


def pwrite_fsync(path: Path, payload: bytes, offset: int) -> None:
    fd = os.open(path, os.O_RDWR)
    try:
        written = os.pwrite(fd, payload, offset)
        assert written == len(payload), (written, len(payload), offset)
        os.fsync(fd)
    finally:
        os.close(fd)


def assert_file(path: Path, expected: bytes, label: str) -> None:
    actual = path.read_bytes()
    assert actual == expected, (
        label,
        len(actual),
        len(expected),
        hashlib.sha256(actual).hexdigest(),
        hashlib.sha256(expected).hexdigest(),
    )
    assert path.stat().st_size == len(expected), (label, path.stat().st_size, len(expected))


def database_connection() -> psycopg2.extensions.connection:
    connection = psycopg2.connect(
        host=os.environ.get("FOD_PG_HOST", "127.0.0.1"),
        port=os.environ.get("FOD_PG_PORT", "5432"),
        dbname=os.environ.get("POSTGRES_DB", "foddbname"),
        user=os.environ.get("POSTGRES_USER", "foduser"),
        password=os.environ.get("POSTGRES_PASSWORD", "cichosza"),
    )
    connection.autocommit = True
    return connection


def copy_range_all(src_path: Path, dst_path: Path, *, src_offset: int, dst_offset: int, count: int, chunk_size: int) -> None:
    with src_path.open("rb") as src_handle, dst_path.open("r+b") as dst_handle:
        copied_total = 0
        while copied_total < count:
            requested = min(chunk_size, count - copied_total)
            copied = os.copy_file_range(
                src_handle.fileno(),
                dst_handle.fileno(),
                requested,
                offset_src=src_offset + copied_total,
                offset_dst=dst_offset + copied_total,
            )
            assert copied > 0, (copied_total, requested, count)
            copied_total += copied
        os.fsync(dst_handle.fileno())


def concurrent_disjoint_writes(path: Path, block_size: int) -> bytes:
    total_size = max(block_size * 2, 16 * 1024)
    expected = bytearray(total_size)

    if block_size >= 16 * 1024:
        offsets = (1024, block_size // 2)
    else:
        offsets = (512, block_size + 512)

    payloads = (
        deterministic_bytes(2048, f"concurrent-a-{block_size}"),
        deterministic_bytes(2048, f"concurrent-b-{block_size}"),
    )
    for offset, payload in zip(offsets, payloads):
        expected[offset : offset + len(payload)] = payload

    write_fsync(path, bytes(total_size))

    barrier = threading.Barrier(3)
    errors: list[BaseException] = []

    def writer(offset: int, payload: bytes) -> None:
        try:
            fd = os.open(path, os.O_RDWR)
            try:
                barrier.wait(timeout=10)
                written = os.pwrite(fd, payload, offset)
                if written != len(payload):
                    raise AssertionError((written, len(payload), offset))
                os.fsync(fd)
            finally:
                os.close(fd)
        except BaseException as exc:  # noqa: BLE001 - test must surface worker failure
            errors.append(exc)

    threads = [
        threading.Thread(target=writer, args=(offsets[index], payloads[index]), daemon=True)
        for index in range(2)
    ]
    for thread in threads:
        thread.start()
    barrier.wait(timeout=10)
    for thread in threads:
        thread.join(timeout=20)
        assert not thread.is_alive(), "concurrent writer did not finish"
    if errors:
        raise errors[0]

    return bytes(expected)


def exercise_primary_mount(mountpoint: Path, block_size: int) -> dict[str, bytes]:
    root = mountpoint / f"storage-block-{block_size}"
    root.mkdir()
    expected: dict[str, bytes] = {}

    partial_path = root / "partial.bin"
    partial = bytearray(deterministic_bytes(block_size * 3 + 777, f"partial-base-{block_size}"))
    write_fsync(partial_path, bytes(partial))

    one_byte_offset = block_size // 3 + 17
    one_byte = b"Z"
    pwrite_fsync(partial_path, one_byte, one_byte_offset)
    partial[one_byte_offset : one_byte_offset + 1] = one_byte
    assert_file(partial_path, bytes(partial), "single-byte-write")
    print("CASE single_byte_write=OK")

    unaligned_offset = block_size // 4 + 73
    unaligned_payload = deterministic_bytes(4096, f"unaligned-4k-{block_size}")
    pwrite_fsync(partial_path, unaligned_payload, unaligned_offset)
    partial[unaligned_offset : unaligned_offset + len(unaligned_payload)] = unaligned_payload

    cross_offset = block_size - 2048 + 37
    cross_payload = deterministic_bytes(4096, f"cross-block-4k-{block_size}")
    pwrite_fsync(partial_path, cross_payload, cross_offset)
    partial[cross_offset : cross_offset + len(cross_payload)] = cross_payload

    append_payload = deterministic_bytes(777, f"append-{block_size}")
    with partial_path.open("ab") as handle:
        handle.write(append_payload)
        handle.flush()
        os.fsync(handle.fileno())
    partial.extend(append_payload)
    assert_file(partial_path, bytes(partial), "partial-and-append")

    fd = os.open(partial_path, os.O_RDONLY)
    try:
        for offset in (0, 113, max(0, block_size - 1024), block_size + 17):
            expected_slice = bytes(partial[offset : offset + 4096])
            actual_slice = os.pread(fd, 4096, offset)
            assert actual_slice == expected_slice, ("4k-read", offset, len(actual_slice), len(expected_slice))
    finally:
        os.close(fd)
    print("CASE unaligned_partial_append_4k_reads=OK")
    expected[str(partial_path.relative_to(mountpoint))] = bytes(partial)

    truncate_path = root / "truncate.bin"
    truncate_source = deterministic_bytes(block_size * 3 + 777, f"truncate-{block_size}")
    write_fsync(truncate_path, truncate_source)
    os.truncate(truncate_path, block_size * 2)
    exact_expected = truncate_source[: block_size * 2]
    assert_file(truncate_path, exact_expected, "truncate-exact-boundary")

    write_fsync(truncate_path, truncate_source)
    off_boundary = block_size + 333
    os.truncate(truncate_path, off_boundary)
    off_expected = truncate_source[:off_boundary]
    assert_file(truncate_path, off_expected, "truncate-off-boundary")

    extended_size = block_size * 2 + 123
    os.truncate(truncate_path, extended_size)
    truncate_expected = off_expected + bytes(extended_size - len(off_expected))
    assert_file(truncate_path, truncate_expected, "truncate-extend-zero-fill")
    expected[str(truncate_path.relative_to(mountpoint))] = truncate_expected
    print("CASE truncate_exact_off_boundary_extend=OK")

    sparse_path = root / "sparse.bin"
    head = deterministic_bytes(1024, f"sparse-head-{block_size}")
    tail = deterministic_bytes(1536, f"sparse-tail-{block_size}")
    tail_offset = block_size * 2 + 1234
    write_fsync(sparse_path, head)
    pwrite_fsync(sparse_path, tail, tail_offset)
    sparse_expected = head + bytes(tail_offset - len(head)) + tail
    assert_file(sparse_path, sparse_expected, "sparse")
    expected[str(sparse_path.relative_to(mountpoint))] = sparse_expected
    print("CASE sparse_hole_zero_fill=OK")

    fallocate_path = root / "fallocate.bin"
    write_fsync(fallocate_path, b"fod")
    if not hasattr(os, "posix_fallocate"):
        raise AssertionError("os.posix_fallocate is not available")
    fallocate_offset = block_size // 2 + 7
    fallocate_length = block_size + 333
    with fallocate_path.open("r+b") as handle:
        os.posix_fallocate(handle.fileno(), fallocate_offset, fallocate_length)
        os.fsync(handle.fileno())
    fallocate_size = fallocate_offset + fallocate_length
    fallocate_expected = b"fod" + bytes(fallocate_size - 3)
    assert_file(fallocate_path, fallocate_expected, "fallocate")
    expected[str(fallocate_path.relative_to(mountpoint))] = fallocate_expected
    print("CASE fallocate_zero_fill=OK")

    copy_src_path = root / "copy-source.bin"
    copy_dst_path = root / "copy-dest.bin"
    copy_source = deterministic_bytes(block_size * 2 + 7777, f"copy-source-{block_size}")
    write_fsync(copy_src_path, copy_source)
    write_fsync(copy_dst_path, b"HEAD")
    src_offset = 37
    dst_offset = 123
    copy_count = len(copy_source) - 137
    copy_range_all(
        copy_src_path,
        copy_dst_path,
        src_offset=src_offset,
        dst_offset=dst_offset,
        count=copy_count,
        chunk_size=max(4096, block_size // 2 + 777),
    )
    copy_expected = b"HEAD" + bytes(dst_offset - 4) + copy_source[src_offset : src_offset + copy_count]
    assert_file(copy_dst_path, copy_expected, "copy-file-range")
    expected[str(copy_src_path.relative_to(mountpoint))] = copy_source
    expected[str(copy_dst_path.relative_to(mountpoint))] = copy_expected
    print("CASE copy_file_range_unaligned=OK")

    concurrent_path = root / "concurrent-partial.bin"
    concurrent_expected = concurrent_disjoint_writes(concurrent_path, block_size)
    assert_file(concurrent_path, concurrent_expected, "concurrent-partial")
    expected[str(concurrent_path.relative_to(mountpoint))] = concurrent_expected
    print("CASE concurrent_disjoint_partial_writes=OK")

    return expected


def verify_after_remount(mountpoint: Path, expected: dict[str, bytes], label: str) -> None:
    for relative, payload in expected.items():
        assert_file(mountpoint / relative, payload, f"{label}:{relative}")
    print(f"CASE {label}=OK")


def exercise_dedupe_crc(mountpoint: Path, block_size: int) -> tuple[str, bytes]:
    root = mountpoint / f"storage-block-{block_size}"
    src_path = root / "dedupe-source.bin"
    dst_path = root / "dedupe-dest.bin"
    payload = deterministic_bytes(block_size * 3 + 321, f"dedupe-{block_size}")
    write_fsync(src_path, payload)
    write_fsync(dst_path, b"")

    copy_range_all(
        src_path,
        dst_path,
        src_offset=0,
        dst_offset=0,
        count=len(payload),
        chunk_size=max(4096, block_size),
    )
    assert_file(dst_path, payload, "dedupe-first-copy")

    copy_range_all(
        src_path,
        dst_path,
        src_offset=0,
        dst_offset=0,
        count=len(payload),
        chunk_size=max(4096, block_size),
    )
    assert_file(dst_path, payload, "dedupe-identical-copy")

    connection = database_connection()
    try:
        with connection.cursor() as cursor:
            cursor.execute("SELECT COUNT(*) FROM fod.copy_block_crc")
            crc_rows = int(cursor.fetchone()[0])
    finally:
        connection.close()
    assert crc_rows > 0, ("copy_block_crc empty", block_size)
    print(f"CASE copy_dedupe_crc=OK crc_rows={crc_rows}")
    return str(dst_path.relative_to(mountpoint)), payload


def main() -> None:
    block_size = int(os.environ.get("FOD_TEST_STORAGE_BLOCK_SIZE", "0"))
    if block_size < 1024 or block_size % 1024 != 0:
        raise SystemExit(f"FOD_TEST_STORAGE_BLOCK_SIZE must be a positive multiple of 1024, got {block_size}")

    launcher = FODMount(str(ROOT), role="primary")
    launcher.init_schema()

    expected: dict[str, bytes]
    with tempfile.TemporaryDirectory(dir="/tmp", prefix=f"fod-storage-block-{block_size}-first.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            expected = exercise_primary_mount(mountpoint, block_size)
        except Exception:
            launcher._dump_log()
            raise
        finally:
            launcher.stop()

    with tempfile.TemporaryDirectory(dir="/tmp", prefix=f"fod-storage-block-{block_size}-remount.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            verify_after_remount(mountpoint, expected, "remount_durability")
        except Exception:
            launcher._dump_log()
            raise
        finally:
            launcher.stop()

    old_env = {
        key: os.environ.get(key)
        for key in (
            "FOD_COPY_DEDUPE_ENABLED",
            "FOD_COPY_DEDUPE_CRC_TABLE",
            "FOD_COPY_DEDUPE_MIN_BLOCKS",
            "FOD_COPY_DEDUPE_MAX_BLOCKS",
        )
    }
    os.environ["FOD_COPY_DEDUPE_ENABLED"] = "1"
    os.environ["FOD_COPY_DEDUPE_CRC_TABLE"] = "1"
    os.environ["FOD_COPY_DEDUPE_MIN_BLOCKS"] = "1"
    os.environ["FOD_COPY_DEDUPE_MAX_BLOCKS"] = "0"
    dedupe_relative = ""
    dedupe_expected = b""
    try:
        with tempfile.TemporaryDirectory(dir="/tmp", prefix=f"fod-storage-block-{block_size}-dedupe.") as tmpdir:
            mountpoint = Path(tmpdir)
            launcher.start(str(mountpoint))
            try:
                verify_after_remount(mountpoint, expected, "dedupe_mount_existing_data")
                dedupe_relative, dedupe_expected = exercise_dedupe_crc(mountpoint, block_size)
            except Exception:
                launcher._dump_log()
                raise
            finally:
                launcher.stop()

        with tempfile.TemporaryDirectory(dir="/tmp", prefix=f"fod-storage-block-{block_size}-dedupe-remount.") as tmpdir:
            mountpoint = Path(tmpdir)
            launcher.start(str(mountpoint))
            try:
                assert_file(mountpoint / dedupe_relative, dedupe_expected, "dedupe-remount")
                print("CASE copy_dedupe_crc_remount=OK")
            except Exception:
                launcher._dump_log()
                raise
            finally:
                launcher.stop()
    finally:
        for key, value in old_env.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value

    print(f"OK storage block size correctness block_size={block_size}")


if __name__ == "__main__":
    main()
