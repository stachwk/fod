#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import ctypes
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

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount

# Linux UAPI values, used here to exercise the mounted Linux FUSE path directly.
UTIME_NOW = (1 << 30) - 1
UTIME_OMIT = (1 << 30) - 2


class Timespec(ctypes.Structure):
    _fields_ = [("tv_sec", ctypes.c_long), ("tv_nsec", ctypes.c_long)]


def call_utimensat(path: Path, atime_nsec: int, mtime_nsec: int) -> None:
    times = (Timespec * 2)(Timespec(0, atime_nsec), Timespec(0, mtime_nsec))
    libc = ctypes.CDLL(None, use_errno=True)
    libc.utimensat.argtypes = [
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.POINTER(Timespec),
        ctypes.c_int,
    ]
    libc.utimensat.restype = ctypes.c_int
    result = libc.utimensat(-100, os.fsencode(path), times, 0)
    if result != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), str(path))


def assert_invalid_utimensat_nanoseconds(path: Path) -> None:
    before = path.stat()
    invalid_fields = (
        (1_000_000_000, UTIME_OMIT),
        (-1, UTIME_OMIT),
        (UTIME_OMIT, 1_000_000_000),
        (UTIME_OMIT, -1),
    )
    for atime_nsec, mtime_nsec in invalid_fields:
        try:
            call_utimensat(path, atime_nsec, mtime_nsec)
        except OSError as error:
            assert error.errno == errno.EINVAL, (
                atime_nsec,
                mtime_nsec,
                error.errno,
            )
        else:
            raise AssertionError(
                "utimensat accepted an invalid tv_nsec value: "
                f"atime={atime_nsec} mtime={mtime_nsec}"
            )
    after = path.stat()
    assert after.st_atime_ns == before.st_atime_ns
    assert after.st_mtime_ns == before.st_mtime_ns


def build_posix_acl(user_perm: int, group_perm: int, other_perm: int) -> bytes:
    version = 0x0002
    acl = bytearray(struct.pack("<I", version))
    acl.extend(struct.pack("<HHI", 0x0001, user_perm & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0004, group_perm & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0010, group_perm & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0020, other_perm & 0o7, 0xFFFFFFFF))
    return bytes(acl)


def build_named_user_acl(
    uid: int,
    permissions: int,
    *,
    user_obj: int = 0o0,
    mask: int | None = None,
    other: int = 0o0,
) -> bytes:
    if mask is None:
        mask = permissions
    acl = bytearray(struct.pack("<I", 0x0002))
    acl.extend(struct.pack("<HHI", 0x0001, user_obj & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0002, permissions & 0o7, uid))
    acl.extend(struct.pack("<HHI", 0x0004, 0o0, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0010, mask & 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0020, other & 0o7, 0xFFFFFFFF))
    return bytes(acl)


def build_directory_acl_for_named_user(uid: int, permissions: int) -> bytes:
    acl = bytearray(struct.pack("<I", 0x0002))
    acl.extend(struct.pack("<HHI", 0x0001, 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0002, permissions & 0o7, uid))
    acl.extend(struct.pack("<HHI", 0x0004, 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0010, 0o7, 0xFFFFFFFF))
    acl.extend(struct.pack("<HHI", 0x0020, 0o7, 0xFFFFFFFF))
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


