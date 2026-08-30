#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def assert_shrink_extend_zero_fill(
    file_path: Path,
    payload: bytes,
    shrink_size: int,
    extend_size: int,
) -> None:
    file_path.write_bytes(payload)

    os.truncate(file_path, shrink_size)
    assert file_path.stat().st_size == shrink_size
    assert file_path.read_bytes() == payload[:shrink_size]

    os.truncate(file_path, extend_size)
    assert file_path.stat().st_size == extend_size

    expected = payload[:shrink_size] + (b"\x00" * (extend_size - shrink_size))
    actual = file_path.read_bytes()
    assert len(actual) == extend_size, len(actual)
    assert actual == expected, (
        shrink_size,
        extend_size,
        actual[shrink_size : min(extend_size, shrink_size + 64)],
    )


def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    block_size = 4096
    payload = (b"A" * block_size) + (b"B" * 1904)
    expected = (b"A" * block_size) + (b"\x00" * block_size)

    with tempfile.TemporaryDirectory(prefix="/tmp/fod-truncate-block.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            dir_path = mountpoint / "truncate_block"
            file_path = dir_path / "payload.bin"

            dir_path.mkdir()
            file_path.write_bytes(payload)

            os.truncate(file_path, block_size)
            assert file_path.stat().st_size == block_size

            os.truncate(file_path, block_size * 2)
            assert file_path.stat().st_size == block_size * 2

            data = file_path.read_bytes()
            assert len(data) == block_size * 2, len(data)
            assert data == expected, data[block_size : block_size + 64]

            cross_block_path = dir_path / "off-boundary-cross-block.bin"
            cross_block_payload = (
                (b"C" * block_size)
                + (b"D" * block_size)
                + (b"E" * block_size)
            )
            assert_shrink_extend_zero_fill(
                cross_block_path,
                cross_block_payload,
                block_size + 333,
                block_size * 2 + 123,
            )

            same_block_path = dir_path / "off-boundary-same-block.bin"
            same_block_payload = b"F" * block_size
            assert_shrink_extend_zero_fill(
                same_block_path,
                same_block_payload,
                333,
                777,
            )

            print("OK truncate shrink block boundary")
            print("OK truncate off-boundary extend zero fill")
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
