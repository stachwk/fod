#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

CORRECTNESS_TEST="tests/integration/test_storage_block_size_correctness.py"
CORRECTNESS_RUNNER="scripts/perf/run_storage_block_correctness_matrix.sh"
CANDIDATE_RUNNER="scripts/perf/run_storage_block_candidate_validation.sh"
PERF_RUNNER="scripts/perf/run_random_storage_block_matrix.sh"

python3 -m py_compile "${CORRECTNESS_TEST}"
bash -n "${CORRECTNESS_RUNNER}"
bash -n "${CANDIDATE_RUNNER}"
bash -n "${PERF_RUNNER}"

grep -Fq 'FOD_STORAGE_CORRECTNESS_BLOCK_SIZES:-4096 16384 65536' "${CORRECTNESS_RUNNER}"
grep -Fq 'FOD_STORAGE_CANDIDATE_BLOCK_SIZES:-8192 16384 32768 65536' "${CANDIDATE_RUNNER}"
grep -Fq 'FOD_STORAGE_CANDIDATE_FILE_SIZE:-1G' "${CANDIDATE_RUNNER}"
grep -Fq 'FOD_STORAGE_CANDIDATE_FIO_BLOCK_SIZE:-512k' "${CANDIDATE_RUNNER}"
grep -Fq 'FOD_STORAGE_CANDIDATE_PAYLOAD_MODE:-random' "${CANDIDATE_RUNNER}"
grep -Fq 'payload mode must remain random' "${CANDIDATE_RUNNER}"

grep -Fq 'FOD_TEST_STORAGE_BLOCK_SIZE' "${CORRECTNESS_TEST}"
grep -Fq 'single_byte_write=OK' "${CORRECTNESS_TEST}"
grep -Fq 'unaligned_partial_append_4k_reads=OK' "${CORRECTNESS_TEST}"
grep -Fq 'truncate_exact_off_boundary_extend=OK' "${CORRECTNESS_TEST}"
grep -Fq 'sparse_hole_zero_fill=OK' "${CORRECTNESS_TEST}"
grep -Fq 'fallocate_zero_fill=OK' "${CORRECTNESS_TEST}"
grep -Fq 'copy_file_range_unaligned=OK' "${CORRECTNESS_TEST}"
grep -Fq 'concurrent_disjoint_partial_writes=OK' "${CORRECTNESS_TEST}"
grep -Fq 'remount_durability' "${CORRECTNESS_TEST}"
grep -Fq 'FOD_COPY_DEDUPE_ENABLED' "${CORRECTNESS_TEST}"
grep -Fq 'FOD_COPY_DEDUPE_CRC_TABLE' "${CORRECTNESS_TEST}"
grep -Fq 'SELECT COUNT(*) FROM fod.copy_block_crc' "${CORRECTNESS_TEST}"
grep -Fq 'copy_dedupe_crc_remount=OK' "${CORRECTNESS_TEST}"

grep -Fq -- '--block-size "${BLOCK_SIZE}"' "${CORRECTNESS_RUNNER}"
grep -Fq "SELECT value FROM fod.config WHERE key = 'block_size'" "${CORRECTNESS_RUNNER}"
grep -Fq 'down -v --remove-orphans' "${CORRECTNESS_RUNNER}"
grep -Fq 'FOD_FOPEN_DIRECT_IO=1' "${CORRECTNESS_RUNNER}"

correctness_line="$(grep -n 'bash "${CORRECTNESS_RUNNER}"' "${CANDIDATE_RUNNER}" | head -n 1 | cut -d: -f1)"
performance_line="$(grep -n 'bash "${PERFORMANCE_RUNNER}"' "${CANDIDATE_RUNNER}" | head -n 1 | cut -d: -f1)"
[[ -n "${correctness_line}" && -n "${performance_line}" ]]
(( correctness_line < performance_line ))

grep -Fq 'Correctness gate failed' "${CANDIDATE_RUNNER}"
grep -Fq 'Correctness gate passed' "${CANDIDATE_RUNNER}"
grep -Fq 'FOD_STORAGE_BLOCK_SIZES="${CANDIDATE_SIZES}"' "${CANDIDATE_RUNNER}"
grep -Fq 'FOD_STORAGE_BLOCK_PAYLOAD_MODE="${PAYLOAD_MODE}"' "${CANDIDATE_RUNNER}"

if grep -Eq '(^|[[:space:]])(sed[[:space:]]+-i|perl[[:space:]]+-pi).*fod_config\.ini' \
    "${CORRECTNESS_RUNNER}" "${CANDIDATE_RUNNER}"; then
    echo "Storage block validation must not rewrite fod_config.ini" >&2
    exit 1
fi

if grep -Fq '.github/workflows' "${CORRECTNESS_RUNNER}" "${CANDIDATE_RUNNER}" "${CORRECTNESS_TEST}"; then
    echo "Storage block validation must not modify GitHub Actions" >&2
    exit 1
fi

echo "Storage block candidate validation policy: OK"
