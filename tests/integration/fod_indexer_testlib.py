#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import hashlib
import os
import shlex
import shutil
import subprocess
import time
from pathlib import Path
from typing import Mapping

import psycopg2

TEST_RUN_TOKEN = f"{os.getpid()}_{time.time_ns()}"


def _safe_name(value: str) -> str:
    safe = "".join(ch if ch.isalnum() or ch in "-_" else "_" for ch in value)
    return safe.strip("-_") or "indexer"


def unique_indexer_name(prefix: str) -> str:
    return f"{_safe_name(prefix)}_{TEST_RUN_TOKEN}"


def unique_source_name(prefix: str) -> str:
    return unique_indexer_name(prefix)


def unique_indexer_path(prefix: str) -> Path:
    return Path("/tmp") / f"fod-indexer-{unique_indexer_name(prefix)}"


def _assert_test_tmp_path(path: Path) -> None:
    resolved = path.resolve()
    if not resolved.as_posix().startswith("/tmp/fod-indexer-"):
        raise ValueError(f"refusing to remove non-indexer test path: {resolved}")


def prepare_clean_dir(path: Path) -> Path:
    _assert_test_tmp_path(path)
    shutil.rmtree(path, ignore_errors=True)
    path.mkdir(parents=True, exist_ok=True)
    return path


def cleanup_test_dir(path: Path) -> None:
    _assert_test_tmp_path(path)
    shutil.rmtree(path, ignore_errors=True)


def sha256_hex(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(128 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def snapshot_tree(root: Path) -> list[tuple[str, ...]]:
    entries: list[tuple[str, ...]] = []
    for current, dirs, files in os.walk(root):
        current_path = Path(current)
        for dirname in sorted(dirs):
            entries.append(("dir", (current_path / dirname).relative_to(root).as_posix()))
        for filename in sorted(files):
            path = current_path / filename
            entries.append(
                (
                    "file",
                    path.relative_to(root).as_posix(),
                    str(path.stat().st_size),
                    sha256_hex(path),
                )
            )
    return sorted(entries)


def write_tree(root: Path, files: Mapping[str, bytes]) -> None:
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True, exist_ok=True)
    for name, content in files.items():
        (root / name).write_bytes(content)


def apply_database_env(root: Path, dsn: dict[str, str]) -> None:
    os.environ["FOD_CONFIG"] = str(root / "fod_config.ini")
    os.environ["POSTGRES_DB"] = dsn["dbname"]
    os.environ["POSTGRES_USER"] = dsn["user"]
    os.environ["POSTGRES_PASSWORD"] = dsn["password"]
    os.environ["POSTGRES_HOST"] = dsn["host"]
    os.environ["POSTGRES_PORT"] = dsn["port"]


def set_fod_search_path(conn) -> None:
    with conn.cursor() as cur:
        cur.execute("SET search_path TO fod, public")


def cleanup_indexer_state(dsn: dict[str, str]) -> None:
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        set_fod_search_path(conn)
        cur.execute(
            """
            TRUNCATE TABLE
                index_import_plan_entries,
                index_import_plans,
                index_duplicate_sets,
                index_file_hashes,
                index_scan_runs,
                index_files,
                index_sources
            CASCADE
            """
        )


def cleanup_materialized_root(dsn: dict[str, str], root_name: str) -> None:
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        set_fod_search_path(conn)
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
            SELECT f.id_file, f.data_object_id
            FROM files f
            WHERE f.id_directory IN (SELECT id_directory FROM subtree)
            """,
            (root_name,),
        )
        rows = cur.fetchall()
        file_ids = [int(row[0]) for row in rows]
        data_object_ids = [int(row[1]) for row in rows]
        if data_object_ids:
            cur.execute("DELETE FROM data_blocks WHERE data_object_id = ANY(%s)", (data_object_ids,))
            cur.execute("DELETE FROM data_extents WHERE data_object_id = ANY(%s)", (data_object_ids,))
            cur.execute("DELETE FROM copy_block_crc WHERE data_object_id = ANY(%s)", (data_object_ids,))
            cur.execute("DELETE FROM files WHERE id_file = ANY(%s)", (file_ids,))
            cur.execute("DELETE FROM data_objects WHERE id_data_object = ANY(%s)", (data_object_ids,))
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
            DELETE FROM directories WHERE id_directory IN (SELECT id_directory FROM subtree)
            """,
            (root_name,),
        )


