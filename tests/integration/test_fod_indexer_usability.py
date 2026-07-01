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
)
from tests.integration.fod_mount import FODMount

USABILITY_ADB_RUNTIME = "fod-indexer-adb-runtime"
USABILITY_CLEAN_FILES: dict[str, bytes] = {
    "a.txt": b"same",
    "b.txt": b"same",
    "c.txt": b"unique",
}

USABILITY_FILES: dict[str, bytes] = {
    "a.txt": b"same",
    "b.txt": b"same",
    "c.txt": b"unique",
}


class SkipTest(RuntimeError):
    pass


def write_usability_tree(parent: Path, indexed_root: Path, browse_only_root: Path) -> None:
    prepare_clean_dir(parent)
    indexed_root.mkdir(parents=True, exist_ok=True)
    browse_only_root.mkdir(parents=True, exist_ok=True)
    for rel_path, content in USABILITY_FILES.items():
        (indexed_root / rel_path).write_bytes(content)


def write_clean_tree(clean_root: Path) -> None:
    prepare_clean_dir(clean_root)
    for rel_path, content in USABILITY_CLEAN_FILES.items():
        (clean_root / rel_path).write_bytes(content)


def run_indexer_result(args: list[str]) -> subprocess.CompletedProcess[str]:
    return run_indexer_result_with_env(args, {})


