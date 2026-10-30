#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import tempfile
import sys
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def set_minimum_client_version(launcher: FODMount, version: str) -> None:
    connection = psycopg2.connect(
        host=os.environ.get("FOD_PG_HOST") or os.environ.get("POSTGRES_HOST") or "127.0.0.1",
        port=os.environ.get("FOD_PG_PORT") or os.environ.get("POSTGRES_PORT") or "5432",
        dbname=launcher.postgres_db,
        user=launcher.postgres_user,
        password=launcher.postgres_password,
    )
    try:
        with connection:
            with connection.cursor() as cursor:
                cursor.execute(
                    "UPDATE fod.schema_version SET minimum_client_version = %s",
                    (version,),
                )
                if cursor.rowcount != 1:
                    raise AssertionError(
                        f"expected one schema_version row, updated {cursor.rowcount}"
                    )
    finally:
        connection.close()


def assert_mount_rejected_below_floor(role: str, mountpoint: Path) -> None:
    launcher = FODMount(str(ROOT), role=role)
    try:
        try:
            launcher.start(str(mountpoint), log_prefix=f"/tmp/fod-client-version-{role}")
        except RuntimeError as error:
            if "exited before becoming ready" not in str(error):
                raise
        else:
            raise AssertionError(f"FOD unexpectedly mounted with role={role}")

        assert launcher.config is not None
        log = launcher.config.log_file.read_text(encoding="utf-8")
        expected = "older than the database minimum client version 99.0.0"
        if expected not in log:
            raise AssertionError(
                f"role={role} mount failed for an unexpected reason; expected {expected!r}\n{log}"
            )
        if FODMount._is_mountpoint(mountpoint):
            raise AssertionError(f"role={role} left a mount attached after version rejection")
        print(f"OK client-version-gate role={role} rejected below minimum")
    finally:
        launcher.stop()


def main() -> None:
    setup = FODMount(str(ROOT), role="primary")
    setup.init_schema()

    connection = psycopg2.connect(
        host=os.environ.get("FOD_PG_HOST") or os.environ.get("POSTGRES_HOST") or "127.0.0.1",
        port=os.environ.get("FOD_PG_PORT") or os.environ.get("POSTGRES_PORT") or "5432",
        dbname=setup.postgres_db,
        user=setup.postgres_user,
        password=setup.postgres_password,
    )
    try:
        with connection.cursor() as cursor:
            cursor.execute(
                "SELECT minimum_client_version FROM fod.schema_version ORDER BY applied_at DESC LIMIT 1"
            )
            row = cursor.fetchone()
            if row is None or not row[0]:
                raise AssertionError("initialized database has no minimum client version")
            original_version = row[0]
    finally:
        connection.close()

    try:
        set_minimum_client_version(setup, "99.0.0")
        with tempfile.TemporaryDirectory(prefix="fod-client-version-gate-") as temp_dir:
            root = Path(temp_dir)
            for role in ("primary", "replica"):
                assert_mount_rejected_below_floor(role, root / role)
    finally:
        set_minimum_client_version(setup, original_version)


if __name__ == "__main__":
    main()
