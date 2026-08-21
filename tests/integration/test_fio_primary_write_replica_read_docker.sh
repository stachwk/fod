#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Docker-only diagnostic:
# write primary -> unmount FOD -> wait WAL replay -> stop primary ->
# restart replica -> fresh replica-only FOD mount -> read.
#
# Host kernel page cache is not dropped because that is a host-global
# privileged operation.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

COMPOSE_FILE="${ROOT}/${REPLICA_READ_COMPOSE_FILE:-docker-compose.replica-read.yml}"
PRIMARY_PORT="${REPLICA_READ_PRIMARY_PORT:-55441}"
REPLICA_PORT="${REPLICA_READ_REPLICA_PORT:-55442}"
FILE_SIZE="${FIO_FILE_SIZE:-1G}"
BLOCK_SIZE="${FIO_BLOCK_SIZE:-4k}"
WAIT_SECONDS="${REPLICA_WAIT_SECONDS:-120}"
PROJECT="fod-replica-read-${BASHPID}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST}-docker-primary-write-replica-read-${RUN_ID}"

read -r -a COMPOSE_CMD <<<"${FOD_REPLICA_READ_COMPOSE:-docker compose}"

for cmd in docker psql fio mountpoint; do
    command -v "$cmd" >/dev/null 2>&1 || {
        echo "Missing required command: $cmd" >&2
        exit 2
    }
done

mkdir -p "${ARTIFACT_DIR}"

compose() {
    COMPOSE_PROJECT_NAME="${PROJECT}" \
    POSTGRES_DB="${POSTGRES_DB:-foddbname}" \
    POSTGRES_USER="${POSTGRES_USER:-foduser}" \
    POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-cichosza}" \
    REPLICA_READ_PRIMARY_PORT="${PRIMARY_PORT}" \
    REPLICA_READ_REPLICA_PORT="${REPLICA_PORT}" \
    "${COMPOSE_CMD[@]}" -f "${COMPOSE_FILE}" "$@"
}

psql_primary() {
    PGPASSWORD="${POSTGRES_PASSWORD:-cichosza}" \
    psql -h 127.0.0.1 -p "${PRIMARY_PORT}" \
        -U "${POSTGRES_USER:-foduser}" \
        -d "${POSTGRES_DB:-foddbname}" -Atq "$@"
}

psql_replica() {
    PGPASSWORD="${POSTGRES_PASSWORD:-cichosza}" \
    psql -h 127.0.0.1 -p "${REPLICA_PORT}" \
        -U "${POSTGRES_USER:-foduser}" \
        -d "${POSTGRES_DB:-foddbname}" -Atq "$@"
}

MOUNTPOINT=""
LOG_FILE=""
FOD_PID=""
STACK_STARTED=0

cleanup() {
    local rc=$?
    set +e
    if [[ -n "${MOUNTPOINT:-}" || -n "${FOD_PID:-}" ]]; then
        fod_test_cleanup
    fi
    if (( STACK_STARTED )); then
        compose logs --no-color >"${ARTIFACT_DIR}/docker-compose.log" 2>&1 || true
        compose down -v --remove-orphans >/dev/null 2>&1 || true
    fi
    echo "artifact_dir=${ARTIFACT_DIR}"
    exit "${rc}"
}
trap cleanup EXIT INT TERM

wait_for_role() {
    local label="$1"
    local expected="$2"
    local which="$3"

    for ((second = 0; second < WAIT_SECONDS; second++)); do
        local value
        if [[ "${which}" == "primary" ]]; then
            value="$(psql_primary -c \
                "SELECT pg_is_in_recovery()::text || '|' || current_setting('transaction_read_only')" \
                2>/dev/null || true)"
        else
            value="$(psql_replica -c \
                "SELECT pg_is_in_recovery()::text || '|' || current_setting('transaction_read_only')" \
                2>/dev/null || true)"
        fi
        value="$(printf '%s' "${value}" | tr -d '[:space:]')"
        if [[ "${value}" == "${expected}" ]]; then
            echo "${label} role=${value} ready_after=${second}s"
            return 0
        fi
        sleep 1
    done

    echo "${label} did not reach role ${expected}" >&2
    return 1
}

echo "=== Docker primary-write / replica-read diagnostic ==="
echo "primary=127.0.0.1:${PRIMARY_PORT}"
echo "replica=127.0.0.1:${REPLICA_PORT}"
echo "size=${FILE_SIZE} block_size=${BLOCK_SIZE}"
echo "artifact_dir=${ARTIFACT_DIR}"
fod_test_write_power_metadata "${ARTIFACT_DIR}/power-before.txt" "before"
fod_test_log_power_metadata "before"

compose down -v --remove-orphans >/dev/null 2>&1 || true
compose up -d primary replica
STACK_STARTED=1

wait_for_role primary "false|off" primary
wait_for_role replica "true|on" replica

