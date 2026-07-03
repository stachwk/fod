#!/usr/bin/env python3
"""Build a concise markdown summary for data_blocks profiling artifacts."""

from __future__ import annotations

import argparse
import datetime as dt
import re
from pathlib import Path
from typing import Iterable


def read_text(path: Path | None) -> str:
    if not path:
        return ""
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except FileNotFoundError:
        return ""


def first_match(pattern: str, text: str, default: str = "n/a") -> str:
    match = re.search(pattern, text, re.MULTILINE)
    if not match:
        return default
    return match.group(1)


def parse_env(text: str) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in text.splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        result[key.strip()] = value.strip()
    return result


def parse_key_values(text: str) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in text.splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            result[key.strip()] = value.strip()
            continue
        parts = line.split("\t")
        if len(parts) == 4 and parts[0] != "metric":
            result[parts[0]] = parts[3]
    return result


def top_sql_rows(text: str) -> dict[str, list[str]]:
    rows = {"copy": [], "merge": []}
    for line in text.splitlines():
        if "COPY fod_persist_block_stage" in line:
            rows["copy"].append(line)
        if "INSERT INTO data_blocks" in line:
            rows["merge"].append(line)
    return rows


def total_exec_ms(row: str) -> str:
    parts = [part.strip() for part in row.split("|")]
    if len(parts) > 2:
        return parts[2]
    return "n/a"


def sum_numbers(values: Iterable[str]) -> str:
    total = 0.0
    found = False
    for value in values:
        try:
            total += float(value)
            found = True
        except ValueError:
            continue
    return f"{total:.3f}" if found else "n/a"


