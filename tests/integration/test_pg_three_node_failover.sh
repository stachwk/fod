#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# PostgreSQL chain failover: master -> promoted slave1 -> slave2 follows slave1.
# FOD client operations are run on this host with slave2 as the replica target.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
COMPOSE_FILE="${ROOT}/docker-compose.ha-chain.yml"
BIND_ADDRESS="${HA_CHAIN_BIND_ADDRESS:-127.0.0.1}"
MASTER_PORT="${HA_CHAIN_MASTER_PORT:-55471}"
SLAVE1_PORT="${HA_CHAIN_SLAVE1_PORT:-55472}"
SLAVE2_PORT="${HA_CHAIN_SLAVE2_PORT:-55473}"
WAIT_SECONDS="${HA_CHAIN_WAIT_SECONDS:-120}"
PROJECT="fod-ha-chain-${BASHPID}"
ARTIFACT_DIR="${ROOT}/artifacts/integration/ha-chain-$(date -u +%Y%m%dT%H%M%SZ)"
STACK_STARTED=0
MOUNTPOINT=""
LOG_FILE=""
FOD_PID=""
STALE_READER_PID=""

for cmd in docker psql mountpoint; do
    command -v "${cmd}" >/dev/null 2>&1 || {
        echo "Missing required command: ${cmd}" >&2
        exit 2
    }
done

fod_test_setup "${ROOT}"
mkdir -p "${ARTIFACT_DIR}"

compose() {
    COMPOSE_PROJECT_NAME="${PROJECT}" \
    POSTGRES_DB="${POSTGRES_DB}" POSTGRES_USER="${POSTGRES_USER}" \
    POSTGRES_PASSWORD="${POSTGRES_PASSWORD}" \
    HA_CHAIN_BIND_ADDRESS="${BIND_ADDRESS}" \
    HA_CHAIN_MASTER_PORT="${MASTER_PORT}" \
    HA_CHAIN_SLAVE1_PORT="${SLAVE1_PORT}" \
    HA_CHAIN_SLAVE2_PORT="${SLAVE2_PORT}" \
    docker compose -f "${COMPOSE_FILE}" "$@"
}

psql_node() {
    local port="$1"
    shift
    PGPASSWORD="${POSTGRES_PASSWORD}" psql -h 127.0.0.1 -p "${port}" \
        -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Atq "$@"
}

psql_container_admin() {
    local node="$1" sql="$2"
    compose exec -T --user postgres "${node}" \
        psql -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Atq -c "${sql}"
}

wait_sql() {
    local label="$1" port="$2" sql="$3" expected="$4" value=""
    for _ in $(seq 1 "${WAIT_SECONDS}"); do
        value="$(psql_node "${port}" -c "${sql}" 2>/dev/null || true)"
        value="$(printf '%s' "${value}" | tr -d '[:space:]')"
        if [[ "${value}" == "${expected}" ]]; then
            printf '%s=%s\n' "${label}" "${value}"
            return 0
        fi
        sleep 1
    done
    echo "Timed out waiting for ${label}: expected=${expected} actual=${value:-unreachable}" >&2
    return 1
}

wait_replay() {
    local label="$1" port="$2" lsn="$3" value=""
    for _ in $(seq 1 "${WAIT_SECONDS}"); do
        value="$(psql_node "${port}" -c \
            "SELECT COALESCE((pg_last_wal_replay_lsn() >= '${lsn}'::pg_lsn)::text, 'false')" \
            2>/dev/null || true)"
        value="$(printf '%s' "${value}" | tr -d '[:space:]')"
        if [[ "${value}" == true ]]; then
            printf '%s_replayed_lsn=%s\n' "${label}" "${lsn}"
            return 0
        fi
        sleep 1
    done
    echo "Timed out waiting for ${label} replay through ${lsn}" >&2
    return 1
}

cleanup() {
    local status=$?
    trap - EXIT
    if [[ -n "${STALE_READER_PID}" ]]; then
        kill "${STALE_READER_PID}" 2>/dev/null || true
        wait "${STALE_READER_PID}" 2>/dev/null || true
    fi
    if [[ -n "${MOUNTPOINT}" ]]; then
        fod_test_cleanup || true
        if [[ -n "${LOG_FILE}" && -f "${LOG_FILE}" ]]; then
            cp "${LOG_FILE}" "${ARTIFACT_DIR}/fod-mount-final.log" 2>/dev/null || true
        fi
    fi
    if ((STACK_STARTED)); then
        compose down -v --remove-orphans >/dev/null 2>&1 || true
    fi
    if ((status != 0)); then
        echo "HA chain test failed; artifacts: ${ARTIFACT_DIR}" >&2
    fi
    exit "${status}"
}
trap cleanup EXIT

