#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Storage-block candidate benchmark that targets three CLEAN measurements per
# candidate instead of three total attempts. Runs classified as storage/WAL
# stalls are retained in artifacts but retried. Before every measured attempt
# the host must pass Dirty/Writeback and same-filesystem fsync preflight checks.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

STORAGE_BLOCK_SIZES="${FOD_STORAGE_CLEAN_BLOCK_SIZES:-16384 32768 65536}"
TARGET_CLEAN_RUNS="${FOD_STORAGE_CLEAN_RUNS:-3}"
MAX_ATTEMPTS_PER_SIZE="${FOD_STORAGE_MAX_ATTEMPTS_PER_SIZE:-7}"
FILE_SIZE="${FOD_STORAGE_CLEAN_FILE_SIZE:-1G}"
FIO_BLOCK_SIZE="${FOD_STORAGE_CLEAN_FIO_BLOCK_SIZE:-512k}"
PAYLOAD_MODE="${FOD_STORAGE_CLEAN_PAYLOAD_MODE:-random}"
RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}"
CARGO_PROFILE="${FOD_CARGO_PROFILE:-${RUNTIME_PROFILE}}"
DIRTY_LIMIT_KB="${FOD_STORAGE_CLEAN_DIRTY_LIMIT_KB:-8192}"
SETTLE_TIMEOUT_SECONDS="${FOD_STORAGE_CLEAN_SETTLE_TIMEOUT_SECONDS:-300}"
SETTLE_POLL_SECONDS="${FOD_STORAGE_CLEAN_SETTLE_POLL_SECONDS:-2}"
COOLDOWN_SECONDS="${FOD_STORAGE_CLEAN_COOLDOWN_SECONDS:-20}"
FSYNC_PROBE_COUNT="${FOD_STORAGE_CLEAN_FSYNC_PROBE_COUNT:-8}"
FSYNC_MEDIAN_LIMIT_MS="${FOD_STORAGE_CLEAN_FSYNC_MEDIAN_LIMIT_MS:-20}"
FSYNC_MAX_LIMIT_MS="${FOD_STORAGE_CLEAN_FSYNC_MAX_LIMIT_MS:-100}"
WAL_STALL_MS="${FOD_STORAGE_CLEAN_WAL_STALL_MS:-5000}"
PG_IO_STALL_MS="${FOD_STORAGE_CLEAN_PG_IO_STALL_MS:-7000}"
MAX_CLEAN_SPREAD_PCT="${FOD_STORAGE_CLEAN_MAX_SPREAD_PCT:-25}"
MATRIX="${ROOT}/scripts/perf/run_random_storage_block_matrix.sh"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${FOD_STORAGE_CLEAN_ARTIFACT_DIR:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-storage-block-clean-stability-${RUN_ID}}"

for cmd in bash make git awk sort tee sync sleep date python3; do
    command -v "${cmd}" >/dev/null 2>&1 || {
        echo "Missing required command: ${cmd}" >&2
        exit 2
    }
done

positive_integer() {
    local name="$1"
    local value="$2"
    case "${value}" in
        ''|*[!0-9]*) echo "${name} must be a positive integer" >&2; exit 2 ;;
    esac
    if (( value < 1 )); then
        echo "${name} must be >= 1" >&2
        exit 2
    fi
}

