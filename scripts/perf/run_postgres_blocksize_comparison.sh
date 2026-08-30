#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Compare PostgreSQL compile-time table block sizes while keeping FOD fixed at
# 32 KiB. Both PostgreSQL variants are built from the same Dockerfile/source
# version. Images and the FOD runtime are built before measured runs.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

PG_BLOCK_SIZES_KB="${FOD_PG_BLOCK_COMPARISON_SIZES_KB:-8 32}"
REPEATS="${FOD_PG_BLOCK_COMPARISON_REPEATS:-3}"
FOD_BLOCK_SIZE="${FOD_PG_BLOCK_COMPARISON_FOD_BLOCK_SIZE:-32768}"
FILE_SIZE="${FOD_PG_BLOCK_COMPARISON_FILE_SIZE:-1G}"
FIO_BLOCK_SIZE="${FOD_PG_BLOCK_COMPARISON_FIO_BLOCK_SIZE:-512k}"
PAYLOAD_MODE="${FOD_PG_BLOCK_COMPARISON_PAYLOAD_MODE:-random}"
RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}"
CARGO_PROFILE="${FOD_CARGO_PROFILE:-${RUNTIME_PROFILE}}"
SETTLE_DIRTY_KB="${FOD_PG_BLOCK_COMPARISON_SETTLE_DIRTY_KB:-32768}"
SETTLE_TIMEOUT_SECONDS="${FOD_PG_BLOCK_COMPARISON_SETTLE_TIMEOUT_SECONDS:-180}"
SETTLE_IDLE_SECONDS="${FOD_PG_BLOCK_COMPARISON_SETTLE_IDLE_SECONDS:-5}"
COMPOSE_FILE="${ROOT}/docker-compose.postgres-blocksize.yml"
MATRIX="${ROOT}/scripts/perf/run_random_storage_block_matrix.sh"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
ARTIFACT_DIR="${FOD_PG_BLOCK_COMPARISON_ARTIFACT_DIR:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-postgres-blocksize-${RUN_ID}}"

for cmd in bash docker make git awk sort tee sync sleep date mktemp; do
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

positive_integer FOD_PG_BLOCK_COMPARISON_REPEATS "${REPEATS}"
positive_integer FOD_PG_BLOCK_COMPARISON_SETTLE_DIRTY_KB "${SETTLE_DIRTY_KB}"
positive_integer FOD_PG_BLOCK_COMPARISON_SETTLE_TIMEOUT_SECONDS "${SETTLE_TIMEOUT_SECONDS}"
positive_integer FOD_PG_BLOCK_COMPARISON_SETTLE_IDLE_SECONDS "${SETTLE_IDLE_SECONDS}"

if [[ "${PAYLOAD_MODE}" != "random" ]]; then
    echo "PostgreSQL block-size comparison requires random payloads; got ${PAYLOAD_MODE}" >&2
    exit 2
fi

case "${FOD_BLOCK_SIZE}" in
    ''|*[!0-9]*) echo "Invalid FOD block size: ${FOD_BLOCK_SIZE}" >&2; exit 2 ;;
esac
if (( FOD_BLOCK_SIZE < 1024 || FOD_BLOCK_SIZE % 1024 != 0 )); then
    echo "FOD block size must be a positive multiple of 1024" >&2
    exit 2
fi