export FOD_PG_HOST=127.0.0.1 FOD_PG_PORT="${MASTER_PORT}"
export FOD_PG_DBNAME="${POSTGRES_DB}" FOD_PG_USER="${POSTGRES_USER}"
export FOD_PG_PASSWORD="${POSTGRES_PASSWORD}"
export FOD_PG_PRIMARY_HOSTS="127.0.0.1:${MASTER_PORT}"
export FOD_PG_REPLICA_HOSTS="127.0.0.1:${SLAVE1_PORT},127.0.0.1:${SLAVE2_PORT}"
export FOD_PG_ENDPOINT_ROUTING_ENABLED=1 FOD_PG_RUNTIME_FAILOVER_ENABLED=0
export FOD_PG_REPLICA_READ_ROUTING_ENABLED=0 FOD_ROLE=primary
export FOD_ATIME_POLICY=noatime FOD_FUSE_WRITEBACK_CACHE=0 FOD_LOG_LEVEL=info

echo "=== Start master, slave1 and slave2 ==="
compose down -v --remove-orphans >/dev/null 2>&1 || true
compose up -d --build master slave1 slave2
STACK_STARTED=1
wait_sql master_is_primary "${MASTER_PORT}" "SELECT NOT pg_is_in_recovery()" t
wait_sql slave1_is_standby "${SLAVE1_PORT}" "SELECT pg_is_in_recovery()" t
wait_sql slave2_is_standby "${SLAVE2_PORT}" "SELECT pg_is_in_recovery()" t

INITIAL_ID="$(psql_container_admin master \
    "SELECT system_identifier FROM pg_control_system()")"
INITIAL_TIMELINE="$(psql_container_admin master \
    "SELECT timeline_id FROM pg_control_checkpoint()")"
printf 'initial_system_identifier=%s\ninitial_timeline=%s\n' \
    "${INITIAL_ID}" "${INITIAL_TIMELINE}" | tee "${ARTIFACT_DIR}/topology.txt"

fod_test_init_schema
fod_test_make_mountpoint "/tmp/fod-ha-chain-before"
FOD_TEST_LOG_ARCHIVE="${ARTIFACT_DIR}/fod-master-mount.log"
fod_test_start_mount "${MOUNTPOINT}"
printf 'marker written through original master\n' >"${MOUNTPOINT}/ha-chain-marker.txt"
printf 'stale-read target written through original master\n' \
    >"${MOUNTPOINT}/ha-chain-estale-marker.txt"
sync
fod_test_cleanup
MOUNTPOINT="" LOG_FILE=""

INITIAL_LSN="$(psql_node "${MASTER_PORT}" -c \
    "SELECT pg_current_wal_flush_lsn()::text")"
printf 'marker_flush_lsn=%s\n' "${INITIAL_LSN}" | tee -a "${ARTIFACT_DIR}/topology.txt"
wait_replay slave1_initial "${SLAVE1_PORT}" "${INITIAL_LSN}"
wait_replay slave2_initial "${SLAVE2_PORT}" "${INITIAL_LSN}"

echo "=== Start degraded mount on slave1 and observe its promotion ==="
export FOD_ROLE=replica FOD_DEGRADED_READONLY_ENABLED=1
fod_test_make_mountpoint "/tmp/fod-ha-chain-role-monitor"
FOD_TEST_LOG_ARCHIVE="${ARTIFACT_DIR}/fod-role-monitor.log"
fod_test_start_mount "${MOUNTPOINT}"
[[ "$(cat "${MOUNTPOINT}/ha-chain-marker.txt")" == "marker written through original master" ]] || {
    echo "Degraded monitor mount could not read the initial marker" >&2
    exit 1
}

echo "=== Verify ESTALE for an opened standby file removed from its replayed snapshot ==="
STALE_CONTROL_DIR="$(mktemp -d "${ARTIFACT_DIR}/stale-read.XXXXXX")"
python3 - "${MOUNTPOINT}/ha-chain-estale-marker.txt" "${STALE_CONTROL_DIR}" <<'PY' &
import errno
import os
import sys
import time
from pathlib import Path

