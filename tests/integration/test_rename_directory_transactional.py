#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import stat
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


def wait_for_symlink(path: Path, expected_target: str) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    observed: str | None = None
    while time.monotonic() < deadline:
        try:
            observed = os.readlink(path)
        except OSError:
            observed = None
        if observed == expected_target:
            return
        time.sleep(0.05)
    raise AssertionError(
        f"{path}: expected symlink target {expected_target!r}, observed={observed!r}"
    )


def wait_for_fifo(path: Path) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    while time.monotonic() < deadline:
        try:
            if stat.S_ISFIFO(path.lstat().st_mode):
                return
        except OSError:
            pass
        time.sleep(0.05)
    raise AssertionError(f"{path}: expected FIFO")


def wait_for_regular_file(path: Path) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    while time.monotonic() < deadline:
        try:
            if stat.S_ISREG(path.lstat().st_mode):
                return
        except OSError:
            pass
        time.sleep(0.05)
    raise AssertionError(f"{path}: expected regular file")


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


def cross_directory_move_updates_parent_link_counts(
    mount_a: Path, mount_b: Path
) -> None:
    suffix = uuid.uuid4().hex
    source_parent = mount_a / f"rename-nlink-source-{suffix}"
    source_parent_other = mount_b / source_parent.name
    destination_parent = mount_a / f"rename-nlink-destination-{suffix}"
    destination_parent_other = mount_b / destination_parent.name
    child = source_parent / "child"
    child_other = source_parent_other / "child"

    source_parent.mkdir()
    destination_parent.mkdir()
    child.mkdir()

    for parent in (source_parent, source_parent_other):
        if parent.stat().st_nlink != 3:
            raise AssertionError(f"{parent}: expected initial st_nlink=3")
    for parent in (destination_parent, destination_parent_other):
        if parent.stat().st_nlink != 2:
            raise AssertionError(f"{parent}: expected initial st_nlink=2")

    os.replace(child_other, destination_parent_other / "child")

    for parent in (source_parent, source_parent_other):
        observed = parent.stat().st_nlink
        if observed != 2:
            raise AssertionError(f"{parent}: st_nlink={observed}, expected 2 after move")
    for parent in (destination_parent, destination_parent_other):
        observed = parent.stat().st_nlink
        if observed != 3:
            raise AssertionError(f"{parent}: st_nlink={observed}, expected 3 after move")

    replacement_source = source_parent / "child-replacement"
    replacement_source_other = source_parent_other / "child-replacement"
    replacement_target = destination_parent / "target-child"
    replacement_target_other = destination_parent_other / "target-child"
    replacement_source.mkdir()
    replacement_target.mkdir()

    for parent in (source_parent, source_parent_other):
        if parent.stat().st_nlink != 3:
            raise AssertionError(f"{parent}: expected replacement source st_nlink=3")
    for parent in (destination_parent, destination_parent_other):
        if parent.stat().st_nlink != 4:
            raise AssertionError(f"{parent}: expected replacement target st_nlink=4")

    os.replace(replacement_source_other, replacement_target_other)

    for parent in (source_parent, source_parent_other):
        observed = parent.stat().st_nlink
        if observed != 2:
            raise AssertionError(
                f"{parent}: st_nlink={observed}, expected 2 after directory replacement"
            )
    for parent in (destination_parent, destination_parent_other):
        observed = parent.stat().st_nlink
        if observed != 4:
            raise AssertionError(
                f"{parent}: st_nlink={observed}, expected 4 after directory replacement"
            )

    print(
        "OK rename-cross-directory-parent-link-counts "
        "move=source:3->2,destination:2->3 "
        "replace=source:3->2,destination:4->4 observed_on_both_mounts=1"
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


def missing_source_preserves_destination(mount_a: Path, mount_b: Path) -> None:
    suffix = uuid.uuid4().hex
    missing_source = mount_a / f"rename-missing-source-{suffix}"
    destination = mount_a / f"rename-missing-destination-{suffix}.bin"
    destination_other = mount_b / destination.name
    payload = b"destination survives missing-source rename"

    destination.write_bytes(payload)
    wait_for_bytes(destination_other, payload)
    expect_replace_errno(missing_source, destination, errno.ENOENT, "missing source")

    if os.path.lexists(missing_source):
        raise AssertionError("missing rename source unexpectedly appeared")
    if destination.read_bytes() != payload:
        raise AssertionError("failed rename changed destination")
    if destination_other.read_bytes() != payload:
        raise AssertionError("failed rename changed destination on other mount")

    print("OK rename-missing-source errno=ENOENT destination_preserved=1")


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


def file_and_symlink_type_pairings(mount_a: Path, mount_b: Path) -> None:
    suffix = uuid.uuid4().hex

    file_source = mount_a / f"rename-file-symlink-source-{suffix}"
    file_source_other = mount_b / file_source.name
    file_target = mount_b / f"rename-file-symlink-target-{suffix}"
    file_target_other = mount_a / file_target.name
    preserved_target = mount_b / f"rename-file-symlink-data-{suffix}"
    file_source.write_bytes(b"replacement-file")
    preserved_target.write_bytes(b"symlink-target-stays")
    file_target.symlink_to(preserved_target.name)
    wait_for_bytes(file_source_other, b"replacement-file")
    wait_for_bytes(file_target, b"symlink-target-stays")
    os.replace(file_source_other, file_target)
    wait_for_absent(file_source)
    wait_for_bytes(file_target, b"replacement-file")
    wait_for_bytes(preserved_target, b"symlink-target-stays")
    wait_for_absent(file_source_other)
    wait_for_bytes(file_target_other, b"replacement-file")
    if file_target.is_symlink():
        raise AssertionError("file-over-symlink rename did not replace the link")
    if file_target_other.is_symlink():
        raise AssertionError("mount A still sees the replaced symlink")

    symlink_data = mount_a / f"rename-symlink-data-{suffix}"
    symlink_data.write_bytes(b"symlink-source-target")
    symlink_source = mount_a / f"rename-symlink-source-{suffix}"
    symlink_source_other = mount_b / symlink_source.name
    regular_target = mount_b / f"rename-symlink-regular-target-{suffix}"
    regular_target_other = mount_a / regular_target.name
    symlink_source.symlink_to(symlink_data.name)
    regular_target.write_bytes(b"regular-target-before")
    wait_for_symlink(symlink_source_other, symlink_data.name)
    wait_for_bytes(regular_target, b"regular-target-before")
    os.replace(symlink_source_other, regular_target)
    wait_for_absent(symlink_source)
    wait_for_symlink(regular_target, symlink_data.name)
    wait_for_bytes(symlink_data, b"symlink-source-target")
    wait_for_absent(symlink_source_other)
    wait_for_symlink(regular_target_other, symlink_data.name)

    first_link_data = mount_a / f"rename-link-data-a-{suffix}"
    second_link_data = mount_b / f"rename-link-data-b-{suffix}"
    first_link_data.write_bytes(b"link-a")
    second_link_data.write_bytes(b"link-b")
    link_source = mount_a / f"rename-symlink-over-symlink-source-{suffix}"
    link_source_other = mount_b / link_source.name
    link_target = mount_b / f"rename-symlink-over-symlink-target-{suffix}"
    link_target_other = mount_a / link_target.name
    link_source.symlink_to(first_link_data.name)
    link_target.symlink_to(second_link_data.name)
    wait_for_symlink(link_source_other, first_link_data.name)
    wait_for_symlink(link_target, second_link_data.name)
    os.replace(link_source_other, link_target)
    wait_for_absent(link_source)
    wait_for_symlink(link_target, first_link_data.name)
    wait_for_bytes(first_link_data, b"link-a")
    wait_for_bytes(second_link_data, b"link-b")
    wait_for_absent(link_source_other)
    wait_for_symlink(link_target_other, first_link_data.name)

    source_link_to_dir = mount_a / f"rename-link-to-dir-source-{suffix}"
    source_link_to_dir_other = mount_b / source_link_to_dir.name
    source_link_data_dir = mount_a / f"rename-link-to-dir-data-{suffix}"
    empty_dir_target = mount_b / f"rename-link-to-dir-target-{suffix}"
    empty_dir_target_other = mount_a / empty_dir_target.name
    source_link_data_dir.mkdir()
    source_link_to_dir.symlink_to(source_link_data_dir.name, target_is_directory=True)
    empty_dir_target.mkdir()
    wait_for_symlink(source_link_to_dir_other, source_link_data_dir.name)
    wait_for_dir(empty_dir_target)
    expect_replace_errno(
        source_link_to_dir_other,
        empty_dir_target,
        errno.EISDIR,
        "symlink-to-directory over directory",
    )
    wait_for_symlink(source_link_to_dir, source_link_data_dir.name)
    wait_for_dir(empty_dir_target)
    wait_for_symlink(source_link_to_dir_other, source_link_data_dir.name)
    wait_for_dir(empty_dir_target_other)
    if list(empty_dir_target.iterdir()):
        raise AssertionError("failed symlink-to-directory rename changed target")

    source_dir = mount_a / f"rename-dir-over-symlink-source-{suffix}"
    source_dir_other = mount_b / source_dir.name
    target_link = mount_b / f"rename-dir-over-symlink-target-{suffix}"
    target_link_other = mount_a / target_link.name
    target_data_dir = mount_b / f"rename-dir-over-symlink-data-{suffix}"
    source_dir.mkdir()
    target_data_dir.mkdir()
    target_link.symlink_to(target_data_dir.name, target_is_directory=True)
    wait_for_dir(source_dir_other)
    wait_for_symlink(target_link, target_data_dir.name)
    expect_replace_errno(
        source_dir_other,
        target_link,
        errno.ENOTDIR,
        "directory over symlink",
    )
    wait_for_dir(source_dir)
    wait_for_symlink(target_link, target_data_dir.name)
    wait_for_dir(target_data_dir)
    wait_for_dir(source_dir_other)
    wait_for_symlink(target_link_other, target_data_dir.name)

    print(
        "OK rename-file-symlink-pairings file_over_symlink=success "
        "symlink_over_file=success symlink_over_symlink=success "
        f"symlink_dir_over_dir={errno.EISDIR} dir_over_symlink={errno.ENOTDIR}"
    )


def fifo_file_type_pairings(mount_a: Path, mount_b: Path) -> None:
    suffix = uuid.uuid4().hex

    fifo_source = mount_a / f"rename-fifo-file-source-{suffix}"
    fifo_source_other = mount_b / fifo_source.name
    regular_target = mount_b / f"rename-fifo-file-target-{suffix}"
    regular_target_other = mount_a / regular_target.name
    os.mkfifo(fifo_source, 0o600)
    regular_target.write_bytes(b"replaced-by-fifo")
    wait_for_fifo(fifo_source_other)
    wait_for_bytes(regular_target, b"replaced-by-fifo")
    os.replace(fifo_source_other, regular_target)
    wait_for_absent(fifo_source)
    wait_for_absent(fifo_source_other)
    wait_for_fifo(regular_target)
    wait_for_fifo(regular_target_other)

    file_source = mount_a / f"rename-file-fifo-source-{suffix}"
    file_source_other = mount_b / file_source.name
    fifo_target = mount_b / f"rename-file-fifo-target-{suffix}"
    fifo_target_other = mount_a / fifo_target.name
    file_source.write_bytes(b"replaced-by-file")
    os.mkfifo(fifo_target, 0o600)
    wait_for_bytes(file_source_other, b"replaced-by-file")
    wait_for_fifo(fifo_target_other)
    os.replace(file_source_other, fifo_target)
    wait_for_absent(file_source)
    wait_for_absent(file_source_other)
    wait_for_regular_file(fifo_target)
    wait_for_regular_file(fifo_target_other)
    wait_for_bytes(fifo_target, b"replaced-by-file")
    wait_for_bytes(fifo_target_other, b"replaced-by-file")

    print("OK rename-fifo-file-pairings fifo_over_file=success file_over_fifo=success")


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
    previous_attr_timeout = os.environ.get("FOD_ATTR_TIMEOUT_SECONDS")
    os.environ["FOD_ATTR_TIMEOUT_SECONDS"] = "0"

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
            cross_directory_move_updates_parent_link_counts(mount_a, mount_b)
            nonempty_target_is_preserved(mount_a, mount_b)
            missing_source_preserves_destination(mount_a, mount_b)
            type_mismatch_errors(mount_a, mount_b)
            file_and_symlink_type_pairings(mount_a, mount_b)
            fifo_file_type_pairings(mount_a, mount_b)
            descendant_is_rejected(mount_a, mount_b)
            rollback_after_target_removal(mount_a, mount_b, barrier_dir)

            print("OK rename-directory-transactional protected_cases=8")
        except BaseException:
            print("\n=== MOUNT A LOG ===")
            launcher_a._dump_log()
            print("\n=== MOUNT B LOG ===")
            launcher_b._dump_log()
            raise
        finally:
            launcher_b.stop()
            launcher_a.stop()
            if previous_attr_timeout is None:
                os.environ.pop("FOD_ATTR_TIMEOUT_SECONDS", None)
            else:
                os.environ["FOD_ATTR_TIMEOUT_SECONDS"] = previous_attr_timeout
            if previous_barrier is None:
                os.environ.pop(BARRIER_ENV, None)
            else:
                os.environ[BARRIER_ENV] = previous_barrier


if __name__ == "__main__":
    main()
