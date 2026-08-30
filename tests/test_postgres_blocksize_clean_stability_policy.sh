#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="${ROOT}/scripts/perf/run_postgres_blocksize_published32k_clean_stability.sh"

bash -n "${RUNNER}"

grep -F 'FOD_PG_BLOCK_CLEAN_TARGET:-3' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_CLEAN_MAX_ATTEMPTS:-8' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_CLEAN_WAL_SYNC_STALL_MS:-3000' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_CLEAN_MAX_WRITE_SPREAD_PCT:-20' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_SIZES_KB="8 32"' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_REPEATS=1' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_FOD_BLOCK_SIZE=32768' "${RUNNER}" >/dev/null
grep -F 'quality=wal_stall' "${RUNNER}" >/dev/null
grep -F 'selection_status=insufficient_clean_runs' "${RUNNER}" >/dev/null
grep -F 'selection_status=unstable' "${RUNNER}" >/dev/null
grep -F 'postgres_32k_vs_8k_primary_write_pct=' "${RUNNER}" >/dev/null
grep -F 'postgres_32k_vs_8k_primary_read_pct=' "${RUNNER}" >/dev/null
grep -F 'postgres_32k_vs_8k_replica_read_pct=' "${RUNNER}" >/dev/null
grep -F 'postgres_32k_vs_8k_insert_wal_bytes_pct=' "${RUNNER}" >/dev/null
grep -F 'postgres_32k_vs_8k_wal_sync_time_pct=' "${RUNNER}" >/dev/null

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${RUNNER}"; then
    echo "Clean stability runner must not perform global Docker pruning" >&2
    exit 1
fi
if grep -Fq '.github/workflows' "${RUNNER}"; then
    echo "Clean stability runner must not modify GitHub Actions" >&2
    exit 1
fi

echo "OK: PostgreSQL block-size clean stability policy"
