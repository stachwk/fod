#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX03I.1: kill a FUSE writer under write pressure; verify durable prefix.

This is a crash-during-write-stream test, not deterministic injection inside
PostgreSQL COMMIT or inside a particular FUSE flush/fsync syscall.
"""

from __future__ import annotations

import errno
import os
import signal
import tempfile
import threading
import time
import uuid
from pathlib import Path

from fod_mount import FODMount
from test_unlink_open_writer_crash_convergence import postgres_connection, scalar


def main() -> None:
    database_name = os.environ.get("FOD_PG_DBNAME") or os.environ.get("POSTGRES_DB")
    if not database_name or not database_name.endswith("_test"):
        raise RuntimeError("TX03I1 requires a dedicated *_test database")

    root = Path(__file__).resolve().parents[2]
    with tempfile.TemporaryDirectory(prefix="fod-tx03i1-") as tmp:
        mount_a, mount_b = Path(tmp) / "mount-a", Path(tmp) / "mount-b"
        a, b = FODMount(str(root)), FODMount(str(root))
        db = postgres_connection(a)
        old_fd: int | None = None
        errors: list[str] = []
        writes = [0]
        start = threading.Event()
        done = threading.Event()
        name = f"tx03i1-{uuid.uuid4().hex}.bin"
        baseline = bytes(range(256)) * 512
        chunk = b"T" * (256 * 1024)
        writer_thread: threading.Thread | None = None

        try:
            a.start(str(mount_a), log_prefix="/tmp/fod-tx03i1-a")
            b.start(str(mount_b), log_prefix="/tmp/fod-tx03i1-b")
            path_a, path_b = mount_a / name, mount_b / name
            path_a.write_bytes(baseline)
            old_fd = os.open(path_a, os.O_WRONLY | os.O_APPEND)
            os.fsync(old_fd)  # Explicit durability boundary for the baseline.

            file_id = scalar(
                db, "SELECT id_file FROM fod.files WHERE id_directory IS NULL "
                "AND name = %s AND NOT unlinked", (name,),
            )
            if file_id is None:
                raise AssertionError("TX03I1 file missing from PostgreSQL")
            old_session = scalar(
                db, "SELECT session_id FROM fod.file_open_leases "
                "WHERE file_id = %s ORDER BY session_id LIMIT 1", (file_id,),
            )
            if old_session is None:
                raise AssertionError("TX03I1 open lease missing from PostgreSQL")
            fuse_pid = scalar(
                db, "SELECT pid FROM fod.client_sessions WHERE session_id = %s",
                (old_session,),
            )
            if not fuse_pid:
                raise AssertionError("TX03I1 FUSE PID unavailable")

            def writer() -> None:
                assert old_fd is not None
                start.set()
                try:
                    for _ in range(4000):
                        count = os.write(old_fd, chunk)
                        if count != len(chunk):
                            raise AssertionError(f"short write: {count}")
                        writes[0] += 1
                except OSError as exc:
                    if exc.errno not in (errno.ENOTCONN, errno.EIO, errno.EBADF):
                        errors.append(repr(exc))
                except Exception as exc:
                    errors.append(repr(exc))
                finally:
                    done.set()

            writer_thread = threading.Thread(target=writer, name="tx03i1-writer")
            writer_thread.start()
            if not start.wait(5):
                raise AssertionError("TX03I1 writer did not start")
            # Wait until at least one write completed; crash while stream is active.
            deadline = time.monotonic() + 15
            while writes[0] == 0 and not done.is_set() and time.monotonic() < deadline:
                time.sleep(0.001)
            if writes[0] == 0 or done.is_set():
                raise AssertionError("TX03I1 could not establish write pressure")

            os.kill(int(fuse_pid), signal.SIGKILL)
            writer_thread.join(timeout=15)
            if writer_thread.is_alive():
                raise AssertionError("TX03I1 writer remained blocked after SIGKILL")
            if errors:
                raise AssertionError(f"TX03I1 unexpected write errors: {errors}")
            print(
                f"TX03I1_KILL=PASS fuse_pid={fuse_pid} session_id={old_session} "
                f"writes_completed_before_observation={writes[0]}",
                flush=True,
            )

            # Crash victim can no longer renew its lease. Only this session
            # is expired; survivor B performs normal maintenance.
            with db.cursor() as cur:
                cur.execute(
                    "UPDATE fod.client_sessions SET lease_expires_at = "
                    "clock_timestamp() - INTERVAL '1 second' "
                    "WHERE session_id = %s", (old_session,),
                )
                if cur.rowcount != 1:
                    raise AssertionError("TX03I1 old session not found for expiry")

            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                remaining = int(scalar(
                    db, "SELECT COUNT(*) FROM fod.file_open_leases "
                    "WHERE session_id = %s", (old_session,),
                ) or 0)
                if remaining == 0:
                    break
                time.sleep(0.1)
            else:
                raise AssertionError("TX03I1 expired open lease was not reclaimed")

            # The durable prefix is immutable. The tail is unacknowledged
            # at the crash boundary, so its final length is not prescribed.
            data = path_b.read_bytes()
            if not data.startswith(baseline):
                raise AssertionError("TX03I1 fsynced prefix corrupted after crash")
            if len(data) < len(baseline):
                raise AssertionError("TX03I1 file truncated below durable prefix")
            if any(value != ord("T") for value in data[len(baseline):]):
                raise AssertionError("TX03I1 recovered tail contains unexpected bytes")

            # Survivor must obtain fresh write ownership, not reuse the
            # killed writer's fenced generation.
            with open(path_b, "r+b", buffering=0) as fh:
                fh.seek(0)
                fh.write(b"RECOVERED")
                os.fsync(fh.fileno())
            if path_b.read_bytes()[:9] != b"RECOVERED":
                raise AssertionError("TX03I1 survivor write failed")
            path_b.unlink()
            print(
                f"TX03I1_RECOVERY=PASS durable_prefix=1 "
                f"tail_bytes={len(data) - len(baseline)} "
                "survivor_write=1 old_open_lease_reclaimed=1",
                flush=True,
            )
        except Exception:
            a._dump_log()
            b._dump_log()
            raise
        finally:
            if writer_thread is not None and writer_thread.is_alive():
                writer_thread.join(timeout=5)
            if old_fd is not None:
                try:
                    os.close(old_fd)
                except OSError:
                    pass
            try:
                b.stop()
            finally:
                try:
                    a.stop()
                finally:
                    db.close()


if __name__ == "__main__":
    main()
