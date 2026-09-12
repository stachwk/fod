#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import subprocess
import tempfile
import time
from pathlib import Path

from fod_mount import FODMount

BLOCK_SIZE = 4096
HOLD_SECONDS = 36.0
FAIL_FAST_LIMIT_SECONDS = 1.0
FILE_NAME = "first-writer-heartbeat.bin"


def psql_scalar(sql: str) -> str:
    env = os.environ.copy()
    host = env.get("POSTGRES_HOST", env.get("FOD_PG_HOST", "127.0.0.1"))
    port = env.get("POSTGRES_PORT", env.get("FOD_PG_PORT", "5432"))
    dbname = env.get("POSTGRES_DB", env.get("FOD_PG_DBNAME", "foddbname_test"))
    user = env.get("POSTGRES_USER", env.get("FOD_PG_USER", "foduser"))
    password = env.get("POSTGRES_PASSWORD", env.get("FOD_PG_PASSWORD", "cichosza"))
    env["PGPASSWORD"] = password

    result = subprocess.run(
        [
            "psql",
            "-h",
            host,
            "-p",
            port,
            "-U",
            user,
            "-d",
            dbname,
            "-At",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            sql,
        ],
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def destination_lease_snapshot() -> tuple[int, str, str, str]:
    raw = psql_scalar(
        f"""
        SELECT
            fencing_token,
            heartbeat_at::text,
            lease_expires_at::text,
            clock_timestamp()::text
        FROM fod.destination_write_leases
        WHERE parent_key = 0
          AND name = '{FILE_NAME}'
        """
    )
    if not raw:
        raise AssertionError("destination write lease missing")
    parts = raw.split("|")
    if len(parts) != 4:
        raise AssertionError(f"unexpected lease snapshot: {raw!r}")
    return int(parts[0]), parts[1], parts[2], parts[3]


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    with tempfile.TemporaryDirectory(prefix="fod-fww-heartbeat-") as temp_dir:
        temp = Path(temp_dir)
        mount_a_dir = temp / "mount-a"
        mount_b_dir = temp / "mount-b"

        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        launcher_a.init_schema()

        fd_a: int | None = None
        fd_b: int | None = None

        try:
            launcher_a.start(str(mount_a_dir), log_prefix="/tmp/fod-fww-heartbeat-a")
            launcher_b.start(str(mount_b_dir), log_prefix="/tmp/fod-fww-heartbeat-b")

            path_a = mount_a_dir / FILE_NAME
            path_b = mount_b_dir / FILE_NAME

            path_a.write_bytes(b"S" * (2 * BLOCK_SIZE))

            fd_a = os.open(path_a, os.O_WRONLY | os.O_TRUNC)
            os.write(fd_a, b"A" * BLOCK_SIZE)
            os.fsync(fd_a)

            token_initial, heartbeat_initial, expires_initial, server_initial = (
                destination_lease_snapshot()
            )

            time.sleep(HOLD_SECONDS)

            token_after, heartbeat_after, expires_after, server_after = (
                destination_lease_snapshot()
            )

            if token_after != token_initial:
                raise AssertionError(
                    f"heartbeat changed destination fencing token: "
                    f"{token_initial}->{token_after}"
                )
            if heartbeat_after == heartbeat_initial:
                raise AssertionError(
                    "write ownership heartbeat_at did not advance while fd stayed open"
                )

            loser_started = time.monotonic()
            loser_errno: int | None = None
            try:
                probe_fd = os.open(path_b, os.O_WRONLY | os.O_TRUNC)
            except OSError as exc:
                loser_errno = exc.errno
            else:
                os.close(probe_fd)
            loser_elapsed = time.monotonic() - loser_started

            if loser_errno != errno.EBUSY:
                raise AssertionError(
                    f"second writer after {HOLD_SECONDS:.1f}s returned "
                    f"errno={loser_errno}, expected EBUSY"
                )
            if loser_elapsed >= FAIL_FAST_LIMIT_SECONDS:
                raise AssertionError(
                    f"second writer did not fail fast after heartbeat hold: "
                    f"elapsed={loser_elapsed:.6f}s"
                )

            os.write(fd_a, b"A" * BLOCK_SIZE)
            os.fsync(fd_a)
            os.close(fd_a)
            fd_a = None

            fd_b = os.open(path_b, os.O_WRONLY | os.O_TRUNC)
            token_takeover, _, _, _ = destination_lease_snapshot()
            if token_takeover <= token_initial:
                raise AssertionError(
                    f"takeover fencing token did not advance: "
                    f"{token_initial}->{token_takeover}"
                )
            os.write(fd_b, b"B" * BLOCK_SIZE)
            os.fsync(fd_b)
            os.close(fd_b)
            fd_b = None

            # Weryfikuj takeover przez mount B, ktory wykonal zapis.
            # Mount A moze nadal miec lokalny read cache i cross-mount cache
            # invalidation nie jest przedmiotem tego testu ownership heartbeat.
            final = path_b.read_bytes()
            if final != b"B" * BLOCK_SIZE:
                raise AssertionError(
                    f"unexpected final payload after takeover on writer mount: "
                    f"len={len(final)} prefix={final[:16]!r}"
                )

            print(
                "OK write-ownership-heartbeat "
                f"hold_seconds={HOLD_SECONDS:.1f} "
                f"loser_errno={loser_errno} "
                f"loser_elapsed_ms={loser_elapsed * 1000.0:.3f} "
                f"heartbeat_initial={heartbeat_initial} "
                f"heartbeat_after={heartbeat_after} "
                f"initial_token={token_initial} "
                f"takeover_token={token_takeover} "
                f"initial_expires={expires_initial} "
                f"initial_server_time={server_initial} "
                f"after_expires={expires_after} "
                f"after_server_time={server_after}"
            )
        except Exception:
            launcher_a._dump_log()
            launcher_b._dump_log()
            raise
        finally:
            if fd_a is not None:
                try:
                    os.close(fd_a)
                except OSError:
                    pass
            if fd_b is not None:
                try:
                    os.close(fd_b)
                except OSError:
                    pass
            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
