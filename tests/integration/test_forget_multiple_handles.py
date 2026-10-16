#!/usr/bin/env python
# -*- coding: utf-8 -*-
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

FORGET_RE = re.compile(
    r"FOD forget profile: ino=(\d+) nlookup=(\d+) remaining=(\d+) evicted=(true|false)"
)
RELEASE_RE = re.compile(
    r"FOD release cache profile: fh=(\d+) ino=(\d+) evicted=(true|false) "
    r"lookup_remaining=(\d+) inode_cached=(true|false) path_cached=(true|false)"
)


def read_log(log_file: Path) -> str:
    try:
        return log_file.read_text(encoding="utf-8")
    except FileNotFoundError:
        return ""


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

    raise AssertionError("forget multiple-handles invalidation hook did not complete")


def assert_no_forget(log_file: Path, target_ino: int, duration: float = 0.5) -> None:
    deadline = time.monotonic() + duration
    while time.monotonic() < deadline:
        matching = [
            match
            for match in FORGET_RE.findall(read_log(log_file))
            if int(match[0]) == target_ino
        ]
        if matching:
            raise AssertionError(
                "kernel emitted FUSE FORGET while at least one file handle was active"
            )
        time.sleep(0.01)


def release_matches(log_file: Path, target_ino: int) -> list[tuple[str, ...]]:
    return [
        match
        for match in RELEASE_RE.findall(read_log(log_file))
        if int(match[1]) == target_ino
    ]


def wait_for_release(
    log_file: Path,
    target_ino: int,
    previous_count: int,
) -> tuple[int, bool, int, bool, bool]:
    deadline = time.monotonic() + WAIT_SECONDS
    last = ""

    while time.monotonic() < deadline:
        last = read_log(log_file)
        matching = [
            match
            for match in RELEASE_RE.findall(last)
            if int(match[1]) == target_ino
        ]
        if len(matching) > previous_count:
            fh, _, evicted, lookup_remaining, inode_cached, path_cached = matching[-1]
            return (
                int(fh),
                evicted == "true",
                int(lookup_remaining),
                inode_cached == "true",
                path_cached == "true",
            )
        time.sleep(0.01)

    raise AssertionError(
        "FUSE release did not emit multiple-handle cache profile\n"
        + last[-4000:]
    )


def wait_for_forget(log_file: Path, target_ino: int) -> tuple[int, int, bool]:
    deadline = time.monotonic() + WAIT_SECONDS
    last = ""

    while time.monotonic() < deadline:
        last = read_log(log_file)
        matching = [
            match
            for match in FORGET_RE.findall(last)
            if int(match[0]) == target_ino
        ]
        if matching:
            _, nlookup, remaining, evicted = matching[-1]
            return int(nlookup), int(remaining), evicted == "true"
        time.sleep(0.01)

    raise AssertionError(
        "kernel did not emit deferred FUSE FORGET after final handle release\n"
        + last[-4000:]
    )


def assert_release_retains_cache(
    *,
    label: str,
    fh: int,
    evicted: bool,
    lookup_remaining: int,
    inode_cached: bool,
    path_cached: bool,
) -> None:
    if evicted:
        raise AssertionError(f"{label} release evicted inode early fh={fh}")
    if lookup_remaining <= 0:
        raise AssertionError(
            f"{label} release unexpectedly observed no lookup ref fh={fh}"
        )
    if not inode_cached or not path_cached:
        raise AssertionError(
            f"{label} release dropped inode/path cache early fh={fh} "
            f"inode_cached={inode_cached} path_cached={path_cached}"
        )


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    fuse_bin = os.environ.get("FOD_RUST_FUSE_BIN", "").strip()
    if not fuse_bin:
        raise AssertionError(
            "FOD_RUST_FUSE_BIN must point to fod-rust-fuse built with "
            "--features integration-test-hooks"
        )

    with tempfile.TemporaryDirectory(prefix="fod-forget-multiple-handles-") as temp_dir:
        temp = Path(temp_dir)
        mountpoint = temp / "mount"
        control_dir = temp / "control"
        control_dir.mkdir()

        launcher = FODMount(str(root))
        launcher.init_schema()

        name = f"forget-multiple-handles-{uuid.uuid4().hex}.bin"
        payload = b"FOD multiple active handles survive invalidation\n"

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

        first = None
        second = None
        try:
            launcher.start(
                str(mountpoint),
                log_prefix="/tmp/fod-forget-multiple-handles",
            )
            if launcher.config is None:
                raise AssertionError("mount config unavailable")

            target = mountpoint / name
            target.write_bytes(payload)
            target_ino = target.stat().st_ino

            first = target.open("rb")
            second = target.open("rb")

            if first.read() != payload or second.read() != payload:
                raise AssertionError("pre-invalidation multiple-handle payload mismatch")
            first.seek(0)
            second.seek(0)

            (control_dir / "trigger").write_text("invalidate\n", encoding="utf-8")
            wait_for_control(control_dir)

            assert_no_forget(launcher.config.log_file, target_ino)

            if first.read() != payload or second.read() != payload:
                raise AssertionError("active handles stopped reading after invalidation")
            first.seek(0)
            second.seek(0)

            releases_before_first = len(
                release_matches(launcher.config.log_file, target_ino)
            )
            first.close()
            first = None

            first_release = wait_for_release(
                launcher.config.log_file,
                target_ino,
                releases_before_first,
            )
            assert_release_retains_cache(
                label="first",
                fh=first_release[0],
                evicted=first_release[1],
                lookup_remaining=first_release[2],
                inode_cached=first_release[3],
                path_cached=first_release[4],
            )

            assert_no_forget(launcher.config.log_file, target_ino)
            if second.read() != payload:
                raise AssertionError("second handle stopped reading after first release")
            second.seek(0)

            releases_before_second = len(
                release_matches(launcher.config.log_file, target_ino)
            )
            second.close()
            second = None

            second_release = wait_for_release(
                launcher.config.log_file,
                target_ino,
                releases_before_second,
            )
            assert_release_retains_cache(
                label="second",
                fh=second_release[0],
                evicted=second_release[1],
                lookup_remaining=second_release[2],
                inode_cached=second_release[3],
                path_cached=second_release[4],
            )

            nlookup, remaining, evicted = wait_for_forget(
                launcher.config.log_file,
                target_ino,
            )
            if nlookup <= 0:
                raise AssertionError(
                    f"invalid deferred forget nlookup={nlookup} ino={target_ino}"
                )
            if remaining != 0 or not evicted:
                raise AssertionError(
                    "deferred FORGET did not retire inode after final handle release: "
                    f"remaining={remaining} evicted={evicted}"
                )

            after = target.stat()
            if after.st_ino != target_ino:
                raise AssertionError(
                    "stable inode changed after multiple-handle FORGET lifecycle: "
                    f"before={target_ino} after={after.st_ino}"
                )
            if target.read_bytes() != payload:
                raise AssertionError("post-FORGET multiple-handle payload mismatch")

            target.unlink()
            print(
                "OK forget-multiple-handles "
                f"ino={target_ino} "
                f"first_release_fh={first_release[0]} first_release_evicted=0 "
                f"second_release_fh={second_release[0]} second_release_evicted=0 "
                f"forget_nlookup={nlookup} forget_remaining=0 forget_evicted=1 "
                "stable_inode=1 relookup=1"
            )
        except Exception:
            launcher._dump_log()
            raise
        finally:
            if first is not None:
                first.close()
            if second is not None:
                second.close()
            launcher.stop()
            for key, value in previous.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value


if __name__ == "__main__":
    main()