path = sys.argv[1]
control = Path(sys.argv[2])
fd = os.open(path, os.O_RDONLY)
try:
    first = os.read(fd, 1)
    if first != b"s":
        raise AssertionError(f"unexpected initial byte: {first!r}")
    (control / "ready").write_text("ready", encoding="ascii")
    deadline = time.monotonic() + 180
    while not (control / "read").exists():
        if time.monotonic() >= deadline:
            raise TimeoutError("timed out waiting for stale read trigger")
        time.sleep(0.02)
    try:
        result = os.read(fd, 1)
    except OSError as exc:
        name = errno.errorcode.get(exc.errno, str(exc.errno))
        (control / "result").write_text(name, encoding="ascii")
    else:
        (control / "result").write_text(f"success:{result!r}", encoding="ascii")
finally:
    os.close(fd)
PY
STALE_READER_PID=$!
for _ in $(seq 1 $((WAIT_SECONDS * 10))); do
    [[ -f "${STALE_CONTROL_DIR}/ready" ]] && break
    kill -0 "${STALE_READER_PID}" 2>/dev/null || {
        echo "ESTALE reader exited before opening its descriptor" >&2
        exit 1
    }
    sleep 0.1
done
[[ -f "${STALE_CONTROL_DIR}/ready" ]] || {
    echo "Timed out waiting for degraded mount to open the ESTALE target" >&2
    exit 1
}
DELETED_FILE_ID="$(psql_node "${MASTER_PORT}" -c \
    "SET search_path TO fod, public; DELETE FROM files WHERE name = 'ha-chain-estale-marker.txt' RETURNING id_file")"
[[ "${DELETED_FILE_ID}" =~ ^[0-9]+$ ]] || {
    echo "Could not remove test file row on primary: ${DELETED_FILE_ID:-empty}" >&2
    exit 1
}
DELETE_LSN="$(psql_node "${MASTER_PORT}" -c \
    "SELECT pg_current_wal_flush_lsn()::text")"
wait_replay slave1_estale_delete "${SLAVE1_PORT}" "${DELETE_LSN}"
touch "${STALE_CONTROL_DIR}/read"
wait "${STALE_READER_PID}"
STALE_READER_PID=""
ESTALE_RESULT="$(cat "${STALE_CONTROL_DIR}/result")"
[[ "${ESTALE_RESULT}" == "ESTALE" ]] || {
    echo "Opened degraded read returned ${ESTALE_RESULT}, expected ESTALE" >&2
    exit 1
}
printf 'degraded_open_file_missing_snapshot_errno=%s file_id=%s\n' \
    "${ESTALE_RESULT}" "${DELETED_FILE_ID}" | tee -a "${ARTIFACT_DIR}/topology.txt"
rm -rf "${STALE_CONTROL_DIR}"

echo "=== Fence old master and promote slave1 ==="
# Stopping the isolated primary container is the fencing action in this test.
compose stop master
if psql_node "${MASTER_PORT}" -c "SELECT 1" >/dev/null 2>&1; then
    echo "Old master is still reachable after fencing" >&2
    exit 1
fi
compose exec -T --user postgres slave1 \
    pg_ctl -D /var/lib/postgresql/data/pgdata promote -w
wait_sql slave1_promoted "${SLAVE1_PORT}" "SELECT NOT pg_is_in_recovery()" t

ROLE_CHANGE_DETECTED=0
for _ in $(seq 1 "${WAIT_SECONDS}"); do
    if grep -Fq "authority=127.0.0.1:${SLAVE1_PORT} configured_role=replica observed_role=primary-writable" \
        "${LOG_FILE}"; then
        ROLE_CHANGE_DETECTED=1
        break
    fi
    sleep 1
done
((ROLE_CHANGE_DETECTED == 1)) || {
    echo "Degraded mount did not report slave1 promotion" >&2
    exit 1
}
mountpoint -q "${MOUNTPOINT}" || {
    echo "Degraded mount exited during PostgreSQL role change" >&2
    exit 1
}
if (printf 'must remain read only\n' >"${MOUNTPOINT}/role-monitor-write-rejected.txt") 2>/dev/null; then
    echo "Degraded mount admitted a write after slave1 promotion" >&2
    exit 1
fi
printf 'role_change_detected=1\nrole_change_write_guard=read_only\n' \
    | tee -a "${ARTIFACT_DIR}/topology.txt"
fod_test_cleanup
MOUNTPOINT="" LOG_FILE=""
unset FOD_DEGRADED_READONLY_ENABLED
export FOD_ROLE=primary

PROMOTED_ID="$(psql_container_admin slave1 \
    "SELECT system_identifier FROM pg_control_system()")"
psql_container_admin slave1 "CHECKPOINT" >/dev/null
PROMOTED_TIMELINE="$(psql_container_admin slave1 \
    "SELECT timeline_id FROM pg_control_checkpoint()")"