def run_indexer_result_with_env(
    args: list[str], extra_env: dict[str, str]
) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env.pop("DATABASE_URL", None)
    env.pop("FOD_INDEXER_CONNINFO", None)
    env.update(extra_env)
    env["INDEXER_ARGS"] = shlex.join(args)
    return subprocess.run(
        ["make", "--no-print-directory", "indexer"],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


def adb_target_serial() -> str:
    if shutil.which("adb") is None:
        raise SkipTest("adb is not installed")

    result = subprocess.run(
        ["adb", "devices"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise SkipTest(f"unable to run adb devices:\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}")

    for line in result.stdout.splitlines()[1:]:
        line = line.strip()
        if not line:
            continue
        fields = line.split()
        serial = fields[0] if fields else ""
        status = fields[1] if len(fields) > 1 else ""
        if status == "device" and serial:
            return serial

    raise SkipTest(f"no authorized adb device found:\n{result.stdout}")


def adb_shell_output(serial: str, args: list[str]) -> str:
    result = subprocess.run(
        ["adb", "-s", serial, "shell", *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(
            f"adb shell failed for {serial} with args {args}:\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result.stdout.strip()


def adb_storage_root(serial: str) -> str:
    candidates: list[str] = []

    def add_candidate(candidate: str) -> None:
        candidate = candidate.strip()
        if candidate and candidate not in candidates:
            candidates.append(candidate)

    for value in [adb_shell_output(serial, ["echo", "$EXTERNAL_STORAGE"]), adb_shell_output(serial, ["echo", "$SECONDARY_STORAGE"])]:
        for candidate in value.split(":"):
            add_candidate(candidate)

    for candidate in ["/sdcard", "/storage/emulated/0", "/storage/self/primary"]:
        add_candidate(candidate)

    for candidate in candidates:
        probe = subprocess.run(
            ["adb", "-s", serial, "shell", "ls", "-d", candidate],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        if probe.returncode == 0:
            return candidate

    raise AssertionError(f"unable to detect a browsable Android storage root for device {serial}")


def assert_not_contains(text: str, needle: str, label: str) -> None:
    if needle in text:
        raise AssertionError(f"unexpected {needle!r} in {label} output:\n{text}")


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
        "fod-indexer clean --source lt7300_Documents --dry-run",
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

    clean_help = run_indexer(ROOT, ["clean", "--help"])
    assert_contains(
        clean_help,
        "Use --dry-run to preview which rows would be removed without touching PostgreSQL.",
        "clean help",
    )
    assert_contains(
        clean_help,
        "remove file entries that no longer exist or should now be ignored",
        "clean help",
    )

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
    source_name = unique_source_name("ux_smoke")
    usability_parent = prepare_clean_dir(unique_indexer_path("usability"))
    usability_indexed_root = usability_parent / "indexed"
    usability_browse_only_root = usability_parent / "browse-only"
    cleanup_indexer_sources(dsn, [source_name])

    parent_snapshot = None
    try:
        write_usability_tree(usability_parent, usability_indexed_root, usability_browse_only_root)
        parent_snapshot = snapshot_tree(usability_parent)

        with tempfile.TemporaryDirectory(prefix="fod-indexer-usability-") as mount_dir:
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
                        str(usability_indexed_root),
                        "--kind",
                        "local",
                    ],
                )
                assert_contains(source_add_output, f"Registered source {source_name} as local", "source add")
                assert_contains(source_add_output, "policy: path-backed", "source add")
                assert_contains(source_add_output, "capabilities: path_backed=true", "source add")

                browse_output = run_indexer(
                    ROOT,
                    ["source", "list", "--path", str(usability_parent), "--kind", "local"],
                )
                assert_contains(browse_output, "FOD indexer source list", "source browse")
                assert_contains(browse_output, "mode: browse", "source browse")
                assert_contains(browse_output, "kind hint: local", "source browse")
                assert_contains(browse_output, "available path=", "source browse")
                assert_contains(browse_output, "added path=", "source browse")
                assert_contains(browse_output, "fod-indexer source add --kind local --path", "source browse")

                scan_output = run_indexer(ROOT, ["scan", "--source", source_name])
                assert_contains(scan_output, "FOD indexer scan progress: phase=started", "scan")
                assert_contains(scan_output, "FOD indexer scan progress: phase=running", "scan")
                assert_contains(scan_output, "current=", "scan")
                assert_contains(scan_output, "scanned files: 3", "scan")
                assert_contains(scan_output, "ok files: 3", "scan")

                hash_output = run_indexer(ROOT, ["hash", "--source", source_name, "--candidates-only"])
                assert_contains(hash_output, "FOD indexer hash progress: phase=started", "hash")
                assert_contains(hash_output, "FOD indexer hash progress: phase=partial", "hash")
                assert_contains(hash_output, "FOD indexer hash progress: phase=done", "hash")
                assert_contains(hash_output, "current=", "hash")
                assert_contains(hash_output, "mode=candidates-only", "hash")

                report_output = run_indexer(ROOT, ["report", "duplicates"])
                assert_contains(report_output, "FOD indexer duplicate report", "duplicates")
                assert_contains(report_output, source_name, "duplicates")
                assert_contains(report_output, "a.txt", "duplicates")
                assert_contains(report_output, "b.txt", "duplicates")

                plan_output = run_indexer(ROOT, ["plan-import", "--source", source_name, "--dry-run"])
                assert_contains(plan_output, "FOD indexer dry-run import plan", "plan-import")
                assert_contains(plan_output, f"source: {source_name}", "plan-import")
                assert_contains(plan_output, "unique payloads: 2", "plan-import")
                assert_contains(plan_output, "estimated saved bytes: 4", "plan-import")

                materialize_preview_output = run_indexer(ROOT, ["materialize", "--source", source_name, "--dry-run"])
                assert_contains(materialize_preview_output, "FOD indexer materialize", "materialize dry-run")
                assert_contains(materialize_preview_output, "mode: dry-run", "materialize dry-run")
                assert_contains(materialize_preview_output, f"source: {source_name}", "materialize dry-run")

                if snapshot_tree(usability_parent) != parent_snapshot:
                    raise AssertionError("user-journey source tree changed during dry-run flow")

                print(
                    f"OK fod-indexer usability source={usability_indexed_root} browse_root={usability_parent}"
                )
    finally:
        if dsn is not None:
            try:
                cleanup_materialized_roots_for_sources(dsn, [source_name])
            except Exception:
                pass
        cleanup_test_dir(usability_parent)
        if dsn is not None:
            try:
                cleanup_indexer_sources(dsn, [source_name])
            except Exception:
                pass


def test_adb_source_list_surfaces_device_and_browse_root() -> None:
    try:
        serial = adb_target_serial()
    except SkipTest as exc:
        print(f"SKIP fod-indexer adb usability: {exc}")
        return

    remote_root = adb_storage_root(serial)

    dsn: dict[str, str] | None = None
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    source_name = unique_source_name("adb_documents")
    cleanup_indexer_sources(dsn, [source_name])

    try:
        with tempfile.TemporaryDirectory(prefix=f"{USABILITY_ADB_RUNTIME}-") as runtime_dir:
            runtime_root = Path(runtime_dir)
            mount_root = runtime_root / "gvfs" / f"mtp:host={serial}" / "Internal storage"
            visible_one = mount_root / "Documents"
            visible_two = mount_root / "Pictures"
            hidden_one = mount_root / ".hidden"
            ignored_one = mount_root / "cache"
            for path in [visible_one, visible_two, hidden_one, ignored_one]:
                path.mkdir(parents=True, exist_ok=True)
            adb_snapshot = snapshot_tree(mount_root)

            with tempfile.TemporaryDirectory(prefix="fod-indexer-adb-smoke-") as mount_dir:
                with FODMount(str(ROOT)) as mount:
                    mount.init_schema()
                    cleanup_indexer_sources(dsn, [source_name])
                    mount.start(mount_dir)
                    cleanup_materialized_roots_for_sources(dsn, [source_name])

                    registered_output = run_indexer(
                        ROOT,
                        [
                            "source",
                            "add",
                            "--name",
                            source_name,
                            "--path",
                            str(visible_one),
                            "--kind",
                            "adb",
                        ],
                    )
                    assert_contains(registered_output, f"Registered source {source_name} as adb", "adb source add")
                    assert_contains(registered_output, "policy: export-backed", "adb source add")

                    browse_output = run_indexer_result_with_env(
                        ["source", "list", "--kind", "adb"],
                        {
                            "XDG_RUNTIME_DIR": str(runtime_root),
                            "ANDROID_SERIAL": serial,
                        },
                    )
                    browse_text = browse_output.stdout + browse_output.stderr
                    assert_contains(browse_text, "FOD indexer source list", "adb source list")
                    assert_contains(browse_text, "mode: adb-shell", "adb source list")
                    assert_contains(browse_text, f"device: {serial}", "adb source list")
                    assert_contains(browse_text, f"adb root: {remote_root}", "adb source list")
                    assert_contains(browse_text, "kind hint: adb", "adb source list")
                    assert_contains(browse_text, "policy: export-backed", "adb source list")
                    assert_contains(browse_text, "directories: 2", "adb source list")
                    assert_contains(browse_text, "Documents", "adb source list")
                    assert_contains(browse_text, "Pictures", "adb source list")
                    assert_contains(browse_text, "added path=", "adb source list")
                    assert_contains(browse_text, source_name, "adb source list")
                    assert_contains(browse_text, "fod-indexer source add --kind adb --path", "adb source list")
                    assert_contains(browse_text, "available path=", "adb source list")
                    assert_not_contains(browse_text, ".hidden", "adb source list")
                    assert_not_contains(browse_text, "cache", "adb source list")

                    if snapshot_tree(mount_root) != adb_snapshot:
                        raise AssertionError("adb browse tree changed unexpectedly")

                    print(
                        f"OK fod-indexer adb browse serial={serial} remote_root={remote_root} runtime={runtime_root}"
                    )
    finally:
        if dsn is not None:
            try:
                cleanup_materialized_roots_for_sources(dsn, [source_name])
            except Exception:
                pass
            try:
                cleanup_indexer_sources(dsn, [source_name])
            except Exception:
                pass


def test_clean_user_journey_prunes_stale_rows_without_touching_source_tree() -> None:
    dsn: dict[str, str] | None = None
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    source_name = unique_source_name("clean_smoke")
    clean_root = prepare_clean_dir(unique_indexer_path("clean-usability"))
    cleanup_indexer_sources(dsn, [source_name])

    post_delete_snapshot = None
    try:
        write_clean_tree(clean_root)

        with tempfile.TemporaryDirectory(prefix="fod-indexer-clean-usability-") as mount_dir:
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
                        str(clean_root),
                        "--kind",
                        "local",
                    ],
                )
                assert_contains(source_add_output, f"Registered source {source_name} as local", "clean source add")
                assert_contains(source_add_output, "policy: path-backed", "clean source add")

                scan_output = run_indexer(ROOT, ["scan", "--source", source_name])
                assert_contains(scan_output, "scanned files: 3", "clean scan")
                assert_contains(scan_output, "ok files: 3", "clean scan")

                hash_output = run_indexer(ROOT, ["hash", "--source", source_name, "--candidates-only"])
                assert_contains(hash_output, "candidate files: 2", "clean hash")
                assert_contains(hash_output, f"source: {source_name}", "clean hash")

                plan_output = run_indexer(ROOT, ["plan-import", "--source", source_name, "--dry-run"])
                assert_contains(plan_output, "FOD indexer dry-run import plan", "clean plan")
                assert_contains(plan_output, f"source: {source_name}", "clean plan")
                assert_contains(plan_output, "unique payloads: 2", "clean plan")
                assert_contains(plan_output, "estimated saved bytes: 4", "clean plan")

                with psycopg2.connect(**dsn) as conn:
                    with conn.cursor() as cur:
                        cur.execute("SET search_path TO fod, public")
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

                (clean_root / "b.txt").unlink()
                post_delete_snapshot = snapshot_tree(clean_root)

                clean_preview_output = run_indexer(ROOT, ["clean", "--source", source_name, "--dry-run"])
                assert_contains(clean_preview_output, "FOD indexer clean", "clean dry-run")
                assert_contains(clean_preview_output, "mode: dry-run", "clean dry-run")
                assert_contains(clean_preview_output, f"source: {source_name}", "clean dry-run")
                assert_contains(clean_preview_output, "source root: present", "clean dry-run")
                assert_contains(clean_preview_output, "indexed files: 3", "clean dry-run")
                assert_contains(clean_preview_output, "present files: 2", "clean dry-run")
                assert_contains(clean_preview_output, "stale files: 1", "clean dry-run")
                assert_contains(clean_preview_output, "skipped files: 0", "clean dry-run")
                assert_contains(clean_preview_output, "plan entries removed: 1", "clean dry-run")
                assert_contains(clean_preview_output, "duplicate sets refreshed: 0", "clean dry-run")

                clean_output = run_indexer(ROOT, ["clean", "--source", source_name])
                assert_contains(clean_output, "FOD indexer clean", "clean")
                assert_contains(clean_output, "mode: clean", "clean")
                assert_contains(clean_output, f"source: {source_name}", "clean")
                assert_contains(clean_output, "source root: present", "clean")
                assert_contains(clean_output, "indexed files: 3", "clean")
                assert_contains(clean_output, "present files: 2", "clean")
                assert_contains(clean_output, "stale files: 1", "clean")
                assert_contains(clean_output, "skipped files: 0", "clean")
                assert_contains(clean_output, "plan entries removed: 1", "clean")
                assert_contains(clean_output, "duplicate sets refreshed: 0", "clean")

                if snapshot_tree(clean_root) != post_delete_snapshot:
                    raise AssertionError("clean source tree changed during cleanup")

                with psycopg2.connect(**dsn) as conn:
                    with conn.cursor() as cur:
                        cur.execute("SET search_path TO fod, public")
                        cur.execute(
                            """
                            SELECT COUNT(*)
                            FROM index_files f
                            JOIN index_sources s ON s.id_index_source = f.id_index_source
                            WHERE s.name = %s
                            """,
                            (source_name,),
                        )
                        file_count_row = cur.fetchone()
                        if file_count_row is None or int(file_count_row[0]) != 2:
                            raise AssertionError(f"unexpected indexed file count after clean: {file_count_row}")
                        cur.execute(
                            "SELECT COUNT(*) FROM index_import_plan_entries WHERE id_import_plan = %s",
                            (plan_id,),
                        )
                        entry_count_row = cur.fetchone()
                        if entry_count_row is None or int(entry_count_row[0]) != 2:
                            raise AssertionError(f"unexpected plan entry count after clean: {entry_count_row}")

                print(f"OK fod-indexer clean usability source={clean_root} plan={plan_id}")
    finally:
        cleanup_test_dir(clean_root)
        if dsn is not None:
            try:
                cleanup_materialized_roots_for_sources(dsn, [source_name])
            except Exception:
                pass
            try:
                cleanup_indexer_sources(dsn, [source_name])
            except Exception:
                pass


def main() -> None:
    test_help_and_usage_read_like_a_short_user_guide()
    test_user_journey_surfaces_progress_and_browse_hints()
    test_adb_source_list_surfaces_device_and_browse_root()
    test_clean_user_journey_prunes_stale_rows_without_touching_source_tree()


if __name__ == "__main__":
    main()
