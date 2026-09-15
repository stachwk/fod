#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import sys
import tempfile
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount
from tests.integration.test_fallocate_contract import (
    FALLOC_FL_KEEP_SIZE,
    FALLOC_FL_PUNCH_HOLE,
    file_snapshot,
    raw_fallocate,
)

MODE = FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE


def main() -> int:
    template = FODMount(str(ROOT))
    template.init_schema()

    launcher = FODMount(str(ROOT))
    launcher.postgres_db = template.postgres_db
    launcher.postgres_user = template.postgres_user
    launcher.postgres_password = template.postgres_password

    suffix = uuid.uuid4().hex[:12]
    name = f"fallocate-timestamps-{suffix}.bin"

    with tempfile.TemporaryDirectory(
        prefix="/tmp/fod-fallocate-timestamps."
    ) as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(tmpdir, log_prefix="/tmp/fod-fallocate-timestamps")
        try:
            path = mountpoint / name
            payload = bytes(((index * 37) + 5) % 251 for index in range(96 * 1024))
            path.write_bytes(payload)

            fd = os.open(path, os.O_RDWR)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)

            before = file_snapshot(path)
            time.sleep(0.02)

            fd = os.open(path, os.O_RDWR)
            try:
                punch = raw_fallocate(fd, MODE, 32 * 1024, 32 * 1024)
                if punch.rc != 0:
                    raise AssertionError(f"in-range punch failed: {punch}")
                os.fsync(fd)
            finally:
                os.close(fd)

            after_punch = file_snapshot(path)
            if after_punch.size != before.size:
                raise AssertionError("PUNCH_HOLE changed logical size")
            if after_punch.mtime_ns <= before.mtime_ns:
                raise AssertionError(
                    f"PUNCH_HOLE did not advance mtime: {before.mtime_ns} -> {after_punch.mtime_ns}"
                )
            if after_punch.ctime_ns <= before.ctime_ns:
                raise AssertionError(
                    f"PUNCH_HOLE did not advance ctime: {before.ctime_ns} -> {after_punch.ctime_ns}"
                )

            time.sleep(0.02)
            before_noop = file_snapshot(path)

            fd = os.open(path, os.O_RDWR)
            try:
                beyond = raw_fallocate(
                    fd,
                    MODE,
                    before_noop.size + (64 * 1024),
                    32 * 1024,
                )
                if beyond.rc != 0:
                    raise AssertionError(f"beyond-EOF punch failed: {beyond}")
                os.fsync(fd)
            finally:
                os.close(fd)

            after_noop = file_snapshot(path)
            if (
                after_noop.size,
                after_noop.blocks_512,
                after_noop.mtime_ns,
                after_noop.ctime_ns,
                after_noop.sha256,
            ) != (
                before_noop.size,
                before_noop.blocks_512,
                before_noop.mtime_ns,
                before_noop.ctime_ns,
                before_noop.sha256,
            ):
                raise AssertionError(
                    "beyond-EOF PUNCH_HOLE changed visible state or timestamps"
                )

            path.unlink()
        finally:
            launcher.stop()

    print(
        "OK fallocate-timestamps "
        "punch_mtime=advanced punch_ctime=advanced beyond_eof_timestamps=stable"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
