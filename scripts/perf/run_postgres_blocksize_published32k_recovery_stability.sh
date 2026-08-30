#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Stable FOD 32K / PostgreSQL 8K-vs-32K comparison with a host recovery gate.
# The benchmark is started only after Dirty+Writeback settles and an fsync probe
# confirms that the artifact filesystem is responsive. PostgreSQL order rotates
# between attempts to avoid systematically favoring the first or second variant.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

BASE="${ROOT}/scripts/perf/run_postgres_blocksize_comparison_published32k.sh"
TARGET_CLEAN="${FOD_PG_BLOCK_RECOVERY_TARGET:-3}"
MAX_ATTEMPTS="${FOD_PG_BLOCK_RECOVERY_MAX_ATTEMPTS:-10}"
WAL_SYNC_STALL_MS="${FOD_PG_BLOCK_RECOVERY_WAL_SYNC_STALL_MS:-3000}"
MAX_WRITE_SPREAD_PCT="${FOD_PG_BLOCK_RECOVERY_MAX_WRITE_SPREAD_PCT:-20}"
SETTLE_DIRTY_KB="${FOD_PG_BLOCK_RECOVERY_SETTLE_DIRTY_KB:-8192}"
SETTLE_TIMEOUT_SECONDS="${FOD_PG_BLOCK_RECOVERY_SETTLE_TIMEOUT_SECONDS:-300}"
SETTLE_POLL_SECONDS="${FOD_PG_BLOCK_RECOVERY_SETTLE_POLL_SECONDS:-2}"
SETTLE_COOLDOWN_SECONDS="${FOD_PG_BLOCK_RECOVERY_COOLDOWN_SECONDS:-20}"
FSYNC_PROBE_COUNT="${FOD_PG_BLOCK_RECOVERY_FSYNC_PROBE_COUNT:-8}"
FSYNC_MEDIAN_MAX_MS="${FOD_PG_BLOCK_RECOVERY_FSYNC_MEDIAN_MAX_MS:-20}"
FSYNC_SINGLE_MAX_MS="${FOD_PG_BLOCK_RECOVERY_FSYNC_SINGLE_MAX_MS:-100}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${FOD_PG_BLOCK_RECOVERY_ARTIFACT_DIR:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-postgres-blocksize-recovery-${RUN_ID}}"
ATTEMPTS_TSV="${ARTIFACT_DIR}/attempts.tsv"
SELECTED_TSV="${ARTIFACT_DIR}/selected-clean.tsv"
MEDIANS_TSV="${ARTIFACT_DIR}/median.tsv"
SETTLE_TSV="${ARTIFACT_DIR}/settle.tsv"
STATUS_TSV="${ARTIFACT_DIR}/attempt-status.tsv"

for cmd in bash awk sort tee date git hostname sync sleep python3; do
    command -v "${cmd}" >/dev/null 2>&1 || {
        echo "Missing required command: ${cmd}" >&2
        exit 2
    }
done
[[ -r "${BASE}" ]] || { echo "Missing comparison wrapper: ${BASE}" >&2; exit 2; }
[[ -r /proc/meminfo ]] || { echo "/proc/meminfo is required" >&2; exit 2; }

positive_integer() {
    local name="$1" value="$2"
    case "${value}" in
        ''|*[!0-9]*) echo "${name} must be a positive integer" >&2; exit 2 ;;
    esac
    (( value >= 1 )) || { echo "${name} must be >= 1" >&2; exit 2; }
}
nonnegative_number() {
    local name="$1" value="$2"
    awk -v name="${name}" -v value="${value}" 'BEGIN {
        if (value !~ /^[0-9]+([.][0-9]+)?$/) {
            print name " must be a non-negative number" > "/dev/stderr";
            exit 2;
        }
    }'
}

