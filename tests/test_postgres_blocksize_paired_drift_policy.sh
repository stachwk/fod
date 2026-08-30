#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="${ROOT}/scripts/perf/run_postgres_blocksize_published32k_paired_drift.sh"

bash -n "${RUNNER}"

grep -F 'design=A-B-B-A' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_PAIRED_TARGET_PER_ORIENTATION:-2' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_PAIRED_MAX_ATTEMPTS:-10' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_PAIRED_WAL_SYNC_STALL_MS:-3000' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_PAIRED_SENTINEL_WRITE_DRIFT_PCT:-10' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_PAIRED_SENTINEL_READ_DRIFT_PCT:-15' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_PAIRED_SENTINEL_SQL_DRIFT_PCT:-20' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_PAIRED_MAX_RATIO_SPREAD_PP:-15' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_PAIRED_SETTLE_DIRTY_KB:-8192' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_PAIRED_FSYNC_MEDIAN_MAX_MS:-5' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_PAIRED_FSYNC_SINGLE_MAX_MS:-25' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_REPEATS=2' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_WRITE_PROFILE_WAL_EVERY=1' "${RUNNER}" >/dev/null
grep -F 'sequence="8 32"; sentinel=8' "${RUNNER}" >/dev/null
grep -F 'sequence="32 8"; sentinel=32' "${RUNNER}" >/dev/null
grep -F 'Expected four A-B-B-A runs' "${RUNNER}" >/dev/null
grep -F 'quality=host_drift' "${RUNNER}" >/dev/null
grep -F 'quality=wal_stall' "${RUNNER}" >/dev/null
grep -F 'selection_status=unstable_pair_ratio' "${RUNNER}" >/dev/null

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${RUNNER}"; then
    echo "Paired drift benchmark must not perform global Docker pruning" >&2
    exit 1
fi
if grep -Fq '.github/workflows' "${RUNNER}"; then
    echo "Paired drift benchmark must not modify GitHub Actions" >&2
    exit 1
fi

echo "OK: PostgreSQL block-size paired drift policy"
