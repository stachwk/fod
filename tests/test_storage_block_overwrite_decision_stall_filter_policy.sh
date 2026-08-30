#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="${ROOT}/scripts/perf/run_storage_block_overwrite_decision_stall_filtered.py"

python3 -m py_compile "${RUNNER}"

grep -Fq 'FOD_STORAGE_DECISION_WAL_STALL_MS", 3000.0' "${RUNNER}"
grep -Fq 'FOD_STORAGE_DECISION_PG_IO_STALL_MS", 7000.0' "${RUNNER}"
grep -Fq 'FOD_STORAGE_DECISION_MAX_ATTEMPTS", "12"' "${RUNNER}"
grep -Fq 'FOD_PG_WRITE_PROFILE_WAL_EVERY", "1"' "${RUNNER}"
grep -Fq 'row["run_quality"] = "wal_stall"' "${RUNNER}"
grep -Fq 'row["run_quality"] = "io_stall"' "${RUNNER}"
grep -Fq 'slow successful runs below these objective I/O limits remain valid' "${RUNNER}"

if grep -Eq 'UPDATE[[:space:]]+config|DELETE[[:space:]]+FROM[[:space:]]+config|fod_config\.ini' "${RUNNER}"; then
    echo "Stall-filter wrapper must not rewrite production configuration" >&2
    exit 1
fi

if grep -Eq 'docker[[:space:]]+system[[:space:]]+prune|docker[[:space:]]+volume[[:space:]]+prune' "${RUNNER}"; then
    echo "Stall-filter wrapper must not perform global Docker cleanup" >&2
    exit 1
fi

echo "Storage block overwrite stall filter policy: OK"
