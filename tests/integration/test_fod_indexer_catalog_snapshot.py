#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
from pathlib import Path

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
from tests.integration.fod_mount import FODMount

INITIAL_FILES: dict[str, bytes] = {
    "a.txt": b"same",
    "b.txt": b"same",
    "c.txt": b"unique",
}

LIVE_FILES: dict[str, bytes] = {
    "a.txt": b"changed-live",
    "c.txt": b"unique",
    "d.txt": b"new",
}


def run_indexer_result(args: list[str]) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env.pop("DATABASE_URL", None)
    env.pop("FOD_INDEXER_CONNINFO", None)
    env["INDEXER_ARGS"] = shlex.join(args)
    return subprocess.run(
        ["make", "--no-print-directory", "indexer"],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


def run_indexer_json(args: list[str]) -> dict[str, object]:
    result = run_indexer_result(args)
    if result.returncode != 0:
        raise AssertionError(
            f"fod-indexer command failed: {' '.join(args)}\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )
    try:
        payload = json.loads(result.stdout.strip())
    except json.JSONDecodeError as err:
        raise AssertionError(
            f"unable to parse JSON output for {' '.join(args)}: {err}\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        ) from err
    if not isinstance(payload, dict):
        raise AssertionError(f"unexpected JSON payload for {' '.join(args)}: {payload!r}")
    return payload


def file_items(payload: dict[str, object], label: str) -> dict[str, dict[str, object]]:
    items = payload.get("items")
    if not isinstance(items, list):
        raise AssertionError(f"missing items in {label}: {payload}")
    by_path: dict[str, dict[str, object]] = {}
    for item in items:
        if not isinstance(item, dict):
            raise AssertionError(f"invalid item in {label}: {item!r}")
        path = item.get("path")
        if not isinstance(path, str):
            raise AssertionError(f"missing path in {label}: {item}")
        by_path[path] = item
    return by_path


def assert_catalog(
    payload: dict[str, object],
    expected_paths: set[str],
    expected_sizes: dict[str, int],
    label: str,
) -> dict[str, dict[str, object]]:
    items = file_items(payload, label)
    if set(items) != expected_paths:
        raise AssertionError(
            f"unexpected paths in {label}: expected={sorted(expected_paths)} actual={sorted(items)}"
        )
    total = payload.get("total")
    if total != len(expected_paths):
        raise AssertionError(f"unexpected total in {label}: expected={len(expected_paths)} actual={total}")
    for path, expected_size in expected_sizes.items():
        actual_size = items[path].get("size")
        if actual_size != expected_size:
            raise AssertionError(
                f"unexpected size for {path} in {label}: expected={expected_size} actual={actual_size}"
            )
    return items


def run_catalog_snapshot_regression() -> None:
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    source_name = unique_source_name("catalog_snapshot")
    source_root = prepare_clean_dir(unique_indexer_path("catalog-snapshot"))
    snapshot_id: int | None = None
    cleanup_indexer_sources(dsn, [source_name])

    try:
        write_tree(source_root, INITIAL_FILES)

        with FODMount(str(ROOT)) as mount:
            mount.init_schema()
            cleanup_indexer_sources(dsn, [source_name])

            run_indexer_json(
                [
                    "--output",
                    "json",
                    "source",
                    "add",
                    "--name",
                    source_name,
                    "--path",
                    str(source_root),
                    "--kind",
                    "local",
                ]
            )

            scan_payload = run_indexer_json(
                ["--output", "json", "scan", "--source", source_name]
            )
            if scan_payload.get("scanned_files") != 3 or scan_payload.get("total_bytes") != 14:
                raise AssertionError(f"unexpected initial scan payload: {scan_payload}")

            run_indexer_json(
                [
                    "--output",
                    "json",
                    "hash",
                    "--source",
                    source_name,
                    "--candidates-only",
                ]
            )

            create_payload = run_indexer_json(
                [
                    "--output",
                    "json",
                    "snapshot",
                    "create",
                    "--source",
                    source_name,
                ]
            )
            snapshot = create_payload.get("snapshot")
            if not isinstance(snapshot, dict):
                raise AssertionError(f"unexpected snapshot create payload: {create_payload}")
            raw_snapshot_id = snapshot.get("snapshot_id")
            if not isinstance(raw_snapshot_id, int):
                raise AssertionError(f"missing snapshot id: {create_payload}")
            snapshot_id = raw_snapshot_id
            if snapshot.get("status") != "complete":
                raise AssertionError(f"snapshot is not complete: {create_payload}")
            if snapshot.get("source_filter") != source_name:
                raise AssertionError(f"unexpected snapshot source filter: {create_payload}")
            if snapshot.get("file_count") != 3 or snapshot.get("total_bytes") != 14:
                raise AssertionError(f"unexpected snapshot totals: {create_payload}")

            list_payload = run_indexer_json(
                ["--output", "json", "snapshot", "list", "--limit", "100"]
            )
            listed = list_payload.get("items")
            if not isinstance(listed, list) or not any(
                isinstance(item, dict) and item.get("snapshot_id") == snapshot_id
                for item in listed
            ):
                raise AssertionError(f"snapshot list is missing {snapshot_id}: {list_payload}")

            show_payload = run_indexer_json(
                [
                    "--output",
                    "json",
                    "snapshot",
                    "show",
                    "--id",
                    str(snapshot_id),
                ]
            )
            shown = show_payload.get("snapshot")
            if not isinstance(shown, dict) or shown.get("snapshot_id") != snapshot_id:
                raise AssertionError(f"unexpected snapshot show payload: {show_payload}")

            snapshot_files = run_indexer_json(
                [
                    "--output",
                    "json",
                    "file",
                    "list",
                    "--snapshot-id",
                    str(snapshot_id),
                    "--source",
                    source_name,
                    "--limit",
                    "10",
                ]
            )
            if snapshot_files.get("consistency") != "stored-snapshot":
                raise AssertionError(f"unexpected snapshot consistency: {snapshot_files}")
            original_items = assert_catalog(
                snapshot_files,
                {"a.txt", "b.txt", "c.txt"},
                {"a.txt": 4, "b.txt": 4, "c.txt": 6},
                "initial snapshot catalogue",
            )

            b_file_id = original_items["b.txt"].get("file_id")
            if not isinstance(b_file_id, int):
                raise AssertionError(f"missing snapshot file id for b.txt: {original_items['b.txt']}")

            snapshot_search = run_indexer_json(
                [
                    "--output",
                    "json",
                    "file",
                    "search",
                    "--name",
                    "b.txt",
                    "--snapshot-id",
                    str(snapshot_id),
                    "--source",
                    source_name,
                    "--limit",
                    "10",
                ]
            )
            assert_catalog(
                snapshot_search,
                {"b.txt"},
                {"b.txt": 4},
                "snapshot search",
            )

            snapshot_file_show = run_indexer_json(
                [
                    "--output",
                    "json",
                    "file",
                    "show",
                    "--id",
                    str(b_file_id),
                    "--snapshot-id",
                    str(snapshot_id),
                ]
            )
            shown_item = snapshot_file_show.get("item")
            if not isinstance(shown_item, dict) or shown_item.get("path") != "b.txt":
                raise AssertionError(f"unexpected snapshot file show payload: {snapshot_file_show}")

            write_tree(source_root, LIVE_FILES)
            live_scan = run_indexer_json(
                ["--output", "json", "scan", "--source", source_name]
            )
            if live_scan.get("scanned_files") != 3 or live_scan.get("total_bytes") != 21:
                raise AssertionError(f"unexpected live scan payload: {live_scan}")

            clean_payload = run_indexer_json(
                ["--output", "json", "clean", "--source", source_name]
            )
            if clean_payload.get("stale_files") != 1:
                raise AssertionError(f"unexpected clean payload: {clean_payload}")

            live_files = run_indexer_json(
                [
                    "--output",
                    "json",
                    "file",
                    "list",
                    "--source",
                    source_name,
                    "--limit",
                    "10",
                ]
            )
            if live_files.get("consistency") != "live":
                raise AssertionError(f"unexpected live consistency: {live_files}")
            assert_catalog(
                live_files,
                {"a.txt", "c.txt", "d.txt"},
                {"a.txt": 12, "c.txt": 6, "d.txt": 3},
                "live catalogue after rescan",
            )

            live_b_search = run_indexer_json(
                [
                    "--output",
                    "json",
                    "file",
                    "search",
                    "--name",
                    "b.txt",
                    "--source",
                    source_name,
                    "--limit",
                    "10",
                ]
            )
            assert_catalog(live_b_search, set(), {}, "live search after clean")

            immutable_files = run_indexer_json(
                [
                    "--output",
                    "json",
                    "file",
                    "list",
                    "--snapshot-id",
                    str(snapshot_id),
                    "--source",
                    source_name,
                    "--limit",
                    "10",
                ]
            )
            assert_catalog(
                immutable_files,
                {"a.txt", "b.txt", "c.txt"},
                {"a.txt": 4, "b.txt": 4, "c.txt": 6},
                "snapshot catalogue after live rescan",
            )

            immutable_b_search = run_indexer_json(
                [
                    "--output",
                    "json",
                    "file",
                    "search",
                    "--name",
                    "b.txt",
                    "--snapshot-id",
                    str(snapshot_id),
                    "--source",
                    source_name,
                    "--limit",
                    "10",
                ]
            )
            assert_catalog(
                immutable_b_search,
                {"b.txt"},
                {"b.txt": 4},
                "snapshot search after live clean",
            )

            delete_payload = run_indexer_json(
                [
                    "--output",
                    "json",
                    "snapshot",
                    "delete",
                    "--id",
                    str(snapshot_id),
                ]
            )
            if delete_payload.get("deleted") is not True:
                raise AssertionError(f"snapshot was not deleted: {delete_payload}")
            if delete_payload.get("deleted_file_count") != 3:
                raise AssertionError(f"unexpected deleted file count: {delete_payload}")

            deleted_id = snapshot_id
            snapshot_id = None
            deleted_show = run_indexer_result(
                [
                    "--output",
                    "json",
                    "snapshot",
                    "show",
                    "--id",
                    str(deleted_id),
                ]
            )
            if deleted_show.returncode == 0:
                raise AssertionError(
                    f"deleted snapshot {deleted_id} is still readable:\n{deleted_show.stdout}"
                )
            if "catalog_snapshot_not_found" not in deleted_show.stderr:
                raise AssertionError(
                    f"unexpected deleted snapshot error:\nstdout:\n{deleted_show.stdout}\n"
                    f"stderr:\n{deleted_show.stderr}"
                )

            print(
                f"OK fod-indexer catalogue snapshot source={source_name} snapshot={deleted_id}"
            )
    finally:
        if snapshot_id is not None:
            try:
                run_indexer_json(
                    [
                        "--output",
                        "json",
                        "snapshot",
                        "delete",
                        "--id",
                        str(snapshot_id),
                    ]
                )
            except Exception:
                pass
        cleanup_test_dir(source_root)
        try:
            cleanup_indexer_sources(dsn, [source_name])
        except Exception:
            pass


if __name__ == "__main__":
    run_catalog_snapshot_regression()
