#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Isolated primary/replica FOD performance diagnostic.
#
# Sequence:
# primary write -> unmount -> WAL replay ->
# primary restart -> fresh primary read -> unmount -> WAL replay ->
# stop primary -> replica restart -> fresh replica read ->
# verify replica write rejection.
#
# Host kernel page cache is not dropped.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

COMPOSE_FILE="${ROOT}/${REPLICA_READ_COMPOSE_FILE:-docker-compose.replica-read.yml}"
BIND_ADDRESS="${REPLICA_READ_BIND_ADDRESS:-127.0.0.1}"
PRIMARY_HOST="${REPLICA_READ_PRIMARY_HOST:-127.0.0.1}"
REPLICA_HOST="${REPLICA_READ_REPLICA_HOST:-${PRIMARY_HOST}}"
PRIMARY_PORT="${REPLICA_READ_PRIMARY_PORT:-55441}"
REPLICA_PORT="${REPLICA_READ_REPLICA_PORT:-55442}"
FILE_SIZE="${FIO_FILE_SIZE:-1G}"
BLOCK_SIZE="${FIO_BLOCK_SIZE:-4k}"
WAIT_SECONDS="${REPLICA_WAIT_SECONDS:-120}"
LABEL="${REPLICA_READ_LABEL:-docker}"
PROJECT="fod-replica-read-${BASHPID}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-${LABEL}-primary-replica-${BLOCK_SIZE}-${RUN_ID}"

read -r -a COMPOSE_CMD <<<"${FOD_REPLICA_READ_COMPOSE:-docker compose}"

for cmd in docker psql fio mountpoint python3; do
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
    REPLICA_READ_BIND_ADDRESS="${BIND_ADDRESS}" \
    REPLICA_READ_PRIMARY_PORT="${PRIMARY_PORT}" \
    REPLICA_READ_REPLICA_PORT="${REPLICA_PORT}" \
    "${COMPOSE_CMD[@]}" -f "${COMPOSE_FILE}" "$@"
}

psql_primary() {
    PGPASSWORD="${POSTGRES_PASSWORD:-cichosza}" \
    psql -h "${PRIMARY_HOST}" -p "${PRIMARY_PORT}" \
        -U "${POSTGRES_USER:-foduser}" \
        -d "${POSTGRES_DB:-foddbname}" -Atq "$@"
}

psql_replica() {
    PGPASSWORD="${POSTGRES_PASSWORD:-cichosza}" \
    psql -h "${REPLICA_HOST}" -p "${REPLICA_PORT}" \
        -U "${POSTGRES_USER:-foduser}" \
        -d "${POSTGRES_DB:-foddbname}" -Atq "$@"
}

fio_metrics() {
    local json_file="$1"
    local op="$2"
    python3 - "${json_file}" "${op}" <<'PY'
import json
import sys

path, op = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as handle:
    payload = json.load(handle)
section = payload["jobs"][0][op]
bw_bytes = section.get("bw_bytes")
if bw_bytes is None:
    bw_bytes = float(section.get("bw", 0.0)) * 1024.0
iops = float(section.get("iops", 0.0))
lat_mean_ns = section.get("lat_ns", {}).get("mean")
if lat_mean_ns is None:
    lat_mean_ns = section.get("clat_ns", {}).get("mean", 0.0)
print(
    f"{float(bw_bytes) / 1048576.0:.3f}\t"
    f"{iops:.3f}\t"
    f"{float(lat_mean_ns) / 1000.0:.3f}"
)
PY
}

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

