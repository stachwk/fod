#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Collect clean FOD 32K / PostgreSQL 8K-vs-32K comparison runs while rejecting
# host/WAL stalls. The published PostgreSQL 32K image is reused by the underlying
# wrapper; the 8K control remains the same locally built custom PostgreSQL.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

BASE="${ROOT}/scripts/perf/run_postgres_blocksize_comparison_published32k.sh"
TARGET_CLEAN="${FOD_PG_BLOCK_CLEAN_TARGET:-3}"
MAX_ATTEMPTS="${FOD_PG_BLOCK_CLEAN_MAX_ATTEMPTS:-8}"
WAL_SYNC_STALL_MS="${FOD_PG_BLOCK_CLEAN_WAL_SYNC_STALL_MS:-3000}"
MAX_WRITE_SPREAD_PCT="${FOD_PG_BLOCK_CLEAN_MAX_WRITE_SPREAD_PCT:-20}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${FOD_PG_BLOCK_CLEAN_ARTIFACT_DIR:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-postgres-blocksize-clean-${RUN_ID}}"
ATTEMPTS_TSV="${ARTIFACT_DIR}/attempts.tsv"
SELECTED_TSV="${ARTIFACT_DIR}/selected-clean.tsv"
MEDIANS_TSV="${ARTIFACT_DIR}/median.tsv"
STATUS_TSV="${ARTIFACT_DIR}/attempt-status.tsv"

for cmd in bash awk sort tee date git hostname; do
    command -v "${cmd}" >/dev/null 2>&1 || {
        echo "Missing required command: ${cmd}" >&2
        exit 2
    }
done
[[ -r "${BASE}" ]] || { echo "Missing comparison wrapper: ${BASE}" >&2; exit 2; }

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

positive_integer FOD_PG_BLOCK_CLEAN_TARGET "${TARGET_CLEAN}"
positive_integer FOD_PG_BLOCK_CLEAN_MAX_ATTEMPTS "${MAX_ATTEMPTS}"
nonnegative_number FOD_PG_BLOCK_CLEAN_WAL_SYNC_STALL_MS "${WAL_SYNC_STALL_MS}"
nonnegative_number FOD_PG_BLOCK_CLEAN_MAX_WRITE_SPREAD_PCT "${MAX_WRITE_SPREAD_PCT}"
if (( MAX_ATTEMPTS < TARGET_CLEAN )); then
    echo "FOD_PG_BLOCK_CLEAN_MAX_ATTEMPTS must be >= FOD_PG_BLOCK_CLEAN_TARGET" >&2
    exit 2
fi

mkdir -p "${ARTIFACT_DIR}"
printf 'attempt\tpg_block_size_kb\tprimary_write_mib_s\tprimary_read_mib_s\treplica_read_mib_s\tcopy_mean_ms\tinsert_mean_ms\tinsert_wal_bytes\twal_bytes_delta\twal_sync_time_delta_ms\tquality\tprofile_artifact_dir\n' >"${ATTEMPTS_TSV}"
printf 'attempt\tpg_block_size_kb\tprimary_write_mib_s\tprimary_read_mib_s\treplica_read_mib_s\tcopy_mean_ms\tinsert_mean_ms\tinsert_wal_bytes\twal_bytes_delta\twal_sync_time_delta_ms\tprofile_artifact_dir\n' >"${SELECTED_TSV}"
printf 'attempt\tstatus\tcomparison_artifact_dir\n' >"${STATUS_TSV}"
printf 'pg_block_size_kb\tclean_runs\tmedian_primary_write_mib_s\tmin_primary_write_mib_s\tmax_primary_write_mib_s\twrite_spread_pct\tmedian_primary_read_mib_s\tmedian_replica_read_mib_s\tmedian_copy_mean_ms\tmedian_insert_mean_ms\tmedian_insert_wal_bytes\tmedian_wal_bytes_delta\tmedian_wal_sync_time_delta_ms\n' >"${MEDIANS_TSV}"

