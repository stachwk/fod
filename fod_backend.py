#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import configparser
import json
import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def _config_path(config_or_root: str | Path) -> Path:
    path = Path(config_or_root)
    if path.is_dir():
        return path / "fod_config.ini"
    return path


def _clean_env() -> dict[str, str]:
    env = os.environ.copy()
    env.pop("FOD_CONFIG", None)
    return env


def _fod_config_cmd() -> list[str]:
    for candidate in (
        ROOT / "rust_mkfs/target/debug/fod-config",
        ROOT / "rust_mkfs/target/release/fod-config",
        Path("/usr/local/bin/fod-config"),
    ):
        if candidate.is_file():
            return [str(candidate)]
    return [
        "cargo",
        "run",
        "--manifest-path",
        str(ROOT / "rust_mkfs/Cargo.toml"),
        "--quiet",
        "--bin",
        "fod-config",
        "--",
    ]


def load_fod_runtime_config(config_or_root: str | Path) -> dict[str, str]:
    config_path = _config_path(config_or_root)
    cmd = _fod_config_cmd() + ["--config-path", str(config_path), "runtime-config"]
    result = subprocess.run(
        cmd,
        cwd=ROOT,
        env=_clean_env(),
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            "fod-config runtime-config failed\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )

    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"failed to parse fod-config runtime-config JSON: {exc}") from exc

    if not isinstance(payload, dict):
        raise RuntimeError("fod-config runtime-config did not return a JSON object")

    return {str(key): str(value) for key, value in payload.items()}


def _resolve_relative_path(value: str, config_dir: Path) -> str:
    path = Path(value).expanduser()
    if not path.is_absolute():
        path = config_dir / path
    return str(path)


def load_dsn_from_config(config_or_root: str | Path) -> tuple[dict[str, str], dict[str, str]]:
    config_path = _config_path(config_or_root)
    parser = configparser.ConfigParser(interpolation=None)
    if not parser.read(config_path):
        raise FileNotFoundError(f"failed to read FOD config: {config_path}")
    if "database" not in parser:
        raise ValueError(f"missing [database] section in {config_path}")

    db_config = {key.lower(): value for key, value in parser["database"].items()}
    config_dir = config_path.parent

    dsn = {
        "host": db_config.get("host", "127.0.0.1"),
        "port": db_config.get("port", "5432"),
        "dbname": db_config.get("dbname", "foddbname"),
        "user": db_config.get("user", "foduser"),
        "password": db_config.get("password", "cichosza"),
    }

    sslmode = db_config.get("sslmode", "").strip()
    if sslmode and sslmode.lower() != "disable":
        dsn["sslmode"] = sslmode

    for key in ("sslrootcert", "sslcert", "sslkey"):
        value = db_config.get(key, "").strip()
        if value:
            dsn[key] = _resolve_relative_path(value, config_dir)

    return dsn, db_config
