#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import os
import statistics
import sys
import time
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_postgres_benchmark import resolve_postgres_dsn


def main() -> None:
    connection_count = int(os.environ.get("PG_CONNECTION_CHURN_COUNT", "100"))
    statement = os.environ.get("PG_CONNECTION_CHURN_SQL", "SELECT 1")
    dsn = resolve_postgres_dsn(ROOT)

    connect_times_ns: list[int] = []
    query_times_ns: list[int] = []

    start_ns = time.perf_counter_ns()
    for _ in range(connection_count):
        connect_start = time.perf_counter_ns()
        conn = psycopg2.connect(**dsn)
        connect_times_ns.append(time.perf_counter_ns() - connect_start)
        try:
            query_start = time.perf_counter_ns()
            with conn.cursor() as cur:
                cur.execute(statement)
                cur.fetchone()
            query_times_ns.append(time.perf_counter_ns() - query_start)
        finally:
            conn.close()
    elapsed_ns = time.perf_counter_ns() - start_ns

    connect_avg_ms = statistics.fmean(connect_times_ns) / 1_000_000
    query_avg_ms = statistics.fmean(query_times_ns) / 1_000_000
    connect_p95_ms = sorted(connect_times_ns)[max(0, int(connection_count * 0.95) - 1)] / 1_000_000
    query_p95_ms = sorted(query_times_ns)[max(0, int(connection_count * 0.95) - 1)] / 1_000_000

    print(
        "OK postgres/connection-churn "
        f"count={connection_count} elapsed_s={elapsed_ns / 1_000_000_000:.3f} "
        f"connect_avg_ms={connect_avg_ms:.3f} connect_p95_ms={connect_p95_ms:.3f} "
        f"query_avg_ms={query_avg_ms:.3f} query_p95_ms={query_p95_ms:.3f}"
    )


if __name__ == "__main__":
    main()
