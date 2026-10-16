#!/usr/bin/env python
# -*- coding: utf-8 -*-
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

            unlinked_stat = os.fstat(fd)
            if unlinked_stat.st_ino != target_ino:
                raise AssertionError(
                    "fstat changed inode after unlink: "
                    f"expected={target_ino} actual={unlinked_stat.st_ino}"
                )
            if unlinked_stat.st_nlink != 0:
                raise AssertionError(
                    "fstat did not report zero links after unlink: "
                    f"st_nlink={unlinked_stat.st_nlink}"
                )

            replacement = b"replacement path payload\n"
            target.write_bytes(replacement)
            replacement_ino = target.stat().st_ino
            if replacement_ino == target_ino:
                raise AssertionError(
                    "recreated pathname unexpectedly reused unlinked open inode"
                )

            visible_names = set(os.listdir(mountpoint))
            if name not in visible_names:
                raise AssertionError("recreated pathname missing from directory listing")
            leaked_internal = sorted(
                item for item in visible_names if item.startswith(".fod-unlinked-")
            )
            if leaked_internal:
                raise AssertionError(
                    f"deferred unlink backend name leaked through readdir: {leaked_internal}"
                )

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

            if target.read_bytes() != replacement:
                raise AssertionError(
                    "post-unlink writes leaked into recreated pathname"
                )

            os.close(fd)
            fd = None

            if target.read_bytes() != replacement:
                raise AssertionError(
                    "recreated pathname changed after final close of old inode"
                )

            target.unlink()

            print(
                "OK unlink-open-writer "
                f"ino={target_ino} pathname_removed=1 "
                f"replacement_ino={replacement_ino} replacement_recreated=1 "
                "fstat_inode_stable=1 fstat_nlink_zero=1 "
                f"pwrite_after_unlink={written} fsync_after_unlink=1 "
                "pread_after_write=1 replacement_isolated=1 hidden_entry_leaks=0 "
                "final_old_close=1 cleanup=1"
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
