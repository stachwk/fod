#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Compare PostgreSQL 8K vs 32K with an A-B-B-A design. The first and last
# runs are the same sentinel variant; a round is accepted only when those
# sentinels show limited host drift and every run stays below the WAL stall
# threshold. The two middle candidate runs are averaged and compared with
# the average of the two sentinels, reducing bias from monotonic host decay.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

BASE="${ROOT}/scripts/perf/run_postgres_blocksize_comparison_published32k.sh"
TARGET_PER_ORIENTATION="${FOD_PG_BLOCK_PAIRED_TARGET_PER_ORIENTATION:-2}"
MAX_ATTEMPTS="${FOD_PG_BLOCK_PAIRED_MAX_ATTEMPTS:-10}"
WAL_SYNC_STALL_MS="${FOD_PG_BLOCK_PAIRED_WAL_SYNC_STALL_MS:-3000}"
SENTINEL_WRITE_DRIFT_PCT="${FOD_PG_BLOCK_PAIRED_SENTINEL_WRITE_DRIFT_PCT:-10}"
SENTINEL_READ_DRIFT_PCT="${FOD_PG_BLOCK_PAIRED_SENTINEL_READ_DRIFT_PCT:-15}"
SENTINEL_SQL_DRIFT_PCT="${FOD_PG_BLOCK_PAIRED_SENTINEL_SQL_DRIFT_PCT:-20}"
MAX_RATIO_SPREAD_PP="${FOD_PG_BLOCK_PAIRED_MAX_RATIO_SPREAD_PP:-15}"
SETTLE_DIRTY_KB="${FOD_PG_BLOCK_PAIRED_SETTLE_DIRTY_KB:-8192}"
SETTLE_TIMEOUT_SECONDS="${FOD_PG_BLOCK_PAIRED_SETTLE_TIMEOUT_SECONDS:-300}"
SETTLE_POLL_SECONDS="${FOD_PG_BLOCK_PAIRED_SETTLE_POLL_SECONDS:-2}"
SETTLE_COOLDOWN_SECONDS="${FOD_PG_BLOCK_PAIRED_COOLDOWN_SECONDS:-20}"
FSYNC_PROBE_COUNT="${FOD_PG_BLOCK_PAIRED_FSYNC_PROBE_COUNT:-8}"
FSYNC_MEDIAN_MAX_MS="${FOD_PG_BLOCK_PAIRED_FSYNC_MEDIAN_MAX_MS:-5}"
FSYNC_SINGLE_MAX_MS="${FOD_PG_BLOCK_PAIRED_FSYNC_SINGLE_MAX_MS:-25}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${FOD_PG_BLOCK_PAIRED_ARTIFACT_DIR:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-postgres-blocksize-paired-${RUN_ID}}"
ROUNDS_TSV="${ARTIFACT_DIR}/rounds.tsv"
ACCEPTED_TSV="${ARTIFACT_DIR}/accepted.tsv"
SETTLE_TSV="${ARTIFACT_DIR}/settle.tsv"
SUMMARY_TSV="${ARTIFACT_DIR}/summary.tsv"

for cmd in bash awk sort tee date git hostname sync sleep python3; do
    command -v "${cmd}" >/dev/null 2>&1 || { echo "Missing required command: ${cmd}" >&2; exit 2; }
done
[[ -r "${BASE}" ]] || { echo "Missing comparison wrapper: ${BASE}" >&2; exit 2; }
[[ -r /proc/meminfo ]] || { echo "/proc/meminfo is required" >&2; exit 2; }

positive_integer() {
    local name="$1" value="$2"
    case "${value}" in ''|*[!0-9]*) echo "${name} must be a positive integer" >&2; exit 2;; esac
    (( value >= 1 )) || { echo "${name} must be >= 1" >&2; exit 2; }
}
nonnegative_number() {
    local name="$1" value="$2"
    awk -v name="${name}" -v value="${value}" 'BEGIN {
        if (value !~ /^[0-9]+([.][0-9]+)?$/) { print name " must be a non-negative number" > "/dev/stderr"; exit 2 }
    }'
}

