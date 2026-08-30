#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""Run the 32K-vs-64K overwrite decision matrix with objective storage-stall retries.

The base decision harness deliberately keeps slow successful runs because a genuine
read-modify-write penalty must remain visible. This wrapper only rejects attempts
whose PostgreSQL measurements show an external storage/WAL stall: excessive WAL
sync time or aggregate pg_stat_io wait. Rejected attempts remain in runs.tsv and
are retried; they never enter the decision medians.
"""

from __future__ import annotations

import importlib.util
import os
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
BASE_PATH = ROOT / "scripts" / "perf" / "run_storage_block_overwrite_decision.py"


def env_float(name: str, default: float) -> float:
    raw = os.environ.get(name, str(default))
    try:
        value = float(raw)
    except ValueError as exc:
        raise SystemExit(f"{name} must be numeric, got {raw!r}") from exc
    if value < 0:
        raise SystemExit(f"{name} must be >= 0, got {value}")
    return value


def load_base() -> Any:
    spec = importlib.util.spec_from_file_location("fod_storage_block_overwrite_decision_base", BASE_PATH)
    if spec is None or spec.loader is None:
        raise SystemExit(f"Cannot load base decision harness: {BASE_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def main() -> int:
    module = load_base()
    original_run_once = module.run_once

    wal_stall_ms = env_float("FOD_STORAGE_DECISION_WAL_STALL_MS", 3000.0)
    pg_io_stall_ms = env_float("FOD_STORAGE_DECISION_PG_IO_STALL_MS", 7000.0)

    # 1 warm-up + 3 accepted measurements + room for objective stall retries.
    os.environ.setdefault("FOD_STORAGE_DECISION_MAX_ATTEMPTS", "12")
    # The 256 MiB sequential workload can finish in only a few seconds. Sample
    # WAL/statements on every profiler loop rather than every fourth loop so the
    # final SQL/WAL accounting is substantially less likely to miss a flush.
    os.environ.setdefault("FOD_PG_WRITE_PROFILE_WAL_EVERY", "1")

    print("=== STORAGE BLOCK DECISION STALL FILTER ===")
    print(f"wal_stall_ms={wal_stall_ms:.3f}")
    print(f"pg_io_stall_ms={pg_io_stall_ms:.3f}")
    print("slow successful runs below these objective I/O limits remain valid")

    def filtered_run_once(
        config: Any,
        *,
        cycle: int,
        attempt: int,
        order: int,
        workload: str,
        block_size: int,
    ) -> dict[str, object]:
        row = original_run_once(
            config,
            cycle=cycle,
            attempt=attempt,
            order=order,
            workload=workload,
            block_size=block_size,
        )
        if row.get("run_quality") != "clean":
            return row

        wal_sync = float(row.get("wal_sync_time_delta_ms", 0) or 0)
        pg_io_wait = float(row.get("pg_io_wait_delta_ms", 0) or 0)
        if wal_sync > wal_stall_ms:
            row["run_quality"] = "wal_stall"
        elif pg_io_wait > pg_io_stall_ms:
            row["run_quality"] = "io_stall"

        if row["run_quality"] != "clean":
            print(
                f"run_quality={row['run_quality']} workload={workload} block={block_size} "
                f"attempt={attempt} write_mib_s={row.get('write_mib_s', 0)} "
                f"wal_sync_ms={wal_sync:.3f} pg_io_wait_ms={pg_io_wait:.3f}"
            )
        return row

    module.run_once = filtered_run_once
    return int(module.main())


if __name__ == "__main__":
    raise SystemExit(main())
