#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import shutil
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
    assert_contains,
    cleanup_indexer_state,
    cleanup_materialized_roots,
    run_indexer,
    snapshot_tree,
)
from tests.integration.fod_mount import FODMount

LOCAL_ROOT = Path("/tmp/fod-indexer-kind-local")
MIRROR_ROOT = Path("/tmp/fod-indexer-kind-mirror")
GITHUB_ROOT = Path("/tmp/fod-indexer-kind-github")

LOCAL_FILES: dict[str, bytes] = {
    "a.txt": b"same",
    "b.txt": b"same",
    "c.txt": b"unique",
}

MIRROR_FILES: dict[str, bytes] = {
    "docs/report.txt": b"mm",
    "visible/readme.pdf": b"mirror-8",
    ".hidden/secret.txt": b"skip",
    "cache/temp.bin": b"skip",
    "node_modules/pkg/index.js": b"skip",
    "build/artifact.bin": b"skip",
}

GITHUB_FILES: dict[str, bytes] = {
    "manual.docx": b"ghi",
    "sheet.xlsx": b"github7",
}


def write_nested_tree(root: Path, files: dict[str, bytes]) -> None:
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True, exist_ok=True)
    for name, content in files.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)


def assert_not_contains(text: str, needle: str, label: str) -> None:
    if needle in text:
        raise AssertionError(f"unexpected {needle!r} in {label} output:\n{text}")