read -r -a PG_BLOCKS <<<"${PG_BLOCK_SIZES_KB}"
if (( ${#PG_BLOCKS[@]} < 2 )); then
    echo "At least two PostgreSQL block sizes are required" >&2
    exit 2
fi
for pg_block_kb in "${PG_BLOCKS[@]}"; do
    case "${pg_block_kb}" in
        1|2|4|8|16|32) ;;
        *) echo "PostgreSQL block size must be one of 1,2,4,8,16,32 KiB: ${pg_block_kb}" >&2; exit 2 ;;
    esac
done

if [[ ! -r /proc/meminfo ]]; then
    echo "/proc/meminfo is required for host writeback stabilization" >&2
    exit 2
fi

mkdir -p "${ARTIFACT_DIR}"
RUNS="${ARTIFACT_DIR}/runs.tsv"
MEDIANS="${ARTIFACT_DIR}/median.tsv"
BUILDS="${ARTIFACT_DIR}/builds.tsv"
SETTLE_LOG="${ARTIFACT_DIR}/settle.tsv"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/fod-pg-blocksize.XXXXXX")"
NO_BUILD_COMPOSE="${TMP_DIR}/docker-compose-no-build"

cleanup() {
    local rc=$?
    set +e
    rm -rf "${TMP_DIR}"
    exit "${rc}"
}
trap cleanup EXIT INT TERM

cat >"${NO_BUILD_COMPOSE}" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail
args=()
for arg in "$@"; do
    if [[ "${arg}" == "--build" ]]; then
        continue
    fi
    args+=("${arg}")
done
exec docker compose "${args[@]}"
WRAPPER
chmod 0700 "${NO_BUILD_COMPOSE}"

printf 'pg_block_size_kb\tpg_block_size_bytes\timage\tpostgres_version\tverified_block_size_bytes\n' >"${BUILDS}"
printf 'timestamp\tlabel\tdirty_kb\twriteback_kb\ttotal_kb\n' >"${SETTLE_LOG}"
printf 'repeat\torder\tpg_block_size_kb\tpg_block_size_bytes\tfod_block_size_bytes\tprimary_write_mib_s\tprimary_read_mib_s\treplica_read_mib_s\tcopy_calls\tcopy_exec_ms\tcopy_mean_ms\tinsert_calls\tinsert_exec_ms\tinsert_mean_ms\tinsert_wal_bytes\twal_bytes_delta\twal_sync_time_delta_ms\tprofile_artifact_dir\n' >"${RUNS}"
printf 'pg_block_size_kb\truns\tmedian_primary_write_mib_s\tmin_primary_write_mib_s\tmax_primary_write_mib_s\twrite_spread_pct\tmedian_primary_read_mib_s\tmedian_replica_read_mib_s\tmedian_copy_mean_ms\tmedian_insert_mean_ms\tmedian_insert_wal_bytes\tmedian_wal_bytes_delta\tmedian_wal_sync_time_delta_ms\n' >"${MEDIANS}"

printf '=== FOD POSTGRESQL BLOCK SIZE COMPARISON ===\n'
printf 'postgres_block_sizes_kb=%s\n' "${PG_BLOCK_SIZES_KB}"
printf 'repeats=%s\n' "${REPEATS}"
printf 'fod_block_size_bytes=%s\n' "${FOD_BLOCK_SIZE}"
printf 'fio_block_size=%s\nfile_size=%s\npayload_mode=%s\n' \
    "${FIO_BLOCK_SIZE}" "${FILE_SIZE}" "${PAYLOAD_MODE}"
printf 'artifact_dir=%s\n' "${ARTIFACT_DIR}"

meminfo_kb() {
    local key="$1"
    awk -v key="${key}:" '$1 == key {print $2 + 0; found=1; exit} END {if (!found) print 0}' /proc/meminfo
}

record_settle() {
    local label="$1"
    local dirty="$2"
    local writeback="$3"
    printf '%s\t%s\t%s\t%s\t%s\n' \
        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${label}" "${dirty}" "${writeback}" "$((dirty + writeback))" \
        >>"${SETTLE_LOG}"
}

settle_host_io() {
    local label="$1"
    local started dirty writeback total confirm_dirty confirm_writeback
    echo "Host settle start label=${label}: sync + Dirty/Writeback <= ${SETTLE_DIRTY_KB} KiB"
    sync
    started="$(date +%s)"
    while true; do
        dirty="$(meminfo_kb Dirty)"
        writeback="$(meminfo_kb Writeback)"
        total=$((dirty + writeback))
        record_settle "${label}" "${dirty}" "${writeback}"
        if (( total <= SETTLE_DIRTY_KB )); then
            sleep "${SETTLE_IDLE_SECONDS}"
            confirm_dirty="$(meminfo_kb Dirty)"
            confirm_writeback="$(meminfo_kb Writeback)"
            record_settle "${label}-confirm" "${confirm_dirty}" "${confirm_writeback}"
            if (( confirm_dirty + confirm_writeback <= SETTLE_DIRTY_KB )); then
                echo "Host settle OK label=${label} dirty_kb=${confirm_dirty} writeback_kb=${confirm_writeback}"
                return 0
            fi
        fi
        if (( $(date +%s) - started >= SETTLE_TIMEOUT_SECONDS )); then
            echo "Host did not settle within ${SETTLE_TIMEOUT_SECONDS}s label=${label}" >&2
            return 1
        fi
        sleep 1
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

wal_delta() {
    local file="$1"
    local column="$2"
    local format="${3:-float}"
    if [[ ! -s "${file}" ]] || (( $(awk 'END {print NR}' "${file}") < 2 )); then
        printf '0\n'
        return
    fi
    if [[ "${format}" == "int" ]]; then
        awk -F '\t' -v column="${column}" 'NR == 1 {first=$column} {last=$column} END {printf "%.0f\n", (last + 0) - (first + 0)}' "${file}"
    else
        awk -F '\t' -v column="${column}" 'NR == 1 {first=$column} {last=$column} END {printf "%.3f\n", (last + 0) - (first + 0)}' "${file}"
    fi
}

values_for() {
    local pg_kb="$1"
    local column="$2"
    awk -F '\t' -v pg_kb="${pg_kb}" -v column="${column}" 'NR > 1 && $3 == pg_kb {print $column}' "${RUNS}"
}

median_for() {
    local pg_kb="$1"
    local column="$2"
    values_for "${pg_kb}" "${column}" | sort -n | awk '
        {v[NR]=$1}
        END {
            if (NR == 0) {print "0"; exit}
            if (NR % 2) printf "%.3f\n", v[(NR + 1) / 2];
            else printf "%.3f\n", (v[NR / 2] + v[NR / 2 + 1]) / 2;
        }'
}

min_for() {
    local pg_kb="$1"
    local column="$2"
    values_for "${pg_kb}" "${column}" | awk '{if (!seen || $1 < min) min=$1; seen=1} END {if (seen) printf "%.3f\n", min; else print "0"}'
}

max_for() {
    local pg_kb="$1"
    local column="$2"
    values_for "${pg_kb}" "${column}" | awk '{if (!seen || $1 > max) max=$1; seen=1} END {if (seen) printf "%.3f\n", max; else print "0"}'
}

count_for() {
    local pg_kb="$1"
    awk -F '\t' -v pg_kb="${pg_kb}" 'NR > 1 && $3 == pg_kb {n++} END {print n + 0}' "${RUNS}"
}

percent_delta() {
    local base="$1"
    local candidate="$2"
    awk -v base="${base}" -v candidate="${candidate}" 'BEGIN {
        if ((base + 0) == 0) {print "0"; exit}
        printf "%.2f\n", ((candidate + 0) / (base + 0) - 1.0) * 100.0;
    }'
}

# Keep FOD compilation outside the measured series.
FOD_CARGO_PROFILE="${CARGO_PROFILE}" \
FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" \
make --no-print-directory build-runtime

# Build and verify every PostgreSQL BLCKSZ image before the measured series.
for pg_block_kb in "${PG_BLOCKS[@]}"; do
    expected_bytes=$((pg_block_kb * 1024))
    image="fod-postgres-blocksize:16-${pg_block_kb}k"
    echo "=== BUILD POSTGRESQL ${pg_block_kb} KiB ==="
    POSTGRES_BLOCK_SIZE_KB="${pg_block_kb}" \
    FOD_EXPECTED_PG_BLOCK_SIZE_BYTES="${expected_bytes}" \
        docker compose -f "${COMPOSE_FILE}" build primary

    postgres_version="$(docker run --rm --entrypoint postgres "${image}" --version)"
    verified_bytes="$(docker run --rm --entrypoint /bin/sh "${image}" -ceu '
        dir="$(mktemp -d)"
        chown postgres:postgres "${dir}"
        su-exec postgres initdb -D "${dir}" >/dev/null
        su-exec postgres postgres -D "${dir}" -C block_size
    ')"
    verified_bytes="$(printf '%s' "${verified_bytes}" | tail -n 1 | tr -d '[:space:]')"
    if [[ "${verified_bytes}" != "${expected_bytes}" ]]; then
        echo "PostgreSQL image block-size verification failed image=${image} expected=${expected_bytes} actual=${verified_bytes}" >&2
        exit 1
    fi
    printf '%s\t%s\t%s\t%s\t%s\n' \
        "${pg_block_kb}" "${expected_bytes}" "${image}" "${postgres_version}" "${verified_bytes}" \
        >>"${BUILDS}"
done

block_count=${#PG_BLOCKS[@]}
for ((repeat=1; repeat<=REPEATS; repeat++)); do
    rotation=$(( (repeat - 1) % block_count ))
    printf '\n=== REPEAT %s/%s rotation=%s ===\n' "${repeat}" "${REPEATS}" "${rotation}"

    for ((offset=0; offset<block_count; offset++)); do
        index=$(( (rotation + offset) % block_count ))
        pg_block_kb="${PG_BLOCKS[$index]}"
        pg_block_bytes=$((pg_block_kb * 1024))
        order=$((offset + 1))
        run_dir="${ARTIFACT_DIR}/repeat-${repeat}/pg-${pg_block_kb}k"
        run_log="${run_dir}/run.log"
        mkdir -p "${run_dir}"

        printf '\n--- repeat=%s order=%s postgres_block=%sKiB FOD_block=%s ---\n' \
            "${repeat}" "${order}" "${pg_block_kb}" "${FOD_BLOCK_SIZE}"
        settle_host_io "pre-r${repeat}-o${order}-pg${pg_block_kb}k"

        set +e
        POSTGRES_BLOCK_SIZE_KB="${pg_block_kb}" \
        FOD_EXPECTED_PG_BLOCK_SIZE_BYTES="${pg_block_bytes}" \
        REPLICA_READ_COMPOSE_FILE="docker-compose.postgres-blocksize.yml" \
        FOD_REPLICA_READ_COMPOSE="${NO_BUILD_COMPOSE}" \
        FOD_CARGO_PROFILE="${CARGO_PROFILE}" \
        FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" \
        FOD_STORAGE_BLOCK_SKIP_BUILD=1 \
        FOD_STORAGE_BLOCK_SIZES="${FOD_BLOCK_SIZE}" \
        FOD_STORAGE_BLOCK_FILE_SIZE="${FILE_SIZE}" \
        FOD_STORAGE_BLOCK_FIO_BLOCK_SIZE="${FIO_BLOCK_SIZE}" \
        FOD_STORAGE_BLOCK_PAYLOAD_MODE="${PAYLOAD_MODE}" \
        FOD_STORAGE_BLOCK_ARTIFACT_DIR="${run_dir}" \
        FOD_REQUIRE_AC_POWER="${FOD_REQUIRE_AC_POWER:-1}" \
            bash "${MATRIX}" 2>&1 | tee "${run_log}"
        status=${PIPESTATUS[0]}
        set -e
        if [[ "${status}" -ne 0 ]]; then
            echo "PostgreSQL block-size run failed repeat=${repeat} pg_block=${pg_block_kb}KiB log=${run_log}" >&2
            exit "${status}"
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
        copy_exec="$(tsv_value "${summary}" 10)"
        insert_calls="$(tsv_value "${summary}" 12)"
        insert_exec="$(tsv_value "${summary}" 14)"
        insert_wal_bytes="$(tsv_value "${summary}" 16)"
        profile_dir="$(tsv_value "${summary}" 17)"
        copy_mean="$(ratio_ms "${copy_exec}" "${copy_calls}")"
        insert_mean="$(ratio_ms "${insert_exec}" "${insert_calls}")"
        wal_file="${profile_dir}/wal.tsv"
        wal_bytes_delta="$(wal_delta "${wal_file}" 4 int)"
        wal_sync_time_delta="$(wal_delta "${wal_file}" 9 float)"

        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "${repeat}" "${order}" "${pg_block_kb}" "${pg_block_bytes}" "${FOD_BLOCK_SIZE}" \
            "${primary_write}" "${primary_read}" "${replica_read}" \
            "${copy_calls}" "${copy_exec}" "${copy_mean}" \
            "${insert_calls}" "${insert_exec}" "${insert_mean}" "${insert_wal_bytes}" \
            "${wal_bytes_delta}" "${wal_sync_time_delta}" "${profile_dir}" \
            >>"${RUNS}"
    done
done

for pg_block_kb in "${PG_BLOCKS[@]}"; do
    runs="$(count_for "${pg_block_kb}")"
    median_write="$(median_for "${pg_block_kb}" 6)"
    min_write="$(min_for "${pg_block_kb}" 6)"
    max_write="$(max_for "${pg_block_kb}" 6)"
    spread="$(awk -v min="${min_write}" -v max="${max_write}" -v median="${median_write}" 'BEGIN {if ((median + 0) == 0) print "0"; else printf "%.2f\n", ((max + 0) - (min + 0)) / (median + 0) * 100.0}')"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "${pg_block_kb}" "${runs}" "${median_write}" "${min_write}" "${max_write}" "${spread}" \
        "$(median_for "${pg_block_kb}" 7)" \
        "$(median_for "${pg_block_kb}" 8)" \
        "$(median_for "${pg_block_kb}" 11)" \
        "$(median_for "${pg_block_kb}" 14)" \
        "$(median_for "${pg_block_kb}" 15)" \
        "$(median_for "${pg_block_kb}" 16)" \
        "$(median_for "${pg_block_kb}" 17)" \
        >>"${MEDIANS}"
done

echo
echo "=== POSTGRESQL BLOCK SIZE BUILDS ==="
cat "${BUILDS}"
echo
echo "=== POSTGRESQL BLOCK SIZE RUNS ==="
cat "${RUNS}"
echo
echo "=== POSTGRESQL BLOCK SIZE MEDIANS ==="
cat "${MEDIANS}"

if grep -q $'^8\t' "${MEDIANS}" && grep -q $'^32\t' "${MEDIANS}"; then
    write_8="$(awk -F '\t' '$1 == 8 {print $3}' "${MEDIANS}")"
    write_32="$(awk -F '\t' '$1 == 32 {print $3}' "${MEDIANS}")"
    insert_8="$(awk -F '\t' '$1 == 8 {print $10}' "${MEDIANS}")"
    insert_32="$(awk -F '\t' '$1 == 32 {print $10}' "${MEDIANS}")"
    wal_8="$(awk -F '\t' '$1 == 8 {print $12}' "${MEDIANS}")"
    wal_32="$(awk -F '\t' '$1 == 32 {print $12}' "${MEDIANS}")"
    echo "postgres_32k_vs_8k_primary_write_pct=$(percent_delta "${write_8}" "${write_32}")"
    echo "postgres_32k_vs_8k_insert_mean_pct=$(percent_delta "${insert_8}" "${insert_32}")"
    echo "postgres_32k_vs_8k_wal_bytes_pct=$(percent_delta "${wal_8}" "${wal_32}")"
fi

echo "comparison_artifact_dir=${ARTIFACT_DIR}"
echo "OK: PostgreSQL block-size comparison"
