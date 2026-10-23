#!/usr/bin/env python
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX03G: repeated open/read versus unlink races on two independent FUSE mounts."""

from __future__ import annotations

import argparse
import errno
import os
import random
import sys
import tempfile
import threading
import time
import uuid
from pathlib import Path

from fod_mount import FODMount


def print_err(value: str, verbose: int = 0) -> None:
    if verbose == 1:
        print(value, file=sys.stderr, flush=True)


def assert_absent(path: Path) -> None:
    # Wynik syscall open jest wazniejszy od cache stat.
    try:
        fd = os.open(path, os.O_RDONLY)
    except OSError as exc:
        if exc.errno == errno.ENOENT:
            return
        raise
    else:
        os.close(fd)
        raise AssertionError(f"TX03G removed path opened: {path}")


def race_once(read_path: Path, unlink_path: Path, payload: bytes,
              rng: random.Random, verbose: int) -> str:
    barrier = threading.Barrier(3)
    outcome: dict[str, object] = {}
    errors: list[BaseException] = []
    read_delay = rng.uniform(0.0, 0.003)
    unlink_delay = rng.uniform(0.0, 0.003)

    def reader() -> None:
        fd = None
        try:
            barrier.wait(timeout=10)
            time.sleep(read_delay)
            try:
                fd = os.open(read_path, os.O_RDONLY)
            except OSError as exc:
                if exc.errno != errno.ENOENT:
                    raise
                outcome["reader"] = "ENOENT"
                return
            # Odczyt po otwarciu moze nastapic juz po zakonczonym unlink.
            time.sleep(0.001)
            data = os.pread(fd, len(payload), 0)
            if data != payload:
                raise AssertionError(
                    f"TX03G corrupted read expected={len(payload)} got={len(data)}"
                )
            outcome["reader"] = "READ_OK"
        except BaseException as exc:
            errors.append(exc)
        finally:
            if fd is not None:
                os.close(fd)

    def unlinker() -> None:
        try:
            barrier.wait(timeout=10)
            time.sleep(unlink_delay)
            unlink_path.unlink()
            outcome["unlink"] = "OK"
        except BaseException as exc:
            errors.append(exc)

    first = threading.Thread(target=reader, name="tx03g-reader")
    second = threading.Thread(target=unlinker, name="tx03g-unlink")
    first.start()
    second.start()
    try:
        barrier.wait(timeout=10)
        first.join(timeout=30)
        second.join(timeout=30)
        if first.is_alive() or second.is_alive():
            raise AssertionError("TX03G worker timeout or deadlock")
        if errors:
            raise AssertionError(f"TX03G concurrent worker failure: {errors!r}")
        if outcome.get("unlink") != "OK" or outcome.get("reader") not in ("READ_OK", "ENOENT"):
            raise AssertionError(f"TX03G incomplete result: {outcome}")
        assert_absent(read_path)
        assert_absent(unlink_path)
        print_err(f"TX03G iteration result={outcome}", verbose)
        return str(outcome["reader"])
    finally:
        # Watki daemon=False; przy przekroczeniu timeout nie zdejmujemy mountow.
        if first.is_alive() or second.is_alive():
            raise RuntimeError("TX03G worker thread remains alive")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=1000)
    parser.add_argument("--seed", type=int, default=20261015)
    parser.add_argument("--verbose", type=int, choices=(0, 1), default=0)
    args = parser.parse_args()
    if args.iterations < 1:
        raise ValueError("--iterations must be positive")
    dbname = os.environ.get("FOD_PG_DBNAME") or os.environ.get("POSTGRES_DB")
    if not dbname or not dbname.endswith("_test"):
        raise RuntimeError("TX03G requires explicit dedicated *_test database")

    root = Path(__file__).resolve().parents[2]
    rng = random.Random(args.seed)
    prefix = "tx03g-" + uuid.uuid4().hex
    payload = bytes(range(256)) * 512
    results = {"READ_OK": 0, "ENOENT": 0}
    with tempfile.TemporaryDirectory(prefix="fod-tx03g-") as tmp:
        a = Path(tmp) / "mount-a"
        b = Path(tmp) / "mount-b"
        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        try:
            launcher_a.start(str(a), log_prefix="/tmp/fod-tx03g-a")
            launcher_b.start(str(b), log_prefix="/tmp/fod-tx03g-b")
            for index in range(args.iterations):
                # Naprzemienny kierunek sprawdza cache obu procesow FUSE.
                read_mount, unlink_mount = ((a, b) if index % 2 == 0 else (b, a))
                name = f"{prefix}-{index:05d}.bin"
                writer = unlink_mount / name
                writer.write_bytes(payload)
                result = race_once(read_mount / name, writer, payload, rng, args.verbose)
                results[result] += 1
                if (index + 1) % 100 == 0:
                    print(f"TX03G_PROGRESS=PASS iterations={index + 1}", flush=True)
            print(
                f"TX03G_RACE=PASS iterations={args.iterations} "
                f"read_ok={results['READ_OK']} enoent={results['ENOENT']} "
                f"seed={args.seed} mounts=2",
                flush=True,
            )
        except Exception:
            launcher_a._dump_log()
            launcher_b._dump_log()
            raise
        finally:
            try:
                launcher_b.stop()
            finally:
                launcher_a.stop()


if __name__ == "__main__":
    main()
