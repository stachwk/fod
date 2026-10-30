#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import ctypes
import errno
import os
import platform
import resource
import sys
import tempfile
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def getdents64_number() -> int:
    numbers = {"x86_64": 217, "aarch64": 61, "i386": 220, "armv7l": 217}
    number = numbers.get(platform.machine())
    if number is None:
        raise RuntimeError(f"unsupported getdents64 syscall architecture: {platform.machine()}")
    return number


def read_chunk(libc: ctypes.CDLL, fd: int, buffer: ctypes.Array[ctypes.c_char]) -> list[str]:
    count = libc.syscall(getdents64_number(), fd, buffer, len(buffer))
    if count == 0:
        return []
    if count < 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))

    names: list[str] = []
    position = 0
    while position < count:
        record_length = int.from_bytes(buffer.raw[position + 16 : position + 18], "little")
        if record_length < 20 or position + record_length > count:
            raise AssertionError(f"invalid getdents64 record length {record_length}")
        name_bytes = buffer.raw[position + 19 : position + record_length].split(b"\0", 1)[0]
        names.append(os.fsdecode(name_bytes))
        position += record_length
    return names


def drain(libc: ctypes.CDLL, fd: int, buffer: ctypes.Array[ctypes.c_char]) -> list[str]:
    names: list[str] = []
    while chunk := read_chunk(libc, fd, buffer):
        names.extend(chunk)
    return names


def assert_no_duplicates(names: list[str], stream: str) -> None:
    children = [name for name in names if name not in {".", ".."}]
    if len(children) != len(set(children)):
        duplicates = sorted(name for name in set(children) if children.count(name) > 1)
        raise AssertionError(f"{stream} repeated directory entries: {duplicates}")


def check_libc_directory_stream(path: Path, expected_entries: int) -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    libc.opendir.argtypes = [ctypes.c_char_p]
    libc.opendir.restype = ctypes.c_void_p
    libc.readdir.argtypes = [ctypes.c_void_p]
    libc.readdir.restype = ctypes.c_void_p
    libc.closedir.argtypes = [ctypes.c_void_p]
    libc.closedir.restype = ctypes.c_int

    ctypes.set_errno(0)
    stream = libc.opendir(os.fsencode(path))
    if not stream:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), str(path))

    count = 0
    try:
        while True:
            ctypes.set_errno(0)
            entry = libc.readdir(stream)
            if not entry:
                error = ctypes.get_errno()
                if error:
                    raise OSError(error, os.strerror(error), str(path))
                break
            count += 1
    finally:
        ctypes.set_errno(0)
        if libc.closedir(stream) != 0:
            error = ctypes.get_errno()
            raise OSError(error, os.strerror(error), str(path))

    if count != expected_entries:
        raise AssertionError(
            f"libc readdir returned {count} entries, expected {expected_entries}"
        )

    missing_path = path / f"missing-{uuid.uuid4().hex}"
    ctypes.set_errno(0)
    missing_stream = libc.opendir(os.fsencode(missing_path))
    error = ctypes.get_errno()
    if missing_stream:
        libc.closedir(missing_stream)
        raise AssertionError(f"libc opendir unexpectedly opened {missing_path}")
    if error != errno.ENOENT:
        raise AssertionError(
            f"libc opendir errno={error}, expected ENOENT ({errno.ENOENT})"
        )

    soft_limit, hard_limit = resource.getrlimit(resource.RLIMIT_NOFILE)
    open_fd_count = len(os.listdir("/proc/self/fd")) - 1
    exhausted_limit = max(1, open_fd_count)
    if exhausted_limit > hard_limit:
        raise AssertionError(
            f"cannot exercise EMFILE: {open_fd_count} descriptors exceed hard limit {hard_limit}"
        )
    try:
        resource.setrlimit(resource.RLIMIT_NOFILE, (exhausted_limit, hard_limit))
        ctypes.set_errno(0)
        exhausted_stream = libc.opendir(os.fsencode(path))
        error = ctypes.get_errno()
        if exhausted_stream:
            libc.closedir(exhausted_stream)
            raise AssertionError("libc opendir succeeded after exhausting the fd limit")
        if error != errno.EMFILE:
            raise AssertionError(
                f"libc opendir errno={error}, expected EMFILE ({errno.EMFILE})"
            )
    finally:
        resource.setrlimit(resource.RLIMIT_NOFILE, (soft_limit, hard_limit))


