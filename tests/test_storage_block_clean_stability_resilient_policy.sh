#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT}/scripts/perf/run_storage_block_clean_stability_resilient.py"

python3 -m py_compile "${SCRIPT}"

grep -Fq 'run_storage_block_clean_stability.py' "${SCRIPT}"
grep -Fq 'os.environ.setdefault("REPLICA_WAIT_SECONDS", "30")' "${SCRIPT}"
grep -Fq 'run_quality": "infra_error"' "${SCRIPT}"
grep -Fq 'except (RuntimeError, TimeoutError, OSError)' "${SCRIPT}"
grep -Fq 'module.run_attempt = resilient_run_attempt' "${SCRIPT}"
grep -Fq 'return int(module.main())' "${SCRIPT}"

if grep -Eq 'UPDATE[[:space:]]+config|DELETE[[:space:]]+FROM[[:space:]]+config|fod_config\.ini' "${SCRIPT}"; then
    echo "Resilient stability wrapper must not rewrite FOD production/runtime configuration" >&2
    exit 1
fi

if grep -Eq 'docker[[:space:]]+system[[:space:]]+prune|docker[[:space:]]+volume[[:space:]]+prune' "${SCRIPT}"; then
    echo "Resilient stability wrapper must not perform global Docker cleanup" >&2
    exit 1
fi

echo "Storage block resilient stability policy: OK"
