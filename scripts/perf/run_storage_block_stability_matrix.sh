#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Repeated stability benchmark for storage block-size candidates. Each measured
# run gets a fresh PostgreSQL primary/replica lab and its own PostgreSQL profile.
# Before every run the host is synchronized and must reach a low Dirty+Writeback
# baseline, so delayed writeback from a previous run cannot dominate WAL fsync.
# Candidate order is rotated between rounds:
#   round 1: 16K, 32K, 64K
#   round 2: 32K, 64K, 16K
#   round 3: 64K, 16K, 32K

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

STORAGE_BLOCK_SIZES="${FOD_STORAGE_STABILITY_BLOCK_SIZES:-16384 32768 65536}"
REPEATS="${FOD_STORAGE_STABILITY_REPEATS:-3}"
FILE_SIZE="${FOD_STORAGE_STABILITY_FILE_SIZE:-1G}"
FIO_BLOCK_SIZE="${FOD_STORAGE_STABILITY_FIO_BLOCK_SIZE:-512k}"
PAYLOAD_MODE="${FOD_STORAGE_STABILITY_PAYLOAD_MODE:-random}"
RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}"
CARGO_PROFILE="${FOD_CARGO_PROFILE:-${RUNTIME_PROFILE}}"
SETTLE_ENABLED="${FOD_STORAGE_STABILITY_SETTLE:-1}"
SETTLE_DIRTY_KB="${FOD_STORAGE_STABILITY_SETTLE_DIRTY_KB:-32768}"
SETTLE_TIMEOUT_SECONDS="${FOD_STORAGE_STABILITY_SETTLE_TIMEOUT_SECONDS:-180}"
SETTLE_POLL_SECONDS="${FOD_STORAGE_STABILITY_SETTLE_POLL_SECONDS:-1}"
SETTLE_IDLE_SECONDS="${FOD_STORAGE_STABILITY_SETTLE_IDLE_SECONDS:-5}"
WAL_STALL_MS="${FOD_STORAGE_STABILITY_WAL_STALL_MS:-5000}"
MAX_CLEAN_SPREAD_PCT="${FOD_STORAGE_STABILITY_MAX_CLEAN_SPREAD_PCT:-25}"
MATRIX="${ROOT}/scripts/perf/run_random_storage_block_matrix.sh"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${FOD_STORAGE_STABILITY_ARTIFACT_DIR:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-storage-block-stability-${RUN_ID}}"

for cmd in bash make git awk sort head tail tee sync sleep date; do
    command -v "${cmd}" >/dev/null 2>&1 || {
        echo "Missing required command: ${cmd}" >&2
        exit 2
    }
done

positive_integer() {
    local name="$1"
    local value="$2"
    case "${value}" in
        ''|*[!0-9]*)
            echo "${name} must be a positive integer" >&2
            exit 2
            ;;
    esac
    if (( value < 1 )); then
        echo "${name} must be >= 1" >&2
        exit 2
    fi
}

positive_integer FOD_STORAGE_STABILITY_REPEATS "${REPEATS}"
positive_integer FOD_STORAGE_STABILITY_SETTLE_DIRTY_KB "${SETTLE_DIRTY_KB}"
positive_integer FOD_STORAGE_STABILITY_SETTLE_TIMEOUT_SECONDS "${SETTLE_TIMEOUT_SECONDS}"
positive_integer FOD_STORAGE_STABILITY_SETTLE_POLL_SECONDS "${SETTLE_POLL_SECONDS}"
positive_integer FOD_STORAGE_STABILITY_SETTLE_IDLE_SECONDS "${SETTLE_IDLE_SECONDS}"
positive_integer FOD_STORAGE_STABILITY_WAL_STALL_MS "${WAL_STALL_MS}"
positive_integer FOD_STORAGE_STABILITY_MAX_CLEAN_SPREAD_PCT "${MAX_CLEAN_SPREAD_PCT}"

case "${SETTLE_ENABLED}" in
    0|1) ;;
    *)
        echo "FOD_STORAGE_STABILITY_SETTLE must be 0 or 1" >&2
        exit 2
        ;;
esac

