#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT}/scripts/perf/run_storage_block_clean_stability.py"

python3 -m py_compile "${SCRIPT}"

grep -Fq 'FOD_STORAGE_CLEAN_BLOCK_SIZES", "16384 32768 65536"' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_CLEAN_RUNS", 3' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_MAX_ATTEMPTS_PER_SIZE", 7' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_CLEAN_FILE_SIZE", "1G"' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_CLEAN_FIO_BLOCK_SIZE", "512k"' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_CLEAN_PAYLOAD_MODE", "random"' "${SCRIPT}"

grep -Fq 'FOD_STORAGE_BLOCK_SKIP_BUILD": "1"' "${SCRIPT}"
grep -Fq 'make", "--no-print-directory", "build-runtime"' "${SCRIPT}"
grep -Fq 'fsync_probe' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_CLEAN_FSYNC_MEDIAN_LIMIT_MS' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_CLEAN_FSYNC_MAX_LIMIT_MS' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_CLEAN_COOLDOWN_SECONDS' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_CLEAN_DIRTY_LIMIT_KB' "${SCRIPT}"

grep -Fq 'wal_stall' "${SCRIPT}"
grep -Fq 'io_stall' "${SCRIPT}"
grep -Fq 'pg_io_wait_delta_ms' "${SCRIPT}"
grep -Fq 'wal_sync_time_delta_ms' "${SCRIPT}"
grep -Fq 'target_clean_runs' "${SCRIPT}"
grep -Fq 'clean_progress' "${SCRIPT}"
grep -Fq 'selection_status' "${SCRIPT}"
grep -Fq 'best_clean_median_primary_write_block_size' "${SCRIPT}"

if grep -Eq 'UPDATE[[:space:]]+config|DELETE[[:space:]]+FROM[[:space:]]+config|fod_config\.ini' "${SCRIPT}"; then
    echo "Clean stability harness must not rewrite FOD production/runtime configuration" >&2
    exit 1
fi

if grep -Eq 'docker[[:space:]]+system[[:space:]]+prune|docker[[:space:]]+volume[[:space:]]+prune' "${SCRIPT}"; then
    echo "Clean stability harness must not perform global Docker cleanup" >&2
    exit 1
fi

echo "Storage block clean stability policy: OK"
