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
            expect_errno(lambda: os.stat(""), errno.ENOENT, "stat empty pathname")
            expect_errno(
                lambda: os.open("", os.O_RDONLY), errno.ENOENT, "open empty pathname"
            )
            expect_errno(lambda: os.mkdir(""), errno.ENOENT, "mkdir empty pathname")
            expect_errno(lambda: os.unlink(""), errno.ENOENT, "unlink empty pathname")
            expect_errno(lambda: os.rmdir(""), errno.ENOENT, "rmdir empty pathname")

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

            file_link = directory / "regular-file-link"
            os.symlink(file_path.name, file_link)
            expect_errno(
                lambda: os.stat(f"{file_link}/"),
                errno.ENOTDIR,
                "stat symlink to regular file/",
            )
            expect_errno(
                lambda: os.open(f"{file_link}/", os.O_RDONLY),
                errno.ENOTDIR,
                "open symlink to regular file/",
            )

            trailing_directory = directory / "created-with-slash"
            os.mkdir(f"{trailing_directory}/")
            assert trailing_directory.is_dir()
            assert os.stat(f"{trailing_directory}/").st_mode & 0o170000 == 0o040000

            directory_link = directory / "directory-link"
            os.symlink(trailing_directory.name, directory_link)
            linked_directory_stat = os.stat(f"{directory_link}/")
            assert linked_directory_stat.st_ino == trailing_directory.stat().st_ino
            assert linked_directory_stat.st_mode & 0o170000 == 0o040000
            linked_directory_with_slash = f"{directory_link}/"
            directory_fd = os.open(linked_directory_with_slash, os.O_RDONLY)
            os.close(directory_fd)
            expect_errno(
                lambda: os.mkdir(linked_directory_with_slash),
                errno.EEXIST,
                "mkdir symlink to directory/",
            )
            expect_errno(
                lambda: os.unlink(linked_directory_with_slash),
                errno.ENOTDIR,
                "unlink symlink to directory/",
            )
            expect_errno(
                lambda: os.rmdir(linked_directory_with_slash),
                errno.ENOTDIR,
                "rmdir symlink to directory/",
            )
            assert directory_link.is_symlink() and trailing_directory.is_dir()

            normalized = f"{directory}//./created-with-slash/../regular-file"
            assert os.stat(normalized).st_ino == file_path.stat().st_ino
            with open(normalized, "rb") as normalized_file:
                assert normalized_file.read() == b"not a directory"

            name_max = os.pathconf(directory, "PC_NAME_MAX")
            assert name_max >= 255, name_max
            max_name = directory / ("n" * name_max)
            max_name.write_bytes(b"name boundary")
            assert max_name.read_bytes() == b"name boundary"
            expect_errno(
                lambda: os.open(directory / ("n" * (name_max + 1)), os.O_CREAT),
                errno.ENAMETOOLONG,
                "open name longer than NAME_MAX",
            )

            invalid_utf8_name = os.fsencode(directory) + b"/invalid-\xff-name"
            replacement_name = directory / "invalid-\ufffd-name"
            expect_errno(
                lambda: os.open(
                    invalid_utf8_name,
                    os.O_CREAT | os.O_RDWR,
                    0o600,
                ),
                errno.EILSEQ,
                "open cannot create a non-UTF-8 component",
            )
            assert not replacement_name.exists(), (
                "invalid UTF-8 component aliased to its replacement character"
            )
            print(
                "OK pathname empty_path=ENOENTx5 trailing slash file=ENOTDIR "
                "symlink_file=ENOTDIR symlink_directory=accepted "
                "slash_symlink_dir=mkdir:EEXIST,unlink:ENOTDIR,rmdir:ENOTDIR "
                "dotdot_repeated_separator=1 name_max_boundary=1 "
                "non_utf8_create=EILSEQ"
            )
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
