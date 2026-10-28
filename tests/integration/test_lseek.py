#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import errno
import os
import tempfile
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
                assert os.pread(fd, 3, 6) == b"678"
                assert os.lseek(fd, 0, os.SEEK_CUR) == 3
                assert os.pwrite(fd, b"XY", 1) == 2
                assert os.lseek(fd, 0, os.SEEK_CUR) == 3
            finally:
                os.close(fd)
            expected_positioned_io = b"0XY3456789"
            actual_positioned_io = positioned_io_path.read_bytes()
            assert actual_positioned_io == expected_positioned_io, (
                actual_positioned_io,
                expected_positioned_io,
            )

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

            print("OK lseek/mount append_after_seek=1 pread_pwrite_offset=1")
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
