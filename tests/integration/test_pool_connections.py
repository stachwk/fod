#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from fod_backend import load_fod_runtime_config


def make_temp_config(pool_max_connections: int) -> Path:
    tmpdir = Path(tempfile.mkdtemp(prefix="fod-pool-config-"))
    config_path = tmpdir / "fod_config.ini"
    config_path.write_text(
        "[fod]\n"
        f"pool_max_connections = {pool_max_connections}\n",
        encoding="utf-8",
    )
    return config_path


def assert_pool_value(runtime: dict[str, str], expected: int) -> None:
    got_raw = runtime.get("pool_max_connections")
    if got_raw is None:
        raise AssertionError("pool_max_connections missing in runtime config")
    try:
        got = int(got_raw)
    except ValueError as exc:
        raise AssertionError(f"pool_max_connections is not integer: {got_raw!r}") from exc
    if got != expected:
        raise AssertionError(f"expected pool_max_connections={expected}, got={got}")


def test_project_config_is_valid() -> None:
    local_config = ROOT / "fod_config.ini"
    config_target = local_config if local_config.exists() else ROOT
    runtime = load_fod_runtime_config(config_target)

    got_raw = runtime.get("pool_max_connections")
    if got_raw is None:
        raise AssertionError("pool_max_connections missing in project runtime config")

    try:
        got = int(got_raw)
    except ValueError as exc:
        raise AssertionError(f"project pool_max_connections is not integer: {got_raw!r}") from exc

    if got <= 0:
        raise AssertionError(f"project pool_max_connections must be > 0, got={got}")


def test_custom_pool_value_roundtrip() -> None:
    expected = 17
    config_path = make_temp_config(expected)
    runtime = load_fod_runtime_config(config_path)
    assert_pool_value(runtime, expected)


def test_zero_pool_value_is_rejected() -> None:
    config_path = make_temp_config(0)
    try:
        load_fod_runtime_config(config_path)
    except RuntimeError as exc:
        combined = str(exc).lower()
        if "pool_max_connections" not in combined:
            raise AssertionError("negative validation did not mention pool_max_connections") from exc
        return

    raise AssertionError("pool_max_connections=0 should be rejected")


def main() -> None:
    test_project_config_is_valid()
    test_custom_pool_value_roundtrip()
    test_zero_pool_value_is_rejected()
    print("OK pool/max_connections")


if __name__ == "__main__":
    main()
