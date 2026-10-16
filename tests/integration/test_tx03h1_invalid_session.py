#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX03H.1: invalid PostgreSQL client session must yield EIO, not ENOENT."""

from __future__ import annotations

import errno
import os
import re
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

from fod_mount import FODMount


def sql_query(launcher: FODMount, statement: str) -> str:
    env = launcher._runtime_env()
    env["PGPASSWORD"] = launcher.postgres_password
    cmd = ["psql", "-X", "-A", "-t", "-v", "ON_ERROR_STOP=1",
           "-d", launcher.postgres_db, "-U", launcher.postgres_user]
    host = env.get("FOD_PG_HOST") or env.get("POSTGRES_HOST")
    port = env.get("FOD_PG_PORT") or env.get("POSTGRES_PORT")
    if host:
        cmd.extend(["-h", host])
    if port:
        cmd.extend(["-p", port])
    cmd.extend(["-c", statement])
    completed = subprocess.run(
        cmd, env=env, check=True, capture_output=True, text=True, timeout=15,
    )
    return completed.stdout.strip()


def active_session_id(launcher: FODMount) -> int:
    assert launcher.config is not None
    match = None
    for _ in range(60):
        logs = launcher.config.log_file.read_text(encoding="utf-8")
        match = re.search(r"FOD client session heartbeat: session_id=(\d+)", logs)
        if match:
            break
        time.sleep(0.1)
    if match is None:
        raise AssertionError("TX03H1 no FOD client session in mount log")
    return int(match.group(1))


def main() -> None:
    database = os.environ.get("FOD_PG_DBNAME") or os.environ.get("POSTGRES_DB")
    if not database or not database.endswith("_test"):
        raise RuntimeError("TX03H1 requires explicit dedicated *_test database")

    root = Path(__file__).resolve().parents[2]
    payload = bytes(range(256)) * 128
    name = f"tx03h1-{uuid.uuid4().hex}.bin"

    with tempfile.TemporaryDirectory(prefix="fod-tx03h1-") as tmp:
        mountpoint = Path(tmp) / "mount-a"
        first = FODMount(str(root))
        try:
            first.start(str(mountpoint), log_prefix="/tmp/fod-tx03h1-a")
            path = mountpoint / name
            path.write_bytes(payload)
            session_id = active_session_id(first)

            # Mutate only this test mount's active client session. No schema
            # changes, no disabling fencing, no manipulation of other mounts.
            # Expire repeatedly to beat the mount's periodic heartbeat.
            saw_eio = False
            errors: list[int] = []
            for _ in range(60):
                updated = sql_query(
                    first,
                    "UPDATE fod.client_sessions "
                    "SET lease_expires_at = clock_timestamp() - INTERVAL '1 hour' "
                    f"WHERE session_id = {session_id} RETURNING session_id",
                )
                if str(session_id) not in updated.splitlines():
                    raise AssertionError(f"TX03H1 session vanished: {updated!r}")
                try:
                    fd = os.open(path, os.O_RDONLY)
                except OSError as exc:
                    errors.append(exc.errno)
                    if exc.errno == errno.EIO:
                        saw_eio = True
                        break
                    if exc.errno == errno.ENOENT:
                        raise AssertionError(
                            "TX03H1 invalid session incorrectly mapped to ENOENT"
                        ) from exc
                    raise
                else:
                    os.close(fd)
            if not saw_eio:
                raise AssertionError(
                    f"TX03H1 invalid session did not yield EIO; errors={errors}"
                )
            print(f"TX03H1_INVALID_SESSION=PASS errno=EIO session_id={session_id}", flush=True)
        except Exception:
            first._dump_log()
            raise
        finally:
            first.stop()

        # Reconnect through a fresh FUSE mount and a new PostgreSQL session.
        second = FODMount(str(root))
        try:
            second.start(str(mountpoint), log_prefix="/tmp/fod-tx03h1-b")
            path = mountpoint / name
            if path.read_bytes() != payload:
                raise AssertionError("TX03H1 file content changed after invalid session")
            if active_session_id(second) == session_id:
                raise AssertionError("TX03H1 expected a new client session")
            path.unlink()
            print("TX03H1_RECOVER_NEW_SESSION=PASS data_preserved=1", flush=True)
        except Exception:
            second._dump_log()
            raise
        finally:
            second.stop()


if __name__ == "__main__":
    main()
