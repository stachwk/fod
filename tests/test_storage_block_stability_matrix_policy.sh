#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT}/scripts/perf/run_storage_block_stability_matrix.sh"
MATRIX="${ROOT}/scripts/perf/run_random_storage_block_matrix.sh"

bash -n "${SCRIPT}"
bash -n "${MATRIX}"

grep -Fq 'STORAGE_BLOCK_SIZES="${FOD_STORAGE_STABILITY_BLOCK_SIZES:-16384 32768 65536}"' "${SCRIPT}"
grep -Fq 'REPEATS="${FOD_STORAGE_STABILITY_REPEATS:-3}"' "${SCRIPT}"
grep -Fq 'FILE_SIZE="${FOD_STORAGE_STABILITY_FILE_SIZE:-1G}"' "${SCRIPT}"
grep -Fq 'FIO_BLOCK_SIZE="${FOD_STORAGE_STABILITY_FIO_BLOCK_SIZE:-512k}"' "${SCRIPT}"
grep -Fq 'PAYLOAD_MODE="${FOD_STORAGE_STABILITY_PAYLOAD_MODE:-random}"' "${SCRIPT}"

grep -Fq 'SETTLE_ENABLED="${FOD_STORAGE_STABILITY_SETTLE:-1}"' "${SCRIPT}"
grep -Fq 'SETTLE_DIRTY_KB="${FOD_STORAGE_STABILITY_SETTLE_DIRTY_KB:-32768}"' "${SCRIPT}"
grep -Fq 'SETTLE_TIMEOUT_SECONDS="${FOD_STORAGE_STABILITY_SETTLE_TIMEOUT_SECONDS:-180}"' "${SCRIPT}"
grep -Fq 'WAL_STALL_MS="${FOD_STORAGE_STABILITY_WAL_STALL_MS:-5000}"' "${SCRIPT}"
grep -Fq 'MAX_CLEAN_SPREAD_PCT="${FOD_STORAGE_STABILITY_MAX_CLEAN_SPREAD_PCT:-25}"' "${SCRIPT}"
grep -Fq 'sync' "${SCRIPT}"
grep -Fq 'meminfo_kb Dirty' "${SCRIPT}"
grep -Fq 'meminfo_kb Writeback' "${SCRIPT}"
grep -Fq 'refusing contaminated benchmark' "${SCRIPT}"

grep -Fq 'rotation=$(( (repeat - 1) % size_count ))' "${SCRIPT}"
grep -Fq 'index=$(( (rotation + offset) % size_count ))' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_BLOCK_SKIP_BUILD=1' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_BLOCK_SIZES="${storage_block_size}"' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_BLOCK_ARTIFACT_DIR="${run_dir}"' "${SCRIPT}"
grep -Fq 'bash "${MATRIX}" 2>&1 | tee "${run_log}"' "${SCRIPT}"

grep -Fq 'wal_sync_time_delta_ms' "${SCRIPT}"
grep -Fq 'wal_write_time_delta_ms' "${SCRIPT}"
grep -Fq 'wal_buffers_full_delta' "${SCRIPT}"
grep -Fq 'run_quality="wal_stall"' "${SCRIPT}"
grep -Fq 'median_primary_write_mib_s_clean' "${SCRIPT}"
grep -Fq 'clean_write_spread_pct' "${SCRIPT}"
grep -Fq 'INSUFFICIENT_CLEAN_RUNS' "${SCRIPT}"
grep -Fq 'UNSTABLE_CLEAN_RUNS' "${SCRIPT}"
grep -Fq 'selection_status="invalid"' "${SCRIPT}"
grep -Fq 'best_clean_median_primary_write_block_size=' "${SCRIPT}"
grep -Fq 'settle_log=' "${SCRIPT}"

grep -Fq 'SKIP_BUILD="${FOD_STORAGE_BLOCK_SKIP_BUILD:-0}"' "${MATRIX}"
grep -Fq 'Runtime build skipped; caller is responsible for prebuilding artifacts.' "${MATRIX}"

if grep -Eq 'UPDATE[[:space:]]+config|DELETE[[:space:]]+FROM[[:space:]]+config|fod_config\.ini' "${SCRIPT}"; then
    echo "Storage stability matrix must not rewrite production/runtime configuration" >&2
    exit 1
fi

if grep -Eq 'docker[[:space:]]+system[[:space:]]+prune|docker[[:space:]]+volume[[:space:]]+prune' "${SCRIPT}"; then
    echo "Storage stability matrix must not perform global Docker cleanup" >&2
    exit 1
fi

echo "Storage block stability matrix policy: OK"
