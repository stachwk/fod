#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount
from tests.integration.test_fallocate_contract import (
    FALLOC_FL_KEEP_SIZE,
    FALLOC_FL_PUNCH_HOLE,
    raw_fallocate,
)
from tests.integration.test_fallocate_punch_hole import configured_block_size, connect
from tests.integration.test_lseek_sparse_contract import (
    SEEK_DATA,
    SEEK_HOLE,
    deterministic_block,
    prepare_layout,
)

MODE_PUNCH = FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE


def assert_seek(path: Path, offset: int, whence: int, expected: int | None) -> None:
    fd = os.open(path, os.O_RDONLY)
    try:
        try:
            actual = int(os.lseek(fd, offset, whence))
        except OSError as error:
            if expected is not None:
                raise AssertionError(
                    f"seek offset={offset} whence={whence} unexpected errno={error.errno}"
                ) from error
            if error.errno != errno.ENXIO:
                raise AssertionError(
                    f"seek offset={offset} whence={whence} expected ENXIO "
                    f"got errno={error.errno}"
                ) from error
            return
    finally:
        os.close(fd)

    if expected is None:
        raise AssertionError(
            f"seek offset={offset} whence={whence} expected ENXIO got result={actual}"
        )
    if actual != expected:
        raise AssertionError(
            f"seek offset={offset} whence={whence} expected={expected} got={actual}"
        )


def assert_layout(path: Path, layout: str, block_size: int) -> None:
    eof = 4 * block_size

    cases = {
        "dense": (
            (0, SEEK_DATA, 0),
            (0, SEEK_HOLE, eof),
            (block_size + 17, SEEK_DATA, block_size + 17),
            (block_size + 17, SEEK_HOLE, eof),
            (eof, SEEK_DATA, None),
            (eof, SEEK_HOLE, None),
            (eof + 1, SEEK_DATA, None),
            (eof + 1, SEEK_HOLE, None),
        ),
        "punched": (
            (0, SEEK_DATA, 0),
            (0, SEEK_HOLE, block_size),
            (block_size, SEEK_DATA, 2 * block_size),
            (block_size, SEEK_HOLE, block_size),
            (block_size + 17, SEEK_DATA, 2 * block_size),
            (block_size + 17, SEEK_HOLE, block_size + 17),
            (2 * block_size, SEEK_DATA, 2 * block_size),
            (2 * block_size, SEEK_HOLE, eof),
        ),
        "sparse_gap": (
            (0, SEEK_DATA, 0),
            (0, SEEK_HOLE, block_size),
            (block_size, SEEK_DATA, 3 * block_size),
            (block_size, SEEK_HOLE, block_size),
            (2 * block_size, SEEK_DATA, 3 * block_size),
            (2 * block_size, SEEK_HOLE, 2 * block_size),
            (3 * block_size, SEEK_DATA, 3 * block_size),
            (3 * block_size, SEEK_HOLE, eof),
        ),
        "trailing_hole": (
            (0, SEEK_DATA, 0),
            (0, SEEK_HOLE, block_size),
            (block_size, SEEK_DATA, None),
            (block_size, SEEK_HOLE, block_size),
            (eof - 1, SEEK_DATA, None),
            (eof - 1, SEEK_HOLE, eof - 1),
            (eof, SEEK_DATA, None),
            (eof, SEEK_HOLE, None),
        ),
        "empty": (
            (0, SEEK_DATA, None),
            (0, SEEK_HOLE, None),
        ),
    }

    for offset, whence, expected in cases[layout]:
        assert_seek(path, offset, whence, expected)


