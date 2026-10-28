#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def assert_root_times_advance(
    before: os.stat_result, after: os.stat_result, operation: str
) -> None:
    if after.st_mtime_ns <= before.st_mtime_ns:
        raise AssertionError(
            f"root mtime did not advance after {operation}: "
            f"before={before.st_mtime_ns} after={after.st_mtime_ns}"
        )
    if after.st_ctime_ns <= before.st_ctime_ns:
        raise AssertionError(
            f"root ctime did not advance after {operation}: "
            f"before={before.st_ctime_ns} after={after.st_ctime_ns}"
        )


def assert_root_identity_stable(
    before: os.stat_result, after: os.stat_result, operation: str
) -> None:
    fields = ("st_ino", "st_mode", "st_uid", "st_gid")
    changed = {
        field: (getattr(before, field), getattr(after, field))
        for field in fields
        if getattr(before, field) != getattr(after, field)
    }
    if changed:
        raise AssertionError(f"root identity changed after {operation}: {changed}")


def assert_root_atime_advances_after_readdir(mount_a: Path, mount_b: Path) -> None:
    old_atime_ns = 1_000_000_000
    root = mount_a.stat()
    os.utime(mount_a, ns=(old_atime_ns, root.st_mtime_ns))
    before = mount_b.stat()
    if before.st_atime_ns != old_atime_ns:
        raise AssertionError(
            f"could not seed old root atime: expected={old_atime_ns} "
            f"actual={before.st_atime_ns}"
        )

    list(mount_a.iterdir())
    after = mount_b.stat()
    if after.st_atime_ns <= old_atime_ns:
        raise AssertionError(
            f"root atime did not advance after readdir: "
            f"before={old_atime_ns} after={after.st_atime_ns}"
        )


def main() -> None:
    # Zero the metadata TTL returned to the kernel and used by FOD's own cache,
    # so mount B must fetch root attributes again after mount A mutates it.
    os.environ["FOD_METADATA_CACHE_TTL_SECONDS"] = "0"
    os.environ["FOD_ATIME_POLICY"] = "strictatime"

    launcher_a = FODMount(str(ROOT))
    launcher_b = FODMount(str(ROOT))
    launcher_a.init_schema()

    with tempfile.TemporaryDirectory(prefix="fod-root-metadata-multimount-") as temp_dir:
        mount_a = Path(temp_dir) / "mount-a"
        mount_b = Path(temp_dir) / "mount-b"
        started_a = False
        started_b = False
        try:
            launcher_a.start(str(mount_a), log_prefix="/tmp/fod-root-meta-a")
            started_a = True
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-root-meta-b")
            started_b = True

            root_a = mount_a.stat()
            root_b = mount_b.stat()
            if root_a.st_ino != root_b.st_ino:
                raise AssertionError(
                    f"logical root inode differs across mounts: {root_a.st_ino} vs {root_b.st_ino}"
                )
            if root_a.st_nlink != root_b.st_nlink:
                raise AssertionError(
                    f"logical root link count differs across mounts: {root_a.st_nlink} vs {root_b.st_nlink}"
                )
            assert_root_atime_advances_after_readdir(mount_a, mount_b)

            child = mount_a / f"root-metadata-{uuid.uuid4().hex}"
            child.mkdir()
            after_create = mount_b.stat()
            assert_root_times_advance(root_b, after_create, "root child creation")
            assert_root_identity_stable(root_b, after_create, "root child creation")
            if after_create.st_nlink != root_b.st_nlink + 1:
                raise AssertionError(
                    "root st_nlink did not increase for a child directory: "
                    f"before={root_b.st_nlink} after={after_create.st_nlink}"
                )

            child.rmdir()
            after_remove = mount_b.stat()
            assert_root_times_advance(after_create, after_remove, "root child removal")
            assert_root_identity_stable(after_create, after_remove, "root child removal")
            if after_remove.st_nlink != after_create.st_nlink - 1:
                raise AssertionError(
                    "root st_nlink did not decrease after child directory removal: "
                    f"before={after_create.st_nlink} after={after_remove.st_nlink}"
                )

            print(
                "OK root-directory-metadata-multimount "
                "create=1 remove=1 readdir_atime=1 mtime=1 ctime=1 "
                "nlink=1 identity=1"
            )
        except Exception:
            launcher_a._dump_log()
            launcher_b._dump_log()
            raise
        finally:
            if started_b:
                launcher_b.stop()
            if started_a:
                launcher_a.stop()


if __name__ == "__main__":
    main()