wait_for_replay_lsn() {
    local lsn="$1"
    local label="$2"

    for ((second = 0; second < WAIT_SECONDS; second++)); do
        local replay
        replay="$(psql_replica -c \
            "SELECT CASE WHEN pg_last_wal_replay_lsn() >= '${lsn}'::pg_lsn THEN '1' ELSE '0' END" \
            2>/dev/null || true)"
        replay="$(printf '%s' "${replay}" | tr -d '[:space:]')"
        if [[ "${replay}" == "1" ]]; then
            echo "${label}_caught_up_after_seconds=${second}" \
                | tee -a "${ARTIFACT_DIR}/replication.txt"
            return 0
        fi
        sleep 1
    done

    echo "Replica did not replay ${lsn} during ${label}" >&2
    return 1
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

echo "=== Primary/replica FOD performance diagnostic ==="
echo "label=${LABEL}"
echo "docker_bind=${BIND_ADDRESS}"
echo "primary=${PRIMARY_HOST}:${PRIMARY_PORT}"
echo "replica=${REPLICA_HOST}:${REPLICA_PORT}"
echo "database=${POSTGRES_DB:-foddbname} user=${POSTGRES_USER:-foduser}"
echo "size=${FILE_SIZE} block_size=${BLOCK_SIZE}"
echo "artifact_dir=${ARTIFACT_DIR}"

fod_test_write_power_metadata "${ARTIFACT_DIR}/power-before.txt" "before"
fod_test_log_power_metadata "before"

compose down -v --remove-orphans >/dev/null 2>&1 || true
compose up -d --build primary replica
STACK_STARTED=1

wait_for_role primary "false|off" primary
wait_for_role replica "true|on" replica

# Normalize the complete FOD endpoint to the credentials used to create this
# isolated primary/replica stack. Makefile exports FOD_PG_* globally, so
# without this override QNAP could combine a remote host with local defaults.
export FOD_PG_HOST="${PRIMARY_HOST}"
export FOD_PG_PORT="${PRIMARY_PORT}"
export FOD_PG_DBNAME="${POSTGRES_DB:-foddbname}"
export FOD_PG_USER="${POSTGRES_USER:-foduser}"
export FOD_PG_PASSWORD="${POSTGRES_PASSWORD:-cichosza}"
export FOD_PG_PRIMARY_HOSTS="${PRIMARY_HOST}:${PRIMARY_PORT}"
export FOD_PG_REPLICA_HOSTS="${REPLICA_HOST}:${REPLICA_PORT}"
export FOD_PG_ENDPOINT_ROUTING_ENABLED=1
export FOD_PG_RUNTIME_FAILOVER_ENABLED=0
export FOD_PG_REPLICA_READ_ROUTING_ENABLED=0

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
export FOD_PROFILE_IO=1
export FOD_LOG_LEVEL=info

fod_test_setup "${ROOT}"
fod_test_init_schema

TEST_BASENAME="fio-primary-replica-${RUN_ID}.bin"

echo "=== PHASE 1: PRIMARY WRITE ==="
FOD_ROLE=primary
fod_test_make_mountpoint "/tmp/fod-primary-write"
FOD_TEST_LOG_ARCHIVE="${ARTIFACT_DIR}/primary-write-mount.log"
fod_test_start_mount "${MOUNTPOINT}"

WRITE_FILE="${MOUNTPOINT}/${TEST_BASENAME}"
WRITE_JSON="${ARTIFACT_DIR}/primary-write-fio.json"

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
    --output-format=json \
    --output="${WRITE_JSON}"

read -r WRITE_MIB WRITE_IOPS WRITE_LAT_US < <(fio_metrics "${WRITE_JSON}" write)
echo "primary_write block_size=${BLOCK_SIZE} mib_s=${WRITE_MIB} iops=${WRITE_IOPS} lat_mean_us=${WRITE_LAT_US}"

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
echo "primary_write_flush_lsn=${PRIMARY_LSN}" >"${ARTIFACT_DIR}/replication.txt"

fod_test_cleanup

echo "=== PHASE 2: WAIT REPLICA AFTER WRITE ==="
wait_for_replay_lsn "${PRIMARY_LSN}" "after_write"

echo "=== PHASE 3: PRIMARY RESTART / FRESH PRIMARY READ ==="
compose restart primary
wait_for_role "primary-after-restart" "false|off" primary

FOD_ROLE=primary
fod_test_make_mountpoint "/tmp/fod-primary-read"
FOD_TEST_LOG_ARCHIVE="${ARTIFACT_DIR}/primary-read-mount.log"
fod_test_start_mount "${MOUNTPOINT}"

PRIMARY_READ_FILE="${MOUNTPOINT}/${TEST_BASENAME}"
[[ -f "${PRIMARY_READ_FILE}" ]] || {
    echo "Primary does not expose ${TEST_BASENAME}" >&2
    exit 1
}

PRIMARY_READ_JSON="${ARTIFACT_DIR}/primary-read-fio.json"
fio \
    --name=primary-read \
    --filename="${PRIMARY_READ_FILE}" \
    --ioengine=sync \
    --rw=read \
    --bs="${BLOCK_SIZE}" \
    --size="${FILE_SIZE}" \
    --numjobs=1 \
    --group_reporting=1 \
    --direct=0 \
    --output-format=json \
    --output="${PRIMARY_READ_JSON}"

read -r PRIMARY_READ_MIB PRIMARY_READ_IOPS PRIMARY_READ_LAT_US < <(
    fio_metrics "${PRIMARY_READ_JSON}" read
)
echo "primary_read block_size=${BLOCK_SIZE} mib_s=${PRIMARY_READ_MIB} iops=${PRIMARY_READ_IOPS} lat_mean_us=${PRIMARY_READ_LAT_US}"

fod_test_cleanup

PRIMARY_AFTER_READ_LSN="$(psql_primary -c "SELECT pg_current_wal_flush_lsn()::text")"
PRIMARY_AFTER_READ_LSN="$(printf '%s' "${PRIMARY_AFTER_READ_LSN}" | tr -d '[:space:]')"
echo "primary_after_read_flush_lsn=${PRIMARY_AFTER_READ_LSN}" \
    | tee -a "${ARTIFACT_DIR}/replication.txt"
wait_for_replay_lsn "${PRIMARY_AFTER_READ_LSN}" "after_primary_read"

echo "=== PHASE 4: STOP PRIMARY / RESTART REPLICA ==="
compose stop primary
if psql_primary -c "SELECT 1" >/dev/null 2>&1; then
    echo "Primary is still reachable" >&2
    exit 1
fi
echo "primary_reachable_before_replica_read=0" \
    | tee -a "${ARTIFACT_DIR}/replication.txt"

compose restart replica
wait_for_role "replica-after-restart" "true|on" replica

echo "=== PHASE 5: FRESH REPLICA READ ==="
FOD_ROLE=replica
fod_test_make_mountpoint "/tmp/fod-replica-read"
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

REPLICA_READ_JSON="${ARTIFACT_DIR}/replica-read-fio.json"
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
    --output-format=json \
    --output="${REPLICA_READ_JSON}"

read -r REPLICA_READ_MIB REPLICA_READ_IOPS REPLICA_READ_LAT_US < <(
    fio_metrics "${REPLICA_READ_JSON}" read
)
echo "replica_read block_size=${BLOCK_SIZE} mib_s=${REPLICA_READ_MIB} iops=${REPLICA_READ_IOPS} lat_mean_us=${REPLICA_READ_LAT_US}"

if touch "${MOUNTPOINT}/replica-write-must-fail-${RUN_ID}" \
    2>"${ARTIFACT_DIR}/replica-write-rejection.txt"; then
    echo "Replica unexpectedly accepted a filesystem write" >&2
    exit 1
fi
echo "replica_write_guard=read_only_rejected" \
    | tee -a "${ARTIFACT_DIR}/replication.txt"

fod_test_cleanup

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

if [[ -z "${FINAL_OPERATION_FAILURES}" ]]; then
    echo "Could not read operation_failures from replica observability" >&2
    exit 1
fi
if [[ "${FINAL_OPERATION_FAILURES}" != "0" ]]; then
    echo "Replica read DB operation failures: ${FINAL_OPERATION_FAILURES}" >&2
    exit 1
fi

fod_test_write_power_metadata "${ARTIFACT_DIR}/power-after.txt" "after"

printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "block_size" "file_size" \
    "primary_write_mib_s" "primary_write_iops" \
    "primary_read_mib_s" "primary_read_iops" \
    "replica_read_mib_s" "replica_read_iops" \
    "replica_operation_failures" "replica_write_guard" \
    >"${ARTIFACT_DIR}/result.tsv"
printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "${BLOCK_SIZE}" "${FILE_SIZE}" \
    "${WRITE_MIB}" "${WRITE_IOPS}" \
    "${PRIMARY_READ_MIB}" "${PRIMARY_READ_IOPS}" \
    "${REPLICA_READ_MIB}" "${REPLICA_READ_IOPS}" \
    "${FINAL_OPERATION_FAILURES}" "read_only_rejected" \
    >>"${ARTIFACT_DIR}/result.tsv"

echo "=== RESULT ==="
cat "${ARTIFACT_DIR}/result.tsv"
echo "--- replication ---"
cat "${ARTIFACT_DIR}/replication.txt"
echo "primary stopped before replica read: yes"
echo "primary PostgreSQL restarted before primary read: yes"
echo "replica PostgreSQL restarted before replica read: yes"
echo "FOD read cache/read-ahead/prefetch disabled: yes"
echo "FOD FUSE direct_io enabled: yes"
echo "host kernel page cache dropped: no"

echo "PERF_RESULT block_size=${BLOCK_SIZE} file_size=${FILE_SIZE} primary_write_mib_s=${WRITE_MIB} primary_write_iops=${WRITE_IOPS} primary_read_mib_s=${PRIMARY_READ_MIB} primary_read_iops=${PRIMARY_READ_IOPS} replica_read_mib_s=${REPLICA_READ_MIB} replica_read_iops=${REPLICA_READ_IOPS} replica_operation_failures=${FINAL_OPERATION_FAILURES} replica_write_guard=read_only_rejected"
echo "OK: primary write/read -> WAL replay -> primary stopped -> replica read"