export FOD_PG_HOST=127.0.0.1
export FOD_PG_PORT="${PRIMARY_PORT}"
export FOD_PG_PRIMARY_HOSTS="127.0.0.1:${PRIMARY_PORT}"
export FOD_PG_REPLICA_HOSTS="127.0.0.1:${REPLICA_PORT}"
export FOD_PG_ENDPOINT_ROUTING_ENABLED=1
export FOD_PG_RUNTIME_FAILOVER_ENABLED=0
export FOD_PG_REPLICA_READ_ROUTING_ENABLED=0

export FOD_READ_CACHE_BLOCKS=0
export FOD_READ_AHEAD_BLOCKS=0
export FOD_SEQUENTIAL_READ_AHEAD_BLOCKS=0
export FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS=0
export FOD_METADATA_CACHE_TTL_SECONDS=0
export FOD_STATFS_CACHE_TTL_SECONDS=0
export FOD_FOPEN_DIRECT_IO=1
export FOD_FUSE_WRITEBACK_CACHE=0
export FOD_PROFILE_IO=1
export FOD_LOG_LEVEL=info

fod_test_setup "${ROOT}"
fod_test_init_schema

TEST_BASENAME="fio-primary-replica-${RUN_ID}.bin"

echo "=== PHASE 1: WRITE PRIMARY ==="
FOD_ROLE=primary
fod_test_make_mountpoint "/tmp/fod-docker-primary-write"
FOD_TEST_LOG_ARCHIVE="${ARTIFACT_DIR}/primary-write-mount.log"
fod_test_start_mount "${MOUNTPOINT}"

WRITE_FILE="${MOUNTPOINT}/${TEST_BASENAME}"

fio \
    --name=primary-write \
    --filename="${WRITE_FILE}" \
    --ioengine=sync \
    --rw=write \
    --bs="${BLOCK_SIZE}" \
    --size="${FILE_SIZE}" \
    --numjobs=1 \
    --group_reporting=1 \
    --direct=0 \
    --fsync_on_close=1 \
    --buffer_pattern=0x5a \
    --output-format=normal \
    | tee "${ARTIFACT_DIR}/primary-write-fio.txt"

EXPECTED_SIZE="$(stat -c '%s' "${WRITE_FILE}")"
[[ -n "${EXPECTED_SIZE}" && "${EXPECTED_SIZE}" != "0" ]] || {
    echo "Written file size is invalid: ${EXPECTED_SIZE}" >&2
    exit 1
}

PRIMARY_LSN="$(psql_primary -c "SELECT pg_current_wal_flush_lsn()::text")"
PRIMARY_LSN="$(printf '%s' "${PRIMARY_LSN}" | tr -d '[:space:]')"
[[ -n "${PRIMARY_LSN}" ]] || {
    echo "Primary WAL flush LSN is empty" >&2
    exit 1
}
echo "primary_flush_lsn=${PRIMARY_LSN}" \
    | tee "${ARTIFACT_DIR}/replication.txt"

fod_test_cleanup
MOUNTPOINT=""
LOG_FILE=""
FOD_PID=""

echo "=== PHASE 2: WAIT REPLICA ==="
caught_up=0
for ((second = 0; second < WAIT_SECONDS; second++)); do
    replay="$(psql_replica -c \
        "SELECT CASE WHEN pg_last_wal_replay_lsn() >= '${PRIMARY_LSN}'::pg_lsn THEN '1' ELSE '0' END" \
        2>/dev/null || true)"
    replay="$(printf '%s' "${replay}" | tr -d '[:space:]')"
    if [[ "${replay}" == "1" ]]; then
        caught_up=1
        echo "replica_caught_up_after_seconds=${second}" \
            | tee -a "${ARTIFACT_DIR}/replication.txt"
        break
    fi
    sleep 1
done

if [[ "${caught_up}" != "1" ]]; then
    echo "Replica did not replay ${PRIMARY_LSN}" >&2
    exit 1
fi

REPLICA_LSN="$(psql_replica -c \
    "SELECT COALESCE(pg_last_wal_replay_lsn()::text, '')")"
REPLICA_LSN="$(printf '%s' "${REPLICA_LSN}" | tr -d '[:space:]')"
echo "replica_replay_lsn=${REPLICA_LSN}" \
    | tee -a "${ARTIFACT_DIR}/replication.txt"

echo "=== PHASE 3: STOP PRIMARY / RESTART REPLICA ==="
compose stop primary
if psql_primary -c "SELECT 1" >/dev/null 2>&1; then
    echo "Primary is still reachable" >&2
    exit 1
fi
echo "primary_reachable_before_read=0" \
    | tee -a "${ARTIFACT_DIR}/replication.txt"

compose restart replica
wait_for_role "replica-after-restart" "true|on" replica

