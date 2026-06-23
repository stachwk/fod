#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


import sys
import tempfile
import shutil
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
    fetch_all,
    fetch_one,
    run_indexer,
    snapshot_tree,
    wait_for_mount_children,
    write_tree,
)
from tests.integration.fod_mount import FODMount

SOURCE_ROOT = Path("/tmp/fod-indexer-src")
SOURCE_FILES: dict[str, bytes] = {
    "a.txt": b"same",
    "b.txt": b"same",
    "c.txt": b"unique",
    "empty.txt": b"",
}


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

        with tempfile.TemporaryDirectory(prefix="fod-indexer-materialize-") as mount_dir:
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
                        "smoke",
                        "--path",
                        str(SOURCE_ROOT),
                        "--kind",
                        "local",
                    ]
                )
                assert_contains(source_add_output, "Registered source smoke as local", "source add")
                assert_contains(source_add_output, str(SOURCE_ROOT), "source add")

                scan_output = run_indexer(ROOT, ["scan", "--source", "smoke"])
                assert_contains(scan_output, "FOD indexer scan", "scan")
                assert_contains(scan_output, "scanned files: 4", "scan")
                assert_contains(scan_output, "ok files: 4", "scan")
                assert_contains(scan_output, "total bytes: 14", "scan")

                hash_output = run_indexer(ROOT, ["hash", "--source", "smoke", "--candidates-only"])
                assert_contains(hash_output, "FOD indexer hash", "hash")
                assert_contains(hash_output, "source: smoke", "hash")
                assert_contains(hash_output, "duplicate sets: 1", "hash")

                report_output = run_indexer(ROOT, ["report", "duplicates"])
                assert_contains(report_output, "FOD indexer duplicate report", "duplicate report")
                assert_contains(report_output, "confirmed duplicate sets: 1", "duplicate report")
                assert_contains(report_output, "a.txt", "duplicate report")
                assert_contains(report_output, "b.txt", "duplicate report")

                dry_run_output = run_indexer(ROOT, ["plan-import", "--source", "smoke", "--dry-run"])
                assert_contains(dry_run_output, "FOD indexer dry-run import plan", "dry-run import plan")
                assert_contains(dry_run_output, "source: smoke", "dry-run import plan")
                assert_contains(dry_run_output, "scanned files: 4", "dry-run import plan")
                assert_contains(dry_run_output, "candidate duplicate groups: 1", "dry-run import plan")
                assert_contains(dry_run_output, "confirmed duplicate groups: 1", "dry-run import plan")
                assert_contains(dry_run_output, "unique payloads: 3", "dry-run import plan")
                assert_contains(dry_run_output, "saved bytes: 4", "dry-run import plan")

                materialize_output = run_indexer(ROOT, ["materialize", "--source", "smoke"])
                assert_contains(materialize_output, "FOD indexer materialize", "materialize")
                assert_contains(materialize_output, "scanned files: 4", "materialize")
                assert_contains(materialize_output, "validated files: 4", "materialize")
                assert_contains(materialize_output, "duplicate groups: 1", "materialize")
                assert_contains(materialize_output, "canonical files: 3", "materialize")
                assert_contains(materialize_output, "reference files: 1", "materialize")
                assert_contains(materialize_output, "source bytes: 14", "materialize")
                assert_contains(materialize_output, "imported bytes: 10", "materialize")
                assert_contains(materialize_output, "saved bytes: 4", "materialize")

                if snapshot_tree(SOURCE_ROOT) != source_snapshot:
                    raise AssertionError("source tree changed during materialize")

                with psycopg2.connect(**dsn) as conn:
                    with conn.cursor() as cur:
                        cur.execute("SET search_path TO fod, public")
                    source_id = int(
                        fetch_one(
                            conn,
                            "SELECT id_index_source FROM index_sources WHERE name = %s",
                            ("smoke",),
                        )
                    )
                    plan_id = int(
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
                    root_name = f"index-source-{source_id}-import-{plan_id}"
                    assert_contains(materialize_output, f"import root: /{root_name}", "materialize")

                    mount_root = mount.config.mountpoint / root_name  # type: ignore[union-attr]
                    wait_for_mount_children(mount_root, sorted(SOURCE_FILES))
                    if not mount_root.is_dir():
                        raise AssertionError(f"missing materialize root on mount: {mount_root}")

                    mount_names = sorted(child.name for child in mount_root.iterdir())
                    if mount_names != sorted(SOURCE_FILES):
                        raise AssertionError(
                            f"unexpected materialize root entries: expected={sorted(SOURCE_FILES)} actual={mount_names}"
                        )
                    for name, content in SOURCE_FILES.items():
                        if (mount_root / name).read_bytes() != content:
                            raise AssertionError(f"unexpected content for {name} in {mount_root}")

                    file_rows = fetch_all(
                        conn,
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
                        SELECT f.name, f.size, f.data_object_id, d.reference_count
                        FROM files f
                        JOIN data_objects d ON d.id_data_object = f.data_object_id
                        WHERE f.id_directory IN (SELECT id_directory FROM subtree)
                        ORDER BY f.name
                        """,
                        (root_name,),
                    )
                    rows_by_name = {
                        row[0]: {
                            "size": int(row[1]),
                            "data_object_id": int(row[2]),
                            "reference_count": int(row[3]),
                        }
                        for row in file_rows
                    }
                    if set(rows_by_name) != set(SOURCE_FILES):
                        raise AssertionError(
                            f"unexpected imported files: expected={sorted(SOURCE_FILES)} actual={sorted(rows_by_name)}"
                        )
                    if rows_by_name["a.txt"]["size"] != 4 or rows_by_name["b.txt"]["size"] != 4:
                        raise AssertionError("duplicate materialized sizes are wrong")
                    if rows_by_name["c.txt"]["size"] != 6:
                        raise AssertionError("unique materialized size is wrong")
                    if rows_by_name["empty.txt"]["size"] != 0:
                        raise AssertionError("zero-length file size is wrong")
                    if rows_by_name["a.txt"]["data_object_id"] != rows_by_name["b.txt"]["data_object_id"]:
                        raise AssertionError("duplicate files do not share a data object")
                    if rows_by_name["a.txt"]["reference_count"] != 2:
                        raise AssertionError("duplicate data object reference_count should be 2")
                    if rows_by_name["c.txt"]["data_object_id"] == rows_by_name["a.txt"]["data_object_id"]:
                        raise AssertionError("unique file unexpectedly reused duplicate data object")
                    if rows_by_name["c.txt"]["reference_count"] != 1:
                        raise AssertionError("unique file reference_count should be 1")
                    if rows_by_name["empty.txt"]["reference_count"] != 1:
                        raise AssertionError("zero-length file reference_count should be 1")

                    plan_rows = fetch_all(
                        conn,
                        """
                        SELECT f.path, e.action, e.canonical_file_id
                        FROM index_import_plan_entries e
                        JOIN index_files f ON f.id_file = e.id_file
                        WHERE e.id_import_plan = %s
                        ORDER BY f.path
                        """,
                        (plan_id,),
                    )
                    plan_rows_by_path = {
                        row[0]: {"action": row[1], "canonical_file_id": int(row[2])}
                        for row in plan_rows
                    }
                    expected_actions = {
                        "a.txt": "materialized_canonical",
                        "b.txt": "materialized_reference",
                        "c.txt": "materialized_unique",
                        "empty.txt": "materialized_unique",
                    }
                    if set(plan_rows_by_path) != set(expected_actions):
                        raise AssertionError(
                            f"unexpected materialize plan entries: expected={sorted(expected_actions)} actual={sorted(plan_rows_by_path)}"
                        )
                    for path, action in expected_actions.items():
                        if plan_rows_by_path[path]["action"] != action:
                            raise AssertionError(
                                f"unexpected action for {path}: expected={action} actual={plan_rows_by_path[path]['action']}"
                            )
                    if plan_rows_by_path["a.txt"]["canonical_file_id"] != plan_rows_by_path["b.txt"]["canonical_file_id"]:
                        raise AssertionError("duplicate plan entries do not point at the same canonical file")

                print(
                    f"OK fod-indexer materialize smoke source={SOURCE_ROOT} root={root_name} "
                    f"source_id={source_id} plan_id={plan_id}"
                )
    finally:
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
