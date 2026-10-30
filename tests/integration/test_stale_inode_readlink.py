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
    # Keep mount B's dentry and inode mapping while mount A replaces the link.
    os.environ["FOD_METADATA_CACHE_TTL_SECONDS"] = "60"

    launcher_a = FODMount(str(ROOT), role="primary")
    launcher_b = FODMount(str(ROOT), role="primary")
    launcher_a.init_schema()

    suffix = uuid.uuid4().hex
    link_name = f"stale-readlink-{suffix}"
    with tempfile.TemporaryDirectory(prefix="fod-stale-readlink-") as temp_dir:
        mount_a = Path(temp_dir) / "mount-a"
        mount_b = Path(temp_dir) / "mount-b"
        started_a = False
        started_b = False
        link_a = mount_a / link_name
        link_b = mount_b / link_name
        try:
            launcher_a.start(str(mount_a), log_prefix="/tmp/fod-stale-readlink-a")
            started_a = True
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-stale-readlink-b")
            started_b = True

            link_a.symlink_to("old-target")
            old_inode = os.lstat(link_b).st_ino
            link_a.unlink()
            link_a.symlink_to("replacement-target")

            try:
                observed_target = os.readlink(link_b)
            except OSError as exc:
                if exc.errno != errno.ENOENT:
                    raise AssertionError(
                        f"stale-inode readlink returned errno {exc.errno}, expected ENOENT"
                    ) from exc
            else:
                raise AssertionError(
                    "stale-inode readlink followed the replacement symlink: "
                    f"inode={old_inode} target={observed_target!r}"
                )

            launcher_b.stop()
            started_b = False
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-stale-readlink-b-fresh")
            started_b = True
            replacement_target = os.readlink(link_b)
            if replacement_target != "replacement-target":
                raise AssertionError(
                    "fresh lookup returned an unexpected symlink target: "
                    f"{replacement_target!r}"
                )
        finally:
            if link_a.is_symlink():
                link_a.unlink()
            if started_b:
                launcher_b.stop()
            if started_a:
                launcher_a.stop()

    print("OK: stale symlink inode cannot read the target of a replacement path")


if __name__ == "__main__":
    main()
