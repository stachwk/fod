#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import tempfile
import time
import uuid
from pathlib import Path

from fod_mount import FODMount

FAIL_FAST_SECONDS = 1.0
WAIT_SECONDS = 10.0


def wait_for_absent(path: Path) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    while time.monotonic() < deadline:
        if not os.path.lexists(path):
            return
        time.sleep(0.05)
    raise AssertionError(f"{path}: path still exists after timeout")


def wait_for_symlink(path: Path, expected_target: str) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    observed: str | None = None
    while time.monotonic() < deadline:
        if os.path.islink(path):
            observed = os.readlink(path)
            if observed == expected_target:
                return
        time.sleep(0.05)
    raise AssertionError(
        f"{path}: expected symlink target {expected_target!r}, observed={observed!r}"
    )


def wait_for_bytes(path: Path, expected: bytes) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    observed: bytes | None = None
    while time.monotonic() < deadline:
        try:
            observed = path.read_bytes()
        except FileNotFoundError:
            observed = None
        if observed == expected:
            return
        time.sleep(0.05)
    raise AssertionError(
        f"{path}: expected payload {expected!r}, observed={observed!r}"
    )


def expect_replace_ebusy(source: Path, destination: Path, label: str) -> float:
    started = time.monotonic()
    observed_errno = None
    try:
        os.replace(source, destination)
    except OSError as exc:
        observed_errno = exc.errno
    elapsed = time.monotonic() - started

    if observed_errno != errno.EBUSY:
        raise AssertionError(
            f"{label}: errno={observed_errno}, expected EBUSY={errno.EBUSY}"
        )
    if elapsed >= FAIL_FAST_SECONDS:
        raise AssertionError(
            f"{label}: did not fail fast; elapsed={elapsed:.6f}s"
        )
    return elapsed


def run_cases(mount_a: Path, mount_b: Path) -> None:
    suffix = uuid.uuid4().hex
    payload_name = f"rename-symlink-payload-{suffix}.bin"
    payload_a = mount_a / payload_name
    payload_b = mount_b / payload_name
    payload = b"symlink-payload"
    payload_a.write_bytes(payload)
    wait_for_bytes(payload_b, payload)

    source_name = f"rename-symlink-source-{suffix}"
    moved_name = f"rename-symlink-moved-{suffix}"
    source_a = mount_a / source_name
    source_b = mount_b / source_name
    moved_a = mount_a / moved_name
    moved_b = mount_b / moved_name

    os.symlink(payload_name, source_a)
    wait_for_symlink(source_b, payload_name)
    os.replace(source_b, moved_b)
    wait_for_absent(source_a)
    wait_for_absent(source_b)
    wait_for_symlink(moved_a, payload_name)
    wait_for_symlink(moved_b, payload_name)

    link_target_name = f"rename-symlink-target-{suffix}"
    replacement_name = f"rename-symlink-file-{suffix}.bin"
    link_target_a = mount_a / link_target_name
    link_target_b = mount_b / link_target_name
    replacement_b = mount_b / replacement_name
    replacement = b"file-replaces-symlink"

    os.symlink(payload_name, link_target_a)
    wait_for_symlink(link_target_b, payload_name)
    replacement_b.write_bytes(replacement)
    os.replace(replacement_b, link_target_b)
    wait_for_bytes(link_target_a, replacement)
    wait_for_bytes(link_target_b, replacement)
    if os.path.islink(link_target_a) or os.path.islink(link_target_b):
        raise AssertionError("file-over-symlink left a symlink at destination")
    wait_for_bytes(payload_a, payload)

    file_target_name = f"rename-symlink-file-target-{suffix}.bin"
    guarded_source_name = f"rename-symlink-guarded-source-{suffix}"
    file_target_a = mount_a / file_target_name
    file_target_b = mount_b / file_target_name
    guarded_source_a = mount_a / guarded_source_name
    guarded_source_b = mount_b / guarded_source_name
    guarded_link_target = "guarded-symlink-target"

    file_target_a.write_bytes(payload)
    wait_for_bytes(file_target_b, payload)
    os.symlink(guarded_link_target, guarded_source_a)
    wait_for_symlink(guarded_source_b, guarded_link_target)

    fd = os.open(file_target_a, os.O_WRONLY)
    try:
        elapsed = expect_replace_ebusy(
            guarded_source_b,
            file_target_b,
            "symlink source replacing active file target",
        )
        wait_for_bytes(file_target_a, payload)
        wait_for_bytes(file_target_b, payload)
        wait_for_symlink(guarded_source_a, guarded_link_target)
        wait_for_symlink(guarded_source_b, guarded_link_target)
        print(
            "OK rename-symlink-target-writer "
            f"errno={errno.EBUSY} elapsed_ms={elapsed * 1000.0:.3f} "
            "target_unchanged=1 source_preserved=1"
        )
    finally:
        os.close(fd)

    os.replace(guarded_source_b, file_target_b)
    wait_for_absent(guarded_source_a)
    wait_for_absent(guarded_source_b)
    wait_for_symlink(file_target_a, guarded_link_target)
    wait_for_symlink(file_target_b, guarded_link_target)

    second_source_a = mount_a / f"rename-symlink-source-two-{suffix}"
    second_source_b = mount_b / second_source_a.name
    second_target_a = mount_a / f"rename-symlink-target-two-{suffix}"
    second_target_b = mount_b / second_target_a.name

    os.symlink("source-two-target", second_source_a)
    os.symlink("target-two-old", second_target_a)
    wait_for_symlink(second_source_b, "source-two-target")
    wait_for_symlink(second_target_b, "target-two-old")
    os.replace(second_source_b, second_target_b)
    wait_for_absent(second_source_a)
    wait_for_absent(second_source_b)
    wait_for_symlink(second_target_a, "source-two-target")
    wait_for_symlink(second_target_b, "source-two-target")

    print(
        "OK rename-symlink-namespace "
        "source_move=1 file_over_symlink=1 "
        "symlink_over_file=1 symlink_over_symlink=1"
    )


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    launcher_a = FODMount(str(root))
    launcher_b = FODMount(str(root))
    launcher_a.init_schema()

    with tempfile.TemporaryDirectory(prefix="fod-rename-symlink-") as temp_dir:
        temp = Path(temp_dir)
        mount_a = temp / "mount-a"
        mount_b = temp / "mount-b"

        try:
            launcher_a.start(str(mount_a), log_prefix="/tmp/fod-rename-symlink-a")
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-rename-symlink-b")
            run_cases(mount_a, mount_b)
            print("OK rename-symlink-transactional protected_cases=4")
        except BaseException:
            print("\n=== MOUNT A LOG ===")
            launcher_a._dump_log()
            print("\n=== MOUNT B LOG ===")
            launcher_b._dump_log()
            raise
        finally:
            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