positive_integer FOD_PG_BLOCK_PAIRED_TARGET_PER_ORIENTATION "${TARGET_PER_ORIENTATION}"
positive_integer FOD_PG_BLOCK_PAIRED_MAX_ATTEMPTS "${MAX_ATTEMPTS}"
positive_integer FOD_PG_BLOCK_PAIRED_SETTLE_DIRTY_KB "${SETTLE_DIRTY_KB}"
positive_integer FOD_PG_BLOCK_PAIRED_SETTLE_TIMEOUT_SECONDS "${SETTLE_TIMEOUT_SECONDS}"
positive_integer FOD_PG_BLOCK_PAIRED_SETTLE_POLL_SECONDS "${SETTLE_POLL_SECONDS}"
positive_integer FOD_PG_BLOCK_PAIRED_COOLDOWN_SECONDS "${SETTLE_COOLDOWN_SECONDS}"
positive_integer FOD_PG_BLOCK_PAIRED_FSYNC_PROBE_COUNT "${FSYNC_PROBE_COUNT}"
for item in \
    "FOD_PG_BLOCK_PAIRED_WAL_SYNC_STALL_MS:${WAL_SYNC_STALL_MS}" \
    "FOD_PG_BLOCK_PAIRED_SENTINEL_WRITE_DRIFT_PCT:${SENTINEL_WRITE_DRIFT_PCT}" \
    "FOD_PG_BLOCK_PAIRED_SENTINEL_READ_DRIFT_PCT:${SENTINEL_READ_DRIFT_PCT}" \
    "FOD_PG_BLOCK_PAIRED_SENTINEL_SQL_DRIFT_PCT:${SENTINEL_SQL_DRIFT_PCT}" \
    "FOD_PG_BLOCK_PAIRED_MAX_RATIO_SPREAD_PP:${MAX_RATIO_SPREAD_PP}" \
    "FOD_PG_BLOCK_PAIRED_FSYNC_MEDIAN_MAX_MS:${FSYNC_MEDIAN_MAX_MS}" \
    "FOD_PG_BLOCK_PAIRED_FSYNC_SINGLE_MAX_MS:${FSYNC_SINGLE_MAX_MS}"; do
    nonnegative_number "${item%%:*}" "${item#*:}"
done

mkdir -p "${ARTIFACT_DIR}"
printf 'attempt\tsentinel_pg_kb\tsequence\tquality\twrite_drift_pct\tprimary_read_drift_pct\treplica_read_drift_pct\tcopy_drift_pct\tinsert_drift_pct\tmax_wal_sync_ms\twrite_32_vs_8_pct\tprimary_read_32_vs_8_pct\treplica_read_32_vs_8_pct\tcopy_32_vs_8_pct\tinsert_32_vs_8_pct\tinsert_wal_32_vs_8_pct\twal_32_vs_8_pct\twal_sync_32_vs_8_pct\tcomparison_artifact_dir\n' >"${ROUNDS_TSV}"
printf 'attempt\tsentinel_pg_kb\twrite_32_vs_8_pct\tprimary_read_32_vs_8_pct\treplica_read_32_vs_8_pct\tcopy_32_vs_8_pct\tinsert_32_vs_8_pct\tinsert_wal_32_vs_8_pct\twal_32_vs_8_pct\twal_sync_32_vs_8_pct\tcomparison_artifact_dir\n' >"${ACCEPTED_TSV}"
printf 'timestamp\tattempt\tdirty_kb\twriteback_kb\ttotal_kb\tfsync_median_ms\tfsync_max_ms\tstatus\n' >"${SETTLE_TSV}"
printf 'metric\tmedian_pct\tmin_pct\tmax_pct\tspread_pp\n' >"${SUMMARY_TSV}"

