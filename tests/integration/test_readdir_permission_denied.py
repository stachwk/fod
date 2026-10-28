#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

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


def main() -> None:
    nobody_probe = subprocess.run(
        ["sudo", "-n", "-u", "nobody", "true"],
        capture_output=True,
        text=True,
        check=False,
    )
    if nobody_probe.returncode != 0:
        print("SKIP readdir-permission-denied (sudo cannot run commands as nobody)")
        return

    os.environ["FOD_DEFAULT_PERMISSIONS"] = "0"
    os.environ["FOD_ALLOW_OTHER"] = "1"
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    with tempfile.TemporaryDirectory(prefix="fod-readdir-permissions-") as temp_dir:
        os.chmod(temp_dir, 0o755)
        mountpoint = Path(temp_dir) / "mount"
        started = False
        try:
            launcher.start(str(mountpoint), log_prefix="/tmp/fod-readdir-permissions")
            started = True
            directory = mountpoint / f"private-{uuid.uuid4().hex}"
            directory.mkdir(mode=0o700)
            (directory / "secret-name").touch()
            directory.chmod(0)

            result = subprocess.run(
                [
                    "sudo",
                    "-n",
                    "-u",
                    "nobody",
                    "python3",
                    "-c",
                    "import os, sys; os.listdir(sys.argv[1])",
                    str(directory),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            if result.returncode == 0:
                raise AssertionError(
                    "nobody listed names in a mode-000 directory with "
                    "FOD_DEFAULT_PERMISSIONS=0"
                )
            if "PermissionError" not in result.stderr:
                raise AssertionError(
                    "directory listing failed for a reason other than denied read access: "
                    f"stdout={result.stdout!r} stderr={result.stderr!r}"
                )
            print("OK readdir-permission-denied default_permissions=0 denied=1")
        except Exception:
            launcher._dump_log()
            raise
        finally:
            if started:
                launcher.stop()


if __name__ == "__main__":
    main()
