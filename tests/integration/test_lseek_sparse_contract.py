#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import json
import os
import sys
import tempfile
import uuid
from dataclasses import asdict, dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount
from tests.integration.test_fallocate_contract import (
    FALLOC_FL_KEEP_SIZE,
    FALLOC_FL_PUNCH_HOLE,
    db_snapshot,
    file_snapshot,
    raw_fallocate,
)
from tests.integration.test_fallocate_punch_hole import (
    configured_block_size,
    connect,
)

SEEK_DATA = getattr(os, "SEEK_DATA", 3)
SEEK_HOLE = getattr(os, "SEEK_HOLE", 4)


@dataclass(frozen=True)
class SeekResult:
    offset: int
    whence: int
    whence_name: str
    rc: int
    result: int | None
    errno: int
    errno_name: str


def seek_once(path: Path, offset: int, whence: int) -> SeekResult:
    whence_name = {
        SEEK_DATA: "SEEK_DATA",
        SEEK_HOLE: "SEEK_HOLE",
    }.get(whence, str(whence))

    fd = os.open(path, os.O_RDONLY)
    try:
        try:
            result = int(os.lseek(fd, offset, whence))
        except OSError as error:
            error_number = int(error.errno or 0)
            return SeekResult(
                offset=offset,
                whence=whence,
                whence_name=whence_name,
                rc=-1,
                result=None,
                errno=error_number,
                errno_name=errno.errorcode.get(error_number, "UNKNOWN"),
            )
    finally:
        os.close(fd)

    return SeekResult(
        offset=offset,
        whence=whence,
        whence_name=whence_name,
        rc=0,
        result=result,
        errno=0,
        errno_name="OK",
    )


def deterministic_block(block_size: int, seed: int) -> bytes:
    return bytes(
        (((index + seed) * 29) + 7) % 251
        for index in range(block_size)
    )


def write_all(fd: int, data: bytes, offset: int) -> None:
    total = 0
    while total < len(data):
        written = os.pwrite(fd, data[total:], offset + total)
        if written <= 0:
            raise AssertionError(
                f"short pwrite offset={offset} total={total} written={written}"
            )
        total += written


def prepare_layout(path: Path, block_size: int, layout: str) -> None:
    fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_TRUNC, 0o644)
    try:
        if layout == "dense":
            payload = b"".join(
                deterministic_block(block_size, seed)
                for seed in range(4)
            )
            write_all(fd, payload, 0)
        elif layout == "punched":
            payload = b"".join(
                deterministic_block(block_size, seed)
                for seed in range(4)
            )
            write_all(fd, payload, 0)
            os.fsync(fd)
            result = raw_fallocate(
                fd,
                FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
                block_size,
                block_size,
            )
            if result.rc != 0:
                raise AssertionError(
                    f"cannot prepare punched layout: {result}"
                )
        elif layout == "sparse_gap":
            os.ftruncate(fd, 4 * block_size)
            write_all(fd, deterministic_block(block_size, 1), 0)
            write_all(
                fd,
                deterministic_block(block_size, 4),
                3 * block_size,
            )
        elif layout == "trailing_hole":
            os.ftruncate(fd, 4 * block_size)
            write_all(fd, deterministic_block(block_size, 2), 0)
        elif layout == "empty":
            pass
        else:
            raise AssertionError(f"unknown layout={layout}")
        os.fsync(fd)
    finally:
        os.close(fd)


def queries(layout: str, block_size: int, size: int) -> list[tuple[int, int]]:
    if layout == "dense":
        return [
            (0, SEEK_DATA),
            (0, SEEK_HOLE),
            (block_size + 17, SEEK_DATA),
            (block_size + 17, SEEK_HOLE),
            (size, SEEK_DATA),
            (size, SEEK_HOLE),
            (size + 1, SEEK_DATA),
            (size + 1, SEEK_HOLE),
        ]
    if layout == "punched":
        return [
            (0, SEEK_DATA),
            (0, SEEK_HOLE),
            (block_size, SEEK_DATA),
            (block_size, SEEK_HOLE),
            (block_size + 17, SEEK_DATA),
            (block_size + 17, SEEK_HOLE),
            (2 * block_size, SEEK_DATA),
            (2 * block_size, SEEK_HOLE),
        ]
    if layout == "sparse_gap":
        return [
            (0, SEEK_DATA),
            (0, SEEK_HOLE),
            (block_size, SEEK_DATA),
            (block_size, SEEK_HOLE),
            (2 * block_size, SEEK_DATA),
            (2 * block_size, SEEK_HOLE),
            (3 * block_size, SEEK_DATA),
            (3 * block_size, SEEK_HOLE),
        ]
    if layout == "trailing_hole":
        return [
            (0, SEEK_DATA),
            (0, SEEK_HOLE),
            (block_size, SEEK_DATA),
            (block_size, SEEK_HOLE),
            (size - 1, SEEK_DATA),
            (size - 1, SEEK_HOLE),
            (size, SEEK_DATA),
            (size, SEEK_HOLE),
        ]
    if layout == "empty":
        return [
            (0, SEEK_DATA),
            (0, SEEK_HOLE),
        ]
    raise AssertionError(f"unknown layout={layout}")