positive_integer FOD_STORAGE_CLEAN_RUNS "${TARGET_CLEAN_RUNS}"
positive_integer FOD_STORAGE_MAX_ATTEMPTS_PER_SIZE "${MAX_ATTEMPTS_PER_SIZE}"
positive_integer FOD_STORAGE_CLEAN_DIRTY_LIMIT_KB "${DIRTY_LIMIT_KB}"
positive_integer FOD_STORAGE_CLEAN_SETTLE_TIMEOUT_SECONDS "${SETTLE_TIMEOUT_SECONDS}"
positive_integer FOD_STORAGE_CLEAN_SETTLE_POLL_SECONDS "${SETTLE_POLL_SECONDS}"
positive_integer FOD_STORAGE_CLEAN_COOLDOWN_SECONDS "${COOLDOWN_SECONDS}"
positive_integer FOD_STORAGE_CLEAN_FSYNC_PROBE_COUNT "${FSYNC_PROBE_COUNT}"
positive_integer FOD_STORAGE_CLEAN_FSYNC_MEDIAN_LIMIT_MS "${FSYNC_MEDIAN_LIMIT_MS}"
positive_integer FOD_STORAGE_CLEAN_FSYNC_MAX_LIMIT_MS "${FSYNC_MAX_LIMIT_MS}"
positive_integer FOD_STORAGE_CLEAN_WAL_STALL_MS "${WAL_STALL_MS}"
positive_integer FOD_STORAGE_CLEAN_PG_IO_STALL_MS "${PG_IO_STALL_MS}"
positive_integer FOD_STORAGE_CLEAN_MAX_SPREAD_PCT "${MAX_CLEAN_SPREAD_PCT}"

if (( MAX_ATTEMPTS_PER_SIZE < TARGET_CLEAN_RUNS )); then
    echo "FOD_STORAGE_MAX_ATTEMPTS_PER_SIZE must be >= FOD_STORAGE_CLEAN_RUNS" >&2
    exit 2
fi
if [[ "${PAYLOAD_MODE}" != "random" ]]; then
    echo "Clean stability benchmark requires random payloads" >&2
    exit 2
fi
if [[ ! -r /proc/meminfo ]]; then
    echo "/proc/meminfo is required" >&2
    exit 2
fi

read -r -a BLOCK_SIZES <<<"${STORAGE_BLOCK_SIZES}"
for block_size in "${BLOCK_SIZES[@]}"; do
    case "${block_size}" in
        ''|*[!0-9]*) echo "Invalid storage block size: ${block_size}" >&2; exit 2 ;;
    esac
    if (( block_size < 1024 || block_size % 1024 != 0 )); then
        echo "Storage block size must be a positive multiple of 1024: ${block_size}" >&2
        exit 2
    fi
done

mkdir -p "${ARTIFACT_DIR}"
RUNS="${ARTIFACT_DIR}/runs.tsv"
MEDIANS="${ARTIFACT_DIR}/median.tsv"
SETTLE_LOG="${ARTIFACT_DIR}/settle.tsv"
PROBE_FILE="${ARTIFACT_DIR}/.fsync-probe"

printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "cycle" "attempt" "order" "storage_block_size" \
    "primary_write_mib_s" "primary_read_mib_s" "replica_read_mib_s" \
    "copy_calls" "copy_rows" "copy_exec_ms" "copy_mean_ms" \
    "insert_calls" "insert_rows" "insert_exec_ms" "insert_mean_ms" "sql_flush_mean_ms" \
    "insert_wal_bytes" "wal_bytes_delta" "wal_buffers_full_delta" \
    "wal_write_delta" "wal_sync_delta" "wal_write_time_delta_ms" "wal_sync_time_delta_ms" \
    "pg_io_wait_delta_ms" "client_write_time_delta_ms" "client_extend_time_delta_ms" \
    "background_writeback_time_delta_ms" "checkpointer_fsync_time_delta_ms" \
    "pre_dirty_kb" "pre_writeback_kb" "pre_fsync_median_ms" "pre_fsync_max_ms" \
    "post_dirty_kb" "post_writeback_kb" "run_quality" "profile_artifact_dir" >"${RUNS}"
printf 'timestamp\tlabel\tdirty_kb\twriteback_kb\ttotal_kb\tfsync_median_ms\tfsync_max_ms\tstatus\n' >"${SETTLE_LOG}"

