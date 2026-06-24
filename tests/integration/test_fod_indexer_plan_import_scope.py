#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import shutil
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
    cleanup_indexer_state,
    fetch_all,
    fetch_one,
    run_indexer,
    snapshot_tree,
    write_tree,
)
from tests.integration.fod_mount import FODMount

SMOKE_ROOT = Path("/tmp/fod-indexer-plan-import-smoke")
OTHER_ROOT = Path("/tmp/fod-indexer-plan-import-other")

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


def main() -> None:
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    cleanup_indexer_state(dsn)

    smoke_snapshot = None
    other_snapshot = None
    try:
        write_tree(SMOKE_ROOT, SMOKE_FILES)
        write_tree(OTHER_ROOT, OTHER_FILES)
        smoke_snapshot = snapshot_tree(SMOKE_ROOT)
        other_snapshot = snapshot_tree(OTHER_ROOT)

        with FODMount(str(ROOT)) as mount:
            mount.init_schema()
            cleanup_indexer_state(dsn)

            smoke_add_output = run_indexer(
                ROOT,
                [
                    "source",
                    "add",
                    "--name",
                    "smoke",
                    "--path",
                    str(SMOKE_ROOT),
                    "--kind",
                    "local",
                ],
            )
            assert_contains(smoke_add_output, "Registered source smoke as local", "source add smoke")
            assert_contains(smoke_add_output, str(SMOKE_ROOT), "source add smoke")

            other_add_output = run_indexer(
                ROOT,
                [
                    "source",
                    "add",
                    "--name",
                    "other",
                    "--path",
                    str(OTHER_ROOT),
                    "--kind",
                    "local",
                ],
            )
            assert_contains(other_add_output, "Registered source other as local", "source add other")
            assert_contains(other_add_output, str(OTHER_ROOT), "source add other")

            smoke_scan_output = run_indexer(ROOT, ["scan", "--source", "smoke"])
            assert_contains(smoke_scan_output, "scanned files: 3", "scan smoke")
            assert_contains(smoke_scan_output, "ok files: 3", "scan smoke")
            assert_contains(smoke_scan_output, "total bytes: 14", "scan smoke")

            other_scan_output = run_indexer(ROOT, ["scan", "--source", "other"])
            assert_contains(other_scan_output, "scanned files: 2", "scan other")
            assert_contains(other_scan_output, "ok files: 2", "scan other")
            assert_contains(other_scan_output, "total bytes: 3", "scan other")

            smoke_hash_output = run_indexer(ROOT, ["hash", "--source", "smoke", "--candidates-only"])
            assert_contains(smoke_hash_output, "duplicate sets: 1", "hash smoke")

            other_hash_output = run_indexer(ROOT, ["hash", "--source", "other", "--candidates-only"])
            assert_contains(other_hash_output, "candidate files: 0", "hash other")
            assert_contains(other_hash_output, "duplicate sets: 1", "hash other")

            smoke_dry_run_output = run_indexer(ROOT, ["plan-import", "--source", "smoke", "--dry-run"])
            assert_contains(smoke_dry_run_output, "FOD indexer dry-run import plan", "dry-run smoke")
            assert_contains(smoke_dry_run_output, "source: smoke", "dry-run smoke")
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
            assert_contains(all_dry_run_output, "scanned files: 5", "dry-run all")
            assert_contains(all_dry_run_output, "candidate duplicate groups: 1", "dry-run all")
            assert_contains(all_dry_run_output, "confirmed duplicate groups: 1", "dry-run all")
            assert_contains(all_dry_run_output, "unique payloads: 4", "dry-run all")
            assert_contains(all_dry_run_output, "source bytes: 17", "dry-run all")
            assert_contains(all_dry_run_output, "estimated import bytes: 13", "dry-run all")
            assert_contains(all_dry_run_output, "estimated saved bytes: 4", "dry-run all")

            if snapshot_tree(SMOKE_ROOT) != smoke_snapshot:
                raise AssertionError("smoke source tree changed during plan-import")
            if snapshot_tree(OTHER_ROOT) != other_snapshot:
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
                        ("smoke",),
                    )
                )
                all_plan_id = int(
                    fetch_one(
                        conn,
                        """
                        SELECT id_import_plan
                        FROM index_import_plans
                        WHERE source_filter IS NULL
                        ORDER BY id_import_plan DESC
                        LIMIT 1
                        """,
                    )
                )

                assert_plan_scope(
                    conn,
                    smoke_plan_id,
                    "smoke",
                    [
                        ("smoke", "a.txt", "canonical"),
                        ("smoke", "b.txt", "reference"),
                        ("smoke", "c.txt", "unique"),
                    ],
                )
                assert_plan_scope(
                    conn,
                    all_plan_id,
                    None,
                    [
                        ("other", "d.txt", "unique"),
                        ("other", "e.txt", "unique"),
                        ("smoke", "a.txt", "canonical"),
                        ("smoke", "b.txt", "reference"),
                        ("smoke", "c.txt", "unique"),
                    ],
                )

            print(
                f"OK fod-indexer plan-import scope smoke_plan={smoke_plan_id} all_plan={all_plan_id}"
            )
    finally:
        shutil.rmtree(SMOKE_ROOT, ignore_errors=True)
        shutil.rmtree(OTHER_ROOT, ignore_errors=True)
        try:
            cleanup_indexer_state(dsn)
        except Exception:
            pass


if __name__ == "__main__":
    main()
