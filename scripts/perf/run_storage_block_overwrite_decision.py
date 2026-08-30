#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""Decision matrix for choosing 32 KiB vs 64 KiB FOD storage blocks.

Workloads:
  * 4 KiB random overwrite of an existing incompressible file
  * 16 KiB random overwrite
  * 64 KiB random overwrite
  * 512 KiB sequential write

The default file and measured I/O size are 256 MiB. Every cell gets one
successful warm-up run that is preserved in runs.tsv but excluded from the
medians, followed by three measured successful runs. Before every attempt the
host must pass a sync + Dirty/Writeback + fsync baseline gate. Infrastructure
failures are retried, but successful slow measured runs are retained: a real
read-modify-write penalty must not be filtered out as a stall.
"""

from __future__ import annotations

import csv
import math
import os
import re
import socket
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ONE = ROOT / "tests/integration/test_fio_storage_block_decision_docker.sh"

WORKLOADS = ("randwrite-4k", "randwrite-16k", "randwrite-64k", "seqwrite-512k")
RUN_FIELDS = (
    "cycle", "attempt", "order", "workload", "storage_block_size", "fio_block_size",
    "write_mib_s", "write_iops", "write_lat_us", "copy_calls", "copy_rows", "copy_exec_ms",
    "copy_mean_ms", "copy_local_blks_written", "insert_calls", "insert_rows", "insert_exec_ms",
    "insert_mean_ms", "insert_shared_blks_written", "insert_wal_bytes", "wal_bytes_delta",
    "wal_sync_time_delta_ms", "pg_io_wait_delta_ms", "wal_amplification",
    "copy_local_page_write_amplification", "insert_shared_page_write_amplification",
    "pre_dirty_kb", "pre_writeback_kb", "pre_fsync_median_ms", "pre_fsync_max_ms",
    "run_quality", "profile_artifact_dir",
)


def env_int(name: str, default: int, minimum: int = 1) -> int:
    raw = os.environ.get(name, str(default))
    try:
        value = int(raw)
    except ValueError as exc:
        raise SystemExit(f"{name} must be an integer, got {raw!r}") from exc
    if value < minimum:
        raise SystemExit(f"{name} must be >= {minimum}")
    return value


def env_float(name: str, default: float, minimum: float = 0.0) -> float:
    raw = os.environ.get(name, str(default))
    try:
        value = float(raw)
    except ValueError as exc:
        raise SystemExit(f"{name} must be numeric, got {raw!r}") from exc
    if value < minimum:
        raise SystemExit(f"{name} must be >= {minimum}")
    return value


def parse_sizes() -> list[int]:
    raw = os.environ.get("FOD_STORAGE_DECISION_BLOCK_SIZES", "32768 65536")
    values = [int(token) for token in raw.split()]
    if values != [32768, 65536]:
        raise SystemExit("FOD_STORAGE_DECISION_BLOCK_SIZES must contain exactly: 32768 65536")
    return values


def size_bytes(text: str) -> int:
    match = re.fullmatch(r"\s*(\d+(?:\.\d+)?)\s*([kKmMgG]?)\s*", text)
    if not match:
        raise SystemExit(f"Unsupported byte size: {text!r}")
    value = float(match.group(1))
    suffix = match.group(2).lower()
    factor = {"": 1, "k": 1024, "m": 1024**2, "g": 1024**3}[suffix]
    return int(value * factor)


def utc_stamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def git_short() -> str:
    proc = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return proc.stdout.strip() or "unknown"


@dataclass(frozen=True)
class Config:
    block_sizes: list[int]
    warmup_runs: int
    target_runs: int
    max_attempts: int
    file_size: str
    io_size: str
    runtime_profile: str
    cargo_profile: str
    max_spread_pct: float
    max_4k_regression_pct: float
    dirty_limit_kb: int
    settle_timeout_seconds: int
    settle_poll_seconds: float
    cooldown_seconds: float
    fsync_probe_count: int
    fsync_median_limit_ms: float
    fsync_max_limit_ms: float
    artifact_dir: Path


def load_config() -> Config:
    runtime = os.environ.get("FOD_RUNTIME_PROFILE", "profiling")
    warmup = env_int("FOD_STORAGE_DECISION_WARMUP_RUNS", 1, minimum=0)
    target = env_int("FOD_STORAGE_DECISION_RUNS", 3)
    attempts = env_int("FOD_STORAGE_DECISION_MAX_ATTEMPTS", 8)
    if attempts < warmup + target:
        raise SystemExit(
            "FOD_STORAGE_DECISION_MAX_ATTEMPTS must be >= "
            "FOD_STORAGE_DECISION_WARMUP_RUNS + FOD_STORAGE_DECISION_RUNS"
        )
    default_artifact = ROOT / "artifacts/perf" / git_short() / (
        f"{socket.gethostname().split('.')[0]}-storage-block-overwrite-decision-{utc_stamp()}"
    )
    return Config(
        block_sizes=parse_sizes(),
        warmup_runs=warmup,
        target_runs=target,
        max_attempts=attempts,
        file_size=os.environ.get("FOD_STORAGE_DECISION_FILE_SIZE", "256M"),
        io_size=os.environ.get("FOD_STORAGE_DECISION_IO_SIZE", "256M"),
        runtime_profile=runtime,
        cargo_profile=os.environ.get("FOD_CARGO_PROFILE", runtime),
        max_spread_pct=env_float("FOD_STORAGE_DECISION_MAX_SPREAD_PCT", 20.0),
        max_4k_regression_pct=env_float("FOD_STORAGE_DECISION_MAX_4K_REGRESSION_PCT", 10.0),
        dirty_limit_kb=env_int("FOD_STORAGE_DECISION_HOST_DIRTY_LIMIT_KB", 8192),
        settle_timeout_seconds=env_int("FOD_STORAGE_DECISION_HOST_SETTLE_TIMEOUT_SECONDS", 300),
        settle_poll_seconds=env_float("FOD_STORAGE_DECISION_HOST_SETTLE_POLL_SECONDS", 2.0, minimum=0.1),
        cooldown_seconds=env_float("FOD_STORAGE_DECISION_HOST_COOLDOWN_SECONDS", 20.0),
        fsync_probe_count=env_int("FOD_STORAGE_DECISION_FSYNC_PROBE_COUNT", 8),
        fsync_median_limit_ms=env_float("FOD_STORAGE_DECISION_FSYNC_MEDIAN_LIMIT_MS", 20.0),
        fsync_max_limit_ms=env_float("FOD_STORAGE_DECISION_FSYNC_MAX_LIMIT_MS", 100.0),
        artifact_dir=Path(os.environ.get("FOD_STORAGE_DECISION_ARTIFACT_DIR", str(default_artifact))),
    )


def stream(command: list[str], env: dict[str, str], log_path: Path) -> int:
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("w", encoding="utf-8") as log:
        proc = subprocess.Popen(
            command,
            cwd=ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            bufsize=1,
        )
        assert proc.stdout is not None
        for line in proc.stdout:
            sys.stdout.write(line)
            log.write(line)
        return proc.wait()


def read_one(path: Path) -> dict[str, str]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle, delimiter="\t"))
    if len(rows) != 1:
        raise RuntimeError(f"Expected one row in {path}, got {len(rows)}")
    return rows[0]


def read_meminfo_kb() -> tuple[int, int]:
    values: dict[str, int] = {}
    with Path("/proc/meminfo").open("r", encoding="utf-8") as handle:
        for line in handle:
            fields = line.split()
            if len(fields) >= 2 and fields[0] in {"Dirty:", "Writeback:"}:
                values[fields[0][:-1]] = int(fields[1])
    return values.get("Dirty", 0), values.get("Writeback", 0)


def fsync_probe(path: Path, count: int) -> tuple[float, float]:
    payload = os.urandom(4096)
    samples: list[float] = []
    fd = os.open(path, os.O_CREAT | os.O_TRUNC | os.O_WRONLY, 0o600)
    try:
        for _ in range(count):
            os.write(fd, payload)
            started = time.perf_counter_ns()
            os.fsync(fd)
            samples.append((time.perf_counter_ns() - started) / 1_000_000.0)
    finally:
        os.close(fd)
        try:
            path.unlink()
        except FileNotFoundError:
            pass
    return statistics.median(samples), max(samples)


def settle_host(config: Config, label: str) -> dict[str, float]:
    print(f"Host settle start label={label}")
    subprocess.run(["sync"], check=True)
    started = time.monotonic()
    probe_path = config.artifact_dir / ".overwrite-decision-fsync-probe"
    while True:
        dirty, writeback = read_meminfo_kb()
        if dirty + writeback <= config.dirty_limit_kb:
            time.sleep(config.cooldown_seconds)
            dirty, writeback = read_meminfo_kb()
            if dirty + writeback <= config.dirty_limit_kb:
                median_ms, max_ms = fsync_probe(probe_path, config.fsync_probe_count)
                if (
                    median_ms <= config.fsync_median_limit_ms
                    and max_ms <= config.fsync_max_limit_ms
                ):
                    print(
                        "Host settle OK "
                        f"dirty_kb={dirty} writeback_kb={writeback} "
                        f"fsync_median_ms={median_ms:.3f} fsync_max_ms={max_ms:.3f}"
                    )
                    return {
                        "dirty_kb": float(dirty),
                        "writeback_kb": float(writeback),
                        "fsync_median_ms": median_ms,
                        "fsync_max_ms": max_ms,
                    }
        if time.monotonic() - started >= config.settle_timeout_seconds:
            raise RuntimeError(
                f"Host did not settle within {config.settle_timeout_seconds}s: "
                f"label={label} dirty_kb={dirty} writeback_kb={writeback}"
            )
        time.sleep(config.settle_poll_seconds)


def wal_metrics(path: Path) -> tuple[float, float]:
    if not path.exists():
        return 0.0, 0.0
    rows = [line.split("\t") for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if len(rows) < 2:
        return 0.0, 0.0
    return float(rows[-1][3]) - float(rows[0][3]), float(rows[-1][8]) - float(rows[0][8])


def pg_io_wait(path: Path) -> float:
    if not path.exists():
        return 0.0
    by_backend: dict[str, list[list[str]]] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        fields = raw.split("\t")
        if len(fields) >= 13:
            by_backend.setdefault(fields[1], []).append(fields)
    total = 0.0
    for rows in by_backend.values():
        if len(rows) < 2:
            continue
        for idx in (3, 5, 7, 9, 12):
            total += float(rows[-1][idx]) - float(rows[0][idx])
    return total


def write_tsv(path: Path, fields: tuple[str, ...] | list[str], rows: list[dict[str, object]]) -> None:
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def mean(total: str, calls: str) -> float:
    count = float(calls)
    return float(total) / count if count else 0.0


def empty_row(
    *, cycle: int, attempt: int, order: int, workload: str, block_size: int,
    quality: str, profile_dir: Path, pre: dict[str, float] | None = None,
) -> dict[str, object]:
    row: dict[str, object] = {field: 0 for field in RUN_FIELDS}
    row.update(
        {
            "cycle": cycle,
            "attempt": attempt,
            "order": order,
            "workload": workload,
            "storage_block_size": block_size,
            "run_quality": quality,
            "profile_artifact_dir": str(profile_dir),
        }
    )
    if pre:
        row.update(
            {
                "pre_dirty_kb": int(pre["dirty_kb"]),
                "pre_writeback_kb": int(pre["writeback_kb"]),
                "pre_fsync_median_ms": f"{pre['fsync_median_ms']:.3f}",
                "pre_fsync_max_ms": f"{pre['fsync_max_ms']:.3f}",
            }
        )
    return row


def run_once(
    config: Config,
    *,
    cycle: int,
    attempt: int,
    order: int,
    workload: str,
    block_size: int,
) -> dict[str, object]:
    run_dir = config.artifact_dir / f"cycle-{cycle}" / f"{workload}-block-{block_size}-attempt-{attempt}"
    profile_dir = run_dir / "postgres"
    run_dir.mkdir(parents=True, exist_ok=True)
    try:
        pre = settle_host(config, f"c{cycle}-o{order}-{workload}-b{block_size}-a{attempt}")
    except (RuntimeError, OSError, subprocess.SubprocessError) as error:
        print(
            f"run_quality=infra_error workload={workload} block={block_size} "
            f"attempt={attempt} preflight_error={error}",
            file=sys.stderr,
        )
        return empty_row(
            cycle=cycle,
            attempt=attempt,
            order=order,
            workload=workload,
            block_size=block_size,
            quality="infra_error",
            profile_dir=profile_dir,
        )

    env = os.environ.copy()
    env.update(
        {
            "FOD_CARGO_PROFILE": config.cargo_profile,
            "FOD_RUNTIME_PROFILE": config.runtime_profile,
            "FOD_TEST_STORAGE_BLOCK_SIZE": str(block_size),
            "FOD_STORAGE_DECISION_WORKLOAD": workload,
            "FOD_STORAGE_DECISION_FILE_SIZE": config.file_size,
            "FOD_STORAGE_DECISION_IO_SIZE": config.io_size,
            "FOD_STORAGE_DECISION_PROFILE_DIR": str(profile_dir),
            "FOD_STORAGE_DECISION_RUN_DIR": str(run_dir),
            "FOD_STORAGE_DECISION_SKIP_BUILD": "1",
            "FOD_STORAGE_DECISION_COOLDOWN_SECONDS": "0",
            "FOD_REQUIRE_AC_POWER": os.environ.get("FOD_REQUIRE_AC_POWER", "1"),
        }
    )
    print(f"\n--- cycle={cycle} order={order} workload={workload} block={block_size} attempt={attempt} ---")
    rc = stream(["bash", str(ONE)], env, run_dir / "run.log")
    if rc:
        return empty_row(
            cycle=cycle,
            attempt=attempt,
            order=order,
            workload=workload,
            block_size=block_size,
            quality="infra_error",
            profile_dir=profile_dir,
            pre=pre,
        )

    summary = read_one(run_dir / "summary.tsv")
    logical = size_bytes(summary["io_size"])
    wal_bytes, wal_sync = wal_metrics(profile_dir / "wal.tsv")
    io_wait = pg_io_wait(profile_dir / "io.tsv")
    copy_mean = mean(summary["copy_exec_ms"], summary["copy_calls"])
    insert_mean = mean(summary["insert_exec_ms"], summary["insert_calls"])
    copy_page_bytes = float(summary["copy_local_blks_written"]) * 8192.0
    insert_page_bytes = float(summary["insert_shared_blks_written"]) * 8192.0
    return {
        "cycle": cycle,
        "attempt": attempt,
        "order": order,
        "workload": workload,
        "storage_block_size": block_size,
        "fio_block_size": summary["fio_block_size"],
        "write_mib_s": summary["write_mib_s"],
        "write_iops": summary["write_iops"],
        "write_lat_us": summary["write_lat_us"],
        "copy_calls": summary["copy_calls"],
        "copy_rows": summary["copy_rows"],
        "copy_exec_ms": summary["copy_exec_ms"],
        "copy_mean_ms": f"{copy_mean:.3f}",
        "copy_local_blks_written": summary["copy_local_blks_written"],
        "insert_calls": summary["insert_calls"],
        "insert_rows": summary["insert_rows"],
        "insert_exec_ms": summary["insert_exec_ms"],
        "insert_mean_ms": f"{insert_mean:.3f}",
        "insert_shared_blks_written": summary["insert_shared_blks_written"],
        "insert_wal_bytes": summary["insert_wal_bytes"],
        "wal_bytes_delta": f"{wal_bytes:.0f}",
        "wal_sync_time_delta_ms": f"{wal_sync:.3f}",
        "pg_io_wait_delta_ms": f"{io_wait:.3f}",
        "wal_amplification": f"{wal_bytes / logical:.4f}",
        "copy_local_page_write_amplification": f"{copy_page_bytes / logical:.4f}",
        "insert_shared_page_write_amplification": f"{insert_page_bytes / logical:.4f}",
        "pre_dirty_kb": int(pre["dirty_kb"]),
        "pre_writeback_kb": int(pre["writeback_kb"]),
        "pre_fsync_median_ms": f"{pre['fsync_median_ms']:.3f}",
        "pre_fsync_max_ms": f"{pre['fsync_max_ms']:.3f}",
        "run_quality": "clean",
        "profile_artifact_dir": str(profile_dir),
    }


def vals(rows: list[dict[str, object]], key: str) -> list[float]:
    return [float(row[key]) for row in rows]


def main() -> int:
    config = load_config()
    config.artifact_dir.mkdir(parents=True, exist_ok=True)
    runs_path = config.artifact_dir / "runs.tsv"
    medians_path = config.artifact_dir / "median.tsv"
    comparisons_path = config.artifact_dir / "comparison.tsv"

    print("=== FOD 32K VS 64K OVERWRITE DECISION MATRIX ===")
    print(f"block_sizes={' '.join(map(str, config.block_sizes))}")
    print(f"workloads={' '.join(WORKLOADS)}")
    print(
        f"warmup_runs={config.warmup_runs} target_runs={config.target_runs} "
        f"max_attempts={config.max_attempts}"
    )
    print(f"file_size={config.file_size} io_size={config.io_size}")
    print(
        f"host_dirty_limit_kb={config.dirty_limit_kb} "
        f"host_cooldown_s={config.cooldown_seconds} "
        f"fsync_median_limit_ms={config.fsync_median_limit_ms} "
        f"fsync_max_limit_ms={config.fsync_max_limit_ms}"
    )
    print(f"artifact_dir={config.artifact_dir}")

    build_env = os.environ.copy()
    build_env.update(
        {
            "FOD_CARGO_PROFILE": config.cargo_profile,
            "FOD_RUNTIME_PROFILE": config.runtime_profile,
        }
    )
    subprocess.run(
        ["make", "--no-print-directory", "build-runtime"],
        cwd=ROOT,
        env=build_env,
        check=True,
    )

    cells = [(workload, size) for workload in WORKLOADS for size in config.block_sizes]
    attempts = {cell: 0 for cell in cells}
    warmup_counts = {cell: 0 for cell in cells}
    clean_counts = {cell: 0 for cell in cells}
    rows: list[dict[str, object]] = []
    cycle = 1

    def cell_pending(cell: tuple[str, int]) -> bool:
        return (
            warmup_counts[cell] < config.warmup_runs
            or clean_counts[cell] < config.target_runs
        ) and attempts[cell] < config.max_attempts

    while any(cell_pending(cell) for cell in cells):
        rotation = (cycle - 1) % len(cells)
        ordered = cells[rotation:] + cells[:rotation]
        print(f"\n=== DECISION CYCLE {cycle} rotation={rotation} ===")
        for order, cell in enumerate(ordered, 1):
            if not cell_pending(cell):
                continue
            workload, size = cell
            attempts[cell] += 1
            row = run_once(
                config,
                cycle=cycle,
                attempt=attempts[cell],
                order=order,
                workload=workload,
                block_size=size,
            )
            if row["run_quality"] == "clean":
                if warmup_counts[cell] < config.warmup_runs:
                    row["run_quality"] = "warmup"
                    warmup_counts[cell] += 1
                else:
                    clean_counts[cell] += 1
            rows.append(row)
            write_tsv(runs_path, RUN_FIELDS, rows)
            print(
                f"progress workload={workload} block={size} "
                f"warmup={warmup_counts[cell]}/{config.warmup_runs} "
                f"clean={clean_counts[cell]}/{config.target_runs}"
            )
        cycle += 1

    median_fields = [
        "workload",
        "storage_block_size",
        "attempts",
        "warmup_runs",
        "clean_runs",
        "infra_errors",
        "median_write_mib_s",
        "min_write_mib_s",
        "max_write_mib_s",
        "write_spread_pct",
        "median_write_iops",
        "median_write_lat_us",
        "median_copy_mean_ms",
        "median_insert_mean_ms",
        "median_wal_amplification",
        "median_copy_local_page_write_amplification",
        "median_insert_shared_page_write_amplification",
        "median_wal_sync_time_delta_ms",
        "median_pg_io_wait_delta_ms",
        "median_pre_fsync_ms",
        "max_pre_fsync_ms",
        "status",
    ]
    medians: list[dict[str, object]] = []
    selection_status = "valid"
    for workload, size in cells:
        all_cell = [
            row
            for row in rows
            if row["workload"] == workload and int(row["storage_block_size"]) == size
        ]
        warmups = [row for row in all_cell if row["run_quality"] == "warmup"]
        clean = [row for row in all_cell if row["run_quality"] == "clean"]
        infra = [row for row in all_cell if row["run_quality"] == "infra_error"]
        write = vals(clean, "write_mib_s")
        med = statistics.median(write) if write else 0.0
        lo, hi = (min(write), max(write)) if write else (0.0, 0.0)
        spread = ((hi - lo) * 100.0 / med) if med else 0.0
        status = "OK"
        if len(warmups) < config.warmup_runs:
            status = "INSUFFICIENT_WARMUP"
        elif len(clean) < config.target_runs:
            status = "INSUFFICIENT_RUNS"
        elif spread > config.max_spread_pct:
            status = "UNSTABLE"
        if status != "OK":
            selection_status = "invalid"
        medians.append(
            {
                "workload": workload,
                "storage_block_size": size,
                "attempts": len(all_cell),
                "warmup_runs": len(warmups),
                "clean_runs": len(clean),
                "infra_errors": len(infra),
                "median_write_mib_s": f"{med:.3f}",
                "min_write_mib_s": f"{lo:.3f}",
                "max_write_mib_s": f"{hi:.3f}",
                "write_spread_pct": f"{spread:.2f}",
                "median_write_iops": f"{statistics.median(vals(clean, 'write_iops')) if clean else 0:.3f}",
                "median_write_lat_us": f"{statistics.median(vals(clean, 'write_lat_us')) if clean else 0:.3f}",
                "median_copy_mean_ms": f"{statistics.median(vals(clean, 'copy_mean_ms')) if clean else 0:.3f}",
                "median_insert_mean_ms": f"{statistics.median(vals(clean, 'insert_mean_ms')) if clean else 0:.3f}",
                "median_wal_amplification": f"{statistics.median(vals(clean, 'wal_amplification')) if clean else 0:.4f}",
                "median_copy_local_page_write_amplification": f"{statistics.median(vals(clean, 'copy_local_page_write_amplification')) if clean else 0:.4f}",
                "median_insert_shared_page_write_amplification": f"{statistics.median(vals(clean, 'insert_shared_page_write_amplification')) if clean else 0:.4f}",
                "median_wal_sync_time_delta_ms": f"{statistics.median(vals(clean, 'wal_sync_time_delta_ms')) if clean else 0:.3f}",
                "median_pg_io_wait_delta_ms": f"{statistics.median(vals(clean, 'pg_io_wait_delta_ms')) if clean else 0:.3f}",
                "median_pre_fsync_ms": f"{statistics.median(vals(clean, 'pre_fsync_median_ms')) if clean else 0:.3f}",
                "max_pre_fsync_ms": f"{max(vals(clean, 'pre_fsync_max_ms')) if clean else 0:.3f}",
                "status": status,
            }
        )
    write_tsv(medians_path, median_fields, medians)

    comparison_fields = [
        "workload",
        "write_32k_mib_s",
        "write_64k_mib_s",
        "write_64k_vs_32k_pct",
        "wal_amp_32k",
        "wal_amp_64k",
        "status",
    ]
    comparisons: list[dict[str, object]] = []
    ratios: list[float] = []
    four_k_pct = 0.0
    by_key = {(row["workload"], int(row["storage_block_size"])): row for row in medians}
    for workload in WORKLOADS:
        r32 = by_key[(workload, 32768)]
        r64 = by_key[(workload, 65536)]
        w32 = float(r32["median_write_mib_s"])
        w64 = float(r64["median_write_mib_s"])
        pct = ((w64 / w32) - 1.0) * 100.0 if w32 else 0.0
        if w32 and w64:
            ratios.append(w64 / w32)
        if workload == "randwrite-4k":
            four_k_pct = pct
        status = "OK" if r32["status"] == "OK" and r64["status"] == "OK" else "INVALID"
        comparisons.append(
            {
                "workload": workload,
                "write_32k_mib_s": f"{w32:.3f}",
                "write_64k_mib_s": f"{w64:.3f}",
                "write_64k_vs_32k_pct": f"{pct:.2f}",
                "wal_amp_32k": r32["median_wal_amplification"],
                "wal_amp_64k": r64["median_wal_amplification"],
                "status": status,
            }
        )
    write_tsv(comparisons_path, comparison_fields, comparisons)

    geometric_ratio = math.prod(ratios) ** (1.0 / len(ratios)) if ratios else 0.0
    recommendation = "none"
    if selection_status == "valid":
        if four_k_pct < -config.max_4k_regression_pct:
            recommendation = "32768"
        elif geometric_ratio >= 1.0:
            recommendation = "65536"
        else:
            recommendation = "32768"

    print("\n=== STORAGE BLOCK OVERWRITE RUNS ===")
    print(runs_path.read_text(encoding="utf-8"), end="")
    print("\n=== STORAGE BLOCK OVERWRITE MEDIANS ===")
    print(medians_path.read_text(encoding="utf-8"), end="")
    print("\n=== 64K VS 32K COMPARISON ===")
    print(comparisons_path.read_text(encoding="utf-8"), end="")
    print(f"selection_status={selection_status}")
    print(f"max_allowed_4k_regression_pct={config.max_4k_regression_pct:.2f}")
    print(f"four_k_64k_vs_32k_pct={four_k_pct:.2f}")
    print(f"geometric_mean_64k_vs_32k_ratio={geometric_ratio:.4f}")
    print(f"decision_hint_block_size={recommendation}")
    print(f"decision_artifact_dir={config.artifact_dir}")
    print("OK: storage block overwrite decision matrix")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
