#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import configparser
import os
import secrets
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from fod_backend import load_dsn_from_config, load_fod_runtime_config
from tests.integration.fod_runtime_testlib import (
    require_root,
    restore_env,
    set_env,
    wait_for_log_contains,
)
from tests.integration.fod_mount import FODMount


def _docker(args: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        ["docker", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if check and result.returncode != 0:
        raise RuntimeError(
            f"docker {' '.join(args)} failed with exit code {result.returncode}\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )
    return result


def _docker_primary_network(container_name: str = "fod-postgres") -> str:
    result = _docker(
        [
            "inspect",
            "-f",
            "{{range $k, $_ := .NetworkSettings.Networks}}{{println $k}}{{end}}",
            container_name,
        ]
    )
    networks = [line.strip() for line in result.stdout.splitlines() if line.strip()]
    if not networks:
        raise RuntimeError(f"could not determine Docker network for {container_name}")
    return networks[0]


def _pick_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _write_recovery_config(source_config: Path, destination_dir: Path, port: int) -> Path:
    source_dsn, _ = load_dsn_from_config(source_config)
    parser = configparser.ConfigParser(interpolation=None)
    if not parser.read(source_config):
        raise FileNotFoundError(f"failed to read FOD config: {source_config}")
    if "database" not in parser:
        raise ValueError(f"missing [database] section in {source_config}")
    parser["database"]["host"] = "127.0.0.1"
    parser["database"]["port"] = str(port)
    for key in ("sslmode", "sslrootcert", "sslcert", "sslkey"):
        if key in source_dsn:
            parser["database"][key] = source_dsn[key]
    config_path = destination_dir / "fod_config.ini"
    with config_path.open("w", encoding="utf-8") as handle:
        parser.write(handle)
    return config_path


def _wait_for_recovery_database(dsn: dict[str, str], container_name: str) -> None:
    deadline = time.monotonic() + 120.0
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
                cur.execute("SHOW transaction_read_only")
                transaction_read_only = cur.fetchone()[0]
                cur.execute("SELECT pg_is_in_recovery()")
                is_in_recovery = cur.fetchone()[0]
                if is_in_recovery and transaction_read_only == "on":
                    return
        except Exception as exc:  # pragma: no cover - failure path only
            last_error = exc
        time.sleep(0.5)
    logs = _docker(["logs", container_name], check=False)
    raise AssertionError(
        "timed out waiting for a true recovery database:\n"
        f"dsn={dsn}\n"
        f"last_error={last_error}\n"
        f"stdout:\n{logs.stdout}\n"
        f"stderr:\n{logs.stderr}"
    )


def main() -> None:
    require_root("tests/integration/test_runtime_profile.py")
    dsn, _ = load_dsn_from_config(ROOT)
    config_path = ROOT / "fod_config.ini"
    original_profile = os.environ.get("FOD_PROFILE")
    original_sync_commit = os.environ.get("FOD_SYNCHRONOUS_COMMIT")
    mount_profile = original_profile or "metadata_heavy"
    mounted_enable_extents_text = "true" if mount_profile == "extents" else "false"
    os.environ.pop("FOD_SYNCHRONOUS_COMMIT", None)
    os.environ.pop("FOD_PROFILE", None)
    base_runtime_config = load_fod_runtime_config(config_path)

    profile_expectations = [
        (
            "bulk_write",
            {
                "write_flush_threshold_bytes": "268435456",
                "read_cache_blocks": "1024",
                "read_ahead_blocks": "4",
                "sequential_read_ahead_blocks": "8",
                "small_file_read_threshold_blocks": "8",
                "workers_read": "4",
                "workers_read_min_blocks": "8",
                "workers_write": "8",
                "workers_write_min_blocks": "16",
                "persist_buffer_chunk_blocks": "512",
                "enable_extents": "false",
                "metadata_cache_ttl_seconds": "1",
                "statfs_cache_ttl_seconds": "1",
                "lock_poll_interval_seconds": "0.1",
            },
        ),
        (
            "metadata_heavy",
            {
                "write_flush_threshold_bytes": "67108864",
                "read_cache_blocks": "2048",
                "read_ahead_blocks": "4",
                "sequential_read_ahead_blocks": "8",
                "small_file_read_threshold_blocks": "16",
                "workers_read": "2",
                "workers_read_min_blocks": "16",
                "workers_write": "2",
                "workers_write_min_blocks": "16",
                "persist_buffer_chunk_blocks": "64",
                "enable_extents": "false",
                "metadata_cache_ttl_seconds": "10",
                "statfs_cache_ttl_seconds": "10",
                "lock_poll_interval_seconds": "0.1",
            },
        ),
        (
            "pg_locking",
            {
                "workers_read": "1",
                "workers_read_min_blocks": "16",
                "workers_write": "1",
                "workers_write_min_blocks": "16",
                "persist_buffer_chunk_blocks": "64",
                "enable_extents": "false",
                "metadata_cache_ttl_seconds": "1",
                "statfs_cache_ttl_seconds": "1",
                "lock_poll_interval_seconds": "0.05",
            },
        ),
        (
            "extents",
            {
                "enable_extents": "true",
            },
        ),
    ]

    try:
        for profile_name, expectations in profile_expectations:
            os.environ["FOD_PROFILE"] = profile_name
            runtime_config = load_fod_runtime_config(config_path)
            assert runtime_config["profile"] == profile_name, runtime_config
            assert runtime_config["synchronous_commit"] == "on", runtime_config
            for attr_name, expected_value in expectations.items():
                assert runtime_config[attr_name] == expected_value, (
                    profile_name,
                    attr_name,
                    runtime_config[attr_name],
                )
            if profile_name == "extents":
                for attr_name, expected_value in base_runtime_config.items():
                    if attr_name in {"profile", "enable_extents"}:
                        continue
                    assert runtime_config[attr_name] == expected_value, (
                        profile_name,
                        attr_name,
                        runtime_config[attr_name],
                        expected_value,
                    )
            with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
                cur.execute("SHOW synchronous_commit")
                assert cur.fetchone()[0] == "on", profile_name

        visible_dir = tempfile.TemporaryDirectory(prefix="/tmp/fod-runtime-visible.")
        mount_overrides = {
            "FOD_PROFILE": mount_profile,
            "FOD_METADATA_CACHE_TTL_SECONDS": "7",
            "FOD_STATFS_CACHE_TTL_SECONDS": "11",
            "FOD_WORKERS_READ": "3",
            "FOD_WORKERS_WRITE": "5",
            "FOD_PG_VISIBLE_PATH": visible_dir.name,
        }
        original_mount_env = set_env(mount_overrides)
        launcher = None
        temp_dir = None
        try:
            launcher = FODMount(str(ROOT))
            launcher.init_schema()
            temp_dir = tempfile.TemporaryDirectory(prefix="/tmp/fod-runtime-profile.")
            launcher.start(temp_dir.name)
            log_text = wait_for_log_contains(launcher.config.log_file, "FOD storage block_size=")
            assert f'profile=Some("{mount_profile}")' in log_text, log_text
            expected_cache_line = (
                "FOD cache metadata_cache_ttl=7s statfs_cache_ttl=11s read_cache_blocks=2048 read_cache_eviction_policy=fifo read_ahead_blocks=4 "
                "sequential_read_ahead_blocks=8 small_file_read_threshold_blocks=16"
                if mount_profile == "metadata_heavy"
                else "FOD cache metadata_cache_ttl=7s statfs_cache_ttl=11s read_cache_blocks=4096 read_cache_eviction_policy=fifo read_ahead_blocks=4 "
                "sequential_read_ahead_blocks=8 small_file_read_threshold_blocks=8"
            )
            expected_storage_line = (
                f'pg_visible_path=Some("{visible_dir.name}") workers_read=3 workers_read_min_blocks=16 '
                "workers_write=5 workers_write_min_blocks=16 persist_buffer_chunk_blocks=64 "
                "persist_block_transport=copy_binary_staging synchronous_commit=on copy_dedupe_enabled=false "
                f"copy_dedupe_min_blocks=16 copy_dedupe_max_blocks=0 copy_dedupe_crc_table=false enable_extents=false"
                if mount_profile == "metadata_heavy"
                else f'pg_visible_path=Some("{visible_dir.name}") workers_read=3 workers_read_min_blocks=8 '
                "workers_write=5 workers_write_min_blocks=8 persist_buffer_chunk_blocks=128 "
                "persist_block_transport=copy_binary_staging synchronous_commit=on copy_dedupe_enabled=false "
                f"copy_dedupe_min_blocks=16 copy_dedupe_max_blocks=0 copy_dedupe_crc_table=false enable_extents=true"
            )
            assert (
                expected_cache_line
                in log_text
            ), log_text
            assert (
                expected_storage_line.replace("enable_extents=false", f"enable_extents={mounted_enable_extents_text}")
                in log_text
            ), log_text
            print("OK runtime-profile-mount")
        finally:
            if launcher is not None:
                launcher.stop()
            if temp_dir is not None:
                temp_dir.cleanup()
            visible_dir.cleanup()
            restore_env(original_mount_env)

        sync_commit_overrides = {
            "FOD_PROFILE": mount_profile,
            "FOD_SYNCHRONOUS_COMMIT": "off",
        }
        original_sync_commit_mount_env = set_env(sync_commit_overrides)
        sync_launcher = None
        sync_temp_dir = None
        try:
            sync_launcher = FODMount(str(ROOT))
            sync_launcher.init_schema()
            sync_temp_dir = tempfile.TemporaryDirectory(prefix="/tmp/fod-runtime-sync-commit.")
            sync_launcher.start(sync_temp_dir.name)
            sync_log_text = wait_for_log_contains(sync_launcher.config.log_file, "FOD storage block_size=")
            assert "synchronous_commit=off" in sync_log_text, sync_log_text
            print("OK runtime-profile-sync-commit")
        finally:
            if sync_launcher is not None:
                sync_launcher.stop()
            if sync_temp_dir is not None:
                sync_temp_dir.cleanup()
            restore_env(original_sync_commit_mount_env)

        read_only_overrides = {
            "FOD_PROFILE": mount_profile,
            "FOD_RUST_FUSE_READONLY": "1",
        }
        original_read_only_mount_env = set_env(read_only_overrides)
        read_only_launcher = None
        read_only_temp_dir = None
        try:
            read_only_launcher = FODMount(str(ROOT))
            read_only_launcher.init_schema()
            read_only_temp_dir = tempfile.TemporaryDirectory(prefix="/tmp/fod-runtime-read-only.")
            read_only_launcher.start(read_only_temp_dir.name)
            read_only_log_text = wait_for_log_contains(
                read_only_launcher.config.log_file,
                "FOD mount read_only=true",
            )
            assert "FOD core role=auto" in read_only_log_text, read_only_log_text
            assert "force_read_only=true" in read_only_log_text, read_only_log_text
            assert "FOD mount read_only=true" in read_only_log_text, read_only_log_text
            assert "FOD lock backend=Memory" in read_only_log_text, read_only_log_text
            assert "FOD mount options:" in read_only_log_text, read_only_log_text
            print("OK runtime-profile-read-only")
        finally:
            if read_only_launcher is not None:
                read_only_launcher.stop()
            if read_only_temp_dir is not None:
                read_only_temp_dir.cleanup()
            restore_env(original_read_only_mount_env)

        selinux_mount_overrides = {
            "FOD_PROFILE": mount_profile,
            "FOD_SELINUX": "off",
            "FOD_SELINUX_CONTEXT": "system_u:object_r:tmp_t:s0",
            "FOD_SELINUX_FSCONTEXT": "system_u:object_r:fusefs_t:s0",
            "FOD_SELINUX_DEFCONTEXT": "system_u:object_r:fusefs_t:s0",
            "FOD_SELINUX_ROOTCONTEXT": "system_u:object_r:fusefs_t:s0",
        }
        original_selinux_mount_env = set_env(selinux_mount_overrides)
        selinux_launcher = None
        selinux_temp_dir = None
        try:
            selinux_launcher = FODMount(str(ROOT))
            selinux_launcher.init_schema()
            selinux_temp_dir = tempfile.TemporaryDirectory(prefix="/tmp/fod-runtime-selinux.")
            try:
                selinux_launcher.start(selinux_temp_dir.name)
            except RuntimeError as exc:
                selinux_log_text = ""
                if selinux_launcher.config is not None and selinux_launcher.config.log_file.exists():
                    selinux_log_text = selinux_launcher.config.log_file.read_text(encoding="utf-8")
                if "Invalid argument" in selinux_log_text or "mount failed" in selinux_log_text:
                    print("SKIP runtime-profile-selinux-contexts (mount labels unsupported on this host)")
                else:
                    raise
            else:
                selinux_log_text = wait_for_log_contains(
                    selinux_launcher.config.log_file,
                    "FOD security selinux_enabled=false",
                )
                assert 'context=Some("system_u:object_r:tmp_t:s0")' in selinux_log_text, selinux_log_text
                assert 'fscontext=Some("system_u:object_r:fusefs_t:s0")' in selinux_log_text, selinux_log_text
                assert 'defcontext=Some("system_u:object_r:fusefs_t:s0")' in selinux_log_text, selinux_log_text
                assert 'rootcontext=Some("system_u:object_r:fusefs_t:s0")' in selinux_log_text, selinux_log_text
                assert "context=system_u:object_r:tmp_t:s0" in selinux_log_text, selinux_log_text
                assert "fscontext=system_u:object_r:fusefs_t:s0" in selinux_log_text, selinux_log_text
                assert "defcontext=system_u:object_r:fusefs_t:s0" in selinux_log_text, selinux_log_text
                assert "rootcontext=system_u:object_r:fusefs_t:s0" in selinux_log_text, selinux_log_text
                assert "FOD mount options:" in selinux_log_text, selinux_log_text
                print("OK runtime-profile-selinux-contexts")
        finally:
            if selinux_launcher is not None:
                selinux_launcher.stop()
            if selinux_temp_dir is not None:
                selinux_temp_dir.cleanup()
            restore_env(original_selinux_mount_env)

        # Create a temporary standby so role=auto sees a real recovery database.
        recovery_launcher = None
        recovery_temp_dir = None
        standby_container_name = f"fod-postgres-recovery-{secrets.token_hex(4)}"
        standby_data_dir = tempfile.TemporaryDirectory(prefix="/tmp/fod-runtime-recovery-data.")
        standby_config_dir = tempfile.TemporaryDirectory(prefix="/tmp/fod-runtime-recovery-config.")
        recovery_port = _pick_free_port()
        standby_backup_path = f"/tmp/fod-runtime-recovery-basebackup-{secrets.token_hex(4)}"
        try:
            FODMount(str(ROOT)).init_schema()
            primary_network = _docker_primary_network()
            standby_data_path = Path(standby_data_dir.name)
            standby_data_path.chmod(0o777)
            standby_pgdata = standby_data_path / "pgdata"
            standby_pgdata.mkdir(parents=True, exist_ok=True)
            _docker(
                [
                    "exec",
                    "-u",
                    "postgres",
                    "fod-postgres",
                    "sh",
                    "-lc",
                    f"mkdir -p {standby_backup_path} && "
                    "pg_basebackup -h 127.0.0.1 -p 5432 -U "
                    f"{dsn['user']} -D {standby_backup_path} -Fp -Xs -P",
                ]
            )
            _docker(
                [
                    "cp",
                    f"fod-postgres:{standby_backup_path}/.",
                    str(standby_pgdata),
                ]
            )
            (standby_pgdata / "standby.signal").touch()
            (standby_pgdata / "postgresql.auto.conf").write_text(
                "primary_conninfo = 'host=fod-postgres port=5432 "
                f"user={dsn['user']} password={dsn['password']}'\n",
                encoding="utf-8",
            )
            recovery_config_path = _write_recovery_config(
                config_path,
                Path(standby_config_dir.name),
                recovery_port,
            )
            standby_result = _docker(
                [
                    "run",
                    "-d",
                    "--name",
                    standby_container_name,
                    "--network",
                    primary_network,
                    "-p",
                    f"{recovery_port}:5432",
                    "-e",
                    f"POSTGRES_DB={dsn['dbname']}",
                    "-e",
                    f"POSTGRES_USER={dsn['user']}",
                    "-e",
                    f"POSTGRES_PASSWORD={dsn['password']}",
                    "-e",
                    "PGDATA=/var/lib/postgresql/data/pgdata",
                    "-v",
                    f"{standby_data_path}:/var/lib/postgresql/data",
                    "postgres:16-alpine",
                ]
            )
            if not standby_result.stdout.strip():
                raise RuntimeError("failed to start recovery standby container")
            recovery_dsn, _ = load_dsn_from_config(recovery_config_path)
            _wait_for_recovery_database(recovery_dsn, standby_container_name)

            original_recovery_mount_env = set_env(
                {
                    "FOD_CONFIG": str(recovery_config_path),
                    "FOD_PROFILE": mount_profile,
                }
            )
            try:
                recovery_launcher = FODMount(str(ROOT))
                recovery_temp_dir = tempfile.TemporaryDirectory(prefix="/tmp/fod-runtime-recovery.")
                recovery_launcher.start(recovery_temp_dir.name)
                recovery_log_text = wait_for_log_contains(
                    recovery_launcher.config.log_file,
                    "FOD recovery_mode=true",
                )
                assert "FOD core role=auto" in recovery_log_text, recovery_log_text
                assert "force_read_only=false" in recovery_log_text, recovery_log_text
                assert "FOD mount read_only=true" in recovery_log_text, recovery_log_text
                assert "FOD lock backend=Memory" in recovery_log_text, recovery_log_text
                assert f"enable_extents={mounted_enable_extents_text}" in recovery_log_text, recovery_log_text
                assert "FOD mount options:" in recovery_log_text, recovery_log_text
                print("OK runtime-profile-auto-recovery")
            finally:
                if recovery_launcher is not None:
                    recovery_launcher.stop()
                if recovery_temp_dir is not None:
                    recovery_temp_dir.cleanup()
                restore_env(original_recovery_mount_env)
        finally:
            _docker(["rm", "-f", standby_container_name], check=False)
            _docker(["exec", "-u", "postgres", "fod-postgres", "rm", "-rf", standby_backup_path], check=False)
            standby_data_dir.cleanup()
            standby_config_dir.cleanup()
        print("OK runtime-profile")
    finally:
        if original_profile is None:
            os.environ.pop("FOD_PROFILE", None)
        else:
            os.environ["FOD_PROFILE"] = original_profile
        if original_sync_commit is None:
            os.environ.pop("FOD_SYNCHRONOUS_COMMIT", None)
        else:
            os.environ["FOD_SYNCHRONOUS_COMMIT"] = original_sync_commit


if __name__ == "__main__":
    main()
