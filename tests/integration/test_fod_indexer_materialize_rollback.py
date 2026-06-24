#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from fod_backend import load_dsn_from_config
from tests.integration.fod_indexer_testlib import (
    apply_database_env,
    assert_contains,
    cleanup_indexer_state,
    cleanup_materialized_roots,
    fetch_one,
    run_indexer,
    snapshot_tree,
    write_tree,
)
from tests.integration.fod_mount import FODMount

SOURCE_ROOT = Path("/tmp/fod-indexer-materialize-rollback-src")
SOURCE_FILES: dict[str, bytes] = {
    "rollback.txt": b"rollback me\n",
}


def wait_for_path_missing(path: Path, timeout_s: float = 10.0) -> None:
    deadline = time.monotonic() + timeout_s
    last_state = True
    while time.monotonic() < deadline:
        last_state = path.exists()
        if not last_state:
            return
        time.sleep(0.2)
    raise AssertionError(f"timed out waiting for {path} to disappear; last_exists={last_state}")


def run_indexer_result(root: Path, args: list[str], extra_env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env.pop("DATABASE_URL", None)
    env.pop("FOD_INDEXER_CONNINFO", None)
    env["INDEXER_ARGS"] = shlex.join(args)
    if extra_env:
        env.update(extra_env)
    return subprocess.run(
        ["make", "--no-print-directory", "indexer"],
        cwd=root,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


def install_materialize_failure_trigger(dsn: dict[str, str]) -> None:
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        cur.execute("SET search_path TO fod, public")
        cur.execute(
            """
            CREATE OR REPLACE FUNCTION fod_force_materialize_rollback_failure()
            RETURNS trigger AS $$
            BEGIN
                RAISE EXCEPTION 'forced materialize failure for rollback smoke';
                RETURN NEW;
            END;
            $$ LANGUAGE plpgsql
            """
        )
        cur.execute("DROP TRIGGER IF EXISTS fod_force_materialize_rollback_failure ON index_import_plan_entries")
        cur.execute(
            """
            CREATE TRIGGER fod_force_materialize_rollback_failure
            BEFORE INSERT ON index_import_plan_entries
            FOR EACH ROW
            EXECUTE FUNCTION fod_force_materialize_rollback_failure()
            """
        )


def remove_materialize_failure_trigger(dsn: dict[str, str]) -> None:
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        cur.execute("SET search_path TO fod, public")
        cur.execute("DROP TRIGGER IF EXISTS fod_force_materialize_rollback_failure ON index_import_plan_entries")
        cur.execute("DROP FUNCTION IF EXISTS fod_force_materialize_rollback_failure()")


def main() -> None:
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    cleanup_indexer_state(dsn)
    cleanup_materialized_roots(dsn)

    source_snapshot = None
    root_name = None
    try:
        write_tree(SOURCE_ROOT, SOURCE_FILES)
        source_snapshot = snapshot_tree(SOURCE_ROOT)

        with tempfile.TemporaryDirectory(prefix="fod-indexer-materialize-rollback-") as mount_dir:
            with FODMount(str(ROOT)) as mount:
                mount.init_schema()
                cleanup_indexer_state(dsn)
                mount.start(mount_dir)
                cleanup_materialized_roots(dsn)

                source_add_output = run_indexer(
                    ROOT,
                    [
                        "source",
                        "add",
                        "--name",
                        "rollback-smoke",
                        "--path",
                        str(SOURCE_ROOT),
                        "--kind",
                        "local",
                    ],
                )
                assert_contains(source_add_output, "Registered source rollback-smoke as local", "source add")
                assert_contains(source_add_output, str(SOURCE_ROOT), "source add")

                install_materialize_failure_trigger(dsn)

                materialize_result = run_indexer_result(
                    ROOT,
                    ["materialize", "--source", "rollback-smoke"],
                )
                if materialize_result.returncode == 0:
                    raise AssertionError("materialize unexpectedly succeeded under forced failure")

                materialize_output = materialize_result.stdout + materialize_result.stderr
                assert_contains(
                    materialize_output,
                    "forced materialize failure for rollback smoke",
                    "materialize failure",
                )
                assert_contains(
                    materialize_output,
                    "partial materialization was rolled back automatically",
                    "materialize failure",
                )

                with psycopg2.connect(**dsn) as conn:
                    with conn.cursor() as cur:
                        cur.execute("SET search_path TO fod, public")
                    source_id = int(
                        fetch_one(
                            conn,
                            "SELECT id_index_source FROM index_sources WHERE name = %s",
                            ("rollback-smoke",),
                        )
                    )
                    plan_row = fetch_one(
                        conn,
                        """
                        SELECT id_import_plan, status
                        FROM index_import_plans
                        WHERE source_filter = %s
                        ORDER BY id_import_plan DESC
                        LIMIT 1
                        """,
                        ("rollback-smoke",),
                    )
                    if not isinstance(plan_row, tuple) or len(plan_row) < 2:
                        raise AssertionError(f"unexpected plan row: {plan_row}")
                    plan_id = int(plan_row[0])
                    plan_status = str(plan_row[1])
                    if plan_status != "materialize_cleaned":
                        raise AssertionError(f"unexpected plan status after automatic rollback: {plan_status}")

                    root_name = f"index-source-{source_id}-import-{plan_id}"
                    assert_contains(materialize_output, f"plan {plan_id}", "materialize failure")

                    mount_root = mount.config.mountpoint / root_name  # type: ignore[union-attr]
                    wait_for_path_missing(mount_root)
                    if mount_root.exists():
                        raise AssertionError(f"rollback root still exists on mount: {mount_root}")

                    with conn.cursor() as cur:
                        cur.execute(
                            "SELECT COUNT(*) FROM directories WHERE id_parent IS NULL AND name = %s",
                            (root_name,),
                        )
                        if int(cur.fetchone()[0]) != 0:
                            raise AssertionError("rollback root still exists in directories")
                        cur.execute(
                            """
                            WITH RECURSIVE subtree AS (
                                SELECT id_directory
                                FROM directories
                                WHERE id_parent IS NULL AND name = %s
                                UNION ALL
                                SELECT d.id_directory
                                FROM directories d
                                JOIN subtree s ON d.id_parent = s.id_directory
                            )
                            SELECT COUNT(*)
                            FROM files
                            WHERE id_directory IN (SELECT id_directory FROM subtree)
                            """,
                            (root_name,),
                        )
                        if int(cur.fetchone()[0]) != 0:
                            raise AssertionError("rollback root still has files")

                if snapshot_tree(SOURCE_ROOT) != source_snapshot:
                    raise AssertionError("source tree changed during automatic rollback")

                print(
                    f"OK fod-indexer materialize rollback smoke source={SOURCE_ROOT} root={root_name} "
                    f"source_id={source_id} plan_id={plan_id}"
                )
    finally:
        try:
            remove_materialize_failure_trigger(dsn)
        except Exception:
            pass
        try:
            cleanup_materialized_roots(dsn)
        except Exception:
            pass
        shutil.rmtree(SOURCE_ROOT, ignore_errors=True)
        try:
            cleanup_indexer_state(dsn)
        except Exception:
            pass


if __name__ == "__main__":
    main()
