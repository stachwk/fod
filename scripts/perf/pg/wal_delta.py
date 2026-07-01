#!/usr/bin/env python3
"""Compute stable TSV deltas from two WAL snapshot files."""

from __future__ import annotations

import argparse
from decimal import Decimal, InvalidOperation
from pathlib import Path
import sys


PREFERRED_METRICS = [
    "wal_records",
    "wal_fpi",
    "wal_bytes",
    "wal_buffers_full",
    "wal_write",
    "wal_sync",
    "wal_write_time",
    "wal_sync_time",
    "checkpoints_timed",
    "checkpoints_req",
    "checkpoint_write_time",
    "checkpoint_sync_time",
    "buffers_checkpoint",
    "buffers_clean",
    "buffers_backend",
    "buffers_backend_fsync",
    "buffers_alloc",
]


def parse_snapshot(path: Path) -> dict[str, Decimal]:
    values: dict[str, Decimal] = {}
    with path.open("r", encoding="utf-8") as handle:
        for raw_line in handle:
            line = raw_line.strip()
            if not line or "\t" not in line:
                continue
            key, value = line.split("\t", 1)
            key = key.strip()
            value = value.strip()
            if not key or key == "metric":
                continue
            try:
                values[key] = Decimal(value)
            except InvalidOperation:
                continue
    return values


def format_decimal(value: Decimal) -> str:
    if value == value.to_integral_value():
        return str(int(value))
    text = format(value.normalize(), "f")
    return text.rstrip("0").rstrip(".")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Compute WAL/checkpointer deltas from two metric<TAB>value snapshots."
    )
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    args = parser.parse_args(argv)

    before = parse_snapshot(args.before)
    after = parse_snapshot(args.after)
    if not before:
        raise SystemExit(f"no metrics parsed from before snapshot: {args.before}")
    if not after:
        raise SystemExit(f"no metrics parsed from after snapshot: {args.after}")

    metrics = [metric for metric in PREFERRED_METRICS if metric in before or metric in after]
    extra_metrics = sorted((set(before) | set(after)) - set(metrics))

    print("metric\tbefore\tafter\tdelta")
    for metric in metrics + extra_metrics:
        before_value = before.get(metric, Decimal(0))
        after_value = after.get(metric, Decimal(0))
        delta = after_value - before_value
        print(
            "\t".join(
                [
                    f"{metric}_delta",
                    format_decimal(before_value),
                    format_decimal(after_value),
                    format_decimal(delta),
                ]
            )
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
