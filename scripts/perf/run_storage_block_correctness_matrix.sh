#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Validate data correctness for alternative FOD logical storage block sizes.
# Every variant uses a fresh isolated PostgreSQL primary volume. Production
# configuration and the normal local fod-postgres container are not modified.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

STORAGE_BLOCK_SIZES="${FOD_STORAGE_CORRECTNESS_BLOCK_SIZES:-4096 16384 65536}"
PRIMARY_HOST="${FOD_STORAGE_CORRECTNESS_PRIMARY_HOST:-127.0.0.1}"
PRIMARY_PORT="${FOD_STORAGE_CORRECTNESS_PRIMARY_PORT:-55441}"
BIND_ADDRESS="${FOD_STORAGE_CORRECTNESS_BIND_ADDRESS:-127.0.0.1}"
COMPOSE_FILE="${ROOT}/${FOD_STORAGE_CORRECTNESS_COMPOSE_FILE:-docker-compose.replica-read.yml}"
RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}"
CARGO_PROFILE="${FOD_CARGO_PROFILE:-${RUNTIME_PROFILE}}"
TEST_SCRIPT="${ROOT}/tests/integration/test_storage_block_size_correctness.py"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${FOD_STORAGE_CORRECTNESS_ARTIFACT_DIR:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-storage-block-correctness-${RUN_ID}}"
PROJECT_PREFIX="fod-storage-correctness-${BASHPID}"

read -r -a COMPOSE_CMD <<<"${FOD_STORAGE_CORRECTNESS_COMPOSE:-docker compose}"

for cmd in docker psql python3 make git mktemp chmod; do
    command -v "${cmd}" >/dev/null 2>&1 || {
        echo "Missing required command: ${cmd}" >&2
        exit 2
    }
done

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
printf 'storage_block_size\tstatus\tconfigured_block_size\tlog\n' >"${SUMMARY}"

printf '=== FOD STORAGE BLOCK CORRECTNESS MATRIX ===\n'
printf 'storage_block_sizes=%s\n' "${STORAGE_BLOCK_SIZES}"
printf 'primary=%s:%s\n' "${PRIMARY_HOST}" "${PRIMARY_PORT}"
printf 'artifact_dir=%s\n' "${ARTIFACT_DIR}"

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

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/fod-storage-correctness.XXXXXX")"
MKFS_WRAPPER="${TMP_DIR}/fod-rust-mkfs-block-size"
CURRENT_PROJECT=""

cleanup() {
    local rc=$?
    set +e
    if [[ -n "${CURRENT_PROJECT:-}" ]]; then
        COMPOSE_PROJECT_NAME="${CURRENT_PROJECT}" \
        POSTGRES_DB="${POSTGRES_DB:-foddbname}" \
        POSTGRES_USER="${POSTGRES_USER:-foduser}" \
        POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-cichosza}" \
        REPLICA_READ_BIND_ADDRESS="${BIND_ADDRESS}" \
        REPLICA_READ_PRIMARY_PORT="${PRIMARY_PORT}" \
            "${COMPOSE_CMD[@]}" -f "${COMPOSE_FILE}" down -v --remove-orphans >/dev/null 2>&1 || true
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

compose() {
    COMPOSE_PROJECT_NAME="${CURRENT_PROJECT}" \
    POSTGRES_DB="${POSTGRES_DB:-foddbname}" \
    POSTGRES_USER="${POSTGRES_USER:-foduser}" \
    POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-cichosza}" \
    REPLICA_READ_BIND_ADDRESS="${BIND_ADDRESS}" \
    REPLICA_READ_PRIMARY_PORT="${PRIMARY_PORT}" \
        "${COMPOSE_CMD[@]}" -f "${COMPOSE_FILE}" "$@"
}

wait_primary() {
    for second in $(seq 0 60); do
        if PGPASSWORD="${POSTGRES_PASSWORD:-cichosza}" \
            psql -X -h "${PRIMARY_HOST}" -p "${PRIMARY_PORT}" \
            -U "${POSTGRES_USER:-foduser}" -d "${POSTGRES_DB:-foddbname}" \
            -Atqc "SELECT CASE WHEN NOT pg_is_in_recovery() AND NOT current_setting('transaction_read_only')::boolean THEN 1 ELSE 0 END" \
            2>/dev/null | grep -qx 1; then
            echo "primary ready_after=${second}s"
            return 0
        fi
        sleep 1
    done
    echo "Primary did not become writable on ${PRIMARY_HOST}:${PRIMARY_PORT}" >&2
    return 1
}