[[ "${PROMOTED_ID}" == "${INITIAL_ID}" ]] || {
    echo "Promoted slave1 system identifier changed" >&2
    exit 1
}
((PROMOTED_TIMELINE > INITIAL_TIMELINE)) || {
    echo "Promoted timeline did not advance: initial=${INITIAL_TIMELINE} promoted=${PROMOTED_TIMELINE}" >&2
    exit 1
}
printf 'promoted_system_identifier=%s\npromoted_timeline=%s\n' \
    "${PROMOTED_ID}" "${PROMOTED_TIMELINE}" | tee -a "${ARTIFACT_DIR}/topology.txt"

echo "=== Repoint slave2 to slave1 ==="
compose exec -T --user postgres slave2 sh -ec \
    'printf "%s\n" "primary_conninfo = '\''host=slave1 port=5432 user=fod_repl password=fod_repl_pass application_name=fod_ha_chain_slave2_reparented'\''" >> "$PGDATA/postgresql.auto.conf"; pg_ctl -D "$PGDATA" reload'
wait_sql slave2_still_standby "${SLAVE2_PORT}" "SELECT pg_is_in_recovery()" t
UPSTREAM=""
for _ in $(seq 1 "${WAIT_SECONDS}"); do
    UPSTREAM="$(psql_container_admin slave2 \
        "SELECT COALESCE(sender_host, '') FROM pg_stat_wal_receiver" 2>/dev/null || true)"
    UPSTREAM="$(printf '%s' "${UPSTREAM}" | tr -d '[:space:]')"
    [[ "${UPSTREAM}" == slave1 ]] && break
    sleep 1
done
[[ "${UPSTREAM}" == slave1 ]] || {
    echo "slave2 did not reconnect to slave1; sender_host=${UPSTREAM:-none}" >&2
    exit 1
}
printf 'slave2_upstream=%s\n' "${UPSTREAM}" | tee -a "${ARTIFACT_DIR}/topology.txt"

echo "=== Pause slave2 replay to test replica-lag fallback ==="
psql_container_admin slave2 "SELECT pg_wal_replay_pause()" >/dev/null
wait_sql slave2_replay_paused "${SLAVE2_PORT}" \
    "SELECT pg_is_wal_replay_paused()" t

echo "=== FOD client uses slave1 authority and slave2 replica ==="
export FOD_PG_PORT="${SLAVE1_PORT}"
export FOD_PG_PRIMARY_HOSTS="127.0.0.1:${SLAVE1_PORT}"
export FOD_PG_REPLICA_HOSTS="127.0.0.1:${SLAVE2_PORT}"
export FOD_PG_REPLICA_READ_ROUTING_ENABLED=1
fod_test_make_mountpoint "/tmp/fod-ha-chain-after"
FOD_TEST_LOG_ARCHIVE="${ARTIFACT_DIR}/fod-reparented-mount.log"
fod_test_start_mount "${MOUNTPOINT}"
[[ "$(cat "${MOUNTPOINT}/ha-chain-marker.txt")" == "marker written through original master" ]] || {
    echo "FOD could not read the pre-promotion marker after reparenting" >&2
    exit 1
}
printf 'marker written through promoted slave1\n' >"${MOUNTPOINT}/ha-chain-marker-after.txt"
sync
POST_PROMOTION_LSN="$(psql_node "${SLAVE1_PORT}" -c \
    "SELECT pg_current_wal_flush_lsn()::text")"
printf 'post_promotion_flush_lsn=%s\n' "${POST_PROMOTION_LSN}" \
    | tee -a "${ARTIFACT_DIR}/topology.txt"
EXPECTED_MARKER_SHA="$(printf 'marker written through promoted slave1\n' | sha256sum | awk '{print $1}')"
ACTUAL_MARKER_SHA="$(sha256sum "${MOUNTPOINT}/ha-chain-marker-after.txt" | awk '{print $1}')"
[[ "${ACTUAL_MARKER_SHA}" == "${EXPECTED_MARKER_SHA}" ]] || {
    echo "FOD stale-read fallback returned the wrong marker hash" >&2
    exit 1
}
printf 'lag_fallback_marker_sha256=%s\n' "${ACTUAL_MARKER_SHA}" \
    | tee -a "${ARTIFACT_DIR}/topology.txt"
