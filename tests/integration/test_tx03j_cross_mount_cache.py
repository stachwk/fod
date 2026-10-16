#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX03J.1: separate cross-mount stale reads from write ownership fencing.

Diagnostic mode reports a stale read without hiding it. --require-coherent
turns any stale read from an already-fsynced writer into a test failure.
"""

from __future__ import annotations

import argparse
import os
import tempfile
import time
import uuid
from pathlib import Path

from fod_mount import FODMount


def read_exact(path: Path, expected: bytes) -> bool:
    return path.read_bytes() == expected


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--direct-io", action="store_true")
    parser.add_argument("--no-read-cache", action="store_true")
    parser.add_argument("--metadata-ttl-zero", action="store_true")
    parser.add_argument("--wait-seconds", type=float, default=2.0)
    parser.add_argument("--require-coherent", action="store_true")
    args = parser.parse_args()
    if args.wait_seconds < 0 or args.wait_seconds > 60:
        raise ValueError("wait-seconds must be between 0 and 60")
    db = os.environ.get("FOD_PG_DBNAME") or os.environ.get("POSTGRES_DB")
    if not db or not db.endswith("_test"):
        raise RuntimeError("TX03J requires a dedicated *_test database")
    if args.direct_io:
        os.environ["FOD_FOPEN_DIRECT_IO"] = "1"
    if args.no_read_cache:
        os.environ["FOD_READ_CACHE_BLOCKS"] = "0"
    if args.metadata_ttl_zero:
        os.environ["FOD_METADATA_CACHE_TTL"] = "0"

    root = Path(__file__).resolve().parents[2]
    name = f"tx03j-{uuid.uuid4().hex}.bin"
    old = b"A" * 65536
    new = b"B" * 65536
    with tempfile.TemporaryDirectory(prefix="fod-tx03j-") as tmp:
        a, b, c = (FODMount(str(root)) for _ in range(3))
        mount_a = Path(tmp) / "mount-a"
        mount_b = Path(tmp) / "mount-b"
        mount_c = Path(tmp) / "mount-c"
        mounted = []
        try:
            for launcher, path, tag in (
                (a, mount_a, "a"), (b, mount_b, "b")
            ):
                launcher.start(str(path), log_prefix=f"/tmp/fod-tx03j-{tag}")
                mounted.append(launcher)

            path_a, path_b = mount_a / name, mount_b / name
            with open(path_a, "wb", buffering=0) as file_a:
                file_a.write(old)
                os.fsync(file_a.fileno())
            if not read_exact(path_b, old):
                raise AssertionError("TX03J initial cross-mount read mismatch")
            if not read_exact(path_a, old):
                raise AssertionError("TX03J initial mount A read mismatch")

            with open(path_b, "r+b", buffering=0) as file_b:
                file_b.seek(0)
                file_b.write(new)
                os.fsync(file_b.fileno())
            if not read_exact(path_b, new):
                raise AssertionError("TX03J writer B cannot read its committed data")

            a_immediate = read_exact(path_a, new)
            time.sleep(args.wait_seconds)
            a_delayed = read_exact(path_a, new)
            c.start(str(mount_c), log_prefix="/tmp/fod-tx03j-c")
            mounted.append(c)
            fresh_c = read_exact(mount_c / name, new)
            if not fresh_c:
                raise AssertionError("TX03J fresh mount C cannot see committed data")

            print(
                f"TX03J_DIAGNOSTIC=PASS direct_io={int(args.direct_io)} "
                f"no_read_cache={int(args.no_read_cache)} "
                f"metadata_ttl_zero={int(args.metadata_ttl_zero)} "
                f"a_immediate_current={int(a_immediate)} "
                f"a_delayed_current={int(a_delayed)} "
                "b_current=1 c_current=1 "
                f"wait_seconds={args.wait_seconds}",
                flush=True,
            )
            if args.require_coherent and (not a_immediate or not a_delayed):
                raise AssertionError("TX03J cross-mount A served stale data")
            path_b.unlink()
        except Exception:
            for launcher in mounted:
                launcher._dump_log()
            raise
        finally:
            for launcher in reversed(mounted):
                launcher.stop()


if __name__ == "__main__":
    main()
