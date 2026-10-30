#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import errno
import fcntl
import os
import struct
import stat
import tempfile
import time
import uuid
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    suffix = uuid.uuid4().hex[:8]
    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-lseek-{suffix}.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            dir_path = mountpoint / f"lseek_{suffix}"
            file_path = dir_path / "payload.txt"
            payload = b"seekable payload"

            dir_path.mkdir()
            file_path.write_bytes(payload)

            overwrite_path = dir_path / "seek-overwrite.txt"
            original = b"0123456789abcdef"
            overwrite_at = 5
            replacement = b"XYZ"
            overwrite_path.write_bytes(original)
            fd = os.open(overwrite_path, os.O_RDWR)
            try:
                read_offset = os.lseek(fd, overwrite_at, os.SEEK_SET)
                assert read_offset == overwrite_at, read_offset
                before = os.read(fd, len(replacement))
                expected_before = original[overwrite_at : overwrite_at + len(replacement)]
                assert before == expected_before, before

                write_offset = os.lseek(fd, overwrite_at, os.SEEK_SET)
                assert write_offset == overwrite_at, write_offset
                written = os.write(fd, replacement)
                assert written == len(replacement), written
            finally:
                os.close(fd)

            expected = (
                original[:overwrite_at]
                + replacement
                + original[overwrite_at + len(replacement) :]
            )
            actual = overwrite_path.read_bytes()
            assert actual == expected, (actual, expected)

            append_path = dir_path / "seek-append.txt"
            append_path.write_bytes(b"prefix")
            fd = os.open(append_path, os.O_WRONLY | os.O_APPEND)
            try:
                assert fcntl.fcntl(fd, fcntl.F_GETFL) & os.O_APPEND
                assert fcntl.fcntl(fd, fcntl.F_GETFD) & fcntl.FD_CLOEXEC
                assert os.lseek(fd, 0, os.SEEK_SET) == 0
                assert os.write(fd, b"-one") == 4
                expected_append = b"prefix-one"
                assert os.lseek(fd, 1, os.SEEK_SET) == 1
                assert os.write(fd, b"-two") == 4
                expected_append += b"-two"
                assert os.lseek(fd, 0, os.SEEK_CUR) == len(expected_append)
            finally:
                os.close(fd)
            actual_append = append_path.read_bytes()
            assert actual_append == expected_append, (
                actual_append,
                expected_append,
            )

            positioned_io_path = dir_path / "positioned-io.txt"
            positioned_io_path.write_bytes(b"0123456789")
            fd = os.open(positioned_io_path, os.O_RDWR)
            try:
                assert os.lseek(fd, 3, os.SEEK_SET) == 3
                assert os.read(fd, 0) == b""
                assert os.lseek(fd, 0, os.SEEK_CUR) == 3
                assert os.pread(fd, 3, 6) == b"678"
                assert os.lseek(fd, 0, os.SEEK_CUR) == 3
                assert os.pwrite(fd, b"XY", 1) == 2
                assert os.lseek(fd, 0, os.SEEK_CUR) == 3

                os.fchmod(fd, 0o640)
                assert stat.S_IMODE(os.fstat(fd).st_mode) == 0o640
                atime_ns = (time.time_ns() - 31_000_000_000) // 1000 * 1000
                mtime_ns = (time.time_ns() - 30_000_000_000) // 1000 * 1000
                os.utime(fd, ns=(atime_ns, mtime_ns))
                descriptor_stat = os.fstat(fd)
                assert descriptor_stat.st_atime_ns == atime_ns
                assert descriptor_stat.st_mtime_ns == mtime_ns
            finally:
                os.close(fd)
            expected_positioned_io = b"0XY3456789"
            actual_positioned_io = positioned_io_path.read_bytes()
            assert actual_positioned_io == expected_positioned_io, (
                actual_positioned_io,
                expected_positioned_io,
            )

            descriptor_metadata_path = dir_path / "invalid-descriptor-metadata.txt"
            descriptor_metadata_path.write_bytes(b"unchanged")
            descriptor_metadata_before = descriptor_metadata_path.stat()
            closed_fd = os.open(descriptor_metadata_path, os.O_RDONLY)
            os.close(closed_fd)
            for operation in (
                lambda: os.fstat(closed_fd),
                lambda: os.fchmod(closed_fd, 0o600),
                lambda: os.fchown(closed_fd, -1, -1),
                lambda: os.utime(
                    closed_fd,
                    ns=(
                        descriptor_metadata_before.st_atime_ns,
                        descriptor_metadata_before.st_mtime_ns,
                    ),
                ),
            ):
                try:
                    operation()
                except OSError as error:
                    assert error.errno == errno.EBADF, error.errno
                else:
                    raise AssertionError("descriptor metadata accepted a closed fd")
            descriptor_metadata_after = descriptor_metadata_path.stat()
            assert stat.S_IMODE(descriptor_metadata_after.st_mode) == stat.S_IMODE(
                descriptor_metadata_before.st_mode
            )
            assert (descriptor_metadata_after.st_uid, descriptor_metadata_after.st_gid) == (
                descriptor_metadata_before.st_uid,
                descriptor_metadata_before.st_gid,
            )
            assert (
                descriptor_metadata_after.st_atime_ns,
                descriptor_metadata_after.st_mtime_ns,
                descriptor_metadata_after.st_ctime_ns,
            ) == (
                descriptor_metadata_before.st_atime_ns,
                descriptor_metadata_before.st_mtime_ns,
                descriptor_metadata_before.st_ctime_ns,
            )
            assert descriptor_metadata_path.read_bytes() == b"unchanged"
            print("OK fstat/fchmod/fchown/futimens closed descriptor errno=EBADF unchanged=1")

            negative_positioned_io_path = dir_path / "negative-positioned-io.txt"
            negative_positioned_io_path.write_bytes(b"unchanged")
            fd = os.open(negative_positioned_io_path, os.O_RDWR)
            try:
                assert os.lseek(fd, 2, os.SEEK_SET) == 2
                for operation in (
                    lambda: os.pread(fd, 1, -1),
                    lambda: os.pwrite(fd, b"X", -1),
                ):
                    try:
                        operation()
                    except OSError as error:
                        assert error.errno == errno.EINVAL, error.errno
                    else:
                        raise AssertionError(
                            "positioned I/O accepted a negative offset"
                        )
                    assert os.lseek(fd, 0, os.SEEK_CUR) == 2
            finally:
                os.close(fd)
            assert negative_positioned_io_path.read_bytes() == b"unchanged"

            off_t_overflow_path = dir_path / "off-t-overflow-pwrite.txt"
            off_t_overflow_path.write_bytes(b"unchanged")
            fd = os.open(off_t_overflow_path, os.O_RDWR)
            try:
                assert os.lseek(fd, 2, os.SEEK_SET) == 2
                off_t_max = (1 << (8 * struct.calcsize("P") - 1)) - 1
                try:
                    os.pwrite(fd, b"XY", off_t_max)
                except OSError as error:
                    # Linux rw_verify_area() rejects pos + count signed
                    # overflow before dispatching the operation to FUSE.
                    assert error.errno == errno.EINVAL, error.errno
                else:
                    raise AssertionError("pwrite beyond signed off_t succeeded")
                assert os.lseek(fd, 0, os.SEEK_CUR) == 2
            finally:
                os.close(fd)
            assert off_t_overflow_path.read_bytes() == b"unchanged"
            print("OK pwrite signed off_t overflow errno=EINVAL unchanged=1")

            fd = os.open(positioned_io_path, os.O_WRONLY)
            try:
                for operation in (
                    lambda: os.read(fd, 1),
                    lambda: os.pread(fd, 1, 0),
                ):
                    try:
                        operation()
                    except OSError as error:
                        assert error.errno == errno.EBADF, error.errno
                    else:
                        raise AssertionError("read from write-only fd succeeded")
                    assert os.lseek(fd, 0, os.SEEK_CUR) == 0
            finally:
                os.close(fd)

            fd = os.open(positioned_io_path, os.O_RDONLY)
            try:
                for operation in (
                    lambda: os.write(fd, b"X"),
                    lambda: os.pwrite(fd, b"X", 0),
                ):
                    try:
                        operation()
                    except OSError as error:
                        assert error.errno == errno.EBADF, error.errno
                    else:
                        raise AssertionError("write to read-only fd succeeded")
                    assert os.lseek(fd, 0, os.SEEK_CUR) == 0
            finally:
                os.close(fd)
            assert positioned_io_path.read_bytes() == expected_positioned_io

            short_read_path = dir_path / "short-read-eof.txt"
            short_read_payload = b"read through eof"
            short_read_path.write_bytes(short_read_payload)
            fd = os.open(short_read_path, os.O_RDONLY)
            try:
                assert os.lseek(fd, len(short_read_payload) - 2, os.SEEK_SET) == len(
                    short_read_payload
                ) - 2
                assert os.read(fd, 8) == short_read_payload[-2:]
                eof_offset = len(short_read_payload)
                assert os.lseek(fd, 0, os.SEEK_CUR) == eof_offset
                assert os.read(fd, 1) == b""
                assert os.lseek(fd, 0, os.SEEK_CUR) == eof_offset
                assert os.pread(fd, 4, eof_offset - 1) == short_read_payload[-1:]
                assert os.lseek(fd, 0, os.SEEK_CUR) == eof_offset
            finally:
                os.close(fd)

            zero_write_path = dir_path / "zero-write.txt"
            zero_write_path.write_bytes(b"unchanged")
            fd = os.open(zero_write_path, os.O_RDWR)
            try:
                zero_write_offset = len(b"unchanged") + 7
                assert os.lseek(fd, zero_write_offset, os.SEEK_SET) == zero_write_offset
                assert os.write(fd, b"") == 0
                assert os.lseek(fd, 0, os.SEEK_CUR) == zero_write_offset
                assert os.fstat(fd).st_size == len(b"unchanged")
            finally:
                os.close(fd)
            assert zero_write_path.read_bytes() == b"unchanged"

            beyond_eof_path = dir_path / "seek-beyond-eof.txt"
            initial = b"abc"
            beyond_eof_path.write_bytes(initial)
            fd = os.open(beyond_eof_path, os.O_RDWR)
            try:
                beyond_eof = len(initial) + 5
                result = os.lseek(fd, beyond_eof, os.SEEK_SET)
                assert result == beyond_eof, result
                assert os.fstat(fd).st_size == len(initial)
                assert os.write(fd, b"Z") == 1
            finally:
                os.close(fd)
            expected_beyond_eof = initial + (b"\0" * 5) + b"Z"
            assert beyond_eof_path.read_bytes() == expected_beyond_eof

            pwrite_beyond_eof_path = dir_path / "pwrite-beyond-eof.txt"
            pwrite_beyond_eof_path.write_bytes(initial)
            fd = os.open(pwrite_beyond_eof_path, os.O_RDWR)
            try:
                assert os.lseek(fd, 1, os.SEEK_SET) == 1
                assert os.pwrite(fd, b"Q", len(initial) + 4) == 1
                assert os.lseek(fd, 0, os.SEEK_CUR) == 1
            finally:
                os.close(fd)
            assert pwrite_beyond_eof_path.read_bytes() == initial + (b"\0" * 4) + b"Q"

            fd = os.open(file_path, os.O_RDONLY)
            try:
                os.lseek(fd, 0, os.SEEK_SET)
                try:
                    os.lseek(fd, -1, os.SEEK_CUR)
                except OSError as error:
                    assert error.errno == errno.EINVAL, error.errno
                else:
                    raise AssertionError("negative resulting offset was accepted")
                assert os.read(fd, 1) == payload[:1]
            finally:
                os.close(fd)

            fd = os.open(file_path, os.O_RDONLY)
            try:
                end_offset = os.lseek(fd, 0, os.SEEK_END)
                assert end_offset == len(payload), end_offset

                current_offset = os.lseek(fd, -1, os.SEEK_CUR)
                assert current_offset == len(payload) - 1, current_offset

                reset_offset = os.lseek(fd, 0, os.SEEK_SET)
                assert reset_offset == 0, reset_offset
            finally:
                os.close(fd)

            dup_path = dir_path / "dup-shared-offset.txt"
            dup_payload = b"shared offset"
            dup_path.write_bytes(dup_payload)
            fd = os.open(dup_path, os.O_RDONLY)
            dup_fd = os.dup(fd)
            try:
                assert os.lseek(fd, 2, os.SEEK_SET) == 2
                assert os.read(fd, 1) == dup_payload[2:3]
                assert os.read(dup_fd, 1) == dup_payload[3:4]
                assert os.lseek(dup_fd, 5, os.SEEK_SET) == 5
                assert os.read(fd, 1) == dup_payload[5:6]
            finally:
                os.close(dup_fd)
                os.close(fd)

            print(
                "OK lseek/mount append_after_seek=1 pread_pwrite_offset=1 "
                "dup_shared_offset=1 eof_short_read=1 zero_write=1 "
                "zero_read=1 pwrite_beyond_eof=1 descriptor_metadata=1 "
                "descriptor_flags=1 access_mode_errors=1 "
                "negative_pread_pwrite=1"
            )
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
