#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

from fod_mount import FODMount


def runtime_profile() -> str:
    return (
        os.environ.get("FOD_RUNTIME_PROFILE")
        or os.environ.get("FOD_CARGO_PROFILE")
        or "release-lto"
    )


def artifact_profile() -> str:
    profile = runtime_profile()
    return "debug" if profile == "dev" else profile


def runtime_binary(root: Path, name: str, env_name: str) -> Path:
    configured = os.environ.get(env_name, "").strip()
    candidates = []
    if configured:
        candidates.append(Path(configured))
    candidates.extend(
        [
            root / "target" / artifact_profile() / name,
            root / "rust_monitor" / "target" / artifact_profile() / name,
            root / "rust_mkfs" / "target" / artifact_profile() / name,
        ]
    )
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise RuntimeError(f"{name} binary not found")


def monitor_binary(root: Path) -> Path:
    return runtime_binary(root, "fod-monitor", "FOD_MONITOR_BIN")


def mkfs_binary(root: Path) -> Path:
    return runtime_binary(root, "fod-rust-mkfs", "FOD_MKFS_BIN")


def report_json(root: Path, env: dict[str, str]) -> dict:
    result = subprocess.run(
        [str(monitor_binary(root)), "report", "--json"],
        cwd=root,
        env=env,
        capture_output=True,
        text=True,
        check=False,
        timeout=15,
    )
    if result.returncode != 0:
        raise RuntimeError(
            "fod-monitor report --json failed "
            f"rc={result.returncode} stderr={result.stderr.strip()}"
        )
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(
            f"fod-monitor report --json returned invalid JSON: {exc}"
        ) from exc


def matching_session(payload: dict, mountpoint: Path) -> dict | None:
    cluster = payload.get("cluster")
    if not isinstance(cluster, dict):
        return None
    sessions = cluster.get("sessions", [])
    mountpoint_text = str(mountpoint)
    for session in sessions:
        if session.get("mountpoint") == mountpoint_text:
            return session
    return None


def wait_for_aggregated_report(
    root: Path,
    env: dict[str, str],
    mountpoint: Path,
    timeout_seconds: float,
) -> dict:
    deadline = time.monotonic() + timeout_seconds
    last_payload = {}
    while time.monotonic() < deadline:
        last_payload = report_json(root, env)
        session = matching_session(last_payload, mountpoint)
        stats = session.get("stats") if session else None
        fuse = stats.get("fuse_compatibility") if isinstance(stats, dict) else None
        if (
            isinstance(last_payload.get("mkfs_status"), dict)
            and isinstance(fuse, dict)
        ):
            return last_payload
        time.sleep(0.1)
    raise RuntimeError(
        "timed out waiting for aggregated compatibility sources; "
        f"last_payload={json.dumps(last_payload, sort_keys=True)}"
    )


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    mountpoint = Path(tempfile.mkdtemp(prefix="fod-monitor-report-compat-"))
    mount = FODMount(str(root))

    interval_name = "FOD_MONITOR_PUBLISH_INTERVAL_MS"
    previous_interval = os.environ.get(interval_name)
    os.environ[interval_name] = "500"

    try:
        mount.start(
            str(mountpoint),
            log_prefix="/tmp/fod-monitor-report-compatibility",
        )

        runtime_env = mount._runtime_env()
        runtime_env[interval_name] = "500"
        runtime_env["FOD_MKFS_BIN"] = str(mkfs_binary(root))

        payload = wait_for_aggregated_report(
            root,
            runtime_env,
            mountpoint,
            timeout_seconds=8.0,
        )

        assert payload["schema_version"] == 2
        assert payload["fod_version"] == "3.4.26"
        assert payload["mkfs_status_error"] is None
        assert payload["cluster_error"] is None

        mkfs_status = payload["mkfs_status"]
        assert mkfs_status["schema_version"] == 1
        assert mkfs_status["fod_version"] == "3.4.26"
        assert mkfs_status["postgresql"]["compatibility"] == "connected"
        assert mkfs_status["schema"]["ready"] is True
        assert mkfs_status["storage_format"]["block_size_bytes"] > 0

        session = matching_session(payload, mountpoint)
        assert session is not None
        stats = session["stats"]
        assert stats["schema_version"] == 2
        compatibility = stats["fuse_compatibility"]
        assert isinstance(compatibility, dict)
        assert compatibility["fuser_version"] == "0.18.0"
        assert compatibility["kernel_protocol"]
        assert compatibility["negotiated_protocol"]

        missing_env = runtime_env.copy()
        missing_env["FOD_MKFS_BIN"] = "/nonexistent/fod-rust-mkfs-p4-3"
        missing_payload = report_json(root, missing_env)

        assert missing_payload["schema_version"] == 2
        assert missing_payload["mkfs_status"] is None
        assert isinstance(missing_payload["mkfs_status_error"], str)
        assert missing_payload["mkfs_status_error"]
        assert matching_session(missing_payload, mountpoint) is not None

        print(
            "OK monitor-report-compatibility-sources "
            f"report_schema={payload['schema_version']} "
            f"mkfs_schema={mkfs_status['schema_version']} "
            f"shared_schema={stats['schema_version']} "
            f"fuser={compatibility['fuser_version']} "
            f"kernel_protocol={compatibility['kernel_protocol']} "
            f"negotiated_protocol={compatibility['negotiated_protocol']} "
            "missing_mkfs_explicit=1"
        )
        return 0
    finally:
        try:
            mount.stop()
        finally:
            if previous_interval is None:
                os.environ.pop(interval_name, None)
            else:
                os.environ[interval_name] = previous_interval
            try:
                mountpoint.rmdir()
            except OSError:
                pass


if __name__ == "__main__":
    raise SystemExit(main())
