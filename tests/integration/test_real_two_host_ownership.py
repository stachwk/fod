#!/usr/bin/env python
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

"""Test ownership i spojnosc danych FOD na dwoch rzeczywistych hostach.

Test uruchamia sie recznie na hostach A i B, z osobnymi montowaniami FOD.
Nie wykonuje mkfs, zmian PostgreSQL ani restartu daemonow.
"""

from __future__ import annotations

import argparse
import errno
import hashlib
import os
import re
import socket
import subprocess
import sys
import time
from pathlib import Path

VERBOSE = 0
A_SIZE = 64 * 1024
B_SIZE = 8 * 1024


def print_err(str):
    # Dodatkowa diagnostyka jest dostepna tylko z --verbose 1.
    if VERBOSE == 1:
        print(f"stderr: {str}", file=sys.stderr)


def expected_bytes(test_id: str, label: str, size: int) -> bytes:
    # Oba hosty odtwarzaja identyczne bajty bez przesylania plikow testowych.
    seed = hashlib.sha256(f"FOD-PR30-REAL-TWO-HOST:{test_id}:{label}".encode()).digest()
    return (seed * ((size + len(seed) - 1) // len(seed)))[:size]


def fingerprint(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def require_fod_mount(path: Path) -> Path:
    # Nie zapisuj niczego na zwyklym katalogu w przypadku martwego FUSE.
    mount = path.resolve(strict=True)
    if not mount.is_dir():
        raise RuntimeError(f"mountpoint is not a directory: {mount}")

    check = subprocess.run(
        ["mountpoint", "-q", str(mount)],
        check=False,
        capture_output=True,
        text=True,
    )
    if check.returncode != 0:
        raise RuntimeError(f"not an active mountpoint: {mount}")

    # Wymagaj FUSE, a nie zwyklego ext4 lub innego lokalnego dysku.
    info = subprocess.run(
        ["findmnt", "-n", "-T", str(mount), "-o", "FSTYPE"],
        check=True,
        capture_output=True,
        text=True,
    )
    fstype = info.stdout.strip()
    if "fuse" not in fstype.lower():
        raise RuntimeError(f"expected a FUSE mount, got {fstype!r}: {mount}")
    print_err(f"mount={mount} fstype={fstype} host={socket.gethostname()}")
    return mount


def write_fully(fd: int, data: bytes) -> None:
    # os.write moze wykonac zapis czesciowy; dopisz pozostale bajty.
    view = memoryview(data)
    while view:
        written = os.write(fd, view)
        if written <= 0:
            raise OSError("short write to FOD mount")
        view = view[written:]


def require_payload(path: Path, expected: bytes, phase: str) -> None:
    actual = path.read_bytes()
    if actual != expected:
        raise AssertionError(
            f"{phase}: payload mismatch path={path.name} "
            f"size={len(actual)} expected_size={len(expected)} "
            f"sha256={fingerprint(actual)} expected_sha256={fingerprint(expected)}"
        )
    print_err(f"{phase}: payload verified bytes={len(actual)} sha256={fingerprint(actual)}")


def phase_a_hold(path: Path, payload_a: bytes) -> None:
    # O_EXCL zabezpiecza przed przypadkowym nadpisaniem starego testu.
    fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o666)
    try:
        write_fully(fd, payload_a)
        os.fsync(fd)
        ino = os.fstat(fd).st_ino
        print(
            f"A_HOLD_READY host={socket.gethostname()} file={path.name} "
            f"ino={ino} size={len(payload_a)} sha256={fingerprint(payload_a)}",
            flush=True,
        )

        # Nie zwalniaj fd przed testem B_DENY. Wymaga interaktywnego stdin.
        if not sys.stdin.isatty():
            raise RuntimeError("a-hold needs an interactive terminal; do not use detached stdin")
        input("A trzyma ownership. Uruchom b-deny na hoscie B, potem nacisnij Enter tutaj: ")
    finally:
        os.close(fd)
    print(f"A_RELEASED host={socket.gethostname()} file={path.name}", flush=True)


def phase_b_deny(path: Path, payload_a: bytes) -> None:
    # B powinien widziec zatwierdzone dane A, ale nie powinien przejac zapisu.
    require_payload(path, payload_a, "B_DENY_READ")
    try:
        fd = os.open(path, os.O_WRONLY | os.O_APPEND)
    except OSError as error:
        if error.errno != errno.EBUSY:
            raise AssertionError(
                f"B_DENY expected EBUSY={errno.EBUSY}, got errno={error.errno}: {error}"
            ) from error
    else:
        os.close(fd)
        raise AssertionError("B_DENY acquired write ownership while A still holds the file")

    require_payload(path, payload_a, "B_DENY_AFTER")
    print(f"B_DENY_PASS host={socket.gethostname()} errno={errno.EBUSY}", flush=True)


def phase_b_takeover(path: Path, payload_a: bytes, payload_b: bytes, timeout: float) -> None:
    # Po zamknieciu przez A dozwolony jest krotki okres FUSE_RELEASE/EBUSY.
    require_payload(path, payload_a, "B_BEFORE_TAKEOVER")
    deadline = time.monotonic() + timeout
    busy_retries = 0
    while True:
        try:
            fd = os.open(path, os.O_WRONLY | os.O_APPEND)
            break
        except OSError as error:
            if error.errno != errno.EBUSY or time.monotonic() >= deadline:
                raise AssertionError(
                    f"B_TAKEOVER open failed after {busy_retries} EBUSY retries: {error}"
                ) from error
            busy_retries += 1
            time.sleep(0.05)

    try:
        write_fully(fd, payload_b)
        os.fsync(fd)
    finally:
        os.close(fd)

    expected = payload_a + payload_b
    require_payload(path, expected, "B_TAKEOVER_AFTER")
    print(
        f"B_TAKEOVER_PASS host={socket.gethostname()} "
        f"size={len(expected)} sha256={fingerprint(expected)} "
        f"busy_retries={busy_retries}",
        flush=True,
    )


def phase_a_verify(path: Path, expected: bytes, timeout: float) -> None:
    # Odczyt na A po przekazaniu ownership sprawdza konwergencje cache.
    deadline = time.monotonic() + timeout
    attempts = 0
    last = b""
    while True:
        attempts += 1
        last = path.read_bytes()
        if last == expected:
            print(
                f"A_VERIFY_PASS host={socket.gethostname()} "
                f"size={len(expected)} sha256={fingerprint(expected)} attempts={attempts}",
                flush=True,
            )
            return
        if time.monotonic() >= deadline:
            break
        time.sleep(0.1)

    raise AssertionError(
        f"A_VERIFY stale or incorrect payload after {attempts} reads: "
        f"size={len(last)} sha256={fingerprint(last)} "
        f"expected_size={len(expected)} expected_sha256={fingerprint(expected)}"
    )


def phase_cleanup(path: Path, expected: bytes) -> None:
    # Usuwaj tylko znany plik testowy, jesli caly payload sie zgadza.
    require_payload(path, expected, "CLEANUP_CHECK")
    path.unlink()
    print(f"CLEANUP_PASS host={socket.gethostname()} file={path.name}", flush=True)


def main() -> int:
    global VERBOSE

    parser = argparse.ArgumentParser(
        description="FOD PR #30: dwu-hostowy test ownership i spojnosci danych"
    )
    parser.add_argument(
        "phase",
        choices=("a-hold", "b-deny", "b-takeover", "a-verify", "cleanup"),
    )
    parser.add_argument("--mount", required=True, type=Path)
    parser.add_argument("--test-id", required=True)
    parser.add_argument("--timeout", type=float, default=5.0)
    parser.add_argument("--verbose", type=int, choices=(0, 1), default=0)
    args = parser.parse_args()
    VERBOSE = args.verbose

    # Jednoznaczny identyfikator gwarantuje brak separatorow i path traversal.
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_-]{5,47}", args.test_id):
        parser.error("--test-id must be 6..48 characters: ASCII letters, digits, _ or -")
    if not (0.1 <= args.timeout <= 30.0):
        parser.error("--timeout must be between 0.1 and 30 seconds")

    try:
        mount = require_fod_mount(args.mount)
        path = mount / f"fod-pr30-real-two-host-{args.test_id}.bin"
        payload_a = expected_bytes(args.test_id, "A", A_SIZE)
        payload_b = expected_bytes(args.test_id, "B", B_SIZE)

        # Kazda faza dziala na jednym hoscie; kolejnosc opisuje runbook.
        if args.phase == "a-hold":
            phase_a_hold(path, payload_a)
        elif args.phase == "b-deny":
            phase_b_deny(path, payload_a)
        elif args.phase == "b-takeover":
            phase_b_takeover(path, payload_a, payload_b, args.timeout)
        elif args.phase == "a-verify":
            phase_a_verify(path, payload_a + payload_b, args.timeout)
        elif args.phase == "cleanup":
            phase_cleanup(path, payload_a + payload_b)
    except (OSError, AssertionError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"REAL_TWO_HOST_FAIL phase={args.phase} error={error}", file=sys.stderr)
        print_err(repr(error))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