positive_integer FOD_PG_BLOCK_RECOVERY_TARGET "${TARGET_CLEAN}"
positive_integer FOD_PG_BLOCK_RECOVERY_MAX_ATTEMPTS "${MAX_ATTEMPTS}"
positive_integer FOD_PG_BLOCK_RECOVERY_SETTLE_DIRTY_KB "${SETTLE_DIRTY_KB}"
positive_integer FOD_PG_BLOCK_RECOVERY_SETTLE_TIMEOUT_SECONDS "${SETTLE_TIMEOUT_SECONDS}"
positive_integer FOD_PG_BLOCK_RECOVERY_SETTLE_POLL_SECONDS "${SETTLE_POLL_SECONDS}"
positive_integer FOD_PG_BLOCK_RECOVERY_COOLDOWN_SECONDS "${SETTLE_COOLDOWN_SECONDS}"
positive_integer FOD_PG_BLOCK_RECOVERY_FSYNC_PROBE_COUNT "${FSYNC_PROBE_COUNT}"
nonnegative_number FOD_PG_BLOCK_RECOVERY_WAL_SYNC_STALL_MS "${WAL_SYNC_STALL_MS}"
nonnegative_number FOD_PG_BLOCK_RECOVERY_MAX_WRITE_SPREAD_PCT "${MAX_WRITE_SPREAD_PCT}"
nonnegative_number FOD_PG_BLOCK_RECOVERY_FSYNC_MEDIAN_MAX_MS "${FSYNC_MEDIAN_MAX_MS}"
nonnegative_number FOD_PG_BLOCK_RECOVERY_FSYNC_SINGLE_MAX_MS "${FSYNC_SINGLE_MAX_MS}"
if (( MAX_ATTEMPTS < TARGET_CLEAN )); then
    echo "FOD_PG_BLOCK_RECOVERY_MAX_ATTEMPTS must be >= FOD_PG_BLOCK_RECOVERY_TARGET" >&2
    exit 2
fi

mkdir -p "${ARTIFACT_DIR}"
printf 'attempt\torder\tpg_block_size_kb\tprimary_write_mib_s\tprimary_read_mib_s\treplica_read_mib_s\tcopy_mean_ms\tinsert_mean_ms\tinsert_wal_bytes\twal_bytes_delta\twal_sync_time_delta_ms\tquality\tprofile_artifact_dir\n' >"${ATTEMPTS_TSV}"
printf 'attempt\torder\tpg_block_size_kb\tprimary_write_mib_s\tprimary_read_mib_s\treplica_read_mib_s\tcopy_mean_ms\tinsert_mean_ms\tinsert_wal_bytes\twal_bytes_delta\twal_sync_time_delta_ms\tprofile_artifact_dir\n' >"${SELECTED_TSV}"
printf 'attempt\tstatus\tcomparison_artifact_dir\n' >"${STATUS_TSV}"
printf 'timestamp\tattempt\tdirty_kb\twriteback_kb\ttotal_kb\tfsync_median_ms\tfsync_max_ms\tstatus\n' >"${SETTLE_TSV}"
printf 'pg_block_size_kb\tclean_runs\tmedian_primary_write_mib_s\tmin_primary_write_mib_s\tmax_primary_write_mib_s\twrite_spread_pct\tmedian_primary_read_mib_s\tmedian_replica_read_mib_s\tmedian_copy_mean_ms\tmedian_insert_mean_ms\tmedian_insert_wal_bytes\tmedian_wal_bytes_delta\tmedian_wal_sync_time_delta_ms\n' >"${MEDIANS_TSV}"

meminfo_kb() {
    local key="$1"
    awk -v key="${key}:" '$1 == key {print $2 + 0; found=1; exit} END {if (!found) print 0}' /proc/meminfo
}

fsync_probe() {
    local probe_file="${ARTIFACT_DIR}/.fsync-probe"
    python3 - "${probe_file}" "${FSYNC_PROBE_COUNT}" <<'PY'
import os
import statistics
import sys
import time

path = sys.argv[1]
count = int(sys.argv[2])
values = []
for index in range(count):
    started = time.perf_counter_ns()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        os.write(fd, b"x" * 4096)
        os.fsync(fd)
    finally:
        os.close(fd)
    values.append((time.perf_counter_ns() - started) / 1_000_000.0)
try:
    os.unlink(path)
except FileNotFoundError:
    pass
print(f"{statistics.median(values):.3f}\t{max(values):.3f}")
PY
}

