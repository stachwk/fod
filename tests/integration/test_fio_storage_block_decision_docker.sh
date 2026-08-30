#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# One isolated storage-block decision workload. Random-overwrite workloads first
# create a 1 GiB incompressible file outside the measured window, force a
# checkpoint + replica catch-up + host sync, then profile only the overwrite.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
cd "${ROOT}"

COMPOSE_FILE="${ROOT}/${REPLICA_READ_COMPOSE_FILE:-docker-compose.replica-read.yml}"
PRIMARY_HOST="${REPLICA_READ_PRIMARY_HOST:-127.0.0.1}"
REPLICA_HOST="${REPLICA_READ_REPLICA_HOST:-${PRIMARY_HOST}}"
PRIMARY_PORT="${REPLICA_READ_PRIMARY_PORT:-55441}"
REPLICA_PORT="${REPLICA_READ_REPLICA_PORT:-55442}"
BIND_ADDRESS="${REPLICA_READ_BIND_ADDRESS:-127.0.0.1}"
WAIT_SECONDS="${REPLICA_WAIT_SECONDS:-120}"
STORAGE_BLOCK_SIZE="${FOD_TEST_STORAGE_BLOCK_SIZE:?FOD_TEST_STORAGE_BLOCK_SIZE is required}"
WORKLOAD="${FOD_STORAGE_DECISION_WORKLOAD:?FOD_STORAGE_DECISION_WORKLOAD is required}"
FILE_SIZE="${FOD_STORAGE_DECISION_FILE_SIZE:-1G}"
IO_SIZE="${FOD_STORAGE_DECISION_IO_SIZE:-1G}"
COOLDOWN_SECONDS="${FOD_STORAGE_DECISION_COOLDOWN_SECONDS:-10}"
DIRTY_LIMIT_KB="${FOD_STORAGE_DECISION_DIRTY_LIMIT_KB:-8192}"
PROFILE_DIR="${FOD_STORAGE_DECISION_PROFILE_DIR:?FOD_STORAGE_DECISION_PROFILE_DIR is required}"
ARTIFACT_DIR="${FOD_STORAGE_DECISION_RUN_DIR:?FOD_STORAGE_DECISION_RUN_DIR is required}"
PROJECT="fod-storage-decision-${BASHPID}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
PROFILER="${ROOT}/scripts/perf/profile_primary_write_postgres.sh"
SKIP_BUILD="${FOD_STORAGE_DECISION_SKIP_BUILD:-0}"

case "${WORKLOAD}" in
    randwrite-4k)   FIO_RW=randwrite; FIO_BS=4k;   PREFILL=1 ;;
    randwrite-16k)  FIO_RW=randwrite; FIO_BS=16k;  PREFILL=1 ;;
    randwrite-64k)  FIO_RW=randwrite; FIO_BS=64k;  PREFILL=1 ;;
    seqwrite-512k)  FIO_RW=write;     FIO_BS=512k; PREFILL=0 ;;
    *) echo "Unsupported FOD_STORAGE_DECISION_WORKLOAD=${WORKLOAD}" >&2; exit 2 ;;
esac

for cmd in docker psql fio python3 sync awk sed grep mktemp chmod; do
    command -v "${cmd}" >/dev/null 2>&1 || { echo "Missing required command: ${cmd}" >&2; exit 2; }
done
mkdir -p "${ARTIFACT_DIR}" "${PROFILE_DIR}"

if [[ "${SKIP_BUILD}" != "1" ]]; then
    FOD_CARGO_PROFILE="${FOD_CARGO_PROFILE:-profiling}" \
    FOD_RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}" \
        make --no-print-directory build-runtime
fi