meminfo_kb() {
    local key="$1"
    awk -v key="${key}:" '$1 == key {print $2 + 0; found=1; exit} END {if (!found) print 0}' /proc/meminfo
}
fsync_probe() {
    local probe_file="${ARTIFACT_DIR}/.fsync-probe"
    python3 - "${probe_file}" "${FSYNC_PROBE_COUNT}" <<'PY'
import os, statistics, sys, time
path = sys.argv[1]
count = int(sys.argv[2])
values = []
for _ in range(count):
    started = time.perf_counter_ns()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        os.write(fd, b"x" * 4096)
        os.fsync(fd)
    finally:
        os.close(fd)
    values.append((time.perf_counter_ns() - started) / 1_000_000.0)
try: os.unlink(path)
except FileNotFoundError: pass
print(f"{statistics.median(values):.3f}\t{max(values):.3f}")
PY
}
wait_for_host_recovery() {
    local attempt="$1" started dirty writeback total probe median_ms max_ms
    started="$(date +%s)"
    while true; do
        sync
        dirty="$(meminfo_kb Dirty)"; writeback="$(meminfo_kb Writeback)"; total=$((dirty + writeback))
        printf '%s\t%s\t%s\t%s\t%s\t-\t-\tmemcheck\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${attempt}" "${dirty}" "${writeback}" "${total}" >>"${SETTLE_TSV}"
        if (( total <= SETTLE_DIRTY_KB )); then
            sleep "${SETTLE_COOLDOWN_SECONDS}"
            dirty="$(meminfo_kb Dirty)"; writeback="$(meminfo_kb Writeback)"; total=$((dirty + writeback))
            if (( total <= SETTLE_DIRTY_KB )); then
                probe="$(fsync_probe)"; median_ms="${probe%%$'\t'*}"; max_ms="${probe##*$'\t'}"
                if awk -v m="${median_ms}" -v x="${max_ms}" -v ml="${FSYNC_MEDIAN_MAX_MS}" -v xl="${FSYNC_SINGLE_MAX_MS}" 'BEGIN {exit !((m+0)<=ml && (x+0)<=xl)}'; then
                    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\tready\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${attempt}" "${dirty}" "${writeback}" "${total}" "${median_ms}" "${max_ms}" >>"${SETTLE_TSV}"
                    echo "Host recovery OK attempt=${attempt} fsync_median_ms=${median_ms} fsync_max_ms=${max_ms}"
                    return 0
                fi
                printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\tfsync_slow\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${attempt}" "${dirty}" "${writeback}" "${total}" "${median_ms}" "${max_ms}" >>"${SETTLE_TSV}"
                echo "Host fsync probe still slow attempt=${attempt} median_ms=${median_ms} max_ms=${max_ms}"
            fi
        fi
        if (( $(date +%s) - started >= SETTLE_TIMEOUT_SECONDS )); then
            echo "Host did not recover within configured gate attempt=${attempt}" >&2
            return 1
        fi
        sleep "${SETTLE_POLL_SECONDS}"
    done
}

abs_pct_delta() {
    local before="$1" after="$2"
    awk -v a="${before}" -v b="${after}" 'BEGIN {if ((a+0)==0) {print 999999; exit}; d=((b/a)-1)*100; if (d<0) d=-d; printf "%.2f\n", d}'
}
pct_delta() {
    local base="$1" candidate="$2"
    awk -v a="${base}" -v b="${candidate}" 'BEGIN {if ((a+0)==0) {print 0; exit}; printf "%.2f\n", ((b/a)-1)*100}'
}
avg2() {
    local a="$1" b="$2"
    awk -v a="${a}" -v b="${b}" 'BEGIN {printf "%.6f\n", (a+b)/2}'
}
max4() {
    awk -v a="$1" -v b="$2" -v c="$3" -v d="$4" 'BEGIN {m=a; if(b>m)m=b; if(c>m)m=c; if(d>m)m=d; printf "%.3f\n", m}'
}
accepted_count() {
    local sentinel="$1"
    awk -F '\t' -v s="${sentinel}" 'NR>1 && $2==s {n++} END {print n+0}' "${ACCEPTED_TSV}"
}

