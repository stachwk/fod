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


def monitor_binary(root: Path) -> Path:
    configured = os.environ.get("FOD_MONITOR_BIN", "").strip()
    candidates = []
    if configured:
        candidates.append(Path(configured))
    candidates.extend(
        [
            root / "target" / artifact_profile() / "fod-monitor",
            root / "rust_monitor" / "target" / artifact_profile() / "fod-monitor",
        ]
    )
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise RuntimeError("fod-monitor binary not found")


def cluster_json(root: Path, env: dict[str, str]) -> dict:
    result = subprocess.run(
        [str(monitor_binary(root)), "cluster", "--json"],
        cwd=root,
        env=env,
        capture_output=True,
        text=True,
        check=False,
        timeout=10,
    )
    if result.returncode != 0:
        raise RuntimeError(
            "fod-monitor cluster --json failed "
            f"rc={result.returncode} stderr={result.stderr.strip()}"
        )
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(
            f"fod-monitor cluster --json returned invalid JSON: {exc}"
        ) from exc


def matching_session(payload: dict, mountpoint: Path) -> dict | None:
    sessions = payload.get("cluster", {}).get("sessions", [])
    mountpoint_text = str(mountpoint)
    for session in sessions:
        if session.get("mountpoint") == mountpoint_text:
            return session
    return None


def wait_for_fuse_compatibility(
    root: Path,
    env: dict[str, str],
    mountpoint: Path,
    timeout_seconds: float,
) -> tuple[dict, dict]:
    deadline = time.monotonic() + timeout_seconds
    last_payload = {}
    while time.monotonic() < deadline:
        last_payload = cluster_json(root, env)
        session = matching_session(last_payload, mountpoint)
        if session is not None:
            stats = session.get("stats")
            if isinstance(stats, dict) and isinstance(
                stats.get("fuse_compatibility"), dict
            ):
                return session, stats
        time.sleep(0.1)
    raise RuntimeError(
        "timed out waiting for negotiated FUSE compatibility telemetry; "
        f"last_payload={json.dumps(last_payload, sort_keys=True)}"
    )


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    mountpoint = Path(tempfile.mkdtemp(prefix="fod-fuse-compat-"))
    mount = FODMount(str(root))

    interval_name = "FOD_MONITOR_PUBLISH_INTERVAL_MS"
    previous_interval = os.environ.get(interval_name)
    os.environ[interval_name] = "500"

    try:
        mount.start(
            str(mountpoint),
            log_prefix="/tmp/fod-fuse-compatibility-telemetry",
        )
        runtime_env = mount._runtime_env()
        runtime_env[interval_name] = "500"

        session, stats = wait_for_fuse_compatibility(
            root,
            runtime_env,
            mountpoint,
            timeout_seconds=8.0,
        )

        assert stats["schema_version"] == 2

        compatibility = stats["fuse_compatibility"]
        assert compatibility["fuser_version"] == "0.18.0"
        assert compatibility["userspace_protocol_max"] == "7.40"
        assert compatibility["kernel_protocol"]
        assert compatibility["negotiated_protocol"]

        requested = compatibility["requested_capabilities"]
        enabled = compatibility["enabled_capabilities"]
        unsupported = compatibility["unsupported_capabilities"]

        assert "ATOMIC_O_TRUNC" in requested
        assert "ATOMIC_O_TRUNC" in enabled
        assert "ATOMIC_O_TRUNC" not in unsupported

        assert compatibility["requested_max_write_bytes"] == 1_048_576
        assert compatibility["effective_max_write_bytes"] > 0
        assert compatibility["requested_max_readahead_bytes"] == 524_288
        assert compatibility["effective_max_readahead_bytes"] >= 0
        assert compatibility["estimated_request_ceiling_bytes"] > 0

        assert compatibility["max_background"] is None
        assert compatibility["congestion_threshold"] is None

        print(
            "OK fuse-compatibility-telemetry "
            f"session_id={session['session_id']} "
            f"stats_schema={stats['schema_version']} "
            f"fuser={compatibility['fuser_version']} "
            f"kernel_protocol={compatibility['kernel_protocol']} "
            f"negotiated_protocol={compatibility['negotiated_protocol']} "
            f"effective_max_write={compatibility['effective_max_write_bytes']} "
            f"effective_max_readahead={compatibility['effective_max_readahead_bytes']} "
            f"request_ceiling={compatibility['estimated_request_ceiling_bytes']}"
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
