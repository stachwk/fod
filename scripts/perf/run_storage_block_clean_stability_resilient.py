#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""Resilient entrypoint for the clean storage-block stability benchmark.

The underlying benchmark intentionally keeps strict replication correctness gates.
A transient Docker/PostgreSQL replication failure must not abort the whole candidate
series, however. This wrapper converts such infrastructure failures into a non-clean
attempt so the existing scheduler retries that candidate until it gets the requested
number of complete clean runs or reaches the configured attempt limit.
"""

from __future__ import annotations

import importlib.util
import os
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
BASE_PATH = ROOT / "scripts" / "perf" / "run_storage_block_clean_stability.py"


def load_base() -> Any:
    spec = importlib.util.spec_from_file_location("fod_storage_block_clean_stability_base", BASE_PATH)
    if spec is None or spec.loader is None:
        raise SystemExit(f"Cannot load base stability harness: {BASE_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def infrastructure_error_row(
    module: Any,
    *,
    cycle: int,
    attempt: int,
    order: int,
    block_size: int,
    error: BaseException,
) -> dict[str, object]:
    row: dict[str, object] = {field: 0 for field in module.RUN_FIELDS}
    row.update(
        {
            "cycle": cycle,
            "attempt": attempt,
            "order": order,
            "storage_block_size": block_size,
            "run_quality": "infra_error",
            "profile_artifact_dir": "",
        }
    )
    print(
        "run_quality=infra_error "
        f"block_size={block_size} attempt={attempt} error={type(error).__name__}: {error}",
        file=sys.stderr,
    )
    return row


def main() -> int:
    module = load_base()
    original_run_attempt = module.run_attempt

    # A broken WAL receiver after a primary restart should fail quickly enough to
    # be retried instead of consuming the base benchmark's full 120 s timeout.
    # The write-replay gate normally completes in seconds; callers can override.
    os.environ.setdefault("REPLICA_WAIT_SECONDS", "30")

    def resilient_run_attempt(
        config: Any,
        *,
        cycle: int,
        attempt: int,
        order: int,
        block_size: int,
        settle_log: Path,
    ) -> dict[str, object]:
        try:
            return original_run_attempt(
                config,
                cycle=cycle,
                attempt=attempt,
                order=order,
                block_size=block_size,
                settle_log=settle_log,
            )
        except (RuntimeError, TimeoutError, OSError) as error:
            return infrastructure_error_row(
                module,
                cycle=cycle,
                attempt=attempt,
                order=order,
                block_size=block_size,
                error=error,
            )

    module.run_attempt = resilient_run_attempt
    return int(module.main())


if __name__ == "__main__":
    raise SystemExit(main())
