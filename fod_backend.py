#!/usr/bin/env python
# -*- coding: utf-8 -*-
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


def _runtime_artifact_profile() -> str:
    profile = os.environ.get("FOD_RUNTIME_PROFILE", "release-lto").strip()
    if not profile:
        profile = "release-lto"
    return "debug" if profile == "dev" else profile


def _cargo_target_root() -> Path:
    configured = os.environ.get("CARGO_TARGET_DIR", "").strip()
    if not configured:
        return ROOT / "target"
    path = Path(configured).expanduser()
    if not path.is_absolute():
        path = ROOT / path
    return path


def _resolve_config_binary(path: Path, source: str) -> list[str]:
    if not path.is_file():
        raise RuntimeError(f"{source} does not point to an existing fod-config binary: {path}")
    if not os.access(path, os.X_OK):
        raise RuntimeError(f"{source} fod-config binary is not executable: {path}")
    return [str(path)]


def _fod_config_cmd() -> list[str]:
    # Jawna binarka ma pierwszenstwo i nigdy nie uruchamia ukrytego builda.
    explicit = os.environ.get("FOD_CONFIG_BIN", "").strip()
    if explicit:
        path = Path(explicit).expanduser()
        if not path.is_absolute():
            path = ROOT / path
        return _resolve_config_binary(path, "FOD_CONFIG_BIN")

    profile = _runtime_artifact_profile()
    candidates = [
        _cargo_target_root() / profile / "fod-config",
        ROOT / "rust_mkfs" / "target" / profile / "fod-config",
        Path("/usr/local/bin/fod-config"),
    ]
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return [str(candidate)]

    raise RuntimeError(
        "fod-config binary not found; run make build-runtime or set "
        "FOD_CONFIG_BIN to an existing runtime binary"
    )


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
