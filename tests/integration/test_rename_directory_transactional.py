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

BARRIER_ENV = "FOD_TEST_RENAME_BEFORE_OWNERSHIP_BARRIER_DIR"
WAIT_SECONDS = 10.0


def wait_for_absent(path: Path) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    while time.monotonic() < deadline:
        if not os.path.lexists(path):
            return
        time.sleep(0.05)
    raise AssertionError(f"{path}: path still exists after timeout")


def wait_for_dir(path: Path) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    while time.monotonic() < deadline:
        if path.is_dir():
            return
        time.sleep(0.05)
    raise AssertionError(f"{path}: directory did not appear")


def wait_for_bytes(path: Path, expected: bytes) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    observed: bytes | None = None
    while time.monotonic() < deadline:
        try:
            observed = path.read_bytes()
        except (FileNotFoundError, IsADirectoryError, NotADirectoryError):
            observed = None
        if observed == expected:
            return
        time.sleep(0.05)
    raise AssertionError(
        f"{path}: expected payload {expected!r}, observed={observed!r}"
    )


def expect_replace_errno(
    source: Path,
    destination: Path,
    expected_errno: int,
    label: str,
) -> None:
    observed_errno = None
    try:
        os.replace(source, destination)
    except OSError as exc:
        observed_errno = exc.errno

    if observed_errno != expected_errno:
        raise AssertionError(
            f"{label}: errno={observed_errno}, expected={expected_errno}"
        )


def directory_move_and_replace(mount_a: Path, mount_b: Path) -> None:
    suffix = uuid.uuid4().hex

    source = mount_a / f"rename-dir-source-{suffix}"
    source_other = mount_b / source.name
    destination = mount_b / f"rename-dir-destination-{suffix}"
    destination_other = mount_a / destination.name

    source.mkdir()
    (source / "payload.bin").write_bytes(b"directory-source-payload")
    wait_for_dir(source_other)
    wait_for_bytes(source_other / "payload.bin", b"directory-source-payload")

    os.replace(source_other, destination)
    wait_for_absent(source)
    wait_for_absent(source_other)
    wait_for_dir(destination)
    wait_for_dir(destination_other)
    wait_for_bytes(destination / "payload.bin", b"directory-source-payload")
    wait_for_bytes(destination_other / "payload.bin", b"directory-source-payload")

    replacement_source = mount_a / f"rename-dir-replacement-{suffix}"
    replacement_source_other = mount_b / replacement_source.name
    empty_target = mount_b / f"rename-dir-empty-target-{suffix}"
    empty_target_other = mount_a / empty_target.name

    replacement_source.mkdir()
    (replacement_source / "replacement.bin").write_bytes(b"replacement-directory")
    empty_target.mkdir()
    wait_for_dir(replacement_source_other)
    wait_for_dir(empty_target_other)

    os.replace(replacement_source_other, empty_target)
    wait_for_absent(replacement_source)
    wait_for_absent(replacement_source_other)
    wait_for_dir(empty_target)
    wait_for_dir(empty_target_other)
    wait_for_bytes(empty_target / "replacement.bin", b"replacement-directory")
    wait_for_bytes(empty_target_other / "replacement.bin", b"replacement-directory")

    print(
        "OK rename-directory-move-replace "
        "move_to_absent=1 replace_empty_target=1 payload_preserved=1"
    )


def nonempty_target_is_preserved(mount_a: Path, mount_b: Path) -> None:
    suffix = uuid.uuid4().hex
    source = mount_a / f"rename-dir-nonempty-source-{suffix}"
    source_other = mount_b / source.name
    target = mount_b / f"rename-dir-nonempty-target-{suffix}"
    target_other = mount_a / target.name

    source.mkdir()
    (source / "source.bin").write_bytes(b"source")
    target.mkdir()
    (target / "target.bin").write_bytes(b"target")
    wait_for_dir(source_other)
    wait_for_dir(target_other)

    expect_replace_errno(source_other, target, errno.ENOTEMPTY, "nonempty directory target")

    wait_for_dir(source)
    wait_for_dir(source_other)
    wait_for_dir(target)
    wait_for_dir(target_other)
    wait_for_bytes(source / "source.bin", b"source")
    wait_for_bytes(target / "target.bin", b"target")

    print(
        "OK rename-directory-nonempty-target "
        f"errno={errno.ENOTEMPTY} source_preserved=1 target_preserved=1"
    )


