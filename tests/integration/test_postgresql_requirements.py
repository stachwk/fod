#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

from datetime import datetime, timezone
import os
import sys
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from fod_backend import load_dsn_from_config, load_fod_runtime_config
from fod_time import db_timestamp_to_epoch, epoch_to_utc_datetime


def _expected_autocommit_mode() -> bool:
    value = os.environ.get("FOD_POSTGRES_AUTOCOMMIT", "off").strip().lower()
    if value in {"1", "true", "yes", "on"}:
        return True
    if value in {"0", "false", "no", "off"}:
        return False
    raise SystemExit(
        "FOD_POSTGRES_AUTOCOMMIT must be one of on/off, true/false, yes/no, or 1/0"
    )


def main() -> None:
    assert db_timestamp_to_epoch(datetime(2026, 4, 19, 12, 0, 0)) == datetime(
        2026, 4, 19, 12, 0, 0, tzinfo=timezone.utc
    ).timestamp()
    assert db_timestamp_to_epoch(epoch_to_utc_datetime(0)) == 0.0

    dsn, _ = load_dsn_from_config(ROOT)
    runtime_config = load_fod_runtime_config(ROOT)
    pool_max_connections = int(runtime_config.get("pool_max_connections", 10))
    expected_autocommit = _expected_autocommit_mode()

    conn = psycopg2.connect(**dsn)
    try:
        conn.autocommit = expected_autocommit
        assert (
            conn.autocommit is expected_autocommit
        ), f"psycopg2 autocommit mode mismatch: expected={expected_autocommit} actual={conn.autocommit}"
        with conn.cursor() as cur:
            cur.execute("SET TIME ZONE 'UTC'")
            cur.execute("SHOW TIME ZONE")
            time_zone = cur.fetchone()[0]
            cur.execute("SHOW server_version_num")
            server_version_num = int(cur.fetchone()[0])
            cur.execute("SHOW max_connections")
            max_connections = int(cur.fetchone()[0])
    finally:
        conn.close()

    required_min_version = 90500
    required_min_connections = pool_max_connections + 2

    assert server_version_num >= required_min_version, (
        f"PostgreSQL {required_min_version // 10000}.{(required_min_version // 100) % 100}+ is required, "
        f"got server_version_num={server_version_num}"
    )
    assert max_connections >= required_min_connections, (
        f"max_connections must be at least pool_max_connections + 2 "
        f"({required_min_connections}), got {max_connections}"
    )
    assert time_zone.upper() == "UTC", f"FOD PostgreSQL sessions must run in UTC, got {time_zone!r}"

    print(
        f"OK postgres-requirements autocommit={'on' if expected_autocommit else 'off'} "
        f"version={server_version_num} max_connections={max_connections} "
        f"pool_max_connections={pool_max_connections}"
    )


if __name__ == "__main__":
    main()
