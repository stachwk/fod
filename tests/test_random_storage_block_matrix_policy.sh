#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT}/scripts/perf/run_random_storage_block_matrix.sh"

bash -n "${SCRIPT}"

grep -Fq 'STORAGE_BLOCK_SIZES="${FOD_STORAGE_BLOCK_SIZES:-4096 16384 65536}"' "${SCRIPT}"
grep -Fq 'FILE_SIZE="${FOD_STORAGE_BLOCK_FILE_SIZE:-1G}"' "${SCRIPT}"
grep -Fq 'FIO_BLOCK_SIZE="${FOD_STORAGE_BLOCK_FIO_BLOCK_SIZE:-512k}"' "${SCRIPT}"
grep -Fq 'PAYLOAD_MODE="${FOD_STORAGE_BLOCK_PAYLOAD_MODE:-random}"' "${SCRIPT}"

grep -Fq 'exec "${REAL}" "$@" --block-size "${BLOCK_SIZE}"' "${SCRIPT}"
grep -Fq 'FOD_MKFS_BIN="${MKFS_WRAPPER}"' "${SCRIPT}"
grep -Fq 'FOD_TEST_STORAGE_BLOCK_SIZE="${storage_block_size}"' "${SCRIPT}"

grep -Fq 'bash "${PROFILER}" >"${profile_log}" 2>&1 &' "${SCRIPT}"
grep -Fq 'bash "${SINGLE}" 2>&1 | tee "${benchmark_log}"' "${SCRIPT}"
grep -Fq 'copy_exec_ms' "${SCRIPT}"
grep -Fq 'insert_exec_ms' "${SCRIPT}"
grep -Fq 'insert_wal_bytes' "${SCRIPT}"

if grep -Eq 'UPDATE[[:space:]]+config|DELETE[[:space:]]+FROM[[:space:]]+config|fod_config\.ini' "${SCRIPT}"; then
    echo "Storage block matrix must not rewrite production/runtime configuration" >&2
    exit 1
fi

echo "Random storage block matrix policy: OK"