def main() -> None:
    launcher_a = FODMount(str(ROOT), role="primary")
    launcher_b = FODMount(str(ROOT), role="primary")
    launcher_a.init_schema()

    with tempfile.TemporaryDirectory(prefix="fod-readdir-snapshot-multimount-") as temp_dir:
        mount_a = Path(temp_dir) / "mount-a"
        mount_b = Path(temp_dir) / "mount-b"
        started_a = False
        started_b = False
        dir_fd: int | None = None
        path_a: Path | None = None
        try:
            launcher_a.start(str(mount_a), log_prefix="/tmp/fod-readdir-snapshot-a")
            started_a = True
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-readdir-snapshot-b")
            started_b = True

            path_a = mount_a / f"entries-{uuid.uuid4().hex}"
            path_b = mount_b / path_a.name
            path_a.mkdir()
            stable = {f"stable-{index:03d}" for index in range(256)}
            removed = {f"removed-{index:03d}" for index in range(256)}
            added = {f"added-{index:03d}" for index in range(128)}
            for name in stable | removed:
                (path_a / name).touch()

            dir_fd = os.open(path_b, os.O_RDONLY | os.O_DIRECTORY)
            libc = ctypes.CDLL(None, use_errno=True)
            libc.syscall.restype = ctypes.c_long
            buffer = ctypes.create_string_buffer(128)

            # Advance into the stream before changing membership through A.
            prefix: list[str] = []
            while not (set(prefix) - {".", ".."}):
                chunk = read_chunk(libc, dir_fd, buffer)
                if not chunk:
                    raise AssertionError("directory stream ended before returning any child")
                prefix.extend(chunk)

            for name in removed:
                (path_a / name).unlink()
            for name in added:
                (path_a / name).touch()

            remainder = drain(libc, dir_fd, buffer)
            original_stream = prefix + remainder
            assert_no_duplicates(original_stream, "pre-rewind stream")
            visible_before_rewind = set(original_stream) - {".", ".."}
            if not stable <= visible_before_rewind:
                raise AssertionError(
                    "unmodified entries were lost while another mount changed membership: "
                    f"missing={sorted(stable - visible_before_rewind)}"
                )

            os.lseek(dir_fd, 0, os.SEEK_SET)
            after_rewind = drain(libc, dir_fd, buffer)
            assert_no_duplicates(after_rewind, "rewound stream")
            expected_after_rewind = stable | added
            actual_after_rewind = set(after_rewind) - {".", ".."}
            if actual_after_rewind != expected_after_rewind:
                raise AssertionError(
                    "rewind did not refresh the directory snapshot: "
                    f"missing={sorted(expected_after_rewind - actual_after_rewind)} "
                    f"extra={sorted(actual_after_rewind - expected_after_rewind)}"
                )
            check_libc_directory_stream(path_b, len(expected_after_rewind) + 2)
        finally:
            if dir_fd is not None:
                os.close(dir_fd)
            if path_a is not None:
                try:
                    for child in path_a.iterdir():
                        child.unlink()
                    path_a.rmdir()
                except OSError:
                    pass
            if started_b:
                launcher_b.stop()
            if started_a:
                launcher_a.stop()

    print(
        "OK: 512-entry cross-mount snapshot has stable continuation, rewind refresh, "
        "and direct libc readdir/errno coverage"
    )


if __name__ == "__main__":
    main()
