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
    # Test korzysta z tego samego endpointu PostgreSQL co mounty FOD.
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
        raise AssertionError(
            f"active file open lease missing for mount {mountpoint}"
        )
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


def wait_crashed_session_pruned_while_survivor_protects(
    connection,
    old_file_id: int,
    crashed_session_id: int,
    survivor_session_id: int,
    timeout_seconds: float,
) -> tuple[int, int, int, int]:
    deadline = time.monotonic() + timeout_seconds
    last_crashed_session = 1
    last_crashed_lease = 1
    last_survivor_lease = 1
    last_old_file = 1

    while time.monotonic() < deadline:
        last_crashed_session = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM fod.client_sessions WHERE session_id = %s",
                (crashed_session_id,),
            )
            or 0
        )
        last_crashed_lease = int(
            scalar(
                connection,
                """
                SELECT COUNT(*)
                FROM fod.file_open_leases
                WHERE file_id = %s
                  AND session_id = %s
                """,
                (old_file_id, crashed_session_id),
            )
            or 0
        )
        last_survivor_lease = int(
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
        last_old_file = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM fod.files WHERE id_file = %s AND unlinked",
                (old_file_id,),
            )
            or 0
        )

        # To jest sedno C4: nawet chwilowe usuniecie starej generacji przy
        # aktywnym lease B jest bledem centralnej koordynacji.
        if last_survivor_lease != 1:
            raise AssertionError(
                "survivor open lease disappeared before crashed session prune"
            )
        if last_old_file != 1:
            raise AssertionError(
                "old generation was purged while survivor open lease was active"
            )

        if last_crashed_session == 0 and last_crashed_lease == 0:
            return (
                last_crashed_session,
                last_crashed_lease,
                last_survivor_lease,
                last_old_file,
            )

        time.sleep(0.1)

    raise AssertionError(
        "crashed session did not expire while survivor protected old generation: "
        f"crashed_session={last_crashed_session} "
        f"crashed_lease={last_crashed_lease} "
        f"survivor_lease={last_survivor_lease} old_file={last_old_file}"
    )


