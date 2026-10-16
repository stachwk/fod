#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX03I.3: verify write-ownership fencing under client-session turnover.

The stale token is tested via PostgreSQL's exact token/session/owner
predicate. This does not claim to exercise a stale FUSE process write.
"""

from __future__ import annotations

import os
import tempfile
import time
import uuid
from pathlib import Path

from fod_mount import FODMount
from test_unlink_open_writer_crash_convergence import postgres_connection, scalar


def ownership(db, file_id: int):
    with db.cursor() as cur:
        cur.execute(
            "SELECT session_id, owner_key, fencing_token "
            "FROM fod.file_write_leases WHERE file_id = %s", (file_id,),
        )
        return cur.fetchone()


def wait_ownership(db, file_id: int, timeout: float = 10.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        row = ownership(db, file_id)
        if row:
            return tuple(int(value) for value in row)
        time.sleep(0.05)
    raise AssertionError("TX03I3 write ownership row not observed")


def main() -> None:
    name = os.environ.get("FOD_PG_DBNAME") or os.environ.get("POSTGRES_DB")
    if not name or not name.endswith("_test"):
        raise RuntimeError("TX03I3 requires an explicit dedicated *_test database")
    root = Path(__file__).resolve().parents[2]
    filename = f"tx03i3-{uuid.uuid4().hex}.bin"
    payload_a = b"A" * 65536
    payload_b = b"B" * 65536
    with tempfile.TemporaryDirectory(prefix="fod-tx03i3-") as tmp:
        a, b = FODMount(str(root)), FODMount(str(root))
        db = postgres_connection(a)
        first_fd = None
        verifier = None
        try:
            a.start(str(Path(tmp) / "mount-a"), log_prefix="/tmp/fod-tx03i3-a")
            b.start(str(Path(tmp) / "mount-b"), log_prefix="/tmp/fod-tx03i3-b")
            path_a = Path(tmp) / "mount-a" / filename
            path_b = Path(tmp) / "mount-b" / filename
            first_fd = os.open(path_a, os.O_CREAT | os.O_RDWR, 0o600)
            os.write(first_fd, payload_a)
            os.fsync(first_fd)
            file_id = scalar(
                db, "SELECT id_file FROM fod.files WHERE id_directory IS NULL "
                "AND name = %s AND NOT unlinked", (filename,),
            )
            if file_id is None:
                raise AssertionError("TX03I3 missing file record")
            old_session, old_owner, old_token = wait_ownership(db, int(file_id))
            if path_b.read_bytes() != payload_a:
                raise AssertionError("TX03I3 initial cross-mount read mismatch")
            # Deliberately end A's hold before B claims the file.
            os.close(first_fd)
            first_fd = None
            with open(path_b, "r+b", buffering=0) as writer_b:
                writer_b.seek(0)
                writer_b.write(payload_b)
                os.fsync(writer_b.fileno())
                new_owner = wait_ownership(db, int(file_id))
                new_session, new_key, new_token = new_owner
                if new_session == old_session:
                    raise AssertionError("TX03I3 expected independent FUSE session")
                if new_token == old_token:
                    raise AssertionError("TX03I3 fencing token reused by new owner")

                # Simulate a delayed heartbeat/release from the old holder.
                # SQL fencing requires all three fields to match.
                with db.cursor() as cur:
                    cur.execute(
                        "UPDATE fod.file_write_leases "
                        "SET lease_expires_at = clock_timestamp() + INTERVAL '30 seconds' "
                        "WHERE file_id = %s AND session_id = %s "
                        "AND owner_key = %s AND fencing_token = %s",
                        (file_id, old_session, old_owner, old_token),
                    )
                    if cur.rowcount != 0:
                        raise AssertionError("TX03I3 stale token renewed new ownership")
                    cur.execute(
                        "DELETE FROM fod.file_write_leases "
                        "WHERE file_id = %s AND session_id = %s "
                        "AND owner_key = %s AND fencing_token = %s",
                        (file_id, old_session, old_owner, old_token),
                    )
                    if cur.rowcount != 0:
                        raise AssertionError("TX03I3 stale token released new ownership")
                if ownership(db, int(file_id)) != new_owner:
                    raise AssertionError("TX03I3 ownership changed after stale operations")
            if path_b.read_bytes() != payload_b:
                raise AssertionError("TX03I3 new owner's persisted data mismatch on mount B")

            # Mount A read the old payload earlier; it can retain a FUSE/kernel
            # cache entry. Observe this separately from fencing correctness.
            stale_read_on_a = path_a.read_bytes() != payload_b

            # Fresh mount C cannot reuse mount A's cached file contents.
            verifier = FODMount(str(root))
            mount_c = Path(tmp) / "mount-c"
            verifier.start(str(mount_c), log_prefix="/tmp/fod-tx03i3-c")
            if (mount_c / filename).read_bytes() != payload_b:
                raise AssertionError("TX03I3 fresh mount C cannot read committed payload")
            verifier.stop()
            verifier = None
            path_b.unlink()
            print(
                f"TX03I3_FENCING=PASS file_id={file_id} "
                f"old_session={old_session} new_session={new_session} "
                f"old_token={old_token} new_token={new_token} "
                "stale_renew_rejected=1 stale_release_rejected=1 "
                f"replacement_payload_intact=1 stale_read_on_a={int(stale_read_on_a)}",
                flush=True,
            )
        except Exception:
            a._dump_log()
            b._dump_log()
            raise
        finally:
            if verifier is not None:
                verifier.stop()
            if first_fd is not None:
                os.close(first_fd)
            try:
                b.stop()
            finally:
                a.stop()
                db.close()


if __name__ == "__main__":
    main()
