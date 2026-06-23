#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import os
import subprocess
import time
from pathlib import Path


def set_env(overrides: dict[str, str]) -> dict[str, str | None]:
    original = {name: os.environ.get(name) for name in overrides}
    for name, value in overrides.items():
        os.environ[name] = value
    return original


def restore_env(original: dict[str, str | None]) -> None:
    for name, value in original.items():
        if value is None:
            os.environ.pop(name, None)
        else:
            os.environ[name] = value


def wait_for_log_contains(log_file: Path, needle: str, timeout_s: float = 15.0) -> str:
    deadline = time.monotonic() + timeout_s
    last_text = ""
    while time.monotonic() < deadline:
        if log_file.exists():
            last_text = log_file.read_text(encoding="utf-8")
            if needle in last_text:
                return last_text
        time.sleep(0.1)
    raise AssertionError(f"timed out waiting for {needle!r} in {log_file}:\n{last_text}")


def require_root(script_name: str) -> None:
    if os.geteuid() != 0:
        raise SystemExit(f"{script_name} must be run via sudo")


def resolve_fod_change_binary(root: Path) -> Path:
    candidates = [
        root / "target/debug/fod-change",
        root / "target/release/fod-change",
        root / "rust_mkfs/target/debug/fod-change",
        root / "rust_mkfs/target/release/fod-change",
        Path("/usr/local/bin/fod-change"),
        Path("/usr/local/bin/fod.change"),
    ]
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise FileNotFoundError("fod-change binary not found; build rust_mkfs first")


def run_fod_change(root: Path, args: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        [str(resolve_fod_change_binary(root)), *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if check and result.returncode != 0:
        raise AssertionError(
            f"fod.change failed: {' '.join(args)}\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )
    return result
