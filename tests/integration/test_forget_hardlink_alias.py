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
CONTROL_COUNT_ENV = "FOD_TEST_FORGET_INVALIDATE_COUNT"
WAIT_SECONDS = 10.0

FORGET_RE = re.compile(
    r"FOD forget profile: ino=(\d+) nlookup=(\d+) remaining=(\d+) evicted=(true|false)"
)


def read_log(log_file: Path) -> str:
    try:
        return log_file.read_text(encoding="utf-8")
    except FileNotFoundError:
        return ""


def wait_for_control(control_dir: Path, cycle: int, expected_name: str) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    done = control_dir / f"done.{cycle}"
    error = control_dir / "error"

    while time.monotonic() < deadline:
        if error.exists():
            raise AssertionError(error.read_text(encoding="utf-8").strip())
        if done.exists():
            value = done.read_text(encoding="utf-8").strip()
            if f"name={expected_name}" not in value:
                raise AssertionError(
                    f"invalidation hook used unexpected name cycle={cycle}: {value}"
                )
            return
        time.sleep(0.01)

    raise AssertionError(f"hardlink invalidation hook timed out cycle={cycle}")


def forget_matches(log_file: Path, target_ino: int) -> list[tuple[str, ...]]:
    return [
        match
        for match in FORGET_RE.findall(read_log(log_file))
        if int(match[0]) == target_ino
    ]


def wait_for_forget(
    log_file: Path,
    target_ino: int,
    previous_count: int,
) -> tuple[int, int, bool]:
    deadline = time.monotonic() + WAIT_SECONDS
    last = ""

    while time.monotonic() < deadline:
        last = read_log(log_file)
        matching = [
            match
            for match in FORGET_RE.findall(last)
            if int(match[0]) == target_ino
        ]
        if len(matching) > previous_count:
            _, nlookup, remaining, evicted = matching[previous_count]
            return int(nlookup), int(remaining), evicted == "true"
        time.sleep(0.01)

    raise AssertionError(
        "kernel did not emit expected hardlink FUSE FORGET\n" + last[-4000:]
    )


def trigger_invalidation(
    *,
    control_dir: Path,
    cycle: int,
    name: str,
) -> None:
    (control_dir / f"trigger.{cycle}").write_text(
        f"name={name}\n",
        encoding="utf-8",
    )
    wait_for_control(control_dir, cycle, name)


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    fuse_bin = os.environ.get("FOD_RUST_FUSE_BIN", "").strip()
    if not fuse_bin:
        raise AssertionError(
            "FOD_RUST_FUSE_BIN must point to fod-rust-fuse built with "
            "--features integration-test-hooks"
        )

    with tempfile.TemporaryDirectory(prefix="fod-forget-hardlink-alias-") as temp_dir:
        temp = Path(temp_dir)
        mountpoint = temp / "mount"
        control_dir = temp / "control"
        control_dir.mkdir()

        launcher = FODMount(str(root))
        launcher.init_schema()

        suffix = uuid.uuid4().hex
        source_name = f"forget-hardlink-source-{suffix}.bin"
        alias_name = f"forget-hardlink-alias-{suffix}.bin"
        payload = b"FOD hardlink lookup refs aggregate by inode\n"

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
        os.environ[CONTROL_NAME_ENV] = source_name
        os.environ[CONTROL_COUNT_ENV] = "2"
        os.environ["FOD_PROFILE_METADATA_CACHE"] = "1"
        os.environ["FOD_READDIR_REGISTER_PATHS"] = "0"
        os.environ["FOD_READDIR_BATCH_METADATA"] = "1"
        os.environ["FOD_LOG_LEVEL"] = "info"

        try:
            launcher.start(
                str(mountpoint),
                log_prefix="/tmp/fod-forget-hardlink-alias",
            )
            if launcher.config is None:
                raise AssertionError("mount config unavailable")

            source = mountpoint / source_name
            alias = mountpoint / alias_name

            source.write_bytes(payload)
            source_ino = source.stat().st_ino

            os.link(source, alias)
            alias_stat = alias.stat()
            alias_ino = alias_stat.st_ino

            if source_ino != alias_ino:
                raise AssertionError(
                    f"hardlink inode mismatch source={source_ino} alias={alias_ino}"
                )
            if alias_stat.st_nlink != 2:
                raise AssertionError(
                    f"hardlink nlink mismatch expected=2 got={alias_stat.st_nlink}"
                )
            if source.read_bytes() != payload or alias.read_bytes() != payload:
                raise AssertionError("hardlink payload mismatch before invalidation")

            before_first = len(forget_matches(launcher.config.log_file, source_ino))
            trigger_invalidation(
                control_dir=control_dir,
                cycle=1,
                name=source_name,
            )
            first_nlookup, first_remaining, first_evicted = wait_for_forget(
                launcher.config.log_file,
                source_ino,
                before_first,
            )

            if first_nlookup <= 0:
                raise AssertionError(
                    f"invalid first hardlink forget nlookup={first_nlookup}"
                )
            if first_remaining <= 0:
                raise AssertionError(
                    "first hardlink alias invalidation retired all lookup refs"
                )
            if first_evicted:
                raise AssertionError(
                    "first hardlink alias invalidation evicted shared inode early"
                )

            # Nie dotykamy zadnej nazwy pomiedzy invalidacjami, aby nie tworzyc
            # nowego lookup ref i zachowac deterministyczny bilans nlookup.
            before_second = len(forget_matches(launcher.config.log_file, source_ino))
            trigger_invalidation(
                control_dir=control_dir,
                cycle=2,
                name=alias_name,
            )
            second_nlookup, second_remaining, second_evicted = wait_for_forget(
                launcher.config.log_file,
                source_ino,
                before_second,
            )

            if second_nlookup <= 0:
                raise AssertionError(
                    f"invalid second hardlink forget nlookup={second_nlookup}"
                )
            if second_remaining != 0 or not second_evicted:
                raise AssertionError(
                    "final hardlink alias invalidation did not retire shared inode: "
                    f"remaining={second_remaining} evicted={second_evicted}"
                )

            source_after = source.stat()
            alias_after = alias.stat()
            if source_after.st_ino != source_ino or alias_after.st_ino != source_ino:
                raise AssertionError(
                    "stable inode changed after hardlink alias FORGET lifecycle: "
                    f"before={source_ino} source_after={source_after.st_ino} "
                    f"alias_after={alias_after.st_ino}"
                )
            if source_after.st_nlink != 2 or alias_after.st_nlink != 2:
                raise AssertionError(
                    "hardlink count changed across FORGET lifecycle: "
                    f"source_nlink={source_after.st_nlink} "
                    f"alias_nlink={alias_after.st_nlink}"
                )
            if source.read_bytes() != payload or alias.read_bytes() != payload:
                raise AssertionError("hardlink payload mismatch after relookup")

            alias.unlink()
            source.unlink()

            print(
                "OK forget-hardlink-alias "
                f"ino={source_ino} "
                f"first_nlookup={first_nlookup} first_remaining={first_remaining} "
                "first_evicted=0 "
                f"second_nlookup={second_nlookup} second_remaining=0 second_evicted=1 "
                "stable_inode=1 nlink=2 payload=1 relookup=1"
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