def main() -> None:
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    cleanup_indexer_state(dsn)
    cleanup_materialized_roots(dsn)

    local_snapshot = None
    mirror_snapshot = None
    github_snapshot = None
    try:
        write_nested_tree(LOCAL_ROOT, LOCAL_FILES)
        write_nested_tree(MIRROR_ROOT, MIRROR_FILES)
        write_nested_tree(GITHUB_ROOT, GITHUB_FILES)
        local_snapshot = snapshot_tree(LOCAL_ROOT)
        mirror_snapshot = snapshot_tree(MIRROR_ROOT)
        github_snapshot = snapshot_tree(GITHUB_ROOT)

        with tempfile.TemporaryDirectory(prefix="fod-indexer-kinds-") as mount_dir:
            with FODMount(str(ROOT)) as mount:
                mount.init_schema()
                cleanup_indexer_state(dsn)
                mount.start(mount_dir)
                cleanup_materialized_roots(dsn)

                local_add_output = run_indexer(
                    ROOT,
                    [
                        "source",
                        "add",
                        "--name",
                        "local-smoke",
                        "--path",
                        str(LOCAL_ROOT),
                        "--kind",
                        "local",
                    ],
                )
                assert_contains(local_add_output, "Registered source local-smoke as local", "local add")
                assert_contains(local_add_output, "policy: path-backed", "local add")
                assert_contains(local_add_output, "capabilities: path_backed=true", "local add")

                mirror_add_output = run_indexer(
                    ROOT,
                    [
                        "source",
                        "add",
                        "--name",
                        "mirror-smoke",
                        "--path",
                        str(MIRROR_ROOT),
                        "--kind",
                        "smb",
                    ],
                )
                assert_contains(mirror_add_output, "Registered source mirror-smoke as smb", "mirror add")
                assert_contains(mirror_add_output, "policy: mirrored", "mirror add")
                assert_contains(mirror_add_output, "capabilities: path_backed=true", "mirror add")

                github_add_output = run_indexer(
                    ROOT,
                    [
                        "source",
                        "add",
                        "--name",
                        "github-smoke",
                        "--path",
                        str(GITHUB_ROOT),
                        "--kind",
                        "github",
                    ],
                )
                assert_contains(github_add_output, "Registered source github-smoke as github", "github add")
                assert_contains(github_add_output, "policy: export-backed", "github add")
                assert_contains(github_add_output, "capabilities: path_backed=true", "github add")

                registered_smb_output = run_indexer(ROOT, ["source", "list", "--kind", "smb"])
                assert_contains(registered_smb_output, "kind filter: smb", "source list smb")
                assert_contains(registered_smb_output, "mirror-smoke", "source list smb")
                assert_contains(registered_smb_output, "policy=mirrored", "source list smb")

                browsed_mirror_output = run_indexer(
                    ROOT,
                    [
                        "source",
                        "list",
                        "--path",
                        str(MIRROR_ROOT),
                        "--kind",
                        "smb",
                    ],
                )
                assert_contains(browsed_mirror_output, "FOD indexer source list", "browse mirror")
                assert_contains(browsed_mirror_output, "mode: browse", "browse mirror")
                assert_contains(browsed_mirror_output, "kind hint: smb", "browse mirror")
                assert_contains(browsed_mirror_output, "policy: mirrored", "browse mirror")
                assert_contains(browsed_mirror_output, "available path=", "browse mirror")
                assert_contains(browsed_mirror_output, "docs", "browse mirror")
                assert_contains(browsed_mirror_output, "visible", "browse mirror")
                assert_not_contains(browsed_mirror_output, ".hidden", "browse mirror")
                assert_not_contains(browsed_mirror_output, "cache", "browse mirror")
                assert_not_contains(browsed_mirror_output, "node_modules", "browse mirror")
                assert_not_contains(browsed_mirror_output, "build", "browse mirror")

                local_scan_output = run_indexer(ROOT, ["scan", "--source", "local-smoke"])
                assert_contains(local_scan_output, "scanned files: 3", "local scan")
                assert_contains(local_scan_output, "ok files: 3", "local scan")
                assert_contains(local_scan_output, "total bytes: 14", "local scan")

                local_hash_output = run_indexer(ROOT, ["hash", "--source", "local-smoke", "--candidates-only"])
                assert_contains(local_hash_output, "candidate files: 2", "local hash")
                assert_contains(local_hash_output, "duplicate sets: 1", "local hash")

                local_plan_output = run_indexer(ROOT, ["plan-import", "--source", "local-smoke", "--dry-run"])
                assert_contains(local_plan_output, "source: local-smoke", "local plan")
                assert_contains(local_plan_output, "scanned files: 3", "local plan")
                assert_contains(local_plan_output, "unique payloads: 2", "local plan")
                assert_contains(local_plan_output, "source bytes: 14", "local plan")
                assert_contains(local_plan_output, "estimated import bytes: 10", "local plan")
                assert_contains(local_plan_output, "estimated saved bytes: 4", "local plan")

                local_materialize_output = run_indexer(ROOT, ["materialize", "--source", "local-smoke"])
                assert_contains(local_materialize_output, "FOD indexer materialize", "local materialize")
                assert_contains(local_materialize_output, "duplicate groups: 1", "local materialize")
                assert_contains(local_materialize_output, "canonical files: 2", "local materialize")
                assert_contains(local_materialize_output, "reference files: 1", "local materialize")
                assert_contains(local_materialize_output, "source bytes: 14", "local materialize")
                assert_contains(local_materialize_output, "imported bytes: 10", "local materialize")
                assert_contains(local_materialize_output, "saved bytes: 4", "local materialize")
                if snapshot_tree(LOCAL_ROOT) != local_snapshot:
                    raise AssertionError("local source tree changed during materialize")

                mirror_scan_output = run_indexer(ROOT, ["scan", "--source", "mirror-smoke"])
                assert_contains(mirror_scan_output, "scanned files: 2", "mirror scan")
                assert_contains(mirror_scan_output, "ok files: 2", "mirror scan")
                assert_contains(mirror_scan_output, "total bytes: 10", "mirror scan")

                mirror_hash_output = run_indexer(ROOT, ["hash", "--source", "mirror-smoke", "--candidates-only"])
                assert_contains(mirror_hash_output, "candidate files: 0", "mirror hash")

                mirror_plan_output = run_indexer(ROOT, ["plan-import", "--source", "mirror-smoke", "--dry-run"])
                assert_contains(mirror_plan_output, "source: mirror-smoke", "mirror plan")
                assert_contains(mirror_plan_output, "scanned files: 2", "mirror plan")
                assert_contains(mirror_plan_output, "unique payloads: 2", "mirror plan")
                assert_contains(mirror_plan_output, "source bytes: 10", "mirror plan")
                assert_contains(mirror_plan_output, "estimated import bytes: 10", "mirror plan")
                assert_contains(mirror_plan_output, "estimated saved bytes: 0", "mirror plan")

                mirror_materialize_output = run_indexer(ROOT, ["materialize", "--source", "mirror-smoke"])
                assert_contains(mirror_materialize_output, "FOD indexer materialize", "mirror materialize")
                assert_contains(mirror_materialize_output, "duplicate groups: 0", "mirror materialize")
                assert_contains(mirror_materialize_output, "canonical files: 2", "mirror materialize")
                assert_contains(mirror_materialize_output, "reference files: 0", "mirror materialize")
                assert_contains(mirror_materialize_output, "source bytes: 10", "mirror materialize")
                assert_contains(mirror_materialize_output, "imported bytes: 10", "mirror materialize")
                assert_contains(mirror_materialize_output, "saved bytes: 0", "mirror materialize")
                if snapshot_tree(MIRROR_ROOT) != mirror_snapshot:
                    raise AssertionError("mirror source tree changed during materialize")

                github_scan_output = run_indexer(ROOT, ["scan", "--source", "github-smoke"])
                assert_contains(github_scan_output, "scanned files: 2", "github scan")
                assert_contains(github_scan_output, "ok files: 2", "github scan")
                assert_contains(github_scan_output, "total bytes: 10", "github scan")

                github_hash_output = run_indexer(ROOT, ["hash", "--source", "github-smoke", "--candidates-only"])
                assert_contains(github_hash_output, "candidate files: 0", "github hash")

                all_plan_output = run_indexer(ROOT, ["plan-import", "--all-sources", "--dry-run"])
                assert_contains(all_plan_output, "source: all sources", "all plan")
                assert_contains(all_plan_output, "scanned files: 7", "all plan")
                assert_contains(all_plan_output, "candidate duplicate groups: 1", "all plan")
                assert_contains(all_plan_output, "confirmed duplicate groups: 1", "all plan")
                assert_contains(all_plan_output, "unique payloads: 6", "all plan")
                assert_contains(all_plan_output, "source bytes: 34", "all plan")
                assert_contains(all_plan_output, "estimated import bytes: 30", "all plan")
                assert_contains(all_plan_output, "estimated saved bytes: 4", "all plan")

                github_plan_output = run_indexer(ROOT, ["plan-import", "--source", "github-smoke", "--dry-run"])
                assert_contains(github_plan_output, "source: github-smoke", "github plan")
                assert_contains(github_plan_output, "scanned files: 2", "github plan")
                assert_contains(github_plan_output, "unique payloads: 2", "github plan")

                github_materialize_output = run_indexer(ROOT, ["materialize", "--source", "github-smoke"])
                assert_contains(github_materialize_output, "FOD indexer materialize", "github materialize")
                assert_contains(github_materialize_output, "duplicate groups: 0", "github materialize")
                assert_contains(github_materialize_output, "canonical files: 2", "github materialize")
                assert_contains(github_materialize_output, "reference files: 0", "github materialize")
                assert_contains(github_materialize_output, "source bytes: 10", "github materialize")
                assert_contains(github_materialize_output, "imported bytes: 10", "github materialize")
                assert_contains(github_materialize_output, "saved bytes: 0", "github materialize")
                if snapshot_tree(GITHUB_ROOT) != github_snapshot:
                    raise AssertionError("github source tree changed during materialize")

                shutil.rmtree(MIRROR_ROOT, ignore_errors=True)

                mirror_clean_preview = run_indexer(ROOT, ["clean", "--source", "mirror-smoke", "--dry-run"])
                assert_contains(mirror_clean_preview, "FOD indexer clean", "mirror clean dry-run")
                assert_contains(mirror_clean_preview, "source root: missing", "mirror clean dry-run")
                assert_contains(mirror_clean_preview, "indexed files: 2", "mirror clean dry-run")
                assert_contains(mirror_clean_preview, "present files: 0", "mirror clean dry-run")
                assert_contains(mirror_clean_preview, "stale files: 2", "mirror clean dry-run")
                assert_contains(mirror_clean_preview, "plan entries removed:", "mirror clean dry-run")

                mirror_clean_output = run_indexer(ROOT, ["clean", "--source", "mirror-smoke"])
                assert_contains(mirror_clean_output, "FOD indexer clean", "mirror clean")
                assert_contains(mirror_clean_output, "source root: missing", "mirror clean")
                assert_contains(mirror_clean_output, "indexed files: 2", "mirror clean")
                assert_contains(mirror_clean_output, "present files: 0", "mirror clean")
                assert_contains(mirror_clean_output, "stale files: 2", "mirror clean")
                assert_contains(mirror_clean_output, "plan entries removed:", "mirror clean")
                assert_contains(mirror_clean_output, "duplicate sets refreshed:", "mirror clean")

                with psycopg2.connect(**dsn) as conn:
                    with conn.cursor() as cur:
                        cur.execute("SET search_path TO fod, public")
                        cur.execute(
                            "SELECT COUNT(*) FROM index_files f JOIN index_sources s ON s.id_index_source = f.id_index_source WHERE s.name = %s",
                            ("mirror-smoke",),
                        )
                        row = cur.fetchone()
                        if row is None or int(row[0]) != 0:
                            raise AssertionError(f"mirror rows still indexed after cleanup: {row}")

                mirror_post_clean_plan = run_indexer(ROOT, ["plan-import", "--source", "mirror-smoke", "--dry-run"])
                assert_contains(mirror_post_clean_plan, "source: mirror-smoke", "mirror post-clean plan")
                assert_contains(mirror_post_clean_plan, "scanned files: 0", "mirror post-clean plan")
                assert_contains(mirror_post_clean_plan, "unique payloads: 0", "mirror post-clean plan")

                print(
                    "OK fod-indexer source-kind matrix "
                    f"local={LOCAL_ROOT} mirror={MIRROR_ROOT} github={GITHUB_ROOT}"
                )
    finally:
        try:
            cleanup_materialized_roots(dsn)
        except Exception:
            pass
        shutil.rmtree(LOCAL_ROOT, ignore_errors=True)
        shutil.rmtree(MIRROR_ROOT, ignore_errors=True)
        shutil.rmtree(GITHUB_ROOT, ignore_errors=True)
        try:
            cleanup_indexer_state(dsn)
        except Exception:
            pass


if __name__ == "__main__":
    main()