selected_count() {
    local pg_kb="$1"
    awk -F '\t' -v pg="${pg_kb}" 'NR > 1 && $2 == pg {n++} END {print n + 0}' "${SELECTED_TSV}"
}
selected_values() {
    local pg_kb="$1" column="$2"
    awk -F '\t' -v pg="${pg_kb}" -v column="${column}" 'NR > 1 && $2 == pg {print $column}' "${SELECTED_TSV}"
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

printf '=== FOD POSTGRESQL BLOCK SIZE CLEAN STABILITY ===\n'
printf 'target_clean_per_size=%s\nmax_attempts=%s\nwal_sync_stall_ms=%s\nmax_write_spread_pct=%s\nartifact_dir=%s\n' \
    "${TARGET_CLEAN}" "${MAX_ATTEMPTS}" "${WAL_SYNC_STALL_MS}" "${MAX_WRITE_SPREAD_PCT}" "${ARTIFACT_DIR}"

for ((attempt=1; attempt<=MAX_ATTEMPTS; attempt++)); do
    clean8="$(selected_count 8)"
    clean32="$(selected_count 32)"
    if (( clean8 >= TARGET_CLEAN && clean32 >= TARGET_CLEAN )); then
        break
    fi

    attempt_dir="${ARTIFACT_DIR}/attempt-${attempt}"
    comparison_dir="${attempt_dir}/comparison"
    attempt_log="${attempt_dir}/run.log"
    mkdir -p "${attempt_dir}"

    printf '\n=== ATTEMPT %s/%s clean8=%s/%s clean32=%s/%s ===\n' \
        "${attempt}" "${MAX_ATTEMPTS}" "${clean8}" "${TARGET_CLEAN}" "${clean32}" "${TARGET_CLEAN}"

    pull_image=0
    if (( attempt == 1 )); then
        pull_image="${FOD_PG32_PULL_IMAGE:-1}"
    fi

    set +e
    FOD_PG32_PULL_IMAGE="${pull_image}" \
    FOD_PG_BLOCK_COMPARISON_SIZES_KB="8 32" \
    FOD_PG_BLOCK_COMPARISON_REPEATS=1 \
    FOD_PG_BLOCK_COMPARISON_FOD_BLOCK_SIZE=32768 \
    FOD_PG_BLOCK_COMPARISON_FILE_SIZE="${FOD_PG_BLOCK_COMPARISON_FILE_SIZE:-1G}" \
    FOD_PG_BLOCK_COMPARISON_FIO_BLOCK_SIZE="${FOD_PG_BLOCK_COMPARISON_FIO_BLOCK_SIZE:-512k}" \
    FOD_PG_BLOCK_COMPARISON_PAYLOAD_MODE=random \
    FOD_PG_BLOCK_COMPARISON_ARTIFACT_DIR="${comparison_dir}" \
    FOD_CARGO_PROFILE="${FOD_CARGO_PROFILE:-profiling}" \
    FOD_RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}" \
    FOD_REQUIRE_AC_POWER="${FOD_REQUIRE_AC_POWER:-1}" \
        bash "${BASE}" 2>&1 | tee "${attempt_log}"
    status=${PIPESTATUS[0]}
    set -e

    if (( status != 0 )); then
        printf '%s\tinfra_error\t%s\n' "${attempt}" "${comparison_dir}" >>"${STATUS_TSV}"
        echo "Attempt ${attempt} failed before quality classification; retrying."
        continue
    fi

    runs="${comparison_dir}/runs.tsv"
    if [[ ! -s "${runs}" ]]; then
        printf '%s\tmissing_runs\t%s\n' "${attempt}" "${comparison_dir}" >>"${STATUS_TSV}"
        echo "Attempt ${attempt} has no runs.tsv; retrying."
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

        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "${attempt}" "${pg_kb}" "${write}" "${pread}" "${rread}" "${copy_mean}" "${insert_mean}" \
            "${insert_wal}" "${wal_delta}" "${wal_sync}" "${quality}" "${profile_dir}" >>"${ATTEMPTS_TSV}"

        current="$(selected_count "${pg_kb}")"
        if [[ "${quality}" == "clean" ]] && (( current < TARGET_CLEAN )); then
            printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
                "${attempt}" "${pg_kb}" "${write}" "${pread}" "${rread}" "${copy_mean}" "${insert_mean}" \
                "${insert_wal}" "${wal_delta}" "${wal_sync}" "${profile_dir}" >>"${SELECTED_TSV}"
        fi
    done <"${runs}"
done

selection_status=valid
for pg_kb in 8 32; do
    clean="$(selected_count "${pg_kb}")"
    if (( clean < TARGET_CLEAN )); then
        selection_status=insufficient_clean_runs
        continue
    fi

    median_write="$(median_for "${pg_kb}" 3)"
    min_write="$(min_for "${pg_kb}" 3)"
    max_write="$(max_for "${pg_kb}" 3)"
    spread="$(awk -v min="${min_write}" -v max="${max_write}" -v median="${median_write}" 'BEGIN {
        if ((median + 0) == 0) print "0";
        else printf "%.2f\n", ((max + 0) - (min + 0)) / (median + 0) * 100.0;
    }')"
    if awk -v spread="${spread}" -v max="${MAX_WRITE_SPREAD_PCT}" 'BEGIN {exit !((spread + 0) > (max + 0))}'; then
        selection_status=unstable
    fi

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "${pg_kb}" "${clean}" "${median_write}" "${min_write}" "${max_write}" "${spread}" \
        "$(median_for "${pg_kb}" 4)" "$(median_for "${pg_kb}" 5)" "$(median_for "${pg_kb}" 6)" \
        "$(median_for "${pg_kb}" 7)" "$(median_for "${pg_kb}" 8)" "$(median_for "${pg_kb}" 9)" \
        "$(median_for "${pg_kb}" 10)" >>"${MEDIANS_TSV}"
done

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

echo "clean_stability_artifact_dir=${ARTIFACT_DIR}"
if [[ "${selection_status}" != "valid" ]]; then
    echo "INVALID: PostgreSQL block-size clean stability ${selection_status}" >&2
    exit 1
fi

echo "OK: PostgreSQL block-size clean stability"
