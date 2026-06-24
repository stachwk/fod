#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import shutil
import sys
import tempfile
import os
import shlex
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from fod_backend import load_dsn_from_config
from tests.integration.fod_indexer_testlib import (
    apply_database_env,
    assert_contains,
    cleanup_indexer_state,
    cleanup_materialized_roots,
    run_indexer,
    snapshot_tree,
)
from tests.integration.fod_mount import FODMount

USABILITY_PARENT = Path("/tmp/fod-indexer-usability")
USABILITY_INDEXED_ROOT = USABILITY_PARENT / "indexed"
USABILITY_BROWSE_ONLY_ROOT = USABILITY_PARENT / "browse-only"

USABILITY_FILES: dict[str, bytes] = {
    "a.txt": b"same",
    "b.txt": b"same",
    "c.txt": b"unique",
}


def write_usability_tree() -> None:
    shutil.rmtree(USABILITY_PARENT, ignore_errors=True)
    USABILITY_INDEXED_ROOT.mkdir(parents=True, exist_ok=True)
    USABILITY_BROWSE_ONLY_ROOT.mkdir(parents=True, exist_ok=True)
    for rel_path, content in USABILITY_FILES.items():
        (USABILITY_INDEXED_ROOT / rel_path).write_bytes(content)


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


def test_help_and_usage_read_like_a_short_user_guide() -> None:
    help_output = run_indexer(ROOT, ["help"])
    assert_contains(
        help_output,
        "Index external files before importing them into FOD.",
        "top-level help",
    )
    assert_contains(
        help_output,
        "fod-indexer source add --path ~/Documents --kind local",
        "top-level help",
    )
    assert_contains(
        help_output,
        "fod-indexer source list --kind adb",
        "top-level help",
    )
    assert_contains(
        help_output,
        "fod-indexer materialize --source lt7300_Documents",
        "top-level help",
    )
    assert_contains(
        help_output,
        "fod-indexer cleanup-failed --plan 42",
        "top-level help",
    )

    source_add_help = run_indexer(ROOT, ["source", "add", "--help"])
    assert_contains(
        source_add_help,
        "fod-indexer source add --path <PATH> [--name <NAME>] [--kind <KIND>]",
        "source add help",
    )
    assert_contains(
        source_add_help,
        "Filesystem path for the source root.",
        "source add help",
    )

    source_list_help = run_indexer(ROOT, ["source", "list", "--help"])
    assert_contains(
        source_list_help,
        "fod-indexer source list [--kind <KIND>] [--path <PATH>]",
        "source list help",
    )
    assert_contains(
        source_list_help,
        "Use --kind adb without --path to probe the device",
        "source list help",
    )

    scan_help = run_indexer(ROOT, ["scan", "--help"])
    assert_contains(scan_help, "Usage: fod-indexer scan --source <SOURCE>", "scan help")
    assert_contains(scan_help, "--source <SOURCE>", "scan help")

    materialize_help = run_indexer(ROOT, ["materialize", "--help"])
    assert_contains(materialize_help, "Use --dry-run to preview", "materialize help")
    assert_contains(materialize_help, "partial tree back automatically", "materialize help")
    assert_contains(materialize_help, "cleanup-failed", "materialize help")

    scan_usage = run_indexer_result(["scan"])
    scan_usage_output = scan_usage.stdout + scan_usage.stderr
    assert_contains(scan_usage_output, "the following required arguments were not provided", "scan usage")
    assert_contains(scan_usage_output, "Usage: fod-indexer scan --source <SOURCE>", "scan usage")

    source_add_usage = run_indexer_result(["source", "add"])
    source_add_usage_output = source_add_usage.stdout + source_add_usage.stderr
    assert_contains(
        source_add_usage_output,
        "the following required arguments were not provided",
        "source add usage",
    )
    assert_contains(
        source_add_usage_output,
        "Usage: fod-indexer source add --path <PATH> [--name <NAME>] [--kind <KIND>]",
        "source add usage",
    )


