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

    with tempfile.TemporaryDirectory(
        prefix="fod-unlink-remote-open-writer-multimount-"
    ) as temp_dir:
        temp = Path(temp_dir)
        mount_a = temp / "mount-a"
        mount_b = temp / "mount-b"

        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        launcher_a.init_schema()

        name = f"unlink-remote-open-writer-{uuid.uuid4().hex}.bin"
        initial = bytearray(b"A" * (128 * 1024))
        patch = b"FOD-REMOTE-UNLINK-WRITE-AFTER-UNLINK"
        patch_offset = 64 * 1024 + 313
        expected_old = bytearray(initial)
        expected_old[patch_offset : patch_offset + len(patch)] = patch
        replacement = b"replacement created after remote unlink\n"

        fd: int | None = None
        try:
            launcher_a.start(
                str(mount_a),
                log_prefix="/tmp/fod-unlink-remote-open-writer-a",
            )
            launcher_b.start(
                str(mount_b),
                log_prefix="/tmp/fod-unlink-remote-open-writer-b",
            )

            target_a = mount_a / name
            target_b = mount_b / name

            # Plik powstaje przez mount A, aby jego uchwyt byl lokalny tylko dla A.
            target_a.write_bytes(initial)
            old_ino = target_a.stat().st_ino

            fd = os.open(target_a, os.O_RDWR)
            if os.pread(fd, len(initial), 0) != initial:
                raise AssertionError("mount A pre-unlink payload mismatch")

            # Mount B nie ma lokalnej wiedzy o fh z mount A. Unlink musi wiec
            # oprzec decyzje o aktywnym otwarciu na stanie autorytatywnym w PostgreSQL.
            target_b.unlink()
            if target_b.exists():
                raise AssertionError("mount B pathname still visible after remote unlink")

            # Ta sama nazwa moze zostac odtworzona przez mount B, ale musi dostac
            # nowa generacje inode, dopoki stary inode jest nadal otwarty na mount A.
            target_b.write_bytes(replacement)
            replacement_stat = target_b.stat()
            replacement_ino = replacement_stat.st_ino

            if replacement_ino == old_ino:
                raise AssertionError(
                    "remote unlink/recreate reused still-live old inode: "
                    f"old={old_ino} replacement={replacement_ino}"
                )

            # Uchwyt A nadal opisuje stara generacje mimo zdalnego unlink/recreate.
            old_stat = os.fstat(fd)
            if old_stat.st_ino != old_ino:
                raise AssertionError(
                    "mount A fstat changed old inode after remote unlink: "
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
                    "old open inode did not preserve write after remote unlink"
                )

            # Zapis przez stary fh nie moze dotknac nowej generacji widocznej na B.
            if target_b.read_bytes() != replacement:
                raise AssertionError(
                    "old inode write leaked into replacement after remote unlink"
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

            # Zamkniecie starej generacji moze ja usunac, ale nowa nazwa i dane
            # musza pozostac bez zmian.
            if target_b.read_bytes() != replacement:
                raise AssertionError(
                    "replacement changed after final close of remotely unlinked inode"
                )

            target_b.unlink()

            print(
                "OK unlink-remote-open-writer-multimount "
                f"old_ino={old_ino} replacement_ino={replacement_ino} "
                "postgres_authority=1 independent_mounts=2 "
                "open_handle_mount=A remote_unlink_mount=B "
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
            try:
                launcher_b.stop()
            finally:
                launcher_a.stop()


if __name__ == "__main__":
    main()