def wait_old_generation_removed(
    connection,
    old_file_id: int,
    survivor_session_id: int,
    timeout_seconds: float = 10.0,
) -> tuple[int, int]:
    deadline = time.monotonic() + timeout_seconds
    last_survivor_lease = 1
    last_old_file = 1

    while time.monotonic() < deadline:
        last_survivor_lease = int(
            scalar(
                connection,
                """
                SELECT COUNT(*)
                FROM fod.file_open_leases
                WHERE file_id = %s
                  AND session_id = %s
                """,
                (old_file_id, survivor_session_id),
            )
            or 0
        )
        last_old_file = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM fod.files WHERE id_file = %s",
                (old_file_id,),
            )
            or 0
        )
        if last_survivor_lease == 0 and last_old_file == 0:
            return last_survivor_lease, last_old_file
        time.sleep(0.1)

    raise AssertionError(
        "old generation survived final survivor close: "
        f"survivor_lease={last_survivor_lease} old_file={last_old_file}"
    )


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    with tempfile.TemporaryDirectory(
        prefix="fod-unlink-multiholder-crash-survivor-"
    ) as temp_dir:
        temp = Path(temp_dir)
        mount_a = temp / "mount-a"
        mount_b = temp / "mount-b"
        mount_c = temp / "mount-c"

        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        launcher_c = FODMount(str(root))
        launcher_a.init_schema()

        name = f"unlink-multiholder-crash-{uuid.uuid4().hex}.bin"
        initial = b"A" * (128 * 1024)
        replacement = b"replacement survives multi-holder crash\n"

        fd_a: int | None = None
        fd_b: int | None = None
        database = postgres_connection(launcher_a)

        try:
            launcher_a.start(
                str(mount_a),
                log_prefix="/tmp/fod-unlink-multiholder-crash-a",
            )
            launcher_b.start(
                str(mount_b),
                log_prefix="/tmp/fod-unlink-multiholder-crash-b",
            )
            launcher_c.start(
                str(mount_c),
                log_prefix="/tmp/fod-unlink-multiholder-crash-c",
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

            # A trzyma writer, B niezalezny reader tej samej generacji.
            fd_a = os.open(target_a, os.O_RDWR)
            fd_b = os.open(target_b, os.O_RDONLY)
            if os.pread(fd_a, len(initial), 0) != initial:
                raise AssertionError("mount A initial read mismatch")
            if os.pread(fd_b, len(initial), 0) != initial:
                raise AssertionError("mount B initial read mismatch")

            session_a, fuse_pid_a = lease_session_for_mount(
                database,
                old_file_id,
                mount_a,
            )
            session_b, _fuse_pid_b = lease_session_for_mount(
                database,
                old_file_id,
                mount_b,
            )
            if session_a == session_b:
                raise AssertionError("independent mounts share one client session")

            active_lease_count = int(
                scalar(
                    database,
                    """
                    SELECT COUNT(*)
                    FROM fod.file_open_leases l
                    JOIN fod.client_sessions s ON s.session_id = l.session_id
                    WHERE l.file_id = %s
                      AND l.lease_expires_at > clock_timestamp()
                      AND s.lease_expires_at > clock_timestamp()
                    """,
                    (old_file_id,),
                )
                or 0
            )
            if active_lease_count < 2:
                raise AssertionError(
                    f"expected two independent open leases, got {active_lease_count}"
                )

            # C zmienia namespace bez lokalnej wiedzy o uchwytach A/B.
            target_c.unlink()
            if target_c.exists():
                raise AssertionError("mount C pathname still visible after unlink")
            old_unlinked = scalar(
                database,
                "SELECT unlinked FROM fod.files WHERE id_file = %s",
                (old_file_id,),
            )
            if old_unlinked is not True:
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
                raise AssertionError("replacement reused old file generation")
            if replacement_ino == old_ino:
                raise AssertionError("replacement reused old inode")

            # B musi nadal czytac stara generacje po remote unlink/recreate.
            if os.pread(fd_b, len(initial), 0) != initial:
                raise AssertionError("survivor reader changed after remote unlink")
            b_stat_after_unlink = os.fstat(fd_b)
            if b_stat_after_unlink.st_ino != old_ino:
                raise AssertionError("survivor reader inode changed after remote unlink")
            if b_stat_after_unlink.st_nlink != 0:
                raise AssertionError(
                    f"survivor reader nlink expected 0, got {b_stat_after_unlink.st_nlink}"
                )

            # Zabijamy tylko FUSE A. B i C pozostaja zywe i odnawiaja swoje
            # sesje. PostgreSQL musi zachowac stara generacje przez lease B.
            if launcher_a.process is None:
                raise AssertionError("mount A bootstrap missing before crash")
            bootstrap_pid_a = launcher_a.process.pid
            os.kill(fuse_pid_a, signal.SIGKILL)
            try:
                bootstrap_rc_a = launcher_a.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                bootstrap_rc_a = None

            ttl_after_crash = remaining_session_ttl(database, session_a)
            if ttl_after_crash <= 0.0:
                raise AssertionError("crashed session was already expired")

            survivor_active_immediate = int(
                scalar(
                    database,
                    """
                    SELECT COUNT(*)
                    FROM fod.file_open_leases l
                    JOIN fod.client_sessions s ON s.session_id = l.session_id
                    WHERE l.file_id = %s
                      AND l.session_id = %s
                      AND l.lease_expires_at > clock_timestamp()
                      AND s.lease_expires_at > clock_timestamp()
                    """,
                    (old_file_id, session_b),
                )
                or 0
            )
            if survivor_active_immediate != 1:
                raise AssertionError("survivor lease inactive immediately after A crash")

            (
                crashed_sessions_left,
                crashed_leases_left,
                survivor_leases_left,
                old_files_left,
            ) = wait_crashed_session_pruned_while_survivor_protects(
                database,
                old_file_id,
                session_a,
                session_b,
                timeout_seconds=max(10.0, ttl_after_crash + 10.0),
            )

            # Po prune A stara generacja nadal musi byc dostepna przez B.
            if os.pread(fd_b, len(initial), 0) != initial:
                raise AssertionError("survivor reader lost old payload after A prune")
            b_stat_after_prune = os.fstat(fd_b)
            if b_stat_after_prune.st_ino != old_ino:
                raise AssertionError("survivor inode changed after A prune")
            if b_stat_after_prune.st_nlink != 0:
                raise AssertionError("survivor nlink changed after A prune")
            if target_c.read_bytes() != replacement:
                raise AssertionError("replacement changed while survivor protected old file")

            # Lokalny fd A nalezy juz do martwego FUSE.
            crashed_fd_close_errno = 0
            try:
                os.close(fd_a)
            except OSError as err:
                if err.errno != errno.ENOTCONN:
                    raise
                crashed_fd_close_errno = err.errno
            fd_a = None
            launcher_a.stop()

            # Dopiero release ostatniego aktywnego lease B moze odblokowac purge.
            os.close(fd_b)
            fd_b = None
            final_survivor_lease, final_old_file = wait_old_generation_removed(
                database,
                old_file_id,
                session_b,
            )

            if target_c.read_bytes() != replacement:
                raise AssertionError("replacement changed after final old-generation purge")
            visible_c = set(os.listdir(mount_c))
            leaked_internal = sorted(
                item for item in visible_c if item.startswith(".fod-unlinked-")
            )
            if leaked_internal:
                raise AssertionError(
                    f"deferred-unlink name leaked after multi-holder crash: {leaked_internal}"
                )

            target_c.unlink()

            print(
                "OK unlink-multiholder-crash-survivor "
                f"old_file_id={old_file_id} replacement_file_id={replacement_file_id} "
                f"old_ino={old_ino} replacement_ino={replacement_ino} "
                f"session_a={session_a} session_b={session_b} "
                f"bootstrap_pid_a={bootstrap_pid_a} fuse_pid_a={fuse_pid_a} "
                f"bootstrap_rc_a={bootstrap_rc_a} crashed_fd_close_errno={crashed_fd_close_errno} "
                f"ttl_after_crash={ttl_after_crash:.3f} "
                "postgres_authority=1 independent_mounts=3 active_open_leases_before_crash=2 "
                f"crashed_sessions_left={crashed_sessions_left} "
                f"crashed_leases_left={crashed_leases_left} "
                f"survivor_leases_left={survivor_leases_left} "
                f"old_files_left_while_survivor_active={old_files_left} "
                "survivor_lease_protected=1 survivor_read_after_prune=1 "
                f"final_survivor_lease={final_survivor_lease} "
                f"final_old_file={final_old_file} "
                "final_survivor_close_purge=1 replacement_isolated=1 "
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
