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


def getdents_syscall_number() -> int:
    numbers = {"x86_64": 217, "aarch64": 61, "i386": 220, "armv7l": 217}
    number = numbers.get(platform.machine())
    if number is None:
        raise RuntimeError(f"unsupported getdents64 syscall architecture: {platform.machine()}")
    return number


def read_directory_in_small_chunks(path: Path) -> list[str]:
    libc = ctypes.CDLL(None, use_errno=True)
    libc.syscall.restype = ctypes.c_long
    dir_fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    buffer = ctypes.create_string_buffer(64)
    names: list[str] = []
    try:
        while True:
            count = libc.syscall(getdents_syscall_number(), dir_fd, buffer, len(buffer))
            if count == 0:
                return names
            if count < 0:
                error = ctypes.get_errno()
                raise OSError(error, os.strerror(error), str(path))

            position = 0
            while position < count:
                record_length = int.from_bytes(buffer.raw[position + 16 : position + 18], "little")
                if record_length < 20 or position + record_length > count:
                    raise AssertionError(f"invalid getdents64 record length {record_length}")
                name_bytes = buffer.raw[position + 19 : position + record_length].split(b"\0", 1)[0]
                names.append(os.fsdecode(name_bytes))
                position += record_length
    finally:
        os.close(dir_fd)


def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()
    with tempfile.TemporaryDirectory(prefix="fod-readdir-small-buffers-") as temp_dir:
        mountpoint = Path(temp_dir) / "mount"
        started = False
        try:
            launcher.start(str(mountpoint), log_prefix="/tmp/fod-readdir-small-buffers")
            started = True
            directory = mountpoint / f"entries-{uuid.uuid4().hex}"
            directory.mkdir()
            expected = {f"entry-{index:02d}" for index in range(12)}
            for name in expected:
                (directory / name).touch()

            actual = read_directory_in_small_chunks(directory)
            actual_children = [name for name in actual if name not in (".", "..")]
            if set(actual) != expected | {".", ".."}:
                raise AssertionError(
                    f"small-buffer readdir entries differ: missing={sorted((expected | {'.', '..'}) - set(actual))} "
                    f"extra={sorted(set(actual) - (expected | {'.', '..'}))}"
                )
            if len(actual_children) != len(expected):
                raise AssertionError(f"duplicate readdir entries: {actual}")
            print(f"OK readdir-small-buffers entries={len(actual)} no_skips=1 no_duplicates=1")
        except Exception:
            launcher._dump_log()
            raise
        finally:
            if started:
                launcher.stop()


if __name__ == "__main__":
    main()
