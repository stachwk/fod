#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import json
import os
import sys
import tempfile
import time
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from fod_backend import load_dsn_from_config, load_fod_runtime_config
from tests.integration.fod_mount import FODMount
from tests.integration.fod_runtime_testlib import (
    require_root,
    restore_env,
    run_fod_change,
    set_env,
    wait_for_log_contains,
)


def cleanup_runtime_overrides(dsn: dict[str, str]) -> None:
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        cur.execute("SET search_path TO fod, public")
        cur.execute("DROP TABLE IF EXISTS runtime_overrides")


def load_schema_admin_password() -> str:
    env_password = os.environ.get("FOD_SCHEMA_ADMIN_PASSWORD", "").strip()
    if env_password:
        return env_password
    password_file = ROOT / ".fod/schema-admin-password"
    if password_file.is_file():
        file_password = password_file.read_text(encoding="utf-8").strip()
        if file_password:
            return file_password
    raise RuntimeError(
        "FOD_SCHEMA_ADMIN_PASSWORD is not set and .fod/schema-admin-password is missing; run make reset first."
    )


def main() -> None:
    require_root("tests/integration/test_runtime_reload.py")
    dsn, _ = load_dsn_from_config(ROOT)
    schema_admin_password = load_schema_admin_password()
    apply_env = {
        "POSTGRES_DB": dsn["dbname"],
        "POSTGRES_USER": dsn["user"],
        "POSTGRES_PASSWORD": dsn["password"],
        "FOD_CONFIG": str(ROOT / "fod_config.ini"),
        "FOD_SCHEMA_ADMIN_PASSWORD": schema_admin_password,
    }
    original_apply_env = set_env(apply_env)
    cleanup_runtime_overrides(dsn)

    original_profile = os.environ.get("FOD_PROFILE")
    mount_profile = original_profile or "metadata_heavy"
    original_mount_env = set_env({"FOD_PROFILE": mount_profile})
    launcher = None
    try:
        config_path = ROOT / "fod_config.ini"
        runtime_config = load_fod_runtime_config(config_path)
        original_read_ahead_blocks = str(runtime_config["read_ahead_blocks"])
        updated_read_ahead_blocks = "5" if original_read_ahead_blocks != "5" else "6"

        with tempfile.TemporaryDirectory(prefix="/tmp/fod-runtime-reload.") as mount_dir:
            with FODMount(str(ROOT)) as launcher:
                launcher.init_schema()
                launcher.start(mount_dir)

                set_result = run_fod_change(
                    ROOT,
                    [
                        "--config-path",
                        str(config_path),
                        "--password",
                        schema_admin_password,
                        "--set",
                        f"read_ahead_blocks={updated_read_ahead_blocks}",
                    ],
                )
                set_payload = json.loads(set_result.stdout.strip())
                if set_payload.get("status") != "stored":
                    raise AssertionError(f"unexpected fod.change set response: {set_payload}")
                if not any(
                    item.get("key") == "read_ahead_blocks" and item.get("value") == updated_read_ahead_blocks
                    for item in set_payload.get("live_snapshot", [])
                ):
                    raise AssertionError(
                        f"live snapshot did not contain read_ahead_blocks={updated_read_ahead_blocks}: {set_payload}"
                    )

                log_text = wait_for_log_contains(launcher.config.log_file, "FOD runtime reload applied")
                if f"read_ahead_blocks={updated_read_ahead_blocks}" not in log_text:
                    raise AssertionError(
                        f"reload log did not show read_ahead_blocks={updated_read_ahead_blocks}:\n{log_text}"
                    )

                get_result = run_fod_change(
                    ROOT,
                    [
                        "--config-path",
                        str(config_path),
                        "--get",
                        "read_ahead_blocks",
                    ],
                )
                get_payload = json.loads(get_result.stdout.strip())
                if get_payload.get("status") != "ok":
                    raise AssertionError(f"unexpected fod.change get response: {get_payload}")
                if get_payload.get("value") != updated_read_ahead_blocks:
                    raise AssertionError(
                        f"live snapshot did not expose read_ahead_blocks={updated_read_ahead_blocks}: {get_payload}"
                    )

                rejected = run_fod_change(
                    ROOT,
                    [
                        "--config-path",
                        str(config_path),
                        "--password",
                        schema_admin_password,
                        "--set",
                        "fopen_direct_io=1",
                    ],
                    check=False,
                )
                if rejected.returncode == 0:
                    raise AssertionError("mount-only change was unexpectedly accepted")
                if "fopen_direct_io is not reloadable" not in rejected.stderr:
                    raise AssertionError(f"missing reload rejection in stderr:\n{rejected.stderr}")
                if "restart FOD to change it" not in rejected.stderr:
                    raise AssertionError(f"missing restart hint in stderr:\n{rejected.stderr}")

                verify_result = run_fod_change(
                    ROOT,
                    [
                        "--config-path",
                        str(config_path),
                        "--get",
                        "read_ahead_blocks",
                    ],
                )
                verify_payload = json.loads(verify_result.stdout.strip())
                if verify_payload.get("value") != updated_read_ahead_blocks:
                    raise AssertionError(
                        f"accepted runtime change was altered by a rejected mount-only update: {verify_payload}"
                    )

                restore_result = run_fod_change(
                    ROOT,
                    [
                        "--config-path",
                        str(config_path),
                        "--password",
                        schema_admin_password,
                        "--set",
                        f"read_ahead_blocks={original_read_ahead_blocks}",
                    ],
                )
                restore_payload = json.loads(restore_result.stdout.strip())
                if restore_payload.get("status") != "stored":
                    raise AssertionError(f"unexpected restore response: {restore_payload}")

                restore_deadline = time.monotonic() + 15.0
                restore_log_text = ""
                while time.monotonic() < restore_deadline:
                    restore_log_text = launcher.config.log_file.read_text(encoding="utf-8")
                    if restore_log_text != log_text and f"read_ahead_blocks={original_read_ahead_blocks}" in restore_log_text:
                        break
                    time.sleep(0.1)
                else:
                    raise AssertionError(
                        f"timed out waiting for restored reload log with read_ahead_blocks={original_read_ahead_blocks}:\n{restore_log_text}"
                    )

                restore_get = run_fod_change(
                    ROOT,
                    [
                        "--config-path",
                        str(config_path),
                        "--get",
                        "read_ahead_blocks",
                    ],
                )
                restore_get_payload = json.loads(restore_get.stdout.strip())
                if restore_get_payload.get("value") != original_read_ahead_blocks:
                    raise AssertionError(
                        f"reloadable snapshot was not restored: {restore_get_payload}"
                    )

                print(
                    "OK runtime-reload "
                    f"profile={mount_profile} read_ahead_blocks={updated_read_ahead_blocks} "
                    f"restored={original_read_ahead_blocks}"
                )
    finally:
        restore_env(original_apply_env)
        restore_env(original_mount_env)
        cleanup_runtime_overrides(dsn)


if __name__ == "__main__":
    main()
