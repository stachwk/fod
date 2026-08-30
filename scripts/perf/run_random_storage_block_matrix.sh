#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Controlled storage-block-size experiment for the isolated primary/replica lab.
# The default experiment keeps fio at 512 KiB and a 1 GiB random payload while
# varying only the FOD logical storage block size: 4 KiB, 16 KiB, and 64 KiB.
# Each variant gets a fresh Compose project/database and its own PostgreSQL
# primary-write profile.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

STORAGE_BLOCK_SIZES="${FOD_STORAGE_BLOCK_SIZES:-4096 16384 65536}"
FILE_SIZE="${FOD_STORAGE_BLOCK_FILE_SIZE:-1G}"
FIO_BLOCK_SIZE="${FOD_STORAGE_BLOCK_FIO_BLOCK_SIZE:-512k}"
PAYLOAD_MODE="${FOD_STORAGE_BLOCK_PAYLOAD_MODE:-random}"
RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}"
CARGO_PROFILE="${FOD_CARGO_PROFILE:-${RUNTIME_PROFILE}}"
SINGLE="${ROOT}/tests/integration/test_fio_primary_write_replica_read_docker.sh"
PROFILER="${ROOT}/scripts/perf/profile_primary_write_postgres.sh"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${FOD_STORAGE_BLOCK_ARTIFACT_DIR:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-random-storage-block-matrix-${RUN_ID}}"

for cmd in bash make git awk grep sed chmod mktemp; do
    command -v "${cmd}" >/dev/null 2>&1 || {
        echo "Missing required command: ${cmd}" >&2
        exit 2
    }
done

if [[ "${PAYLOAD_MODE}" != "random" ]]; then
    echo "WARN: this experiment is intended for random payloads; payload_mode=${PAYLOAD_MODE}" >&2
fi

for storage_block_size in ${STORAGE_BLOCK_SIZES}; do
    case "${storage_block_size}" in
        ''|*[!0-9]*)
            echo "Invalid storage block size: ${storage_block_size}" >&2
            exit 2
            ;;
    esac
    if (( storage_block_size < 1024 || storage_block_size % 1024 != 0 )); then
        echo "Storage block size must be a positive multiple of 1024: ${storage_block_size}" >&2
        exit 2
    fi
done

mkdir -p "${ARTIFACT_DIR}"
SUMMARY="${ARTIFACT_DIR}/summary.tsv"
printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "storage_block_size" "fio_block_size" "file_size" "payload_mode" \
    "primary_write_mib_s" "primary_read_mib_s" "replica_read_mib_s" \
    "copy_calls" "copy_rows" "copy_exec_ms" "copy_local_blks_written" \
    "insert_calls" "insert_rows" "insert_exec_ms" "insert_shared_blks_written" \
    "insert_wal_bytes" "profile_artifact_dir" >"${SUMMARY}"

printf '=== FOD RANDOM STORAGE BLOCK MATRIX ===\n'
printf 'storage_block_sizes=%s\n' "${STORAGE_BLOCK_SIZES}"
printf 'fio_block_size=%s\nfile_size=%s\npayload_mode=%s\n' \
    "${FIO_BLOCK_SIZE}" "${FILE_SIZE}" "${PAYLOAD_MODE}"
printf 'artifact_dir=%s\n' "${ARTIFACT_DIR}"

# Build once. The individual benchmark runs reuse these binaries while creating
# fresh PostgreSQL primary/replica volumes for every storage block size.
FOD_CARGO_PROFILE="${CARGO_PROFILE}" \
FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" \
make --no-print-directory build-runtime

artifact_profile="${RUNTIME_PROFILE}"
if [[ "${artifact_profile}" == "dev" ]]; then
    artifact_profile="debug"
fi

