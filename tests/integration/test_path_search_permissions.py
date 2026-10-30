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


def run_as_root(script: str, *paths: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["sudo", "-n", "python3", "-c", script, *(str(path) for path in paths)],
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


def assert_errno_from_nobody(
    result: subprocess.CompletedProcess[str], expected: int, operation: str
) -> None:
    if result.returncode != 0 or result.stdout.strip() != str(expected):
        raise AssertionError(
            f"{operation}: errno={result.stdout.strip()!r}, expected={expected}; "
            f"stderr={result.stderr!r}"
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
    root_probe = subprocess.run(
        ["sudo", "-n", "true"], capture_output=True, text=True, check=False
    )
    if root_probe.returncode != 0:
        print("SKIP path-search-permissions (sudo cannot run commands as root)")
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
            owner_payload_setup = run_as_nobody(
                "import pathlib, sys; "
                "pathlib.Path(sys.argv[1]).write_bytes(b'parent search truncate payload')",
                owner_file,
            )
            if owner_payload_setup.returncode != 0:
                raise AssertionError(
                    "nobody could not prepare the truncate payload: "
                    f"stdout={owner_payload_setup.stdout!r} "
                    f"stderr={owner_payload_setup.stderr!r}"
                )
            metadata_owner_file = directory / "metadata-owner"
            metadata_owner_file.write_text("owner metadata\n", encoding="utf-8")
            metadata_owner_file.chmod(0o666)

            real_uid_file = directory / "real-uid-only"
            real_uid_file.write_text("real uid access\n", encoding="utf-8")
            real_uid_file.chmod(0o400)
            real_uid_access = run_as_root(
                """
import os
import pwd
import sys
identity = pwd.getpwnam("nobody")
os.setresgid(identity.pw_gid, 0, 0)
os.setresuid(identity.pw_uid, 0, 0)
print(int(os.getuid() != os.geteuid()), int(os.access(sys.argv[1], os.R_OK)))
""",
                real_uid_file,
            )
            if real_uid_access.returncode != 0 or real_uid_access.stdout.strip() != "1 0":
                raise AssertionError(
                    "access() did not use the real uid with default permissions disabled: "
                    f"stdout={real_uid_access.stdout!r} stderr={real_uid_access.stderr!r}"
                )

            root_noexec_file = directory / "root-noexec"
            root_noexec_file.write_text("no execute bit\n", encoding="utf-8")
            root_noexec_file.chmod(0o644)
            root_exec_file = directory / "root-exec"
            root_exec_file.write_text("execute bit\n", encoding="utf-8")
            root_exec_file.chmod(0o100)
            for path, expected, label in (
                (root_noexec_file, "0", "root X_OK without an execute bit"),
                (root_exec_file, "1", "root X_OK with an execute bit"),
            ):
                result = run_as_root(
                    "import os, sys; print(int(os.access(sys.argv[1], os.X_OK)))", path
                )
                if result.returncode != 0 or result.stdout.strip() != expected:
                    raise AssertionError(
                        f"{label}: result={result.stdout.strip()!r}, expected={expected!r}; "
                        f"stderr={result.stderr!r}"
                    )

            assert_errno_from_nobody(
                run_as_nobody(
                    """
import os
import sys
try:
    os.chmod(sys.argv[1], 0o600)
except OSError as error:
    print(error.errno)
else:
    print(0)
""",
                    metadata_owner_file,
                ),
                errno.EPERM,
                "chmod of another user's file",
            )
            assert_errno_from_nobody(
                run_as_nobody(
                    """
import os
import sys
try:
    os.chown(sys.argv[1], int(sys.argv[2]), -1)
except OSError as error:
    print(error.errno)
else:
    print(0)
""",
                    metadata_owner_file,
                    str(os.getuid()),
                ),
                errno.EPERM,
                "chown of another user's file",
            )
            readonly_file = directory / "nobody-readonly"
            readonly_setup = run_as_nobody(
                "import os, pathlib, sys; "
                "pathlib.Path(sys.argv[1]).write_bytes(b'readonly truncate payload'); "
                "os.chmod(sys.argv[1], 0o444)",
                readonly_file,
            )
            if readonly_setup.returncode != 0:
                raise AssertionError(
                    "nobody could not create the read-only truncate test file: "
                    f"stdout={readonly_setup.stdout!r} stderr={readonly_setup.stderr!r}"
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
            assert_errno_from_nobody(
                run_as_nobody(
                    """
import errno
import os
import sys
try:
    os.truncate(sys.argv[1], 0)
except OSError as error:
    print(error.errno)
else:
    print(0)
""",
                    readonly_file,
                ),
                errno.EACCES,
                "truncate of a read-only file",
            )
            assert readonly_file.read_bytes() == b"readonly truncate payload"
            assert_permission_error(
                run_as_nobody(
                    "import sys; open(sys.argv[1], 'wb').close()", readonly_file
                ),
                "open(O_TRUNC) of a read-only file",
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
            assert_errno_from_nobody(
                run_as_nobody(
                    """
import errno
import pathlib
import sys
try:
    pathlib.Path(sys.argv[1]).mkdir()
except OSError as error:
    print(error.errno)
else:
    print(0)
""",
                    directory / "new-directory",
                ),
                errno.EACCES,
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
            assert_errno_from_nobody(
                run_as_nobody(
                    """
import errno
import os
import sys
try:
    os.rmdir(sys.argv[1])
except OSError as error:
    print(error.errno)
else:
    print(0)
""",
                    owner_dir,
                ),
                errno.EACCES,
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
            assert_errno_from_nobody(
                run_as_nobody(
                    """
import errno
import os
import sys
try:
    os.truncate(sys.argv[1], 0)
except OSError as error:
    print(error.errno)
else:
    print(0)
""",
                    owner_file,
                ),
                errno.EACCES,
                "truncate of an owned file through mode-000 directory",
            )
            directory.chmod(0o777)
            assert owner_file.read_bytes() == b"parent search truncate payload"
            directory.chmod(0)
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
                "nonowner_chmod_chown_errno=EPERM "
                "access_real_uid=1 root_x_ok=0,1 "
                "open_trunc_denied=1 readonly_truncate_errno=EACCES "
                "parent_truncate_errno=EACCES "
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