def assert_namespace_mutation_denied_to_nobody(
    operation: str, *paths: Path
) -> None:
    script = """
import os
import stat
import sys

operation, *paths = sys.argv[1:]
try:
    if operation == "unlink":
        os.unlink(paths[0])
    elif operation == "rmdir":
        os.rmdir(paths[0])
    elif operation == "rename":
        os.rename(paths[0], paths[1])
    elif operation == "create":
        fd = os.open(paths[0], os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.close(fd)
    elif operation == "mkdir":
        os.mkdir(paths[0])
    elif operation == "mknod":
        os.mknod(paths[0], stat.S_IFIFO | 0o600)
    elif operation == "symlink":
        os.symlink("missing-target", paths[0])
    elif operation == "link":
        os.link(paths[0], paths[1])
    else:
        raise AssertionError(f"unknown namespace operation: {operation}")
except OSError as error:
    print(error.errno)
else:
    print(0)
"""
    result = subprocess.run(
        [
            "sudo",
            "-n",
            "-u",
            "nobody",
            "python3",
            "-c",
            script,
            operation,
            *(str(path) for path in paths),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == str(errno.EACCES), (
        operation,
        paths,
        result.stdout,
        result.stderr,
    )


def assert_parent_acl_denies_namespace_mutations(mountpoint: Path, suffix: str) -> None:
    if not can_run_as_nobody():
        return

    nobody_uid = pwd.getpwnam("nobody").pw_uid
    if nobody_uid == os.geteuid():
        print("SKIP acl-parent-namespace-denial (test process is nobody)")
        return

    denied_parent = mountpoint / f"acl-denied-parent-{suffix}"
    denied_parent.mkdir(mode=0o777)
    os.setxattr(
        denied_parent,
        "system.posix_acl_access",
        build_directory_acl_for_named_user(nobody_uid, 0o1),
    )
    assert stat.S_IMODE(denied_parent.stat().st_mode) == 0o777

    unlink_target = denied_parent / "unlink-target"
    unlink_target.write_bytes(b"preserve unlink target")
    rmdir_target = denied_parent / "rmdir-target"
    rmdir_target.mkdir()
    assert_namespace_mutation_denied_to_nobody("unlink", unlink_target)
    assert_namespace_mutation_denied_to_nobody("rmdir", rmdir_target)
    assert unlink_target.read_bytes() == b"preserve unlink target"
    assert rmdir_target.is_dir()

    allowed_parent = mountpoint / f"acl-allowed-parent-{suffix}"
    allowed_parent.mkdir(mode=0o777)

    create_source = allowed_parent / "hardlink-source"
    create_source.write_bytes(b"hardlink source")
    chown_source = subprocess.run(
        ["sudo", "-n", "chown", str(nobody_uid), str(create_source)],
        check=False,
        capture_output=True,
        text=True,
    )
    assert chown_source.returncode == 0, chown_source.stderr
    assert create_source.stat().st_uid == nobody_uid
    denied_create_paths = {
        "create": denied_parent / "create-target",
        "mkdir": denied_parent / "mkdir-target",
        "mknod": denied_parent / "mknod-target",
        "symlink": denied_parent / "symlink-target",
    }
    for operation, target in denied_create_paths.items():
        assert_namespace_mutation_denied_to_nobody(operation, target)
    denied_hardlink = denied_parent / "hardlink-target"
    assert_namespace_mutation_denied_to_nobody("link", create_source, denied_hardlink)
    assert all(not path.exists() for path in denied_create_paths.values())
    assert not denied_hardlink.exists()
    assert create_source.read_bytes() == b"hardlink source"

    denied_source = denied_parent / "rename-source-denied"
    denied_source.write_bytes(b"preserve source")
    allowed_destination = allowed_parent / "rename-destination-denied-source"
    assert_namespace_mutation_denied_to_nobody(
        "rename", denied_source, allowed_destination
    )
    assert denied_source.read_bytes() == b"preserve source"
    assert not allowed_destination.exists()

    allowed_source = allowed_parent / "rename-source-denied-destination"
    allowed_source.write_bytes(b"preserve destination")
    denied_destination = denied_parent / "rename-destination"
    assert_namespace_mutation_denied_to_nobody(
        "rename", allowed_source, denied_destination
    )
    assert allowed_source.read_bytes() == b"preserve destination"
    assert not denied_destination.exists()


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
    before_payload = b"acl payload\n"
    expected_size = len(b"acl payload\n")
    truncate_result = subprocess.run(
        [
            "sudo",
            "-n",
            "-u",
            "nobody",
            "python3",
            "-c",
            """
import os
import sys
try:
    os.truncate(sys.argv[1], 0)
except OSError as error:
    print(error.errno)
else:
    print(0)
""",
            str(path),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    assert truncate_result.returncode == 0 and truncate_result.stdout.strip() == str(errno.EACCES), (
        "ACL-denied truncate returned "
        f"errno={truncate_result.stdout.strip()!r}, expected {errno.EACCES}; "
        f"stderr={truncate_result.stderr!r}"
    )

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
    connection = psycopg2.connect(
        host=os.environ.get("FOD_PG_HOST") or os.environ.get("POSTGRES_HOST") or "127.0.0.1",
        port=int(os.environ.get("FOD_PG_PORT") or os.environ.get("POSTGRES_PORT") or "5432"),
        dbname=os.environ.get("FOD_PG_DBNAME") or os.environ.get("POSTGRES_DB") or "foddbname",
        user=os.environ.get("FOD_PG_USER") or os.environ.get("POSTGRES_USER") or "foduser",
        password=os.environ.get("FOD_PG_PASSWORD") or os.environ.get("POSTGRES_PASSWORD") or "cichosza",
    )
    try:
        with connection.cursor() as cursor:
            cursor.execute(
                "SELECT d.file_size, encode(b.data, 'hex') "
                "FROM fod.files f "
                "JOIN fod.data_objects d ON d.id_data_object = f.data_object_id "
                "JOIN fod.data_blocks b "
                "  ON b.data_object_id = d.id_data_object AND b._order = 0 "
                "WHERE f.name = %s AND NOT f.unlinked",
                (path.name,),
            )
            state = cursor.fetchone()
        assert state is not None, f"ACL-denied truncate removed payload for {path.name}"
        database_size, block_hex = state
        assert database_size == expected_size, (expected_size, database_size)
        assert bytes.fromhex(block_hex).startswith(before_payload)
    finally:
        connection.close()

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
missing = os.fsencode(sys.argv[1] + ".utime-omit-missing")
result = libc.utimensat(-100, missing, times, 0)
if result != 0:
    error = ctypes.get_errno()
    raise OSError(error, os.strerror(error), os.fsdecode(missing))
""",
            str(path),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    assert omit_times.returncode == 0, omit_times.stderr
    assert not Path(f"{path}.utime-omit-missing").exists()
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


def assert_utime_now_omit_field_independence(path: Path) -> None:
    before = path.stat()
    call_utimensat(path, UTIME_OMIT, UTIME_NOW)
    after_mtime = path.stat()
    assert after_mtime.st_atime_ns == before.st_atime_ns, (
        before.st_atime_ns,
        after_mtime.st_atime_ns,
    )
    assert after_mtime.st_mtime_ns >= before.st_mtime_ns, (
        before.st_mtime_ns,
        after_mtime.st_mtime_ns,
    )

    call_utimensat(path, UTIME_NOW, UTIME_OMIT)
    after_atime = path.stat()
    assert after_atime.st_atime_ns >= after_mtime.st_atime_ns, (
        after_mtime.st_atime_ns,
        after_atime.st_atime_ns,
    )
    assert after_atime.st_mtime_ns == after_mtime.st_mtime_ns, (
        after_mtime.st_mtime_ns,
        after_atime.st_mtime_ns,
    )


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


def assert_open_preserves_malformed_acl_errno(
    path: Path, acl_value: bytes, launcher: FODMount, log_path: Path
) -> None:
    connection = psycopg2.connect(
        host=os.environ.get("FOD_PG_HOST") or os.environ.get("POSTGRES_HOST") or "127.0.0.1",
        port=int(os.environ.get("FOD_PG_PORT") or os.environ.get("POSTGRES_PORT") or "5432"),
        dbname=launcher.postgres_db,
        user=launcher.postgres_user,
        password=launcher.postgres_password,
    )
    connection.autocommit = True
    malformed_acl = b"\x03\x00\x00\x00"
    try:
        with connection.cursor() as cursor:
            cursor.execute(
                "UPDATE fod.xattrs AS x SET value = %s "
                "FROM fod.files AS f "
                "WHERE x.owner_kind = 'file' AND x.owner_id = f.id_file "
                "AND f.name = %s AND NOT f.unlinked "
                "AND x.name = 'system.posix_acl_access'",
                (malformed_acl, path.name),
            )
            assert cursor.rowcount == 1, (
                f"expected one ACL row for {path.name}, got {cursor.rowcount}"
            )

        try:
            fd = os.open(path, os.O_RDONLY)
        except OSError as error:
            assert error.errno == errno.EINVAL, (
                f"open returned errno={error.errno}, expected {errno.EINVAL}"
            )
        else:
            os.close(fd)
            raise AssertionError("open accepted a malformed stored access ACL")

        callback_log = log_path.read_text(encoding="utf-8")
        assert f"errno={errno.EINVAL}" in callback_log and "acl_check" in callback_log, (
            "open did not report the ACL evaluation error from the FOD callback"
        )
    finally:
        with connection.cursor() as cursor:
            cursor.execute(
                "UPDATE fod.xattrs AS x SET value = %s "
                "FROM fod.files AS f "
                "WHERE x.owner_kind = 'file' AND x.owner_id = f.id_file "
                "AND f.name = %s AND NOT f.unlinked "
                "AND x.name = 'system.posix_acl_access'",
                (acl_value, path.name),
            )
            assert cursor.rowcount == 1, f"failed to restore ACL for {path.name}"
        connection.close()


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
                assert_open_preserves_malformed_acl_errno(
                    file_path, allow_acl, launcher, log_path
                )

                nobody = pwd.getpwnam("nobody")
                nobody_group = grp.getgrgid(nobody.pw_gid)
                kill_sgid_acl_path = mountpoint / f"acl-kill-sgid-{suffix}.txt"
                kill_sgid_acl_path.write_bytes(b"setgid ACL payload\n")
                subprocess.run(
                    [
                        "sudo",
                        "-n",
                        "chown",
                        f"{nobody.pw_uid}:0",
                        str(kill_sgid_acl_path),
                    ],
                    check=True,
                    capture_output=True,
                    text=True,
                )
                subprocess.run(
                    ["sudo", "-n", "chmod", "2755", str(kill_sgid_acl_path)],
                    check=True,
                    capture_output=True,
                    text=True,
                )
                assert kill_sgid_acl_path.stat().st_mode & 0o2000
                if can_run_as_nobody():
                    set_acl_as_owner = subprocess.run(
                        [
                            "sudo",
                            "-n",
                            "-u",
                            "nobody",
                            sys.executable,
                            "-c",
                            "import os,sys; os.setxattr(sys.argv[1], 'system.posix_acl_access', bytes.fromhex(sys.argv[2]))",
                            str(kill_sgid_acl_path),
                            allow_acl.hex(),
                        ],
                        check=False,
                        capture_output=True,
                        text=True,
                    )
                    assert set_acl_as_owner.returncode == 0, set_acl_as_owner.stderr
                    assert stat.S_IMODE(kill_sgid_acl_path.stat().st_mode) == 0o600
                    assert kill_sgid_acl_path.stat().st_mode & 0o2000 == 0
                else:
                    print("SKIP acl-kill-sgid (sudo cannot run commands as nobody)")

                chmod_acl_path = mountpoint / f"acl-chmod-{suffix}.txt"
                chmod_acl_path.write_bytes(b"chmod ACL payload\n")
                nobody_uid = pwd.getpwnam("nobody").pw_uid
                named_acl = build_named_user_acl(nobody_uid, 0o6)
                os.setxattr(chmod_acl_path, "system.posix_acl_access", named_acl)
                assert stat.S_IMODE(chmod_acl_path.stat().st_mode) == 0o060
                os.chmod(chmod_acl_path, 0o640)
                expected_chmod_acl = build_named_user_acl(
                    nobody_uid, 0o6, user_obj=0o6, mask=0o4
                )
                actual_chmod_acl = os.getxattr(
                    chmod_acl_path, "system.posix_acl_access"
                )
                assert actual_chmod_acl == expected_chmod_acl, (
                    actual_chmod_acl.hex(),
                    expected_chmod_acl.hex(),
                )
                assert stat.S_IMODE(chmod_acl_path.stat().st_mode) == 0o640

                chmod_acl_dir = mountpoint / f"acl-chmod-dir-{suffix}"
                chmod_acl_dir.mkdir()
                os.setxattr(chmod_acl_dir, "system.posix_acl_access", named_acl)
                assert stat.S_IMODE(chmod_acl_dir.stat().st_mode) == 0o060
                os.chmod(chmod_acl_dir, 0o750)
                expected_chmod_dir_acl = build_named_user_acl(
                    nobody_uid, 0o6, user_obj=0o7, mask=0o5
                )
                actual_chmod_dir_acl = os.getxattr(
                    chmod_acl_dir, "system.posix_acl_access"
                )
                assert actual_chmod_dir_acl == expected_chmod_dir_acl, (
                    actual_chmod_dir_acl.hex(),
                    expected_chmod_dir_acl.hex(),
                )
                assert stat.S_IMODE(chmod_acl_dir.stat().st_mode) == 0o750

                acl_dir_path = mountpoint / f"acl-default-{suffix}"
                acl_dir_path.mkdir()
                os.setxattr(acl_dir_path, "system.posix_acl_default", allow_acl)
                child_path = acl_dir_path / "child.txt"
                child_path.write_bytes(b"inherited acl\n")
                assert os.getxattr(child_path, "system.posix_acl_access") == allow_acl

                assert_parent_acl_denies_namespace_mutations(mountpoint, suffix)

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
                assert_invalid_utimensat_nanoseconds(timestamp_path)
                assert_utime_now_omit_field_independence(timestamp_path)
                assert_non_owner_timestamp_rules(timestamp_path)

                print(
                    "OK acl-mount-option utimens_invalid_nsec=EINVAL "
                    "acl_open_malformed_acl_errno=EINVAL "
                    "acl_denied_truncate_errno=EACCES "
                    "parent_namespace_create_unlink_rmdir_rename=EACCES"
                )
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
