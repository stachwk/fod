#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import grp
import os
import pwd
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


def build_named_user_acl(uid: int, permissions: int) -> bytes:
    acl = bytearray(struct.pack("<I", 0x0002))
    acl.extend(struct.pack("<HHI", 0x0001, 0o0, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0002, permissions & 0o7, uid))
    acl.extend(struct.pack("<HHI", 0x0004, 0o0, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0010, permissions & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0020, 0o0, 0xFFFFFFFF))
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


def can_run_as_nobody() -> bool:
    nobody_probe = subprocess.run(
        ["sudo", "-n", "-u", "nobody", "true"],
        check=False,
        capture_output=True,
        text=True,
    )
    if nobody_probe.returncode != 0:
        print("SKIP acl-mutation-denial (sudo cannot run commands as nobody)")
        return False
    return True


def assert_mutations_denied_to_nobody(path: Path) -> None:
    if not can_run_as_nobody():
        return

    stat_result = subprocess.run(
        ["sudo", "-n", "-u", "nobody", "stat", "--format=%s", str(path)],
        check=False,
        capture_output=True,
        text=True,
    )
    assert stat_result.returncode == 0, (
        "nobody could not reach the ACL test file through the mount: "
        + stat_result.stderr
    )

    before = path.stat()
    expected_size = len(b"acl payload\n")
    truncate_result = subprocess.run(
        ["sudo", "-n", "-u", "nobody", "truncate", "-s", "0", str(path)],
        check=False,
        capture_output=True,
        text=True,
    )
    assert truncate_result.returncode != 0, "ACL-denied truncate unexpectedly succeeded"

    timestamp_result = subprocess.run(
        ["sudo", "-n", "-u", "nobody", "touch", "-d", "@1234567890", str(path)],
        check=False,
        capture_output=True,
        text=True,
    )
    assert timestamp_result.returncode != 0, "ACL-denied timestamp update unexpectedly succeeded"
    after = path.stat()
    assert after.st_size == expected_size, (expected_size, after.st_size)
    assert after.st_mtime_ns == before.st_mtime_ns, (before.st_mtime_ns, after.st_mtime_ns)

    omit_times = subprocess.run(
        [
            "sudo",
            "-n",
            "-u",
            "nobody",
            "python3",
            "-c",
            """
import ctypes
import os
import sys

class Timespec(ctypes.Structure):
    _fields_ = [("tv_sec", ctypes.c_long), ("tv_nsec", ctypes.c_long)]

omit = (1 << 30) - 2
times = (Timespec * 2)(Timespec(0, omit), Timespec(0, omit))
libc = ctypes.CDLL(None, use_errno=True)
libc.utimensat.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.POINTER(Timespec), ctypes.c_int]
libc.utimensat.restype = ctypes.c_int
result = libc.utimensat(-100, os.fsencode(sys.argv[1]), times, 0)
if result != 0:
    error = ctypes.get_errno()
    raise OSError(error, os.strerror(error), sys.argv[1])
""",
            str(path),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    assert omit_times.returncode == 0, omit_times.stderr
    after_omit = path.stat()
    assert after_omit.st_atime_ns == after.st_atime_ns, (
        after.st_atime_ns,
        after_omit.st_atime_ns,
    )
    assert after_omit.st_mtime_ns == after.st_mtime_ns, (
        after.st_mtime_ns,
        after_omit.st_mtime_ns,
    )


def assert_non_owner_timestamp_rules(path: Path) -> None:
    root_probe = subprocess.run(
        ["sudo", "-n", "true"], check=False, capture_output=True, text=True
    )
    if root_probe.returncode != 0:
        print("SKIP acl-utimens-owner-rules (sudo cannot change test file owner)")
        return

    actor_uid = os.getuid()
    nobody = pwd.getpwnam("nobody")
    nobody_group = grp.getgrgid(nobody.pw_gid)
    os.setxattr(path, "system.posix_acl_access", build_named_user_acl(actor_uid, 0o2))
    chown_result = subprocess.run(
        ["sudo", "-n", "chown", f"{nobody.pw_uid}:{nobody_group.gr_gid}", str(path)],
        check=False,
        capture_output=True,
        text=True,
    )
    assert chown_result.returncode == 0, chown_result.stderr
    assert path.stat().st_uid == nobody.pw_uid
    assert os.access(path, os.W_OK), "named ACL write permission was not honored"

    before_now = path.stat().st_mtime_ns
    os.utime(path, None)
    after_now = path.stat().st_mtime_ns
    assert after_now >= before_now, (before_now, after_now)

    explicit_ns = 1_234_567_890_000_000_000
    try:
        os.utime(path, ns=(explicit_ns, explicit_ns))
    except PermissionError as exc:
        assert exc.errno == errno.EPERM, exc.errno
    else:
        raise AssertionError("non-owner with ACL write access set explicit timestamps")
    assert path.stat().st_mtime_ns == after_now, (after_now, path.stat().st_mtime_ns)


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
        os.chmod(tmp_path, 0o755)
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
            f"selinux={selinux_mode},default_permissions,allow_other"
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
                assert_mutations_denied_to_nobody(file_path)

                timestamp_path = mountpoint / f"acl-timestamp-{suffix}.txt"
                timestamp_path.write_bytes(b"timestamp payload\n")
                assert_non_owner_timestamp_rules(timestamp_path)

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
