#!/usr/bin/env python3
"""Compute deltas between two storage-table DML profiling snapshots."""

from __future__ import annotations

from decimal import Decimal, DivisionByZero
from pathlib import Path
import sys

from metric_snapshot import delta_value, format_decimal, parse_snapshot


TABLES = [
    "data_blocks",
    "copy_block_crc",
    "files",
    "data_objects",
]

TABLE_METRICS = [
    "seq_scan",
    "seq_tup_read",
    "idx_scan",
    "idx_tup_fetch",
    "n_tup_ins",
    "n_tup_upd",
    "n_tup_hot_upd",
    "n_tup_newpage_upd",
    "n_tup_del",
    "n_live_tup",
    "n_dead_tup",
    "n_mod_since_analyze",
    "n_ins_since_vacuum",
    "vacuum_count",
    "autovacuum_count",
    "analyze_count",
    "autoanalyze_count",
]

INDEXES = [
    "idx_data_blocks_object_order",
    "idx_data_blocks_data_object_id",
    "idx_copy_block_crc_object_order",
    "idx_copy_block_crc_data_object_id",
]

INDEX_METRICS = [
    "idx_scan",
    "idx_tup_read",
    "idx_tup_fetch",
]

RELATIONS = [
    "data_blocks",
    "copy_block_crc",
    "files",
    "data_objects",
    "idx_data_blocks_object_order",
    "idx_data_blocks_data_object_id",
    "idx_copy_block_crc_object_order",
    "idx_copy_block_crc_data_object_id",
]

RELATION_METRICS = [
    "relation_size_bytes",
    "total_size_bytes",
]

DELTA_KEYS = [
    *(f"{table}_{metric}" for table in TABLES for metric in TABLE_METRICS),
    *(f"{index}_{metric}" for index in INDEXES for metric in INDEX_METRICS),
    *(f"{relation}_{metric}" for relation in RELATIONS for metric in RELATION_METRICS),
]


def percent(numerator: Decimal, denominator: Decimal) -> str:
    if denominator == 0:
        return "n/a"
    try:
        return format_decimal((numerator * Decimal(100)) / denominator)
    except DivisionByZero:
        return "n/a"


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print("Usage: table_dml_delta.py BEFORE_SNAPSHOT AFTER_SNAPSHOT", file=sys.stderr)
        return 2

    before_path = Path(argv[1])
    after_path = Path(argv[2])

    if not before_path.exists():
        print(f"ERROR: before DML snapshot not found: {before_path}", file=sys.stderr)
        return 2
    if not after_path.exists():
        print(f"ERROR: after DML snapshot not found: {after_path}", file=sys.stderr)
        return 2

    before = parse_snapshot(before_path)
    after = parse_snapshot(after_path)

    print(f"before_file={before_path}")
    print(f"after_file={after_path}")
    print(f"before_captured_at={before.get('captured_at', '')}")
    print(f"after_captured_at={after.get('captured_at', '')}")
    print(f"before_database_stats_reset={before.get('database_stats_reset', '')}")
    print(f"after_database_stats_reset={after.get('database_stats_reset', '')}")
    print(
        "warning_database_stats_reset_changed="
        + ("1" if before.get("database_stats_reset") != after.get("database_stats_reset") else "0")
    )

    deltas: dict[str, Decimal] = {}
    for key in DELTA_KEYS:
        delta = delta_value(before, after, key)
        deltas[key] = delta
        print(f"{key}_delta={format_decimal(delta)}")

    updates = deltas["data_blocks_n_tup_upd"]
    hot_updates = deltas["data_blocks_n_tup_hot_upd"]
    non_hot_updates = updates - hot_updates
    print(f"data_blocks_non_hot_update_delta={format_decimal(non_hot_updates)}")
    print(f"data_blocks_hot_update_ratio_percent={percent(hot_updates, updates)}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
