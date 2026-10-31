#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import hashlib
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
)
from tests.integration.fod_mount import FODMount


def run_indexer_json(args: list[str]) -> dict[str, object]:
    env = os.environ.copy()
    env.pop("DATABASE_URL", None)
    env.pop("FOD_INDEXER_CONNINFO", None)
    indexer_bin = env.get("FOD_INDEXER_BIN")
    if indexer_bin:
        command = [indexer_bin, *args]
    else:
        env["INDEXER_ARGS"] = shlex.join(args)
        command = ["make", "--no-print-directory", "indexer"]
    result = subprocess.run(
        command,
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(
            f"fod-indexer command failed: {' '.join(args)}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    try:
        value = json.loads(result.stdout.strip())
    except json.JSONDecodeError as err:
        raise AssertionError(f"invalid indexer JSON output: {err}\n{result.stdout}") from err
    if not isinstance(value, dict):
        raise AssertionError(f"unexpected indexer JSON value: {value!r}")
    return value


def digest_stream(label: str, length: int) -> str:
    chunks: list[str] = []
    counter = 0
    while sum(map(len, chunks)) < length:
        chunks.append(hashlib.sha256(f"{label}:{counter}".encode()).hexdigest())
        counter += 1
    return "".join(chunks)[:length]


def main() -> None:
    dsn, _ = load_dsn_from_config(ROOT)
    apply_database_env(ROOT, dsn)
    source_name = digest_stream(f"long-source-{os.getpid()}", 3000)
    source_root = prepare_clean_dir(unique_indexer_path("long-path"))
    snapshot_id: int | None = None
    cleanup_indexer_sources(dsn, [source_name])

    relative_parts = [digest_stream(f"component-{i}", 100) for i in range(30)]
    relative_parts.append("payload.txt")
    relative_path = "/".join(relative_parts)
    if len(relative_path.encode()) <= 2700:
        raise AssertionError("fixture path is not long enough to exercise an 8 KiB B-tree page")

    try:
        file_path = source_root.joinpath(*relative_parts)
        file_path.parent.mkdir(parents=True)
        file_path.write_bytes(b"long path index fixture\n")
        if len(str(file_path).encode()) >= 4096:
            raise AssertionError("fixture exceeds the Linux pathname limit before reaching FOD")

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

            scan = run_indexer_json(["--output", "json", "scan", "--source", source_name])
            if scan.get("scanned_files") != 1:
                raise AssertionError(f"long path was not indexed: {scan}")
            rescanned = run_indexer_json(
                ["--output", "json", "scan", "--source", source_name]
            )
            if rescanned.get("scanned_files") != 1:
                raise AssertionError(f"long path was not re-indexed: {rescanned}")

            created = run_indexer_json(
                ["--output", "json", "snapshot", "create", "--source", source_name]
            )
            snapshot = created.get("snapshot")
            if not isinstance(snapshot, dict) or snapshot.get("file_count") != 1:
                raise AssertionError(f"long path snapshot was not created: {created}")
            snapshot_id = snapshot.get("snapshot_id")
            if not isinstance(snapshot_id, int):
                raise AssertionError(f"snapshot response has no ID: {created}")

            files = run_indexer_json(
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
            items = files.get("items")
            if (
                not isinstance(items, list)
                or len(items) != 1
                or not isinstance(items[0], dict)
            ):
                raise AssertionError(f"long path snapshot row is missing: {files}")
            if items[0].get("path") != relative_path:
                raise AssertionError("snapshot did not preserve the complete long path")

            print(
                "OK fod-indexer long paths "
                f"source_name_bytes={len(source_name)} path_bytes={len(relative_path)}"
            )
    finally:
        if snapshot_id is not None:
            try:
                run_indexer_json(["--output", "json", "snapshot", "delete", "--id", str(snapshot_id)])
            except Exception:
                pass
        cleanup_test_dir(source_root)
        try:
            cleanup_indexer_sources(dsn, [source_name])
        except Exception:
            pass


if __name__ == "__main__":
    main()