def type_mismatch_errors(mount_a: Path, mount_b: Path) -> None:
    suffix = uuid.uuid4().hex

    source_dir = mount_a / f"rename-dir-over-file-source-{suffix}"
    source_dir_other = mount_b / source_dir.name
    file_target = mount_b / f"rename-dir-over-file-target-{suffix}.bin"
    file_target_other = mount_a / file_target.name

    source_dir.mkdir()
    file_target.write_bytes(b"file-target")
    wait_for_dir(source_dir_other)
    wait_for_bytes(file_target_other, b"file-target")

    expect_replace_errno(source_dir_other, file_target, errno.ENOTDIR, "directory over file")
    wait_for_dir(source_dir)
    wait_for_dir(source_dir_other)
    wait_for_bytes(file_target, b"file-target")
    wait_for_bytes(file_target_other, b"file-target")

    file_source = mount_a / f"rename-file-over-dir-source-{suffix}.bin"
    file_source_other = mount_b / file_source.name
    target_dir = mount_b / f"rename-file-over-dir-target-{suffix}"
    target_dir_other = mount_a / target_dir.name

    file_source.write_bytes(b"file-source")
    target_dir.mkdir()
    wait_for_bytes(file_source_other, b"file-source")
    wait_for_dir(target_dir_other)

    expect_replace_errno(file_source_other, target_dir, errno.EISDIR, "file over directory")
    wait_for_bytes(file_source, b"file-source")
    wait_for_bytes(file_source_other, b"file-source")
    wait_for_dir(target_dir)
    wait_for_dir(target_dir_other)

    print(
        "OK rename-directory-type-errors "
        f"dir_over_file_errno={errno.ENOTDIR} file_over_dir_errno={errno.EISDIR}"
    )


def descendant_is_rejected(mount_a: Path, mount_b: Path) -> None:
    suffix = uuid.uuid4().hex
    root = mount_a / f"rename-dir-cycle-{suffix}"
    root_other = mount_b / root.name
    child = root / "child"
    child_other = root_other / "child"

    child.mkdir(parents=True)
    wait_for_dir(child_other)

    observed_errno = None
    try:
        os.replace(root_other, child_other / "inner")
    except OSError as exc:
        observed_errno = exc.errno

    if observed_errno not in (errno.EINVAL, errno.EBUSY):
        raise AssertionError(
            f"directory descendant rename errno={observed_errno}, expected EINVAL/EBUSY"
        )

    wait_for_dir(root)
    wait_for_dir(root_other)
    wait_for_dir(child)
    wait_for_dir(child_other)

    print(
        "OK rename-directory-descendant "
        f"errno={observed_errno} source_preserved=1"
    )


def rollback_after_target_removal(
    mount_a: Path,
    mount_b: Path,
    barrier_dir: Path,
) -> None:
    suffix = uuid.uuid4().hex
    source = mount_a / f"rename-dir-fault-source-{suffix}"
    source_other = mount_b / source.name
    target = mount_b / f"rename-dir-fault-target-{suffix}"
    target_other = mount_a / target.name

    source.mkdir()
    (source / "payload.bin").write_bytes(b"rollback-source")
    target.mkdir()
    wait_for_dir(source_other)
    wait_for_dir(target_other)
    wait_for_bytes(source_other / "payload.bin", b"rollback-source")

    fault_path = barrier_dir / "fail_after_target_removal"
    fault_path.write_text(target.name, encoding="utf-8")
    try:
        expect_replace_errno(source_other, target, errno.EIO, "directory rollback injection")

        wait_for_dir(source)
        wait_for_dir(source_other)
        wait_for_dir(target)
        wait_for_dir(target_other)
        wait_for_bytes(source / "payload.bin", b"rollback-source")
        wait_for_bytes(source_other / "payload.bin", b"rollback-source")

        print(
            "OK rename-directory-fault-rollback "
            f"errno={errno.EIO} source_restored=1 target_restored=1"
        )
    finally:
        fault_path.unlink(missing_ok=True)

    os.replace(source_other, target)
    wait_for_absent(source)
    wait_for_absent(source_other)
    wait_for_dir(target)
    wait_for_dir(target_other)
    wait_for_bytes(target / "payload.bin", b"rollback-source")


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    launcher_a = FODMount(str(root))
    launcher_b = FODMount(str(root))
    launcher_a.init_schema()

    with tempfile.TemporaryDirectory(prefix="fod-rename-dir-") as temp_dir:
        temp = Path(temp_dir)
        mount_a = temp / "mount-a"
        mount_b = temp / "mount-b"
        barrier_dir = temp / "rename-barrier"
        barrier_dir.mkdir()

        previous_barrier = os.environ.get(BARRIER_ENV)
        os.environ[BARRIER_ENV] = str(barrier_dir)

        try:
            launcher_a.start(str(mount_a), log_prefix="/tmp/fod-rename-dir-a")
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-rename-dir-b")

            directory_move_and_replace(mount_a, mount_b)
            nonempty_target_is_preserved(mount_a, mount_b)
            type_mismatch_errors(mount_a, mount_b)
            descendant_is_rejected(mount_a, mount_b)
            rollback_after_target_removal(mount_a, mount_b, barrier_dir)

            print("OK rename-directory-transactional protected_cases=5")
        except BaseException:
            print("\n=== MOUNT A LOG ===")
            launcher_a._dump_log()
            print("\n=== MOUNT B LOG ===")
            launcher_b._dump_log()
            raise
        finally:
            launcher_b.stop()
            launcher_a.stop()
            if previous_barrier is None:
                os.environ.pop(BARRIER_ENV, None)
            else:
                os.environ[BARRIER_ENV] = previous_barrier


if __name__ == "__main__":
    main()