if [[ "${PAYLOAD_MODE}" != "random" ]]; then
    echo "Storage stability benchmark requires random payloads; got ${PAYLOAD_MODE}" >&2
    exit 2
fi

if [[ ! -r /proc/meminfo ]]; then
    echo "/proc/meminfo is required for host writeback stabilization" >&2
    exit 2
fi

read -r -a BLOCK_SIZES <<<"${STORAGE_BLOCK_SIZES}"
if (( ${#BLOCK_SIZES[@]} < 1 )); then
    echo "At least one storage block size is required" >&2
    exit 2
fi
for storage_block_size in "${BLOCK_SIZES[@]}"; do
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
RUNS="${ARTIFACT_DIR}/runs.tsv"
MEDIANS="${ARTIFACT_DIR}/median.tsv"
SETTLE_LOG="${ARTIFACT_DIR}/settle.tsv"

{
    printf '%s\t' \
        "repeat" "order" "storage_block_size" \
        "primary_write_mib_s" "primary_read_mib_s" "replica_read_mib_s" \
        "copy_calls" "copy_rows" "copy_exec_ms" "copy_mean_ms" \
        "insert_calls" "insert_rows" "insert_exec_ms" "insert_mean_ms" "sql_flush_mean_ms" \
        "insert_wal_bytes" "wal_bytes_delta" "wal_buffers_full_delta" \
        "wal_write_delta" "wal_sync_delta" "wal_write_time_delta_ms" "wal_sync_time_delta_ms" \
        "pre_dirty_kb" "pre_writeback_kb" "post_dirty_kb" "post_writeback_kb" "run_quality"
    printf '%s\n' "profile_artifact_dir"
} >"${RUNS}"
printf 'timestamp\tlabel\tdirty_kb\twriteback_kb\ttotal_kb\n' >"${SETTLE_LOG}"

printf '=== FOD STORAGE BLOCK STABILITY MATRIX ===\n'
printf 'storage_block_sizes=%s\n' "${STORAGE_BLOCK_SIZES}"
printf 'repeats=%s\n' "${REPEATS}"
printf 'file_size=%s\nfio_block_size=%s\npayload_mode=%s\n' \
    "${FILE_SIZE}" "${FIO_BLOCK_SIZE}" "${PAYLOAD_MODE}"
printf 'settle=%s dirty_limit_kb=%s timeout_s=%s poll_s=%s idle_s=%s\n' \
    "${SETTLE_ENABLED}" "${SETTLE_DIRTY_KB}" "${SETTLE_TIMEOUT_SECONDS}" \
    "${SETTLE_POLL_SECONDS}" "${SETTLE_IDLE_SECONDS}"
printf 'wal_stall_ms=%s max_clean_spread_pct=%s\n' "${WAL_STALL_MS}" "${MAX_CLEAN_SPREAD_PCT}"
printf 'artifact_dir=%s\n' "${ARTIFACT_DIR}"

meminfo_kb() {
    local key="$1"
    awk -v key="${key}:" '$1 == key {print $2 + 0; found=1; exit} END {if (!found) print 0}' /proc/meminfo
}

record_settle_sample() {
    local label="$1"
    local dirty="$2"
    local writeback="$3"
    local total=$(( dirty + writeback ))
    printf '%s\t%s\t%s\t%s\t%s\n' \
        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${label}" "${dirty}" "${writeback}" "${total}" \
        >>"${SETTLE_LOG}"
}

settle_host_io() {
    local label="$1"
    if [[ "${SETTLE_ENABLED}" == "0" ]]; then
        echo "Host settle disabled label=${label}"
        return 0
    fi

    echo "Host settle start label=${label}: sync + Dirty/Writeback gate <= ${SETTLE_DIRTY_KB} KiB"
    sync
    local started now elapsed dirty writeback total confirm_dirty confirm_writeback confirm_total
    started="$(date +%s)"
    while true; do
        dirty="$(meminfo_kb Dirty)"
        writeback="$(meminfo_kb Writeback)"
        total=$(( dirty + writeback ))
        record_settle_sample "${label}" "${dirty}" "${writeback}"
        if (( total <= SETTLE_DIRTY_KB )); then
            sleep "${SETTLE_IDLE_SECONDS}"
            confirm_dirty="$(meminfo_kb Dirty)"
            confirm_writeback="$(meminfo_kb Writeback)"
            confirm_total=$(( confirm_dirty + confirm_writeback ))
            record_settle_sample "${label}-confirm" "${confirm_dirty}" "${confirm_writeback}"
            if (( confirm_total <= SETTLE_DIRTY_KB )); then
                echo "Host settle OK label=${label} dirty_kb=${confirm_dirty} writeback_kb=${confirm_writeback}"
                return 0
            fi
        fi

        now="$(date +%s)"
        elapsed=$(( now - started ))
        if (( elapsed >= SETTLE_TIMEOUT_SECONDS )); then
            echo "Host did not settle within ${SETTLE_TIMEOUT_SECONDS}s label=${label} dirty_kb=${dirty} writeback_kb=${writeback}; refusing contaminated benchmark" >&2
            return 1
        fi
        sleep "${SETTLE_POLL_SECONDS}"
    done
}

tsv_value() {
    local file="$1"
    local column="$2"
    awk -F '\t' -v column="${column}" 'NR == 2 {print $column}' "${file}"
}

ratio_ms() {
    local total="$1"
    local calls="$2"
    awk -v total="${total}" -v calls="${calls}" 'BEGIN {
        if ((calls + 0) > 0) printf "%.3f\n", (total + 0) / (calls + 0);
        else print "0";
    }'
}

sum_numeric() {
    local left="$1"
    local right="$2"
    awk -v left="${left}" -v right="${right}" 'BEGIN {printf "%.3f\n", (left + 0) + (right + 0)}'
}

wal_delta() {
    local file="$1"
    local column="$2"
    local format="${3:-float}"
    if [[ ! -s "${file}" ]] || (( $(awk 'END {print NR}' "${file}") < 2 )); then
        printf '0\n'
        return
    fi
    if [[ "${format}" == "int" ]]; then
        awk -F '\t' -v column="${column}" '
            NR == 1 {first=$column}
            {last=$column}
            END {printf "%.0f\n", (last + 0) - (first + 0)}
        ' "${file}"
    else
        awk -F '\t' -v column="${column}" '
            NR == 1 {first=$column}
            {last=$column}
            END {printf "%.3f\n", (last + 0) - (first + 0)}
        ' "${file}"
    fi
}

values_for() {
    local size="$1"
    local column="$2"
    local quality="${3:-all}"
    awk -F '\t' -v size="${size}" -v column="${column}" -v quality="${quality}" '
        NR > 1 && $3 == size && (quality == "all" || $27 == quality) {print $column}
    ' "${RUNS}"
}

median_for() {
    local size="$1"
    local column="$2"
    local quality="${3:-all}"
    values_for "${size}" "${column}" "${quality}" \
        | sort -n \
        | awk '
            {values[NR]=$1}
            END {
                if (NR == 0) {print "0"; exit}
                if (NR % 2 == 1) printf "%.3f\n", values[(NR + 1) / 2];
                else printf "%.3f\n", (values[NR / 2] + values[NR / 2 + 1]) / 2;
            }
        '
}

min_for() {
    local size="$1"
    local column="$2"
    local quality="${3:-all}"
    values_for "${size}" "${column}" "${quality}" \
        | awk '{if (!seen || $1 < min) min=$1; seen=1} END {if (seen) printf "%.3f\n", min; else print "0"}'
}

max_for() {
    local size="$1"
    local column="$2"
    local quality="${3:-all}"
    values_for "${size}" "${column}" "${quality}" \
        | awk '{if (!seen || $1 > max) max=$1; seen=1} END {if (seen) printf "%.3f\n", max; else print "0"}'
}

count_for() {
    local size="$1"
    local quality="${2:-all}"
    awk -F '\t' -v size="${size}" -v quality="${quality}" '
        NR > 1 && $3 == size && (quality == "all" || $27 == quality) {count++}
        END {print count + 0}
    ' "${RUNS}"
}

# Keep compilation and its filesystem writes outside all measured runs.
FOD_CARGO_PROFILE="${CARGO_PROFILE}" \
FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" \
make --no-print-directory build-runtime

size_count=${#BLOCK_SIZES[@]}
for (( repeat=1; repeat<=REPEATS; repeat++ )); do
    rotation=$(( (repeat - 1) % size_count ))
    printf '\n=== REPEAT %s/%s rotation=%s ===\n' "${repeat}" "${REPEATS}" "${rotation}"

    for (( offset=0; offset<size_count; offset++ )); do
        index=$(( (rotation + offset) % size_count ))
        storage_block_size="${BLOCK_SIZES[$index]}"
        order=$(( offset + 1 ))
        run_dir="${ARTIFACT_DIR}/repeat-${repeat}/block-${storage_block_size}"
        run_log="${run_dir}/run.log"
        mkdir -p "${run_dir}"

        printf '\n--- repeat=%s order=%s block_size=%s ---\n' \
            "${repeat}" "${order}" "${storage_block_size}"

        settle_host_io "pre-r${repeat}-o${order}-b${storage_block_size}"
        pre_dirty_kb="$(meminfo_kb Dirty)"
        pre_writeback_kb="$(meminfo_kb Writeback)"

        set +e
        FOD_CARGO_PROFILE="${CARGO_PROFILE}" \
        FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" \
        FOD_STORAGE_BLOCK_SKIP_BUILD=1 \
        FOD_STORAGE_BLOCK_SIZES="${storage_block_size}" \
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
            echo "Stability run failed repeat=${repeat} block_size=${storage_block_size}; log=${run_log}" >&2
            exit "${run_status}"
        fi

        summary="${run_dir}/summary.tsv"
        if [[ ! -s "${summary}" ]]; then
            echo "Missing run summary: ${summary}" >&2
            exit 1
        fi

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
        sql_flush_mean_ms="$(sum_numeric "${copy_mean_ms}" "${insert_mean_ms}")"

        wal_file="${profile_dir}/wal.tsv"
        wal_bytes_delta="$(wal_delta "${wal_file}" 4 int)"
        wal_buffers_full_delta="$(wal_delta "${wal_file}" 5 int)"
        wal_write_delta="$(wal_delta "${wal_file}" 6 int)"
        wal_sync_delta="$(wal_delta "${wal_file}" 7 int)"
        wal_write_time_delta_ms="$(wal_delta "${wal_file}" 8 float)"
        wal_sync_time_delta_ms="$(wal_delta "${wal_file}" 9 float)"

        run_quality="clean"
        if awk -v value="${wal_sync_time_delta_ms}" -v limit="${WAL_STALL_MS}" 'BEGIN {exit !((value + 0) > (limit + 0))}'; then
            run_quality="wal_stall"
        fi

        {
            printf '%s\t' \
                "${repeat}" "${order}" "${storage_block_size}" \
                "${primary_write}" "${primary_read}" "${replica_read}" \
                "${copy_calls}" "${copy_rows}" "${copy_exec_ms}" "${copy_mean_ms}" \
                "${insert_calls}" "${insert_rows}" "${insert_exec_ms}" "${insert_mean_ms}" "${sql_flush_mean_ms}" \
                "${insert_wal_bytes}" "${wal_bytes_delta}" "${wal_buffers_full_delta}" \
                "${wal_write_delta}" "${wal_sync_delta}" "${wal_write_time_delta_ms}" "${wal_sync_time_delta_ms}" \
                "${pre_dirty_kb}" "${pre_writeback_kb}" "${post_dirty_kb}" "${post_writeback_kb}" "${run_quality}"
            printf '%s\n' "${profile_dir}"
        } >>"${RUNS}"

        echo "run_quality=${run_quality} primary_write_mib_s=${primary_write} wal_sync_time_delta_ms=${wal_sync_time_delta_ms} pre_dirty_kb=${pre_dirty_kb} post_dirty_kb=${post_dirty_kb}"
    done
done

{
    printf '%s\t' \
        "storage_block_size" "runs" "clean_runs" "wal_stall_runs" \
        "median_primary_write_mib_s_all" "median_primary_write_mib_s_clean" \
        "min_primary_write_mib_s_clean" "max_primary_write_mib_s_clean" "clean_write_spread_pct" \
        "median_primary_read_mib_s_clean" "median_replica_read_mib_s_clean" \
        "median_copy_mean_ms_clean" "median_insert_mean_ms_clean" "median_sql_flush_mean_ms_clean" \
        "median_wal_bytes_delta_clean" "median_wal_sync_time_delta_ms_clean" \
        "max_wal_sync_time_delta_ms_all" "median_wal_buffers_full_delta_clean"
    printf '%s\n' "status"
} >"${MEDIANS}"

for storage_block_size in "${BLOCK_SIZES[@]}"; do
    runs="$(count_for "${storage_block_size}" all)"
    clean_runs="$(count_for "${storage_block_size}" clean)"
    wal_stall_runs="$(count_for "${storage_block_size}" wal_stall)"
    median_write_all="$(median_for "${storage_block_size}" 4 all)"
    median_write_clean="$(median_for "${storage_block_size}" 4 clean)"
    min_write_clean="$(min_for "${storage_block_size}" 4 clean)"
    max_write_clean="$(max_for "${storage_block_size}" 4 clean)"
    spread_pct="$(awk -v min="${min_write_clean}" -v max="${max_write_clean}" -v median="${median_write_clean}" 'BEGIN {
        if ((median + 0) > 0) printf "%.2f", ((max + 0) - (min + 0)) * 100 / (median + 0);
        else print "0";
    }')"

    status="OK"
    if (( clean_runs < 2 )); then
        status="INSUFFICIENT_CLEAN_RUNS"
    elif awk -v spread="${spread_pct}" -v limit="${MAX_CLEAN_SPREAD_PCT}" 'BEGIN {exit !((spread + 0) > (limit + 0))}'; then
        status="UNSTABLE_CLEAN_RUNS"
    fi

    {
        printf '%s\t' \
            "${storage_block_size}" "${runs}" "${clean_runs}" "${wal_stall_runs}" \
            "${median_write_all}" "${median_write_clean}" "${min_write_clean}" "${max_write_clean}" "${spread_pct}" \
            "$(median_for "${storage_block_size}" 5 clean)" \
            "$(median_for "${storage_block_size}" 6 clean)" \
            "$(median_for "${storage_block_size}" 10 clean)" \
            "$(median_for "${storage_block_size}" 14 clean)" \
            "$(median_for "${storage_block_size}" 15 clean)" \
            "$(median_for "${storage_block_size}" 17 clean)" \
            "$(median_for "${storage_block_size}" 22 clean)" \
            "$(max_for "${storage_block_size}" 22 all)" \
            "$(median_for "${storage_block_size}" 18 clean)"
        printf '%s\n' "${status}"
    } >>"${MEDIANS}"
done

selection_status="valid"
if awk -F '\t' 'NR > 1 && $19 != "OK" {bad=1} END {exit bad ? 0 : 1}' "${MEDIANS}"; then
    selection_status="invalid"
fi

best_size=""
best_write=""
if [[ "${selection_status}" == "valid" ]]; then
    best_size="$(awk -F '\t' 'NR > 1 {if (!seen || $6 > best) {best=$6; size=$1; seen=1}} END {print size}' "${MEDIANS}")"
    best_write="$(awk -F '\t' -v size="${best_size}" '$1 == size {print $6}' "${MEDIANS}")"
fi

echo
echo "=== STORAGE BLOCK STABILITY RUNS ==="
cat "${RUNS}"
echo
echo "=== STORAGE BLOCK STABILITY MEDIANS ==="
cat "${MEDIANS}"
echo "selection_status=${selection_status}"
if [[ "${selection_status}" == "valid" ]]; then
    echo "best_clean_median_primary_write_block_size=${best_size}"
    echo "best_clean_median_primary_write_mib_s=${best_write}"
else
    echo "No block size selected: at least one candidate lacks stable clean runs."
fi
echo "settle_log=${SETTLE_LOG}"
echo "stability_artifact_dir=${ARTIFACT_DIR}"
echo "OK: storage block stability matrix"
