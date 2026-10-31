#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import ctypes
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

            for operation_name, operation in (
                ("lstat", lambda: os.lstat("")),
                ("readlink", lambda: os.readlink("")),
                ("chmod", lambda: os.chmod("", 0o600)),
                ("chown", lambda: os.chown("", -1, -1)),
                ("utime", lambda: os.utime("", None)),
                ("truncate", lambda: os.truncate("", 0)),
                ("statvfs", lambda: os.statvfs("")),
                ("link source", lambda: os.link("", directory / "empty-link-source")),
                ("link destination", lambda: os.link(file_path, "")),
                ("symlink destination", lambda: os.symlink("target", "")),
                (
                    "rename source",
                    lambda: os.rename("", directory / "empty-rename-source"),
                ),
                ("rename destination", lambda: os.rename(file_path, "")),
            ):
                expect_errno(operation, errno.ENOENT, f"{operation_name} empty pathname")

            libc = ctypes.CDLL(None, use_errno=True)
            libc.access.argtypes = (ctypes.c_char_p, ctypes.c_int)
            libc.access.restype = ctypes.c_int
            ctypes.set_errno(0)
            access_result = libc.access(b"", os.F_OK)
            access_error = ctypes.get_errno()
            assert access_result == -1 and access_error == errno.ENOENT, (
                "access empty pathname",
                access_result,
                access_error,
            )

            assert file_path.read_bytes() == b"not a directory"
            assert not (directory / "empty-link-source").exists()
            assert not (directory / "empty-rename-source").exists()

            rename_utf8_source = directory / "rename-source-\uFFFD"
            rename_utf8_target = directory / "rename-target-\uFFFD"
            rename_utf8_source.write_bytes(b"valid source")
            rename_utf8_target.write_bytes(b"valid target")
            invalid_rename_source = os.fsencode(directory) + b"/rename-source-\xff"
            invalid_rename_target = os.fsencode(directory) + b"/rename-target-\xff"
            expect_errno(
                lambda: os.rename(invalid_rename_source, rename_utf8_target),
                errno.EILSEQ,
                "rename source with invalid UTF-8",
            )
            expect_errno(
                lambda: os.rename(rename_utf8_source, invalid_rename_target),
                errno.EILSEQ,
                "rename destination with invalid UTF-8",
            )
            assert rename_utf8_source.read_bytes() == b"valid source"
            assert rename_utf8_target.read_bytes() == b"valid target"
            rename_utf8_source.unlink()
            rename_utf8_target.unlink()

            path_max = os.pathconf(mountpoint, "PC_PATH_MAX")
            overlong_path = "/" + ("x/" * (path_max // 2 + 1))
            assert len(os.fsencode(overlong_path)) > path_max
            for operation_name, operation in (
                ("stat", lambda: os.stat(overlong_path)),
                ("lstat", lambda: os.lstat(overlong_path)),
                ("open", lambda: os.open(overlong_path, os.O_RDONLY)),
                ("mkdir", lambda: os.mkdir(overlong_path)),
                ("unlink", lambda: os.unlink(overlong_path)),
                ("rmdir", lambda: os.rmdir(overlong_path)),
                ("truncate", lambda: os.truncate(overlong_path, 0)),
                ("chmod", lambda: os.chmod(overlong_path, 0o600)),
                ("chown", lambda: os.chown(overlong_path, -1, -1)),
                ("utime", lambda: os.utime(overlong_path, None)),
                ("statvfs", lambda: os.statvfs(overlong_path)),
                ("readlink", lambda: os.readlink(overlong_path)),
                (
                    "link source",
                    lambda: os.link(overlong_path, directory / "overlong-link-source"),
                ),
                (
                    "link destination",
                    lambda: os.link(file_path, overlong_path),
                ),
                (
                    "symlink destination",
                    lambda: os.symlink("target", overlong_path),
                ),
                (
                    "rename source",
                    lambda: os.rename(overlong_path, directory / "overlong-rename-source"),
                ),
                (
                    "rename destination",
                    lambda: os.rename(file_path, overlong_path),
                ),
            ):
                expect_errno(
                    operation,
                    errno.ENAMETOOLONG,
                    f"{operation_name} beyond PATH_MAX",
                )
            ctypes.set_errno(0)
            access_result = libc.access(os.fsencode(overlong_path), os.F_OK)
            access_error = ctypes.get_errno()
            assert access_result == -1 and access_error == errno.ENAMETOOLONG, (
                "access beyond PATH_MAX",
                access_result,
                access_error,
            )
            assert file_path.read_bytes() == b"not a directory"
            assert not (directory / "overlong-link-source").exists()
            assert not (directory / "overlong-rename-source").exists()

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
            rename_source = directory / "rename-source"
            rename_source.write_bytes(b"rename source")
            for operation_name, operation, expected_errno in (
                ("lstat", lambda: os.lstat(trailing_slash), errno.ENOTDIR),
                ("mkdir", lambda: os.mkdir(trailing_slash), errno.EEXIST),
                ("rmdir", lambda: os.rmdir(trailing_slash), errno.ENOTDIR),
                (
                    "truncate",
                    lambda: os.truncate(trailing_slash, 0),
                    errno.ENOTDIR,
                ),
                (
                    "chmod",
                    lambda: os.chmod(trailing_slash, 0o600),
                    errno.ENOTDIR,
                ),
                (
                    "chown",
                    lambda: os.chown(trailing_slash, -1, -1),
                    errno.ENOTDIR,
                ),
                (
                    "utime",
                    lambda: os.utime(trailing_slash, None),
                    errno.ENOTDIR,
                ),
                (
                    "readlink",
                    lambda: os.readlink(trailing_slash),
                    errno.ENOTDIR,
                ),
                (
                    "statvfs",
                    lambda: os.statvfs(trailing_slash),
                    errno.ENOTDIR,
                ),
                (
                    "link source",
                    lambda: os.link(trailing_slash, directory / "file-slash-link"),
                    errno.ENOTDIR,
                ),
                (
                    "link destination",
                    lambda: os.link(rename_source, trailing_slash),
                    errno.EEXIST,
                ),
                (
                    "symlink destination",
                    lambda: os.symlink("unused", trailing_slash),
                    errno.EEXIST,
                ),
                (
                    "rename source",
                    lambda: os.rename(trailing_slash, directory / "renamed-file"),
                    errno.ENOTDIR,
                ),
                (
                    "rename destination",
                    lambda: os.rename(rename_source, trailing_slash),
                    errno.ENOTDIR,
                ),
            ):
                expect_errno(
                    operation,
                    expected_errno,
                    f"{operation_name} regular file/",
                )
            ctypes.set_errno(0)
            access_result = libc.access(os.fsencode(trailing_slash), os.F_OK)
            access_error = ctypes.get_errno()
            assert access_result == -1 and access_error == errno.ENOTDIR, (
                "access regular file/",
                access_result,
                access_error,
            )
            assert file_path.read_bytes() == b"not a directory"
            assert rename_source.read_bytes() == b"rename source"
            assert not (directory / "file-slash-link").exists()
            assert not (directory / "renamed-file").exists()

            file_link = directory / "regular-file-link"
            os.symlink(file_path.name, file_link)
            file_link_slash = f"{file_link}/"
            expect_errno(
                lambda: os.stat(file_link_slash),
                errno.ENOTDIR,
                "stat symlink to regular file/",
            )
            expect_errno(
                lambda: os.open(file_link_slash, os.O_RDONLY),
                errno.ENOTDIR,
                "open symlink to regular file/",
            )
            expect_errno(
                lambda: os.lstat(file_link_slash),
                errno.ENOTDIR,
                "lstat symlink to regular file/",
            )
            expect_errno(
                lambda: os.unlink(file_link_slash),
                errno.ENOTDIR,
                "unlink symlink to regular file/",
            )
            expect_errno(
                lambda: os.rmdir(file_link_slash),
                errno.ENOTDIR,
                "rmdir symlink to regular file/",
            )
            expect_errno(
                lambda: os.truncate(file_link_slash, 0),
                errno.ENOTDIR,
                "truncate symlink to regular file/",
            )
            expect_errno(
                lambda: os.chmod(file_link_slash, 0o600),
                errno.ENOTDIR,
                "chmod symlink to regular file/",
            )
            expect_errno(
                lambda: os.utime(file_link_slash, None),
                errno.ENOTDIR,
                "utime symlink to regular file/",
            )
            for operation_name, operation, expected_errno in (
                ("mkdir", lambda: os.mkdir(file_link_slash), errno.EEXIST),
                (
                    "chown",
                    lambda: os.chown(file_link_slash, -1, -1),
                    errno.ENOTDIR,
                ),
                (
                    "readlink",
                    lambda: os.readlink(file_link_slash),
                    errno.ENOTDIR,
                ),
                (
                    "statvfs",
                    lambda: os.statvfs(file_link_slash),
                    errno.ENOTDIR,
                ),
                (
                    "link source",
                    lambda: os.link(file_link_slash, directory / "symlink-slash-link"),
                    errno.ENOTDIR,
                ),
                (
                    "link destination",
                    lambda: os.link(rename_source, file_link_slash),
                    errno.EEXIST,
                ),
                (
                    "symlink destination",
                    lambda: os.symlink("unused", file_link_slash),
                    errno.EEXIST,
                ),
                (
                    "rename source",
                    lambda: os.rename(file_link_slash, directory / "renamed-symlink"),
                    errno.ENOTDIR,
                ),
                (
                    "rename destination",
                    lambda: os.rename(rename_source, file_link_slash),
                    errno.ENOTDIR,
                ),
            ):
                expect_errno(
                    operation,
                    expected_errno,
                    f"{operation_name} symlink to regular file/",
                )
            ctypes.set_errno(0)
            access_result = libc.access(os.fsencode(file_link_slash), os.F_OK)
            access_error = ctypes.get_errno()
            assert access_result == -1 and access_error == errno.ENOTDIR, (
                "access symlink to regular file/",
                access_result,
                access_error,
            )
            assert file_link.is_symlink() and file_path.read_bytes() == b"not a directory"
            assert rename_source.read_bytes() == b"rename source"
            assert not (directory / "symlink-slash-link").exists()
            assert not (directory / "renamed-symlink").exists()

            dangling_link = directory / "dangling-directory-link"
            os.symlink("missing-target", dangling_link)
            dangling_slash = f"{dangling_link}/"
            for operation_name, operation in (
                ("stat", lambda: os.stat(dangling_slash)),
                ("lstat", lambda: os.lstat(dangling_slash)),
                ("open", lambda: os.open(dangling_slash, os.O_RDONLY)),
                ("truncate", lambda: os.truncate(dangling_slash, 0)),
                ("chmod", lambda: os.chmod(dangling_slash, 0o600)),
                ("chown", lambda: os.chown(dangling_slash, -1, -1)),
                ("utime", lambda: os.utime(dangling_slash, None)),
                ("readlink", lambda: os.readlink(dangling_slash)),
                ("statvfs", lambda: os.statvfs(dangling_slash)),
                (
                    "link",
                    lambda: os.link(dangling_slash, directory / "dangling-hardlink"),
                ),
            ):
                expect_errno(
                    operation,
                    errno.ENOENT,
                    f"{operation_name} dangling symlink/",
                )
            expect_errno(
                lambda: os.mkdir(dangling_slash),
                errno.EEXIST,
                "mkdir dangling symlink/",
            )
            expect_errno(
                lambda: os.symlink("unused", dangling_slash),
                errno.EEXIST,
                "symlink destination dangling symlink/",
            )
            expect_errno(
                lambda: os.link(rename_source, dangling_slash),
                errno.EEXIST,
                "link destination dangling symlink/",
            )
            for operation_name, operation in (
                ("unlink", lambda: os.unlink(dangling_slash)),
                ("rmdir", lambda: os.rmdir(dangling_slash)),
                (
                    "rename",
                    lambda: os.rename(
                        dangling_slash, directory / "renamed-dangling-link"
                    ),
                ),
            ):
                expect_errno(
                    operation,
                    errno.ENOTDIR,
                    f"{operation_name} dangling symlink/",
                )
            expect_errno(
                lambda: os.rename(rename_source, dangling_slash),
                errno.ENOTDIR,
                "rename destination dangling symlink/",
            )
            ctypes.set_errno(0)
            access_result = libc.access(os.fsencode(dangling_slash), os.F_OK)
            access_error = ctypes.get_errno()
            assert access_result == -1 and access_error == errno.ENOENT, (
                "access dangling symlink/",
                access_result,
                access_error,
            )
            assert dangling_link.is_symlink()
            assert os.readlink(dangling_link) == "missing-target"
            assert rename_source.read_bytes() == b"rename source"

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
            linked_directory_lstat = os.lstat(linked_directory_with_slash)
            assert linked_directory_lstat.st_ino == trailing_directory.stat().st_ino
            assert linked_directory_lstat.st_mode & 0o170000 == 0o040000
            assert os.statvfs(linked_directory_with_slash).f_bsize > 0
            os.chown(linked_directory_with_slash, -1, -1)
            assert os.access(linked_directory_with_slash, os.F_OK)
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
            symlink_mtime_ns = os.lstat(directory_link).st_mtime_ns
            os.chmod(linked_directory_with_slash, 0o711)
            assert trailing_directory.stat().st_mode & 0o777 == 0o711
            assert os.lstat(directory_link).st_mtime_ns == symlink_mtime_ns
            target_mtime_ns = 1_700_000_000_123_456_000
            os.utime(
                linked_directory_with_slash,
                ns=(target_mtime_ns, target_mtime_ns),
            )
            assert trailing_directory.stat().st_mtime_ns == target_mtime_ns
            assert os.lstat(directory_link).st_mtime_ns == symlink_mtime_ns
            expect_errno(
                lambda: os.truncate(linked_directory_with_slash, 0),
                errno.EISDIR,
                "truncate symlink to directory/",
            )
            expect_errno(
                lambda: os.readlink(linked_directory_with_slash),
                errno.EINVAL,
                "readlink symlink to directory/",
            )
            renamed_directory_target = directory / "renamed-directory-target"
            expect_errno(
                lambda: os.rename(
                    linked_directory_with_slash, renamed_directory_target
                ),
                errno.ENOTDIR,
                "rename symlink to directory/",
            )
            expect_errno(
                lambda: os.rename(rename_source, linked_directory_with_slash),
                errno.ENOTDIR,
                "rename destination symlink to directory/",
            )
            expect_errno(
                lambda: os.link(
                    linked_directory_with_slash,
                    directory / "directory-symlink-hardlink",
                ),
                errno.EPERM,
                "link source symlink to directory/",
            )
            expect_errno(
                lambda: os.link(rename_source, linked_directory_with_slash),
                errno.EEXIST,
                "link destination symlink to directory/",
            )
            expect_errno(
                lambda: os.symlink("unused", linked_directory_with_slash),
                errno.EEXIST,
                "symlink destination symlink to directory/",
            )
            assert directory_link.is_symlink() and trailing_directory.is_dir()
            assert rename_source.read_bytes() == b"rename source"
            assert not (directory / "directory-symlink-hardlink").exists()
            assert not renamed_directory_target.exists()

            normalized = f"{directory}//./created-with-slash/../regular-file"
            assert os.stat(normalized).st_ino == file_path.stat().st_ino
            with open(normalized, "rb") as normalized_file:
                assert normalized_file.read() == b"not a directory"
            absolute_suffix = file_path.as_posix().lstrip("/")
            for leading_slashes in ("//", "///"):
                leading_slash_path = f"{leading_slashes}{absolute_suffix}"
                assert os.stat(leading_slash_path).st_ino == file_path.stat().st_ino
                with open(leading_slash_path, "rb") as repeated_slash_file:
                    assert repeated_slash_file.read() == b"not a directory"

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
            max_rename_source = directory / ("r" * name_max)
            too_long_rename_target = directory / ("r" * (name_max + 1))
            max_rename_source.write_bytes(b"rename NAME_MAX boundary")
            expect_errno(
                lambda: os.rename(max_rename_source, too_long_rename_target),
                errno.ENAMETOOLONG,
                "rename destination longer than NAME_MAX",
            )
            assert max_rename_source.read_bytes() == b"rename NAME_MAX boundary"
            expect_errno(
                lambda: os.lstat(too_long_rename_target),
                errno.ENAMETOOLONG,
                "lstat destination longer than NAME_MAX",
            )
            max_rename_target = directory / ("s" * name_max)
            os.rename(max_rename_source, max_rename_target)
            assert max_rename_target.read_bytes() == b"rename NAME_MAX boundary"

            plain_name = directory / "trailing"
            space_name = directory / "trailing "
            tab_name = directory / "trailing\t"
            plain_name.write_bytes(b"plain")
            space_name.write_bytes(b"space")
            tab_name.write_bytes(b"tab")
            assert plain_name.read_bytes() == b"plain"
            assert space_name.read_bytes() == b"space"
            assert tab_name.read_bytes() == b"tab"
            assert len(
                {plain_name.stat().st_ino, space_name.stat().st_ino, tab_name.stat().st_ino}
            ) == 3

            renamed_space_name = directory / "renamed "
            renamed_tab_name = directory / "renamed\t"
            os.rename(space_name, renamed_space_name)
            os.rename(tab_name, renamed_tab_name)
            assert renamed_space_name.read_bytes() == b"space"
            assert renamed_tab_name.read_bytes() == b"tab"
            os.unlink(renamed_space_name)
            os.unlink(renamed_tab_name)
            assert plain_name.read_bytes() == b"plain"

            for parent_name, child_payload in (
                ("parent ", b"space-parent"),
                ("parent\t", b"tab-parent"),
            ):
                whitespace_parent = directory / parent_name
                whitespace_parent.mkdir()
                child = whitespace_parent / "child "
                renamed_child = whitespace_parent / "renamed\t"
                child.write_bytes(child_payload)
                os.rename(child, renamed_child)
                assert renamed_child.read_bytes() == child_payload
                os.unlink(renamed_child)
                os.rmdir(whitespace_parent)

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
                "OK pathname empty_path=ENOENTx17 access=ENOENT "
                "path_max_errors=ENAMETOOLONGx17 access=ENAMETOOLONG "
                "trailing_slash_file=ENOTDIRx14,EEXISTx3 "
                "access=ENOTDIR "
                "symlink_file=ENOTDIRx14,EEXISTx3 access=ENOTDIR "
                "symlink_directory=accepted "
                "dangling_slash=ENOENTx10,EEXISTx3,ENOTDIRx4 access=ENOENT "
                "slash_symlink_dir=statvfs:ok,chown:ok,access:ok,link:EPERM "
                "mkdir/link/symlink_dest:EEXIST,unlink/rmdir/rename:ENOTDIR "
                "chmod_target=0o711 utime_target=exact truncate:EISDIR "
                "readlink:EINVAL rename:ENOTDIR leading_slashes=//,/// "
                "whitespace_names=distinct rename=exact parent_space/tab=exact "
                "dotdot_repeated_separator=1 name_max_boundary=1 "
                f"rename_name_max={name_max},rename_too_long=ENAMETOOLONG,preserved=1 "
                "non_utf8_create=EILSEQ rename_non_utf8=EILSEQx2 aliases=0"
            )
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
