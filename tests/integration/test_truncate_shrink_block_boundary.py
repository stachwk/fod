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
            assert data == expected, data[block_size:block_size + 64]

            print("OK truncate shrink block boundary")
        finally:
            launcher.stop()


if __name__ == "__main__":
    main()
