#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import os
import shlex
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable
from pathlib import Path

import psycopg2
from psycopg2 import sql

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from fod_backend import load_dsn_from_config
from tests.integration.fod_indexer_testlib import (
    apply_database_env,
    assert_contains,
    cleanup_indexer_sources,
    cleanup_materialized_roots_for_sources,
    cleanup_test_dir,
    fetch_one,
    prepare_clean_dir,
    run_indexer,
    snapshot_tree,
    unique_indexer_path,
    unique_source_name,
    write_tree,
)
from tests.integration.fod_mount import FODMount

SOURCE_FILES_PLAN_ENTRY_FAILURE: dict[str, bytes] = {
    "rollback.txt": b"rollback me\n",
}

SOURCE_FILES_COMPLETED_FAILURE: dict[str, bytes] = {
    "a.txt": b"same",
    "b.txt": b"same",
    "c.txt": b"unique",
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


def sql_suffix(value: str) -> str:
    safe = "".join(ch if ch.isalnum() else "_" for ch in value)
    return safe[-40:]


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


def install_materialize_plan_entry_failure_trigger(dsn: dict[str, str], source_name: str) -> None:
    suffix = sql_suffix(source_name)
    function_name = f"fod_force_plan_entry_failure_{suffix}"
    trigger_name = f"fod_force_plan_entry_failure_{suffix}"
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        cur.execute("SET search_path TO fod, public")
        cur.execute(
            sql.SQL(
                """
            CREATE OR REPLACE FUNCTION {}()
            RETURNS trigger AS $$
            BEGIN
                IF EXISTS (
                    SELECT 1
                    FROM index_import_plans
                    WHERE id_import_plan = NEW.id_import_plan
                      AND source_filter = {}
                ) THEN
                    RAISE EXCEPTION 'forced materialize failure for rollback smoke';
                END IF;
                RETURN NEW;
            END;
            $$ LANGUAGE plpgsql
            """
            ).format(sql.Identifier(function_name), sql.Literal(source_name))
        )
        cur.execute(
            sql.SQL("DROP TRIGGER IF EXISTS {} ON index_import_plan_entries").format(
                sql.Identifier(trigger_name)
            )
        )
        cur.execute(
            sql.SQL(
                """
            CREATE TRIGGER {}
            BEFORE INSERT ON index_import_plan_entries
            FOR EACH ROW
            EXECUTE FUNCTION {}()
            """
            ).format(sql.Identifier(trigger_name), sql.Identifier(function_name))
        )


def install_materialize_completed_failure_trigger(dsn: dict[str, str], source_name: str) -> None:
    suffix = sql_suffix(source_name)
    function_name = f"fod_force_completed_failure_{suffix}"
    trigger_name = f"fod_force_completed_failure_{suffix}"
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        cur.execute("SET search_path TO fod, public")
        cur.execute(
            sql.SQL(
                """
            CREATE OR REPLACE FUNCTION {}()
            RETURNS trigger AS $$
            BEGIN
                IF NEW.status = 'materialize_completed' AND NEW.source_filter = {} THEN
                    RAISE EXCEPTION 'forced materialize completed failure for rollback smoke';
                END IF;
                RETURN NEW;
            END;
            $$ LANGUAGE plpgsql
            """
            ).format(sql.Identifier(function_name), sql.Literal(source_name))
        )
        cur.execute(
            sql.SQL("DROP TRIGGER IF EXISTS {} ON index_import_plans").format(
                sql.Identifier(trigger_name)
            )
        )
        cur.execute(
            sql.SQL(
                """
            CREATE TRIGGER {}
            BEFORE UPDATE ON index_import_plans
            FOR EACH ROW
            EXECUTE FUNCTION {}()
            """
            ).format(sql.Identifier(trigger_name), sql.Identifier(function_name))
        )


def remove_materialize_failure_triggers(dsn: dict[str, str], source_name: str) -> None:
    suffix = sql_suffix(source_name)
    names = [
        (
            "index_import_plan_entries",
            f"fod_force_plan_entry_failure_{suffix}",
            f"fod_force_plan_entry_failure_{suffix}",
        ),
        (
            "index_import_plans",
            f"fod_force_completed_failure_{suffix}",
            f"fod_force_completed_failure_{suffix}",
        ),
    ]
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        cur.execute("SET search_path TO fod, public")
        for table_name, trigger_name, function_name in names:
            cur.execute(
                sql.SQL("DROP TRIGGER IF EXISTS {} ON {}").format(
                    sql.Identifier(trigger_name), sql.Identifier(table_name)
                )
            )
            cur.execute(
                sql.SQL("DROP FUNCTION IF EXISTS {}()").format(sql.Identifier(function_name))
            )


def run_materialize_rollback_case(
    dsn: dict[str, str],
    mount: FODMount,
    source_root: Path,
    source_files: dict[str, bytes],
    source_name: str,
    expected_plan_entry_count: int,
    install_failure_trigger: Callable[[dict[str, str], str], None],
    failure_label: str,
) -> None:
    source_snapshot = None
    root_name = None
    try:
        write_tree(source_root, source_files)
        source_snapshot = snapshot_tree(source_root)

        source_add_output = run_indexer(
            ROOT,
            [
                "source",
                "add",
                "--name",
                source_name,
                "--path",
                str(source_root),
                "--kind",
                "local",
            ],
        )
        assert_contains(source_add_output, f"Registered source {source_name} as local", "source add")
        assert_contains(source_add_output, str(source_root), "source add")

        install_failure_trigger(dsn, source_name)

        materialize_result = run_indexer_result(
            ROOT,
            ["materialize", "--source", source_name],
        )
        if materialize_result.returncode == 0:
            raise AssertionError("materialize unexpectedly succeeded under forced failure")

        materialize_output = materialize_result.stdout + materialize_result.stderr
        assert_contains(
            materialize_output,
            failure_label,
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
                    (source_name,),
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
                (source_name,),
            )
            if not isinstance(plan_row, tuple) or len(plan_row) < 2:
                raise AssertionError(f"unexpected plan row: {plan_row}")
            plan_id = int(plan_row[0])
            plan_status = str(plan_row[1])
            if plan_status != "materialize_cleaned":
                raise AssertionError(f"unexpected plan status after automatic rollback: {plan_status}")

            root_name = f"index-source-{source_id}-import-{plan_id}"
            assert_contains(materialize_output, f"plan {plan_id}", "materialize failure")

            with conn.cursor() as cur:
                cur.execute(
                    "SELECT COUNT(*) FROM index_import_plan_entries WHERE id_import_plan = %s",
                    (plan_id,),
                )
                plan_entry_count = int(cur.fetchone()[0])
            if plan_entry_count != expected_plan_entry_count:
                raise AssertionError(
                    f"unexpected plan entry count after rollback: expected={expected_plan_entry_count} actual={plan_entry_count}"
                )

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

        if snapshot_tree(source_root) != source_snapshot:
            raise AssertionError("source tree changed during automatic rollback")

        print(
            f"OK fod-indexer materialize rollback smoke source={source_root} root={root_name} "
            f"source_id={source_id} plan_id={plan_id} plan_entries={expected_plan_entry_count}"
        )
    finally:
        try:
            remove_materialize_failure_triggers(dsn, source_name)
        except Exception:
            pass
        try:
            cleanup_materialized_roots_for_sources(dsn, [source_name])
        except Exception:
            pass
        try:
            cleanup_indexer_sources(dsn, [source_name])
        except Exception:
            pass
        cleanup_test_dir(source_root)


def main() -> None:
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    early_source_name = unique_source_name("rollback_early")
    late_source_name = unique_source_name("rollback_late")
    source_names = [early_source_name, late_source_name]
    early_source_root = prepare_clean_dir(unique_indexer_path("materialize-rollback-early-src"))
    late_source_root = prepare_clean_dir(unique_indexer_path("materialize-rollback-late-src"))
    cleanup_indexer_sources(dsn, source_names)

    try:
        with tempfile.TemporaryDirectory(prefix="fod-indexer-materialize-rollback-") as mount_dir:
            with FODMount(str(ROOT)) as mount:
                mount.init_schema()
                cleanup_indexer_sources(dsn, source_names)
                mount.start(mount_dir)
                cleanup_materialized_roots_for_sources(dsn, source_names)
                run_materialize_rollback_case(
                    dsn,
                    mount,
                    early_source_root,
                    SOURCE_FILES_PLAN_ENTRY_FAILURE,
                    early_source_name,
                    0,
                    install_materialize_plan_entry_failure_trigger,
                    "forced materialize failure for rollback smoke",
                )
                run_materialize_rollback_case(
                    dsn,
                    mount,
                    late_source_root,
                    SOURCE_FILES_COMPLETED_FAILURE,
                    late_source_name,
                    3,
                    install_materialize_completed_failure_trigger,
                    "forced materialize completed failure for rollback smoke",
                )
    finally:
        for source_name in source_names:
            try:
                remove_materialize_failure_triggers(dsn, source_name)
            except Exception:
                pass
        try:
            cleanup_materialized_roots_for_sources(dsn, source_names)
        except Exception:
            pass
        cleanup_test_dir(early_source_root)
        cleanup_test_dir(late_source_root)
        try:
            cleanup_indexer_sources(dsn, source_names)
        except Exception:
            pass


if __name__ == "__main__":
    main()
