#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Run the isolated primary/replica benchmark for a matrix of block sizes and
# write-payload modes.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BLOCK_SIZES="${FIO_BLOCK_SIZES:-4k 16k 64k 256k 512k 1m}"
PAYLOAD_MODES="${FIO_PAYLOAD_MODES:-pattern}"
FILE_SIZE="${FIO_FILE_SIZE:-256M}"
LABEL="${REPLICA_READ_LABEL:-matrix}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-${LABEL}-matrix-${RUN_ID}"
SINGLE="${ROOT}/tests/integration/test_fio_primary_write_replica_read_docker.sh"

mkdir -p "${ARTIFACT_DIR}"
SUMMARY="${ARTIFACT_DIR}/summary.tsv"
printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "block_size" "file_size" "payload_mode" \
    "primary_write_mib_s" "primary_write_iops" \
    "primary_read_mib_s" "primary_read_iops" \
    "replica_read_mib_s" "replica_read_iops" \
    "replica_operation_failures" "replica_write_guard" \
    >"${SUMMARY}"

field() {
    local line="$1"
    local key="$2"
    printf '%s\n' "${line}" \
        | tr ' ' '\n' \
        | sed -n "s/^${key}=//p" \
        | tail -n 1
}

for block_size in ${BLOCK_SIZES}; do
    safe_block_size="${block_size//[^A-Za-z0-9_.-]/_}"
    for payload_mode in ${PAYLOAD_MODES}; do
        safe_payload_mode="${payload_mode//[^A-Za-z0-9_.-]/_}"
        log="${ARTIFACT_DIR}/${safe_block_size}-${safe_payload_mode}.log"

        echo "=== MATRIX block_size=${block_size} file_size=${FILE_SIZE} payload_mode=${payload_mode} ==="
        set +e
        FIO_FILE_SIZE="${FILE_SIZE}" \
        FIO_BLOCK_SIZE="${block_size}" \
        FIO_PAYLOAD_MODE="${payload_mode}" \
        REPLICA_READ_LABEL="${LABEL}-${safe_block_size}-${safe_payload_mode}" \
        bash "${SINGLE}" 2>&1 | tee "${log}"
        status=${PIPESTATUS[0]}
        set -e
        if [[ "${status}" -ne 0 ]]; then
            echo "Benchmark failed for block_size=${block_size} payload_mode=${payload_mode}; log=${log}" >&2
            exit "${status}"
        fi

        result="$(grep '^PERF_RESULT ' "${log}" | tail -n 1)"
        [[ -n "${result}" ]] || {
            echo "Missing PERF_RESULT for block_size=${block_size} payload_mode=${payload_mode}" >&2
            exit 1
        }

        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "$(field "${result}" block_size)" \
            "$(field "${result}" file_size)" \
            "$(field "${result}" payload_mode)" \
            "$(field "${result}" primary_write_mib_s)" \
            "$(field "${result}" primary_write_iops)" \
            "$(field "${result}" primary_read_mib_s)" \
            "$(field "${result}" primary_read_iops)" \
            "$(field "${result}" replica_read_mib_s)" \
            "$(field "${result}" replica_read_iops)" \
            "$(field "${result}" replica_operation_failures)" \
            "$(field "${result}" replica_write_guard)" \
            >>"${SUMMARY}"
    done
done

echo "=== PRIMARY/REPLICA MATRIX RESULT ==="
cat "${SUMMARY}"
echo "matrix_artifact_dir=${ARTIFACT_DIR}"
echo "OK: primary/replica performance matrix"
