#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from fod_backend import load_dsn_from_config
from tests.integration.fod_indexer_testlib import (
    apply_database_env,
    cleanup_indexer_state,
    cleanup_materialized_roots,
    snapshot_tree,
)
from tests.integration.fod_mount import FODMount

SOURCE_PARENT = Path("/tmp/fod-indexer-json-output")
SOURCE_ROOT = SOURCE_PARENT / "indexed"
BROWSE_ONLY_ROOT = SOURCE_PARENT / "browse-only"
SOURCE_FILES: dict[str, bytes] = {
    "a.txt": b"same",
    "b.txt": b"same",
    "c.txt": b"unique",
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
            f"fod-indexer command failed: {' '.join(args)}\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    try:
        return json.loads(result.stdout.strip())
    except json.JSONDecodeError as err:
        raise AssertionError(
            f"unable to parse JSON output for {' '.join(args)}: {err}\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        ) from err


def prepare_source_tree() -> None:
    shutil.rmtree(SOURCE_PARENT, ignore_errors=True)
    SOURCE_ROOT.mkdir(parents=True, exist_ok=True)
    BROWSE_ONLY_ROOT.mkdir(parents=True, exist_ok=True)
    for rel_path, content in SOURCE_FILES.items():
        (SOURCE_ROOT / rel_path).write_bytes(content)


def main() -> None:
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    cleanup_indexer_state(dsn)
    cleanup_materialized_roots(dsn)

    source_snapshot = None
    try:
        prepare_source_tree()
        source_snapshot = snapshot_tree(SOURCE_PARENT)

        with tempfile.TemporaryDirectory(prefix="fod-indexer-json-output-"):
            with FODMount(str(ROOT)) as mount:
                mount.init_schema()
                cleanup_indexer_state(dsn)
                cleanup_materialized_roots(dsn)

                add_payload = run_indexer_json(
                    [
                        "--output",
                        "json",
                        "source",
                        "add",
                        "--name",
                        "json-smoke",
                        "--path",
                        str(SOURCE_ROOT),
                        "--kind",
                        "local",
                    ]
                )
                source_record = add_payload["source"]
                if not isinstance(source_record, dict):
                    raise AssertionError(f"unexpected source add payload: {add_payload}")
                if source_record.get("name") != "json-smoke":
                    raise AssertionError(f"unexpected source add name: {source_record}")
                if source_record.get("kind") != "local":
                    raise AssertionError(f"unexpected source add kind: {source_record}")
                capabilities = source_record.get("capabilities")
                if not isinstance(capabilities, dict) or capabilities.get("path_backed") is not True:
                    raise AssertionError(f"unexpected capabilities payload: {source_record}")

                registered_payload = run_indexer_json(["--output", "json", "source", "list", "--kind", "local"])
                if registered_payload.get("mode") != "registered":
                    raise AssertionError(f"unexpected registered source mode: {registered_payload}")
                registered_sources = registered_payload.get("registered_sources")
                if not isinstance(registered_sources, list) or len(registered_sources) != 1:
                    raise AssertionError(f"unexpected registered sources payload: {registered_payload}")
                registered_source = registered_sources[0]
                if not isinstance(registered_source, dict) or registered_source.get("name") != "json-smoke":
                    raise AssertionError(f"unexpected registered source entry: {registered_payload}")

                browse_payload = run_indexer_json(
                    [
                        "--output",
                        "json",
                        "source",
                        "list",
                        "--path",
                        str(SOURCE_PARENT),
                        "--kind",
                        "local",
                    ]
                )
                if browse_payload.get("mode") != "browse":
                    raise AssertionError(f"unexpected browse mode payload: {browse_payload}")
                directories = browse_payload.get("directories")
                if not isinstance(directories, list) or len(directories) != 2:
                    raise AssertionError(f"unexpected browse directories payload: {browse_payload}")

                scan_payload = run_indexer_json(["--output", "json", "scan", "--source", "json-smoke"])
                if scan_payload.get("source_name") != "json-smoke":
                    raise AssertionError(f"unexpected scan payload: {scan_payload}")
                if scan_payload.get("source_path") != str(SOURCE_ROOT.resolve()):
                    raise AssertionError(f"unexpected scan source path: {scan_payload}")
                if scan_payload.get("scanned_files") != 3:
                    raise AssertionError(f"unexpected scan count: {scan_payload}")
                if scan_payload.get("ok_files") != 3:
                    raise AssertionError(f"unexpected scan ok count: {scan_payload}")

                hash_payload = run_indexer_json(["--output", "json", "hash", "--source", "json-smoke", "--candidates-only"])
                if hash_payload.get("source_name") != "json-smoke":
                    raise AssertionError(f"unexpected hash payload: {hash_payload}")
                if hash_payload.get("source_path") != str(SOURCE_ROOT.resolve()):
                    raise AssertionError(f"unexpected hash source path: {hash_payload}")
                if hash_payload.get("duplicate_sets") != 1:
                    raise AssertionError(f"unexpected hash duplicate set count: {hash_payload}")

                duplicate_report = run_indexer_json(["--output", "json", "report", "duplicates"])
                if duplicate_report.get("confirmed_duplicate_sets") != 1:
                    raise AssertionError(f"unexpected duplicate report payload: {duplicate_report}")
                duplicate_sets = duplicate_report.get("duplicate_sets")
                if not isinstance(duplicate_sets, list) or len(duplicate_sets) != 1:
                    raise AssertionError(f"unexpected duplicate sets payload: {duplicate_report}")
                first_set = duplicate_sets[0]
                if not isinstance(first_set, dict):
                    raise AssertionError(f"unexpected duplicate set payload: {duplicate_report}")
                set_id = first_set.get("duplicate_set", {}).get("id_duplicate_set")
                if not isinstance(set_id, int):
                    raise AssertionError(f"missing duplicate set id: {duplicate_report}")

                duplicate_snapshot = run_indexer_json(["--output", "json", "report", "duplicates", "--id", str(set_id)])
                if duplicate_snapshot.get("duplicate_set", {}).get("id_duplicate_set") != set_id:
                    raise AssertionError(f"unexpected duplicate set snapshot: {duplicate_snapshot}")
                members = duplicate_snapshot.get("members")
                if not isinstance(members, list) or len(members) != 2:
                    raise AssertionError(f"unexpected duplicate snapshot members: {duplicate_snapshot}")

                plan_payload = run_indexer_json(["--output", "json", "plan-import", "--source", "json-smoke", "--dry-run"])
                plan_id = plan_payload.get("plan_id")
                if not isinstance(plan_id, int):
                    raise AssertionError(f"missing plan id in import plan: {plan_payload}")
                if plan_payload.get("source_filter") != "json-smoke":
                    raise AssertionError(f"unexpected plan source filter: {plan_payload}")
                if plan_payload.get("unique_payload_count") != 2:
                    raise AssertionError(f"unexpected plan unique payload count: {plan_payload}")

                plan_snapshot = run_indexer_json(["--output", "json", "plan", "show", "--id", str(plan_id)])
                summary = plan_snapshot.get("summary")
                if not isinstance(summary, dict) or summary.get("plan_id") != plan_id:
                    raise AssertionError(f"unexpected plan snapshot summary: {plan_snapshot}")
                entries = plan_snapshot.get("entries")
                if not isinstance(entries, list) or len(entries) != 3:
                    raise AssertionError(f"unexpected plan snapshot entries: {plan_snapshot}")

                clean_payload = run_indexer_json(["--output", "json", "clean", "--source", "json-smoke", "--dry-run"])
                if clean_payload.get("source_name") != "json-smoke":
                    raise AssertionError(f"unexpected clean payload: {clean_payload}")
                if clean_payload.get("dry_run") is not True:
                    raise AssertionError(f"unexpected clean dry-run payload: {clean_payload}")
                if clean_payload.get("indexed_files") != 3:
                    raise AssertionError(f"unexpected clean indexed file count: {clean_payload}")

                materialize_payload = run_indexer_json(
                    ["--output", "json", "materialize", "--source", "json-smoke", "--dry-run"]
                )
                if materialize_payload.get("source_name") != "json-smoke":
                    raise AssertionError(f"unexpected materialize payload: {materialize_payload}")
                if materialize_payload.get("dry_run") is not True:
                    raise AssertionError(f"unexpected materialize dry-run payload: {materialize_payload}")
                if materialize_payload.get("scanned_files") != 3:
                    raise AssertionError(f"unexpected materialize scanned count: {materialize_payload}")

                if snapshot_tree(SOURCE_PARENT) != source_snapshot:
                    raise AssertionError("source tree changed during JSON-output smoke")

                print(
                    f"OK fod-indexer json output source={SOURCE_ROOT} plan_id={plan_id} duplicate_set_id={set_id}"
                )
    finally:
        try:
            cleanup_materialized_roots(dsn)
        except Exception:
            pass
        shutil.rmtree(SOURCE_PARENT, ignore_errors=True)
        try:
            cleanup_indexer_state(dsn)
        except Exception:
            pass


if __name__ == "__main__":
    main()
