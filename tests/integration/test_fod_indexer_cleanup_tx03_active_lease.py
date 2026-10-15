#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX-03B: failed-import cleanup must refuse active FOD write leases.

SQL fixtures exercise file and destination lease checks separately against
a uniquely materialized source. No FUSE mount or real active writer is used.
Requires a dedicated *_test PostgreSQL database; never migrates schema.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import uuid
from pathlib import Path

import psycopg2
from psycopg2.extensions import make_dsn

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_indexer_testlib import (
    apply_database_env,
    cleanup_indexer_sources,
    cleanup_test_dir,
    prepare_clean_dir,
    unique_indexer_path,
    unique_source_name,
    write_tree,
)
from tests.integration.test_fod_indexer_cleanup_tx03_atomicity import (
    direct_indexer,
    isolated_dsn,
    plan_status,
    root_file_count,
    scalar,
)

DENIAL = "FOD_INDEXER_CLEANUP_ACTIVE_WRITER"


def run_cleanup(binary: Path, dsn: dict[str, str], plan_id: int) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env.pop("DATABASE_URL", None)
    env["FOD_INDEXER_CONNINFO"] = make_dsn(**dsn)
    env["FOD_PG_DBNAME"] = dsn["dbname"]
    env["POSTGRES_DB"] = dsn["dbname"]
    return subprocess.run(
        [str(binary), "cleanup-failed", "--plan", str(plan_id)],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
        timeout=60,
    )


