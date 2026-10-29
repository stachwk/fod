#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

"""A failed FOD statfs snapshot is reported to callers as EIO."""

from __future__ import annotations

import errno
import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    with tempfile.TemporaryDirectory(prefix="fod-statfs-eio-") as tmpdir:
        mountpoint = Path(tmpdir)
        old_value = os.environ.get("FOD_TEST_STATFS_FORCE_EIO")
        os.environ["FOD_TEST_STATFS_FORCE_EIO"] = "1"
        try:
            launcher.start(str(mountpoint))
            try:
                try:
                    os.statvfs(mountpoint)
                except OSError as exc:
                    assert exc.errno == errno.EIO, exc
                else:
                    log = launcher.config.log_file.read_text(encoding="utf-8") if launcher.config else ""
                    raise AssertionError(
                        "statvfs succeeded despite injected FOD snapshot failure\n" + log[-4000:]
                    )
                print("OK statfs-eio errno=EIO")
            finally:
                launcher.stop()
        finally:
            if old_value is None:
                os.environ.pop("FOD_TEST_STATFS_FORCE_EIO", None)
            else:
                os.environ["FOD_TEST_STATFS_FORCE_EIO"] = old_value


if __name__ == "__main__":
    main()
