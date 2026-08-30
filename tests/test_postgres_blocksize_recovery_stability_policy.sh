#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="${ROOT}/scripts/perf/run_postgres_blocksize_published32k_recovery_stability.sh"

bash -n "${RUNNER}"

grep -F 'FOD_PG_BLOCK_RECOVERY_TARGET:-3' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_RECOVERY_MAX_ATTEMPTS:-10' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_RECOVERY_WAL_SYNC_STALL_MS:-3000' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_RECOVERY_SETTLE_DIRTY_KB:-8192' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_RECOVERY_SETTLE_TIMEOUT_SECONDS:-300' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_RECOVERY_COOLDOWN_SECONDS:-20' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_RECOVERY_FSYNC_PROBE_COUNT:-8' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_RECOVERY_FSYNC_MEDIAN_MAX_MS:-20' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_RECOVERY_FSYNC_SINGLE_MAX_MS:-100' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_WRITE_PROFILE_WAL_EVERY=1' "${RUNNER}" >/dev/null
grep -F 'pg_order="8 32"' "${RUNNER}" >/dev/null
grep -F 'pg_order="32 8"' "${RUNNER}" >/dev/null
grep -F 'Host recovery gate attempt=' "${RUNNER}" >/dev/null
grep -F 'fsync_median_ms=' "${RUNNER}" >/dev/null
grep -F 'selection_status=host_not_ready' "${RUNNER}" >/dev/null
grep -F 'selection_status=insufficient_clean_runs' "${RUNNER}" >/dev/null
grep -F 'selection_status=unstable' "${RUNNER}" >/dev/null
grep -F 'postgres_32k_vs_8k_primary_write_pct=' "${RUNNER}" >/dev/null
grep -F 'postgres_32k_vs_8k_insert_mean_pct=' "${RUNNER}" >/dev/null
grep -F 'postgres_32k_vs_8k_wal_bytes_pct=' "${RUNNER}" >/dev/null

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${RUNNER}"; then
    echo "Recovery stability runner must not perform global Docker pruning" >&2
    exit 1
fi

if grep -Fq '.github/workflows' "${RUNNER}"; then
    echo "Recovery stability runner must not modify GitHub Actions" >&2
    exit 1
fi

echo "OK: PostgreSQL block-size recovery stability policy"
