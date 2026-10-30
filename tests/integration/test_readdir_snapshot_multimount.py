#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import ctypes
import os
import platform
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
            stable = {f"stable-{index:02d}" for index in range(24)}
            removed = {f"removed-{index:02d}" for index in range(24)}
            added = {f"added-{index:02d}" for index in range(12)}
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

    print("OK: cross-mount membership changes preserve the open stream; rewind refreshes it")


if __name__ == "__main__":
    main()