psql_container_admin slave2 "SELECT pg_wal_replay_resume()" >/dev/null
wait_replay slave2_after_reparent "${SLAVE2_PORT}" "${POST_PROMOTION_LSN}"
ACTUAL_REPLAYED_MARKER_SHA="$(sha256sum "${MOUNTPOINT}/ha-chain-marker-after.txt" | awk '{print $1}')"
[[ "${ACTUAL_REPLAYED_MARKER_SHA}" == "${EXPECTED_MARKER_SHA}" ]] || {
    echo "FOD replica read returned the wrong post-replay marker hash" >&2
    exit 1
}
printf 'post_replay_marker_sha256=%s\n' "${ACTUAL_REPLAYED_MARKER_SHA}" \
    | tee -a "${ARTIFACT_DIR}/topology.txt"
fod_test_cleanup
MOUNTPOINT="" LOG_FILE=""

FOD_LOG="${ARTIFACT_DIR}/fod-reparented-mount.log"
grep -Fq "selected_authority=127.0.0.1:${SLAVE1_PORT}" "${FOD_LOG}" || {
    echo "FOD did not select promoted slave1 as writable authority" >&2
    exit 1
}
grep -Fq "replica_active_authority=127.0.0.1:${SLAVE2_PORT}" "${FOD_LOG}" || {
    echo "FOD did not configure slave2 as the replica read target" >&2
    exit 1
}
FINAL_LANE_LINE="$(grep -F 'stage=post-mount lane=shared' "${FOD_LOG}" | tail -n 1)"
REPLICA_READS="$(sed -n 's/.* replica_reads=\([0-9][0-9]*\) .*/\1/p' \
    <<<"${FINAL_LANE_LINE}")"
OPERATION_FAILURES="$(sed -n 's/.* operation_failures=\([0-9][0-9]*\) .*/\1/p' \
    <<<"${FINAL_LANE_LINE}")"
LAG_FALLBACKS="$(sed -n 's/.* replica_lag_fallbacks=\([0-9][0-9]*\) .*/\1/p' \
    <<<"${FINAL_LANE_LINE}")"
if [[ ! "${REPLICA_READS}" =~ ^[0-9]+$ ]] || ((REPLICA_READS == 0)); then
    echo "FOD did not complete a read on slave2" >&2
    exit 1
fi
if [[ ! "${OPERATION_FAILURES}" =~ ^[0-9]+$ ]] || ((OPERATION_FAILURES != 0)); then
    echo "FOD reported PostgreSQL operation failures: ${OPERATION_FAILURES:-missing}" >&2
    exit 1
fi
if [[ ! "${LAG_FALLBACKS}" =~ ^[0-9]+$ ]] || ((LAG_FALLBACKS == 0)); then
    echo "FOD did not record a replica lag fallback while slave2 replay was paused" >&2
    exit 1
fi
printf 'fod_replica_reads=%s\nfod_lag_fallbacks=%s\nfod_operation_failures=%s\n' \
    "${REPLICA_READS}" "${LAG_FALLBACKS}" "${OPERATION_FAILURES}" \
    | tee -a "${ARTIFACT_DIR}/topology.txt"

[[ "$(psql_node "${SLAVE2_PORT}" -c \
    "SELECT pg_is_in_recovery() AND EXISTS (SELECT 1 FROM fod.files WHERE name = 'ha-chain-marker-after.txt')")" == t ]] || {
    echo "slave2 did not replay the FOD marker created on slave1" >&2
    exit 1
}

echo "=== Restart old master to simulate split brain ==="
compose start master
wait_sql old_master_is_writable "${MASTER_PORT}" "SELECT NOT pg_is_in_recovery()" t
export FOD_PG_PRIMARY_HOSTS="127.0.0.1:${MASTER_PORT},127.0.0.1:${SLAVE1_PORT}"
fod_test_make_mountpoint "/tmp/fod-ha-chain-split-brain"
FOD_TEST_LOG_ARCHIVE="${ARTIFACT_DIR}/fod-split-brain-rejection.log"
if fod_test_start_mount "${MOUNTPOINT}"; then
    fod_test_cleanup
    MOUNTPOINT="" LOG_FILE=""
    echo "FOD accepted a mount while two writable PostgreSQL authorities were reachable" >&2
    exit 1
fi
fod_test_cleanup
MOUNTPOINT="" LOG_FILE=""
grep -Fq "multiple writable PostgreSQL endpoints" \
    "${ARTIFACT_DIR}/fod-split-brain-rejection.log" || {
    echo "FOD failed closed for an unexpected reason during split-brain simulation" >&2
    exit 1
}
echo "split_brain_mount=rejected"
echo "split_brain_mount=rejected" >>"${ARTIFACT_DIR}/topology.txt"

echo "three_node_failover=passed"
echo "old_master_fenced=container_stopped"
echo "slave1_role=primary"
echo "slave2_upstream=slave1"
echo "fod_authority=slave1"
echo "artifacts=${ARTIFACT_DIR}"
