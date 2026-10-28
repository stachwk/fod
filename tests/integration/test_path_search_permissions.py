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


def run_as_nobody(script: str, path: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "sudo",
            "-n",
            "-u",
            "nobody",
            "python3",
            "-c",
            script,
            str(path),
        ],
        capture_output=True,
        text=True,
        check=False,
    )


def assert_permission_error(
    result: subprocess.CompletedProcess[str], operation: str
) -> None:
    if result.returncode == 0:
        raise AssertionError(f"nobody unexpectedly completed {operation}")
    if "PermissionError" not in result.stderr:
        raise AssertionError(
            f"{operation} failed for a reason other than denied access: "
            f"stdout={result.stdout!r} stderr={result.stderr!r}"
        )


def main() -> None:
    nobody_probe = subprocess.run(
        ["sudo", "-n", "-u", "nobody", "true"],
        capture_output=True,
        text=True,
        check=False,
    )
    if nobody_probe.returncode != 0:
        print("SKIP path-search-permissions (sudo cannot run commands as nobody)")
        return

    os.environ["FOD_DEFAULT_PERMISSIONS"] = "0"
    os.environ["FOD_ALLOW_OTHER"] = "1"
    os.environ["FOD_METADATA_CACHE_TTL_SECONDS"] = "0"
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    with tempfile.TemporaryDirectory(prefix="fod-path-search-permissions-") as temp_dir:
        os.chmod(temp_dir, 0o755)
        mountpoint = Path(temp_dir) / "mount"
        started = False
        try:
            launcher.start(str(mountpoint), log_prefix="/tmp/fod-path-search-permissions")
            started = True
            directory = mountpoint / f"private-{uuid.uuid4().hex}"
            directory.mkdir(mode=0o700)
            (directory / "secret-name").touch()
            directory.chmod(0)

            assert_permission_error(
                run_as_nobody("import os, sys; os.listdir(sys.argv[1])", directory),
                "readdir in mode-000 directory",
            )
            assert_permission_error(
                run_as_nobody(
                    "import os, sys; os.stat(sys.argv[1])", directory / "secret-name"
                ),
                "stat through mode-000 directory",
            )
            assert_permission_error(
                run_as_nobody(
                    "import pathlib, sys; pathlib.Path(sys.argv[1]).read_bytes()",
                    directory / "secret-name",
                ),
                "open/read through mode-000 directory",
            )

            access_result = run_as_nobody(
                "import os, sys; sys.exit(0 if not os.access(sys.argv[1], os.R_OK) else 1)",
                directory / "secret-name",
            )
            if access_result.returncode != 0:
                raise AssertionError(
                    "os.access reported read access through a mode-000 directory: "
                    f"stdout={access_result.stdout!r} stderr={access_result.stderr!r}"
                )

            assert_permission_error(
                run_as_nobody(
                    "import pathlib, sys; pathlib.Path(sys.argv[1]).touch()",
                    directory / "new-name",
                ),
                "create in mode-000 directory",
            )
            assert_permission_error(
                run_as_nobody(
                    "import os, sys; os.unlink(sys.argv[1])", directory / "secret-name"
                ),
                "unlink in mode-000 directory",
            )
            print(
                "OK path-search-permissions default_permissions=0 "
                "readdir_denied=1 stat_denied=1 open_denied=1 access_denied=1 "
                "create_denied=1 unlink_denied=1"
            )
        except Exception:
            launcher._dump_log()
            raise
        finally:
            if started:
                launcher.stop()


if __name__ == "__main__":
    main()
