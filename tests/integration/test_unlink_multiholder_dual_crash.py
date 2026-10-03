#!/usr/bin/env python
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import signal
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

import psycopg2

from fod_mount import FODMount


def postgres_connection(launcher: FODMount):
    # Wszystkie decyzje testowe obserwuja ten sam autorytatywny PostgreSQL.
    host = os.environ.get("FOD_PG_HOST") or os.environ.get("POSTGRES_HOST") or "127.0.0.1"
    port = int(os.environ.get("FOD_PG_PORT") or os.environ.get("POSTGRES_PORT") or "5432")
    connection = psycopg2.connect(
        host=host,
        port=port,
        dbname=launcher.postgres_db,
        user=launcher.postgres_user,
        password=launcher.postgres_password,
    )
    connection.autocommit = True
    return connection


def scalar(connection, sql: str, params: tuple[object, ...] = ()):
    with connection.cursor() as cursor:
        cursor.execute(sql, params)
        row = cursor.fetchone()
    return None if row is None else row[0]


def lease_session_for_mount(
    connection,
    file_id: int,
    mountpoint: Path,
) -> tuple[int, int]:
    with connection.cursor() as cursor:
        cursor.execute(
            """
            SELECT s.session_id, s.pid
            FROM fod.file_open_leases l
            JOIN fod.client_sessions s ON s.session_id = l.session_id
            WHERE l.file_id = %s
              AND s.mountpoint = %s
              AND l.lease_expires_at > clock_timestamp()
              AND s.lease_expires_at > clock_timestamp()
            ORDER BY s.session_id DESC
            LIMIT 1
            """,
            (file_id, str(mountpoint)),
        )
        row = cursor.fetchone()
    if row is None:
        raise AssertionError(f"active open lease missing for mount {mountpoint}")
    return int(row[0]), int(row[1])


def remaining_session_ttl(connection, session_id: int) -> float:
    return float(
        scalar(
            connection,
            """
            SELECT GREATEST(
                EXTRACT(EPOCH FROM (lease_expires_at - clock_timestamp())),
                0
            )::double precision
            FROM fod.client_sessions
            WHERE session_id = %s
            """,
            (session_id,),
        )
        or 0.0
    )


def holder_state(
    connection,
    old_file_id: int,
    session_a: int,
    session_b: int,
) -> tuple[int, int, int, int]:
    # Jeden SELECT daje jeden snapshot PostgreSQL. Dzieki temu test nie
    # pomyli legalnego przejscia lease->expiry->purge z niespojnym odczytem
    # wykonanym kilkoma osobnymi zapytaniami.
    with connection.cursor() as cursor:
        cursor.execute(
            """
            SELECT
                (
                    SELECT COUNT(*)
                    FROM fod.client_sessions
                    WHERE session_id IN (%s, %s)
                ) AS session_rows,
                (
                    SELECT COUNT(*)
                    FROM fod.file_open_leases
                    WHERE file_id = %s
                      AND session_id IN (%s, %s)
                ) AS lease_rows,
                (
                    SELECT COUNT(*)
                    FROM fod.file_open_leases l
                    JOIN fod.client_sessions s ON s.session_id = l.session_id
                    WHERE l.file_id = %s
                      AND l.session_id IN (%s, %s)
                      AND l.lease_expires_at > clock_timestamp()
                      AND s.lease_expires_at > clock_timestamp()
                ) AS active_leases,
                (
                    SELECT COUNT(*)
                    FROM fod.files
                    WHERE id_file = %s
                      AND unlinked
                ) AS old_file_rows
            """,
            (
                session_a,
                session_b,
                old_file_id,
                session_a,
                session_b,
                old_file_id,
                session_a,
                session_b,
                old_file_id,
            ),
        )
        row = cursor.fetchone()
    if row is None:
        raise AssertionError("PostgreSQL holder-state snapshot missing")
    return tuple(int(value) for value in row)


def wait_dual_crash_convergence(
    connection,
    old_file_id: int,
    session_a: int,
    session_b: int,
    timeout_seconds: float,
) -> tuple[tuple[int, int, int, int], bool, bool]:
    deadline = time.monotonic() + timeout_seconds
    last_state = (2, 2, 2, 1)
    observed_one_active_lease = False
    observed_zero_active_before_purge = False

    while time.monotonic() < deadline:
        last_state = holder_state(
            connection,
            old_file_id,
            session_a,
            session_b,
        )
        session_rows, lease_rows, active_leases, old_file_rows = last_state

        # Glowny invariant C6: reclaim jest zabroniony, gdy PostgreSQL widzi
        # choc jeden aktywny open lease starej generacji.
        if active_leases > 0 and old_file_rows != 1:
            raise AssertionError(
                "old generation disappeared while an authoritative open lease was active: "
                f"sessions={session_rows} leases={lease_rows} "
                f"active_leases={active_leases} old_file={old_file_rows}"
            )

        if active_leases == 1:
            observed_one_active_lease = True
        if active_leases == 0 and old_file_rows == 1:
            observed_zero_active_before_purge = True

        if session_rows == 0 and lease_rows == 0 and old_file_rows == 0:
            return (
                last_state,
                observed_one_active_lease,
                observed_zero_active_before_purge,
            )

        time.sleep(0.05)

    raise AssertionError(
        "dual crash did not converge: "
        f"sessions={last_state[0]} leases={last_state[1]} "
        f"active_leases={last_state[2]} old_file={last_state[3]}"
    )