def test_user_journey_surfaces_progress_and_browse_hints() -> None:
    dsn: dict[str, str] | None = None
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    cleanup_indexer_state(dsn)
    cleanup_materialized_roots(dsn)

    parent_snapshot = None
    try:
        write_usability_tree()
        parent_snapshot = snapshot_tree(USABILITY_PARENT)

        with tempfile.TemporaryDirectory(prefix="fod-indexer-usability-") as mount_dir:
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
                        "ux-smoke",
                        "--path",
                        str(USABILITY_INDEXED_ROOT),
                        "--kind",
                        "local",
                    ],
                )
                assert_contains(source_add_output, "Registered source ux-smoke as local", "source add")
                assert_contains(source_add_output, "policy: path-backed", "source add")
                assert_contains(source_add_output, "capabilities: path_backed=true", "source add")

                browse_output = run_indexer(
                    ROOT,
                    ["source", "list", "--path", str(USABILITY_PARENT), "--kind", "local"],
                )
                assert_contains(browse_output, "FOD indexer source list", "source browse")
                assert_contains(browse_output, "mode: browse", "source browse")
                assert_contains(browse_output, "kind hint: local", "source browse")
                assert_contains(browse_output, "available path=", "source browse")
                assert_contains(browse_output, "added path=", "source browse")
                assert_contains(browse_output, "fod-indexer source add --kind local --path", "source browse")

                scan_output = run_indexer(ROOT, ["scan", "--source", "ux-smoke"])
                assert_contains(scan_output, "FOD indexer scan progress: phase=started", "scan")
                assert_contains(scan_output, "FOD indexer scan progress: phase=running", "scan")
                assert_contains(scan_output, "current=", "scan")
                assert_contains(scan_output, "scanned files: 3", "scan")
                assert_contains(scan_output, "ok files: 3", "scan")

                hash_output = run_indexer(ROOT, ["hash", "--source", "ux-smoke", "--candidates-only"])
                assert_contains(hash_output, "FOD indexer hash progress: phase=started", "hash")
                assert_contains(hash_output, "FOD indexer hash progress: phase=partial", "hash")
                assert_contains(hash_output, "FOD indexer hash progress: phase=done", "hash")
                assert_contains(hash_output, "current=", "hash")
                assert_contains(hash_output, "mode=candidates-only", "hash")
                assert_contains(hash_output, "duplicate sets: 1", "hash")

                report_output = run_indexer(ROOT, ["report", "duplicates"])
                assert_contains(report_output, "FOD indexer duplicate report", "duplicates")
                assert_contains(report_output, "confirmed duplicate sets: 1", "duplicates")

                plan_output = run_indexer(ROOT, ["plan-import", "--source", "ux-smoke", "--dry-run"])
                assert_contains(plan_output, "FOD indexer dry-run import plan", "plan-import")
                assert_contains(plan_output, "source: ux-smoke", "plan-import")
                assert_contains(plan_output, "unique payloads: 2", "plan-import")
                assert_contains(plan_output, "estimated saved bytes: 4", "plan-import")

                materialize_preview_output = run_indexer(ROOT, ["materialize", "--source", "ux-smoke", "--dry-run"])
                assert_contains(materialize_preview_output, "FOD indexer materialize", "materialize dry-run")
                assert_contains(materialize_preview_output, "mode: dry-run", "materialize dry-run")
                assert_contains(materialize_preview_output, "source: ux-smoke", "materialize dry-run")

                if snapshot_tree(USABILITY_PARENT) != parent_snapshot:
                    raise AssertionError("user-journey source tree changed during dry-run flow")

                print(
                    f"OK fod-indexer usability source={USABILITY_INDEXED_ROOT} browse_root={USABILITY_PARENT}"
                )
    finally:
        if dsn is not None:
            try:
                cleanup_materialized_roots(dsn)
            except Exception:
                pass
        shutil.rmtree(USABILITY_PARENT, ignore_errors=True)
        if dsn is not None:
            try:
                cleanup_indexer_state(dsn)
            except Exception:
                pass


def main() -> None:
    test_help_and_usage_read_like_a_short_user_guide()
    test_user_journey_surfaces_progress_and_browse_hints()


if __name__ == "__main__":
    main()
