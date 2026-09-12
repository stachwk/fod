#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import signal
import subprocess
import tempfile
import time
from pathlib import Path

from fod_mount import FODMount

DF_TIMEOUT_SECONDS = 8.0
GUARD_TIMEOUT_SECONDS = "2"
GUARD_INTERVAL_SECONDS = "1"


def mountinfo_has_mountpoint(mountpoint: Path) -> bool:
    wanted = str(mountpoint)
    try:
        lines = Path("/proc/self/mountinfo").read_text(
            encoding="utf-8", errors="replace"
        ).splitlines()
    except OSError:
        return False

    for line in lines:
        fields = line.split()
        if len(fields) >= 5 and fields[4].replace("\\040", " ") == wanted:
            return True
    return False


def force_lazy_unmount(mountpoint: Path) -> None:
    if not mountinfo_has_mountpoint(mountpoint):
        return

    for command in (
        ["fusermount3", "-uz", str(mountpoint)],
        ["fusermount", "-uz", str(mountpoint)],
        ["umount", "-l", str(mountpoint)],
    ):
        try:
            result = subprocess.run(
                command,
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=3.0,
            )
        except (FileNotFoundError, subprocess.TimeoutExpired):
            continue
        if result.returncode == 0 and not mountinfo_has_mountpoint(mountpoint):
            return


def rust_fuse_child_pid(bootstrap_pid: int) -> int:
    children_path = Path(f"/proc/{bootstrap_pid}/task/{bootstrap_pid}/children")
    deadline = time.monotonic() + 5.0
    while time.monotonic() < deadline:
        try:
            raw = children_path.read_text(encoding="utf-8").strip()
        except FileNotFoundError:
            raw = ""
        if raw:
            for pid_text in raw.split():
                pid = int(pid_text)
                try:
                    cmdline = Path(f"/proc/{pid}/cmdline").read_bytes()
                except FileNotFoundError:
                    continue
                if b"fod-rust-fuse" in cmdline:
                    return pid
        time.sleep(0.05)
    raise AssertionError(
        f"fod-rust-fuse child not found for bootstrap pid={bootstrap_pid}"
    )


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    previous_timeout = os.environ.get("FOD_FUSE_HANG_GUARD_SECONDS")
    previous_interval = os.environ.get("FOD_FUSE_HANG_GUARD_INTERVAL_SECONDS")
    os.environ["FOD_FUSE_HANG_GUARD_SECONDS"] = GUARD_TIMEOUT_SECONDS
    os.environ["FOD_FUSE_HANG_GUARD_INTERVAL_SECONDS"] = GUARD_INTERVAL_SECONDS

    fuse_pid: int | None = None

    try:
        with tempfile.TemporaryDirectory(prefix="fod-fuse-hang-guard-") as temp_dir:
            mountpoint = Path(temp_dir) / "mount"
            launcher = FODMount(str(root))
            launcher.init_schema()

            try:
                launcher.start(
                    str(mountpoint),
                    log_prefix="/tmp/fod-fuse-hang-guard",
                )
                if launcher.process is None:
                    raise AssertionError("bootstrap process is missing")

                bootstrap_pid = launcher.process.pid
                fuse_pid = rust_fuse_child_pid(bootstrap_pid)

                before = subprocess.run(
                    ["df", "-P", str(mountpoint)],
                    check=False,
                    capture_output=True,
                    text=True,
                    timeout=3.0,
                )
                if before.returncode != 0:
                    raise AssertionError(
                        f"initial df failed rc={before.returncode}: "
                        f"{before.stderr.strip()}"
                    )

                os.kill(fuse_pid, signal.SIGSTOP)

                started = time.monotonic()
                try:
                    after = subprocess.run(
                        ["df", "-P", str(mountpoint)],
                        check=False,
                        capture_output=True,
                        text=True,
                        timeout=DF_TIMEOUT_SECONDS,
                    )
                except subprocess.TimeoutExpired as exc:
                    raise AssertionError(
                        "df remained hung despite FOD FUSE hang guard"
                    ) from exc
                elapsed = time.monotonic() - started

                deadline = time.monotonic() + 4.0
                while launcher.process.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.05)
                if launcher.process.poll() is None:
                    raise AssertionError(
                        "fod-bootstrap did not terminate after hang guard abort"
                    )

                unmount_deadline = time.monotonic() + 4.0
                while (
                    mountinfo_has_mountpoint(mountpoint)
                    and time.monotonic() < unmount_deadline
                ):
                    time.sleep(0.05)
                if mountinfo_has_mountpoint(mountpoint):
                    raise AssertionError(
                        "hang guard aborted FUSE but left a disconnected mountpoint"
                    )

                log_text = launcher.config.log_file.read_text(
                    encoding="utf-8", errors="replace"
                )
                if "FOD FUSE hang guard timeout" not in log_text:
                    raise AssertionError(
                        "hang guard timeout marker missing from bootstrap log"
                    )

                print(
                    "OK fuse-hang-guard "
                    f"bootstrap_pid={bootstrap_pid} "
                    f"fuse_pid={fuse_pid} "
                    f"df_rc={after.returncode} "
                    f"df_elapsed_ms={elapsed * 1000.0:.3f}"
                )
            except Exception:
                launcher._dump_log()
                raise
            finally:
                if fuse_pid is not None:
                    try:
                        os.kill(fuse_pid, signal.SIGCONT)
                    except ProcessLookupError:
                        pass
                force_lazy_unmount(mountpoint)
                launcher.stop()
                force_lazy_unmount(mountpoint)
    finally:
        if previous_timeout is None:
            os.environ.pop("FOD_FUSE_HANG_GUARD_SECONDS", None)
        else:
            os.environ["FOD_FUSE_HANG_GUARD_SECONDS"] = previous_timeout

        if previous_interval is None:
            os.environ.pop("FOD_FUSE_HANG_GUARD_INTERVAL_SECONDS", None)
        else:
            os.environ["FOD_FUSE_HANG_GUARD_INTERVAL_SECONDS"] = previous_interval


if __name__ == "__main__":
    main()
