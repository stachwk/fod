#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX-03A: verify atomic visibility of indexer cleanup under a blocked COMMIT.

Operates ONLY on an explicitly selected isolated test database. Creates a
uniquely named index source, materializes two files, then locks the import plan
row so cleanup blocks at its final UPDATE. Independent SQL must still see every
file and the old plan state until the cleanup transaction commits.

No FUSE mount, production database, schema migration or role changes.
This test does NOT cover active FUSE writers or lease/advisory protection.
"""

from __future__ import annotations

import argparse
import os
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
                f"cleanup finished before blocked UPDATE rc={process.returncode}\n"
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
                  AND query ILIKE '%%UPDATE index_import_plans%%'
                ORDER BY pid
                LIMIT 1
                """,
                (blocker_pid, blocker_pid),
            )
            row = cur.fetchone()
        if row:
            return int(row[0])
        time.sleep(0.1)
    raise AssertionError("TX-03A did not observe a blocked cleanup UPDATE before timeout")


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
        "--kill-before-commit",
        action="store_true",
        help="TX-03B: kill cleanup while UPDATE is blocked and verify rollback plus retry",
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
    source_registered = False
    try:
        write_tree(
            source_dir,
            {"first.txt": b"TX03 first payload", "second.txt": b"TX03 second payload"},
        )
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

        if args.kill_before_commit:
            # Zabij proces przed COMMIT, gdy kasowanie jest jeszcze niezatwierdzone.
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
        if process is not None:
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
            if source_registered:
                cleanup_indexer_sources(dsn, [source_name])
        finally:
            cleanup_test_dir(source_dir)


if __name__ == "__main__":
    main()
