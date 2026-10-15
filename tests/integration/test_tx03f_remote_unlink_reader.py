#!/usr/bin/env python
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX03F: remote unlink must not invalidate an already-open read-only descriptor."""

from __future__ import annotations

import errno
import os
import sys
import tempfile
import uuid
from pathlib import Path

from fod_mount import FODMount


def print_err(str):
    print(f"stderr: {str}", file=sys.stderr)



def assert_open_enoent(path: Path) -> None:
    # Sprawdz faktyczne otwarcie, nie tylko atrybuty z cache FUSE.
    try:
        handle = os.open(path, os.O_RDONLY)
    except OSError as err:
        if err.errno != errno.ENOENT:
            raise
    else:
        os.close(handle)
        raise AssertionError(f"TX03F open unexpectedly succeeded: {path}")


def main() -> None:
    # Wymagamy jawnej izolowanej bazy, zeby test nie dotykal produkcji.
    dbname = os.environ.get("FOD_PG_DBNAME") or os.environ.get("POSTGRES_DB")
    if not dbname or not dbname.endswith("_test"):
        raise RuntimeError("TX03F requires an explicit dedicated *_test database")

    root = Path(__file__).resolve().parents[2]
    with tempfile.TemporaryDirectory(prefix="fod-tx03f-read-unlink-") as tmp:
        mount_a = Path(tmp) / "mount-a"
        mount_b = Path(tmp) / "mount-b"
        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        name = f"tx03f-{uuid.uuid4().hex}.bin"
        payload = bytes(range(256)) * 512
        fd = None
        try:
            # Dwa osobne procesy FUSE korzystaja z jednej bazy PostgreSQL.
            launcher_a.start(str(mount_a), log_prefix="/tmp/fod-tx03f-a")
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-tx03f-b")
            path_a = mount_a / name
            path_b = mount_b / name
            path_a.write_bytes(payload)
            before = path_a.stat()
            fd = os.open(path_a, os.O_RDONLY)
            if os.pread(fd, len(payload), 0) != payload:
                raise AssertionError("TX03F pre-unlink read mismatch")

            # Drugi mount usuwa nazwe, mimo ze pierwszy ma otwarty uchwyt.
            path_b.unlink()
            print("TX03F_REMOTE_UNLINK=PASS", flush=True)

            # Ponowny open po usunieciu nazwy musi zwrocic ENOENT.
            for path in (path_a, path_b):
                assert_open_enoent(path)
            print("TX03F_NEW_OPEN_ENOENT=PASS", flush=True)

            # Stary deskryptor pozostaje poprawny po zdalnym unlink.
            old_stat = os.fstat(fd)
            if old_stat.st_ino != before.st_ino or old_stat.st_nlink != 0:
                raise AssertionError("TX03F old inode or link count changed")
            if os.pread(fd, len(payload), 0) != payload:
                raise AssertionError("TX03F post-unlink read mismatch")
            if os.pread(fd, 4096, 65536) != payload[65536:69632]:
                raise AssertionError("TX03F post-unlink partial read mismatch")
            print("TX03F_OPEN_READ_SURVIVES=PASS", flush=True)

            os.close(fd)
            fd = None
            # Kernel moze jeszcze udostepniac dodatni wpis atrybutow.
            # Bezpieczenstwo sprawdzamy przez open z obu montowan.
            assert_open_enoent(path_a)
            assert_open_enoent(path_b)
            print("TX03F_FINAL_CLOSE=PASS", flush=True)

            # Odwrotna kolejnosc: najpierw unlink, dopiero potem open/read.
            # Nowa nazwa nie jest wczesniej lookupowana na mount A.
            reverse_name = f"tx03f-reverse-{uuid.uuid4().hex}.bin"
            reverse_a = mount_a / reverse_name
            reverse_b = mount_b / reverse_name
            reverse_b.write_bytes(payload)
            reverse_b.unlink()
            assert_open_enoent(reverse_a)
            assert_open_enoent(reverse_b)
            print("TX03F_UNLINK_BEFORE_OPEN=PASS errno=ENOENT mounts=2", flush=True)

            # Sprawdz rowniez odwrotny kierunek usuniecia.
            reverse2_name = f"tx03f-reverse-a-{uuid.uuid4().hex}.bin"
            reverse2_a = mount_a / reverse2_name
            reverse2_b = mount_b / reverse2_name
            reverse2_a.write_bytes(payload)
            reverse2_a.unlink()
            assert_open_enoent(reverse2_b)
            assert_open_enoent(reverse2_a)
            print("TX03F_UNLINK_BEFORE_REMOTE_OPEN=PASS errno=ENOENT mounts=2", flush=True)
        except Exception:
            launcher_a._dump_log()
            launcher_b._dump_log()
            raise
        finally:
            if fd is not None:
                os.close(fd)
            try:
                launcher_b.stop()
            finally:
                launcher_a.stop()


if __name__ == "__main__":
    main()
