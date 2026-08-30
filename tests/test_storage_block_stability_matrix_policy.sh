#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT}/scripts/perf/run_storage_block_stability_matrix.sh"

bash -n "${SCRIPT}"

grep -Fq 'STORAGE_BLOCK_SIZES="${FOD_STORAGE_STABILITY_BLOCK_SIZES:-16384 32768 65536}"' "${SCRIPT}"
grep -Fq 'REPEATS="${FOD_STORAGE_STABILITY_REPEATS:-3}"' "${SCRIPT}"
grep -Fq 'FILE_SIZE="${FOD_STORAGE_STABILITY_FILE_SIZE:-1G}"' "${SCRIPT}"
grep -Fq 'FIO_BLOCK_SIZE="${FOD_STORAGE_STABILITY_FIO_BLOCK_SIZE:-512k}"' "${SCRIPT}"
grep -Fq 'PAYLOAD_MODE="${FOD_STORAGE_STABILITY_PAYLOAD_MODE:-random}"' "${SCRIPT}"

grep -Fq 'rotation=$(( (repeat - 1) % size_count ))' "${SCRIPT}"
grep -Fq 'index=$(( (rotation + offset) % size_count ))' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_BLOCK_SIZES="${storage_block_size}"' "${SCRIPT}"
grep -Fq 'FOD_STORAGE_BLOCK_ARTIFACT_DIR="${run_dir}"' "${SCRIPT}"
grep -Fq 'bash "${MATRIX}" 2>&1 | tee "${run_log}"' "${SCRIPT}"

grep -Fq 'wal_sync_time_delta_ms' "${SCRIPT}"
grep -Fq 'wal_write_time_delta_ms' "${SCRIPT}"
grep -Fq 'wal_buffers_full_delta' "${SCRIPT}"
grep -Fq 'median_primary_write_mib_s' "${SCRIPT}"
grep -Fq 'write_spread_pct' "${SCRIPT}"
grep -Fq 'median_sql_flush_mean_ms' "${SCRIPT}"
grep -Fq 'max_wal_sync_time_delta_ms' "${SCRIPT}"
grep -Fq 'best_median_primary_write_block_size=' "${SCRIPT}"

if grep -Eq 'UPDATE[[:space:]]+config|DELETE[[:space:]]+FROM[[:space:]]+config|fod_config\.ini' "${SCRIPT}"; then
    echo "Storage stability matrix must not rewrite production/runtime configuration" >&2
    exit 1
fi

if grep -Eq 'docker[[:space:]]+system[[:space:]]+prune|docker[[:space:]]+volume[[:space:]]+prune' "${SCRIPT}"; then
    echo "Storage stability matrix must not perform global Docker cleanup" >&2
    exit 1
fi

echo "Storage block stability matrix policy: OK"
