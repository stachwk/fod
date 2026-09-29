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


def main():
    suffix = uuid.uuid4().hex[:8]
    dir_name = f"inode_{suffix}"
    file_name = f"payload.txt"
    hardlink_name = f"payload-hard.txt"
    symlink_name = f"payload-link.txt"
    child_dir_name = "child-dir"
    payload = b"inode-model\n"

    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-inode-{suffix}.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            dir_path = mountpoint / dir_name
            file_path = dir_path / file_name
            hardlink_path = dir_path / hardlink_name
            symlink_path = dir_path / symlink_name
            child_dir_path = dir_path / child_dir_name

            dir_path.mkdir()
            child_dir_path.mkdir()
            file_path.write_bytes(payload)
            os.link(file_path, hardlink_path)
            os.symlink(file_path.name, symlink_path)

            snapshot = {
                "dir": dir_path.stat().st_ino,
                "child_dir": child_dir_path.stat().st_ino,
                "file": file_path.stat().st_ino,
                "hardlink": hardlink_path.stat().st_ino,
                "symlink": symlink_path.lstat().st_ino,
            }
            assert snapshot["file"] == snapshot["hardlink"], snapshot

            entries = {entry.name: entry for entry in os.scandir(dir_path)}
            assert entries[file_name].inode() == snapshot["file"], snapshot
            assert entries[hardlink_name].inode() == snapshot["hardlink"], snapshot
            assert entries[symlink_name].inode() == snapshot["symlink"], snapshot
            assert entries[child_dir_name].inode() == snapshot["child_dir"], snapshot
            assert entries[file_name].is_file(follow_symlinks=False), entries[file_name]
            assert entries[hardlink_name].is_file(follow_symlinks=False), entries[hardlink_name]
            assert entries[symlink_name].is_symlink(), entries[symlink_name]
            assert entries[child_dir_name].is_dir(follow_symlinks=False), entries[child_dir_name]

            launcher.stop()
            launcher.start(str(mountpoint))
            try:
                assert dir_path.stat().st_ino == snapshot["dir"], snapshot
                assert child_dir_path.stat().st_ino == snapshot["child_dir"], snapshot
                assert file_path.stat().st_ino == snapshot["file"], snapshot
                assert hardlink_path.stat().st_ino == snapshot["hardlink"], snapshot
                assert symlink_path.lstat().st_ino == snapshot["symlink"], snapshot

                entries = {entry.name: entry for entry in os.scandir(dir_path)}
                assert entries[file_name].inode() == snapshot["file"], snapshot
                assert entries[hardlink_name].inode() == snapshot["hardlink"], snapshot
                assert entries[symlink_name].inode() == snapshot["symlink"], snapshot
                assert entries[child_dir_name].inode() == snapshot["child_dir"], snapshot
                assert entries[file_name].is_file(follow_symlinks=False), entries[file_name]
                assert entries[hardlink_name].is_file(follow_symlinks=False), entries[hardlink_name]
                assert entries[symlink_name].is_symlink(), entries[symlink_name]
                assert entries[child_dir_name].is_dir(follow_symlinks=False), entries[child_dir_name]
                print("OK inode/model readdir_parity=1")
            finally:
                launcher.stop()
        finally:
            try:
                hardlink_path.unlink()
            except Exception:
                pass
            try:
                symlink_path.unlink()
            except Exception:
                pass
            try:
                file_path.unlink()
            except Exception:
                pass
            try:
                child_dir_path.rmdir()
            except Exception:
                pass
            try:
                dir_path.rmdir()
            except Exception:
                pass


if __name__ == "__main__":
    main()