artifact_profile="${FOD_RUNTIME_PROFILE:-profiling}"
[[ "${artifact_profile}" == "dev" ]] && artifact_profile=debug
target_root="${CARGO_TARGET_DIR:-${ROOT}/target}"
[[ "${target_root}" != /* ]] && target_root="${ROOT}/${target_root}"
REAL_MKFS="${FOD_MKFS_REAL_BIN:-${target_root}/${artifact_profile}/fod-rust-mkfs}"
[[ -x "${REAL_MKFS}" ]] || { echo "Missing mkfs binary: ${REAL_MKFS}" >&2; exit 1; }

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/fod-storage-decision.XXXXXX")"
MKFS_WRAPPER="${TMP_DIR}/fod-rust-mkfs-block-size"
cat >"${MKFS_WRAPPER}" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail
REAL="${FOD_MKFS_REAL_BIN:?}"
BLOCK_SIZE="${FOD_TEST_STORAGE_BLOCK_SIZE:?}"
if [[ "${1:-}" == "init" ]]; then
    for arg in "$@"; do
        case "${arg}" in --block-size|--block-size=*) exec "${REAL}" "$@" ;; esac
    done
    exec "${REAL}" "$@" --block-size "${BLOCK_SIZE}"
fi
exec "${REAL}" "$@"
WRAPPER
chmod 0700 "${MKFS_WRAPPER}"
export FOD_MKFS_BIN="${MKFS_WRAPPER}"
export FOD_MKFS_REAL_BIN="${REAL_MKFS}"

read -r -a COMPOSE_CMD <<<"${FOD_REPLICA_READ_COMPOSE:-docker compose}"
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
    PGPASSWORD="${POSTGRES_PASSWORD:-cichosza}" psql -X -h "${PRIMARY_HOST}" -p "${PRIMARY_PORT}" \
        -U "${POSTGRES_USER:-foduser}" -d "${POSTGRES_DB:-foddbname}" -Atq "$@"
}
psql_replica() {
    PGPASSWORD="${POSTGRES_PASSWORD:-cichosza}" psql -X -h "${REPLICA_HOST}" -p "${REPLICA_PORT}" \
        -U "${POSTGRES_USER:-foduser}" -d "${POSTGRES_DB:-foddbname}" -Atq "$@"
}
wait_for_role() {
    local expected="$1" which="$2" value second
    for ((second=0; second<WAIT_SECONDS; second++)); do
        if [[ "${which}" == primary ]]; then
            value="$(psql_primary -c "SELECT pg_is_in_recovery()::text || '|' || current_setting('transaction_read_only')" 2>/dev/null || true)"
        else
            value="$(psql_replica -c "SELECT pg_is_in_recovery()::text || '|' || current_setting('transaction_read_only')" 2>/dev/null || true)"
        fi
        value="$(printf '%s' "${value}" | tr -d '[:space:]')"
        [[ "${value}" == "${expected}" ]] && return 0
        sleep 1
    done
    echo "${which} did not reach ${expected}" >&2; return 1
}
wait_for_replay_lsn() {
    local lsn="$1" second replay
    for ((second=0; second<WAIT_SECONDS; second++)); do
        replay="$(psql_replica -c "SELECT CASE WHEN pg_last_wal_replay_lsn() >= '${lsn}'::pg_lsn THEN '1' ELSE '0' END" 2>/dev/null || true)"
        replay="$(printf '%s' "${replay}" | tr -d '[:space:]')"
        [[ "${replay}" == 1 ]] && { echo "replica_caught_up_after_seconds=${second}"; return 0; }
        sleep 1
    done
    echo "Replica did not replay ${lsn}" >&2; return 1
}
fio_metric() {
    python3 - "$1" <<'PY'
import json,sys
p=json.load(open(sys.argv[1],encoding='utf-8'))['jobs'][0]['write']
bw=p.get('bw_bytes',float(p.get('bw',0))*1024)
iops=float(p.get('iops',0))
lat=p.get('lat_ns',{}).get('mean',p.get('clat_ns',{}).get('mean',0))
print(f"{float(bw)/1048576:.3f}\t{iops:.3f}\t{float(lat)/1000:.3f}")
PY
}
wait_host_dirty() {
    local started now dirty writeback total
    sync
    started="$(date +%s)"
    while true; do
        dirty="$(awk '$1=="Dirty:"{print $2}' /proc/meminfo)"
        writeback="$(awk '$1=="Writeback:"{print $2}' /proc/meminfo)"
        total=$(( dirty + writeback ))
        if (( total <= DIRTY_LIMIT_KB )); then
            sleep "${COOLDOWN_SECONDS}"
            dirty="$(awk '$1=="Dirty:"{print $2}' /proc/meminfo)"
            writeback="$(awk '$1=="Writeback:"{print $2}' /proc/meminfo)"
            total=$(( dirty + writeback ))
            (( total <= DIRTY_LIMIT_KB )) && { echo "host_settle_dirty_kb=${dirty} host_settle_writeback_kb=${writeback}"; return 0; }
        fi
        now="$(date +%s)"
        (( now - started >= 180 )) && { echo "Host did not settle" >&2; return 1; }
        sleep 2
    done
}

MOUNTPOINT=""; LOG_FILE=""; FOD_PID=""; STACK_STARTED=0; PROFILE_PID=""
cleanup() {
    local rc=$?
    set +e
    [[ -n "${PROFILE_PID:-}" ]] && kill -0 "${PROFILE_PID}" 2>/dev/null && kill "${PROFILE_PID}" 2>/dev/null
    [[ -n "${PROFILE_PID:-}" ]] && wait "${PROFILE_PID}" 2>/dev/null
    [[ -n "${MOUNTPOINT:-}" || -n "${FOD_PID:-}" ]] && fod_test_cleanup
    (( STACK_STARTED )) && compose logs --no-color >"${ARTIFACT_DIR}/docker-compose.log" 2>&1
    (( STACK_STARTED )) && compose down -v --remove-orphans >/dev/null 2>&1
    rm -rf "${TMP_DIR}"
    exit "${rc}"
}
trap cleanup EXIT INT TERM

compose down -v --remove-orphans >/dev/null 2>&1 || true
compose up -d primary replica
STACK_STARTED=1
wait_for_role "false|off" primary
wait_for_role "true|on" replica

export FOD_PG_HOST="${PRIMARY_HOST}" FOD_PG_PORT="${PRIMARY_PORT}" FOD_PG_DBNAME="${POSTGRES_DB:-foddbname}"
export FOD_PG_USER="${POSTGRES_USER:-foduser}" FOD_PG_PASSWORD="${POSTGRES_PASSWORD:-cichosza}"
export FOD_PG_PRIMARY_HOSTS="${PRIMARY_HOST}:${PRIMARY_PORT}" FOD_PG_REPLICA_HOSTS="${REPLICA_HOST}:${REPLICA_PORT}"
export FOD_PG_ENDPOINT_ROUTING_ENABLED=1 FOD_PG_RUNTIME_FAILOVER_ENABLED=0 FOD_PG_REPLICA_READ_ROUTING_ENABLED=0
export FOD_READ_CACHE_BLOCKS=0 FOD_READ_AHEAD_BLOCKS=0 FOD_SEQUENTIAL_READ_AHEAD_BLOCKS=0
export FOD_DIRECT_IO_READ_PREFETCH_BLOCKS=0 FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS=0
export FOD_METADATA_CACHE_TTL_SECONDS=0 FOD_STATFS_CACHE_TTL_SECONDS=0 FOD_FOPEN_DIRECT_IO=1
export FOD_FUSE_WRITEBACK_CACHE=0 FOD_ATIME_POLICY=noatime FOD_PROFILE_IO=1 FOD_LOG_LEVEL=info

fod_test_setup "${ROOT}"
fod_test_init_schema

TARGET_BASENAME="storage-decision-${WORKLOAD}-${RUN_ID}.bin"
if (( PREFILL )); then
    fod_test_make_mountpoint "/tmp/fod-storage-decision-prefill"
    FOD_TEST_LOG_ARCHIVE="${ARTIFACT_DIR}/prefill-mount.log"
    fod_test_start_mount "${MOUNTPOINT}"
    TARGET_FILE="${MOUNTPOINT}/${TARGET_BASENAME}"
    fio --name=prefill --filename="${TARGET_FILE}" --ioengine=sync --rw=write --bs=512k \
        --size="${FILE_SIZE}" --numjobs=1 --group_reporting=1 --direct=0 --fsync_on_close=1 \
        --refill_buffers=1 --randrepeat=0 --output-format=json --output="${ARTIFACT_DIR}/prefill.json"
    fod_test_cleanup
    BASELINE_LSN="$(psql_primary -c "CHECKPOINT; SELECT pg_current_wal_flush_lsn()::text" | tail -1 | tr -d '[:space:]')"
    wait_for_replay_lsn "${BASELINE_LSN}"
else
    BASELINE_LSN="$(psql_primary -c "CHECKPOINT; SELECT pg_current_wal_flush_lsn()::text" | tail -1 | tr -d '[:space:]')"
    wait_for_replay_lsn "${BASELINE_LSN}"
fi
wait_host_dirty | tee "${ARTIFACT_DIR}/pre-measure-settle.txt"

FOD_PG_WRITE_PROFILE_OUT="${PROFILE_DIR}" \
FOD_PG_WRITE_PROFILE_PROCESS_MATCH="/tmp/fod-storage-decision-measured." \
    bash "${PROFILER}" >"${ARTIFACT_DIR}/postgres-profile.log" 2>&1 &
PROFILE_PID=$!

fod_test_make_mountpoint "/tmp/fod-storage-decision-measured"
FOD_TEST_LOG_ARCHIVE="${ARTIFACT_DIR}/measured-mount.log"
fod_test_start_mount "${MOUNTPOINT}"
TARGET_FILE="${MOUNTPOINT}/${TARGET_BASENAME}"

if (( PREFILL )); then
    [[ -f "${TARGET_FILE}" ]] || { echo "Prefilled file missing before overwrite" >&2; exit 1; }
    [[ "$(stat -c '%s' "${TARGET_FILE}")" == "1073741824" || "${FILE_SIZE}" != "1G" ]] || { echo "Unexpected prefill size" >&2; exit 1; }
    OVERWRITE_ARGS=(--overwrite=1)
else
    OVERWRITE_ARGS=()
fi

MEASURED_JSON="${ARTIFACT_DIR}/measured-fio.json"
fio --name=measured --filename="${TARGET_FILE}" --ioengine=sync --rw="${FIO_RW}" --bs="${FIO_BS}" \
    --size="${FILE_SIZE}" --io_size="${IO_SIZE}" --numjobs=1 --group_reporting=1 --direct=0 --fsync_on_close=1 \
    --refill_buffers=1 --randrepeat=0 "${OVERWRITE_ARGS[@]}" --output-format=json --output="${MEASURED_JSON}"
read -r WRITE_MIB WRITE_IOPS WRITE_LAT_US < <(fio_metric "${MEASURED_JSON}")
MEASURED_LSN="$(psql_primary -c "SELECT pg_current_wal_flush_lsn()::text" | tr -d '[:space:]')"
fod_test_cleanup
wait_for_replay_lsn "${MEASURED_LSN}" | tee "${ARTIFACT_DIR}/replication.txt"

if ! wait "${PROFILE_PID}"; then
    PROFILE_PID=""
    echo "PostgreSQL profiler failed" >&2
    exit 1
fi
PROFILE_PID=""

STATEMENTS="${PROFILE_DIR}/statements.tsv"
[[ -s "${STATEMENTS}" ]] || { echo "Missing profiler statements.tsv" >&2; exit 1; }
copy_line="$(awk -F '\t' '$2=="copy_stage"{line=$0} END{print line}' "${STATEMENTS}")"
insert_line="$(awk -F '\t' '$2=="insert_on_conflict"{line=$0} END{print line}' "${STATEMENTS}")"
field() { printf '%s\n' "$1" | awk -F '\t' -v c="$2" '{print $c+0}'; }

SUMMARY="${ARTIFACT_DIR}/summary.tsv"
printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    workload storage_block_size fio_block_size file_size io_size prefilled write_mib_s write_iops write_lat_us \
    copy_calls copy_rows copy_exec_ms copy_local_blks_written insert_calls insert_rows insert_exec_ms insert_shared_blks_written insert_wal_bytes \
    >"${SUMMARY}"
printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "${WORKLOAD}" "${STORAGE_BLOCK_SIZE}" "${FIO_BS}" "${FILE_SIZE}" "${IO_SIZE}" "${PREFILL}" \
    "${WRITE_MIB}" "${WRITE_IOPS}" "${WRITE_LAT_US}" \
    "$(field "${copy_line}" 3)" "$(field "${copy_line}" 6)" "$(field "${copy_line}" 4)" "$(field "${copy_line}" 14)" \
    "$(field "${insert_line}" 3)" "$(field "${insert_line}" 6)" "$(field "${insert_line}" 4)" "$(field "${insert_line}" 10)" "$(field "${insert_line}" 23)" \
    >>"${SUMMARY}"

echo "=== STORAGE BLOCK DECISION RESULT ==="
cat "${SUMMARY}"
echo "profile_artifact_dir=${PROFILE_DIR}"
echo "OK: storage block decision workload"