wait_for_host_recovery() {
    local attempt="$1"
    local started dirty writeback total probe median_ms max_ms
    started="$(date +%s)"
    echo "Host recovery gate attempt=${attempt}: Dirty+Writeback <= ${SETTLE_DIRTY_KB} KiB, cooldown=${SETTLE_COOLDOWN_SECONDS}s, fsync median<=${FSYNC_MEDIAN_MAX_MS}ms max<=${FSYNC_SINGLE_MAX_MS}ms"

    while true; do
        sync
        dirty="$(meminfo_kb Dirty)"
        writeback="$(meminfo_kb Writeback)"
        total=$((dirty + writeback))
        printf '%s\t%s\t%s\t%s\t%s\t-\t-\tmemcheck\n' \
            "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${attempt}" "${dirty}" "${writeback}" "${total}" >>"${SETTLE_TSV}"

        if (( total <= SETTLE_DIRTY_KB )); then
            sleep "${SETTLE_COOLDOWN_SECONDS}"
            dirty="$(meminfo_kb Dirty)"
            writeback="$(meminfo_kb Writeback)"
            total=$((dirty + writeback))
            if (( total <= SETTLE_DIRTY_KB )); then
                probe="$(fsync_probe)"
                median_ms="${probe%%$'\t'*}"
                max_ms="${probe##*$'\t'}"
                if awk -v median="${median_ms}" -v max="${max_ms}" -v median_limit="${FSYNC_MEDIAN_MAX_MS}" -v max_limit="${FSYNC_SINGLE_MAX_MS}" 'BEGIN {exit !((median + 0) <= (median_limit + 0) && (max + 0) <= (max_limit + 0))}'; then
                    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\tready\n' \
                        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${attempt}" "${dirty}" "${writeback}" "${total}" "${median_ms}" "${max_ms}" >>"${SETTLE_TSV}"
                    echo "Host recovery OK attempt=${attempt} dirty_kb=${dirty} writeback_kb=${writeback} fsync_median_ms=${median_ms} fsync_max_ms=${max_ms}"
                    return 0
                fi
                printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\tfsync_slow\n' \
                    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${attempt}" "${dirty}" "${writeback}" "${total}" "${median_ms}" "${max_ms}" >>"${SETTLE_TSV}"
                echo "Host fsync probe still slow attempt=${attempt} median_ms=${median_ms} max_ms=${max_ms}"
            fi
        fi

        if (( $(date +%s) - started >= SETTLE_TIMEOUT_SECONDS )); then
            printf '%s\t%s\t%s\t%s\t%s\t-\t-\ttimeout\n' \
                "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${attempt}" "${dirty}" "${writeback}" "${total}" >>"${SETTLE_TSV}"
            echo "Host did not recover within configured gate attempt=${attempt}" >&2
            return 1
        fi
        sleep "${SETTLE_POLL_SECONDS}"
    done
}

selected_count() {
    local pg_kb="$1"
    awk -F '\t' -v pg="${pg_kb}" 'NR > 1 && $3 == pg {n++} END {print n + 0}' "${SELECTED_TSV}"
}
selected_values() {
    local pg_kb="$1" column="$2"
    awk -F '\t' -v pg="${pg_kb}" -v column="${column}" 'NR > 1 && $3 == pg {print $column}' "${SELECTED_TSV}"
}
median_for() {
    local pg_kb="$1" column="$2"
    selected_values "${pg_kb}" "${column}" | sort -n | awk '
        {v[NR]=$1}
        END {
            if (NR == 0) {print "0"; exit}
            if (NR % 2) printf "%.3f\n", v[(NR + 1) / 2];
            else printf "%.3f\n", (v[NR / 2] + v[NR / 2 + 1]) / 2;
        }'
}
min_for() {
    local pg_kb="$1" column="$2"
    selected_values "${pg_kb}" "${column}" | awk '{if (!seen || $1 < min) min=$1; seen=1} END {if (seen) printf "%.3f\n", min; else print "0"}'
}
max_for() {
    local pg_kb="$1" column="$2"
    selected_values "${pg_kb}" "${column}" | awk '{if (!seen || $1 > max) max=$1; seen=1} END {if (seen) printf "%.3f\n", max; else print "0"}'
}
percent_delta() {
    local base="$1" candidate="$2"
    awk -v base="${base}" -v candidate="${candidate}" 'BEGIN {
        if ((base + 0) == 0) {print "0"; exit}
        printf "%.2f\n", ((candidate + 0) / (base + 0) - 1.0) * 100.0;
    }'
}

