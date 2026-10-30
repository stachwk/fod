#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""Verify the client-version floor on a real PostgreSQL primary and standby."""

from __future__ import annotations

import contextlib
import datetime
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def query(connection, sql, params=()):
    with connection.cursor() as cursor:
        cursor.execute(sql, params or None)
        return cursor.fetchall()


def execute(connection, sql, params=()):
    with connection.cursor() as cursor:
        cursor.execute(sql, params or None)


def main() -> None:
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    artifacts = ROOT / "artifacts/integration" / f"client-version-gate-replica-{stamp}"
    artifacts.mkdir(parents=True)

    primary_port = os.environ.get("CLIENT_VERSION_PRIMARY_PORT", "55621")
    replica_port = os.environ.get("CLIENT_VERSION_REPLICA_PORT", "55622")
    project = f"fod-client-version-gate-{os.getpid()}"
    compose_command = [
        "docker",
        "compose",
        "-p",
        project,
        "-f",
        str(ROOT / "docker-compose.replica-read.yml"),
    ]

    def compose(*args, check=True):
        return subprocess.run(
            compose_command + list(args),
            cwd=ROOT,
            check=check,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )

    original_env = {
        key: os.environ.get(key)
        for key in (
            "POSTGRES_DB",
            "POSTGRES_USER",
            "POSTGRES_PASSWORD",
            "POSTGRES_HOST",
            "POSTGRES_PORT",
            "REPLICA_READ_BIND_ADDRESS",
            "REPLICA_READ_PRIMARY_PORT",
            "REPLICA_READ_REPLICA_PORT",
            "FOD_PG_HOST",
            "FOD_PG_PORT",
            "FOD_PG_DBNAME",
            "FOD_PG_USER",
            "FOD_PG_PASSWORD",
            "FOD_PG_ENDPOINT_ROUTING_ENABLED",
            "FOD_PG_RUNTIME_FAILOVER_ENABLED",
            "FOD_PG_REPLICA_READ_ROUTING_ENABLED",
            "FOD_PG_PRIMARY_HOSTS",
            "FOD_PG_REPLICA_HOSTS",
            "FOD_ROLE",
            "FOD_DEGRADED_READONLY_ENABLED",
            "FOD_SCHEMA_ADMIN_PASSWORD",
            "FOD_CONFIG",
            "FOD_ATIME_POLICY",
            "FOD_METADATA_CACHE_TTL_SECONDS",
            "FOD_RUNTIME_PROFILE",
            "FOD_TELEMETRY_DSN",
            "FOD_MONITOR_DSN",
        )
    }
    os.environ.update(
        POSTGRES_DB="fod_client_version_gate_test",
        POSTGRES_USER="foduser",
        POSTGRES_PASSWORD="cichosza",
        POSTGRES_HOST="127.0.0.1",
        POSTGRES_PORT=primary_port,
        REPLICA_READ_BIND_ADDRESS="127.0.0.1",
        REPLICA_READ_PRIMARY_PORT=primary_port,
        REPLICA_READ_REPLICA_PORT=replica_port,
        FOD_PG_HOST="127.0.0.1",
        FOD_PG_PORT=primary_port,
        FOD_PG_DBNAME="fod_client_version_gate_test",
        FOD_PG_USER="foduser",
        FOD_PG_PASSWORD="cichosza",
        FOD_PG_ENDPOINT_ROUTING_ENABLED="1",
        FOD_PG_RUNTIME_FAILOVER_ENABLED="0",
        FOD_PG_REPLICA_READ_ROUTING_ENABLED="0",
        FOD_PG_PRIMARY_HOSTS=f"127.0.0.1:{primary_port}",
        FOD_PG_REPLICA_HOSTS=f"127.0.0.1:{replica_port}",
        FOD_ROLE="primary",
        FOD_SCHEMA_ADMIN_PASSWORD="client-version-gate-test-admin",
        FOD_CONFIG=str(ROOT / "fod_config.ini"),
        FOD_ATIME_POLICY="noatime",
        FOD_METADATA_CACHE_TTL_SECONDS="0",
        FOD_RUNTIME_PROFILE=os.environ.get("FOD_RUNTIME_PROFILE", "release-lto"),
    )
    os.environ.pop("FOD_DEGRADED_READONLY_ENABLED", None)
    for key in ("FOD_TELEMETRY_DSN", "FOD_MONITOR_DSN"):
        os.environ.pop(key, None)

    connections = []
    mounts = []
    results = {}
    primary = None
    replica = None
    replica_replay_paused = False
    temporary_root = Path(tempfile.mkdtemp(prefix="fod-client-version-gate-replica-"))
    try:
        started = compose(
            "up", "-d", "--wait", "--wait-timeout", "90", check=False
        )
        (artifacts / "compose-start.log").write_text(started.stdout, encoding="utf-8")
        if started.returncode != 0:
            raise RuntimeError(
                f"replica test PostgreSQL stack failed to start (rc={started.returncode}); "
                f"see {artifacts / 'compose-start.log'}\n{started.stdout}"
            )

        def connect(port):
            connection = psycopg2.connect(
                host="127.0.0.1",
                port=port,
                dbname=os.environ["POSTGRES_DB"],
                user=os.environ["POSTGRES_USER"],
                password=os.environ["POSTGRES_PASSWORD"],
            )
            connection.autocommit = True
            connections.append(connection)
            return connection

        primary = connect(primary_port)
        replica = connect(replica_port)
        if query(primary, "SELECT pg_is_in_recovery()")[0][0]:
            raise AssertionError("configured primary endpoint is in recovery")
        if not query(replica, "SELECT pg_is_in_recovery()")[0][0]:
            raise AssertionError("configured replica endpoint is not in recovery")

        FODMount(str(ROOT), role="primary").init_schema()

        def wait_for_replica_barrier():
            lsn = query(primary, "SELECT pg_current_wal_flush_lsn()::text")[0][0]
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                replayed = query(
                    replica,
                    "SELECT pg_last_wal_replay_lsn() >= %s::pg_lsn",
                    (lsn,),
                )[0][0]
                if replayed:
                    return lsn
                time.sleep(0.05)
            raise AssertionError(f"physical standby did not replay primary LSN {lsn}")

        def reject_mounts(gate_name, expected_message):
            with tempfile.TemporaryDirectory(
                prefix=f"fod-client-version-gate-{gate_name}-", dir=temporary_root
            ) as mount_temp:
                for role, port in (("primary", primary_port), ("replica", replica_port)):
                    os.environ["FOD_ROLE"] = role
                    if role == "replica":
                        os.environ["FOD_DEGRADED_READONLY_ENABLED"] = "1"
                    else:
                        os.environ.pop("FOD_DEGRADED_READONLY_ENABLED", None)

                    launcher = FODMount(str(ROOT), role=role)
                    mounts.append(launcher)
                    mountpoint = Path(mount_temp) / role
                    try:
                        with contextlib.redirect_stdout(io.StringIO()):
                            launcher.start(
                                str(mountpoint),
                                log_prefix=f"/tmp/fod-client-version-{gate_name}-{role}",
                            )
                    except RuntimeError as error:
                        if "exited before becoming ready" not in str(error):
                            raise
                    else:
                        raise AssertionError(
                            f"gate={gate_name} role={role} unexpectedly mounted"
                        )

                    assert launcher.config is not None
                    log = launcher.config.log_file.read_text(encoding="utf-8")
                    (artifacts / f"mount-{gate_name}-{role}.log").write_text(
                        log, encoding="utf-8"
                    )
                    expected_authority = f"127.0.0.1:{port}"
                    if expected_message not in log:
                        raise AssertionError(
                            f"gate={gate_name} role={role} failed unexpectedly; "
                            f"expected {expected_message!r}\n{log[-6000:]}"
                        )
                    if expected_authority not in log:
                        raise AssertionError(
                            f"role={role} did not report selected endpoint {expected_authority}\n{log[-6000:]}"
                        )
                    expected_observed_role = (
                        'observed_role=Some("replica")'
                        if role == "replica"
                        else 'observed_role=Some("primary-writable")'
                    )
                    if expected_observed_role not in log:
                        raise AssertionError(
                            f"role={role} did not confirm expected PostgreSQL role {expected_observed_role}\n{log[-6000:]}"
                        )
                    if FODMount._is_mountpoint(mountpoint):
                        raise AssertionError(
                            f"role={role} left a mount attached after admission rejection"
                        )
                    results[f"{gate_name}_{role}_rejected"] = True

        def reject_replica_without_primary(gate_name, env_overrides, expected_message):
            changed_keys = set(env_overrides) | {
                "FOD_DEGRADED_READONLY_ENABLED",
                "FOD_ROLE",
            }
            previous = {key: os.environ.get(key) for key in changed_keys}
            os.environ.update(env_overrides)
            os.environ["FOD_ROLE"] = "replica"
            os.environ.pop("FOD_DEGRADED_READONLY_ENABLED", None)
            try:
                with tempfile.TemporaryDirectory(
                    prefix=f"fod-client-version-gate-{gate_name}-",
                    dir=temporary_root,
                ) as mount_temp:
                    launcher = FODMount(str(ROOT), role="replica")
                    mountpoint = Path(mount_temp) / "replica"
                    try:
                        try:
                            with contextlib.redirect_stdout(io.StringIO()):
                                launcher.start(
                                    str(mountpoint),
                                    log_prefix=f"/tmp/fod-client-version-{gate_name}-replica",
                                )
                        except RuntimeError as error:
                            if "exited before becoming ready" not in str(error):
                                raise
                        else:
                            raise AssertionError(
                                f"gate={gate_name} replica unexpectedly mounted"
                            )

                        assert launcher.config is not None
                        log = launcher.config.log_file.read_text(encoding="utf-8")
                        (artifacts / f"mount-{gate_name}-replica.log").write_text(
                            log, encoding="utf-8"
                        )
                        if expected_message not in log:
                            raise AssertionError(
                                f"gate={gate_name} failed for an unexpected reason; "
                                f"expected {expected_message!r}\n{log[-6000:]}"
                            )
                        if FODMount._is_mountpoint(mountpoint):
                            raise AssertionError(
                                f"gate={gate_name} left a mount attached"
                            )
                        results[f"{gate_name}_replica_rejected"] = True
                    finally:
                        launcher.stop()
            finally:
                for key, value in previous.items():
                    if value is None:
                        os.environ.pop(key, None)
                    else:
                        os.environ[key] = value

        def allow_degraded_replica_without_primary():
            changed_keys = {
                "FOD_DEGRADED_READONLY_ENABLED",
                "FOD_PG_PRIMARY_HOSTS",
                "FOD_ROLE",
            }
            previous = {key: os.environ.get(key) for key in changed_keys}
            os.environ["FOD_DEGRADED_READONLY_ENABLED"] = "1"
            os.environ["FOD_PG_PRIMARY_HOSTS"] = "127.0.0.1:1"
            os.environ["FOD_ROLE"] = "replica"
            try:
                with tempfile.TemporaryDirectory(
                    prefix="fod-client-version-gate-degraded-",
                    dir=temporary_root,
                ) as mount_temp:
                    launcher = FODMount(str(ROOT), role="replica")
                    mountpoint = Path(mount_temp) / "replica"
                    try:
                        with contextlib.redirect_stdout(io.StringIO()):
                            launcher.start(
                                str(mountpoint),
                                log_prefix="/tmp/fod-client-version-degraded-replica",
                            )
                        assert launcher.config is not None
                        log = launcher.config.log_file.read_text(encoding="utf-8")
                        (artifacts / "mount-degraded-primary-unavailable-replica.log").write_text(
                            log, encoding="utf-8"
                        )
                        if "admission uses the standby's replayed compatibility state" not in log:
                            raise AssertionError(
                                "degraded mount did not report use of the replayed local compatibility gate\n"
                                + log[-6000:]
                            )
                        results["degraded_replica_without_primary_mounted"] = True
                    finally:
                        launcher.stop()
            finally:
                for key, value in previous.items():
                    if value is None:
                        os.environ.pop(key, None)
                    else:
                        os.environ[key] = value

        wait_for_replica_barrier()
        original_version = query(
            primary,
            "SELECT minimum_client_version FROM fod.schema_version ORDER BY applied_at DESC LIMIT 1",
        )[0][0]
        if not original_version:
            raise AssertionError("initialized schema has no minimum client version")

        execute(
            primary,
            "UPDATE fod.schema_version SET upgrade_in_progress = TRUE",
        )
        barrier_lsn = wait_for_replica_barrier()
        replicated_upgrade_state = query(
            replica,
            "SELECT upgrade_in_progress FROM fod.schema_version ORDER BY applied_at DESC LIMIT 1",
        )[0][0]
        if not replicated_upgrade_state:
            raise AssertionError("physical standby did not replay upgrade-in-progress gate")
        results["upgrade_gate_replicated_lsn"] = barrier_lsn
        reject_mounts(
            "upgrade-in-progress",
            "FOD schema upgrade is in progress or was interrupted",
        )
        execute(
            primary,
            "UPDATE fod.schema_version SET upgrade_in_progress = FALSE",
        )
        wait_for_replica_barrier()

        execute(
            primary,
            "UPDATE fod.schema_version SET schema_number = 31",
        )
        wait_for_replica_barrier()
        reject_mounts(
            "required-schema-version",
            f"FOD client {(ROOT / 'fod_version.txt').read_text(encoding='utf-8').strip()} "
            "requires database schema 32",
        )
        execute(
            primary,
            "UPDATE fod.schema_version SET schema_number = 32",
        )
        wait_for_replica_barrier()

        execute(replica, "SELECT pg_wal_replay_pause()")
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if query(replica, "SELECT pg_is_wal_replay_paused()")[0][0]:
                replica_replay_paused = True
                break
            time.sleep(0.05)
        if not replica_replay_paused:
            raise AssertionError("physical standby did not pause WAL replay")
        execute(
            primary,
            "UPDATE fod.schema_version SET minimum_client_version = %s",
            ("99.0.0",),
        )
        replicated_version = query(
            replica,
            "SELECT minimum_client_version FROM fod.schema_version ORDER BY applied_at DESC LIMIT 1",
        )[0][0]
        if replicated_version != original_version:
            raise AssertionError(
                "standby should retain its old floor while replay is paused: "
                f"expected {original_version!r}, got {replicated_version!r}"
            )
        results["paused_replica_floor"] = {
            "version": replicated_version,
            "primary_floor": "99.0.0",
            "primary_flush_lsn": query(
                primary, "SELECT pg_current_wal_flush_lsn()::text"
            )[0][0],
        }

        reject_mounts(
            "minimum-client-version",
            "older than the database minimum client version 99.0.0",
        )

        execute(
            primary,
            "UPDATE fod.schema_version SET minimum_client_version = %s",
            (original_version,),
        )
        execute(replica, "SELECT pg_wal_replay_resume()")
        replica_replay_paused = False
        wait_for_replica_barrier()
        results["restored_floor"] = original_version
        reject_replica_without_primary(
            "primary-unavailable",
            {"FOD_PG_PRIMARY_HOSTS": "127.0.0.1:1"},
            "replica mount requires an available writable PostgreSQL primary",
        )
        allow_degraded_replica_without_primary()
        print(json.dumps(results, sort_keys=True), flush=True)
    finally:
        for index, launcher in enumerate(reversed(mounts)):
            log_file = launcher.config.log_file if launcher.config is not None else None
            if log_file is not None:
                launcher.stop()
                if log_file.exists():
                    shutil.copyfile(log_file, artifacts / f"mount-cleanup-{index}.log")
        if replica_replay_paused and replica is not None:
            with contextlib.suppress(Exception):
                execute(replica, "SELECT pg_wal_replay_resume()")
        for connection in connections:
            connection.close()
        shutil.rmtree(temporary_root)
        try:
            logs = compose("logs", "--no-color", "primary", "replica", check=False)
            (artifacts / "compose-postgres.log").write_text(
                logs.stdout, encoding="utf-8"
            )
            stopped = compose("down", "-v", "--remove-orphans", check=False)
            (artifacts / "compose-stop.log").write_text(
                stopped.stdout, encoding="utf-8"
            )
        finally:
            for key, value in original_env.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value
        (artifacts / "results.json").write_text(
            json.dumps(results, indent=2), encoding="utf-8"
        )
        print(f"artifacts={artifacts}", flush=True)


if __name__ == "__main__":
    main()