printf '=== FOD CLEAN STORAGE BLOCK STABILITY MATRIX ===\n'
printf 'storage_block_sizes=%s\ntarget_clean_runs=%s\nmax_attempts_per_size=%s\n' \
    "${STORAGE_BLOCK_SIZES}" "${TARGET_CLEAN_RUNS}" "${MAX_ATTEMPTS_PER_SIZE}"
printf 'file_size=%s\nfio_block_size=%s\npayload_mode=%s\n' "${FILE_SIZE}" "${FIO_BLOCK_SIZE}" "${PAYLOAD_MODE}"
printf 'dirty_limit_kb=%s cooldown_s=%s fsync_median_limit_ms=%s fsync_max_limit_ms=%s\n' \
    "${DIRTY_LIMIT_KB}" "${COOLDOWN_SECONDS}" "${FSYNC_MEDIAN_LIMIT_MS}" "${FSYNC_MAX_LIMIT_MS}"
printf 'wal_stall_ms=%s pg_io_stall_ms=%s artifact_dir=%s\n' "${WAL_STALL_MS}" "${PG_IO_STALL_MS}" "${ARTIFACT_DIR}"

meminfo_kb() {
    local key="$1"
    awk -v key="${key}:" '$1 == key {print $2 + 0; found=1; exit} END {if (!found) print 0}' /proc/meminfo
}

fsync_probe() {
    python3 - "${PROBE_FILE}" "${FSYNC_PROBE_COUNT}" <<'PY'
import os
import statistics
import sys
import time

path = sys.argv[1]
count = int(sys.argv[2])
payload = os.urandom(4096)
values = []
fd = os.open(path, os.O_CREAT | os.O_TRUNC | os.O_WRONLY, 0o600)
try:
    for _ in range(count):
        os.write(fd, payload)
        started = time.perf_counter_ns()
        os.fsync(fd)
        values.append((time.perf_counter_ns() - started) / 1_000_000.0)
finally:
    os.close(fd)
    try:
        os.unlink(path)
    except FileNotFoundError:
        pass
print(f"{statistics.median(values):.3f}\t{max(values):.3f}")
PY
}

SETTLE_DIRTY=0
SETTLE_WRITEBACK=0
SETTLE_FSYNC_MEDIAN=0
SETTLE_FSYNC_MAX=0

settle_host() {
    local label="$1"
    local started now elapsed dirty writeback total probe median max
    echo "Host settle start label=${label}"
    sync
    started="$(date +%s)"
    while true; do
        dirty="$(meminfo_kb Dirty)"
        writeback="$(meminfo_kb Writeback)"
        total=$(( dirty + writeback ))
        if (( total <= DIRTY_LIMIT_KB )); then
            sleep "${COOLDOWN_SECONDS}"
            dirty="$(meminfo_kb Dirty)"
            writeback="$(meminfo_kb Writeback)"
            total=$(( dirty + writeback ))
            read -r median max < <(fsync_probe)
            probe="probe"
            if (( total <= DIRTY_LIMIT_KB )) \
                && awk -v value="${median}" -v limit="${FSYNC_MEDIAN_LIMIT_MS}" 'BEGIN {exit !((value + 0) <= (limit + 0))}' \
                && awk -v value="${max}" -v limit="${FSYNC_MAX_LIMIT_MS}" 'BEGIN {exit !((value + 0) <= (limit + 0))}'; then
                printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
                    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${label}" "${dirty}" "${writeback}" "${total}" "${median}" "${max}" "ready" >>"${SETTLE_LOG}"
                SETTLE_DIRTY="${dirty}"
                SETTLE_WRITEBACK="${writeback}"
                SETTLE_FSYNC_MEDIAN="${median}"
                SETTLE_FSYNC_MAX="${max}"
                echo "Host settle OK dirty_kb=${dirty} writeback_kb=${writeback} fsync_median_ms=${median} fsync_max_ms=${max}"
                return 0
            fi
            printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
                "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${label}" "${dirty}" "${writeback}" "${total}" "${median}" "${max}" "retry" >>"${SETTLE_LOG}"
        else
            printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
                "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${label}" "${dirty}" "${writeback}" "${total}" "-" "-" "dirty" >>"${SETTLE_LOG}"
        fi
        now="$(date +%s)"
        elapsed=$(( now - started ))
        if (( elapsed >= SETTLE_TIMEOUT_SECONDS )); then
            echo "Host did not reach a clean I/O baseline within ${SETTLE_TIMEOUT_SECONDS}s label=${label}" >&2
            return 1
        fi
        sleep "${SETTLE_POLL_SECONDS}"
    done
}

