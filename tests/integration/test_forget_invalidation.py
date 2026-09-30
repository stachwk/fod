#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import re
import tempfile
import time
import uuid
from pathlib import Path

from fod_mount import FODMount

CONTROL_DIR_ENV = "FOD_TEST_FORGET_INVALIDATE_DIR"
CONTROL_NAME_ENV = "FOD_TEST_FORGET_INVALIDATE_NAME"
WAIT_SECONDS = 10.0
FORGET_RE = re.compile(\n    r"FOD forget profile: ino=(\\d+) nlookup=(\\d+) remaining=(\\d+) evicted=(true|false)"\n)


def wait_for_control(control_dir: Path) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    done = control_dir / "done"
    error = control_dir / "error"

    while time.monotonic() < deadline:
        if error.exists():
            raise AssertionError(error.read_text(encoding="utf-8").strip())
        if done.exists():
            return
        time.sleep(0.01)

    raise AssertionError("forget invalidation hook did not complete")


def wait_for_forget(log_file: Path) -> tuple[int, int, int, bool]:
    deadline = time.monotonic() + WAIT_SECONDS
    last = ""

    while time.monotonic() < deadline:
        try:
            last = log_file.read_text(encoding="utf-8")
        except FileNotFoundError:
            last = ""
        matches = FORGET_RE.findall(last)
        if matches:
            ino, nlookup, remaining, evicted = matches[-1]
            return (
                int(ino),
                int(nlookup),
                int(remaining),
                evicted == "true",
            )
        time.sleep(0.01)

    raise AssertionError(
        "kernel did not emit FUSE FORGET after notifier invalidation\n"
        + last[-4000:]
    )


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    fuse_bin = os.environ.get("FOD_RUST_FUSE_BIN", "").strip()
    if not fuse_bin:
        raise AssertionError(
            "FOD_RUST_FUSE_BIN must point to fod-rust-fuse built with "
            "--features integration-test-hooks"
        )

    with tempfile.TemporaryDirectory(prefix="fod-forget-invalidation-") as temp_dir:
        temp = Path(temp_dir)
        mountpoint = temp / "mount"
        control_dir = temp / "control"
        control_dir.mkdir()

        launcher = FODMount(str(root))
        launcher.init_schema()

        name = f"forget-invalidation-{uuid.uuid4().hex}.bin"
        payload = b"FOD deterministic forget invalidation\n"
        previous = {
            CONTROL_DIR_ENV: os.environ.get(CONTROL_DIR_ENV),
            CONTROL_NAME_ENV: os.environ.get(CONTROL_NAME_ENV),
            "FOD_PROFILE_METADATA_CACHE": os.environ.get("FOD_PROFILE_METADATA_CACHE"),
            "FOD_READDIR_REGISTER_PATHS": os.environ.get("FOD_READDIR_REGISTER_PATHS"),
            "FOD_READDIR_BATCH_METADATA": os.environ.get("FOD_READDIR_BATCH_METADATA"),
            "FOD_LOG_LEVEL": os.environ.get("FOD_LOG_LEVEL"),
        }

        os.environ[CONTROL_DIR_ENV] = str(control_dir)
        os.environ[CONTROL_NAME_ENV] = name
        os.environ["FOD_PROFILE_METADATA_CACHE"] = "1"
        os.environ["FOD_READDIR_REGISTER_PATHS"] = "0"
        os.environ["FOD_READDIR_BATCH_METADATA"] = "1"
        os.environ["FOD_LOG_LEVEL"] = "info"

        try:
            launcher.start(
                str(mountpoint),
                log_prefix="/tmp/fod-forget-invalidation",
            )

            target = mountpoint / name
            target.write_bytes(payload)
            before = target.stat()
            if target.read_bytes() != payload:
                raise AssertionError("pre-invalidation payload mismatch")

            (control_dir / "trigger").write_text("invalidate\n", encoding="utf-8")
            wait_for_control(control_dir)

            if launcher.config is None:
                raise AssertionError("mount config unavailable")
            (
                forget_ino,
                forget_nlookup,
                forget_remaining,
                forget_evicted,
            ) = wait_for_forget(launcher.config.log_file)

            if forget_nlookup <= 0:
                raise AssertionError(
                    f"invalid forget nlookup={forget_nlookup} ino={forget_ino}"
                )
            if forget_remaining != 0 or not forget_evicted:
                raise AssertionError(
                    "forget did not retire cached inode path: "
                    f"ino={forget_ino} remaining={forget_remaining} "
                    f"evicted={forget_evicted}"
                )

            after = target.stat()
            if target.read_bytes() != payload:
                raise AssertionError("post-invalidation payload mismatch")
            if after.st_ino != before.st_ino:
                raise AssertionError(
                    "stable inode changed across forget/relookup: "
                    f"before={before.st_ino} after={after.st_ino}"
                )

            target.unlink()
            print(
                "OK forget-invalidation "
                f"ino={forget_ino} nlookup={forget_nlookup} "
                f"remaining={forget_remaining} evicted=1 "
                f"stable_inode={after.st_ino} relookup=1"
            )
        except Exception:
            launcher._dump_log()
            raise
        finally:
            launcher.stop()
            for key, value in previous.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value


if __name__ == "__main__":
    main()
