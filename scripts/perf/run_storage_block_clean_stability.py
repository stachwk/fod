#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""Collect stable storage-block-size measurements.

The target is N *clean* runs per candidate, not merely N attempts. Attempts
showing large PostgreSQL WAL or storage I/O wait are preserved in runs.tsv but
retried up to a configurable limit. Before every attempt the host must pass a
Dirty/Writeback gate and a small fsync probe on the same filesystem as the
artifacts.
"""

from __future__ import annotations

import csv
import os
import socket
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable

ROOT = Path(__file__).resolve().parents[2]
MATRIX = ROOT / "scripts/perf/run_random_storage_block_matrix.sh"


def env_int(name: str, default: int, *, minimum: int = 1) -> int:
    raw = os.environ.get(name, str(default))
    try:
        value = int(raw)
    except ValueError as exc:
        raise SystemExit(f"{name} must be an integer, got {raw!r}") from exc
    if value < minimum:
        raise SystemExit(f"{name} must be >= {minimum}, got {value}")
    return value


def env_float(name: str, default: float, *, minimum: float = 0.0) -> float:
    raw = os.environ.get(name, str(default))
    try:
        value = float(raw)
    except ValueError as exc:
        raise SystemExit(f"{name} must be numeric, got {raw!r}") from exc
    if value < minimum:
        raise SystemExit(f"{name} must be >= {minimum}, got {value}")
    return value


def parse_block_sizes() -> list[int]:
    raw = os.environ.get("FOD_STORAGE_CLEAN_BLOCK_SIZES", "16384 32768 65536")
    values: list[int] = []
    for token in raw.split():
        try:
            value = int(token)
        except ValueError as exc:
            raise SystemExit(f"Invalid storage block size: {token!r}") from exc
        if value < 1024 or value % 1024:
            raise SystemExit(f"Storage block size must be a positive multiple of 1024: {value}")
        values.append(value)
    if not values:
        raise SystemExit("At least one storage block size is required")
    if len(set(values)) != len(values):
        raise SystemExit("Storage block sizes must be unique")
    return values


def git_short_head() -> str:
    proc = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return proc.stdout.strip() or "unknown"


def utc_stamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


@dataclass(frozen=True)
class Config:
    block_sizes: list[int]
    target_clean_runs: int
    max_attempts_per_size: int
    file_size: str
    fio_block_size: str
    payload_mode: str
    runtime_profile: str
    cargo_profile: str
    dirty_limit_kb: int
    settle_timeout_seconds: int
    settle_poll_seconds: float
    cooldown_seconds: float
    fsync_probe_count: int
    fsync_median_limit_ms: float
    fsync_max_limit_ms: float
    wal_stall_ms: float
    pg_io_stall_ms: float
    max_clean_spread_pct: float
    artifact_dir: Path


def load_config() -> Config:
    runtime_profile = os.environ.get("FOD_RUNTIME_PROFILE", "profiling")
    cargo_profile = os.environ.get("FOD_CARGO_PROFILE", runtime_profile)
    artifact_default = (
        ROOT
        / "artifacts/perf"
        / git_short_head()
        / f"{socket.gethostname().split('.')[0]}-storage-block-clean-stability-{utc_stamp()}"
    )
    artifact_dir = Path(os.environ.get("FOD_STORAGE_CLEAN_ARTIFACT_DIR", str(artifact_default)))
    target = env_int("FOD_STORAGE_CLEAN_RUNS", 3)
    max_attempts = env_int("FOD_STORAGE_MAX_ATTEMPTS_PER_SIZE", 7)
    if max_attempts < target:
        raise SystemExit("FOD_STORAGE_MAX_ATTEMPTS_PER_SIZE must be >= FOD_STORAGE_CLEAN_RUNS")
    payload_mode = os.environ.get("FOD_STORAGE_CLEAN_PAYLOAD_MODE", "random")
    if payload_mode != "random":
        raise SystemExit(f"Clean stability benchmark requires random payloads, got {payload_mode!r}")
    return Config(
        block_sizes=parse_block_sizes(),
        target_clean_runs=target,
        max_attempts_per_size=max_attempts,
        file_size=os.environ.get("FOD_STORAGE_CLEAN_FILE_SIZE", "1G"),
        fio_block_size=os.environ.get("FOD_STORAGE_CLEAN_FIO_BLOCK_SIZE", "512k"),
        payload_mode=payload_mode,
        runtime_profile=runtime_profile,
        cargo_profile=cargo_profile,
        dirty_limit_kb=env_int("FOD_STORAGE_CLEAN_DIRTY_LIMIT_KB", 8192),
        settle_timeout_seconds=env_int("FOD_STORAGE_CLEAN_SETTLE_TIMEOUT_SECONDS", 300),
        settle_poll_seconds=env_float("FOD_STORAGE_CLEAN_SETTLE_POLL_SECONDS", 2.0, minimum=0.1),
        cooldown_seconds=env_float("FOD_STORAGE_CLEAN_COOLDOWN_SECONDS", 20.0),
        fsync_probe_count=env_int("FOD_STORAGE_CLEAN_FSYNC_PROBE_COUNT", 8),
        fsync_median_limit_ms=env_float("FOD_STORAGE_CLEAN_FSYNC_MEDIAN_LIMIT_MS", 20.0),
        fsync_max_limit_ms=env_float("FOD_STORAGE_CLEAN_FSYNC_MAX_LIMIT_MS", 100.0),
        wal_stall_ms=env_float("FOD_STORAGE_CLEAN_WAL_STALL_MS", 5000.0),
        pg_io_stall_ms=env_float("FOD_STORAGE_CLEAN_PG_IO_STALL_MS", 7000.0),
        max_clean_spread_pct=env_float("FOD_STORAGE_CLEAN_MAX_SPREAD_PCT", 25.0),
        artifact_dir=artifact_dir,
    )


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


def append_tsv(path: Path, fields: Iterable[object]) -> None:
    with path.open("a", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, delimiter="\t", lineterminator="\n")
        writer.writerow(list(fields))


def settle_host(config: Config, label: str, settle_log: Path) -> dict[str, float]:
    print(f"Host settle start label={label}")
    subprocess.run(["sync"], check=True)
    started = time.monotonic()
    probe_path = config.artifact_dir / ".fsync-probe"
    while True:
        dirty, writeback = read_meminfo_kb()
        total = dirty + writeback
        if total <= config.dirty_limit_kb:
            time.sleep(config.cooldown_seconds)
            dirty, writeback = read_meminfo_kb()
            total = dirty + writeback
            median_ms, max_ms = fsync_probe(probe_path, config.fsync_probe_count)
            ready = (
                total <= config.dirty_limit_kb
                and median_ms <= config.fsync_median_limit_ms
                and max_ms <= config.fsync_max_limit_ms
            )
            append_tsv(
                settle_log,
                [utc_now(), label, dirty, writeback, total, f"{median_ms:.3f}", f"{max_ms:.3f}", "ready" if ready else "retry"],
            )
            if ready:
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
        else:
            append_tsv(settle_log, [utc_now(), label, dirty, writeback, total, "-", "-", "dirty"])

        if time.monotonic() - started >= config.settle_timeout_seconds:
            raise RuntimeError(
                f"Host did not reach clean I/O baseline within {config.settle_timeout_seconds}s: "
                f"label={label} dirty_kb={dirty} writeback_kb={writeback}"
            )
        time.sleep(config.settle_poll_seconds)


def stream_command(command: list[str], *, cwd: Path, env: dict[str, str], log_path: Path) -> int:
    with log_path.open("w", encoding="utf-8") as log:
        proc = subprocess.Popen(
            command,
            cwd=cwd,
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


def read_single_summary(path: Path) -> dict[str, str]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle, delimiter="\t"))
    if len(rows) != 1:
        raise RuntimeError(f"Expected exactly one summary row in {path}, got {len(rows)}")
    return rows[0]


def numeric_delta(first: list[str], last: list[str], index: int) -> float:
    return float(last[index]) - float(first[index])


def wal_metrics(path: Path) -> dict[str, float]:
    if not path.exists():
        return {key: 0.0 for key in ("wal_bytes", "wal_buffers_full", "wal_write", "wal_sync", "wal_write_time", "wal_sync_time")}
    rows = [line.rstrip("\n").split("\t") for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if len(rows) < 2:
        return {key: 0.0 for key in ("wal_bytes", "wal_buffers_full", "wal_write", "wal_sync", "wal_write_time", "wal_sync_time")}
    first, last = rows[0], rows[-1]
    return {
        "wal_bytes": numeric_delta(first, last, 3),
        "wal_buffers_full": numeric_delta(first, last, 4),
        "wal_write": numeric_delta(first, last, 5),
        "wal_sync": numeric_delta(first, last, 6),
        "wal_write_time": numeric_delta(first, last, 7),
        "wal_sync_time": numeric_delta(first, last, 8),
    }


def io_metrics(path: Path) -> dict[str, float]:
    metrics = {
        "pg_io_wait": 0.0,
        "client_write_time": 0.0,
        "client_extend_time": 0.0,
        "background_writeback_time": 0.0,
        "checkpointer_fsync_time": 0.0,
    }
    if not path.exists():
        return metrics
    by_backend: dict[str, list[list[str]]] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        if not raw.strip():
            continue
        fields = raw.split("\t")
        if len(fields) < 13:
            continue
        by_backend.setdefault(fields[1], []).append(fields)

    timing_indexes = (3, 5, 7, 9, 12)  # read/write/writeback/extend/fsync time
    for backend, rows in by_backend.items():
        if len(rows) < 2:
            continue
        first, last = rows[0], rows[-1]
        metrics["pg_io_wait"] += sum(numeric_delta(first, last, idx) for idx in timing_indexes)
        if backend == "client backend":
            metrics["client_write_time"] = numeric_delta(first, last, 5)
            metrics["client_extend_time"] = numeric_delta(first, last, 9)
        elif backend == "background writer":
            metrics["background_writeback_time"] = numeric_delta(first, last, 7)
        elif backend == "checkpointer":
            metrics["checkpointer_fsync_time"] = numeric_delta(first, last, 12)
    return metrics


def f(value: str | float | int) -> float:
    return float(value)


RUN_FIELDS = [
    "cycle", "attempt", "order", "storage_block_size",
    "primary_write_mib_s", "primary_read_mib_s", "replica_read_mib_s",
    "copy_calls", "copy_rows", "copy_exec_ms", "copy_mean_ms",
    "insert_calls", "insert_rows", "insert_exec_ms", "insert_mean_ms", "sql_flush_mean_ms",
    "insert_wal_bytes", "wal_bytes_delta", "wal_buffers_full_delta", "wal_write_delta", "wal_sync_delta",
    "wal_write_time_delta_ms", "wal_sync_time_delta_ms", "pg_io_wait_delta_ms",
    "client_write_time_delta_ms", "client_extend_time_delta_ms", "background_writeback_time_delta_ms",
    "checkpointer_fsync_time_delta_ms", "pre_dirty_kb", "pre_writeback_kb", "pre_fsync_median_ms",
    "pre_fsync_max_ms", "post_dirty_kb", "post_writeback_kb", "run_quality", "profile_artifact_dir",
]


def write_runs(path: Path, rows: list[dict[str, object]]) -> None:
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=RUN_FIELDS, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def run_attempt(
    config: Config,
    *,
    cycle: int,
    attempt: int,
    order: int,
    block_size: int,
    settle_log: Path,
) -> dict[str, object]:
    run_dir = config.artifact_dir / f"cycle-{cycle}" / f"block-{block_size}-attempt-{attempt}"
    run_dir.mkdir(parents=True, exist_ok=True)
    print(
        f"\n--- cycle={cycle} order={order} block_size={block_size} attempt={attempt} ---"
    )
    pre = settle_host(config, f"c{cycle}-o{order}-b{block_size}-a{attempt}", settle_log)

    env = os.environ.copy()
    env.update(
        {
            "FOD_CARGO_PROFILE": config.cargo_profile,
            "FOD_RUNTIME_PROFILE": config.runtime_profile,
            "FOD_STORAGE_BLOCK_SKIP_BUILD": "1",
            "FOD_STORAGE_BLOCK_SIZES": str(block_size),
            "FOD_STORAGE_BLOCK_FILE_SIZE": config.file_size,
            "FOD_STORAGE_BLOCK_FIO_BLOCK_SIZE": config.fio_block_size,
            "FOD_STORAGE_BLOCK_PAYLOAD_MODE": config.payload_mode,
            "FOD_STORAGE_BLOCK_ARTIFACT_DIR": str(run_dir),
            "FOD_REQUIRE_AC_POWER": os.environ.get("FOD_REQUIRE_AC_POWER", "1"),
        }
    )
    status = stream_command(["bash", str(MATRIX)], cwd=ROOT, env=env, log_path=run_dir / "run.log")
    post_dirty, post_writeback = read_meminfo_kb()
    if status:
        raise RuntimeError(f"Benchmark failed block_size={block_size} attempt={attempt} rc={status}")

    summary = read_single_summary(run_dir / "summary.tsv")
    profile_dir = Path(summary["profile_artifact_dir"])
    copy_calls = f(summary["copy_calls"])
    insert_calls = f(summary["insert_calls"])
    copy_exec = f(summary["copy_exec_ms"])
    insert_exec = f(summary["insert_exec_ms"])
    copy_mean = copy_exec / copy_calls if copy_calls else 0.0
    insert_mean = insert_exec / insert_calls if insert_calls else 0.0
    wal = wal_metrics(profile_dir / "wal.tsv")
    io = io_metrics(profile_dir / "io.tsv")

    quality = "clean"
    if wal["wal_sync_time"] > config.wal_stall_ms:
        quality = "wal_stall"
    elif io["pg_io_wait"] > config.pg_io_stall_ms:
        quality = "io_stall"

    row: dict[str, object] = {
        "cycle": cycle,
        "attempt": attempt,
        "order": order,
        "storage_block_size": block_size,
        "primary_write_mib_s": summary["primary_write_mib_s"],
        "primary_read_mib_s": summary["primary_read_mib_s"],
        "replica_read_mib_s": summary["replica_read_mib_s"],
        "copy_calls": summary["copy_calls"],
        "copy_rows": summary["copy_rows"],
        "copy_exec_ms": summary["copy_exec_ms"],
        "copy_mean_ms": f"{copy_mean:.3f}",
        "insert_calls": summary["insert_calls"],
        "insert_rows": summary["insert_rows"],
        "insert_exec_ms": summary["insert_exec_ms"],
        "insert_mean_ms": f"{insert_mean:.3f}",
        "sql_flush_mean_ms": f"{copy_mean + insert_mean:.3f}",
        "insert_wal_bytes": summary["insert_wal_bytes"],
        "wal_bytes_delta": f"{wal['wal_bytes']:.0f}",
        "wal_buffers_full_delta": f"{wal['wal_buffers_full']:.0f}",
        "wal_write_delta": f"{wal['wal_write']:.0f}",
        "wal_sync_delta": f"{wal['wal_sync']:.0f}",
        "wal_write_time_delta_ms": f"{wal['wal_write_time']:.3f}",
        "wal_sync_time_delta_ms": f"{wal['wal_sync_time']:.3f}",
        "pg_io_wait_delta_ms": f"{io['pg_io_wait']:.3f}",
        "client_write_time_delta_ms": f"{io['client_write_time']:.3f}",
        "client_extend_time_delta_ms": f"{io['client_extend_time']:.3f}",
        "background_writeback_time_delta_ms": f"{io['background_writeback_time']:.3f}",
        "checkpointer_fsync_time_delta_ms": f"{io['checkpointer_fsync_time']:.3f}",
        "pre_dirty_kb": int(pre["dirty_kb"]),
        "pre_writeback_kb": int(pre["writeback_kb"]),
        "pre_fsync_median_ms": f"{pre['fsync_median_ms']:.3f}",
        "pre_fsync_max_ms": f"{pre['fsync_max_ms']:.3f}",
        "post_dirty_kb": post_dirty,
        "post_writeback_kb": post_writeback,
        "run_quality": quality,
        "profile_artifact_dir": str(profile_dir),
    }
    print(
        f"run_quality={quality} block_size={block_size} "
        f"write_mib_s={summary['primary_write_mib_s']} "
        f"wal_sync_ms={wal['wal_sync_time']:.3f} pg_io_wait_ms={io['pg_io_wait']:.3f}"
    )
    return row


def numeric_values(rows: list[dict[str, object]], field: str) -> list[float]:
    return [float(row[field]) for row in rows]


def median_value(rows: list[dict[str, object]], field: str) -> float:
    values = numeric_values(rows, field)
    return statistics.median(values) if values else 0.0


def table_print(path: Path) -> None:
    print(path.read_text(encoding="utf-8"), end="")


def main() -> int:
    config = load_config()
    if not MATRIX.exists():
        raise SystemExit(f"Missing matrix runner: {MATRIX}")
    if not Path("/proc/meminfo").is_file():
        raise SystemExit("/proc/meminfo is required")

    config.artifact_dir.mkdir(parents=True, exist_ok=True)
    runs_path = config.artifact_dir / "runs.tsv"
    medians_path = config.artifact_dir / "median.tsv"
    settle_log = config.artifact_dir / "settle.tsv"
    append_tsv(settle_log, ["timestamp", "label", "dirty_kb", "writeback_kb", "total_kb", "fsync_median_ms", "fsync_max_ms", "status"])

    print("=== FOD CLEAN STORAGE BLOCK STABILITY MATRIX ===")
    print(f"storage_block_sizes={' '.join(map(str, config.block_sizes))}")
    print(f"target_clean_runs={config.target_clean_runs}")
    print(f"max_attempts_per_size={config.max_attempts_per_size}")
    print(f"file_size={config.file_size} fio_block_size={config.fio_block_size} payload_mode={config.payload_mode}")
    print(
        f"dirty_limit_kb={config.dirty_limit_kb} cooldown_s={config.cooldown_seconds} "
        f"fsync_median_limit_ms={config.fsync_median_limit_ms} fsync_max_limit_ms={config.fsync_max_limit_ms}"
    )
    print(f"wal_stall_ms={config.wal_stall_ms} pg_io_stall_ms={config.pg_io_stall_ms}")
    print(f"artifact_dir={config.artifact_dir}")

    build_env = os.environ.copy()
    build_env.update({"FOD_CARGO_PROFILE": config.cargo_profile, "FOD_RUNTIME_PROFILE": config.runtime_profile})
    subprocess.run(["make", "--no-print-directory", "build-runtime"], cwd=ROOT, env=build_env, check=True)

    rows: list[dict[str, object]] = []
    attempts = {size: 0 for size in config.block_sizes}
    clean_counts = {size: 0 for size in config.block_sizes}
    cycle = 1
    while True:
        pending = [size for size in config.block_sizes if clean_counts[size] < config.target_clean_runs]
        if not pending:
            break
        runnable = [size for size in pending if attempts[size] < config.max_attempts_per_size]
        if not runnable:
            break

        rotation = (cycle - 1) % len(config.block_sizes)
        ordered = config.block_sizes[rotation:] + config.block_sizes[:rotation]
        print(f"\n=== CLEAN CYCLE {cycle} rotation={rotation} ===")
        for order, block_size in enumerate(ordered, start=1):
            if clean_counts[block_size] >= config.target_clean_runs:
                continue
            if attempts[block_size] >= config.max_attempts_per_size:
                continue
            attempts[block_size] += 1
            row = run_attempt(
                config,
                cycle=cycle,
                attempt=attempts[block_size],
                order=order,
                block_size=block_size,
                settle_log=settle_log,
            )
            rows.append(row)
            if row["run_quality"] == "clean":
                clean_counts[block_size] += 1
            write_runs(runs_path, rows)
            print(f"clean_progress block_size={block_size} clean={clean_counts[block_size]}/{config.target_clean_runs}")
        cycle += 1

    median_fields = [
        "storage_block_size", "attempts", "clean_runs", "stall_runs",
        "median_primary_write_mib_s_clean", "min_primary_write_mib_s_clean", "max_primary_write_mib_s_clean",
        "clean_write_spread_pct", "median_primary_read_mib_s_clean", "median_replica_read_mib_s_clean",
        "median_copy_mean_ms_clean", "median_insert_mean_ms_clean", "median_sql_flush_mean_ms_clean",
        "median_wal_bytes_delta_clean", "median_wal_sync_time_delta_ms_clean", "median_pg_io_wait_delta_ms_clean",
        "max_wal_sync_time_delta_ms_all", "max_pg_io_wait_delta_ms_all", "status",
    ]
    median_rows: list[dict[str, object]] = []
    selection_status = "valid"
    for block_size in config.block_sizes:
        all_rows = [row for row in rows if int(row["storage_block_size"]) == block_size]
        clean_rows = [row for row in all_rows if row["run_quality"] == "clean"]
        write_values = numeric_values(clean_rows, "primary_write_mib_s")
        med_write = statistics.median(write_values) if write_values else 0.0
        min_write = min(write_values) if write_values else 0.0
        max_write = max(write_values) if write_values else 0.0
        spread = ((max_write - min_write) * 100.0 / med_write) if med_write else 0.0
        status = "OK"
        if len(clean_rows) < config.target_clean_runs:
            status = "INSUFFICIENT_CLEAN_RUNS"
        elif spread > config.max_clean_spread_pct:
            status = "UNSTABLE_CLEAN_RUNS"
        if status != "OK":
            selection_status = "invalid"
        median_rows.append(
            {
                "storage_block_size": block_size,
                "attempts": len(all_rows),
                "clean_runs": len(clean_rows),
                "stall_runs": len(all_rows) - len(clean_rows),
                "median_primary_write_mib_s_clean": f"{med_write:.3f}",
                "min_primary_write_mib_s_clean": f"{min_write:.3f}",
                "max_primary_write_mib_s_clean": f"{max_write:.3f}",
                "clean_write_spread_pct": f"{spread:.2f}",
                "median_primary_read_mib_s_clean": f"{median_value(clean_rows, 'primary_read_mib_s'):.3f}",
                "median_replica_read_mib_s_clean": f"{median_value(clean_rows, 'replica_read_mib_s'):.3f}",
                "median_copy_mean_ms_clean": f"{median_value(clean_rows, 'copy_mean_ms'):.3f}",
                "median_insert_mean_ms_clean": f"{median_value(clean_rows, 'insert_mean_ms'):.3f}",
                "median_sql_flush_mean_ms_clean": f"{median_value(clean_rows, 'sql_flush_mean_ms'):.3f}",
                "median_wal_bytes_delta_clean": f"{median_value(clean_rows, 'wal_bytes_delta'):.0f}",
                "median_wal_sync_time_delta_ms_clean": f"{median_value(clean_rows, 'wal_sync_time_delta_ms'):.3f}",
                "median_pg_io_wait_delta_ms_clean": f"{median_value(clean_rows, 'pg_io_wait_delta_ms'):.3f}",
                "max_wal_sync_time_delta_ms_all": f"{max(numeric_values(all_rows, 'wal_sync_time_delta_ms'), default=0.0):.3f}",
                "max_pg_io_wait_delta_ms_all": f"{max(numeric_values(all_rows, 'pg_io_wait_delta_ms'), default=0.0):.3f}",
                "status": status,
            }
        )

    with medians_path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=median_fields, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(median_rows)

    best_size = ""
    best_write = ""
    if selection_status == "valid":
        best = max(median_rows, key=lambda row: float(row["median_primary_write_mib_s_clean"]))
        best_size = str(best["storage_block_size"])
        best_write = str(best["median_primary_write_mib_s_clean"])

    print("\n=== CLEAN STORAGE BLOCK RUNS ===")
    table_print(runs_path)
    print("\n=== CLEAN STORAGE BLOCK MEDIANS ===")
    table_print(medians_path)
    print(f"selection_status={selection_status}")
    if selection_status == "valid":
        print(f"best_clean_median_primary_write_block_size={best_size}")
        print(f"best_clean_median_primary_write_mib_s={best_write}")
    else:
        print("No block size selected: clean-run target or stability criterion was not met.")
    print(f"settle_log={settle_log}")
    print(f"stability_artifact_dir={config.artifact_dir}")
    print("OK: clean storage block stability matrix")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