target_root="${CARGO_TARGET_DIR:-${ROOT}/target}"
if [[ "${target_root}" != /* ]]; then
    target_root="${ROOT}/${target_root}"
fi
REAL_MKFS="${FOD_MKFS_REAL_BIN:-${target_root}/${artifact_profile}/fod-rust-mkfs}"
if [[ ! -x "${REAL_MKFS}" ]]; then
    echo "FOD mkfs binary not found or not executable: ${REAL_MKFS}" >&2
    exit 1
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/fod-storage-block-matrix.XXXXXX")"
MKFS_WRAPPER="${TMP_DIR}/fod-rust-mkfs-block-size"
PROFILE_PID=""

cleanup() {
    local rc=$?
    set +e
    if [[ -n "${PROFILE_PID:-}" ]] && kill -0 "${PROFILE_PID}" >/dev/null 2>&1; then
        kill "${PROFILE_PID}" >/dev/null 2>&1 || true
        wait "${PROFILE_PID}" >/dev/null 2>&1 || true
    fi
    rm -rf "${TMP_DIR}"
    exit "${rc}"
}
trap cleanup EXIT INT TERM

cat >"${MKFS_WRAPPER}" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail

REAL="${FOD_MKFS_REAL_BIN:?FOD_MKFS_REAL_BIN is required}"
BLOCK_SIZE="${FOD_TEST_STORAGE_BLOCK_SIZE:?FOD_TEST_STORAGE_BLOCK_SIZE is required}"

if [[ "${1:-}" == "init" ]]; then
    for arg in "$@"; do
        case "${arg}" in
            --block-size|--block-size=*)
                exec "${REAL}" "$@"
                ;;
        esac
    done
    exec "${REAL}" "$@" --block-size "${BLOCK_SIZE}"
fi

exec "${REAL}" "$@"
WRAPPER
chmod 0700 "${MKFS_WRAPPER}"

field() {
    local line="$1"
    local key="$2"
    printf '%s\n' "${line}" \
        | tr ' ' '\n' \
        | sed -n "s/^${key}=//p" \
        | tail -n 1
}

statement_last() {
    local statements_file="$1"
    local kind="$2"
    awk -F '\t' -v expected="${kind}" '$2 == expected {line=$0} END {print line}' "${statements_file}"
}

statement_field() {
    local line="$1"
    local column="$2"
    if [[ -z "${line}" ]]; then
        printf '0\n'
        return
    fi
    printf '%s\n' "${line}" | awk -F '\t' -v column="${column}" '{print $column}'
}

for storage_block_size in ${STORAGE_BLOCK_SIZES}; do
    variant="block-${storage_block_size}"
    variant_dir="${ARTIFACT_DIR}/${variant}"
    benchmark_log="${variant_dir}/benchmark.log"
    profile_log="${variant_dir}/postgres-profile.log"
    profile_dir="${variant_dir}/postgres"
    mkdir -p "${variant_dir}"

    printf '\n=== STORAGE BLOCK SIZE %s ===\n' "${storage_block_size}"

    FOD_PG_WRITE_PROFILE_OUT="${profile_dir}" \
        bash "${PROFILER}" >"${profile_log}" 2>&1 &
    PROFILE_PID=$!

    set +e
    FOD_CARGO_PROFILE="${CARGO_PROFILE}" \
    FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" \
    FOD_MKFS_BIN="${MKFS_WRAPPER}" \
    FOD_MKFS_REAL_BIN="${REAL_MKFS}" \
    FOD_TEST_STORAGE_BLOCK_SIZE="${storage_block_size}" \
    FIO_FILE_SIZE="${FILE_SIZE}" \
    FIO_BLOCK_SIZE="${FIO_BLOCK_SIZE}" \
    FIO_PAYLOAD_MODE="${PAYLOAD_MODE}" \
    REPLICA_READ_LABEL="storage-block-${storage_block_size}" \
    FOD_REQUIRE_AC_POWER="${FOD_REQUIRE_AC_POWER:-1}" \
        bash "${SINGLE}" 2>&1 | tee "${benchmark_log}"
    benchmark_status=${PIPESTATUS[0]}
    set -e

    if [[ "${benchmark_status}" -ne 0 ]]; then
        if kill -0 "${PROFILE_PID}" >/dev/null 2>&1; then
            kill "${PROFILE_PID}" >/dev/null 2>&1 || true
        fi
        wait "${PROFILE_PID}" >/dev/null 2>&1 || true
        PROFILE_PID=""
        echo "Benchmark failed for storage_block_size=${storage_block_size}; log=${benchmark_log}" >&2
        exit "${benchmark_status}"
    fi

    if ! wait "${PROFILE_PID}"; then
        PROFILE_PID=""
        echo "PostgreSQL profiler failed for storage_block_size=${storage_block_size}; log=${profile_log}" >&2
        exit 1
    fi
    PROFILE_PID=""

    perf_result="$(grep '^PERF_RESULT ' "${benchmark_log}" | tail -n 1)"
    if [[ -z "${perf_result}" ]]; then
        echo "Missing PERF_RESULT for storage_block_size=${storage_block_size}" >&2
        exit 1
    fi

    statements_file="${profile_dir}/statements.tsv"
    if [[ ! -s "${statements_file}" ]]; then
        echo "Missing PostgreSQL statements profile: ${statements_file}" >&2
        exit 1
    fi

    copy_line="$(statement_last "${statements_file}" copy_stage)"
    insert_line="$(statement_last "${statements_file}" insert_on_conflict)"

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "${storage_block_size}" "${FIO_BLOCK_SIZE}" "${FILE_SIZE}" "${PAYLOAD_MODE}" \
        "$(field "${perf_result}" primary_write_mib_s)" \
        "$(field "${perf_result}" primary_read_mib_s)" \
        "$(field "${perf_result}" replica_read_mib_s)" \
        "$(statement_field "${copy_line}" 3)" \
        "$(statement_field "${copy_line}" 6)" \
        "$(statement_field "${copy_line}" 4)" \
        "$(statement_field "${copy_line}" 14)" \
        "$(statement_field "${insert_line}" 3)" \
        "$(statement_field "${insert_line}" 6)" \
        "$(statement_field "${insert_line}" 4)" \
        "$(statement_field "${insert_line}" 10)" \
        "$(statement_field "${insert_line}" 23)" \
        "${profile_dir}" >>"${SUMMARY}"

    echo "--- PostgreSQL profile summary (${storage_block_size}) ---"
    cat "${profile_log}"
done

echo
echo "=== RANDOM STORAGE BLOCK MATRIX RESULT ==="
cat "${SUMMARY}"
echo "matrix_artifact_dir=${ARTIFACT_DIR}"
echo "OK: random storage block size matrix"
