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
    # Wszystkie obserwacje stanu lease ida do tego samego PostgreSQL co mounty.
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


def active_lease_count(connection, file_id: int) -> int:
    return int(
        scalar(
            connection,
            """
            SELECT COUNT(*)
            FROM fod.file_open_leases l
            JOIN fod.client_sessions s ON s.session_id = l.session_id
            WHERE l.file_id = %s
              AND l.lease_expires_at > clock_timestamp()
              AND s.lease_expires_at > clock_timestamp()
            """,
            (file_id,),
        )
        or 0
    )


def wait_first_crash_pruned(
    connection,
    old_file_id: int,
    crashed_session_id: int,
    survivor_session_id: int,
    timeout_seconds: float,
) -> tuple[int, int, int, int]:
    deadline = time.monotonic() + timeout_seconds
    state = (1, 1, 1, 1)

    while time.monotonic() < deadline:
        crashed_session = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM fod.client_sessions WHERE session_id = %s",
                (crashed_session_id,),
            )
            or 0
        )
        crashed_lease = int(
            scalar(
                connection,
                """
                SELECT COUNT(*)
                FROM fod.file_open_leases
                WHERE file_id = %s AND session_id = %s
                """,
                (old_file_id, crashed_session_id),
            )
            or 0
        )
        survivor_lease = int(
            scalar(
                connection,
                """
                SELECT COUNT(*)
                FROM fod.file_open_leases l
                JOIN fod.client_sessions s ON s.session_id = l.session_id
                WHERE l.file_id = %s
                  AND l.session_id = %s
                  AND l.lease_expires_at > clock_timestamp()
                  AND s.lease_expires_at > clock_timestamp()
                """,
                (old_file_id, survivor_session_id),
            )
            or 0
        )
        old_file = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM fod.files WHERE id_file = %s AND unlinked",
                (old_file_id,),
            )
            or 0
        )
        state = (crashed_session, crashed_lease, survivor_lease, old_file)

        # PostgreSQL nie moze uznac starej generacji za osierocona,
        # dopoki drugi niezalezny holder ma aktywny lease.
        if survivor_lease != 1:
            raise AssertionError(
                "second holder lease disappeared before first crash convergence"
            )
        if old_file != 1:
            raise AssertionError(
                "old generation was purged while second holder lease was active"
            )

        if crashed_session == 0 and crashed_lease == 0:
            return state

        time.sleep(0.1)

    raise AssertionError(
        "first crashed session did not converge: "
        f"session={state[0]} lease={state[1]} "
        f"survivor_lease={state[2]} old_file={state[3]}"
    )