tsv_value() {
    awk -F '\t' -v column="$2" 'NR == 2 {print $column}' "$1"
}

ratio_ms() {
    awk -v total="$1" -v calls="$2" 'BEGIN {if ((calls + 0) > 0) printf "%.3f", (total + 0)/(calls + 0); else print "0"}'
}

sum_values() {
    awk 'BEGIN {s=0; for (i=1; i<ARGC; i++) s += ARGV[i] + 0; printf "%.3f", s}' "$@"
}

wal_delta() {
    local file="$1" column="$2" format="${3:-float}"
    if [[ ! -s "${file}" ]]; then echo 0; return; fi
    if [[ "${format}" == "int" ]]; then
        awk -F '\t' -v c="${column}" 'NR==1{f=$c}{l=$c}END{printf "%.0f",(l+0)-(f+0)}' "${file}"
    else
        awk -F '\t' -v c="${column}" 'NR==1{f=$c}{l=$c}END{printf "%.3f",(l+0)-(f+0)}' "${file}"
    fi
}

io_delta() {
    local file="$1" backend="$2" column="$3"
    if [[ ! -s "${file}" ]]; then echo 0; return; fi
    awk -F '\t' -v backend="${backend}" -v c="${column}" '
        $2 == backend {if (!seen) {first=$c; seen=1}; last=$c}
        END {if (seen) printf "%.3f", (last+0)-(first+0); else print "0"}
    ' "${file}"
}

values_for() {
    local size="$1" column="$2" quality="${3:-clean}"
    awk -F '\t' -v size="${size}" -v column="${column}" -v quality="${quality}" \
        'NR>1 && $4==size && (quality=="all" || $35==quality) {print $column}' "${RUNS}"
}

median_for() {
    values_for "$1" "$2" "${3:-clean}" | sort -n | awk '{v[NR]=$1} END {if(NR==0){print 0}else if(NR%2){printf "%.3f",v[(NR+1)/2]}else{printf "%.3f",(v[NR/2]+v[NR/2+1])/2}}'
}

min_for() {
    values_for "$1" "$2" "${3:-clean}" | awk '{if(!s||$1<m)m=$1;s=1}END{if(s)printf "%.3f",m;else print 0}'
}

max_for() {
    values_for "$1" "$2" "${3:-clean}" | awk '{if(!s||$1>m)m=$1;s=1}END{if(s)printf "%.3f",m;else print 0}'
}

count_for() {
    local size="$1" quality="${2:-clean}"
    awk -F '\t' -v size="${size}" -v quality="${quality}" 'NR>1 && $4==size && (quality=="all" || $35==quality){n++}END{print n+0}' "${RUNS}"
}

# Build once, outside every measured attempt.
FOD_CARGO_PROFILE="${CARGO_PROFILE}" FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" make --no-print-directory build-runtime

declare -A ATTEMPTS CLEAN_COUNTS
for block_size in "${BLOCK_SIZES[@]}"; do
    ATTEMPTS["${block_size}"]=0
    CLEAN_COUNTS["${block_size}"]=0
done