def cleanup_materialized_roots(dsn: dict[str, str]) -> None:
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        set_fod_search_path(conn)
        cur.execute(
            """
            SELECT name
            FROM directories
            WHERE id_parent IS NULL AND name LIKE 'index-source-%'
            ORDER BY name
            """
        )
        root_names = [str(row[0]) for row in cur.fetchall()]
    for root_name in root_names:
        cleanup_materialized_root(dsn, root_name)


def materialized_root_names_for_sources(
    dsn: dict[str, str], source_names: list[str]
) -> list[str]:
    if not source_names:
        return []
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        set_fod_search_path(conn)
        cur.execute(
            """
            SELECT id_index_source
            FROM index_sources
            WHERE name = ANY(%s)
            ORDER BY id_index_source
            """,
            (source_names,),
        )
        source_ids = [int(row[0]) for row in cur.fetchall()]
        if not source_ids:
            return []
        patterns = [f"index-source-{source_id}-import-%" for source_id in source_ids]
        cur.execute(
            """
            SELECT name
            FROM directories
            WHERE id_parent IS NULL
              AND name LIKE ANY(%s)
            ORDER BY name
            """,
            (patterns,),
        )
        return [str(row[0]) for row in cur.fetchall()]


def cleanup_materialized_roots_for_sources(
    dsn: dict[str, str], source_names: list[str]
) -> None:
    for root_name in materialized_root_names_for_sources(dsn, source_names):
        cleanup_materialized_root(dsn, root_name)


def cleanup_indexer_sources(dsn: dict[str, str], source_names: list[str]) -> None:
    if not source_names:
        return
    cleanup_materialized_roots_for_sources(dsn, source_names)
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        set_fod_search_path(conn)
        cur.execute(
            """
            DELETE FROM index_import_plans p
            WHERE p.source_filter = ANY(%s)
               OR EXISTS (
                    SELECT 1
                    FROM index_import_plan_entries e
                    JOIN index_files f ON f.id_file = e.id_file
                    JOIN index_sources s
                      ON s.id_index_source = f.id_index_source
                    WHERE e.id_import_plan = p.id_import_plan
                      AND s.name = ANY(%s)
               )
            """,
            (source_names, source_names),
        )
        cur.execute(
            """
            DELETE FROM index_sources
            WHERE name = ANY(%s)
            """,
            (source_names,),
        )


def run_indexer(root: Path, args: list[str]) -> str:
    env = os.environ.copy()
    env.pop("DATABASE_URL", None)
    env.pop("FOD_INDEXER_CONNINFO", None)
    env["INDEXER_ARGS"] = shlex.join(args)
    result = subprocess.run(
        ["make", "--no-print-directory", "indexer"],
        cwd=root,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    output = result.stdout + result.stderr
    if result.returncode != 0:
        raise AssertionError(
            f"fod-indexer command failed: {' '.join(args)}\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )
    return output


def assert_contains(text: str, needle: str, label: str) -> None:
    if needle not in text:
        raise AssertionError(f"missing {needle!r} in {label} output:\n{text}")


def fetch_one(conn, sql: str, params: tuple[object, ...] = ()) -> object:
    with conn.cursor() as cur:
        cur.execute(sql, params)
        row = cur.fetchone()
        if row is None:
            raise AssertionError(f"query returned no rows:\n{sql}")
        return row[0] if len(row) == 1 else row


def fetch_all(conn, sql: str, params: tuple[object, ...] = ()) -> list[tuple[object, ...]]:
    with conn.cursor() as cur:
        cur.execute(sql, params)
        return list(cur.fetchall())


def wait_for_mount_children(mount_root: Path, expected_names: list[str], timeout_s: float = 10.0) -> None:
    deadline = time.monotonic() + timeout_s
    expected = sorted(expected_names)
    last_names: list[str] = []
    while time.monotonic() < deadline:
        try:
            if mount_root.is_dir():
                last_names = sorted(child.name for child in mount_root.iterdir())
                if last_names == expected:
                    return
        except FileNotFoundError:
            pass
        time.sleep(0.2)
    raise AssertionError(
        f"timed out waiting for {mount_root} to show {expected}; last names={last_names}"
    )