printf '=== FOD POSTGRESQL BLOCK SIZE RECOVERY STABILITY ===\n'
printf 'target_clean_per_size=%s\nmax_attempts=%s\nwal_sync_stall_ms=%s\nmax_write_spread_pct=%s\nsettle_dirty_kb=%s\nsettle_timeout_seconds=%s\nsettle_cooldown_seconds=%s\nfsync_probe_count=%s\nfsync_median_max_ms=%s\nfsync_single_max_ms=%s\nartifact_dir=%s\n' \
    "${TARGET_CLEAN}" "${MAX_ATTEMPTS}" "${WAL_SYNC_STALL_MS}" "${MAX_WRITE_SPREAD_PCT}" \
    "${SETTLE_DIRTY_KB}" "${SETTLE_TIMEOUT_SECONDS}" "${SETTLE_COOLDOWN_SECONDS}" "${FSYNC_PROBE_COUNT}" \
    "${FSYNC_MEDIAN_MAX_MS}" "${FSYNC_SINGLE_MAX_MS}" "${ARTIFACT_DIR}"

selection_status=valid
for ((attempt=1; attempt<=MAX_ATTEMPTS; attempt++)); do
    clean8="$(selected_count 8)"
    clean32="$(selected_count 32)"
    if (( clean8 >= TARGET_CLEAN && clean32 >= TARGET_CLEAN )); then
        break
    fi

    if ! wait_for_host_recovery "${attempt}"; then
        selection_status=host_not_ready
        printf '%s\thost_not_ready\t-\n' "${attempt}" >>"${STATUS_TSV}"
        break
    fi

    if (( attempt % 2 == 1 )); then
        pg_order="8 32"
    else
        pg_order="32 8"
    fi

    attempt_dir="${ARTIFACT_DIR}/attempt-${attempt}"
    comparison_dir="${attempt_dir}/comparison"
    attempt_log="${attempt_dir}/run.log"
    mkdir -p "${attempt_dir}"

    printf '\n=== ATTEMPT %s/%s order=%s clean8=%s/%s clean32=%s/%s ===\n' \
        "${attempt}" "${MAX_ATTEMPTS}" "${pg_order}" "${clean8}" "${TARGET_CLEAN}" "${clean32}" "${TARGET_CLEAN}"

    pull_image=0
    if (( attempt == 1 )); then
        pull_image="${FOD_PG32_PULL_IMAGE:-1}"
    fi

    set +e
    FOD_PG32_PULL_IMAGE="${pull_image}" \
    FOD_PG_BLOCK_COMPARISON_SIZES_KB="${pg_order}" \
    FOD_PG_BLOCK_COMPARISON_REPEATS=1 \
    FOD_PG_BLOCK_COMPARISON_FOD_BLOCK_SIZE=32768 \
    FOD_PG_BLOCK_COMPARISON_FILE_SIZE="${FOD_PG_BLOCK_COMPARISON_FILE_SIZE:-1G}" \
    FOD_PG_BLOCK_COMPARISON_FIO_BLOCK_SIZE="${FOD_PG_BLOCK_COMPARISON_FIO_BLOCK_SIZE:-512k}" \
    FOD_PG_BLOCK_COMPARISON_PAYLOAD_MODE=random \
    FOD_PG_BLOCK_COMPARISON_ARTIFACT_DIR="${comparison_dir}" \
    FOD_PG_WRITE_PROFILE_WAL_EVERY=1 \
    FOD_CARGO_PROFILE="${FOD_CARGO_PROFILE:-profiling}" \
    FOD_RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}" \
    FOD_REQUIRE_AC_POWER="${FOD_REQUIRE_AC_POWER:-1}" \
        bash "${BASE}" 2>&1 | tee "${attempt_log}"
    status=${PIPESTATUS[0]}
    set -e

    if (( status != 0 )); then
        printf '%s\tinfra_error\t%s\n' "${attempt}" "${comparison_dir}" >>"${STATUS_TSV}"
        echo "Attempt ${attempt} failed before quality classification; retrying after next recovery gate."
        continue
    fi

    runs="${comparison_dir}/runs.tsv"
    if [[ ! -s "${runs}" ]]; then
        printf '%s\tmissing_runs\t%s\n' "${attempt}" "${comparison_dir}" >>"${STATUS_TSV}"
        echo "Attempt ${attempt} has no runs.tsv; retrying after next recovery gate."
        continue
    fi

    printf '%s\tcompleted\t%s\n' "${attempt}" "${comparison_dir}" >>"${STATUS_TSV}"

    while IFS=$'\t' read -r repeat order pg_kb pg_bytes fod_bytes write pread rread copy_calls copy_exec copy_mean insert_calls insert_exec insert_mean insert_wal wal_delta wal_sync profile_dir; do
        [[ "${repeat}" == "repeat" ]] && continue
        quality=clean
        if awk -v value="${wal_sync}" -v threshold="${WAL_SYNC_STALL_MS}" 'BEGIN {exit !((value + 0) > (threshold + 0))}'; then
            quality=wal_stall
        fi
        if awk -v value="${write}" 'BEGIN {exit !((value + 0) <= 0)}'; then
            quality=invalid_throughput
        fi

        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "${attempt}" "${order}" "${pg_kb}" "${write}" "${pread}" "${rread}" "${copy_mean}" "${insert_mean}" \
            "${insert_wal}" "${wal_delta}" "${wal_sync}" "${quality}" "${profile_dir}" >>"${ATTEMPTS_TSV}"

        current="$(selected_count "${pg_kb}")"
        if [[ "${quality}" == "clean" ]] && (( current < TARGET_CLEAN )); then
            printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
                "${attempt}" "${order}" "${pg_kb}" "${write}" "${pread}" "${rread}" "${copy_mean}" "${insert_mean}" \
                "${insert_wal}" "${wal_delta}" "${wal_sync}" "${profile_dir}" >>"${SELECTED_TSV}"
        fi
    done <"${runs}"