cycle=1
size_count=${#BLOCK_SIZES[@]}
while true; do
    pending=0
    runnable=0
    for block_size in "${BLOCK_SIZES[@]}"; do
        if (( CLEAN_COUNTS["${block_size}"] < TARGET_CLEAN_RUNS )); then
            pending=1
            if (( ATTEMPTS["${block_size}"] < MAX_ATTEMPTS_PER_SIZE )); then
                runnable=1
            fi
        fi
    done
    (( pending == 0 )) && break
    (( runnable == 0 )) && break

    rotation=$(( (cycle - 1) % size_count ))
    echo
    echo "=== CLEAN CYCLE ${cycle} rotation=${rotation} ==="
    for (( offset=0; offset<size_count; offset++ )); do
        index=$(( (rotation + offset) % size_count ))
        block_size="${BLOCK_SIZES[$index]}"
        if (( CLEAN_COUNTS["${block_size}"] >= TARGET_CLEAN_RUNS )); then
            continue
        fi
        if (( ATTEMPTS["${block_size}"] >= MAX_ATTEMPTS_PER_SIZE )); then
            continue
        fi

        ATTEMPTS["${block_size}"]=$(( ATTEMPTS["${block_size}"] + 1 ))
        attempt="${ATTEMPTS["${block_size}"]}"
        order=$(( offset + 1 ))
        run_dir="${ARTIFACT_DIR}/cycle-${cycle}/block-${block_size}-attempt-${attempt}"
        run_log="${run_dir}/run.log"
        mkdir -p "${run_dir}"

        echo
        echo "--- cycle=${cycle} order=${order} block_size=${block_size} attempt=${attempt} clean=${CLEAN_COUNTS["${block_size}"]}/${TARGET_CLEAN_RUNS} ---"
        settle_host "c${cycle}-o${order}-b${block_size}-a${attempt}"
        pre_dirty_kb="${SETTLE_DIRTY}"
        pre_writeback_kb="${SETTLE_WRITEBACK}"
        pre_fsync_median_ms="${SETTLE_FSYNC_MEDIAN}"
        pre_fsync_max_ms="${SETTLE_FSYNC_MAX}"

        set +e
        FOD_CARGO_PROFILE="${CARGO_PROFILE}" \
        FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" \
        FOD_STORAGE_BLOCK_SKIP_BUILD=1 \
        FOD_STORAGE_BLOCK_SIZES="${block_size}" \
        FOD_STORAGE_BLOCK_FILE_SIZE="${FILE_SIZE}" \
        FOD_STORAGE_BLOCK_FIO_BLOCK_SIZE="${FIO_BLOCK_SIZE}" \
        FOD_STORAGE_BLOCK_PAYLOAD_MODE="${PAYLOAD_MODE}" \
        FOD_STORAGE_BLOCK_ARTIFACT_DIR="${run_dir}" \
        FOD_REQUIRE_AC_POWER="${FOD_REQUIRE_AC_POWER:-1}" \
            bash "${MATRIX}" 2>&1 | tee "${run_log}"
        run_status=${PIPESTATUS[0]}
        set -e
        post_dirty_kb="$(meminfo_kb Dirty)"
        post_writeback_kb="$(meminfo_kb Writeback)"
        if [[ "${run_status}" -ne 0 ]]; then
            echo "Benchmark failed block_size=${block_size} attempt=${attempt}" >&2
            exit "${run_status}"
        fi

        summary="${run_dir}/summary.tsv"
        primary_write="$(tsv_value "${summary}" 5)"
        primary_read="$(tsv_value "${summary}" 6)"
        replica_read="$(tsv_value "${summary}" 7)"
        copy_calls="$(tsv_value "${summary}" 8)"
        copy_rows="$(tsv_value "${summary}" 9)"
        copy_exec_ms="$(tsv_value "${summary}" 10)"
        insert_calls="$(tsv_value "${summary}" 12)"
        insert_rows="$(tsv_value "${summary}" 13)"
        insert_exec_ms="$(tsv_value "${summary}" 14)"
        insert_wal_bytes="$(tsv_value "${summary}" 16)"
        profile_dir="$(tsv_value "${summary}" 17)"
        copy_mean_ms="$(ratio_ms "${copy_exec_ms}" "${copy_calls}")"
        insert_mean_ms="$(ratio_ms "${insert_exec_ms}" "${insert_calls}")"
        sql_flush_mean_ms="$(sum_values "${copy_mean_ms}" "${insert_mean_ms}")"

        wal_file="${profile_dir}/wal.tsv"
        io_file="${profile_dir}/io.tsv"
        wal_bytes_delta="$(wal_delta "${wal_file}" 4 int)"
        wal_buffers_full_delta="$(wal_delta "${wal_file}" 5 int)"
        wal_write_delta="$(wal_delta "${wal_file}" 6 int)"
        wal_sync_delta="$(wal_delta "${wal_file}" 7 int)"
        wal_write_time_delta_ms="$(wal_delta "${wal_file}" 8)"
        wal_sync_time_delta_ms="$(wal_delta "${wal_file}" 9)"

        client_write_time_delta_ms="$(io_delta "${io_file}" "client backend" 6)"
        client_extend_time_delta_ms="$(io_delta "${io_file}" "client backend" 10)"
        background_writeback_time_delta_ms="$(io_delta "${io_file}" "background writer" 8)"
        checkpointer_fsync_time_delta_ms="$(io_delta "${io_file}" "checkpointer" 13)"
        checkpointer_write_time_delta_ms="$(io_delta "${io_file}" "checkpointer" 6)"
        background_write_time_delta_ms="$(io_delta "${io_file}" "background writer" 6)"
        pg_io_wait_delta_ms="$(sum_values \
            "${client_write_time_delta_ms}" "${client_extend_time_delta_ms}" \
            "${background_writeback_time_delta_ms}" "${checkpointer_fsync_time_delta_ms}" \
            "${checkpointer_write_time_delta_ms}" "${background_write_time_delta_ms}")"

        run_quality="clean"
        if awk -v v="${wal_sync_time_delta_ms}" -v l="${WAL_STALL_MS}" 'BEGIN{exit !((v+0)>(l+0))}'; then
            run_quality="wal_stall"
        elif awk -v v="${pg_io_wait_delta_ms}" -v l="${PG_IO_STALL_MS}" 'BEGIN{exit !((v+0)>(l+0))}'; then
            run_quality="io_stall"
        fi

        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "${cycle}" "${attempt}" "${order}" "${block_size}" \
            "${primary_write}" "${primary_read}" "${replica_read}" \
            "${copy_calls}" "${copy_rows}" "${copy_exec_ms}" "${copy_mean_ms}" \
            "${insert_calls}" "${insert_rows}" "${insert_exec_ms}" "${insert_mean_ms}" "${sql_flush_mean_ms}" \
            "${insert_wal_bytes}" "${wal_bytes_delta}" "${wal_buffers_full_delta}" \
            "${wal_write_delta}" "${wal_sync_delta}" "${wal_write_time_delta_ms}" "${wal_sync_time_delta_ms}" \
            "${pg_io_wait_delta_ms}" "${client_write_time_delta_ms}" "${client_extend_time_delta_ms}" \
            "${background_writeback_time_delta_ms}" "${checkpointer_fsync_time_delta_ms}" \
            "${pre_dirty_kb}" "${pre_writeback_kb}" "${pre_fsync_median_ms}" "${pre_fsync_max_ms}" \
            "${post_dirty_kb}" "${post_writeback_kb}" "${run_quality}" "${profile_dir}" >>"${RUNS}"

        if [[ "${run_quality}" == "clean" ]]; then
            CLEAN_COUNTS["${block_size}"]=$(( CLEAN_COUNTS["${block_size}"] + 1 ))
        fi
        echo "run_quality=${run_quality} block_size=${block_size} write_mib_s=${primary_write} wal_sync_ms=${wal_sync_time_delta_ms} pg_io_wait_ms=${pg_io_wait_delta_ms} clean=${CLEAN_COUNTS["${block_size}"]}/${TARGET_CLEAN_RUNS}"
    done
    cycle=$(( cycle + 1 ))
done

printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "storage_block_size" "attempts" "clean_runs" "stall_runs" \
    "median_primary_write_mib_s_clean" "min_primary_write_mib_s_clean" "max_primary_write_mib_s_clean" "clean_write_spread_pct" \
    "median_primary_read_mib_s_clean" "median_replica_read_mib_s_clean" \
    "median_copy_mean_ms_clean" "median_insert_mean_ms_clean" "median_sql_flush_mean_ms_clean" \
    "median_wal_bytes_delta_clean" "median_wal_sync_time_delta_ms_clean" "median_pg_io_wait_delta_ms_clean" \
    "max_wal_sync_time_delta_ms_all" "max_pg_io_wait_delta_ms_all" "status" >"${MEDIANS}"

selection_status="valid"
for block_size in "${BLOCK_SIZES[@]}"; do
    attempts="$(count_for "${block_size}" all)"
    clean_runs="$(count_for "${block_size}" clean)"
    stall_runs=$(( attempts - clean_runs ))
    median_write="$(median_for "${block_size}" 5 clean)"
    min_write="$(min_for "${block_size}" 5 clean)"
    max_write="$(max_for "${block_size}" 5 clean)"
    spread="$(awk -v min="${min_write}" -v max="${max_write}" -v med="${median_write}" 'BEGIN{if((med+0)>0)printf "%.2f",((max+0)-(min+0))*100/(med+0);else print 0}')"
    status="OK"
    if (( clean_runs < TARGET_CLEAN_RUNS )); then
        status="INSUFFICIENT_CLEAN_RUNS"
        selection_status="invalid"
    elif awk -v v="${spread}" -v l="${MAX_CLEAN_SPREAD_PCT}" 'BEGIN{exit !((v+0)>(l+0))}'; then
        status="UNSTABLE_CLEAN_RUNS"
        selection_status="invalid"
    fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "${block_size}" "${attempts}" "${clean_runs}" "${stall_runs}" \
        "${median_write}" "${min_write}" "${max_write}" "${spread}" \
        "$(median_for "${block_size}" 6 clean)" "$(median_for "${block_size}" 7 clean)" \
        "$(median_for "${block_size}" 11 clean)" "$(median_for "${block_size}" 15 clean)" "$(median_for "${block_size}" 16 clean)" \
        "$(median_for "${block_size}" 18 clean)" "$(median_for "${block_size}" 23 clean)" "$(median_for "${block_size}" 24 clean)" \
        "$(max_for "${block_size}" 23 all)" "$(max_for "${block_size}" 24 all)" "${status}" >>"${MEDIANS}"
done

best_size=""
best_write=""
if [[ "${selection_status}" == "valid" ]]; then
    best_size="$(awk -F '\t' 'NR>1{if(!s||$5>b){b=$5;size=$1;s=1}}END{print size}' "${MEDIANS}")"
    best_write="$(awk -F '\t' -v size="${best_size}" '$1==size{print $5}' "${MEDIANS}")"
fi

echo
echo "=== CLEAN STORAGE BLOCK RUNS ==="
cat "${RUNS}"
echo
echo "=== CLEAN STORAGE BLOCK MEDIANS ==="
cat "${MEDIANS}"
echo "selection_status=${selection_status}"
if [[ "${selection_status}" == "valid" ]]; then
    echo "best_clean_median_primary_write_block_size=${best_size}"
    echo "best_clean_median_primary_write_mib_s=${best_write}"
else
    echo "No block size selected: clean-run target or stability criterion was not met."
fi
echo "settle_log=${SETTLE_LOG}"
echo "stability_artifact_dir=${ARTIFACT_DIR}"
echo "OK: clean storage block stability matrix"