for storage_block_size in ${STORAGE_BLOCK_SIZES}; do
    CURRENT_PROJECT="${PROJECT_PREFIX}-${storage_block_size}"
    variant_dir="${ARTIFACT_DIR}/block-${storage_block_size}"
    log_file="${variant_dir}/correctness.log"
    mkdir -p "${variant_dir}"

    printf '\n=== STORAGE BLOCK SIZE %s ===\n' "${storage_block_size}"
    compose down -v --remove-orphans >/dev/null 2>&1 || true
    compose up -d --build primary
    wait_primary

    export FOD_PG_HOST="${PRIMARY_HOST}"
    export FOD_PG_PORT="${PRIMARY_PORT}"
    export FOD_PG_DBNAME="${POSTGRES_DB:-foddbname}"
    export FOD_PG_USER="${POSTGRES_USER:-foduser}"
    export FOD_PG_PASSWORD="${POSTGRES_PASSWORD:-cichosza}"
    export FOD_PG_PRIMARY_HOSTS="${PRIMARY_HOST}:${PRIMARY_PORT}"
    export FOD_PG_REPLICA_HOSTS=""
    export FOD_PG_ENDPOINT_ROUTING_ENABLED=1
    export FOD_PG_RUNTIME_FAILOVER_ENABLED=0
    export FOD_PG_REPLICA_READ_ROUTING_ENABLED=0
    export POSTGRES_HOST="${PRIMARY_HOST}"
    export POSTGRES_PORT="${PRIMARY_PORT}"

    export FOD_READ_CACHE_BLOCKS=0
    export FOD_READ_AHEAD_BLOCKS=0
    export FOD_SEQUENTIAL_READ_AHEAD_BLOCKS=0
    export FOD_DIRECT_IO_READ_PREFETCH_BLOCKS=0
    export FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS=0
    export FOD_METADATA_CACHE_TTL_SECONDS=0
    export FOD_STATFS_CACHE_TTL_SECONDS=0
    export FOD_FOPEN_DIRECT_IO=1
    export FOD_FUSE_WRITEBACK_CACHE=0
    export FOD_ATIME_POLICY=noatime
    export FOD_LOG_LEVEL=info

    set +e
    FOD_CARGO_PROFILE="${CARGO_PROFILE}" \
    FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" \
    FOD_MKFS_BIN="${MKFS_WRAPPER}" \
    FOD_MKFS_REAL_BIN="${REAL_MKFS}" \
    FOD_TEST_STORAGE_BLOCK_SIZE="${storage_block_size}" \
        python3 "${TEST_SCRIPT}" 2>&1 | tee "${log_file}"
    test_status=${PIPESTATUS[0]}
    set -e

    configured_block_size="$(
        PGPASSWORD="${POSTGRES_PASSWORD:-cichosza}" \
        psql -X -h "${PRIMARY_HOST}" -p "${PRIMARY_PORT}" \
            -U "${POSTGRES_USER:-foduser}" -d "${POSTGRES_DB:-foddbname}" \
            -Atqc "SELECT value FROM fod.config WHERE key = 'block_size'" 2>/dev/null \
        | tr -d '[:space:]'
    )"

    if [[ "${test_status}" -ne 0 ]]; then
        printf '%s\tFAIL\t%s\t%s\n' "${storage_block_size}" "${configured_block_size:-unknown}" "${log_file}" >>"${SUMMARY}"
        compose logs --no-color >"${variant_dir}/docker-compose.log" 2>&1 || true
        compose down -v --remove-orphans >/dev/null 2>&1 || true
        CURRENT_PROJECT=""
        echo "Correctness test failed for storage_block_size=${storage_block_size}" >&2
        exit "${test_status}"
    fi

    if [[ "${configured_block_size}" != "${storage_block_size}" ]]; then
        printf '%s\tFAIL\t%s\t%s\n' "${storage_block_size}" "${configured_block_size:-unknown}" "${log_file}" >>"${SUMMARY}"
        compose logs --no-color >"${variant_dir}/docker-compose.log" 2>&1 || true
        compose down -v --remove-orphans >/dev/null 2>&1 || true
        CURRENT_PROJECT=""
        echo "Configured block_size mismatch expected=${storage_block_size} actual=${configured_block_size:-unknown}" >&2
        exit 1
    fi

    printf '%s\tPASS\t%s\t%s\n' "${storage_block_size}" "${configured_block_size}" "${log_file}" >>"${SUMMARY}"
    compose logs --no-color >"${variant_dir}/docker-compose.log" 2>&1 || true
    compose down -v --remove-orphans >/dev/null 2>&1 || true
    CURRENT_PROJECT=""
done

printf '\n=== STORAGE BLOCK CORRECTNESS RESULT ===\n'
cat "${SUMMARY}"
echo "matrix_artifact_dir=${ARTIFACT_DIR}"
echo "OK: storage block correctness matrix"
