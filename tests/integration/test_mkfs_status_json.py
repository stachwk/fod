#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path


def runtime_profile() -> str:
    return (
        os.environ.get("FOD_RUNTIME_PROFILE")
        or os.environ.get("FOD_CARGO_PROFILE")
        or "release-lto"
    )


def artifact_profile() -> str:
    profile = runtime_profile()
    return "debug" if profile == "dev" else profile


def mkfs_binary(root: Path) -> Path:
    configured = os.environ.get("FOD_MKFS_BIN", "").strip()
    candidates = []
    if configured:
        candidates.append(Path(configured))
    candidates.extend(
        [
            root / "target" / artifact_profile() / "fod-rust-mkfs",
            root / "rust_mkfs" / "target" / artifact_profile() / "fod-rust-mkfs",
        ]
    )
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise RuntimeError("fod-rust-mkfs binary not found")


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    version = (root / "fod_version.txt").read_text(
        encoding="utf-8"
    ).strip()

    result = subprocess.run(
        [str(mkfs_binary(root)), "status", "--json"],
        cwd=root,
        env=os.environ.copy(),
        capture_output=True,
        text=True,
        check=False,
    )

    if result.returncode != 0:
        raise RuntimeError(
            "fod-rust-mkfs status --json failed "
            f"rc={result.returncode} stderr={result.stderr.strip()}"
        )

    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(
            f"status --json did not emit valid JSON: {exc}"
        ) from exc

    assert payload["schema_version"] == 1
    assert payload["fod_version"] == version

    postgres = payload["postgresql"]
    assert postgres["compatibility"] == "connected"
    assert postgres["libpq_runtime"]["version_num"] > 0
    assert postgres["server_runtime"]["version_num"] >= 90500

    requirements = postgres["requirements"]
    assert requirements["minimum_server_version_num"] == 90500
    assert requirements["pool_max_connections"] > 0
    assert (
        requirements["required_max_connections"]
        >= requirements["pool_max_connections"]
    )
    assert isinstance(requirements["settings"], list)

    schema = payload["schema"]
    assert schema["name"] == "fod"
    assert schema["objects_present"] is True
    assert schema["latest_shape"] is True
    assert schema["admin_secret_present"] is True
    assert schema["ready"] is True
    assert schema["version"] == schema["latest_migration_version"]
    assert schema["pending_migrations"] == []
    assert isinstance(schema["migration_path"], list)
    assert len(schema["migration_path"]) == schema["latest_migration_version"]

    storage = payload["storage_format"]
    assert storage["canonical_schema"] == "fod"
    assert storage["active_schema"] == "fod"
    assert storage["block_size_bytes"] is not None
    assert storage["block_size_bytes"] > 0
    assert storage["block_size_bytes"] % 1024 == 0
    assert storage["max_fs_size_bytes"] is not None
    assert storage["max_fs_size_bytes"] > 0

    print(
        "OK mkfs-status-json "
        f"schema_version={payload['schema_version']} "
        f"fod_version={payload['fod_version']} "
        f"server_version_num={postgres['server_runtime']['version_num']} "
        f"block_size_bytes={storage['block_size_bytes']} "
        f"ready={int(schema['ready'])}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
