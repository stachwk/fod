#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import os
import select
import sys
import tempfile
import uuid
import errno
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def close_fd_allow_replica_rofs(fd):
    # FOD/Rust FUSE moze zwrocic EROFS dopiero przy close/flush/release.
    # Dla fd repliki readonly to jest akceptowalne w tym tescie.
    try:
        os.close(fd)
    except OSError as exc:
        if exc.errno == errno.EROFS:
            return
        raise



def main() -> None:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    suffix = uuid.uuid4().hex[:8]
    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-poll-{suffix}.") as tmpdir:
        mountpoint = Path(tmpdir)
        replica_mountpoint = Path(tempfile.mkdtemp(prefix=f"/tmp/fod-poll-replica-{suffix}."))
        launcher.start(str(mountpoint))
        replica = FODMount(str(ROOT), role="replica")
        replica.start(str(replica_mountpoint))
        try:
            dir_path = mountpoint / f"poll_{suffix}"
            file_path = dir_path / "payload.txt"
            payload = b"poll payload"

            dir_path.mkdir()
            file_path.write_bytes(payload)

            fd = os.open(file_path, os.O_RDWR)
            try:
                poller = select.poll()
                poller.register(fd, select.POLLIN | select.POLLOUT)
                events = dict(poller.poll(0))
                mask = events.get(fd, 0)
                assert mask & select.POLLIN, mask
                assert mask & select.POLLOUT, mask
            finally:
                os.close(fd)

            replica_path = replica_mountpoint / f"poll_{suffix}" / "payload.txt"
            replica_fd = os.open(replica_path, os.O_RDONLY)
            try:
                poller = select.poll()
                poller.register(replica_fd, select.POLLIN | select.POLLOUT)
                events = dict(poller.poll(0))
                replica_mask = events.get(replica_fd, 0)
                assert replica_mask & select.POLLIN, replica_mask
                assert not (replica_mask & select.POLLOUT), replica_mask
            finally:
                close_fd_allow_replica_rofs(replica_fd)

            print("OK poll/mount")
        finally:
            replica.stop()
            launcher.stop()


if __name__ == "__main__":
    main()
