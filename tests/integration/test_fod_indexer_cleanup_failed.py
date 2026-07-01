#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

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
    cleanup_indexer_sources,
    cleanup_materialized_roots_for_sources,
    cleanup_test_dir,
    fetch_one,
    prepare_clean_dir,
    run_indexer,
    snapshot_tree,
    unique_indexer_path,
    unique_source_name,
    wait_for_mount_children,
    write_tree,
)
from tests.integration.fod_mount import FODMount

SOURCE_FILES: dict[str, bytes] = {
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


def wait_for_path_present(path: Path, timeout_s: float = 10.0) -> None:
    deadline = time.monotonic() + timeout_s
    last_state = False
    while time.monotonic() < deadline:
        last_state = path.exists()
        if last_state:
            return
        time.sleep(0.2)
    raise AssertionError(f"timed out waiting for {path} to appear; last_exists={last_state}")


def cleanup_shared_reference(dsn: dict[str, str], file_id: int, data_object_id: int) -> None:
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        cur.execute("SET search_path TO fod, public")
        cur.execute("DELETE FROM data_blocks WHERE data_object_id = %s", (data_object_id,))
        cur.execute("DELETE FROM data_extents WHERE data_object_id = %s", (data_object_id,))
        cur.execute("DELETE FROM copy_block_crc WHERE data_object_id = %s", (data_object_id,))
        cur.execute("DELETE FROM files WHERE id_file = %s", (file_id,))
        cur.execute("DELETE FROM data_objects WHERE id_data_object = %s", (data_object_id,))


def main() -> None:
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    source_name = unique_source_name("cleanup_failed")
    source_root = prepare_clean_dir(unique_indexer_path("cleanup-src"))
    cleanup_indexer_sources(dsn, [source_name])

    source_snapshot = None
    root_name = None
    shared_file_id = None
    shared_data_object_id = None
    shared_file_name = None
    try:
        write_tree(source_root, SOURCE_FILES)
        source_snapshot = snapshot_tree(source_root)

        with tempfile.TemporaryDirectory(prefix="fod-indexer-cleanup-") as mount_dir:
            with FODMount(str(ROOT)) as mount:
                mount.init_schema()
                cleanup_indexer_sources(dsn, [source_name])
                mount.start(mount_dir)
                cleanup_materialized_roots_for_sources(dsn, [source_name])

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

                scan_output = run_indexer(ROOT, ["scan", "--source", source_name])
                assert_contains(scan_output, "scanned files: 3", "scan")
                assert_contains(scan_output, "ok files: 3", "scan")

                hash_output = run_indexer(ROOT, ["hash", "--source", source_name, "--candidates-only"])
                assert_contains(hash_output, "FOD indexer hash", "hash")
                assert_contains(hash_output, f"source: {source_name}", "hash")

                materialize_output = run_indexer(ROOT, ["materialize", "--source", source_name])
                assert_contains(materialize_output, "FOD indexer materialize", "materialize")

                if snapshot_tree(source_root) != source_snapshot:
                    raise AssertionError("source tree changed during materialize")

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
                            (source_name,),
                        )
                    )
                    root_name = f"index-source-{source_id}-import-{plan_id}"
                    mount_root = mount.config.mountpoint / root_name  # type: ignore[union-attr]
                    wait_for_mount_children(mount_root, sorted(SOURCE_FILES))
                    if not mount_root.is_dir():
                        raise AssertionError(f"missing materialize root on mount: {mount_root}")

                    canonical_file_row = fetch_one(
                        conn,
                        """
                        SELECT id_file, data_object_id, size, mode, uid, gid
                        FROM files
                        WHERE name = %s
                          AND id_directory = (
                              SELECT id_directory
                              FROM directories
                              WHERE id_parent IS NULL AND name = %s
                          )
                        """,
                        ("a.txt", root_name),
                    )
                    if not isinstance(canonical_file_row, tuple) or len(canonical_file_row) < 6:
                        raise AssertionError(f"unexpected canonical file row: {canonical_file_row}")

                    canonical_file_id = int(canonical_file_row[0])
                    canonical_data_object_id = int(canonical_file_row[1])
                    shared_file_name = f"shared-outside-{plan_id}"
                    shared_path = mount.config.mountpoint / shared_file_name  # type: ignore[union-attr]
                    shared_path.write_bytes(SOURCE_FILES["a.txt"])
                    wait_for_path_present(shared_path)
                    if not shared_path.is_file():
                        raise AssertionError(f"missing shared file on mount: {shared_path}")
                    if shared_path.read_bytes() != SOURCE_FILES["a.txt"]:
                        raise AssertionError("shared file does not expose the canonical payload")

                    shared_file_row = fetch_one(
                        conn,
                        """
                        SELECT id_file, data_object_id
                        FROM files
                        WHERE name = %s AND id_directory IS NULL
                        """,
                        (shared_file_name,),
                    )
                    if not isinstance(shared_file_row, tuple) or len(shared_file_row) < 2:
                        raise AssertionError(f"unexpected shared file row: {shared_file_row}")
                    shared_file_id = int(shared_file_row[0])
                    shared_data_object_id = int(shared_file_row[1])

                    if shared_data_object_id != canonical_data_object_id:
                        with conn.cursor() as cur:
                            cur.execute(
                                """
                                UPDATE files
                                SET data_object_id = %s
                                WHERE id_file = %s
                                """,
                                (canonical_data_object_id, shared_file_id),
                            )
                            cur.execute(
                                """
                                UPDATE data_objects
                                SET reference_count = reference_count + 1,
                                    modification_date = NOW()
                                WHERE id_data_object = %s
                                """,
                                (canonical_data_object_id,),
                            )
                            cur.execute(
                                "DELETE FROM data_blocks WHERE data_object_id = %s",
                                (shared_data_object_id,),
                            )
                            cur.execute(
                                "DELETE FROM data_extents WHERE data_object_id = %s",
                                (shared_data_object_id,),
                            )
                            cur.execute(
                                "DELETE FROM copy_block_crc WHERE data_object_id = %s",
                                (shared_data_object_id,),
                            )
                            cur.execute(
                                "DELETE FROM data_objects WHERE id_data_object = %s",
                                (shared_data_object_id,),
                            )
                        shared_data_object_id = canonical_data_object_id

                    with conn.cursor() as cur:
                        cur.execute(
                            "UPDATE index_import_plans SET status = %s, updated_at = NOW() WHERE id_import_plan = %s",
                            ("materialize_failed", plan_id),
                        )

                cleanup_output = run_indexer(ROOT, ["cleanup-failed", "--plan", str(plan_id)])
                assert_contains(cleanup_output, "FOD indexer cleanup failed materialization", "cleanup")
                assert_contains(cleanup_output, f"plan id: {plan_id}", "cleanup")
                assert_contains(cleanup_output, f"source: {source_name}", "cleanup")
                assert_contains(cleanup_output, f"import root: /{root_name}", "cleanup")
                assert_contains(cleanup_output, "files removed: 3", "cleanup")
                assert_contains(cleanup_output, "directories removed: 1", "cleanup")
                assert_contains(cleanup_output, "exclusive data objects removed: 1", "cleanup")
                assert_contains(cleanup_output, "shared data objects preserved: 1", "cleanup")
                assert_contains(cleanup_output, "skipping shared data object during failed import cleanup", "cleanup")
                assert_contains(cleanup_output, "plan status: materialize_failed -> materialize_cleaned", "cleanup")

                wait_for_path_missing(mount_root)
                shared_path = mount.config.mountpoint / shared_file_name  # type: ignore[union-attr]
                wait_for_path_present(shared_path)
                if not shared_path.is_file():
                    raise AssertionError(f"missing shared file after cleanup: {shared_path}")
                if shared_path.read_bytes() != SOURCE_FILES["a.txt"]:
                    raise AssertionError("shared file changed during cleanup")
                if snapshot_tree(source_root) != source_snapshot:
                    raise AssertionError("source tree changed during cleanup")

                with psycopg2.connect(**dsn) as conn:
                    with conn.cursor() as cur:
                        cur.execute("SET search_path TO fod, public")
                        cur.execute(
                            "SELECT status FROM index_import_plans WHERE id_import_plan = %s",
                            (plan_id,),
                        )
                        row = cur.fetchone()
                        if row is None or row[0] != "materialize_cleaned":
                            raise AssertionError(f"unexpected cleanup status: {row}")
                        cur.execute(
                            "SELECT COUNT(*) FROM directories WHERE id_parent IS NULL AND name = %s",
                            (root_name,),
                        )
                        if cur.fetchone()[0] != 0:
                            raise AssertionError("cleanup root still exists in directories")
                        cur.execute(
                            "SELECT reference_count FROM data_objects WHERE id_data_object = %s",
                            (shared_data_object_id,),
                        )
                        row = cur.fetchone()
                        if row is None or int(row[0]) != 1:
                            raise AssertionError(f"shared data object was not preserved correctly: {row}")
                        cur.execute(
                            "SELECT COUNT(*) FROM files WHERE id_file = %s",
                            (shared_file_id,),
                        )
                        if cur.fetchone()[0] != 1:
                            raise AssertionError("shared file disappeared during cleanup")
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
                        if cur.fetchone()[0] != 0:
                            raise AssertionError("cleanup root still has files")

                print(f"OK fod-indexer cleanup-failed plan={plan_id} root={root_name}")
    finally:
        if shared_file_id is not None and shared_data_object_id is not None:
            try:
                cleanup_shared_reference(dsn, shared_file_id, shared_data_object_id)
            except Exception:
                pass
        try:
            cleanup_materialized_roots_for_sources(dsn, [source_name])
        except Exception:
            pass
        cleanup_test_dir(source_root)
        try:
            cleanup_indexer_sources(dsn, [source_name])
        except Exception:
            pass


if __name__ == "__main__":
    main()
