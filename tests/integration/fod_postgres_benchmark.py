#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import os
from pathlib import Path

import psycopg2

from fod_backend import load_dsn_from_config


def resolve_postgres_dsn(root: Path) -> dict[str, str]:
    dsn, _ = load_dsn_from_config(root)

    override_map = {
        "host": ("FOD_PG_HOST",),
        "port": ("FOD_PG_PORT",),
        "dbname": ("FOD_PG_DBNAME", "POSTGRES_DB"),
        "user": ("FOD_PG_USER", "POSTGRES_USER"),
        "password": ("FOD_PG_PASSWORD", "POSTGRES_PASSWORD"),
        "sslmode": ("FOD_PG_SSLMODE",),
        "sslrootcert": ("FOD_PG_SSLROOTCERT",),
        "sslcert": ("FOD_PG_SSLCERT",),
        "sslkey": ("FOD_PG_SSLKEY",),
    }

    for key, env_names in override_map.items():
        for env_name in env_names:
            value = os.environ.get(env_name, "").strip()
            if value:
                dsn[key] = value
                break

    return dsn


def _row_to_dict(cursor, row) -> dict[str, object]:
    return {column[0]: value for column, value in zip(cursor.description, row, strict=True)}


def capture_postgres_stats(dsn: dict[str, str]) -> dict[str, dict[str, object]]:
    with psycopg2.connect(**dsn) as conn, conn.cursor() as cur:
        cur.execute("SELECT * FROM pg_stat_bgwriter")
        bgwriter = _row_to_dict(cur, cur.fetchone())
        cur.execute("SELECT * FROM pg_stat_wal")
        wal = _row_to_dict(cur, cur.fetchone())
    return {"bgwriter": bgwriter, "wal": wal}


def diff_stats(after: dict[str, object], before: dict[str, object]) -> dict[str, object]:
    delta: dict[str, object] = {}
    for key, value in after.items():
        if key not in before:
            continue
        try:
            delta[key] = value - before[key]
        except TypeError:
            continue
    return delta
