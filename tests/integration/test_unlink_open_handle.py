#!/usr/bin/env python
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import tempfile
import uuid
from pathlib import Path

from fod_mount import FODMount


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    with tempfile.TemporaryDirectory(prefix="fod-unlink-open-handle-") as temp_dir:
        mountpoint = Path(temp_dir) / "mount"

        launcher = FODMount(str(root))
        launcher.init_schema()

        name = f"unlink-open-handle-{uuid.uuid4().hex}.bin"
        payload = b"FOD open file handle must survive unlink\n"

        handle = None
        try:
            launcher.start(
                str(mountpoint),
                log_prefix="/tmp/fod-unlink-open-handle",
            )

            target = mountpoint / name
            target.write_bytes(payload)

            before = target.stat()
            target_ino = before.st_ino

            handle = target.open("rb")
            if handle.read() != payload:
                raise AssertionError("pre-unlink open-handle payload mismatch")
            handle.seek(0)

            target.unlink()

            if target.exists():
                raise AssertionError("unlinked pathname is still visible")

            try:
                after_unlink = handle.read()
            except OSError as err:
                raise AssertionError(
                    "open file handle failed after unlink: "
                    f"errno={err.errno} error={err}"
                ) from err

            if after_unlink != payload:
                raise AssertionError(
                    "open file handle returned wrong payload after unlink: "
                    f"expected={payload!r} actual={after_unlink!r}"
                )

            handle.seek(0)
            try:
                second_read = os.read(handle.fileno(), len(payload))
            except OSError as err:
                raise AssertionError(
                    "raw fd read failed after unlink: "
                    f"errno={err.errno} error={err}"
                ) from err

            if second_read != payload:
                raise AssertionError(
                    "raw fd returned wrong payload after unlink: "
                    f"expected={payload!r} actual={second_read!r}"
                )

            handle.close()
            handle = None

            if target.exists():
                raise AssertionError("unlinked pathname reappeared after final close")

            print(
                "OK unlink-open-handle "
                f"ino={target_ino} pathname_removed=1 "
                "buffered_read_after_unlink=1 raw_fd_read_after_unlink=1 "
                "final_close=1 pathname_absent=1"
            )
        except Exception:
            launcher._dump_log()
            raise
        finally:
            if handle is not None:
                handle.close()
            launcher.stop()


if __name__ == "__main__":
    main()
