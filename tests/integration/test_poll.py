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
        temp_root = Path(tmpdir)
        mountpoint = temp_root / "mount"
        writer_mountpoint = temp_root / "writer"
        replica_mountpoint = temp_root / "replica"
        launcher.start(str(mountpoint))
        writer = FODMount(str(ROOT))
        writer.start(str(writer_mountpoint))
        replica = FODMount(str(ROOT), role="replica")
        replica.start(str(replica_mountpoint))
        try:
            dir_path = mountpoint / f"poll_{suffix}"
            file_path = dir_path / "payload.txt"
            payload = b"poll payload"

            dir_path.mkdir()
            file_path.write_bytes(payload)

            empty_path = dir_path / "empty.txt"
            empty_path.touch()
            empty_fd = os.open(empty_path, os.O_RDONLY)
            try:
                poller = select.poll()
                poller.register(empty_fd, select.POLLIN)
                empty_events = dict(poller.poll(0))
                empty_mask = empty_events.get(empty_fd, 0)
                assert empty_mask & select.POLLIN, empty_mask
            finally:
                os.close(empty_fd)

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

            # poll() on an open descriptor must remain attached to its inode
            # after another mount removes the pathname.
            detached_path = dir_path / "open-unlinked.txt"
            detached_payload = b"open unlinked poll payload"
            detached_path.write_bytes(detached_payload)
            detached_fd = os.open(detached_path, os.O_RDONLY)
            writer_detached_path = writer_mountpoint / f"poll_{suffix}" / "open-unlinked.txt"
            try:
                writer_detached_path.unlink()
                poller = select.poll()
                poller.register(detached_fd, select.POLLIN)
                detached_events = dict(poller.poll(0))
                detached_mask = detached_events.get(detached_fd, 0)
                assert detached_mask & select.POLLIN, detached_mask
                assert os.pread(detached_fd, len(detached_payload), 0) == detached_payload
            finally:
                os.close(detached_fd)

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
            writer.stop()
            launcher.stop()


if __name__ == "__main__":
    main()
