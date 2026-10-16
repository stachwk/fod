#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX03I.2: database lock-induced precommit crash and post-fsync recovery.

The precommit gate is a PostgreSQL table lock held by the test. It is NOT
fault injection into libpq COMMIT or into an internal fsync instruction.
"""

from __future__ import annotations

import argparse
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


def require_test_db() -> None:
    name = os.environ.get("FOD_PG_DBNAME") or os.environ.get("POSTGRES_DB")
    if not name or not name.endswith("_test"):
        raise RuntimeError("TX03I2 requires an explicit dedicated *_test database")


def wait_reclaimed(db, session: int, timeout: float = 25.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        remaining = int(scalar(
            db, "SELECT count(*) FROM fod.file_open_leases WHERE session_id = %s",
            (session,),
        ) or 0)
        if remaining == 0:
            return
        time.sleep(0.1)
    raise AssertionError("TX03I2 orphaned open lease not reclaimed")


def session_for_file(db, name: str) -> tuple[int, int]:
    file_id = scalar(
        db, "SELECT id_file FROM fod.files "
        "WHERE id_directory IS NULL AND name = %s AND NOT unlinked", (name,),
    )
    if file_id is None:
        raise AssertionError("TX03I2 test file missing")
    session = scalar(
        db, "SELECT session_id FROM fod.file_open_leases "
        "WHERE file_id = %s ORDER BY session_id LIMIT 1", (file_id,),
    )
    if session is None:
        raise AssertionError("TX03I2 test open lease missing")
    pid = scalar(
        db, "SELECT pid FROM fod.client_sessions WHERE session_id = %s", (session,),
    )
    if not pid:
        raise AssertionError("TX03I2 FUSE PID not found")
    return int(session), int(pid)


def run(mode: str) -> None:
    root = Path(__file__).resolve().parents[2]
    name = f"tx03i2-{uuid.uuid4().hex}.bin"
    baseline = b"B" * (128 * 1024)
    replacement = b"N" * (128 * 1024)
    with tempfile.TemporaryDirectory(prefix="fod-tx03i2-") as temp:
        a, b = FODMount(str(root)), FODMount(str(root))
        mount_a, mount_b = Path(temp) / "mount-a", Path(temp) / "mount-b"
        db = postgres_connection(a)
        lock_conn = postgres_connection(a)
        lock_conn.autocommit = False
        fd: int | None = None
        writer: threading.Thread | None = None
        writer_error: list[BaseException] = []
        started = threading.Event()
        finished = threading.Event()
        lock_held = False
        try:
            a.start(str(mount_a), log_prefix="/tmp/fod-tx03i2-a")
            b.start(str(mount_b), log_prefix="/tmp/fod-tx03i2-b")
            path_a, path_b = mount_a / name, mount_b / name
            path_a.write_bytes(baseline)
            fd = os.open(path_a, os.O_RDWR)
            os.fsync(fd)
            session, fuse_pid = session_for_file(db, name)

            if mode == "before-commit":
                # Block DML against the data block table while leaving reads
                # from the second mount available after releasing the lock.
                with lock_conn.cursor() as cur:
                    cur.execute("LOCK TABLE fod.data_blocks IN ACCESS EXCLUSIVE MODE")
                lock_held = True

                def blocked_write() -> None:
                    started.set()
                    try:
                        assert fd is not None
                        os.pwrite(fd, replacement, 0)
                        os.fsync(fd)
                    except OSError as exc:
                        if exc.errno not in (errno.EIO, errno.ENOTCONN, errno.EBADF):
                            writer_error.append(exc)
                    except BaseException as exc:
                        writer_error.append(exc)
                    finally:
                        finished.set()

                writer = threading.Thread(target=blocked_write, name="tx03i2-blocked-writer")
                writer.start()
                if not started.wait(5):
                    raise AssertionError("TX03I2 write worker did not start")
                # No assumption about timing: prove PostgreSQL is blocking
                # the test mount's persistence operation before SIGKILL.
                deadline = time.monotonic() + 15
                blocked = False
                while time.monotonic() < deadline:
                    blocked = bool(scalar(
                        db,
                        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity "
                        "WHERE datname = current_database() AND pid <> pg_backend_pid() "
                        "AND wait_event_type = 'Lock' "
                        "AND query ILIKE %s)",
                        ("%data_blocks%",),
                    ))
                    if blocked:
                        break
                    if finished.is_set():
                        break
                    time.sleep(0.05)
                if not blocked:
                    raise AssertionError(
                        "TX03I2 did not observe a blocked data_blocks persistence; "
                        "cannot claim precommit crash"
                    )
                os.kill(fuse_pid, signal.SIGKILL)
                # Release gate so survivor can progress.
                lock_conn.rollback()
                lock_held = False
                writer.join(timeout=15)
                if writer.is_alive():
                    raise AssertionError("TX03I2 crashed writer remained blocked")
                if writer_error:
                    raise AssertionError(f"TX03I2 unexpected writer errors: {writer_error}")
            else:
                os.pwrite(fd, replacement, 0)
                os.fsync(fd)
                if path_b.read_bytes() != replacement:
                    raise AssertionError("TX03I2 post-fsync replica mismatch")
                os.kill(fuse_pid, signal.SIGKILL)

            # Expire only the killed process's session and allow surviving
            # FUSE to prune it; do not disable or modify the fencing rules.
            with db.cursor() as cur:
                cur.execute(
                    "UPDATE fod.client_sessions SET lease_expires_at = "
                    "clock_timestamp() - INTERVAL '1 second' WHERE session_id = %s",
                    (session,),
                )
                if cur.rowcount != 1:
                    raise AssertionError("TX03I2 session not found")
            wait_reclaimed(db, session)

            # This verifies the baseline for a blocked precommit persistence,
            # and the fsync-confirmed payload for the postcommit case.
            expected = baseline if mode == "before-commit" else replacement
            actual = path_b.read_bytes()
            if actual != expected:
                raise AssertionError(
                    f"TX03I2 data mismatch mode={mode} "
                    f"expected={len(expected)} actual={len(actual)}"
                )
            with open(path_b, "r+b", buffering=0) as survivor:
                survivor.seek(0)
                survivor.write(b"SURVIVOR")
                os.fsync(survivor.fileno())
            if path_b.read_bytes()[:8] != b"SURVIVOR":
                raise AssertionError("TX03I2 survivor failed to reacquire write ownership")
            path_b.unlink()
            print(
                f"TX03I2=PASS mode={mode} old_session={session} "
                "old_lease_reclaimed=1 survivor_write=1 "
                f"data_consistent=1 sigkill_pid={fuse_pid}",
                flush=True,
            )
        except Exception:
            a._dump_log()
            b._dump_log()
            raise
        finally:
            if lock_held:
                lock_conn.rollback()
            if writer and writer.is_alive():
                writer.join(timeout=5)
            if fd is not None:
                try:
                    os.close(fd)
                except OSError:
                    pass
            try:
                b.stop()
            finally:
                try:
                    a.stop()
                finally:
                    lock_conn.close()
                    db.close()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--mode", choices=("before-commit", "after-fsync"),
        default="before-commit",
    )
    args = parser.parse_args()
    require_test_db()
    run(args.mode)


if __name__ == "__main__":
    main()