done

for pg_kb in 8 32; do
    clean="$(selected_count "${pg_kb}")"
    if (( clean < TARGET_CLEAN )); then
        if [[ "${selection_status}" == "valid" ]]; then
            selection_status=insufficient_clean_runs
        fi
        continue
    fi

    median_write="$(median_for "${pg_kb}" 4)"
    min_write="$(min_for "${pg_kb}" 4)"
    max_write="$(max_for "${pg_kb}" 4)"
    spread="$(awk -v min="${min_write}" -v max="${max_write}" -v median="${median_write}" 'BEGIN {
        if ((median + 0) == 0) print "0";
        else printf "%.2f\n", ((max + 0) - (min + 0)) / (median + 0) * 100.0;
    }')"
    if awk -v spread="${spread}" -v max="${MAX_WRITE_SPREAD_PCT}" 'BEGIN {exit !((spread + 0) > (max + 0))}'; then
        selection_status=unstable
    fi

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "${pg_kb}" "${clean}" "${median_write}" "${min_write}" "${max_write}" "${spread}" \
        "$(median_for "${pg_kb}" 5)" "$(median_for "${pg_kb}" 6)" "$(median_for "${pg_kb}" 7)" \
        "$(median_for "${pg_kb}" 8)" "$(median_for "${pg_kb}" 9)" "$(median_for "${pg_kb}" 10)" \
        "$(median_for "${pg_kb}" 11)" >>"${MEDIANS_TSV}"
