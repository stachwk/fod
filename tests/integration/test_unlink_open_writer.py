#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import tempfile
import uuid
from pathlib import Path

from fod_mount import FODMount


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    with tempfile.TemporaryDirectory(prefix="fod-unlink-open-writer-") as temp_dir:
        mountpoint = Path(temp_dir) / "mount"

        launcher = FODMount(str(root))
        launcher.init_schema()

        name = f"unlink-open-writer-{uuid.uuid4().hex}.bin"
        initial = bytearray(b"A" * (128 * 1024))
        patch = b"FOD-WRITE-AFTER-UNLINK"
        patch_offset = 64 * 1024 + 123
        expected = bytearray(initial)
        expected[patch_offset : patch_offset + len(patch)] = patch

        fd: int | None = None
        try:
            launcher.start(
                str(mountpoint),
                log_prefix="/tmp/fod-unlink-open-writer",
            )

            target = mountpoint / name
            target.write_bytes(initial)
            target_ino = target.stat().st_ino

            fd = os.open(target, os.O_RDWR)
            before = os.pread(fd, len(initial), 0)
            if before != initial:
                raise AssertionError("pre-unlink writable-fd payload mismatch")

            try:
                target.unlink()
            except OSError as err:
                raise AssertionError(
                    "unlink failed while writable fd was open: "
                    f"errno={err.errno} error={err}"
                ) from err

            if target.exists():
                raise AssertionError("unlinked pathname is still visible")

            try:
                written = os.pwrite(fd, patch, patch_offset)
            except OSError as err:
                raise AssertionError(
                    "pwrite failed after unlink on open fd: "
                    f"errno={err.errno} error={err}"
                ) from err

            if written != len(patch):
                raise AssertionError(
                    f"short pwrite after unlink expected={len(patch)} actual={written}"
                )

            try:
                os.fsync(fd)
            except OSError as err:
                raise AssertionError(
                    "fsync failed after unlink on open fd: "
                    f"errno={err.errno} error={err}"
                ) from err

            try:
                after = os.pread(fd, len(expected), 0)
            except OSError as err:
                raise AssertionError(
                    "pread failed after unlink/write/fsync: "
                    f"errno={err.errno} error={err}"
                ) from err

            if after != expected:
                raise AssertionError(
                    "open writable fd did not preserve post-unlink write: "
                    f"expected_len={len(expected)} actual_len={len(after)}"
                )

            os.close(fd)
            fd = None

            if target.exists():
                raise AssertionError("unlinked pathname reappeared after writer close")

            print(
                "OK unlink-open-writer "
                f"ino={target_ino} pathname_removed=1 "
                f"pwrite_after_unlink={written} fsync_after_unlink=1 "
                "pread_after_write=1 final_close=1 pathname_absent=1"
            )
        except Exception:
            launcher._dump_log()
            raise
        finally:
            if fd is not None:
                os.close(fd)
            launcher.stop()


if __name__ == "__main__":
    main()
