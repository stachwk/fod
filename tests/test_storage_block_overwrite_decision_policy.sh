#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ONE="${ROOT}/tests/integration/test_fio_storage_block_decision_docker.sh"
RUNNER="${ROOT}/scripts/perf/run_storage_block_overwrite_decision.py"

bash -n "${ONE}"
python3 -m py_compile "${RUNNER}"

grep -Fq 'randwrite-4k)' "${ONE}"
grep -Fq 'randwrite-16k)' "${ONE}"
grep -Fq 'randwrite-64k)' "${ONE}"
grep -Fq 'seqwrite-512k)' "${ONE}"
grep -Fq 'PREFILL=1' "${ONE}"
grep -Fq -- '--overwrite=1' "${ONE}"
grep -Fq -- '--refill_buffers=1 --randrepeat=0' "${ONE}"
grep -Fq 'CHECKPOINT; SELECT pg_current_wal_flush_lsn()::text' "${ONE}"
grep -Fq 'wait_for_replay_lsn "${BASELINE_LSN}"' "${ONE}"
grep -Fq 'wait_host_dirty' "${ONE}"
grep -Fq 'pg-stat-statements-reset.tsv' "${ONE}"
grep -Fq 'FOD_PG_WRITE_PROFILE_PROCESS_MATCH="/tmp/fod-storage-decision-measured."' "${ONE}"
grep -Fq 'wait_for_replay_lsn "${MEASURED_LSN}"' "${ONE}"

grep -Fq 'FOD_STORAGE_DECISION_BLOCK_SIZES", "32768 65536"' "${RUNNER}"
grep -Fq 'FOD_STORAGE_DECISION_RUNS", 3' "${RUNNER}"
grep -Fq 'randwrite-4k' "${RUNNER}"
grep -Fq 'randwrite-16k' "${RUNNER}"
grep -Fq 'randwrite-64k' "${RUNNER}"
grep -Fq 'seqwrite-512k' "${RUNNER}"
grep -Fq 'wal_amplification' "${RUNNER}"
grep -Fq 'copy_local_page_write_amplification' "${RUNNER}"
grep -Fq 'insert_shared_page_write_amplification' "${RUNNER}"
grep -Fq 'four_k_64k_vs_32k_pct=' "${RUNNER}"
grep -Fq 'geometric_mean_64k_vs_32k_ratio=' "${RUNNER}"
grep -Fq 'decision_hint_block_size=' "${RUNNER}"

if grep -Eq 'UPDATE[[:space:]]+config|DELETE[[:space:]]+FROM[[:space:]]+config|fod_config\.ini' "${ONE}" "${RUNNER}"; then
    echo "Decision benchmark must not rewrite production/runtime configuration" >&2
    exit 1
fi

if grep -Eq 'docker[[:space:]]+system[[:space:]]+prune|docker[[:space:]]+volume[[:space:]]+prune' "${ONE}" "${RUNNER}"; then
    echo "Decision benchmark must not perform global Docker cleanup" >&2
    exit 1
fi

echo "Storage block overwrite decision policy: OK"