printf '=== FOD POSTGRESQL BLOCK SIZE PAIRED DRIFT STABILITY ===\n'
printf 'design=A-B-B-A\ntarget_per_orientation=%s\nmax_attempts=%s\nwal_sync_stall_ms=%s\nsentinel_write_drift_pct=%s\nsentinel_read_drift_pct=%s\nsentinel_sql_drift_pct=%s\nmax_ratio_spread_pp=%s\nartifact_dir=%s\n' \
    "${TARGET_PER_ORIENTATION}" "${MAX_ATTEMPTS}" "${WAL_SYNC_STALL_MS}" "${SENTINEL_WRITE_DRIFT_PCT}" "${SENTINEL_READ_DRIFT_PCT}" "${SENTINEL_SQL_DRIFT_PCT}" "${MAX_RATIO_SPREAD_PP}" "${ARTIFACT_DIR}"

for ((attempt=1; attempt<=MAX_ATTEMPTS; attempt++)); do
    count8="$(accepted_count 8)"; count32="$(accepted_count 32)"
    if (( count8 >= TARGET_PER_ORIENTATION && count32 >= TARGET_PER_ORIENTATION )); then break; fi
    if ! wait_for_host_recovery "${attempt}"; then break; fi

    if (( attempt % 2 == 1 )); then
        sequence="8 32"; sentinel=8
    else
        sequence="32 8"; sentinel=32
    fi
    attempt_dir="${ARTIFACT_DIR}/attempt-${attempt}"
    comparison_dir="${attempt_dir}/comparison"
    mkdir -p "${attempt_dir}"
    pull_image=0; (( attempt == 1 )) && pull_image="${FOD_PG32_PULL_IMAGE:-1}"

    echo "=== ATTEMPT ${attempt}/${MAX_ATTEMPTS} sentinel=${sentinel} sequence=${sequence} then rotated ==="
    set +e
    FOD_PG32_PULL_IMAGE="${pull_image}" \
    FOD_PG_BLOCK_COMPARISON_SIZES_KB="${sequence}" \
    FOD_PG_BLOCK_COMPARISON_REPEATS=2 \
    FOD_PG_BLOCK_COMPARISON_FOD_BLOCK_SIZE=32768 \
    FOD_PG_BLOCK_COMPARISON_FILE_SIZE="${FOD_PG_BLOCK_COMPARISON_FILE_SIZE:-1G}" \
    FOD_PG_BLOCK_COMPARISON_FIO_BLOCK_SIZE="${FOD_PG_BLOCK_COMPARISON_FIO_BLOCK_SIZE:-512k}" \
    FOD_PG_BLOCK_COMPARISON_PAYLOAD_MODE=random \
    FOD_PG_BLOCK_COMPARISON_ARTIFACT_DIR="${comparison_dir}" \
    FOD_PG_WRITE_PROFILE_WAL_EVERY=1 \
    FOD_CARGO_PROFILE="${FOD_CARGO_PROFILE:-profiling}" \
    FOD_RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}" \
    FOD_REQUIRE_AC_POWER="${FOD_REQUIRE_AC_POWER:-1}" \
        bash "${BASE}" 2>&1 | tee "${attempt_dir}/run.log"
    status=${PIPESTATUS[0]}
    set -e
    (( status == 0 )) || { echo "Attempt ${attempt} failed; retrying."; continue; }

    runs="${comparison_dir}/runs.tsv"
    [[ -s "${runs}" ]] || { echo "Missing ${runs}; retrying."; continue; }
    mapfile -t rows < <(awk -F '\t' 'NR>1 {print}' "${runs}")
    if (( ${#rows[@]} != 4 )); then echo "Expected four A-B-B-A runs, got ${#rows[@]}" >&2; continue; fi

    declare -a pg write pread rread copy insert insertwal wal syncms
    for i in 0 1 2 3; do
        IFS=$'\t' read -r _repeat _order pg[$i] _pgbytes _fodbytes write[$i] pread[$i] rread[$i] _copycalls _copyexec copy[$i] _insertcalls _insertexec insert[$i] insertwal[$i] wal[$i] syncms[$i] _profile <<<"${rows[$i]}"
    done
    if [[ "${pg[0]}" != "${sentinel}" || "${pg[3]}" != "${sentinel}" || "${pg[1]}" == "${sentinel}" || "${pg[2]}" == "${sentinel}" ]]; then
        echo "Unexpected A-B-B-A order in ${runs}" >&2
        continue
    fi

    write_drift="$(abs_pct_delta "${write[0]}" "${write[3]}")"
    pread_drift="$(abs_pct_delta "${pread[0]}" "${pread[3]}")"
    rread_drift="$(abs_pct_delta "${rread[0]}" "${rread[3]}")"
    copy_drift="$(abs_pct_delta "${copy[0]}" "${copy[3]}")"
    insert_drift="$(abs_pct_delta "${insert[0]}" "${insert[3]}")"
    max_sync="$(max4 "${syncms[0]}" "${syncms[1]}" "${syncms[2]}" "${syncms[3]}")"
    quality=accepted
    awk -v v="${max_sync}" -v t="${WAL_SYNC_STALL_MS}" 'BEGIN {exit !((v+0)>t)}' && quality=wal_stall
    awk -v v="${write_drift}" -v t="${SENTINEL_WRITE_DRIFT_PCT}" 'BEGIN {exit !((v+0)>t)}' && quality=host_drift
    awk -v a="${pread_drift}" -v b="${rread_drift}" -v t="${SENTINEL_READ_DRIFT_PCT}" 'BEGIN {exit !((a+0)>t || (b+0)>t)}' && quality=host_drift
    awk -v a="${copy_drift}" -v b="${insert_drift}" -v t="${SENTINEL_SQL_DRIFT_PCT}" 'BEGIN {exit !((a+0)>t || (b+0)>t)}' && quality=host_drift

    s_write="$(avg2 "${write[0]}" "${write[3]}")"; b_write="$(avg2 "${write[1]}" "${write[2]}")"
    s_pread="$(avg2 "${pread[0]}" "${pread[3]}")"; b_pread="$(avg2 "${pread[1]}" "${pread[2]}")"
    s_rread="$(avg2 "${rread[0]}" "${rread[3]}")"; b_rread="$(avg2 "${rread[1]}" "${rread[2]}")"
    s_copy="$(avg2 "${copy[0]}" "${copy[3]}")"; b_copy="$(avg2 "${copy[1]}" "${copy[2]}")"
    s_insert="$(avg2 "${insert[0]}" "${insert[3]}")"; b_insert="$(avg2 "${insert[1]}" "${insert[2]}")"
    s_insertwal="$(avg2 "${insertwal[0]}" "${insertwal[3]}")"; b_insertwal="$(avg2 "${insertwal[1]}" "${insertwal[2]}")"
    s_wal="$(avg2 "${wal[0]}" "${wal[3]}")"; b_wal="$(avg2 "${wal[1]}" "${wal[2]}")"
    s_sync="$(avg2 "${syncms[0]}" "${syncms[3]}")"; b_sync="$(avg2 "${syncms[1]}" "${syncms[2]}")"

    if (( sentinel == 8 )); then
        p_write="$(pct_delta "${s_write}" "${b_write}")"; p_pread="$(pct_delta "${s_pread}" "${b_pread}")"; p_rread="$(pct_delta "${s_rread}" "${b_rread}")"
        p_copy="$(pct_delta "${s_copy}" "${b_copy}")"; p_insert="$(pct_delta "${s_insert}" "${b_insert}")"; p_insertwal="$(pct_delta "${s_insertwal}" "${b_insertwal}")"; p_wal="$(pct_delta "${s_wal}" "${b_wal}")"; p_sync="$(pct_delta "${s_sync}" "${b_sync}")"
    else
        p_write="$(pct_delta "${b_write}" "${s_write}")"; p_pread="$(pct_delta "${b_pread}" "${s_pread}")"; p_rread="$(pct_delta "${b_rread}" "${s_rread}")"
        p_copy="$(pct_delta "${b_copy}" "${s_copy}")"; p_insert="$(pct_delta "${b_insert}" "${s_insert}")"; p_insertwal="$(pct_delta "${b_insertwal}" "${s_insertwal}")"; p_wal="$(pct_delta "${b_wal}" "${s_wal}")"; p_sync="$(pct_delta "${b_sync}" "${s_sync}")"
    fi

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "${attempt}" "${sentinel}" "${sequence}" "${quality}" "${write_drift}" "${pread_drift}" "${rread_drift}" "${copy_drift}" "${insert_drift}" "${max_sync}" \
        "${p_write}" "${p_pread}" "${p_rread}" "${p_copy}" "${p_insert}" "${p_insertwal}" "${p_wal}" "${p_sync}" "${comparison_dir}" >>"${ROUNDS_TSV}"
    if [[ "${quality}" == "accepted" ]]; then
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "${attempt}" "${sentinel}" "${p_write}" "${p_pread}" "${p_rread}" "${p_copy}" "${p_insert}" "${p_insertwal}" "${p_wal}" "${p_sync}" "${comparison_dir}" >>"${ACCEPTED_TSV}"
    fi
done

metric_summary() {
    local name="$1" column="$2"
    awk -F '\t' -v c="${column}" 'NR>1 {v[++n]=$c} END {for(i=1;i<=n;i++) for(j=i+1;j<=n;j++) if(v[j]<v[i]){t=v[i];v[i]=v[j];v[j]=t} if(n==0){print "0\t0\t0\t0"; exit} if(n%2)m=v[(n+1)/2]; else m=(v[n/2]+v[n/2+1])/2; printf "%.2f\t%.2f\t%.2f\t%.2f\n",m,v[1],v[n],v[n]-v[1]}' "${ACCEPTED_TSV}" | awk -v n="${name}" '{print n"\t"$0}' >>"${SUMMARY_TSV}"
}

selection_status=valid
count8="$(accepted_count 8)"; count32="$(accepted_count 32)"
if (( count8 < TARGET_PER_ORIENTATION || count32 < TARGET_PER_ORIENTATION )); then selection_status=insufficient_stable_rounds; fi
if [[ "${selection_status}" == "valid" ]]; then
    metric_summary primary_write 3
    metric_summary primary_read 4
    metric_summary replica_read 5
    metric_summary copy_mean 6
    metric_summary insert_mean 7
    metric_summary insert_wal_bytes 8
    metric_summary wal_bytes 9
    metric_summary wal_sync_time 10
    write_spread="$(awk -F '\t' '$1=="primary_write" {print $5}' "${SUMMARY_TSV}")"
    if awk -v s="${write_spread}" -v m="${MAX_RATIO_SPREAD_PP}" 'BEGIN {exit !((s+0)>m)}'; then selection_status=unstable_pair_ratio; fi
fi

echo
echo "=== A-B-B-A ROUNDS ==="; cat "${ROUNDS_TSV}"
echo; echo "=== ACCEPTED ROUNDS ==="; cat "${ACCEPTED_TSV}"
echo; echo "=== PAIRED RATIO SUMMARY ==="; cat "${SUMMARY_TSV}"
echo "selection_status=${selection_status}"
echo "paired_stability_artifact_dir=${ARTIFACT_DIR}"
if [[ "${selection_status}" != "valid" ]]; then echo "INVALID: PostgreSQL block-size paired stability ${selection_status}" >&2; exit 1; fi
echo "OK: PostgreSQL block-size paired stability"
