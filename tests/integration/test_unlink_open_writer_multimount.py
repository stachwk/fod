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

    with tempfile.TemporaryDirectory(prefix="fod-unlink-open-writer-multimount-") as temp_dir:
        temp = Path(temp_dir)
        mount_a = temp / "mount-a"
        mount_b = temp / "mount-b"

        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        launcher_a.init_schema()

        name = f"unlink-open-writer-multimount-{uuid.uuid4().hex}.bin"
        initial = bytearray(b"A" * (128 * 1024))
        patch = b"FOD-MULTIMOUNT-WRITE-AFTER-UNLINK"
        patch_offset = 64 * 1024 + 211
        expected_old = bytearray(initial)
        expected_old[patch_offset : patch_offset + len(patch)] = patch
        replacement = b"replacement created by independent FOD mount\n"

        fd: int | None = None
        try:
            launcher_a.start(
                str(mount_a),
                log_prefix="/tmp/fod-unlink-open-writer-multimount-a",
            )
            launcher_b.start(
                str(mount_b),
                log_prefix="/tmp/fod-unlink-open-writer-multimount-b",
            )

            target_a = mount_a / name
            target_b = mount_b / name

            target_a.write_bytes(initial)
            old_ino = target_a.stat().st_ino

            fd = os.open(target_a, os.O_RDWR)
            if os.pread(fd, len(initial), 0) != initial:
                raise AssertionError("mount A pre-unlink payload mismatch")

            target_a.unlink()
            if target_a.exists():
                raise AssertionError("mount A pathname still visible after unlink")

            # Mount B has independent process/kernel state. Recreating the same
            # namespace entry must therefore be decided by PostgreSQL, not by
            # mount A's local fh/cache state.
            target_b.write_bytes(replacement)
            replacement_stat = target_b.stat()
            replacement_ino = replacement_stat.st_ino

            if replacement_ino == old_ino:
                raise AssertionError(
                    "independent mount recreated pathname with still-live old inode: "
                    f"old={old_ino} replacement={replacement_ino}"
                )

            old_stat = os.fstat(fd)
            if old_stat.st_ino != old_ino:
                raise AssertionError(
                    "mount A fstat changed old inode after mount B recreate: "
                    f"expected={old_ino} actual={old_stat.st_ino}"
                )
            if old_stat.st_nlink != 0:
                raise AssertionError(
                    "mount A open-unlinked fd did not report zero links: "
                    f"st_nlink={old_stat.st_nlink}"
                )

            written = os.pwrite(fd, patch, patch_offset)
            if written != len(patch):
                raise AssertionError(
                    f"short pwrite to old inode expected={len(patch)} actual={written}"
                )
            os.fsync(fd)

            old_after = os.pread(fd, len(expected_old), 0)
            if old_after != expected_old:
                raise AssertionError(
                    "old open inode did not preserve cross-mount post-unlink write"
                )

            if target_b.read_bytes() != replacement:
                raise AssertionError(
                    "old inode write leaked into replacement created on mount B"
                )

            visible_b = set(os.listdir(mount_b))
            if name not in visible_b:
                raise AssertionError("replacement missing from mount B directory listing")
            leaked_internal = sorted(
                item for item in visible_b if item.startswith(".fod-unlinked-")
            )
            if leaked_internal:
                raise AssertionError(
                    f"backend deferred-unlink name leaked on mount B: {leaked_internal}"
                )

            os.close(fd)
            fd = None

            if target_b.read_bytes() != replacement:
                raise AssertionError(
                    "replacement changed after final close of old inode on mount A"
                )

            target_b.unlink()

            print(
                "OK unlink-open-writer-multimount "
                f"old_ino={old_ino} replacement_ino={replacement_ino} "
                "postgres_authority=1 independent_mounts=2 "
                "old_fstat_inode_stable=1 old_fstat_nlink_zero=1 "
                f"old_pwrite_after_unlink={written} old_fsync=1 old_pread=1 "
                "replacement_isolated=1 hidden_entry_leaks=0 "
                "final_old_close=1 cleanup=1"
            )
        except Exception:
            launcher_a._dump_log()
            launcher_b._dump_log()
            raise
        finally:
            if fd is not None:
                os.close(fd)
            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
