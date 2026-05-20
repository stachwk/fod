#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import errno
import os
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def _run_as_nobody(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["sudo", "-n", "-u", "nobody", *args], check=False)


def _chgrp(path: Path, gid: int, *, follow_symlinks: bool = True) -> None:
    try:
        os.chown(path, -1, gid, follow_symlinks=follow_symlinks)
    except PermissionError as exc:
        raise AssertionError(("chgrp failed", str(path), gid, exc.errno)) from exc


def _assert_ctime_not_older(before: float, after: float) -> None:
    assert after >= before, (before, after)


def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    current_uid = os.getuid()
    current_gid = os.getgid()
    groups = [gid for gid in os.getgroups() if gid != current_gid]
    if not groups:
        raise AssertionError("expected at least one supplementary group")
    alt_gid = groups[0]

    suffix = uuid.uuid4().hex[:8]
    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-permissions-{suffix}.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            sticky_dir = mountpoint / f"sticky-{suffix}"
            sticky_file = sticky_dir / "payload.txt"
            sticky_subdir = sticky_dir / "nested"

            suid_dir = mountpoint / f"suid-{suffix}"
            suid_file = suid_dir / "suid.txt"
            suid_link = suid_dir / "suid-link"
            group_file = suid_dir / "group.txt"
            group_dir = suid_dir / "groupdir"

            sticky_dir.mkdir()
            os.chmod(sticky_dir, 0o1777)
            sticky_subdir.mkdir()
            sticky_file.write_text("sticky\n", encoding="utf-8")

            assert sticky_dir.stat().st_uid == current_uid, sticky_dir.stat()
            assert sticky_subdir.stat().st_uid == current_uid, sticky_subdir.stat()
            assert sticky_file.stat().st_uid == current_uid, sticky_file.stat()

            os.chmod(sticky_file, 0o600)

            unlink_attempt = _run_as_nobody("rm", "-f", str(sticky_file))
            assert unlink_attempt.returncode != 0, "expected sticky-bit unlink by other user to fail"

            rmdir_attempt = _run_as_nobody("rmdir", str(sticky_subdir))
            assert rmdir_attempt.returncode != 0, "expected sticky-bit rmdir by other user to fail"

            sticky_file.unlink()
            sticky_subdir.rmdir()
            os.chmod(sticky_dir, 0o755)
            sticky_dir.rmdir()

            suid_dir.mkdir()
            os.chmod(suid_dir, 0o755)
            suid_file.write_text("suid\n", encoding="utf-8")
            os.chmod(suid_file, 0o6755)
            before = suid_file.stat().st_mode
            assert before & 0o6000 == 0o6000, oct(before)

            _chgrp(suid_file, alt_gid)
            after = suid_file.stat().st_mode
            assert after & 0o6000 == 0, oct(after)

            group_file.write_text("group\n", encoding="utf-8")
            _chgrp(group_file, alt_gid)
            group_stat = group_file.stat()
            assert group_stat.st_gid == alt_gid, group_stat
            group_ctime_before = group_stat.st_ctime
            group_mode_before = group_stat.st_mode
            _chgrp(group_file, alt_gid)
            group_stat_same = group_file.stat()
            assert group_stat_same.st_gid == alt_gid, group_stat_same
            _assert_ctime_not_older(group_ctime_before, group_stat_same.st_ctime)
            assert group_stat_same.st_mode == group_mode_before, (group_mode_before, group_stat_same.st_mode)
            _chgrp(group_file, alt_gid)
            group_stat_same_all = group_file.stat()
            assert group_stat_same_all.st_uid == current_uid, group_stat_same_all
            assert group_stat_same_all.st_gid == alt_gid, group_stat_same_all
            _assert_ctime_not_older(group_ctime_before, group_stat_same_all.st_ctime)
            assert group_stat_same_all.st_mode == group_mode_before, (group_mode_before, group_stat_same_all.st_mode)
            os.chmod(group_file, group_mode_before & 0o7777)
            group_stat_chmod_same = group_file.stat()
            assert group_stat_chmod_same.st_mode == group_mode_before, (group_mode_before, group_stat_chmod_same.st_mode)
            _assert_ctime_not_older(group_ctime_before, group_stat_chmod_same.st_ctime)
            group_ctime_after = group_file.stat().st_ctime
            _assert_ctime_not_older(group_ctime_before, group_ctime_after)

            group_dir.mkdir()
            os.chmod(group_dir, 0o2755)
            group_dir_before = group_dir.stat()
            assert group_dir_before.st_mode & 0o2000, group_dir_before
            _chgrp(group_dir, alt_gid)
            group_dir_after = group_dir.stat()
            assert group_dir_after.st_gid == alt_gid, group_dir_after
            assert group_dir_after.st_mode & 0o2000, group_dir_after
            group_dir_ctime_before = group_dir_after.st_ctime
            group_dir_mode_before = group_dir_after.st_mode
            _chgrp(group_dir, alt_gid)
            group_dir_same_all = group_dir.stat()
            assert group_dir_same_all.st_uid == current_uid, group_dir_same_all
            assert group_dir_same_all.st_gid == alt_gid, group_dir_same_all
            _assert_ctime_not_older(group_dir_ctime_before, group_dir_same_all.st_ctime)
            assert group_dir_same_all.st_mode == group_dir_mode_before, (group_dir_mode_before, group_dir_same_all.st_mode)
            _chgrp(group_dir, alt_gid)
            group_dir_same = group_dir.stat()
            assert group_dir_same.st_gid == alt_gid, group_dir_same
            _assert_ctime_not_older(group_dir_ctime_before, group_dir_same.st_ctime)
            assert group_dir_same.st_mode == group_dir_mode_before, (group_dir_mode_before, group_dir_same.st_mode)
            group_dir_noop = group_dir.stat()
            assert group_dir_noop.st_uid == current_uid, group_dir_noop
            assert group_dir_noop.st_gid == alt_gid, group_dir_noop
            _assert_ctime_not_older(group_dir_ctime_before, group_dir_noop.st_ctime)
            assert group_dir_noop.st_mode == group_dir_mode_before, (group_dir_mode_before, group_dir_noop.st_mode)

            suid_subdir = suid_dir / "suiddir"
            suid_subdir.mkdir()
            os.chmod(suid_subdir, 0o6755)
            suid_dir_before = suid_subdir.stat()
            assert suid_dir_before.st_mode & 0o6000 == 0o6000, suid_dir_before
            _chgrp(suid_subdir, alt_gid)
            suid_dir_after = suid_subdir.stat()
            assert suid_dir_after.st_gid == alt_gid, suid_dir_after
            suid_dir_mode_after = suid_dir_after.st_mode & 0o7777
            assert suid_dir_after.st_mode & 0o2000 == 0o2000, suid_dir_after
            assert suid_dir_mode_after in {0o2755, 0o6755}, oct(suid_dir_mode_after)

            suid_link.symlink_to(suid_file)
            _chgrp(suid_link, alt_gid, follow_symlinks=False)
            symlink_stat = suid_link.lstat()
            assert symlink_stat.st_uid == current_uid, symlink_stat
            assert symlink_stat.st_gid == alt_gid, symlink_stat

            try:
                os.chmod(suid_link, 0o777, follow_symlinks=False)
            except (NotImplementedError, PermissionError, OSError):
                pass

            suid_link.unlink()
            suid_file.unlink()
            group_file.unlink()
            group_dir.rmdir()
            suid_subdir.rmdir()
            suid_dir.rmdir()

            print("OK permissions/sticky/chown")
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
