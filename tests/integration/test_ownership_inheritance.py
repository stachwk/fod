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


def assert_current_fod_gid(stat_result, label):
    expected_gid = os.getegid()
    assert stat_result.st_gid == expected_gid, (
        f"{label}: FOD 2.4.0 nie dziedziczy obecnie gid po katalogu nadrzednym; "
        f"expected_gid={expected_gid}, stat={stat_result}"
    )

def assert_current_fod_no_setgid(stat_result, label):
    assert not (stat_result.st_mode & 0o2000), (
        f"{label}: FOD 2.4.0 nie dziedziczy obecnie bitu setgid; "
        f"mode={oct(stat_result.st_mode)}, stat={stat_result}"
    )

def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    current_uid = os.getuid()
    current_gid = os.getgid()
    supplementary_groups = [gid for gid in os.getgroups() if gid != current_gid]
    if not supplementary_groups:
        raise AssertionError("expected at least one supplementary group for ownership inheritance test")
    inherited_gid = supplementary_groups[0]

    suffix = uuid.uuid4().hex[:8]
    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-ownership-{suffix}.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            parent_dir = mountpoint / f"ownership_{suffix}"
            child_dir = parent_dir / "child"
            nested_dir = parent_dir / "nested" / "leaf"
            child_file = parent_dir / "child.txt"
            moved_dir_src = mountpoint / f"ownership_move_src_{suffix}"
            moved_dir_dst = parent_dir / "moved"

            parent_dir.mkdir()
            os.chown(parent_dir, current_uid, inherited_gid)
            os.chmod(parent_dir, 0o2755)

            (parent_dir / "nested").mkdir()
            nested_dir.mkdir()
            nested_parent_stat = (parent_dir / "nested").stat()
            nested_leaf_stat = nested_dir.stat()
            expected_gid = os.getegid()

            assert nested_parent_stat.st_gid == expected_gid, (
                "FOD 2.4.0 nie dziedziczy obecnie gid po katalogu nadrzednym; "
                f"expected_gid={expected_gid}, stat={nested_parent_stat}"
            )

            assert not (nested_parent_stat.st_mode & 0o2000), (
                "FOD 2.4.0 nie dziedziczy obecnie bitu setgid po katalogu nadrzednym; "
                f"stat={nested_parent_stat}"
            )

            assert_current_fod_gid(nested_leaf_stat, "nested_leaf")
            assert_current_fod_no_setgid(nested_leaf_stat, "nested_leaf")

            child_file.write_bytes(b"ownership\n")
            child_dir.mkdir()
            moved_dir_src.mkdir()
            os.rename(moved_dir_src, moved_dir_dst)

            parent_stat = parent_dir.stat()
            file_stat = child_file.stat()
            dir_stat = child_dir.stat()
            moved_stat = moved_dir_dst.stat()

            assert parent_stat.st_gid == inherited_gid, parent_stat
            assert_current_fod_gid(file_stat, "file")
            assert_current_fod_gid(dir_stat, "dir")
            assert moved_stat.st_gid == current_gid, moved_stat
            assert not (moved_stat.st_mode & 0o2000), moved_stat

            umask_file = parent_dir / "umask-file"
            umask_dir = parent_dir / "umask-dir"
            previous_umask = os.umask(0o027)
            try:
                fd = os.open(umask_file, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o666)
                os.close(fd)
                umask_dir.mkdir(mode=0o777)
            finally:
                os.umask(previous_umask)

            umask_file_mode = umask_file.stat().st_mode & 0o777
            umask_dir_mode = umask_dir.stat().st_mode & 0o777
            assert umask_file_mode == 0o640, oct(umask_file_mode)
            assert umask_dir_mode == 0o750, oct(umask_dir_mode)
            print("OK ownership/inheritance umask_file=640 umask_dir=750")
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
