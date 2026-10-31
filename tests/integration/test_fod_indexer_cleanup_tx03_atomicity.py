#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX-03A: verify atomic visibility of indexer cleanup under a blocked COMMIT.

Operates ONLY on an explicitly selected isolated test database. Creates a
uniquely named index source, materializes two files, then locks the import plan
row so cleanup blocks on its initial SELECT FOR UPDATE. Independent SQL must
still see every file and the old plan state until cleanup commits.

No FUSE mount, production database, schema migration or role changes.
This test does NOT cover active FUSE writers or lease/advisory protection.
"""

from __future__ import annotations

import argparse
import os
import select
import subprocess
import sys
import time
from pathlib import Path

import psycopg2
from psycopg2.extensions import make_dsn

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from fod_backend import load_dsn_from_config
from tests.integration.fod_indexer_testlib import (
    apply_database_env,
    cleanup_indexer_sources,
    cleanup_test_dir,
    prepare_clean_dir,
    unique_indexer_path,
    unique_source_name,
    write_tree,
)


def isolated_dsn() -> dict[str, str]:
    explicit_db = os.environ.get("FOD_PG_DBNAME") or os.environ.get("POSTGRES_DB")
    if (
        not explicit_db
        or explicit_db == "foddbname"
        or not explicit_db.lower().endswith("_test")
    ):
        raise RuntimeError(
            "TX-03A requires explicit FOD_PG_DBNAME=<dedicated_db_test> "
            "(never default foddbname)"
        )
    dsn, _ = load_dsn_from_config(ROOT)
    dsn = dict(dsn)
    dsn["dbname"] = explicit_db
    overrides = (
        ("host", "FOD_PG_HOST", "POSTGRES_HOST"),
        ("port", "FOD_PG_PORT", "POSTGRES_PORT"),
        ("user", "FOD_PG_USER", "POSTGRES_USER"),
        ("password", "FOD_PG_PASSWORD", "POSTGRES_PASSWORD"),
    )
    for key, primary, legacy in overrides:
        override = os.environ.get(primary) or os.environ.get(legacy)
        if override:
            dsn[key] = override
    return dsn


def direct_indexer(
    binary: Path, dsn: dict[str, str], args: list[str], timeout: int = 120
) -> str:
    env = os.environ.copy()
    env.pop("DATABASE_URL", None)
    env["FOD_INDEXER_CONNINFO"] = make_dsn(**dsn)
    env["FOD_PG_DBNAME"] = dsn["dbname"]
    env["POSTGRES_DB"] = dsn["dbname"]
    result = subprocess.run(
        [str(binary), *args],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )
    output = result.stdout + result.stderr
    if result.returncode:
        raise AssertionError(f"fod-indexer {args!r} failed rc={result.returncode}:\n{output}")
    return output


def scalar(conn, statement: str, params: tuple[object, ...]) -> object:
    with conn.cursor() as cur:
        cur.execute(statement, params)
        row = cur.fetchone()
        if row is None:
            raise AssertionError(f"empty result for {statement}")
        return row[0]


def root_file_count(conn, root_name: str) -> int:
    return int(
        scalar(
            conn,
            """
            SELECT COUNT(*)
            FROM files f
            JOIN directories d ON d.id_directory = f.id_directory
            WHERE d.id_parent IS NULL AND d.name = %s
            """,
            (root_name,),
        )
    )


def plan_status(conn, plan_id: int) -> str:
    return str(
        scalar(
            conn,
            "SELECT status FROM index_import_plans WHERE id_import_plan = %s",
            (plan_id,),
        )
    )


def wait_until_cleanup_blocked(observer, blocker_pid: int, process, timeout: float) -> int:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            stdout, stderr = process.communicate()
            raise AssertionError(
                f"cleanup finished before blocked plan lock rc={process.returncode}\n"
                f"stdout={stdout}\nstderr={stderr}"
            )
        with observer.cursor() as cur:
            cur.execute(
                """
                SELECT pid
                FROM pg_stat_activity
                WHERE datname = current_database()
                  AND pid <> %s
                  AND %s = ANY(pg_blocking_pids(pid))
                  AND query ILIKE '%%FROM index_import_plans%%'\n                  AND query ILIKE '%%FOR UPDATE%%'
                ORDER BY pid
                LIMIT 1
                """,
                (blocker_pid, blocker_pid),
            )
            row = cur.fetchone()
        if row:
            return int(row[0])
        time.sleep(0.1)
    raise AssertionError("TX-03A did not observe a blocked cleanup plan lock before timeout")


def wait_for_checkpoint(process, marker: str, timeout: float) -> None:
    if process.stderr is None:
        raise AssertionError("cleanup worker has no stderr pipe")
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            stdout, stderr = process.communicate()
            raise AssertionError(
                f"cleanup exited before checkpoint {marker!r} rc={process.returncode}\n"
                f"stdout={stdout}\nstderr={stderr}"
            )
        readable, _, _ = select.select([process.stderr], [], [], 0.2)
        if readable and marker in process.stderr.readline():
            return
    raise AssertionError(f"cleanup did not reach checkpoint {marker!r}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--indexer",
        type=Path,
        default=ROOT / "target/debug/fod-indexer",
        help="path to already built fod-indexer executable",
    )
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument(
        "--crash-after-delete", action="store_true",
        help="TX03D: kill debug indexer after DELETE before COMMIT",
    )
    parser.add_argument(
        "--concurrent-cleanup", action="store_true",
        help="TX-03C: run two independent cleanup processes on the same plan",
    )
    parser.add_argument(
        "--concurrent-reference-increment", action="store_true",
        help="TX-03D: add a shared-object file reference after cleanup scans survivors",
    )
    parser.add_argument(
        "--kill-before-commit",
        action="store_true",
        help="TX-03B: kill cleanup while plan row is blocked and verify rollback plus retry",
    )
    args = parser.parse_args()
    binary = args.indexer.resolve()
    if not binary.is_file():
        raise RuntimeError(
            f"indexer binary missing: {binary}; run cargo build --locked -p fod-rust-indexer"
        )

    dsn = isolated_dsn()
    apply_database_env(ROOT, dsn)
    os.environ["FOD_PG_DBNAME"] = dsn["dbname"]
    source_name = unique_source_name("tx03_atomic")
    source_dir = prepare_clean_dir(unique_indexer_path("tx03-atomic-src"))
    blocker = None
    observer = None
    process = None
    second_process = None
    source_registered = False
    tx03d_external_names: list[str] = []
    tx03d_data_object_id: int | None = None
    tx03d_payload = f"TX03 shared payload {source_name}".encode()
    try:
        source_files = (
            {"first.txt": tx03d_payload, "second.txt": tx03d_payload}
            if args.concurrent_reference_increment
            else {"first.txt": b"TX03 first payload", "second.txt": b"TX03 second payload"}
        )
        write_tree(source_dir, source_files)
        # Readiness check precedes all mutations. In particular, this test
        # must not initialize or upgrade a real FOD schema.
        with psycopg2.connect(**dsn) as conn:
            with conn.cursor() as cur:
                cur.execute(
                    """
                    SELECT current_database(),
                           to_regclass('fod.files') IS NOT NULL,
                           to_regclass('fod.index_import_plans') IS NOT NULL
                    """
                )
                dbname, files_ready, plans_ready = cur.fetchone()
            if dbname != dsn["dbname"] or not (files_ready and plans_ready):
                raise RuntimeError(
                    f"TX-03A wrong or uninitialized database: {dbname!r}"
                )

        direct_indexer(
            binary, dsn,
            ["source", "add", "--name", source_name, "--path", str(source_dir), "--kind", "local"],
        )
        source_registered = True
        direct_indexer(binary, dsn, ["scan", "--source", source_name])
        if args.concurrent_reference_increment:
            direct_indexer(binary, dsn, ["hash", "--source", source_name])
        direct_indexer(binary, dsn, ["materialize", "--source", source_name])

        observer = psycopg2.connect(**dsn)
        observer.autocommit = True
        with observer.cursor() as cur:
            cur.execute("SET search_path TO fod, public")
        source_id = int(
            scalar(observer, "SELECT id_index_source FROM index_sources WHERE name = %s", (source_name,))
        )
        plan_id = int(
            scalar(
                observer,
                """
                SELECT id_import_plan
                FROM index_import_plans
                WHERE source_filter = %s
                ORDER BY id_import_plan DESC LIMIT 1
                """,
                (source_name,),
            )
        )
        root_name = f"index-source-{source_id}-import-{plan_id}"
        files_before = root_file_count(observer, root_name)
        if files_before != 2:
            raise AssertionError(
                f"TX-03A expected 2 materialized files, got {files_before}"
            )
        with observer.cursor() as cur:
            cur.execute(
                """
                UPDATE index_import_plans
                SET status = 'materialize_failed', updated_at = NOW()
                WHERE id_import_plan = %s
                """,
                (plan_id,),
            )

        if args.concurrent_reference_increment:
            imported_rows = []
            with observer.cursor() as cur:
                cur.execute(
                    """
                    SELECT id_file, data_object_id
                    FROM files
                    WHERE id_directory = (
                        SELECT id_directory FROM directories
                        WHERE id_parent IS NULL AND name = %s
                    )
                    ORDER BY id_file
                    """,
                    (root_name,),
                )
                imported_rows = [(int(row[0]), int(row[1])) for row in cur.fetchall()]
            if len(imported_rows) != 2 or imported_rows[0][1] != imported_rows[1][1]:
                raise AssertionError(
                    f"TX-03D expected two files sharing one object: {imported_rows}"
                )
            tx03d_data_object_id = imported_rows[0][1]
            tx03d_external_names = [
                f"tx03d-survivor-{plan_id}",
                f"tx03d-concurrent-{plan_id}",
            ]
            with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
                cur.execute("SET search_path TO fod, public")
                cur.execute(
                    """
                    INSERT INTO files (
                        data_object_id, id_directory, name, size, mode, uid, gid,
                        inode_seed, modification_date, access_date, change_date, creation_date
                    )
                    SELECT data_object_id, NULL, %s, size, mode, uid, gid, %s,
                           modification_date, access_date, change_date, creation_date
                    FROM files WHERE id_file = %s
                    RETURNING id_file
                    """,
                    (tx03d_external_names[0], f"tx03d-{plan_id}-survivor", imported_rows[0][0]),
                )
                if cur.fetchone() is None:
                    raise AssertionError("TX-03D could not create initial shared survivor")
                cur.execute(
                    """
                    UPDATE data_objects
                    SET reference_count = reference_count + 1
                    WHERE id_data_object = %s
                    """,
                    (tx03d_data_object_id,),
                )
                if cur.rowcount != 1:
                    raise AssertionError("TX-03D failed to count the initial shared survivor")

            env = os.environ.copy()
            env.pop("DATABASE_URL", None)
            env["FOD_INDEXER_CONNINFO"] = make_dsn(**dsn)
            env["FOD_PG_DBNAME"] = dsn["dbname"]
            env["POSTGRES_DB"] = dsn["dbname"]
            env["FOD_TX03D_AFTER_REFERENCE_SCAN_PAUSE"] = "1"
            process = subprocess.Popen(
                [str(binary), "cleanup-failed", "--plan", str(plan_id)],
                cwd=ROOT,
                env=env,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                bufsize=1,
            )
            wait_for_checkpoint(
                process,
                f"TX03D_AFTER_REFERENCE_SCAN_READY plan_id={plan_id}",
                args.timeout,
            )

            with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
                cur.execute("SET search_path TO fod, public")
                cur.execute(
                    """
                    INSERT INTO files (
                        data_object_id, id_directory, name, size, mode, uid, gid,
                        inode_seed, modification_date, access_date, change_date, creation_date
                    )
                    SELECT data_object_id, NULL, %s, size, mode, uid, gid, %s,
                           modification_date, access_date, change_date, creation_date
                    FROM files WHERE id_file = %s
                    RETURNING id_file
                    """,
                    (tx03d_external_names[1], f"tx03d-{plan_id}-concurrent", imported_rows[0][0]),
                )
                if cur.fetchone() is None:
                    raise AssertionError("TX-03D could not create concurrent shared reference")
                cur.execute(
                    """
                    UPDATE data_objects
                    SET reference_count = reference_count + 1
                    WHERE id_data_object = %s
                    """,
                    (tx03d_data_object_id,),
                )
                if cur.rowcount != 1:
                    raise AssertionError("TX-03D failed to count concurrent shared reference")

            stdout, stderr = process.communicate(
                input="resume\n", timeout=args.timeout
            )
            returncode = process.returncode
            process = None
            if returncode:
                raise AssertionError(
                    f"TX-03D cleanup failed rc={returncode}\n{stdout}\n{stderr}"
                )

            actual_count = int(
                scalar(
                    observer,
                    "SELECT reference_count FROM data_objects WHERE id_data_object = %s",
                    (tx03d_data_object_id,),
                )
            )
            file_count = int(
                scalar(
                    observer,
                    "SELECT COUNT(*) FROM files WHERE data_object_id = %s",
                    (tx03d_data_object_id,),
                )
            )
            payload = bytes(
                scalar(
                    observer,
                    "SELECT data FROM data_blocks WHERE data_object_id = %s AND _order = 0",
                    (tx03d_data_object_id,),
                )
            )
            if actual_count != 2 or file_count != 2:
                raise AssertionError(
                    f"TX-03D reference mismatch count={actual_count} files={file_count}"
                )
            if payload != tx03d_payload:
                raise AssertionError(f"TX-03D surviving payload changed: {payload!r}")
            if root_file_count(observer, root_name) != 0:
                raise AssertionError("TX-03D cleanup left imported file rows")
            if plan_status(observer, plan_id) != "materialize_cleaned":
                raise AssertionError("TX-03D cleanup did not mark plan cleaned")
            print("TX03D_REFCOUNT_RACE=PASS count=2 files=2 payload=preserved")
            return

        if args.crash_after_delete:
            # Test wykorzystuje punkt kontrolny w debug Rust, a nie blokade planu.
            env = os.environ.copy()
            env.pop("DATABASE_URL", None)
            env["FOD_INDEXER_CONNINFO"] = make_dsn(**dsn)
            env["FOD_PG_DBNAME"] = dsn["dbname"]
            env["POSTGRES_DB"] = dsn["dbname"]
            env["FOD_TX03D_AFTER_DELETE_PAUSE"] = "1"
            process = subprocess.Popen(
                [str(binary), "cleanup-failed", "--plan", str(plan_id)],
                cwd=ROOT, env=env, stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, text=True, bufsize=1,
            )
            deadline = time.monotonic() + args.timeout
            ready = False
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    break
                # Odczytujemy znacznik bez ryzyka blokowania readline bez danych.
                readable, _, _ = select.select([process.stderr], [], [], 0.2)
                if readable:
                    line = process.stderr.readline()
                    if f"TX03D_AFTER_DELETE_READY plan_id={plan_id}" in line:
                        ready = True
                        break
            if not ready:
                raise AssertionError("TX03D missing post-DELETE checkpoint")
            # Inna sesja nadal widzi stan sprzed transakcji.
            if root_file_count(observer, root_name) != files_before:
                raise AssertionError("TX03D uncommitted file deletion became visible")
            if plan_status(observer, plan_id) != "materialize_failed":
                raise AssertionError("TX03D uncommitted plan status became visible")
            print("TX03D_AFTER_DELETE_PRECOMMIT=PASS files=2", flush=True)

            # SIGKILL wymusza zerwanie polaczenia i rollback w PostgreSQL.
            process.kill()
            process.communicate(timeout=args.timeout)
            process = None
            if root_file_count(observer, root_name) != files_before:
                raise AssertionError("TX03D rollback lost files")
            if plan_status(observer, plan_id) != "materialize_failed":
                raise AssertionError("TX03D rollback changed plan")
            print("TX03D_DELETE_ROLLBACK=PASS files=2 plan=materialize_failed", flush=True)
            direct_indexer(binary, dsn, ["cleanup-failed", "--plan", str(plan_id)])
            if root_file_count(observer, root_name) != 0:
                raise AssertionError("TX03D retry left files")
            if plan_status(observer, plan_id) != "materialize_cleaned":
                raise AssertionError("TX03D retry did not clean plan")
            print("TX03D_RETRY_COMMIT=PASS files=0 plan=materialize_cleaned")
            return

        blocker = psycopg2.connect(**dsn)
        with blocker.cursor() as cur:
            cur.execute("SET search_path TO fod, public")
            cur.execute(
                "SELECT id_import_plan FROM index_import_plans WHERE id_import_plan = %s FOR UPDATE",
                (plan_id,),
            )
            if cur.fetchone() is None:
                raise AssertionError("TX-03A import plan disappeared before row lock")
            cur.execute("SELECT pg_backend_pid()")
            blocker_pid = int(cur.fetchone()[0])

        env = os.environ.copy()
        env.pop("DATABASE_URL", None)
        env["FOD_INDEXER_CONNINFO"] = make_dsn(**dsn)
        env["FOD_PG_DBNAME"] = dsn["dbname"]
        env["POSTGRES_DB"] = dsn["dbname"]
        process = subprocess.Popen(
            [str(binary), "cleanup-failed", "--plan", str(plan_id)],
            cwd=ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        cleanup_pid = wait_until_cleanup_blocked(
            observer, blocker_pid, process, args.timeout
        )
        before_commit_files = root_file_count(observer, root_name)
        before_commit_plan = plan_status(observer, plan_id)
        if before_commit_files != files_before or before_commit_plan != "materialize_failed":
            raise AssertionError(
                "TX-03A partial cleanup became visible before COMMIT: "
                f"files={before_commit_files} plan={before_commit_plan}"
            )
        print(
            "TX03_ATOMIC_PRECOMMIT=PASS "
            f"files={before_commit_files} plan={before_commit_plan} cleanup_pid={cleanup_pid}",
            flush=True,
        )

        if args.concurrent_cleanup:
            # Uruchom drugiego indexera, gdy pierwszy oczekuje na blokade planu.
            second_process = subprocess.Popen(
                [str(binary), "cleanup-failed", "--plan", str(plan_id)],
                cwd=ROOT, env=env, stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, text=True,
            )
            deadline = time.monotonic() + args.timeout
            # Kolejka blokad moze byc lancuchowa: drugi proces czeka na pierwszy.\n            blocked_count = 0
            while time.monotonic() < deadline:
                if second_process.poll() is not None or process.poll() is not None:
                    raise AssertionError("TX-03C cleanup worker exited before releasing plan lock")
                with observer.cursor() as cur:
                    cur.execute(
                        """
                        SELECT count(*)
                        FROM pg_stat_activity
                        WHERE datname = current_database()
                          AND pid <> %s
                          AND cardinality(pg_blocking_pids(pid)) > 0
                          AND wait_event_type = 'Lock'
                          AND query ILIKE '%%FROM index_import_plans%%'
                          AND query ILIKE '%%FOR UPDATE%%'
                        """,
                        (blocker_pid,),
                    )
                    blocked_count = int(cur.fetchone()[0])
                if blocked_count >= 2:
                    break
                time.sleep(0.1)
            if blocked_count < 2:
                raise AssertionError(
                    f"TX-03C expected two blocked workers, observed {blocked_count}"
                )
            print("TX03C_TWO_WORKERS_BLOCKED=PASS workers=2", flush=True)
            blocker.commit()
            first_out, first_err = process.communicate(timeout=args.timeout)
            second_out, second_err = second_process.communicate(timeout=args.timeout)
            first_rc = process.returncode
            second_rc = second_process.returncode
            process = None
            second_process = None
            if first_rc != 0 or second_rc != 0:
                raise AssertionError(
                    f"TX-03C workers failed: first={first_rc} second={second_rc}\n"
                    f"first: {first_out} {first_err}\nsecond: {second_out} {second_err}"
                )
            after_files = root_file_count(observer, root_name)
            after_plan = plan_status(observer, plan_id)
            if after_files != 0 or after_plan != "materialize_cleaned":
                raise AssertionError(
                    f"TX-03C inconsistent final state files={after_files} plan={after_plan}"
                )
            print("TX03C_SERIALIZED_COMMIT=PASS workers=2 files=0 plan=materialize_cleaned")
            return

        if args.kill_before_commit:
            # Zabij proces, gdy oczekuje na blokade planu przed kasowaniem.
            process.kill()
            process.communicate(timeout=args.timeout)
            process = None

            # Zwolnij blokade dopiero po zamknieciu procesu i jego transakcji.
            blocker.rollback()
            rolled_back_files = root_file_count(observer, root_name)
            rolled_back_plan = plan_status(observer, plan_id)
            if rolled_back_files != files_before or rolled_back_plan != "materialize_failed":
                raise AssertionError(
                    "TX-03B rollback lost data or modified plan: "
                    f"files={rolled_back_files} plan={rolled_back_plan}"
                )
            print(
                "TX03B_KILL_ROLLBACK=PASS "
                f"files={rolled_back_files} plan={rolled_back_plan}",
                flush=True,
            )

            # Ponowienie polecenia musi posprzatac calosc po wycofanej transakcji.
            direct_indexer(binary, dsn, ["cleanup-failed", "--plan", str(plan_id)])
            retried_files = root_file_count(observer, root_name)
            retried_plan = plan_status(observer, plan_id)
            if retried_files != 0 or retried_plan != "materialize_cleaned":
                raise AssertionError(
                    "TX-03B retry failed: "
                    f"files={retried_files} plan={retried_plan}"
                )
            print("TX03B_RETRY_COMMIT=PASS files=0 plan=materialize_cleaned")
            return

        blocker.commit()
        stdout, stderr = process.communicate(timeout=args.timeout)
        if process.returncode:
            raise AssertionError(
                f"TX-03A cleanup failed rc={process.returncode}\n{stdout}\n{stderr}"
            )
        process = None
        after_files = root_file_count(observer, root_name)
        after_plan = plan_status(observer, plan_id)
        if after_files != 0 or after_plan != "materialize_cleaned":
            raise AssertionError(
                f"TX-03A cleanup not atomically committed: files={after_files} plan={after_plan}"
            )
        print("TX03_ATOMIC_COMMIT=PASS files=0 plan=materialize_cleaned")
    finally:
        # Stop the diagnostic worker BEFORE releasing the blocker. Otherwise
        # a failed pre-COMMIT assertion might unintentionally allow cleanup
        # to commit while the failure handler is tearing down the fixture.
        if second_process is not None:
            if second_process.poll() is None:
                second_process.kill()
            second_process.communicate()
        if process is not None:
            if process.poll() is None:
                process.kill()
            process.communicate()
        if blocker is not None:
            try:
                blocker.rollback()
            finally:
                blocker.close()
        if observer is not None:
            observer.close()
        try:
            if tx03d_external_names and tx03d_data_object_id is not None:
                with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
                    cur.execute("SET search_path TO fod, public")
                    cur.execute(
                        "DELETE FROM files WHERE id_directory IS NULL AND name = ANY(%s)",
                        (tx03d_external_names,),
                    )
                    # Restore this isolated fixture from its surviving file rows,
                    # even when the regression correctly exposed a stale counter.
                    cur.execute(
                        """
                        UPDATE data_objects d
                        SET reference_count = (
                                SELECT COUNT(*) FROM files f
                                WHERE f.data_object_id = d.id_data_object
                            ),
                            modification_date = NOW()
                        WHERE d.id_data_object = %s
                        """,
                        (tx03d_data_object_id,),
                    )
                    cur.execute(
                        """
                        DELETE FROM data_objects
                        WHERE id_data_object = %s AND reference_count = 0
                        """,
                        (tx03d_data_object_id,),
                    )
            if source_registered:
                cleanup_indexer_sources(dsn, [source_name])
        finally:
            cleanup_test_dir(source_dir)


if __name__ == "__main__":
    main()
