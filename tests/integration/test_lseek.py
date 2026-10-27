#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

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

            print("OK lseek/mount")
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
