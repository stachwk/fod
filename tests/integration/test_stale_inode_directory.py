#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import sys
import tempfile
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def main() -> None:
    # Keep the inode metadata cached on mount B while mount A replaces the
    # directory. This exercises opendir with the old inode without waiting for
    # the kernel's ordinary cache expiry.
    os.environ["FOD_METADATA_CACHE_TTL_SECONDS"] = "60"

    launcher_a = FODMount(str(ROOT), role="primary")
    launcher_b = FODMount(str(ROOT), role="primary")
    launcher_a.init_schema()

    suffix = uuid.uuid4().hex
    with tempfile.TemporaryDirectory(prefix="fod-stale-directory-") as temp_dir:
        mount_a = Path(temp_dir) / "mount-a"
        mount_b = Path(temp_dir) / "mount-b"
        started_a = False
        started_b = False
        directory_fd: int | None = None
        path_a: Path | None = None
        try:
            launcher_a.start(str(mount_a), log_prefix="/tmp/fod-stale-dir-a")
            started_a = True
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-stale-dir-b")
            started_b = True

            path_a = mount_a / f"replaced-{suffix}"
            path_b = mount_b / path_a.name
            path_a.mkdir()
            directory_fd = os.open(path_b, os.O_RDONLY | os.O_DIRECTORY)
            if os.listdir(directory_fd):
                raise AssertionError("new directory unexpectedly contained entries")
            old_inode = os.fstat(directory_fd).st_ino

            path_a.rmdir()
            path_a.mkdir()
            (path_a / "replacement.txt").write_text("new directory", encoding="utf-8")

            try:
                stale_fd = os.open(path_b, os.O_RDONLY | os.O_DIRECTORY)
            except OSError as exc:
                if exc.errno != errno.ENOENT:
                    raise AssertionError(
                        f"stale-inode opendir returned errno {exc.errno}, expected ENOENT"
                    ) from exc
            else:
                try:
                    current_inode = os.fstat(stale_fd).st_ino
                    if current_inode == old_inode:
                        raise AssertionError(
                            "stale-inode opendir opened the replacement directory"
                        )
                    if set(os.listdir(stale_fd)) != {"replacement.txt"}:
                        raise AssertionError("fresh opendir did not see the replacement child")
                finally:
                    os.close(stale_fd)

            os.lseek(directory_fd, 0, os.SEEK_SET)
            if os.listdir(directory_fd):
                raise AssertionError("rewound old directory handle exposed replacement entries")

            os.close(directory_fd)
            directory_fd = None
            launcher_b.stop()
            started_b = False

            # Remount clears mount B's kernel dentry cache; a new lookup must
            # resolve the replacement directory and enumerate its child.
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-stale-dir-b-fresh")
            started_b = True
            replacement_inode = os.stat(path_b).st_ino
            if replacement_inode == old_inode:
                raise AssertionError("replacement directory reused the deleted directory inode")
            entries = set(os.listdir(path_b))
            if entries != {"replacement.txt"}:
                raise AssertionError(f"fresh lookup returned unexpected entries: {sorted(entries)}")
        finally:
            if directory_fd is not None:
                os.close(directory_fd)
            if path_a is not None:
                try:
                    child = path_a / "replacement.txt"
                    if child.exists():
                        child.unlink()
                    path_a.rmdir()
                except OSError:
                    pass
            if started_b:
                launcher_b.stop()
            if started_a:
                launcher_a.stop()

    print("OK: stale directory handles do not follow a replaced path; fresh lookup sees replacement")


if __name__ == "__main__":
    main()
