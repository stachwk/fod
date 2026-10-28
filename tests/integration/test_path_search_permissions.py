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


def run_as_nobody(
    script: str, *paths: Path
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "sudo",
            "-n",
            "-u",
            "nobody",
            "python3",
            "-c",
            script,
            *(str(path) for path in paths),
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
            directory.mkdir(mode=0o777)
            directory.chmod(0o777)
            (directory / "secret-name").touch()
            owner_file = directory / "nobody-owned"
            owner_create = run_as_nobody(
                "import pathlib, sys; pathlib.Path(sys.argv[1]).touch()", owner_file
            )
            if owner_create.returncode != 0:
                raise AssertionError(
                    "nobody could not create the owner-permission test file in mode-777 directory: "
                    f"stdout={owner_create.stdout!r} stderr={owner_create.stderr!r}"
                )
            xattr_setup = run_as_nobody(
                "import os, sys; os.setxattr(sys.argv[1], 'user.path-search', b'value')",
                owner_file,
            )
            if xattr_setup.returncode != 0:
                raise AssertionError(
                    "nobody could not create the xattr test value in mode-777 directory: "
                    f"stdout={xattr_setup.stdout!r} stderr={xattr_setup.stderr!r}"
                )
            owner_dir = directory / "nobody-owned-dir"
            owner_mkdir = run_as_nobody(
                "import pathlib, sys; pathlib.Path(sys.argv[1]).mkdir()", owner_dir
            )
            if owner_mkdir.returncode != 0:
                raise AssertionError(
                    "nobody could not create the directory test entry in mode-777 directory: "
                    f"stdout={owner_mkdir.stdout!r} stderr={owner_mkdir.stderr!r}"
                )
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
                    "import pathlib, sys; pathlib.Path(sys.argv[1]).mkdir()",
                    directory / "new-directory",
                ),
                "mkdir in mode-000 directory",
            )
            assert_permission_error(
                run_as_nobody(
                    "import os, sys; os.unlink(sys.argv[1])", directory / "secret-name"
                ),
                "unlink in mode-000 directory",
            )
            assert_permission_error(
                run_as_nobody(
                    "import os, sys; os.rename(sys.argv[1], sys.argv[2])",
                    directory / "secret-name",
                    directory / "renamed-name",
                ),
                "rename in mode-000 directory",
            )
            assert_permission_error(
                run_as_nobody("import os, sys; os.rmdir(sys.argv[1])", owner_dir),
                "rmdir in mode-000 directory",
            )
            assert_permission_error(
                run_as_nobody(
                    "import os, sys; os.chmod(sys.argv[1], 0o600)", owner_file
                ),
                "chmod of an owned file through mode-000 directory",
            )
            assert_permission_error(
                run_as_nobody("import os, sys; os.utime(sys.argv[1], None)", owner_file),
                "utimens of an owned file through mode-000 directory",
            )
            assert_permission_error(
                run_as_nobody(
                    "import os, sys; open(sys.argv[1], 'r+b').truncate()", owner_file
                ),
                "truncate of an owned file through mode-000 directory",
            )
            for label, script in (
                (
                    "getxattr",
                    "import os, sys; os.getxattr(sys.argv[1], 'user.path-search')",
                ),
                ("listxattr", "import os, sys; os.listxattr(sys.argv[1])"),
                (
                    "setxattr",
                    "import os, sys; os.setxattr(sys.argv[1], 'user.after-lock', b'value')",
                ),
                (
                    "removexattr",
                    "import os, sys; os.removexattr(sys.argv[1], 'user.path-search')",
                ),
            ):
                assert_permission_error(
                    run_as_nobody(script, owner_file),
                    f"{label} through mode-000 directory",
                )
            print(
                "OK path-search-permissions default_permissions=0 "
                "readdir_denied=1 stat_denied=1 open_denied=1 access_denied=1 "
                "create_denied=1 mkdir_denied=1 unlink_denied=1 rename_denied=1 "
                "rmdir_denied=1 chmod_denied=1 utimens_denied=1 truncate_denied=1 "
                "getxattr_denied=1 listxattr_denied=1 setxattr_denied=1 "
                "removexattr_denied=1"
            )
        except Exception:
            launcher._dump_log()
            raise
        finally:
            if started:
                launcher.stop()


if __name__ == "__main__":
    main()
