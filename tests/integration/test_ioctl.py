#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import ctypes
import errno
import fcntl
import os
import sys
import tempfile
import termios
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount

FICLONE = 0x40049409


def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    suffix = uuid.uuid4().hex[:8]
    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-ioctl-{suffix}.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            dir_path = mountpoint / f"ioctl_{suffix}"
            file_path = dir_path / "payload.txt"
            payload = b"ioctl payload"

            dir_path.mkdir()
            file_path.write_bytes(payload)

            fd = os.open(file_path, os.O_RDONLY)
            try:
                out = ctypes.c_int(-1)
                fcntl.ioctl(fd, termios.FIONREAD, out, True)
                assert out.value == len(payload), out.value
            finally:
                os.close(fd)

            clone_source = dir_path / f"clone-source-{suffix}.txt"
            clone_dest = dir_path / f"clone-dest-{suffix}.txt"
            clone_source_payload = b"ficlone source payload\n"
            clone_dest_payload = b"existing destination payload\n"
            clone_source.write_bytes(clone_source_payload)
            clone_dest.write_bytes(clone_dest_payload)

            src_fd = os.open(clone_source, os.O_RDONLY)
            dst_fd = os.open(clone_dest, os.O_RDWR)
            try:
                clone_supported = True
                try:
                    fcntl.ioctl(dst_fd, FICLONE, src_fd)
                except OSError as exc:
                    clone_supported = False
                    assert exc.errno in {
                        errno.EOPNOTSUPP,
                        errno.ENOTSUP,
                        errno.EINVAL,
                        errno.ENOTTY,
                    }, exc.errno
            finally:
                os.close(src_fd)
                os.close(dst_fd)

            if clone_supported:
                assert clone_dest.read_bytes() == clone_source_payload
            else:
                assert clone_dest.read_bytes() == clone_dest_payload

            print("OK ioctl/mount")
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
