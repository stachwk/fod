#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Compare uncached and userspace-cache read paths against a physical replica.
# The primary remains online as the authority for leases and namespace state.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SINGLE="${ROOT}/tests/integration/test_fio_primary_write_replica_read_docker.sh"
# 4 KiB creates a very large FUSE callback count with direct I/O. Keep it an
# explicit stress case instead of making every default matrix take hours.
BLOCK_SIZES="${FIO_BLOCK_SIZES:-64k 512k}"
FILE_SIZE="${FIO_FILE_SIZE:-128M}"
READ_LOOPS="${FIO_READ_LOOPS:-3}"
REPETITIONS="${FIO_REPETITIONS:-5}"
REPEAT_START="${FIO_REPEAT_START:-1}"
CACHE_BLOCKS="${FOD_CACHE_ON_BLOCKS:-4096}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-read-cache-matrix-${RUN_ID}"
SUMMARY="${ARTIFACT_DIR}/summary.tsv"

for value in "${READ_LOOPS}" "${REPETITIONS}" "${REPEAT_START}" "${CACHE_BLOCKS}"; do
    if [[ ! "${value}" =~ ^[0-9]+$ ]]; then
        echo "FIO_READ_LOOPS, FIO_REPETITIONS, FIO_REPEAT_START and FOD_CACHE_ON_BLOCKS must be non-negative integers" >&2
        exit 2
    fi
done
if (( READ_LOOPS < 1 || REPETITIONS < 1 || REPEAT_START < 1 || CACHE_BLOCKS < 1 )); then
    echo "FIO_READ_LOOPS, FIO_REPETITIONS, FIO_REPEAT_START and FOD_CACHE_ON_BLOCKS must be positive" >&2
    exit 2
fi
if (( REPEAT_START > REPETITIONS )); then
    echo "FIO_REPEAT_START must be less than or equal to FIO_REPETITIONS" >&2
    exit 2
fi

mkdir -p "${ARTIFACT_DIR}"
SUMMARY="${FIO_MATRIX_APPEND_TO:-${SUMMARY}}"
mkdir -p "$(dirname "${SUMMARY}")"
if [[ ! -s "${SUMMARY}" ]]; then
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "block_size" "repeat" "cache_state" "cache_blocks" "file_size" \
        "read_loops" "primary_read_mib_s" "replica_read_mib_s" "replica_reads" \
        "primary_read_fallbacks" "replica_lag_fallbacks" \
        "replica_operation_failures" "replica_write_guard" "sha256_primary" \
        "sha256_replica" >"${SUMMARY}"
fi

for block_size in ${BLOCK_SIZES}; do
    safe_block_size="${block_size//[^A-Za-z0-9_.-]/_}"
    for ((repeat = REPEAT_START; repeat <= REPETITIONS; repeat++)); do
        if (( repeat % 2 == 1 )); then
            states=(off on)
        else
            states=(on off)
        fi
        for cache_state in "${states[@]}"; do
            if [[ "${cache_state}" == off ]]; then
                cache_value=0
                policy_label=cache-off
            else
                cache_value="${CACHE_BLOCKS}"
                policy_label="cache-on-${CACHE_BLOCKS}"
            fi

            label="${HOST_NAME}-read-cache-${cache_state}-${safe_block_size}-r${repeat}"
            log="${ARTIFACT_DIR}/${safe_block_size}-${cache_state}-r${repeat}.log"
            echo "=== block_size=${block_size} repeat=${repeat} cache=${cache_state} blocks=${cache_value} ==="
            if ! REPLICA_READ_MODE=routed \
                REPLICA_READ_LABEL="${label}" \
                REPLICA_READ_POLICY_LABEL="${policy_label}" \
                REPLICA_READ_PRIMARY_PORT="${REPLICA_READ_PRIMARY_PORT:-55451}" \
                REPLICA_READ_REPLICA_PORT="${REPLICA_READ_REPLICA_PORT:-55452}" \
                REPLICA_WAIT_SECONDS="${REPLICA_WAIT_SECONDS:-60}" \
                FOD_RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-release-lto}" \
                FIO_FILE_SIZE="${FILE_SIZE}" \
                FIO_BLOCK_SIZE="${block_size}" \
                FIO_READ_LOOPS="${READ_LOOPS}" \
                FOD_READ_CACHE_BLOCKS="${cache_value}" \
                FOD_READ_AHEAD_BLOCKS=0 \
                FOD_SEQUENTIAL_READ_AHEAD_BLOCKS=0 \
                FOD_DIRECT_IO_READ_PREFETCH_BLOCKS=0 \
                FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS=0 \
                FOD_METADATA_CACHE_TTL_SECONDS=0 \
                FOD_FOPEN_DIRECT_IO=1 \
                bash "${SINGLE}" >"${log}" 2>&1; then
                echo "Benchmark failed: ${log}" >&2
                tail -n 100 "${log}" >&2
                exit 1
            fi

            result="$(grep '^PERF_RESULT ' "${log}" | tail -n 1)"
            if [[ -z "${result}" ]]; then
                echo "Missing PERF_RESULT: ${log}" >&2
                exit 1
            fi
            python3 - "${SUMMARY}" "${result}" "${block_size}" "${repeat}" "${cache_state}" "${cache_value}" "${FILE_SIZE}" "${READ_LOOPS}" "${log}" <<'PY'
import sys

summary, result, block_size, repeat, cache_state, cache_blocks, file_size, loops, log = sys.argv[1:]
fields = {}
for item in result.split()[1:]:
    key, separator, value = item.partition("=")
    if separator:
        fields[key] = value
log_text = open(log, "r", encoding="utf-8", errors="replace").read()
def digest(prefix):
    for line in log_text.splitlines():
        if line.startswith(prefix + "_sha256="):
            return line.split("=", 1)[1]
    return "missing"
row = [
    block_size,
    repeat,
    cache_state,
    cache_blocks,
    file_size,
    loops,
    fields.get("primary_read_mib_s", "missing"),
    fields.get("replica_read_mib_s", "missing"),
    fields.get("replica_reads", "missing"),
    fields.get("primary_read_fallbacks", "missing"),
    fields.get("replica_lag_fallbacks", "missing"),
    fields.get("replica_operation_failures", "missing"),
    fields.get("replica_write_guard", "missing"),
    digest("primary_read"),
    digest("replica_read"),
]
with open(summary, "a", encoding="utf-8") as handle:
    handle.write("\t".join(row) + "\n")
print(
    f"RESULT block={block_size} repeat={repeat} cache={cache_state} "
    f"primary={row[6]}MiB/s replica={row[7]}MiB/s "
    f"replica_reads={row[8]} primary_fallbacks={row[9]} "
    f"replica_lag_fallbacks={row[10]}"
)
PY
        done
    done
done

echo "=== CACHE MATRIX SUMMARY ==="
cat "${SUMMARY}"
echo "matrix_artifact_dir=${ARTIFACT_DIR}"
echo "matrix_summary_file=${SUMMARY}"
echo "OK: repeated FOD read-cache comparison on primary and routed replica reads"
