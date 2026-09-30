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


# Test korzysta z tego samego hooka co pojedyncza invalidacja, ale prosi
# runtime o wykonanie wielu cykli w ramach jednego montowania.
CONTROL_DIR_ENV = "FOD_TEST_FORGET_INVALIDATE_DIR"
CONTROL_NAME_ENV = "FOD_TEST_FORGET_INVALIDATE_NAME"
CONTROL_COUNT_ENV = "FOD_TEST_FORGET_INVALIDATE_COUNT"

# Dziesiec cykli jest celowo male: to gate funkcjonalny C1, a nie jeszcze soak.
CYCLES = 10
WAIT_SECONDS = 10.0

# Profil FORGET jest zrodlem prawdy dla licznika lookup oraz decyzji eviction.
FORGET_RE = re.compile(
    r"FOD forget profile: ino=(\d+) nlookup=(\d+) remaining=(\d+) evicted=(true|false)"
)


def wait_for_control(control_dir: Path, cycle: int) -> None:
    """Czeka na zakonczenie konkretnego cyklu invalidacji."""
    deadline = time.monotonic() + WAIT_SECONDS
    done = control_dir / f"done.{cycle}"
    error = control_dir / "error"

    while time.monotonic() < deadline:
        if error.exists():
            raise AssertionError(error.read_text(encoding="utf-8").strip())
        if done.exists():
            return
        time.sleep(0.01)

    raise AssertionError(f"forget invalidation hook did not complete cycle={cycle}")


def wait_for_forget(
    log_file: Path,
    target_ino: int,
    expected_count: int,
) -> tuple[int, int, int, bool]:
    """Czeka az log zawiera oczekiwana liczbe FORGET dla badanego inode."""
    deadline = time.monotonic() + WAIT_SECONDS
    last = ""

    while time.monotonic() < deadline:
        try:
            last = log_file.read_text(encoding="utf-8")
        except FileNotFoundError:
            last = ""

        # Filtrujemy po inode, aby obce zdarzenia FORGET nie zaliczyly cyklu.
        matching = [
            match
            for match in FORGET_RE.findall(last)
            if int(match[0]) == target_ino
        ]
        if len(matching) >= expected_count:
            ino, nlookup, remaining, evicted = matching[expected_count - 1]
            return (
                int(ino),
                int(nlookup),
                int(remaining),
                evicted == "true",
            )
        time.sleep(0.01)

    raise AssertionError(
        "kernel did not emit expected repeated FUSE FORGET "
        f"ino={target_ino} expected_count={expected_count}\n"
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

    with tempfile.TemporaryDirectory(prefix="fod-forget-repeated-") as temp_dir:
        temp = Path(temp_dir)
        mountpoint = temp / "mount"
        control_dir = temp / "control"
        control_dir.mkdir()

        launcher = FODMount(str(root))
        launcher.init_schema()

        name = f"forget-repeated-{uuid.uuid4().hex}.bin"
        payload = b"FOD repeated deterministic forget invalidation\n"

        # Przywracamy srodowisko po tescie, aby nie zmieniac kolejnych gate'ow.
        previous = {
            CONTROL_DIR_ENV: os.environ.get(CONTROL_DIR_ENV),
            CONTROL_NAME_ENV: os.environ.get(CONTROL_NAME_ENV),
            CONTROL_COUNT_ENV: os.environ.get(CONTROL_COUNT_ENV),
            "FOD_PROFILE_METADATA_CACHE": os.environ.get("FOD_PROFILE_METADATA_CACHE"),
            "FOD_READDIR_REGISTER_PATHS": os.environ.get("FOD_READDIR_REGISTER_PATHS"),
            "FOD_READDIR_BATCH_METADATA": os.environ.get("FOD_READDIR_BATCH_METADATA"),
            "FOD_LOG_LEVEL": os.environ.get("FOD_LOG_LEVEL"),
        }

        os.environ[CONTROL_DIR_ENV] = str(control_dir)
        os.environ[CONTROL_NAME_ENV] = name
        os.environ[CONTROL_COUNT_ENV] = str(CYCLES)
        os.environ["FOD_PROFILE_METADATA_CACHE"] = "1"
        os.environ["FOD_READDIR_REGISTER_PATHS"] = "0"
        os.environ["FOD_READDIR_BATCH_METADATA"] = "1"
        os.environ["FOD_LOG_LEVEL"] = "info"

        try:
            launcher.start(
                str(mountpoint),
                log_prefix="/tmp/fod-forget-repeated-invalidation",
            )

            target = mountpoint / name
            target.write_bytes(payload)
            initial = target.stat()
            target_ino = initial.st_ino

            if target.read_bytes() != payload:
                raise AssertionError("pre-invalidation payload mismatch")

            # Kazdy cykl wymusza invalidacje dentry, oczekuje FORGET, a potem
            # robi relookup. Stable inode musi pozostac identyczny.
            for cycle in range(1, CYCLES + 1):
                (control_dir / f"trigger.{cycle}").write_text(
                    "invalidate\n",
                    encoding="utf-8",
                )
                wait_for_control(control_dir, cycle)

                if launcher.config is None:
                    raise AssertionError("mount config unavailable")

                forget_ino, nlookup, remaining, evicted = wait_for_forget(
                    launcher.config.log_file,
                    target_ino,
                    cycle,
                )

                if forget_ino != target_ino:
                    raise AssertionError(
                        f"forget inode mismatch expected={target_ino} got={forget_ino}"
                    )
                if nlookup <= 0:
                    raise AssertionError(
                        f"invalid forget nlookup={nlookup} ino={forget_ino} cycle={cycle}"
                    )
                if remaining != 0 or not evicted:
                    raise AssertionError(
                        "forget did not retire cached inode path: "
                        f"cycle={cycle} ino={forget_ino} "
                        f"remaining={remaining} evicted={evicted}"
                    )

                after = target.stat()
                if after.st_ino != target_ino:
                    raise AssertionError(
                        "stable inode changed across repeated forget/relookup: "
                        f"cycle={cycle} before={target_ino} after={after.st_ino}"
                    )
                if target.read_bytes() != payload:
                    raise AssertionError(
                        f"post-invalidation payload mismatch cycle={cycle}"
                    )

            target.unlink()
            print(
                "OK forget-repeated-invalidation "
                f"cycles={CYCLES} ino={target_ino} "
                "remaining=0 evicted=1 stable_inode=1 relookup=1"
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
