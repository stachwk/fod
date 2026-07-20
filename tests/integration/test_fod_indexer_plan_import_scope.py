#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import sys
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from fod_backend import load_dsn_from_config
from tests.integration.fod_indexer_testlib import (
    apply_database_env,
    assert_contains,
    cleanup_indexer_sources,
    cleanup_test_dir,
    fetch_all,
    fetch_one,
    prepare_clean_dir,
    run_indexer,
    snapshot_tree,
    unique_indexer_path,
    unique_source_name,
    write_tree,
)
from tests.integration.fod_mount import FODMount
from tests.integration.test_fod_indexer_catalog_snapshot import (
    run_catalog_snapshot_regression,
)

SMOKE_FILES: dict[str, bytes] = {
    "a.txt": b"same",
    "b.txt": b"same",
    "c.txt": b"unique",
}

OTHER_FILES: dict[str, bytes] = {
    "d.txt": b"x",
    "e.txt": b"yy",
}


def plan_entries(conn, plan_id: int) -> list[tuple[str, str, str]]:
    return fetch_all(
        conn,
        """
        SELECT s.name, f.path, e.action
        FROM index_import_plan_entries e
        JOIN index_files f ON f.id_file = e.id_file
        JOIN index_sources s ON s.id_index_source = f.id_index_source
        WHERE e.id_import_plan = %s
        ORDER BY s.name, f.path
        """,
        (plan_id,),
    )  # type: ignore[return-value]


def assert_plan_scope(
    conn,
    plan_id: int,
    expected_scope: str | None,
    expected_entries: list[tuple[str, str, str]],
) -> None:
    with conn.cursor() as cur:
        cur.execute(
            "SELECT source_filter FROM index_import_plans WHERE id_import_plan = %s",
            (plan_id,),
        )
        row = cur.fetchone()
        if row is None:
            raise AssertionError(f"missing import plan {plan_id}")
        actual_scope = row[0]
    if actual_scope != expected_scope:
        raise AssertionError(
            f"unexpected source scope for plan {plan_id}: expected={expected_scope!r} actual={actual_scope!r}"
        )

    entries = plan_entries(conn, plan_id)
    if entries != expected_entries:
        raise AssertionError(
            f"unexpected plan entries for plan {plan_id}: expected={expected_entries} actual={entries}"
        )


def assert_plan_contains_entries(
    conn,
    plan_id: int,
    expected_scope: str | None,
    expected_entries: list[tuple[str, str, str]],
) -> None:
    with conn.cursor() as cur:
        cur.execute(
            "SELECT source_filter FROM index_import_plans WHERE id_import_plan = %s",
            (plan_id,),
        )
        row = cur.fetchone()
        if row is None:
            raise AssertionError(f"missing import plan {plan_id}")
        actual_scope = row[0]
    if actual_scope != expected_scope:
        raise AssertionError(
            f"unexpected source scope for plan {plan_id}: expected={expected_scope!r} actual={actual_scope!r}"
        )

    entries = plan_entries(conn, plan_id)
    missing = [entry for entry in expected_entries if entry not in entries]
    if missing:
        raise AssertionError(
            f"missing expected plan entries for plan {plan_id}: missing={missing} actual={entries}"
        )


def latest_all_sources_plan_for_sources(conn, source_names: list[str]) -> int:
    conditions = "\n".join(
        f"""
        AND EXISTS (
            SELECT 1
            FROM index_import_plan_entries e{idx}
            JOIN index_files f{idx} ON f{idx}.id_file = e{idx}.id_file
            JOIN index_sources s{idx} ON s{idx}.id_index_source = f{idx}.id_index_source
            WHERE e{idx}.id_import_plan = p.id_import_plan
              AND s{idx}.name = %s
        )
        """
        for idx, _ in enumerate(source_names)
    )
    return int(
        fetch_one(
            conn,
            f"""
            SELECT p.id_import_plan
            FROM index_import_plans p
            WHERE p.source_filter IS NULL
            {conditions}
            ORDER BY p.id_import_plan DESC
            LIMIT 1
            """,
            tuple(source_names),
        )
    )


