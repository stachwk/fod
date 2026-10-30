#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import errno
import os
import sys
import tempfile
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def assert_truncate_times_advanced(
    before: os.stat_result,
    after: os.stat_result,
    label: str,
) -> None:
    if after.st_mtime_ns <= before.st_mtime_ns:
        raise AssertionError(
            f"{label} did not advance mtime: "
            f"before={before.st_mtime_ns} after={after.st_mtime_ns}"
        )
    if after.st_ctime_ns <= before.st_ctime_ns:
        raise AssertionError(
            f"{label} did not advance ctime: "
            f"before={before.st_ctime_ns} after={after.st_ctime_ns}"
        )


def assert_shrink_extend_zero_fill(
    file_path: Path,
    payload: bytes,
    shrink_size: int,
    extend_size: int,
) -> None:
    file_path.write_bytes(payload)

    os.truncate(file_path, shrink_size)
    assert file_path.stat().st_size == shrink_size
    assert file_path.read_bytes() == payload[:shrink_size]

    os.truncate(file_path, extend_size)
    assert file_path.stat().st_size == extend_size

    expected = payload[:shrink_size] + (b"\x00" * (extend_size - shrink_size))
    actual = file_path.read_bytes()
    assert len(actual) == extend_size, len(actual)
    assert actual == expected, (
        shrink_size,
        extend_size,
        actual[shrink_size : min(extend_size, shrink_size + 64)],
    )


def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    block_size = 4096
    payload = (b"A" * block_size) + (b"B" * 1904)
    expected = (b"A" * block_size) + (b"\x00" * block_size)

    with tempfile.TemporaryDirectory(prefix="/tmp/fod-truncate-block.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            dir_path = mountpoint / f"truncate_block_{uuid.uuid4().hex[:8]}"
            file_path = dir_path / "payload.bin"

            dir_path.mkdir()
            file_path.write_bytes(payload)

            old_mtime_ns = (time.time_ns() - 60_000_000_000) // 1000 * 1000
            os.utime(file_path, ns=(old_mtime_ns, old_mtime_ns))
            before_truncate = file_path.stat()
            time.sleep(0.01)

            os.truncate(file_path, block_size)
            truncated_stat = file_path.stat()
            assert truncated_stat.st_size == block_size
            assert truncated_stat.st_mtime_ns > old_mtime_ns
            assert_truncate_times_advanced(
                before_truncate, truncated_stat, "path truncate shrink"
            )

            os.truncate(file_path, block_size * 2)
            assert file_path.stat().st_size == block_size * 2

            symlink_target = dir_path / "truncate-symlink-target.bin"
            symlink_path = dir_path / "truncate-symlink.bin"
            symlink_target.write_bytes(b"truncate follows symlink")
            symlink_path.symlink_to(symlink_target.name)
            os.truncate(symlink_path, 8)
            assert symlink_path.is_symlink()
            assert symlink_target.read_bytes() == b"truncate"

            for path, expected_errno, label in (
                (dir_path, errno.EISDIR, "directory"),
                (dir_path / "missing-truncate-target", errno.ENOENT, "missing path"),
            ):
                try:
                    os.truncate(path, 0)
                except OSError as error:
                    assert error.errno == expected_errno, (label, error.errno)
                else:
                    raise AssertionError(f"truncate on {label} succeeded")

            size_before_invalid_truncate = file_path.stat().st_size
            try:
                os.truncate(file_path, -1)
            except OSError as error:
                assert error.errno == errno.EINVAL, error.errno
            else:
                raise AssertionError("truncate with negative length succeeded")
            assert file_path.stat().st_size == size_before_invalid_truncate

            data = file_path.read_bytes()
            assert len(data) == block_size * 2, len(data)
            assert data == expected, data[block_size : block_size + 64]

            descriptor_path = dir_path / "ftruncate-descriptor.bin"
            descriptor_payload = b"ftruncate keeps the current offset"
            descriptor_path.write_bytes(descriptor_payload)
            fd = os.open(descriptor_path, os.O_RDWR)
            try:
                seek_offset = 12
                assert os.lseek(fd, seek_offset, os.SEEK_SET) == seek_offset
                before_shrink = os.fstat(fd)
                time.sleep(0.01)
                os.ftruncate(fd, 5)
                after_shrink = os.fstat(fd)
                assert_truncate_times_advanced(
                    before_shrink, after_shrink, "ftruncate shrink"
                )
                assert os.lseek(fd, 0, os.SEEK_CUR) == seek_offset
                assert after_shrink.st_size == 5
                assert os.read(fd, 1) == b""

                before_extend = os.fstat(fd)
                time.sleep(0.01)
                os.ftruncate(fd, 9)
                after_extend = os.fstat(fd)
                assert_truncate_times_advanced(
                    before_extend, after_extend, "ftruncate extend"
                )
                assert os.lseek(fd, 0, os.SEEK_CUR) == seek_offset
                assert after_extend.st_size == 9
                assert os.pread(fd, 9, 0) == descriptor_payload[:5] + (b"\x00" * 4)
            finally:
                os.close(fd)

            readonly_fd = os.open(descriptor_path, os.O_RDONLY)
            try:
                try:
                    os.ftruncate(readonly_fd, 0)
                except OSError as error:
                    assert error.errno in (errno.EBADF, errno.EINVAL), error.errno
                else:
                    raise AssertionError("ftruncate on read-only fd succeeded")
            finally:
                os.close(readonly_fd)
            assert descriptor_path.stat().st_size == 9

            cross_block_path = dir_path / "off-boundary-cross-block.bin"
            cross_block_payload = (
                (b"C" * block_size)
                + (b"D" * block_size)
                + (b"E" * block_size)
            )
            assert_shrink_extend_zero_fill(
                cross_block_path,
                cross_block_payload,
                block_size + 333,
                block_size * 2 + 123,
            )

            same_block_path = dir_path / "off-boundary-same-block.bin"
            same_block_payload = b"F" * block_size
            assert_shrink_extend_zero_fill(
                same_block_path,
                same_block_payload,
                333,
                777,
            )

            print("OK truncate shrink block boundary timestamp_update=1")
            print("OK truncate symlink and path errors EISDIR/ENOENT/EINVAL")
            print("OK truncate off-boundary extend zero fill")
            print("OK ftruncate descriptor offset and access mode")
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
