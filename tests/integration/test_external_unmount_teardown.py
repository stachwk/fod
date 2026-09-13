#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from fod_mount import FODMount


def is_mountpoint(path: Path) -> bool:
    return subprocess.run(
        ["mountpoint", "-q", str(path)],
        check=False,
    ).returncode == 0


def wait_until_unmounted(path: Path, timeout_seconds: float) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if not is_mountpoint(path):
            return True
        time.sleep(0.05)
    return not is_mountpoint(path)


def external_unmount_command(mountpoint: Path) -> list[str]:
    if shutil.which("fusermount3"):
        return ["fusermount3", "-u", str(mountpoint)]
    if shutil.which("fusermount"):
        return ["fusermount", "-u", str(mountpoint)]
    raise RuntimeError("fusermount3/fusermount is required")


def teardown_einval_lines(log_text: str) -> list[str]:
    matches = []
    for line in log_text.splitlines():
        lowered = line.lower()
        if "invalid argument" not in lowered:
            continue
        if "umount" in lowered or "unmount" in lowered:
            matches.append(line)
    return matches


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    mountpoint = Path(tempfile.mkdtemp(prefix="fod-external-unmount-"))
    mount = FODMount(str(root))

    external_rc = None
    process_rc = None
    mount_gone = False
    log_text = ""

    try:
        mount.start(
            str(mountpoint),
            log_prefix="/tmp/fod-external-unmount-teardown",
        )

        if mount.config is None or mount.process is None:
            raise RuntimeError("FOD mount did not expose config/process state")

        probe_file = mountpoint / "external-unmount-probe.bin"
        with open(probe_file, "wb") as handle:
            handle.write(b"fod-external-unmount-teardown\n")
            handle.flush()
            os.fsync(handle.fileno())

        command = external_unmount_command(mountpoint)
        started = time.monotonic()
        result = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            timeout=15,
        )
        elapsed_ms = (time.monotonic() - started) * 1000.0
        external_rc = result.returncode

        if external_rc != 0:
            raise RuntimeError(
                f"external unmount failed rc={external_rc} "
                f"stderr={result.stderr.strip()}"
            )

        mount_gone = wait_until_unmounted(mountpoint, 5.0)
        if not mount_gone:
            raise RuntimeError("mountpoint remained mounted after external unmount")

        try:
            process_rc = mount.process.wait(timeout=10)
        except subprocess.TimeoutExpired as exc:
            raise RuntimeError(
                "FOD process did not exit after external unmount"
            ) from exc

        if process_rc != 0:
            raise RuntimeError(
                f"FOD process exited non-zero after external unmount: {process_rc}"
            )

        if hasattr(mount, "_log_handle"):
            mount._log_handle.flush()

        log_path = mount.config.log_file
        if log_path.exists():
            log_text = log_path.read_text(
                encoding="utf-8",
                errors="replace",
            )

        einval_lines = teardown_einval_lines(log_text)
        if einval_lines:
            raise RuntimeError(
                "external unmount reproduced teardown EINVAL: "
                + " | ".join(einval_lines)
            )

        print(
            "OK external-unmount-teardown "
            f"external_rc={external_rc} "
            f"elapsed_ms={elapsed_ms:.3f} "
            f"mount_gone={int(mount_gone)} "
            f"process_rc={process_rc} "
            "teardown_einval=0"
        )
        return 0
    finally:
        try:
            mount.stop()
        finally:
            if is_mountpoint(mountpoint):
                raise RuntimeError(
                    f"mount remained after final cleanup: {mountpoint}"
                )
            try:
                mountpoint.rmdir()
            except OSError:
                pass


if __name__ == "__main__":
    raise SystemExit(main())