def close_crashed_fd(fd: int) -> int:
    close_errno = 0
    try:
        os.close(fd)
    except OSError as err:
        if err.errno != errno.ENOTCONN:
            raise
        close_errno = err.errno
    return close_errno


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    with tempfile.TemporaryDirectory(
        prefix="fod-unlink-dual-crash-"
    ) as temp_dir:
        temp = Path(temp_dir)
        mount_a = temp / "mount-a"
        mount_b = temp / "mount-b"
        mount_c = temp / "mount-c"

        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        launcher_c = FODMount(str(root))
        launcher_a.init_schema()

        name = f"unlink-dual-crash-{uuid.uuid4().hex}.bin"
        initial = b"A" * (128 * 1024)
        replacement = b"replacement survives near-simultaneous holder crashes\n"

        fd_a: int | None = None
        fd_b: int | None = None
        database = postgres_connection(launcher_a)

        try:
            launcher_a.start(
                str(mount_a),
                log_prefix="/tmp/fod-unlink-dual-crash-a",
            )
            launcher_b.start(
                str(mount_b),
                log_prefix="/tmp/fod-unlink-dual-crash-b",
            )
            launcher_c.start(
                str(mount_c),
                log_prefix="/tmp/fod-unlink-dual-crash-c",
            )

            target_a = mount_a / name
            target_b = mount_b / name
            target_c = mount_c / name

            target_a.write_bytes(initial)
            old_ino = target_a.stat().st_ino
            old_file_id = scalar(
                database,
                """
                SELECT id_file
                FROM fod.files
                WHERE id_directory IS NULL
                  AND name = %s
                  AND NOT unlinked
                """,
                (name,),
            )
            if old_file_id is None:
                raise AssertionError("old file_id missing")
            old_file_id = int(old_file_id)

            fd_a = os.open(target_a, os.O_RDWR)
            fd_b = os.open(target_b, os.O_RDONLY)
            if os.pread(fd_a, len(initial), 0) != initial:
                raise AssertionError("holder A initial payload mismatch")
            if os.pread(fd_b, len(initial), 0) != initial:
                raise AssertionError("holder B initial payload mismatch")

            session_a, fuse_pid_a = lease_session_for_mount(
                database,
                old_file_id,
                mount_a,
            )
            session_b, fuse_pid_b = lease_session_for_mount(
                database,
                old_file_id,
                mount_b,
            )
            if session_a == session_b:
                raise AssertionError("holders share one client session")
            if fuse_pid_a == fuse_pid_b:
                raise AssertionError("holders share one FUSE process")

            initial_state = holder_state(
                database,
                old_file_id,
                session_a,
                session_b,
            )
            if initial_state != (2, 2, 2, 0):
                raise AssertionError(
                    "unexpected pre-unlink holder state: "
                    f"sessions={initial_state[0]} leases={initial_state[1]} "
                    f"active_leases={initial_state[2]} old_file={initial_state[3]}"
                )

            target_c.unlink()
            if target_c.exists():
                raise AssertionError("remote pathname still visible after unlink")
            if scalar(
                database,
                "SELECT unlinked FROM fod.files WHERE id_file = %s",
                (old_file_id,),
            ) is not True:
                raise AssertionError("old generation was not staged as unlinked")

            staged_state = holder_state(
                database,
                old_file_id,
                session_a,
                session_b,
            )
            if staged_state != (2, 2, 2, 1):
                raise AssertionError(
                    "unexpected post-unlink holder state: "
                    f"sessions={staged_state[0]} leases={staged_state[1]} "
                    f"active_leases={staged_state[2]} old_file={staged_state[3]}"
                )

            target_c.write_bytes(replacement)
            replacement_ino = target_c.stat().st_ino
            replacement_file_id = scalar(
                database,
                """
                SELECT id_file
                FROM fod.files
                WHERE id_directory IS NULL
                  AND name = %s
                  AND NOT unlinked
                """,
                (name,),
            )
            if replacement_file_id is None:
                raise AssertionError("replacement file_id missing")
            replacement_file_id = int(replacement_file_id)
            if replacement_file_id == old_file_id:
                raise AssertionError("replacement reused old PostgreSQL generation")
            if replacement_ino == old_ino:
                raise AssertionError("replacement reused old inode generation")

            if os.pread(fd_b, len(initial), 0) != initial:
                raise AssertionError("holder B payload changed before crash")
            if target_c.read_bytes() != replacement:
                raise AssertionError("replacement changed before crash")

            if launcher_a.process is None or launcher_b.process is None:
                raise AssertionError("bootstrap process missing before dual crash")
            bootstrap_pid_a = launcher_a.process.pid
            bootstrap_pid_b = launcher_b.process.pid

            # Oba SIGKILL ida jeden po drugim bez oczekiwania na bootstrap.
            # Okno mierzymy, aby test rzeczywiscie reprezentowal jeden epizod
            # awarii wzgledem 30-sekundowego lease TTL.
            crash_started = time.monotonic()
            os.kill(fuse_pid_a, signal.SIGKILL)
            os.kill(fuse_pid_b, signal.SIGKILL)
            dual_crash_window_ms = (time.monotonic() - crash_started) * 1000.0
            if dual_crash_window_ms > 2000.0:
                raise AssertionError(
                    f"dual crash window too wide: {dual_crash_window_ms:.3f} ms"
                )

            try:
                bootstrap_rc_a = launcher_a.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                bootstrap_rc_a = None
            try:
                bootstrap_rc_b = launcher_b.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                bootstrap_rc_b = None

            ttl_a_after_crash = remaining_session_ttl(database, session_a)
            ttl_b_after_crash = remaining_session_ttl(database, session_b)
            if ttl_a_after_crash <= 0.0 or ttl_b_after_crash <= 0.0:
                raise AssertionError(
                    "a holder session was already expired immediately after dual crash"
                )

            immediate_state = holder_state(
                database,
                old_file_id,
                session_a,
                session_b,
            )
            if immediate_state[2] != 2 or immediate_state[3] != 1:
                raise AssertionError(
                    "dual crash lost authoritative leases before immediate snapshot: "
                    f"sessions={immediate_state[0]} leases={immediate_state[1]} "
                    f"active_leases={immediate_state[2]} old_file={immediate_state[3]}"
                )

            max_ttl = max(ttl_a_after_crash, ttl_b_after_crash)
            (
                final_state,
                observed_one_active_lease,
                observed_zero_active_before_purge,
            ) = wait_dual_crash_convergence(
                database,
                old_file_id,
                session_a,
                session_b,
                timeout_seconds=max(10.0, max_ttl + 10.0),
            )

            if target_c.read_bytes() != replacement:
                raise AssertionError("replacement changed during dual crash convergence")
            if target_c.stat().st_ino != replacement_ino:
                raise AssertionError("replacement inode changed during dual crash convergence")

            visible_c = set(os.listdir(mount_c))
            leaked_internal = sorted(
                item for item in visible_c if item.startswith(".fod-unlinked-")
            )
            if leaked_internal:
                raise AssertionError(
                    f"deferred-unlink name leaked after dual crash: {leaked_internal}"
                )

            crashed_fd_a_errno = close_crashed_fd(fd_a)
            fd_a = None
            crashed_fd_b_errno = close_crashed_fd(fd_b)
            fd_b = None
            launcher_a.stop()
            launcher_b.stop()
            target_c.unlink()

            print(
                "OK unlink-multiholder-dual-crash "
                f"old_file_id={old_file_id} replacement_file_id={replacement_file_id} "
                f"old_ino={old_ino} replacement_ino={replacement_ino} "
                f"session_a={session_a} session_b={session_b} "
                f"bootstrap_pid_a={bootstrap_pid_a} fuse_pid_a={fuse_pid_a} "
                f"bootstrap_pid_b={bootstrap_pid_b} fuse_pid_b={fuse_pid_b} "
                f"bootstrap_rc_a={bootstrap_rc_a} bootstrap_rc_b={bootstrap_rc_b} "
                f"crashed_fd_a_errno={crashed_fd_a_errno} "
                f"crashed_fd_b_errno={crashed_fd_b_errno} "
                f"dual_crash_window_ms={dual_crash_window_ms:.3f} "
                f"ttl_a_after_crash={ttl_a_after_crash:.3f} "
                f"ttl_b_after_crash={ttl_b_after_crash:.3f} "
                "postgres_authority=1 independent_mounts=3 "
                f"pre_unlink_old_file={initial_state[3]} "
                f"staged_old_file={staged_state[3]} "
                "active_open_leases_before_crash=2 "
                f"immediate_active_leases={immediate_state[2]} "
                f"immediate_old_file={immediate_state[3]} "
                f"observed_one_active_lease={int(observed_one_active_lease)} "
                f"observed_zero_active_before_purge={int(observed_zero_active_before_purge)} "
                f"final_session_rows={final_state[0]} "
                f"final_lease_rows={final_state[1]} "
                f"final_active_leases={final_state[2]} "
                f"final_old_file={final_state[3]} "
                "last_active_lease_blocks_purge=1 "
                "dual_crash_final_purge=1 replacement_isolated=1 "
                "hidden_entry_leaks=0 cleanup=1"
            )
        except Exception:
            launcher_a._dump_log()
            launcher_b._dump_log()
            launcher_c._dump_log()
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
            try:
                launcher_c.stop()
            finally:
                try:
                    launcher_b.stop()
                finally:
                    launcher_a.stop()
                    database.close()


if __name__ == "__main__":
    main()
