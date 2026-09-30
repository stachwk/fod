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

    raise AssertionError("forget active-handle invalidation hook did not complete")


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
        "kernel did not emit FUSE FORGET for active handle\n" + last[-4000:]
    )


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
        "FUSE release did not emit active-handle cache retirement profile\n"
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

    with tempfile.TemporaryDirectory(prefix="fod-forget-active-handle-") as temp_dir:
        temp = Path(temp_dir)
        mountpoint = temp / "mount"
        control_dir = temp / "control"
        control_dir.mkdir()

        launcher = FODMount(str(root))
        launcher.init_schema()

        name = f"forget-active-handle-{uuid.uuid4().hex}.bin"
        payload = b"FOD active handle survives forget invalidation\n"

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

        handle = None
        try:
            launcher.start(
                str(mountpoint),
                log_prefix="/tmp/fod-forget-active-handle",
            )
            if launcher.config is None:
                raise AssertionError("mount config unavailable")

            target = mountpoint / name
            target.write_bytes(payload)
            target_ino = target.stat().st_ino

            handle = target.open("rb")
            if handle.read() != payload:
                raise AssertionError("pre-invalidation active handle payload mismatch")
            handle.seek(0)

            (control_dir / "trigger").write_text("invalidate\n", encoding="utf-8")
            wait_for_control(control_dir)

            nlookup, remaining, evicted = wait_for_forget(
                launcher.config.log_file,
                target_ino,
            )
            if nlookup <= 0:
                raise AssertionError(
                    f"invalid active-handle forget nlookup={nlookup} ino={target_ino}"
                )
            if remaining != 0:
                raise AssertionError(
                    f"active-handle forget retained lookup refs remaining={remaining}"
                )
            if evicted:
                raise AssertionError(
                    "active handle did not protect inode cache from FORGET eviction"
                )

            if handle.read() != payload:
                raise AssertionError("open handle stopped reading after FORGET")
            handle.seek(0)

            releases_before_close = len(
                release_matches(launcher.config.log_file, target_ino)
            )
            handle.close()
            handle = None

            (
                released_fh,
                release_evicted,
                lookup_remaining,
                inode_cached,
                path_cached,
            ) = wait_for_release(
                launcher.config.log_file,
                target_ino,
                releases_before_close,
            )

            if not release_evicted:
                raise AssertionError(
                    f"last active handle release did not evict inode fh={released_fh}"
                )
            if lookup_remaining != 0:
                raise AssertionError(
                    "lookup refs remained after last active handle release: "
                    f"{lookup_remaining}"
                )
            if inode_cached or path_cached:
                raise AssertionError(
                    "inode/path cache remained after last active handle release: "
                    f"inode_cached={inode_cached} path_cached={path_cached}"
                )

            after = target.stat()
            if after.st_ino != target_ino:
                raise AssertionError(
                    "stable inode changed after active-handle release/relookup: "
                    f"before={target_ino} after={after.st_ino}"
                )
            if target.read_bytes() != payload:
                raise AssertionError("post-release relookup payload mismatch")

            target.unlink()
            print(
                "OK forget-active-handle "
                f"ino={target_ino} forget_nlookup={nlookup} "
                "forget_remaining=0 forget_evicted=0 "
                f"release_fh={released_fh} release_evicted=1 "
                "lookup_remaining=0 inode_cached=0 path_cached=0 "
                "stable_inode=1 relookup=1"
            )
        except Exception:
            launcher._dump_log()
            raise
        finally:
            if handle is not None:
                handle.close()
            launcher.stop()
            for key, value in previous.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value


if __name__ == "__main__":
    main()
