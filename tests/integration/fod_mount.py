#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import os
import signal
import subprocess
import tempfile
import time
import shutil
import secrets
from dataclasses import dataclass
from pathlib import Path


def _bool_env(value: str) -> bool:
    return value not in {"0", "false", "False", "no", "off", ""}


@dataclass
class MountConfig:
    root: Path
    mountpoint: Path
    log_file: Path
    postgres_db: str
    postgres_user: str
    postgres_password: str
    role: str = "auto"
    selinux: str = "off"
    acl: str = "off"
    default_permissions: bool = True
    atime_policy: str = "default"
    lazytime: bool = False
    sync: bool = False
    dirsync: bool = False
    synchronous_commit: str = "on"
    selinux_context: str = ""
    selinux_fscontext: str = ""
    selinux_defcontext: str = ""
    selinux_rootcontext: str = ""


class FODMount:
    def __init__(self, root: str, *, role: str | None = None):
        self.root = Path(root)
        self.postgres_db = os.environ.get("POSTGRES_DB", "foddbname")
        self.postgres_user = os.environ.get("POSTGRES_USER", "foduser")
        self.postgres_password = os.environ.get("POSTGRES_PASSWORD", "cichosza")
        self.schema_admin_password = os.environ.get("FOD_SCHEMA_ADMIN_PASSWORD") or f"fod-{secrets.token_urlsafe(24)}"
        self.role = (role or os.environ.get("FOD_ROLE", "auto")).lower()
        self.selinux = os.environ.get("FOD_SELINUX", "off")
        self.acl = os.environ.get("FOD_ACL", "off")
        self.default_permissions = _bool_env(os.environ.get("FOD_DEFAULT_PERMISSIONS", "1"))
        self.atime_policy = os.environ.get("FOD_ATIME_POLICY", "default")
        self.lazytime = _bool_env(os.environ.get("FOD_LAZYTIME", "0"))
        self.sync = _bool_env(os.environ.get("FOD_SYNC", "0"))
        self.dirsync = _bool_env(os.environ.get("FOD_DIRSYNC", "0"))
        self.synchronous_commit = os.environ.get("FOD_SYNCHRONOUS_COMMIT", "on")
        self.selinux_context = os.environ.get("FOD_SELINUX_CONTEXT", "")
        self.selinux_fscontext = os.environ.get("FOD_SELINUX_FSCONTEXT", "")
        self.selinux_defcontext = os.environ.get("FOD_SELINUX_DEFCONTEXT", "")
        self.selinux_rootcontext = os.environ.get("FOD_SELINUX_ROOTCONTEXT", "")
        self.process: subprocess.Popen[str] | None = None
        self.config: MountConfig | None = None

    def _runtime_env(self) -> dict[str, str]:
        env = os.environ.copy()
        env["POSTGRES_DB"] = self.postgres_db
        env["POSTGRES_USER"] = self.postgres_user
        env["POSTGRES_PASSWORD"] = self.postgres_password
        return env

    def _resolve_binary(self, env_var: str, candidates: list[Path], missing_error: str) -> Path:
        if path := os.environ.get(env_var):
            candidate = Path(path)
            if candidate.is_file():
                return candidate
        for candidate in candidates:
            if candidate.is_file():
                return candidate
        raise FileNotFoundError(missing_error)

    def _mkfs_binary(self) -> Path:
        return self._resolve_binary(
            "FOD_MKFS_BIN",
            [
                self.root / "target/debug/fod-rust-mkfs",
                self.root / "target/release/fod-rust-mkfs",
                self.root / "rust_mkfs/target/debug/fod-rust-mkfs",
                self.root / "rust_mkfs/target/release/fod-rust-mkfs",
                Path("/usr/local/bin/fod-rust-mkfs"),
            ],
            "fod-rust-mkfs binary not found; build rust_mkfs first",
        )

    def _bootstrap_binary(self) -> Path:
        return self._resolve_binary(
            "FOD_BOOTSTRAP_BIN",
            [
                self.root / "target/debug/fod-bootstrap",
                self.root / "target/release/fod-bootstrap",
                self.root / "rust_mkfs/target/debug/fod-bootstrap",
                self.root / "rust_mkfs/target/release/fod-bootstrap",
                Path("/usr/local/bin/fod-bootstrap"),
            ],
            "fod-bootstrap binary not found; build rust_mkfs first",
        )

    def build_mount_args(self) -> list[str]:
        args = []
        profile = os.environ.get("FOD_PROFILE", "").strip()
        if profile:
            args.extend(["--profile", profile])
        args.extend(["--role", self.role, "--selinux", self.selinux, "--acl", self.acl, "--atime-policy", self.atime_policy])
        args.append("--default-permissions" if self.default_permissions else "--no-default-permissions")
        if self.lazytime:
            args.append("--lazytime")
        if self.sync:
            args.append("--sync")
        if self.dirsync:
            args.append("--dirsync")
        return args

    def init_schema(self) -> None:
        status = subprocess.run(
            [str(self._mkfs_binary()), "status"],
            cwd=self.root,
            env=self._runtime_env(),
            capture_output=True,
            text=True,
            check=False,
        )
        if status.returncode == 0 and "FOD ready: yes" in status.stdout:
            return
        subprocess.run(
            [
                str(self._mkfs_binary()),
                "init",
                "--schema-admin-password",
                self.schema_admin_password,
            ],
            cwd=self.root,
            env=self._runtime_env(),
            check=True,
        )

    def start(self, mountpoint: str, *, log_prefix: str | None = None) -> "FODMount":
        mount_dir = Path(mountpoint)
        mount_dir.mkdir(parents=True, exist_ok=True)
        prefix = log_prefix or f"/tmp/fod-{mount_dir.name}"
        fd, log_path = tempfile.mkstemp(prefix=Path(prefix).name + ".", suffix=".log")
        os.close(fd)
        log_file = Path(log_path)
        self.config = MountConfig(
            root=self.root,
            mountpoint=mount_dir,
            log_file=log_file,
            postgres_db=self.postgres_db,
            postgres_user=self.postgres_user,
            postgres_password=self.postgres_password,
            role=self.role,
            selinux=self.selinux,
            acl=self.acl,
            default_permissions=self.default_permissions,
            atime_policy=self.atime_policy,
            lazytime=self.lazytime,
            sync=self.sync,
            dirsync=self.dirsync,
            synchronous_commit=self.synchronous_commit,
            selinux_context=self.selinux_context,
            selinux_fscontext=self.selinux_fscontext,
            selinux_defcontext=self.selinux_defcontext,
            selinux_rootcontext=self.selinux_rootcontext,
        )
        env = self._runtime_env()
        if self.selinux_context:
            env["FOD_SELINUX_CONTEXT"] = self.selinux_context
        if self.selinux_fscontext:
            env["FOD_SELINUX_FSCONTEXT"] = self.selinux_fscontext
        if self.selinux_defcontext:
            env["FOD_SELINUX_DEFCONTEXT"] = self.selinux_defcontext
        if self.selinux_rootcontext:
            env["FOD_SELINUX_ROOTCONTEXT"] = self.selinux_rootcontext
        env["FOD_SYNCHRONOUS_COMMIT"] = self.synchronous_commit
        env["FOD_USE_FUSE_CONTEXT"] = "1"
        env["FOD_USE_RUST_FUSE"] = "1"

        log_handle = open(log_file, "w", encoding="utf-8")
        self.process = subprocess.Popen(
            [str(self._bootstrap_binary()), *self.build_mount_args(), "-f", str(mount_dir)],
            cwd=self.root,
            env=env,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
            text=True,
        )
        self._log_handle = log_handle

        for _ in range(60):
            if self.mountpoint_ready():
                return self
            if self.process.poll() is not None:
                self._dump_log()
                raise RuntimeError("FOD mount exited before becoming ready")
            time.sleep(1)

        if not self.mountpoint_ready():
            self._dump_log()
            raise RuntimeError("FOD mount did not become ready")
        return self

    def mountpoint_ready(self) -> bool:
        return self.config is not None and self.config.mountpoint.exists() and self._is_mountpoint(self.config.mountpoint)

    @staticmethod
    def _is_mountpoint(path: Path) -> bool:
        return subprocess.run(["mountpoint", "-q", str(path)], check=False).returncode == 0

    def _dump_log(self) -> None:
        if self.config is not None and self.config.log_file.exists():
            print(self.config.log_file.read_text(encoding="utf-8"), end="")

    def stop(self) -> None:
        if self.config is None:
            return

        mountpoint = str(self.config.mountpoint)
        try:
            if self._is_mountpoint(self.config.mountpoint):
                if shutil.which("fusermount3"):
                    subprocess.run(["fusermount3", "-u", mountpoint], check=False)
                elif shutil.which("fusermount"):
                    subprocess.run(["fusermount", "-u", mountpoint], check=False)
                else:
                    subprocess.run(["umount", mountpoint], check=False)
        finally:
            if self.process is not None and self.process.poll() is None:
                self.process.send_signal(signal.SIGTERM)
                try:
                    self.process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=5)
            if hasattr(self, "_log_handle"):
                self._log_handle.close()
            try:
                self.config.log_file.unlink(missing_ok=True)
            except Exception:
                pass
            self.config = None
            self.process = None

    def __enter__(self) -> "FODMount":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.stop()
