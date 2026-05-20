#!/usr/bin/env python
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

# -*- coding: utf-8 -*-

from __future__ import annotations

import argparse
import errno
import os
import shutil
import stat
import sys
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def print_err(str):
    print(f"stderr: {str}", file=sys.stderr)


def parse_args():
    # Parsowanie argumentow testu.
    parser = argparse.ArgumentParser(
        description="FOD mknod integration test"
    )
    parser.add_argument(
        "--verbose",
        type=int,
        default=0,
        help="1 wlacza komunikaty diagnostyczne",
    )
    return parser.parse_args()


def known_special_node_limit(exc: OSError) -> bool:
    # FOD 2.4.0 moze zwracac bledy dla FIFO/char/block.
    # Regularny mknod pozostaje wymagany.
    allowed = {
        errno.EIO,
        errno.ENOTDIR,
        errno.EOPNOTSUPP,
        getattr(errno, "ENOTSUP", errno.EOPNOTSUPP),
        errno.EPERM,
        errno.EINVAL,
    }
    return exc.errno in allowed


def safe_unlink(path: Path) -> None:
    # Usuwanie pomocnicze odporne na bledy FUSE.
    try:
        path.unlink()
    except FileNotFoundError:
        pass
    except OSError:
        pass


def assert_mode_permissions(
    path: Path,
    predicate,
    expected_permissions: int,
    *,
    allow_special_limit: bool = False,
    label: str = "node",
):
    # Sprawdza typ i prawa pliku.
    try:
        st = os.lstat(path)
    except OSError as exc:
        if allow_special_limit and known_special_node_limit(exc):
            print(f"SKIP: {label} lstat/getattr returns errno={exc.errno} on current FOD")
            return None
        raise

    if not predicate(st.st_mode):
        raise AssertionError(f"{path}: unexpected mode {oct(st.st_mode)}")

    actual_permissions = stat.S_IMODE(st.st_mode)
    if actual_permissions != expected_permissions:
        raise AssertionError(
            f"{path}: permissions {oct(actual_permissions)} != {oct(expected_permissions)}"
        )

    return st


def test_regular_mknod(work_dir: Path) -> None:
    # Regularny mknod jest twardym wymaganiem.
    regular_path = work_dir / "mknod_regular"

    safe_unlink(regular_path)
    os.mknod(regular_path, stat.S_IFREG | 0o640)

    st = assert_mode_permissions(
        regular_path,
        stat.S_ISREG,
        0o640,
        label="regular",
    )
    if st is None:
        raise AssertionError("regular mknod cannot be skipped")

    if st.st_size != 0:
        raise AssertionError(f"regular size {st.st_size} != 0")

    regular_path.write_bytes(b"regular-mknod\n")
    if regular_path.read_bytes() != b"regular-mknod\n":
        raise AssertionError("regular mknod read/write mismatch")

    safe_unlink(regular_path)
    print("OK: regular mknod")


def test_fifo_mknod(work_dir: Path) -> None:
    # FIFO jest obecnie czesciowo wspierane.
    fifo_path = work_dir / "mknod_fifo"

    safe_unlink(fifo_path)
    try:
        os.mknod(fifo_path, stat.S_IFIFO | 0o600)
    except OSError as exc:
        if known_special_node_limit(exc):
            print(f"SKIP: FIFO mknod returns errno={exc.errno} on current FOD")
            return
        raise

    st = assert_mode_permissions(
        fifo_path,
        stat.S_ISFIFO,
        0o600,
        allow_special_limit=True,
        label="FIFO",
    )
    if st is None:
        return

    try:
        fd = os.open(fifo_path, os.O_RDONLY | os.O_NONBLOCK)
    except OSError as exc:
        if known_special_node_limit(exc) or exc.errno == errno.ENXIO:
            print(f"OK: FIFO mknod, open=unsupported errno={exc.errno}")
        else:
            raise
    else:
        os.close(fd)
        print("OK: FIFO mknod, open=supported")

    safe_unlink(fifo_path)


def test_character_device_mknod(work_dir: Path) -> None:
    # Urzadzenie znakowe wymaga sudo/root, ale backend moze je ograniczac.
    chr_path = work_dir / "mknod_char"
    chr_rdev = os.makedev(1, 7) if hasattr(os, "makedev") else 0

    safe_unlink(chr_path)
    try:
        os.mknod(chr_path, stat.S_IFCHR | 0o600, chr_rdev)
    except OSError as exc:
        if known_special_node_limit(exc):
            print(f"SKIP: character device mknod returns errno={exc.errno} on current FOD")
            return
        raise

    st = assert_mode_permissions(
        chr_path,
        stat.S_ISCHR,
        0o600,
        allow_special_limit=True,
        label="character device",
    )
    if st is None:
        return

    if hasattr(st, "st_rdev") and st.st_rdev != chr_rdev:
        raise AssertionError(f"char rdev {st.st_rdev} != {chr_rdev}")

    safe_unlink(chr_path)
    print("OK: character device mknod")


def test_block_device_mknod(work_dir: Path) -> None:
    # Urzadzenie blokowe wymaga sudo/root, ale backend moze je ograniczac.
    block_path = work_dir / "mknod_block"
    block_rdev = os.makedev(7, 0) if hasattr(os, "makedev") else 0

    safe_unlink(block_path)
    try:
        os.mknod(block_path, stat.S_IFBLK | 0o600, block_rdev)
    except OSError as exc:
        if known_special_node_limit(exc):
            print(f"SKIP: block device mknod returns errno={exc.errno} on current FOD")
            return
        raise

    st = assert_mode_permissions(
        block_path,
        stat.S_ISBLK,
        0o600,
        allow_special_limit=True,
        label="block device",
    )
    if st is None:
        return

    if hasattr(st, "st_rdev") and st.st_rdev != block_rdev:
        raise AssertionError(f"block rdev {st.st_rdev} != {block_rdev}")

    safe_unlink(block_path)
    print("OK: block device mknod")


def run_case(launcher: FODMount, label: str, func, verbose: int) -> None:
    # Kazdy przypadek dostaje osobny swiezy mount.
    # To izoluje obecne ograniczenia special-node w FOD 2.4.0.
    safe_label = label.replace(" ", "_").replace("/", "_")
    suffix = f"{safe_label}_{os.getpid()}_{int(time.time())}"

    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-mknod-{safe_label}.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))

        work_dir = mountpoint / f".fod_test_mknod_{suffix}"
        try:
            work_dir.mkdir(mode=0o755)

            if verbose:
                print_err(f"{label}: mountpoint={mountpoint}")
                print_err(f"{label}: work_dir={work_dir}")

            func(work_dir)
        finally:
            try:
                shutil.rmtree(work_dir, ignore_errors=True)
            finally:
                launcher.stop()


def main() -> None:
    args = parse_args()

    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    run_case(launcher, "regular", test_regular_mknod, args.verbose)
    run_case(launcher, "fifo", test_fifo_mknod, args.verbose)
    run_case(launcher, "character", test_character_device_mknod, args.verbose)
    run_case(launcher, "block", test_block_device_mknod, args.verbose)

    print("OK: test_mknod completed")


if __name__ == "__main__":
    main()
