#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def build_posix_acl(user_perm: int, group_perm: int, other_perm: int) -> bytes:
    version = 0x0002
    acl = bytearray(struct.pack("<I", version))
    acl.extend(struct.pack("<HHI", 0x0001, user_perm & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0004, group_perm & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0010, group_perm & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0020, other_perm & 0o7, 0xFFFFFFFF))
    return bytes(acl)


def wait_for_mount(mountpoint: Path, process: subprocess.Popen[str], log_path: Path) -> None:
    for _ in range(60):
        if subprocess.run(["mountpoint", "-q", str(mountpoint)], check=False).returncode == 0:
            return
        if process.poll() is not None:
            print(log_path.read_text(encoding="utf-8"), end="")
            raise RuntimeError("mount.fod exited before the mount became ready")
        time.sleep(1)
    print(log_path.read_text(encoding="utf-8"), end="")
    raise RuntimeError("mount.fod did not become ready")


def unmount(mountpoint: Path) -> None:
    commands = []
    if shutil.which("fusermount3"):
        commands.append(["fusermount3", "-u", str(mountpoint)])
    if shutil.which("fusermount"):
        commands.append(["fusermount", "-u", str(mountpoint)])
    commands.append(["umount", str(mountpoint)])

    for command in commands:
        result = subprocess.run(command, check=False, capture_output=True, text=True)
        if result.returncode == 0:
            return
    raise RuntimeError(f"failed to unmount {mountpoint}")


def assert_permission_denied(path: Path) -> None:
    if os.geteuid() == 0:
        print("SKIP acl-owner-read-denial (root can bypass DAC)")
        return

    try:
        path.read_bytes()
    except PermissionError:
        return
    raise AssertionError("ACL deny entry did not block file read")


def expected_optional_selinux_errno(err_no: int | None) -> bool:
    expected_errno = {
        errno.EPERM,
        errno.EACCES,
        errno.ENOTSUP,
        getattr(errno, "EOPNOTSUPP", errno.ENOTSUP),
    }
    return err_no in expected_errno


def assert_capability_enabled(log_text: str, capability: str) -> None:
    compatibility_line = next(
        (line for line in log_text.splitlines() if "FOD FUSE compatibility:" in line),
        "",
    )
    for field in ("fod_requested_capabilities=", "fod_enabled_capabilities="):
        assert field in compatibility_line, compatibility_line
        value = compatibility_line.split(field, 1)[1].split(" ", 1)[0]
        assert capability in value, compatibility_line


def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    suffix = uuid.uuid4().hex[:8]
    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-acl-mount-option-{suffix}.") as tmpdir:
        tmp_path = Path(tmpdir)
        mountpoint = tmp_path / "mnt"
        mountpoint.mkdir()
        log_path = tmp_path / "mount.log"

        env = os.environ.copy()
        env["POSTGRES_DB"] = launcher.postgres_db
        env["POSTGRES_USER"] = launcher.postgres_user
        env["POSTGRES_PASSWORD"] = launcher.postgres_password
        env["FOD_BOOTSTRAP_BIN"] = str(launcher._bootstrap_binary())
        env["FOD_USE_RUST_FUSE"] = "1"
        env["FOD_USE_FUSE_CONTEXT"] = "1"

        selinux_mode = os.environ.get("FOD_TEST_ACL_MOUNT_SELINUX", "off")
        options = (
            f"ini={launcher._config_path()},role=auto,acl=on,"
            f"selinux={selinux_mode},default_permissions"
        )
        with log_path.open("w", encoding="utf-8") as log_handle:
            process = subprocess.Popen(
                [str(ROOT / "mount.fod"), "none", str(mountpoint), "-o", options],
                cwd=ROOT,
                env=env,
                stdout=log_handle,
                stderr=subprocess.STDOUT,
                text=True,
            )
            try:
                wait_for_mount(mountpoint, process, log_path)

                log_text = log_path.read_text(encoding="utf-8")
                assert "acl_enabled=true" in log_text, log_text
                assert_capability_enabled(log_text, "POSIX_ACL")
                if selinux_mode == "on":
                    assert "selinux_enabled=true" in log_text, log_text

                file_path = mountpoint / f"acl-{suffix}.txt"
                file_path.write_bytes(b"acl payload\n")
                if selinux_mode == "on":
                    selinux_value = b"system_u:object_r:tmp_t:s0"
                    try:
                        os.setxattr(file_path, "security.selinux", selinux_value)
                    except OSError as exc:
                        assert expected_optional_selinux_errno(exc.errno), (
                            f"unexpected SELinux xattr errno: {exc.errno}"
                        )
                        print(
                            "SKIP acl-mount-option-selinux-xattr "
                            f"(security.selinux unavailable, errno={exc.errno})"
                        )
                    else:
                        assert os.getxattr(file_path, "security.selinux") == selinux_value
                        assert "security.selinux" in os.listxattr(file_path)
                allow_acl = build_posix_acl(user_perm=0o6, group_perm=0o0, other_perm=0o0)
                os.setxattr(file_path, "system.posix_acl_access", allow_acl)
                assert os.getxattr(file_path, "system.posix_acl_access") == allow_acl
                assert stat.S_IMODE(file_path.stat().st_mode) == 0o600
                assert "system.posix_acl_access" in os.listxattr(file_path)
                assert os.access(file_path, os.R_OK)
                assert os.access(file_path, os.W_OK)
                assert not os.access(file_path, os.X_OK)

                acl_dir_path = mountpoint / f"acl-default-{suffix}"
                acl_dir_path.mkdir()
                os.setxattr(acl_dir_path, "system.posix_acl_default", allow_acl)
                child_path = acl_dir_path / "child.txt"
                child_path.write_bytes(b"inherited acl\n")
                assert os.getxattr(child_path, "system.posix_acl_access") == allow_acl

                deny_acl = build_posix_acl(user_perm=0o0, group_perm=0o0, other_perm=0o0)
                os.setxattr(file_path, "system.posix_acl_access", deny_acl)
                assert os.getxattr(file_path, "system.posix_acl_access") == deny_acl
                assert stat.S_IMODE(file_path.stat().st_mode) == 0o000
                if os.geteuid() != 0:
                    assert not os.access(file_path, os.R_OK)
                assert_permission_denied(file_path)

                print("OK acl-mount-option")
            finally:
                try:
                    unmount(mountpoint)
                finally:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)


if __name__ == "__main__":
    main()
