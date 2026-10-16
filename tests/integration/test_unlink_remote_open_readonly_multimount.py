#!/usr/bin/env python
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import tempfile
import time
import uuid
from pathlib import Path

import psycopg2

from fod_mount import FODMount


def postgres_connection(launcher: FODMount):
    # Test obserwuje ten sam autorytatywny PostgreSQL co oba mounty.
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


def active_lease_session(
    connection,
    file_id: int,
    mountpoint: Path,
) -> int:
    with connection.cursor() as cursor:
        cursor.execute(
            """
            SELECT s.session_id
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
            f"read-only mount has no active central open lease: {mountpoint}"
        )
    return int(row[0])


def wait_file_removed(connection, file_id: int, timeout_seconds: float = 10.0) -> None:
    deadline = time.monotonic() + timeout_seconds
    last = 1
    while time.monotonic() < deadline:
        last = int(
            scalar(
                connection,
                "SELECT COUNT(*) FROM fod.files WHERE id_file = %s",
                (file_id,),
            )
            or 0
        )
        if last == 0:
            return
        time.sleep(0.05)
    raise AssertionError(
        f"old read-only-held generation was not purged file_id={file_id} rows={last}"
    )


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    # Ten gate celowo omija endpoint routing i explicit telemetry DSN.
    # Read-only mount wskazuje bezposrednio writable primary jako data repo.
    # W takim ukladzie ten sam primary musi pelnic role centralnej authority
    # dla client_sessions i file_open_leases.
    previous = {
        "FOD_PG_ENDPOINT_ROUTING_ENABLED": os.environ.get(
            "FOD_PG_ENDPOINT_ROUTING_ENABLED"
        ),
        "FOD_TELEMETRY_DSN": os.environ.get("FOD_TELEMETRY_DSN"),
        "FOD_MONITOR_DSN": os.environ.get("FOD_MONITOR_DSN"),
    }
    os.environ["FOD_PG_ENDPOINT_ROUTING_ENABLED"] = "0"
    os.environ.pop("FOD_TELEMETRY_DSN", None)
    os.environ.pop("FOD_MONITOR_DSN", None)

    try:
        with tempfile.TemporaryDirectory(
            prefix="fod-unlink-remote-open-readonly-"
        ) as temp_dir:
            temp = Path(temp_dir)
            mount_a = temp / "mount-a"
            mount_b = temp / "mount-b"

            launcher_a = FODMount(str(root), role="primary")
            launcher_b = FODMount(str(root), role="replica")
            launcher_a.init_schema()

            name = f"unlink-remote-open-readonly-{uuid.uuid4().hex}.bin"
            initial = b"FOD read-only open lease survives remote unlink\n"
            replacement = b"replacement after read-only remote unlink\n"

            fd: int | None = None
            database = postgres_connection(launcher_a)
            try:
                launcher_a.start(
                    str(mount_a),
                    log_prefix="/tmp/fod-unlink-remote-open-readonly-a",
                )
                launcher_b.start(
                    str(mount_b),
                    log_prefix="/tmp/fod-unlink-remote-open-readonly-b",
                )

                target_a = mount_a / name
                target_b = mount_b / name

                target_a.write_bytes(initial)
                before = target_b.stat()
                old_ino = before.st_ino
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

                fd = os.open(target_b, os.O_RDONLY)
                if os.pread(fd, len(initial), 0) != initial:
                    raise AssertionError("read-only pre-unlink payload mismatch")

                session_b = active_lease_session(
                    database,
                    old_file_id,
                    mount_b,
                )

                # Writable mount A usuwa namespace. Decyzja o zachowaniu starej
                # generacji musi uwzglednic lease read-only mounta B.
                target_a.unlink()
                if target_a.exists():
                    raise AssertionError("remote pathname still visible after unlink")
                if scalar(
                    database,
                    "SELECT unlinked FROM fod.files WHERE id_file = %s",
                    (old_file_id,),
                ) is not True:
                    raise AssertionError(
                        "old generation was not staged while read-only lease was active"
                    )

                target_a.write_bytes(replacement)
                replacement_ino = target_a.stat().st_ino
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

                # Zdalny host nie moze natychmiast uniewaznic kernelowego
                # attr cache tego mounta. Czekamy tylko na jego ograniczona
                # konwergencje; inode starego fd musi pozostac stabilny przez
                # caly okres, a authority w PostgreSQL juz ma unlinked=true.
                attr_deadline = time.monotonic() + 5.0
                old_stat = os.fstat(fd)
                while old_stat.st_nlink != 0 and time.monotonic() < attr_deadline:
                    if old_stat.st_ino != old_ino:
                        raise AssertionError(
                            "read-only open fd changed inode during attr convergence: "
                            f"expected={old_ino} actual={old_stat.st_ino}"
                        )
                    time.sleep(0.05)
                    old_stat = os.fstat(fd)

                if old_stat.st_ino != old_ino:
                    raise AssertionError(
                        "read-only open fd changed inode after remote unlink: "
                        f"expected={old_ino} actual={old_stat.st_ino}"
                    )
                if old_stat.st_nlink != 0:
                    raise AssertionError(
                        "read-only open-unlinked fd did not converge to zero links: "
                        f"st_nlink={old_stat.st_nlink}"
                    )
                if os.pread(fd, len(initial), 0) != initial:
                    raise AssertionError(
                        "read-only open fd lost old payload after remote unlink"
                    )
                if target_a.read_bytes() != replacement:
                    raise AssertionError(
                        "read-only old generation affected replacement payload"
                    )

                os.close(fd)
                fd = None
                wait_file_removed(database, old_file_id)

                if target_a.read_bytes() != replacement:
                    raise AssertionError(
                        "replacement changed after read-only lease release"
                    )
                if target_a.stat().st_ino != replacement_ino:
                    raise AssertionError(
                        "replacement inode changed after read-only lease release"
                    )

                leaked_internal = sorted(
                    item
                    for item in os.listdir(mount_a)
                    if item.startswith(".fod-unlinked-")
                )
                if leaked_internal:
                    raise AssertionError(
                        f"deferred-unlink name leaked after read-only close: {leaked_internal}"
                    )

                target_a.unlink()

                print(
                    "OK unlink-remote-open-readonly-multimount "
                    f"old_file_id={old_file_id} "
                    f"replacement_file_id={replacement_file_id} "
                    f"old_ino={old_ino} replacement_ino={replacement_ino} "
                    f"readonly_session={session_b} "
                    "postgres_authority=1 independent_mounts=2 "
                    "readonly_central_session=1 readonly_open_lease=1 "
                    "remote_unlink=1 old_fstat_inode_stable=1 "
                    "old_fstat_nlink_zero=1 old_pread_after_unlink=1 "
                    "final_old_purge=1 replacement_isolated=1 "
                    "hidden_entry_leaks=0 cleanup=1"
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
                        # Cleanup nie moze zamaskowac pierwotnego bledu testu.
                        # Poprawny close jest sprawdzany jawnie na sciezce PASS.
                        pass
                database.close()
                try:
                    launcher_b.stop()
                finally:
                    launcher_a.stop()
    finally:
        for key, value in previous.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value


if __name__ == "__main__":
    main()