def partial_block_case(path: Path, block_size: int) -> None:
    payload = b"".join(
        deterministic_block(block_size, seed)
        for seed in range(4)
    )
    fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_TRUNC, 0o644)
    try:
        written = os.write(fd, payload)
        if written != len(payload):
            raise AssertionError(f"short write {written}/{len(payload)}")
        os.fsync(fd)
        start = block_size + (block_size // 4)
        result = raw_fallocate(
            fd,
            MODE_PUNCH,
            start,
            block_size // 2,
        )
        if result.rc != 0:
            raise AssertionError(f"partial PUNCH_HOLE failed: {result}")
        os.fsync(fd)
    finally:
        os.close(fd)

    inside = block_size + (block_size // 2)
    eof = 4 * block_size
    assert_seek(path, inside, SEEK_DATA, inside)
    assert_seek(path, inside, SEEK_HOLE, eof)


def pending_write_case(path: Path, block_size: int) -> None:
    fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_TRUNC, 0o644)
    try:
        os.ftruncate(fd, 4 * block_size)
        data = deterministic_block(block_size, 9)
        written = os.pwrite(fd, data, 2 * block_size)
        if written != len(data):
            raise AssertionError(f"short pending write {written}/{len(data)}")

        data_at = int(os.lseek(fd, 0, SEEK_DATA))
        if data_at != 2 * block_size:
            raise AssertionError(
                f"pending SEEK_DATA expected={2 * block_size} got={data_at}"
            )
        hole_at = int(os.lseek(fd, 2 * block_size, SEEK_HOLE))
        if hole_at != 3 * block_size:
            raise AssertionError(
                f"pending SEEK_HOLE expected={3 * block_size} got={hole_at}"
            )
    finally:
        os.close(fd)


def multi_digit_order_case(path: Path, block_size: int) -> None:
    fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_TRUNC, 0o644)
    try:
        os.ftruncate(fd, 12 * block_size)

        block_two = deterministic_block(block_size, 12)
        block_ten = deterministic_block(block_size, 20)

        written = os.pwrite(fd, block_two, 2 * block_size)
        if written != len(block_two):
            raise AssertionError(
                f"multi-digit short write block=2 {written}/{len(block_two)}"
            )

        written = os.pwrite(fd, block_ten, 10 * block_size)
        if written != len(block_ten):
            raise AssertionError(
                f"multi-digit short write block=10 {written}/{len(block_ten)}"
            )

        os.fsync(fd)
    finally:
        os.close(fd)

    assert_seek(path, 0, SEEK_DATA, 2 * block_size)
    assert_seek(path, 3 * block_size, SEEK_DATA, 10 * block_size)
    assert_seek(path, 2 * block_size, SEEK_HOLE, 3 * block_size)
    assert_seek(path, 10 * block_size, SEEK_HOLE, 11 * block_size)


def remount_case(
    template: FODMount,
    block_size: int,
    suffix: str,
) -> None:
    name = f"s1-runtime-remount-{suffix}.bin"

    with tempfile.TemporaryDirectory(
        prefix="/tmp/fod-lseek-s1-remount."
    ) as tmpdir:
        mountpoint = Path(tmpdir)

        first = FODMount(str(ROOT))
        first.postgres_db = template.postgres_db
        first.postgres_user = template.postgres_user
        first.postgres_password = template.postgres_password
        first.start(
            tmpdir,
            log_prefix="/tmp/fod-lseek-s1-remount-first",
        )
        try:
            path = mountpoint / name
            prepare_layout(path, block_size, "punched")
            assert_layout(path, "punched", block_size)
        finally:
            first.stop()

        second = FODMount(str(ROOT))
        second.postgres_db = template.postgres_db
        second.postgres_user = template.postgres_user
        second.postgres_password = template.postgres_password
        second.start(
            tmpdir,
            log_prefix="/tmp/fod-lseek-s1-remount-second",
        )
        try:
            path = mountpoint / name
            assert_layout(path, "punched", block_size)
            path.unlink()
        finally:
            second.stop()


def main() -> int:
    if sys.platform != "linux":
        raise RuntimeError("S1.2 sparse lseek runtime test is Linux-specific")

    template = FODMount(str(ROOT))
    template.init_schema()

    launcher = FODMount(str(ROOT))
    launcher.postgres_db = template.postgres_db
    launcher.postgres_user = template.postgres_user
    launcher.postgres_password = template.postgres_password

    suffix = uuid.uuid4().hex[:12]

    with tempfile.TemporaryDirectory(prefix="/tmp/fod-lseek-s1-runtime.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(tmpdir, log_prefix="/tmp/fod-lseek-s1-runtime")
        try:
            connection = connect(launcher)
            try:
                block_size = configured_block_size(connection)
            finally:
                connection.close()

            for layout in (
                "dense",
                "punched",
                "sparse_gap",
                "trailing_hole",
                "empty",
            ):
                path = mountpoint / f"s1-runtime-{layout}-{suffix}.bin"
                prepare_layout(path, block_size, layout)
                assert_layout(path, layout, block_size)
                path.unlink()

            partial_path = mountpoint / f"s1-runtime-partial-{suffix}.bin"
            partial_block_case(partial_path, block_size)
            partial_path.unlink()

            pending_path = mountpoint / f"s1-runtime-pending-{suffix}.bin"
            multi_digit_path = (
                mountpoint / f"s1-runtime-multi-digit-{suffix}.bin"
            )
            multi_digit_order_case(multi_digit_path, block_size)
            multi_digit_path.unlink()

            pending_write_case(pending_path, block_size)
            pending_path.unlink()

            remount_case(template, block_size, suffix)

            log_text = (
                launcher.config.log_file.read_text(
                    encoding="utf-8",
                    errors="replace",
                )
                if launcher.config is not None
                else ""
            )
            default_hits = log_text.count("[Not Implemented] lseek(")
            if default_hits != 0:
                raise AssertionError(
                    f"default fuser lseek callback used {default_hits} times"
                )
        finally:
            launcher.stop()

    print(
        "OK lseek-sparse-runtime "
        f"block_size={block_size} "
        "dense=1 punched=1 sparse_gap=1 trailing_hole=1 empty=1 "
        "partial_block=1 multi_digit_order=1 pending_write=1 remount=1 default_fuser_callback=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
