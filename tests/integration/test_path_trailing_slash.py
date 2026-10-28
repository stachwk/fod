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

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def expect_errno(operation, expected: int, label: str) -> None:
    try:
        operation()
    except OSError as error:
        assert error.errno == expected, (label, error.errno, expected)
    else:
        raise AssertionError(f"{label} unexpectedly succeeded")


def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()
    suffix = uuid.uuid4().hex[:8]

    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-path-slash-{suffix}.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            directory = mountpoint / f"path-{suffix}"
            directory.mkdir()
            file_path = directory / "regular-file"
            file_path.write_bytes(b"not a directory")
            trailing_slash = f"{file_path}/"

            expect_errno(
                lambda: os.stat(trailing_slash), errno.ENOTDIR, "stat regular file/"
            )
            expect_errno(
                lambda: os.open(trailing_slash, os.O_RDONLY),
                errno.ENOTDIR,
                "open regular file/",
            )
            expect_errno(
                lambda: os.unlink(trailing_slash),
                errno.ENOTDIR,
                "unlink regular file/",
            )
            assert file_path.read_bytes() == b"not a directory"

            trailing_directory = directory / "created-with-slash"
            os.mkdir(f"{trailing_directory}/")
            assert trailing_directory.is_dir()
            assert os.stat(f"{trailing_directory}/").st_mode & 0o170000 == 0o040000
            print("OK pathname trailing slash file=ENOTDIR directory=accepted")
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