def main() -> None:
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    smoke_source = unique_source_name("plan_import_scope_smoke")
    other_source = unique_source_name("plan_import_scope_other")
    source_names = [smoke_source, other_source]
    smoke_root = prepare_clean_dir(unique_indexer_path("plan-import-smoke"))
    other_root = prepare_clean_dir(unique_indexer_path("plan-import-other"))
    cleanup_indexer_sources(dsn, source_names)

    smoke_snapshot = None
    other_snapshot = None
    try:
        write_tree(smoke_root, SMOKE_FILES)
        write_tree(other_root, OTHER_FILES)
        smoke_snapshot = snapshot_tree(smoke_root)
        other_snapshot = snapshot_tree(other_root)

        with FODMount(str(ROOT)) as mount:
            mount.init_schema()
            cleanup_indexer_sources(dsn, source_names)

            smoke_add_output = run_indexer(
                ROOT,
                [
                    "source",
                    "add",
                    "--name",
                    smoke_source,
                    "--path",
                    str(smoke_root),
                    "--kind",
                    "local",
                ],
            )
            assert_contains(smoke_add_output, f"Registered source {smoke_source} as local", "source add smoke")
            assert_contains(smoke_add_output, str(smoke_root), "source add smoke")

            other_add_output = run_indexer(
                ROOT,
                [
                    "source",
                    "add",
                    "--name",
                    other_source,
                    "--path",
                    str(other_root),
                    "--kind",
                    "local",
                ],
            )
            assert_contains(other_add_output, f"Registered source {other_source} as local", "source add other")
            assert_contains(other_add_output, str(other_root), "source add other")

            smoke_scan_output = run_indexer(ROOT, ["scan", "--source", smoke_source])
            assert_contains(smoke_scan_output, "scanned files: 3", "scan smoke")
            assert_contains(smoke_scan_output, "ok files: 3", "scan smoke")
            assert_contains(smoke_scan_output, "total bytes: 14", "scan smoke")

            other_scan_output = run_indexer(ROOT, ["scan", "--source", other_source])
            assert_contains(other_scan_output, "scanned files: 2", "scan other")
            assert_contains(other_scan_output, "ok files: 2", "scan other")
            assert_contains(other_scan_output, "total bytes: 3", "scan other")

            smoke_hash_output = run_indexer(ROOT, ["hash", "--source", smoke_source, "--candidates-only"])
            assert_contains(smoke_hash_output, f"source: {smoke_source}", "hash smoke")

            other_hash_output = run_indexer(ROOT, ["hash", "--source", other_source, "--candidates-only"])
            assert_contains(other_hash_output, "candidate files: 0", "hash other")
            assert_contains(other_hash_output, f"source: {other_source}", "hash other")

            smoke_dry_run_output = run_indexer(ROOT, ["plan-import", "--source", smoke_source, "--dry-run"])
            assert_contains(smoke_dry_run_output, "FOD indexer dry-run import plan", "dry-run smoke")
            assert_contains(smoke_dry_run_output, f"source: {smoke_source}", "dry-run smoke")
            assert_contains(smoke_dry_run_output, "scanned files: 3", "dry-run smoke")
            assert_contains(smoke_dry_run_output, "candidate duplicate groups: 1", "dry-run smoke")
            assert_contains(smoke_dry_run_output, "confirmed duplicate groups: 1", "dry-run smoke")
            assert_contains(smoke_dry_run_output, "unique payloads: 2", "dry-run smoke")
            assert_contains(smoke_dry_run_output, "source bytes: 14", "dry-run smoke")
            assert_contains(smoke_dry_run_output, "estimated import bytes: 10", "dry-run smoke")
            assert_contains(smoke_dry_run_output, "estimated saved bytes: 4", "dry-run smoke")

            all_dry_run_output = run_indexer(ROOT, ["plan-import", "--all-sources", "--dry-run"])
            assert_contains(all_dry_run_output, "FOD indexer dry-run import plan", "dry-run all")
            assert_contains(all_dry_run_output, "source: all sources", "dry-run all")

            if snapshot_tree(smoke_root) != smoke_snapshot:
                raise AssertionError("smoke source tree changed during plan-import")
            if snapshot_tree(other_root) != other_snapshot:
                raise AssertionError("other source tree changed during plan-import")

            with psycopg2.connect(**dsn) as conn:
                with conn.cursor() as cur:
                    cur.execute("SET search_path TO fod, public")

                smoke_plan_id = int(
                    fetch_one(
                        conn,
                        """
                        SELECT id_import_plan
                        FROM index_import_plans
                        WHERE source_filter = %s
                        ORDER BY id_import_plan DESC
                        LIMIT 1
                        """,
                        (smoke_source,),
                    )
                )
                all_plan_id = latest_all_sources_plan_for_sources(conn, source_names)

                assert_plan_scope(
                    conn,
                    smoke_plan_id,
                    smoke_source,
                    [
                        (smoke_source, "a.txt", "canonical"),
                        (smoke_source, "b.txt", "reference"),
                        (smoke_source, "c.txt", "unique"),
                    ],
                )
                assert_plan_contains_entries(
                    conn,
                    all_plan_id,
                    None,
                    [
                        (other_source, "d.txt", "unique"),
                        (other_source, "e.txt", "unique"),
                        (smoke_source, "a.txt", "canonical"),
                        (smoke_source, "b.txt", "reference"),
                        (smoke_source, "c.txt", "unique"),
                    ],
                )

            print(
                f"OK fod-indexer plan-import scope smoke_plan={smoke_plan_id} all_plan={all_plan_id}"
            )
    finally:
        cleanup_test_dir(smoke_root)
        cleanup_test_dir(other_root)
        try:
            cleanup_indexer_sources(dsn, source_names)
        except Exception:
            pass


if __name__ == "__main__":
    main()
    run_catalog_snapshot_regression()
