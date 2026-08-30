#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Gate storage-block performance refinement on correctness. The correctness
# matrix must pass before the 8K/16K/32K/64K random-payload benchmark starts.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

CORRECTNESS_SIZES="${FOD_STORAGE_CORRECTNESS_BLOCK_SIZES:-4096 16384 65536}"
CANDIDATE_SIZES="${FOD_STORAGE_CANDIDATE_BLOCK_SIZES:-8192 16384 32768 65536}"
FILE_SIZE="${FOD_STORAGE_CANDIDATE_FILE_SIZE:-1G}"
FIO_BLOCK_SIZE="${FOD_STORAGE_CANDIDATE_FIO_BLOCK_SIZE:-512k}"
PAYLOAD_MODE="${FOD_STORAGE_CANDIDATE_PAYLOAD_MODE:-random}"
RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}"
CARGO_PROFILE="${FOD_CARGO_PROFILE:-${RUNTIME_PROFILE}}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${FOD_STORAGE_CANDIDATE_ARTIFACT_DIR:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-storage-block-candidate-validation-${RUN_ID}}"
CORRECTNESS_DIR="${ARTIFACT_DIR}/correctness"
PERFORMANCE_DIR="${ARTIFACT_DIR}/performance"
CORRECTNESS_RUNNER="${ROOT}/scripts/perf/run_storage_block_correctness_matrix.sh"
PERFORMANCE_RUNNER="${ROOT}/scripts/perf/run_random_storage_block_matrix.sh"

for cmd in bash git mkdir tee; do
    command -v "${cmd}" >/dev/null 2>&1 || {
        echo "Missing required command: ${cmd}" >&2
        exit 2
    }
done

if [[ "${PAYLOAD_MODE}" != "random" ]]; then
    echo "Refusing candidate validation: payload mode must remain random, got '${PAYLOAD_MODE}'" >&2
    exit 2
fi

mkdir -p "${ARTIFACT_DIR}"

printf '=== FOD STORAGE BLOCK CANDIDATE VALIDATION ===\n'
printf 'correctness_sizes=%s\n' "${CORRECTNESS_SIZES}"
printf 'candidate_sizes=%s\n' "${CANDIDATE_SIZES}"
printf 'file_size=%s\nfio_block_size=%s\npayload_mode=%s\n' \
    "${FILE_SIZE}" "${FIO_BLOCK_SIZE}" "${PAYLOAD_MODE}"
printf 'artifact_dir=%s\n' "${ARTIFACT_DIR}"

printf '\n=== PHASE 1: CORRECTNESS GATE ===\n'
FOD_CARGO_PROFILE="${CARGO_PROFILE}" \
FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" \
FOD_STORAGE_CORRECTNESS_BLOCK_SIZES="${CORRECTNESS_SIZES}" \
FOD_STORAGE_CORRECTNESS_ARTIFACT_DIR="${CORRECTNESS_DIR}" \
    bash "${CORRECTNESS_RUNNER}" 2>&1 | tee "${ARTIFACT_DIR}/correctness.log"

SUMMARY="${CORRECTNESS_DIR}/summary.tsv"
if [[ ! -s "${SUMMARY}" ]]; then
    echo "Correctness summary is missing: ${SUMMARY}" >&2
    exit 1
fi

expected_count="$(printf '%s\n' ${CORRECTNESS_SIZES} | awk 'NF {count++} END {print count+0}')"
pass_count="$(awk -F '\t' 'NR > 1 && $2 == "PASS" {count++} END {print count+0}' "${SUMMARY}")"
row_count="$(awk 'NR > 1 && NF {count++} END {print count+0}' "${SUMMARY}")"

if [[ "${row_count}" -ne "${expected_count}" || "${pass_count}" -ne "${expected_count}" ]]; then
    echo "Correctness gate failed: expected=${expected_count} rows=${row_count} pass=${pass_count}" >&2
    cat "${SUMMARY}" >&2
    exit 1
fi

echo "Correctness gate passed for all ${expected_count} storage block sizes."

printf '\n=== PHASE 2: RANDOM PAYLOAD PERFORMANCE REFINEMENT ===\n'
FOD_CARGO_PROFILE="${CARGO_PROFILE}" \
FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" \
FOD_STORAGE_BLOCK_SIZES="${CANDIDATE_SIZES}" \
FOD_STORAGE_BLOCK_FILE_SIZE="${FILE_SIZE}" \
FOD_STORAGE_BLOCK_FIO_BLOCK_SIZE="${FIO_BLOCK_SIZE}" \
FOD_STORAGE_BLOCK_PAYLOAD_MODE="${PAYLOAD_MODE}" \
FOD_STORAGE_BLOCK_ARTIFACT_DIR="${PERFORMANCE_DIR}" \
FOD_REQUIRE_AC_POWER="${FOD_REQUIRE_AC_POWER:-1}" \
    bash "${PERFORMANCE_RUNNER}" 2>&1 | tee "${ARTIFACT_DIR}/performance.log"

PERFORMANCE_SUMMARY="${PERFORMANCE_DIR}/summary.tsv"
if [[ ! -s "${PERFORMANCE_SUMMARY}" ]]; then
    echo "Performance summary is missing: ${PERFORMANCE_SUMMARY}" >&2
    exit 1
fi

printf '\n=== STORAGE BLOCK CANDIDATE VALIDATION RESULT ===\n'
printf '%s\n' '--- correctness ---'
cat "${SUMMARY}"
printf '%s\n' '--- performance ---'
cat "${PERFORMANCE_SUMMARY}"
echo "validation_artifact_dir=${ARTIFACT_DIR}"
echo "OK: storage block candidate validation"
