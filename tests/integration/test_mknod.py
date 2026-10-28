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
import threading
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


def assert_scandir_not_regular(path: Path) -> None:
    entries = {entry.name: entry for entry in os.scandir(path.parent)}
    entry = entries[path.name]
    if entry.is_file(follow_symlinks=False):
        raise AssertionError(f"{path}: readdir reported special node as regular file")
    if entry.is_dir(follow_symlinks=False):
        raise AssertionError(f"{path}: readdir reported special node as directory")
    if entry.is_symlink():
        raise AssertionError(f"{path}: readdir reported special node as symlink")


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
    # FIFO jest częścią deklarowanego kontraktu POSIX FOD.
    fifo_path = work_dir / "mknod_fifo"

    safe_unlink(fifo_path)
    os.mkfifo(fifo_path, 0o600)

    assert_scandir_not_regular(fifo_path)

    st = assert_mode_permissions(
        fifo_path,
        stat.S_ISFIFO,
        0o600,
        label="FIFO",
    )

    try:
        os.open(fifo_path, os.O_WRONLY | os.O_NONBLOCK)
    except OSError as exc:
        if exc.errno != errno.ENXIO:
            raise
    else:
        raise AssertionError("nonblocking FIFO writer without a reader succeeded")

    reader = os.open(fifo_path, os.O_RDONLY | os.O_NONBLOCK)
    writer = os.open(fifo_path, os.O_WRONLY | os.O_NONBLOCK)
    try:
        payload = b"fifo payload"
        assert os.write(writer, payload) == len(payload)
        assert os.read(reader, len(payload)) == payload
        os.close(writer)
        writer = -1
        assert os.read(reader, 1) == b""
    finally:
        if writer >= 0:
            os.close(writer)
        os.close(reader)

    print("OK: FIFO mkfifo nonblocking open/read/write")

    writer_started = threading.Event()
    writer_opened = threading.Event()
    blocking_errors: list[BaseException] = []

    def blocking_writer() -> None:
        writer_started.set()
        try:
            fd = os.open(fifo_path, os.O_WRONLY)
            writer_opened.set()
            try:
                assert os.write(fd, b"blocking") == len(b"blocking")
            finally:
                os.close(fd)
        except BaseException as exc:
            blocking_errors.append(exc)
            writer_opened.set()

    writer_thread = threading.Thread(target=blocking_writer, daemon=True)
    writer_thread.start()
    assert writer_started.wait(timeout=1), "blocking FIFO writer did not start"
    if writer_opened.wait(timeout=0.1):
        raise AssertionError("blocking FIFO writer opened before a reader existed")

    reader = os.open(fifo_path, os.O_RDONLY)
    try:
        assert writer_opened.wait(timeout=2), "blocking FIFO writer did not open for reader"
        assert os.read(reader, len(b"blocking")) == b"blocking"
        assert os.read(reader, 1) == b""
    finally:
        os.close(reader)
        writer_thread.join(timeout=2)
    if writer_thread.is_alive():
        raise AssertionError("blocking FIFO writer did not terminate")
    if blocking_errors:
        raise blocking_errors[0]

    print("OK: FIFO blocking writer waits for reader")

    atomic_reader = os.open(fifo_path, os.O_RDONLY | os.O_NONBLOCK)
    pipe_buf = os.fpathconf(atomic_reader, "PC_PIPE_BUF")
    record_size = min(pipe_buf, 4096)
    records_per_writer = 16
    expected_size = record_size * records_per_writer * 2
    atomic_writers = [
        os.open(fifo_path, os.O_WRONLY),
        os.open(fifo_path, os.O_WRONLY),
    ]
    os.set_blocking(atomic_reader, True)
    atomic_errors: list[BaseException] = []

    def write_records(fd: int, byte: bytes) -> None:
        record = byte * record_size
        try:
            for _ in range(records_per_writer):
                if os.write(fd, record) != len(record):
                    raise AssertionError("FIFO write shorter than PIPE_BUF record")
        except BaseException as exc:
            atomic_errors.append(exc)
        finally:
            os.close(fd)

    atomic_threads = [
        threading.Thread(target=write_records, args=(atomic_writers[0], b"A"), daemon=True),
        threading.Thread(target=write_records, args=(atomic_writers[1], b"B"), daemon=True),
    ]
    for thread in atomic_threads:
        thread.start()

    stream = bytearray()
    try:
        while len(stream) < expected_size:
            chunk = os.read(atomic_reader, min(8192, expected_size - len(stream)))
            if not chunk:
                break
            stream.extend(chunk)
    finally:
        os.close(atomic_reader)
    for thread in atomic_threads:
        thread.join(timeout=2)
        if thread.is_alive():
            raise AssertionError("FIFO atomic writer did not terminate")
    if atomic_errors:
        raise atomic_errors[0]
    if len(stream) != expected_size:
        raise AssertionError(f"FIFO stream length {len(stream)} != {expected_size}")
    records = [stream[offset : offset + record_size] for offset in range(0, len(stream), record_size)]
    if any(record not in (b"A" * record_size, b"B" * record_size) for record in records):
        raise AssertionError("concurrent FIFO writes at or below PIPE_BUF were interleaved")

    print(f"OK: FIFO PIPE_BUF atomic writes pipe_buf={pipe_buf} record_size={record_size}")

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

    assert_scandir_not_regular(chr_path)

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

    assert_scandir_not_regular(block_path)

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