def assert_denied(
    observer, binary: Path, dsn: dict[str, str], plan_id: int, root_name: str,
    expected_count: int, lease_kind: str, file_id: int,
) -> None:
    result = run_cleanup(binary, dsn, plan_id)
    combined = result.stdout + result.stderr
    if result.returncode == 0 or DENIAL not in combined:
        raise AssertionError(
            f"TX-03B {lease_kind} did not fail closed: "
            f"rc={result.returncode}\n{combined}"
        )
    actual_files = root_file_count(observer, root_name)
    actual_status = plan_status(observer, plan_id)
    active_lease = bool(
        scalar(
            observer,
            "SELECT EXISTS (SELECT 1 FROM {} WHERE {} = %s "
            "AND lease_expires_at > clock_timestamp())".format(
                "file_write_leases" if lease_kind == "file" else "destination_write_leases",
                "file_id" if lease_kind == "file" else "parent_key",
            ),
            (file_id,),
        )
    )
    if actual_files != expected_count or actual_status != "materialize_failed" or not active_lease:
        raise AssertionError(
            f"TX-03B {lease_kind} cleanup modified state despite live lease: "
            f"files={actual_files}, plan={actual_status}, lease={active_lease}"
        )
    print(
        f"TX03_ACTIVE_{lease_kind.upper()}_LEASE=DENIED "
        f"files_preserved={actual_files} plan_unchanged=1 lease_preserved=1",
        flush=True,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--indexer", type=Path, default=ROOT / "target/debug/fod-indexer"
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
    source_name = unique_source_name("tx03_active_lease")
    source_dir = prepare_clean_dir(unique_indexer_path("tx03-active-lease-src"))
    session_id = None
    registered = False
    observer = None
    try:
        with psycopg2.connect(**dsn) as conn:
            with conn.cursor() as cur:
                cur.execute(
                    """
                    SELECT current_database(),
                           to_regclass('fod.files') IS NOT NULL,
                           to_regclass('fod.client_sessions') IS NOT NULL,
                           to_regclass('fod.file_write_leases') IS NOT NULL,
                           to_regclass('fod.destination_write_leases') IS NOT NULL
                    """
                )
                values = cur.fetchone()
                if values is None or values[0] != dsn["dbname"] or not all(values[1:]):
                    raise RuntimeError(f"TX-03B wrong or incomplete test DB: {values}")

        write_tree(source_dir, {"protected.bin": b"TX03 protected writer payload"})
        direct_indexer(
            binary, dsn,
            ["source", "add", "--name", source_name, "--path", str(source_dir), "--kind", "local"],
        )
        registered = True
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
                "SELECT id_import_plan FROM index_import_plans "
                "WHERE source_filter = %s ORDER BY id_import_plan DESC LIMIT 1",
                (source_name,),
            )
        )
        root_name = f"index-source-{source_id}-import-{plan_id}"
        row = None
        with observer.cursor() as cur:
            cur.execute(
                """
                SELECT f.id_file, f.id_directory, f.name, f.data_object_id
                FROM files f JOIN directories d ON f.id_directory = d.id_directory
                WHERE d.id_parent IS NULL AND d.name = %s
                """,
                (root_name,),
            )
            row = cur.fetchone()
        if row is None or root_file_count(observer, root_name) != 1:
            raise AssertionError("TX-03B expected exactly one materialized file")
        file_id, parent_id, file_name, data_object_id = row

        with observer.cursor() as cur:
            cur.execute(
                "UPDATE index_import_plans SET status = 'materialize_failed', "
                "updated_at = NOW() WHERE id_import_plan = %s",
                (plan_id,),
            )
            cur.execute(
                """
                INSERT INTO client_sessions (
                    host_name, mountpoint, mount_mode, lock_backend, pid,
                    request_token, lease_expires_at, heartbeat_at, started_at, updated_at
                ) VALUES (
                    'tx03-test-host', '/tmp/tx03-test-mount', 'primary', 'postgres_lease',
                    %s, %s, NOW() + INTERVAL '5 minutes', NOW(), NOW(), NOW()
                ) RETURNING session_id
                """,
                (os.getpid(), "tx03-" + uuid.uuid4().hex),
            )
            session_id = int(cur.fetchone()[0])
            cur.execute(
                """
                INSERT INTO file_write_leases (
                    file_id, session_id, owner_key, lease_expires_at, heartbeat_at
                ) VALUES (%s, %s, 987654321, NOW() + INTERVAL '5 minutes', NOW())
                """,
                (file_id, session_id),
            )

        assert_denied(observer, binary, dsn, plan_id, root_name, 1, "file", file_id)

        with observer.cursor() as cur:
            cur.execute("DELETE FROM file_write_leases WHERE file_id = %s", (file_id,))
            cur.execute(
                """
                INSERT INTO destination_write_leases (
                    parent_key, name, session_id, owner_key, lease_expires_at, heartbeat_at
                ) VALUES (%s, %s, %s, 987654321, NOW() + INTERVAL '5 minutes', NOW())
                """,
                (parent_id, file_name, session_id),
            )
        assert_denied(observer, binary, dsn, plan_id, root_name, 1, "destination", parent_id)

        # A completed active-writer interval ends only after the fixture
        # session and its cascaded lease have been removed.
        with observer.cursor() as cur:
            cur.execute("DELETE FROM client_sessions WHERE session_id = %s", (session_id,))
        session_id = None

        clean = run_cleanup(binary, dsn, plan_id)
        if clean.returncode or root_file_count(observer, root_name) != 0 or plan_status(observer, plan_id) != "materialize_cleaned":
            raise AssertionError(
                f"TX-03B cleanup did not resume after lease release: "
                f"rc={clean.returncode}, stdout={clean.stdout}, stderr={clean.stderr}"
            )
        print(
            "TX03_CLEANUP_AFTER_LEASE_RELEASE=PASS "
            f"file_id={file_id} data_object_id={data_object_id} "
            "files=0 plan=materialize_cleaned"
        )
    finally:
        if observer is not None:
            if session_id is not None:
                with observer.cursor() as cur:
                    cur.execute("DELETE FROM client_sessions WHERE session_id = %s", (session_id,))
            observer.close()
        try:
            if registered:
                cleanup_indexer_sources(dsn, [source_name])
        finally:
            cleanup_test_dir(source_dir)


if __name__ == "__main__":
    main()