def bloat_snapshot(text: str) -> dict[str, str]:
    snapshot = {
        "data_blocks_n_live_tup": "n/a",
        "data_blocks_n_dead_tup": "n/a",
        "data_blocks_relation_size": "n/a",
        "idx_data_blocks_object_order_relation_size": "n/a",
    }
    for line in text.splitlines():
        if " data_blocks " in f" {line} " and "|" in line:
            parts = [part.strip() for part in line.split("|")]
            if len(parts) >= 5 and parts[1] == "data_blocks" and parts[2].isdigit():
                snapshot["data_blocks_n_live_tup"] = parts[2]
                snapshot["data_blocks_n_dead_tup"] = parts[3]
        if "|" not in line:
            continue
        parts = [part.strip() for part in line.split("|")]
        if len(parts) >= 3 and parts[0] == "data_blocks":
            snapshot["data_blocks_relation_size"] = parts[1]
        if len(parts) >= 3 and parts[0] == "idx_data_blocks_object_order":
            snapshot["idx_data_blocks_object_order_relation_size"] = parts[1]
    return snapshot


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact-dir", required=True, type=Path)
    parser.add_argument("--large-copy-log", required=True, type=Path)
    parser.add_argument("--pg-top", required=True, type=Path)
    parser.add_argument("--wal-delta", required=True, type=Path)
    parser.add_argument("--table-dml-delta", type=Path)
    parser.add_argument("--data-blocks-bloat", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--run-id")
    parser.add_argument("--host")
    parser.add_argument("--conclusion", default="Real-path data_blocks profile captured.")
    parser.add_argument(
        "--next-candidate",
        default="Keep runtime SQL unchanged until repeated local/QNAP data confirms the next bottleneck.",
    )
    args = parser.parse_args()

    env = parse_env(read_text(args.artifact_dir / "env.txt"))
    log_text = read_text(args.large_copy_log)
    pg_top_text = read_text(args.pg_top)
    wal = parse_key_values(read_text(args.wal_delta))
    dml = parse_key_values(read_text(args.table_dml_delta))
    bloat = bloat_snapshot(read_text(args.data_blocks_bloat))
    sql_rows = top_sql_rows(pg_top_text)

    elapsed = first_match(r"elapsed_s=([0-9.]+)", log_text)
    throughput = first_match(r"throughput_mib_s=([0-9.]+)", log_text)
    copy_total = sum_numbers(total_exec_ms(row) for row in sql_rows["copy"])
    merge_total = sum_numbers(total_exec_ms(row) for row in sql_rows["merge"][:2])

    today = dt.date.today().isoformat()
    output = args.output or Path(f"docs/performance-data-blocks-profile-{today}.md")
    run_id = args.run_id or args.artifact_dir.name
    host = args.host or first_match(r"^([^-/]+)-", args.artifact_dir.name, "n/a")

    lines = [
        f"# FOD Data Blocks Profile - {today}",
        "",
        "## Run Metadata",
        "",
        f"- Run ID: `{run_id}`",
        f"- Host: `{host}`",
        f"- Commit: `{env.get('commit', 'n/a')}`",
        f"- FOD version: `{env.get('fod_version', 'n/a')}`",
        f"- Artifact directory: `{args.artifact_dir}`",
        "",
        "## Large Copy Workload",
        "",
        f"- `elapsed_s`: `{elapsed}`",
        f"- `throughput_mib_s`: `{throughput}`",
        f"- `COPY fod_persist_block_stage total_exec_ms`: `{copy_total}`",
        f"- `data_blocks merge total_exec_ms`: `{merge_total}`",
        "",
        "## WAL Delta",
        "",
        f"- `wal_records_delta`: `{wal.get('wal_records_delta', 'n/a')}`",
        f"- `wal_fpi_delta`: `{wal.get('wal_fpi_delta', 'n/a')}`",
        f"- `wal_bytes_delta`: `{wal.get('wal_bytes_delta', 'n/a')}`",
        f"- `wal_buffers_full_delta`: `{wal.get('wal_buffers_full_delta', 'n/a')}`",
        f"- `wal_write_delta`: `{wal.get('wal_write_delta', 'n/a')}`",
        f"- `wal_sync_delta`: `{wal.get('wal_sync_delta', 'n/a')}`",
        f"- `buffers_checkpoint_delta`: `{wal.get('buffers_checkpoint_delta', 'n/a')}`",
        f"- `buffers_backend_delta`: `{wal.get('buffers_backend_delta', 'n/a')}`",
        f"- `buffers_backend_fsync_delta`: `{wal.get('buffers_backend_fsync_delta', 'n/a')}`",
        "",
        "## Table DML Delta",
        "",
        f"- `data_blocks_n_tup_ins_delta`: `{dml.get('data_blocks_n_tup_ins_delta', 'n/a')}`",
        f"- `data_blocks_n_tup_upd_delta`: `{dml.get('data_blocks_n_tup_upd_delta', 'n/a')}`",
        f"- `data_blocks_n_tup_hot_upd_delta`: `{dml.get('data_blocks_n_tup_hot_upd_delta', 'n/a')}`",
        f"- `data_blocks_non_hot_update_delta`: `{dml.get('data_blocks_non_hot_update_delta', 'n/a')}`",
        f"- `data_blocks_hot_update_ratio_percent`: `{dml.get('data_blocks_hot_update_ratio_percent', 'n/a')}`",
        f"- `data_blocks_n_tup_del_delta`: `{dml.get('data_blocks_n_tup_del_delta', 'n/a')}`",
        f"- `data_blocks_n_dead_tup_delta`: `{dml.get('data_blocks_n_dead_tup_delta', 'n/a')}`",
        f"- `idx_data_blocks_object_order_idx_scan_delta`: `{dml.get('idx_data_blocks_object_order_idx_scan_delta', 'n/a')}`",
        f"- `idx_data_blocks_object_order_idx_tup_read_delta`: `{dml.get('idx_data_blocks_object_order_idx_tup_read_delta', 'n/a')}`",
        f"- `idx_data_blocks_object_order_idx_tup_fetch_delta`: `{dml.get('idx_data_blocks_object_order_idx_tup_fetch_delta', 'n/a')}`",
        "",
        "## Bloat / Churn Snapshot",
        "",
        f"- `data_blocks_n_live_tup`: `{bloat['data_blocks_n_live_tup']}`",
        f"- `data_blocks_n_dead_tup`: `{bloat['data_blocks_n_dead_tup']}`",
        f"- `data_blocks_relation_size`: `{bloat['data_blocks_relation_size']}`",
        f"- `idx_data_blocks_object_order_relation_size`: `{bloat['idx_data_blocks_object_order_relation_size']}`",
        "",
        "## Conclusion",
        "",
        args.conclusion,
        "",
        "## Next Candidate",
        "",
        args.next_candidate,
        "",
    ]
    output.write_text("\n".join(lines), encoding="utf-8")
    print(f"Wrote {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