echo "=== PHASE 4: READ REPLICA ==="
FOD_ROLE=replica
fod_test_make_mountpoint "/tmp/fod-docker-replica-read"
FOD_TEST_LOG_ARCHIVE="${ARTIFACT_DIR}/replica-read-mount.log"
fod_test_start_mount "${MOUNTPOINT}"

READ_FILE="${MOUNTPOINT}/${TEST_BASENAME}"
[[ -f "${READ_FILE}" ]] || {
    echo "Replica does not expose ${TEST_BASENAME}" >&2
    exit 1
}

ACTUAL_SIZE="$(stat -c '%s' "${READ_FILE}")"
[[ "${ACTUAL_SIZE}" == "${EXPECTED_SIZE}" ]] || {
    echo "Replica size mismatch expected=${EXPECTED_SIZE} actual=${ACTUAL_SIZE}" >&2
    exit 1
}

fio \
    --name=replica-read \
    --filename="${READ_FILE}" \
    --ioengine=sync \
    --rw=read \
    --bs="${BLOCK_SIZE}" \
    --size="${FILE_SIZE}" \
    --numjobs=1 \
    --group_reporting=1 \
    --direct=0 \
    --output-format=normal \
    | tee "${ARTIFACT_DIR}/replica-read-fio.txt"

fod_test_cleanup
MOUNTPOINT=""
LOG_FILE=""
FOD_PID=""

# Strict read-only regression guard.
if grep -q 'SQLSTATE 25006' "${ARTIFACT_DIR}/replica-read-mount.log"; then
    echo "Replica FOD log contains forbidden read-only DML (SQLSTATE 25006)" >&2
    exit 1
fi
if grep -q 'FOD atime touch failed' "${ARTIFACT_DIR}/replica-read-mount.log"; then
    echo "Replica FOD log contains an atime write attempt" >&2
    exit 1
fi

compose logs --no-color replica \
    >"${ARTIFACT_DIR}/replica-postgres-after-read.log" 2>&1 || true

if grep -Eq 'ERROR:.*cannot execute .* in a read-only transaction' \
    "${ARTIFACT_DIR}/replica-postgres-after-read.log"; then
    echo "PostgreSQL replica observed forbidden write SQL during the test" >&2
    exit 1
fi

FINAL_LANE_LINE="$(
    grep 'FOD PostgreSQL lane observability' \
        "${ARTIFACT_DIR}/replica-read-mount.log" \
        | tail -n 1
)"
FINAL_OPERATION_FAILURES="$(
    printf '%s\n' "${FINAL_LANE_LINE}" \
        | tr ' ' '\n' \
        | sed -n 's/^operation_failures=//p'
)"
FINAL_CONNECTION_CREATES="$(
    printf '%s\n' "${FINAL_LANE_LINE}" \
        | tr ' ' '\n' \
        | sed -n 's/^connection_create_count=//p'
)"

if [[ -z "${FINAL_OPERATION_FAILURES}" ]]; then
    echo "Could not read operation_failures from replica observability" >&2
    exit 1
fi
if [[ "${FINAL_OPERATION_FAILURES}" != "0" ]]; then
    echo "Replica read DB operation failures: ${FINAL_OPERATION_FAILURES}" >&2
    exit 1
fi

echo "replica_operation_failures=${FINAL_OPERATION_FAILURES}" \
    | tee -a "${ARTIFACT_DIR}/replication.txt"
echo "replica_connection_create_count=${FINAL_CONNECTION_CREATES:-unknown}" \
    | tee -a "${ARTIFACT_DIR}/replication.txt"
fod_test_write_power_metadata "${ARTIFACT_DIR}/power-after.txt" "after"

echo "=== RESULT ==="
echo "primary stopped before read: yes"
echo "replica PostgreSQL restarted before read: yes"
echo "FOD read cache/read-ahead disabled: yes"
echo "FOD FUSE direct_io enabled: yes"
echo "strict read-only replica DML guard: passed"
echo "replica DB operation_failures: ${FINAL_OPERATION_FAILURES}"
echo "replica connection_create_count: ${FINAL_CONNECTION_CREATES:-unknown}"
echo "Docker host kernel page cache dropped: no"
grep -E 'READ:|read:' "${ARTIFACT_DIR}/replica-read-fio.txt" | tail -n 4 || true
echo "--- replication ---"
cat "${ARTIFACT_DIR}/replication.txt"
echo "--- power before ---"
cat "${ARTIFACT_DIR}/power-before.txt"
echo "--- power after ---"
cat "${ARTIFACT_DIR}/power-after.txt"
echo "--- replica mount evidence ---"
grep -E \
    'selected_authority|selected_read_only|routing_enabled|read_only|endpoint' \
    "${ARTIFACT_DIR}/replica-read-mount.log" \
    | tail -n 30 || true

echo "OK: Docker primary write -> unmount -> WAL replay -> primary stopped -> replica restart -> replica read"