def stable_file_state(snapshot) -> tuple:
    return (
        snapshot.size,
        snapshot.blocks_512,
        snapshot.mtime_ns,
        snapshot.ctime_ns,
        snapshot.sha256,
    )


def run_layout(
    template: FODMount,
    layout: str,
    block_size: int,
    suffix: str,
) -> dict:
    launcher = FODMount(str(ROOT))
    launcher.postgres_db = template.postgres_db
    launcher.postgres_user = template.postgres_user
    launcher.postgres_password = template.postgres_password

    name = f"lseek-s1-{layout}-{suffix}.bin"

    with tempfile.TemporaryDirectory(
        prefix=f"/tmp/fod-lseek-s1-{layout}."
    ) as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(
            tmpdir,
            log_prefix=f"/tmp/fod-lseek-s1-{layout}",
        )
        try:
            path = mountpoint / name
            prepare_layout(path, block_size, layout)

            connection = connect(launcher)
            try:
                before_file = file_snapshot(path)
                before_db = db_snapshot(connection, name)

                results = [
                    seek_once(path, offset, whence)
                    for offset, whence in queries(
                        layout,
                        block_size,
                        before_file.size,
                    )
                ]

                after_file = file_snapshot(path)
                after_db = db_snapshot(connection, name)
            finally:
                connection.close()

            if stable_file_state(after_file) != stable_file_state(before_file):
                raise AssertionError(
                    f"{layout}: lseek calls mutated visible file state"
                )
            if after_db != before_db:
                raise AssertionError(
                    f"{layout}: lseek calls mutated PostgreSQL state"
                )

            log_text = (
                launcher.config.log_file.read_text(
                    encoding="utf-8",
                    errors="replace",
                )
                if launcher.config is not None
                else ""
            )
            default_count = log_text.count("[Not Implemented] lseek(")

            record = {
                "layout": layout,
                "file": asdict(before_file),
                "db": asdict(before_db),
                "default_fuser_lseek_count": default_count,
                "results": [asdict(result) for result in results],
            }

            print(json.dumps(record, sort_keys=True))
            path.unlink()
            return record
        finally:
            launcher.stop()


def main() -> int:
    if sys.platform != "linux":
        raise RuntimeError(
            "S1 sparse lseek baseline is Linux-specific"
        )

    template = FODMount(str(ROOT))
    template.init_schema()

    probe = FODMount(str(ROOT))
    probe.postgres_db = template.postgres_db
    probe.postgres_user = template.postgres_user
    probe.postgres_password = template.postgres_password

    with tempfile.TemporaryDirectory(
        prefix="/tmp/fod-lseek-s1-probe."
    ) as tmpdir:
        probe.start(
            tmpdir,
            log_prefix="/tmp/fod-lseek-s1-probe",
        )
        try:
            connection = connect(probe)
            try:
                block_size = configured_block_size(connection)
            finally:
                connection.close()
        finally:
            probe.stop()

    suffix = uuid.uuid4().hex[:12]
    layouts = (
        "dense",
        "punched",
        "sparse_gap",
        "trailing_hole",
        "empty",
    )
    records = [
        run_layout(
            template,
            layout,
            block_size,
            suffix,
        )
        for layout in layouts
    ]

    callback_hits = sum(
        int(record["default_fuser_lseek_count"])
        for record in records
    )
    successes = sum(
        1
        for record in records
        for result in record["results"]
        if int(result["rc"]) == 0
    )
    failures = sum(
        1
        for record in records
        for result in record["results"]
        if int(result["rc"]) != 0
    )

    print(
        "OK lseek-sparse-contract-s1 "
        f"layouts={len(records)} "
        f"block_size={block_size} "
        f"successes={successes} "
        f"failures={failures} "
        f"default_fuser_callback_hits={callback_hits} "
        "state_unchanged=1"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
