#!/usr/bin/env python
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import time
import tempfile
import uuid
from pathlib import Path

import psycopg2

from fod_mount import FODMount


def postgres_connection(launcher: FODMount):
    # Test integracyjny uzywa tego samego lokalnego endpointu co pozostale
    # testy uruchamiane przez make. Zmienne srodowiskowe maja pierwszenstwo.
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
    # Kazdy odczyt wykonujemy poza dluga transakcja, aby obserwowac biezacy
    # autorytatywny stan PostgreSQL zmieniany przez drugi mount.
    with connection.cursor() as cursor:
        cursor.execute(sql, params)
        row = cursor.fetchone()
    return None if row is None else row[0]


def env_truthy(name: str, default: bool) -> bool:
    value = os.environ.get(name)
    if value is None:
        return default
    return value.strip().lower() in {"1", "true", "yes", "on"}


def wait_until_old_generation_purged(
    connection,
    old_file_id: int,
    old_session_id: int,
    timeout_seconds: float = 10.0,
) -> tuple[bool, int, int, int]:
    deadline = time.monotonic() + timeout_seconds
    last_session = 1
    last_open_leases = 1
    last_file = 1

    while time.monotonic() < deadline:
        last_session = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM fod.client_sessions WHERE session_id = %s",
                (old_session_id,),
            )
            or 0
        )
        last_open_leases = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM fod.file_open_leases WHERE file_id = %s",
                (old_file_id,),
            )
            or 0
        )
        last_file = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM fod.files WHERE id_file = %s",
                (old_file_id,),
            )
            or 0
        )

        if last_session == 0 and last_open_leases == 0 and last_file == 0:
            return True, last_session, last_open_leases, last_file

        time.sleep(0.1)

    return False, last_session, last_open_leases, last_file


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    with tempfile.TemporaryDirectory(
        prefix="fod-unlink-open-writer-crash-convergence-"
    ) as temp_dir:
        temp = Path(temp_dir)
        mount_a = temp / "mount-a"
        mount_b = temp / "mount-b"

        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        launcher_a.init_schema()

        name = f"unlink-open-writer-crash-{uuid.uuid4().hex}.bin"
        initial = b"A" * (128 * 1024)
        replacement = b"replacement survives crashed old generation\n"

        fd: int | None = None
        database = postgres_connection(launcher_a)
        force_session_expiry = env_truthy("FOD_TEST_FORCE_SESSION_EXPIRY", True)

        try:
            launcher_a.start(
                str(mount_a),
                log_prefix="/tmp/fod-unlink-open-writer-crash-a",
            )
            launcher_b.start(
                str(mount_b),
                log_prefix="/tmp/fod-unlink-open-writer-crash-b",
            )

            target_a = mount_a / name
            target_b = mount_b / name

            # A tworzy i otwiera plik. Jawny fd pozostaje aktywny az do awarii
            # procesu FUSE i musi byc reprezentowany przez file_open_leases.
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
                raise AssertionError("old file_id missing before open")
            old_file_id = int(old_file_id)

            fd = os.open(target_a, os.O_RDWR)
            if os.pread(fd, len(initial), 0) != initial:
                raise AssertionError("mount A pre-unlink payload mismatch")

            old_session_id = scalar(
                database,
                """
                SELECT session_id
                FROM fod.file_open_leases
                WHERE file_id = %s
                ORDER BY session_id
                LIMIT 1
                """,
                (old_file_id,),
            )
            if old_session_id is None:
                raise AssertionError("PostgreSQL open lease missing for mount A handle")
            old_session_id = int(old_session_id)

            # B wykonuje unlink bez lokalnej wiedzy o fh z A. Stara generacja
            # musi pozostac w PostgreSQL jako unlinked, dopoki lease A jest zywy.
            target_b.unlink()
            if target_b.exists():
                raise AssertionError("mount B pathname still visible after remote unlink")

            old_unlinked = scalar(
                database,
                "SELECT unlinked FROM fod.files WHERE id_file = %s",
                (old_file_id,),
            )
            if old_unlinked is not True:
                raise AssertionError(
                    f"old generation not staged as unlinked: value={old_unlinked!r}"
                )

            # B odtwarza namespace. Nowy obiekt musi miec osobny file_id/inode
            # i przetrwac pozniejsze sprzatanie starej generacji.
            target_b.write_bytes(replacement)
            replacement_ino = target_b.stat().st_ino
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
                raise AssertionError(
                    "replacement reused old PostgreSQL file generation"
                )
            if replacement_ino == old_ino:
                raise AssertionError("replacement reused old inode generation")

            # SIGKILL symuluje utrate hosta/procesu. Nie wolno wywolywac normalnego
            # release przed sprawdzeniem, ze lease nadal istnieje w PostgreSQL.
            if launcher_a.process is None:
                raise AssertionError("mount A process missing before crash")
            launcher_a.process.kill()
            crash_rc = launcher_a.process.wait(timeout=5)

            # Stan PostgreSQL sprawdzamy natychmiast po SIGKILL, zanim
            # dotkniemy fd nalezacego do martwego polaczenia FUSE. close(2) na
            # martwym mountcie nie jest elementem crash semantics i nie moze
            # wplywac na pomiar naturalnego TTL.
            with database.cursor() as cursor:
                cursor.execute(
                    """
                    SELECT
                        (s.lease_expires_at > clock_timestamp()) AS session_active,
                        (l.lease_expires_at > clock_timestamp()) AS open_lease_active,
                        GREATEST(
                            EXTRACT(EPOCH FROM (s.lease_expires_at - clock_timestamp())),
                            0
                        )::double precision AS session_remaining,
                        GREATEST(
                            EXTRACT(EPOCH FROM (l.lease_expires_at - clock_timestamp())),
                            0
                        )::double precision AS open_lease_remaining
                    FROM fod.client_sessions s
                    JOIN fod.file_open_leases l
                      ON l.session_id = s.session_id
                    WHERE s.session_id = %s
                      AND l.file_id = %s
                    LIMIT 1
                    """,
                    (old_session_id, old_file_id),
                )
                lease_row = cursor.fetchone()

            if lease_row is None:
                raise AssertionError(
                    "crash lost PostgreSQL open lease before immediate snapshot"
                )

            session_active_after_crash = bool(lease_row[0])
            open_lease_active_after_crash = bool(lease_row[1])
            session_remaining_after_crash = float(lease_row[2] or 0.0)
            open_lease_remaining_after_crash = float(lease_row[3] or 0.0)

            if not session_active_after_crash or not open_lease_active_after_crash:
                raise AssertionError(
                    "crash left an already-expired PostgreSQL lease: "
                    f"session_active={int(session_active_after_crash)} "
                    f"open_lease_active={int(open_lease_active_after_crash)} "
                    f"session_remaining={session_remaining_after_crash:.3f} "
                    f"open_lease_remaining={open_lease_remaining_after_crash:.3f}"
                )

            # Tryb szybki przesuwa sesje za granice expiry. Tryb naturalny
            # nie modyfikuje lease i czeka na rzeczywiste lease_expires_at
            # wyliczone przez PostgreSQL. W obu przypadkach prune oraz purge
            # wykonuje zwykly maintenance thread zywego mountu B.
            expiry_remaining_seconds = float(
                scalar(
                    database,
                    """
                    SELECT GREATEST(
                        EXTRACT(EPOCH FROM (lease_expires_at - clock_timestamp())),
                        0
                    )::double precision
                    FROM fod.client_sessions
                    WHERE session_id = %s
                    """,
                    (old_session_id,),
                )
                or 0.0
            )

            if force_session_expiry:
                with database.cursor() as cursor:
                    cursor.execute(
                        """
                        UPDATE fod.client_sessions
                        SET lease_expires_at = clock_timestamp() - INTERVAL '1 second'
                        WHERE session_id = %s
                        """,
                        (old_session_id,),
                    )
                    if cursor.rowcount != 1:
                        raise AssertionError(
                            "failed to expire crashed client session in PostgreSQL"
                        )
                convergence_timeout_seconds = 10.0
            else:
                # Dodajemy zapas na maintenance B oraz scheduler hosta. Test
                # nadal czeka na rzeczywisty zegar PostgreSQL, bez zmiany lease.
                convergence_timeout_seconds = max(
                    10.0,
                    expiry_remaining_seconds + 10.0,
                )

            purged, sessions_left, open_leases_left, files_left = (
                wait_until_old_generation_purged(
                    database,
                    old_file_id,
                    old_session_id,
                    timeout_seconds=convergence_timeout_seconds,
                )
            )
            if not purged:
                raise AssertionError(
                    "live mount did not converge crashed deferred unlink: "
                    f"sessions={sessions_left} open_leases={open_leases_left} "
                    f"files={files_left}"
                )

            # Cleanup starej generacji nie moze naruszyc replacementu.
            if target_b.read_bytes() != replacement:
                raise AssertionError(
                    "replacement changed while crashed old generation was purged"
                )

            visible_b = set(os.listdir(mount_b))
            leaked_internal = sorted(
                item for item in visible_b if item.startswith(".fod-unlinked-")
            )
            if leaked_internal:
                raise AssertionError(
                    f"deferred-unlink name leaked after crash convergence: {leaked_internal}"
                )

            # Cleanup martwego mountu wykonujemy dopiero po potwierdzeniu
            # naturalnej konwergencji w PostgreSQL. Do tego momentu lokalny fd
            # pozostaje otwarty, ale nie istnieje juz proces, ktory moglby
            # odnowic lub zwolnic jego lease.
            launcher_a.stop()
            os.close(fd)
            fd = None

            target_b.unlink()

            print(
                "OK unlink-open-writer-crash-convergence "
                f"old_file_id={old_file_id} replacement_file_id={replacement_file_id} "
                f"old_ino={old_ino} replacement_ino={replacement_ino} "
                f"crash_rc={crash_rc} postgres_authority=1 "
                "lease_survived_crash=1 "
                f"session_remaining_after_crash={session_remaining_after_crash:.3f} "
                f"open_lease_remaining_after_crash={open_lease_remaining_after_crash:.3f} "
                f"forced_session_expiry={int(force_session_expiry)} "
                f"natural_session_expiry={int(not force_session_expiry)} "
                f"expiry_remaining_seconds={expiry_remaining_seconds:.3f} "
                "remote_prune=1 orphan_purge=1 old_generation_removed=1 "
                "replacement_isolated=1 hidden_entry_leaks=0 cleanup=1"
            )
        except Exception:
            launcher_a._dump_log()
            launcher_b._dump_log()
            raise
        finally:
            if fd is not None:
                try:
                    os.close(fd)
                except OSError:
                    pass
            try:
                launcher_b.stop()
            finally:
                launcher_a.stop()
                database.close()


if __name__ == "__main__":
    main()