def wait_second_crash_purged(
    connection,
    old_file_id: int,
    second_session_id: int,
    timeout_seconds: float,
) -> tuple[int, int, int]:
    deadline = time.monotonic() + timeout_seconds
    state = (1, 1, 1)

    while time.monotonic() < deadline:
        second_session = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM fod.client_sessions WHERE session_id = %s",
                (second_session_id,),
            )
            or 0
        )
        second_lease = int(
            scalar(
                connection,
                """
                SELECT COUNT(*)
                FROM fod.file_open_leases
                WHERE file_id = %s AND session_id = %s
                """,
                (old_file_id, second_session_id),
            )
            or 0
        )
        old_file = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM fod.files WHERE id_file = %s",
                (old_file_id,),
            )
            or 0
        )
        state = (second_session, second_lease, old_file)

        if second_session == 0 and second_lease == 0 and old_file == 0:
            return state

        time.sleep(0.1)

    raise AssertionError(
        "second crashed holder did not release old generation: "
        f"session={state[0]} lease={state[1]} old_file={state[2]}"
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
        prefix="fod-unlink-staggered-crash-"
    ) as temp_dir:
        temp = Path(temp_dir)
        mount_a = temp / "mount-a"
        mount_b = temp / "mount-b"
        mount_c = temp / "mount-c"

        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        launcher_c = FODMount(str(root))
        launcher_a.init_schema()

        name = f"unlink-staggered-crash-{uuid.uuid4().hex}.bin"
        initial = b"A" * (128 * 1024)
        replacement = b"replacement survives staggered holder crashes\n"

        fd_a: int | None = None
        fd_b: int | None = None
        database = postgres_connection(launcher_a)

        try:
            launcher_a.start(
                str(mount_a),
                log_prefix="/tmp/fod-unlink-staggered-crash-a",
            )
            launcher_b.start(
                str(mount_b),
                log_prefix="/tmp/fod-unlink-staggered-crash-b",
            )
            launcher_c.start(
                str(mount_c),
                log_prefix="/tmp/fod-unlink-staggered-crash-c",
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
                database, old_file_id, mount_a
            )
            session_b, fuse_pid_b = lease_session_for_mount(
                database, old_file_id, mount_b
            )
            if session_a == session_b:
                raise AssertionError("holders A and B share a client session")
            if active_lease_count(database, old_file_id) != 2:
                raise AssertionError("expected exactly two active old-generation leases")

            target_c.unlink()
            if target_c.exists():
                raise AssertionError("remote pathname still visible after unlink")
            if scalar(
                database,
                "SELECT unlinked FROM fod.files WHERE id_file = %s",
                (old_file_id,),
            ) is not True:
                raise AssertionError("old generation was not staged as unlinked")

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

            if launcher_a.process is None or launcher_b.process is None:
                raise AssertionError("bootstrap process missing before staggered crash")
            bootstrap_pid_a = launcher_a.process.pid
            bootstrap_pid_b = launcher_b.process.pid

            # Etap 1: ginie A. B musi samodzielnie utrzymac stara generacje.
            os.kill(fuse_pid_a, signal.SIGKILL)
            try:
                bootstrap_rc_a = launcher_a.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                bootstrap_rc_a = None

            ttl_a_after_crash = remaining_session_ttl(database, session_a)
            if ttl_a_after_crash <= 0.0:
                raise AssertionError("session A already expired after crash")

            (
                session_a_left,
                lease_a_left,
                lease_b_after_a_prune,
                old_file_after_a_prune,
            ) = wait_first_crash_pruned(
                database,
                old_file_id,
                session_a,
                session_b,
                timeout_seconds=max(10.0, ttl_a_after_crash + 10.0),
            )

            if os.pread(fd_b, len(initial), 0) != initial:
                raise AssertionError("holder B lost old payload after A expiry")
            stat_b_after_a_prune = os.fstat(fd_b)
            if stat_b_after_a_prune.st_ino != old_ino:
                raise AssertionError("holder B inode changed after A expiry")
            if stat_b_after_a_prune.st_nlink != 0:
                raise AssertionError("holder B nlink changed after remote unlink")
            if target_c.read_bytes() != replacement:
                raise AssertionError("replacement changed after first crash convergence")

            crashed_fd_a_errno = close_crashed_fd(fd_a)
            fd_a = None
            launcher_a.stop()

            # Etap 2: dopiero teraz ginie ostatni holder B. C pozostaje zywe
            # i jego maintenance ma usunac sesje B oraz stara generacje.
            ttl_b_before_crash = remaining_session_ttl(database, session_b)
            if ttl_b_before_crash <= 0.0:
                raise AssertionError("survivor B lease expired before second crash")

            os.kill(fuse_pid_b, signal.SIGKILL)
            try:
                bootstrap_rc_b = launcher_b.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                bootstrap_rc_b = None

            ttl_b_after_crash = remaining_session_ttl(database, session_b)
            if ttl_b_after_crash <= 0.0:
                raise AssertionError("session B already expired immediately after crash")

            session_b_left, lease_b_left, old_file_final = wait_second_crash_purged(
                database,
                old_file_id,
                session_b,
                timeout_seconds=max(10.0, ttl_b_after_crash + 10.0),
            )

            if target_c.read_bytes() != replacement:
                raise AssertionError("replacement changed after final crash purge")
            if target_c.stat().st_ino != replacement_ino:
                raise AssertionError("replacement inode changed after final crash purge")

            visible_c = set(os.listdir(mount_c))
            leaked_internal = sorted(
                item for item in visible_c if item.startswith(".fod-unlinked-")
            )
            if leaked_internal:
                raise AssertionError(
                    f"deferred-unlink name leaked after staggered crashes: {leaked_internal}"
                )

            crashed_fd_b_errno = close_crashed_fd(fd_b)
            fd_b = None
            launcher_b.stop()
            target_c.unlink()

            print(
                "OK unlink-multiholder-staggered-crash "
                f"old_file_id={old_file_id} replacement_file_id={replacement_file_id} "
                f"old_ino={old_ino} replacement_ino={replacement_ino} "
                f"session_a={session_a} session_b={session_b} "
                f"bootstrap_pid_a={bootstrap_pid_a} fuse_pid_a={fuse_pid_a} "
                f"bootstrap_pid_b={bootstrap_pid_b} fuse_pid_b={fuse_pid_b} "
                f"bootstrap_rc_a={bootstrap_rc_a} bootstrap_rc_b={bootstrap_rc_b} "
                f"crashed_fd_a_errno={crashed_fd_a_errno} "
                f"crashed_fd_b_errno={crashed_fd_b_errno} "
                f"ttl_a_after_crash={ttl_a_after_crash:.3f} "
                f"ttl_b_before_crash={ttl_b_before_crash:.3f} "
                f"ttl_b_after_crash={ttl_b_after_crash:.3f} "
                "postgres_authority=1 independent_mounts=3 "
                "active_open_leases_before_first_crash=2 "
                f"session_a_left={session_a_left} lease_a_left={lease_a_left} "
                f"lease_b_after_a_prune={lease_b_after_a_prune} "
                f"old_file_after_a_prune={old_file_after_a_prune} "
                "first_crash_survivor_protected=1 "
                f"session_b_left={session_b_left} lease_b_left={lease_b_left} "
                f"old_file_final={old_file_final} "
                "second_crash_last_holder_purge=1 replacement_isolated=1 "
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