done

echo
echo "=== HOST RECOVERY GATE ==="
cat "${SETTLE_TSV}"
echo
echo "=== CLEAN/STALLED ATTEMPTS ==="
cat "${ATTEMPTS_TSV}"
echo
echo "=== SELECTED CLEAN RUNS ==="
cat "${SELECTED_TSV}"
echo
echo "=== CLEAN MEDIANS ==="
cat "${MEDIANS_TSV}"
echo "selection_status=${selection_status}"

if grep -q $'^8\t' "${MEDIANS_TSV}" && grep -q $'^32\t' "${MEDIANS_TSV}"; then
    write8="$(awk -F '\t' '$1 == 8 {print $3}' "${MEDIANS_TSV}")"
    write32="$(awk -F '\t' '$1 == 32 {print $3}' "${MEDIANS_TSV}")"
    pread8="$(awk -F '\t' '$1 == 8 {print $7}' "${MEDIANS_TSV}")"
    pread32="$(awk -F '\t' '$1 == 32 {print $7}' "${MEDIANS_TSV}")"
    rread8="$(awk -F '\t' '$1 == 8 {print $8}' "${MEDIANS_TSV}")"
    rread32="$(awk -F '\t' '$1 == 32 {print $8}' "${MEDIANS_TSV}")"
    copy8="$(awk -F '\t' '$1 == 8 {print $9}' "${MEDIANS_TSV}")"
    copy32="$(awk -F '\t' '$1 == 32 {print $9}' "${MEDIANS_TSV}")"
    insert8="$(awk -F '\t' '$1 == 8 {print $10}' "${MEDIANS_TSV}")"
    insert32="$(awk -F '\t' '$1 == 32 {print $10}' "${MEDIANS_TSV}")"
    insertwal8="$(awk -F '\t' '$1 == 8 {print $11}' "${MEDIANS_TSV}")"
    insertwal32="$(awk -F '\t' '$1 == 32 {print $11}' "${MEDIANS_TSV}")"
    wal8="$(awk -F '\t' '$1 == 8 {print $12}' "${MEDIANS_TSV}")"
    wal32="$(awk -F '\t' '$1 == 32 {print $12}' "${MEDIANS_TSV}")"
    sync8="$(awk -F '\t' '$1 == 8 {print $13}' "${MEDIANS_TSV}")"
    sync32="$(awk -F '\t' '$1 == 32 {print $13}' "${MEDIANS_TSV}")"

    echo "postgres_32k_vs_8k_primary_write_pct=$(percent_delta "${write8}" "${write32}")"
    echo "postgres_32k_vs_8k_primary_read_pct=$(percent_delta "${pread8}" "${pread32}")"
    echo "postgres_32k_vs_8k_replica_read_pct=$(percent_delta "${rread8}" "${rread32}")"
    echo "postgres_32k_vs_8k_copy_mean_pct=$(percent_delta "${copy8}" "${copy32}")"
    echo "postgres_32k_vs_8k_insert_mean_pct=$(percent_delta "${insert8}" "${insert32}")"
    echo "postgres_32k_vs_8k_insert_wal_bytes_pct=$(percent_delta "${insertwal8}" "${insertwal32}")"
    echo "postgres_32k_vs_8k_wal_bytes_pct=$(percent_delta "${wal8}" "${wal32}")"
    echo "postgres_32k_vs_8k_wal_sync_time_pct=$(percent_delta "${sync8}" "${sync32}")"
fi

echo "recovery_stability_artifact_dir=${ARTIFACT_DIR}"
if [[ "${selection_status}" != "valid" ]]; then
    echo "INVALID: PostgreSQL block-size recovery stability ${selection_status}" >&2
    exit 1
fi

echo "OK: PostgreSQL block-size recovery stability"
