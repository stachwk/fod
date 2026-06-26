#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import os
import sys
import tempfile
import time
import uuid
from pathlib import Path

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount
from tests.integration.fod_postgres_benchmark import capture_postgres_stats, diff_stats, resolve_postgres_dsn


def _size_to_bytes(value: str) -> int:
    value = value.strip()
    suffix = value[-1:].lower()
    if suffix == "k":
        return int(value[:-1]) * 1024
    if suffix == "m":
        return int(value[:-1]) * 1024 * 1024
    if suffix == "g":
        return int(value[:-1]) * 1024 * 1024 * 1024
    return int(value)


def _bool_env(name: str, default: str = "1") -> bool:
    value = os.environ.get(name, default).strip().lower()
    return value not in {"0", "false", "no", "off", ""}


def _build_payload(size: int) -> bytes:
    seed = b"fod-postgres-wal-pressure-"
    repeated = seed * ((size // len(seed)) + 1)
    return repeated[:size]


def main() -> None:
    file_count = int(os.environ.get("PG_WAL_PRESSURE_COUNT", "64"))
    block_size_text = os.environ.get("PG_WAL_PRESSURE_BLOCK_SIZE", "512k")
    sync_mode = _bool_env("PG_WAL_PRESSURE_SYNC", "1")
    force_checkpoint = _bool_env("PG_WAL_PRESSURE_FORCE_CHECKPOINT", "0")
    benchmark_label = os.environ.get("POSTGRES_BENCHMARK_LABEL", "default")
    block_size = _size_to_bytes(block_size_text)
    total_bytes = file_count * block_size
    payload = _build_payload(block_size)
    suffix = uuid.uuid4().hex[:8]

    dsn = resolve_postgres_dsn(ROOT)
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    with tempfile.TemporaryDirectory(prefix=f"/tmp/fod-pg-wal-{suffix}.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint))
        try:
            stats_before = capture_postgres_stats(dsn)
            target_dir = mountpoint / f"postgres_wal_pressure_{suffix}"
            target_dir.mkdir()

            start_ns = time.perf_counter_ns()
            for index in range(file_count):
                file_path = target_dir / f"chunk-{index:05d}.bin"
                with file_path.open("wb") as handle:
                    handle.write(payload)
                    if sync_mode:
                        handle.flush()
                        os.fsync(handle.fileno())
            elapsed_ns = time.perf_counter_ns() - start_ns

            actual_files = sum(1 for child in target_dir.iterdir() if child.is_file())
            if actual_files != file_count:
                raise AssertionError(
                    f"expected {file_count} WAL pressure files, got {actual_files}"
                )

            for sample_index in (0, file_count - 1):
                sample_file = target_dir / f"chunk-{sample_index:05d}.bin"
                if sample_file.stat().st_size != block_size:
                    raise AssertionError(
                        f"unexpected WAL pressure sample size for {sample_file}: "
                        f"{sample_file.stat().st_size} != {block_size}"
                    )

            checkpoint_elapsed_ns = None
            if force_checkpoint:
                checkpoint_start_ns = time.perf_counter_ns()
                with psycopg2.connect(**dsn) as conn:
                    conn.autocommit = True
                    with conn.cursor() as cur:
                        cur.execute("CHECKPOINT")
                checkpoint_elapsed_ns = time.perf_counter_ns() - checkpoint_start_ns

            stats_after = capture_postgres_stats(dsn)
        finally:
            launcher.stop()

    bgwriter_delta = diff_stats(stats_after["bgwriter"], stats_before["bgwriter"])
    wal_delta = diff_stats(stats_after["wal"], stats_before["wal"])
    elapsed_s = elapsed_ns / 1_000_000_000
    files_per_s = file_count / elapsed_s if elapsed_s > 0 else 0.0
    mebibytes_per_s = total_bytes / 1024 / 1024 / elapsed_s if elapsed_s > 0 else 0.0
    checkpoint_elapsed_s = (
        checkpoint_elapsed_ns / 1_000_000_000 if checkpoint_elapsed_ns is not None else None
    )

    print(
        "OK postgres/wal-pressure "
        f"backend={benchmark_label} "
        f"files={file_count} block_size={block_size_text} sync={int(sync_mode)} "
        f"checkpoint={int(force_checkpoint)} "
        f"elapsed_s={elapsed_s:.3f} files_per_s={files_per_s:.2f} "
        f"mib_per_s={mebibytes_per_s:.2f} total_bytes={total_bytes}"
    )
    if checkpoint_elapsed_s is not None:
        print(f"CHECKPOINT elapsed_s={checkpoint_elapsed_s:.3f}")
    print(
        "pg_stat_wal delta "
        + " ".join(
            f"{key}={wal_delta[key]}"
            for key in (
                "wal_records",
                "wal_fpi",
                "wal_bytes",
                "wal_buffers_full",
                "wal_write",
                "wal_sync",
                "wal_write_time",
                "wal_sync_time",
            )
            if key in wal_delta
        )
    )
    print(
        "pg_stat_bgwriter delta "
        + " ".join(
            f"{key}={bgwriter_delta[key]}"
            for key in (
                "checkpoints_timed",
                "checkpoints_req",
                "checkpoint_write_time",
                "checkpoint_sync_time",
                "buffers_checkpoint",
                "buffers_clean",
                "maxwritten_clean",
                "buffers_backend",
                "buffers_backend_fsync",
                "buffers_alloc",
            )
            if key in bgwriter_delta
        )
    )


if __name__ == "__main__":
    main()
