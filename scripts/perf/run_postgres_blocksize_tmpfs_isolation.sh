#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Isolate PostgreSQL BLCKSZ effects from host SSD sustained-write variability.
# Primary and replica PGDATA live in host /dev/shm directories bind-mounted into
# the containers. This keeps them RAM-backed while preserving PGDATA across the
# container restarts required by the primary/replica benchmark.
# Measured artifacts are collected in /dev/shm and copied to the repository only
# after all measured runs finish. This is NOT a durability/storage benchmark;
# fsync on tmpfs is not representative of persistent media. It is intended only
# to compare PostgreSQL 8K vs 32K CPU, executor/page-layout, WAL volume and FOD
# throughput without the SSD bottleneck.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

COMPOSE="${ROOT}/docker-compose.postgres-blocksize-tmpfs.yml"
MATRIX="${ROOT}/scripts/perf/run_random_storage_block_matrix.sh"
PREFLIGHT="${ROOT}/scripts/perf/check_postgres_blocksize_tmpfs_runtime.sh"
PUBLISHED_32K="${FOD_PG32_IMAGE:-ghcr.io/stachwk/postgres-16-fod-32k:16.15}"
REPEATS="${FOD_PG_BLOCK_TMPFS_REPEATS:-3}"
FILE_SIZE="${FOD_PG_BLOCK_TMPFS_FILE_SIZE:-512M}"
FIO_BLOCK_SIZE="${FOD_PG_BLOCK_TMPFS_FIO_BLOCK_SIZE:-512k}"
FOD_BLOCK_SIZE="${FOD_PG_BLOCK_TMPFS_FOD_BLOCK_SIZE:-32768}"
RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}"
CARGO_PROFILE="${FOD_CARGO_PROFILE:-${RUNTIME_PROFILE}}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
FINAL_DIR="${FOD_PG_BLOCK_TMPFS_ARTIFACT_DIR:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-postgres-blocksize-tmpfs-${RUN_ID}}"
RAM_ROOT="${FOD_PG_BLOCK_TMPFS_RAM_ROOT:-/dev/shm/fod-postgres-blocksize-${RUN_ID}-${BASHPID}}"
RUNS="${RAM_ROOT}/runs.tsv"
MEDIANS="${RAM_ROOT}/median.tsv"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/fod-pg-tmpfs.XXXXXX")"
NO_BUILD_COMPOSE="${TMP_DIR}/docker-compose-no-build"

for cmd in bash docker make git awk sort sync sleep cp mkdir mktemp rm; do
    command -v "${cmd}" >/dev/null 2>&1 || { echo "Missing required command: ${cmd}" >&2; exit 2; }
done
[[ -r "${COMPOSE}" ]] || { echo "Missing tmpfs compose: ${COMPOSE}" >&2; exit 2; }
[[ -r "${MATRIX}" ]] || { echo "Missing matrix runner: ${MATRIX}" >&2; exit 2; }
[[ -r "${PREFLIGHT}" ]] || { echo "Missing tmpfs runtime preflight: ${PREFLIGHT}" >&2; exit 2; }
[[ -d /dev/shm && -w /dev/shm ]] || { echo "/dev/shm must be writable" >&2; exit 2; }
case "${REPEATS}" in ''|*[!0-9]*) echo "FOD_PG_BLOCK_TMPFS_REPEATS must be a positive integer" >&2; exit 2;; esac
(( REPEATS >= 1 )) || { echo "FOD_PG_BLOCK_TMPFS_REPEATS must be >= 1" >&2; exit 2; }
case "${RAM_ROOT}" in /dev/shm/*) ;; *) echo "FOD_PG_BLOCK_TMPFS_RAM_ROOT must be under /dev/shm: ${RAM_ROOT}" >&2; exit 2;; esac

cleanup() {
    local rc=$?
    set +e
    rm -rf "${TMP_DIR}"
    if [[ "${FOD_PG_BLOCK_TMPFS_KEEP_RAM_ARTIFACTS:-0}" != "1" ]]; then
        rm -rf "${RAM_ROOT}"
    fi
    exit "${rc}"
}
trap cleanup EXIT INT TERM

mkdir -p "${RAM_ROOT}"
cat >"${NO_BUILD_COMPOSE}" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail
args=()
for arg in "$@"; do
    [[ "${arg}" == "--build" ]] && continue
    args+=("${arg}")
done
exec docker compose "${args[@]}"
WRAPPER
chmod 0700 "${NO_BUILD_COMPOSE}"

printf 'repeat\torder\tpg_block_size_kb\tprimary_write_mib_s\tprimary_read_mib_s\treplica_read_mib_s\tcopy_mean_ms\tinsert_mean_ms\tinsert_wal_bytes\twal_bytes_delta\twal_sync_time_delta_ms\tprofile_artifact_dir\n' >"${RUNS}"
printf 'pg_block_size_kb\truns\tmedian_primary_write_mib_s\twrite_spread_pct\tmedian_primary_read_mib_s\tmedian_replica_read_mib_s\tmedian_copy_mean_ms\tmedian_insert_mean_ms\tmedian_insert_wal_bytes\tmedian_wal_bytes_delta\tmedian_wal_sync_time_delta_ms\n' >"${MEDIANS}"

median_values() {
    sort -n | awk '{v[NR]=$1} END {if(NR==0){print 0}else if(NR%2){printf "%.3f\n",v[(NR+1)/2]}else{printf "%.3f\n",(v[NR/2]+v[NR/2+1])/2}}'
}
values_for() {
    local pg="$1" col="$2"
    awk -F '\t' -v pg="${pg}" -v col="${col}" 'NR>1 && $3==pg {print $col}' "${RUNS}"
}
median_for() { values_for "$1" "$2" | median_values; }
min_for() { values_for "$1" "$2" | awk '{if(!s||$1<m)m=$1;s=1} END{if(s)printf "%.3f\n",m;else print 0}'; }
max_for() { values_for "$1" "$2" | awk '{if(!s||$1>m)m=$1;s=1} END{if(s)printf "%.3f\n",m;else print 0}'; }
pct_delta() { awk -v a="$1" -v b="$2" 'BEGIN{if((a+0)==0){print 0}else printf "%.2f\n",((b/a)-1)*100}'; }
wal_delta() {
    local file="$1" col="$2" fmt="${3:-float}"
    [[ -s "${file}" ]] || { echo 0; return; }
    if [[ "${fmt}" == int ]]; then
        awk -F '\t' -v c="${col}" 'NR==1{a=$c}{b=$c} END{printf "%.0f\n",b-a}' "${file}"
    else
        awk -F '\t' -v c="${col}" 'NR==1{a=$c}{b=$c} END{printf "%.3f\n",b-a}' "${file}"
    fi
}

printf '=== POSTGRESQL BLOCK SIZE TMPFS ISOLATION ===\n'
printf 'warning=tmpfs_lab_not_durability_benchmark\n'
printf 'postgres_32k_image=%s\nrepeats=%s\nfile_size=%s\nfio_block_size=%s\nfod_block_size=%s\nram_artifact_dir=%s\nfinal_artifact_dir=%s\n' \
    "${PUBLISHED_32K}" "${REPEATS}" "${FILE_SIZE}" "${FIO_BLOCK_SIZE}" "${FOD_BLOCK_SIZE}" "${RAM_ROOT}" "${FINAL_DIR}"

# Compile FOD once before measurements.
FOD_CARGO_PROFILE="${CARGO_PROFILE}" FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" make --no-print-directory build-runtime

# Reuse the published 32K image and build only the 8K control from the same Dockerfile.
if [[ "${FOD_PG32_PULL_IMAGE:-1}" == "1" ]]; then
    docker pull "${PUBLISHED_32K}"
fi
docker tag "${PUBLISHED_32K}" fod-postgres-blocksize:16-32k
build_primary_dir="${RAM_ROOT}/compose-build-primary"
build_replica_dir="${RAM_ROOT}/compose-build-replica"
mkdir -p "${build_primary_dir}" "${build_replica_dir}"
FOD_PG_BLOCK_TMPFS_PRIMARY_DIR="${build_primary_dir}" \
FOD_PG_BLOCK_TMPFS_REPLICA_DIR="${build_replica_dir}" \
POSTGRES_BLOCK_SIZE_KB=8 FOD_EXPECTED_PG_BLOCK_SIZE_BYTES=8192 \
    docker compose -f "${COMPOSE}" build primary
rm -rf "${build_primary_dir}" "${build_replica_dir}"

for pg in 8 32; do
    image="fod-postgres-blocksize:16-${pg}k"
    expected=$((pg * 1024))
    actual="$(docker run --rm --entrypoint /bin/sh "${image}" -ceu '
      d="$(mktemp -d)"; chown postgres:postgres "$d";
      su-exec postgres initdb --no-sync -D "$d" >/dev/null;
      su-exec postgres postgres -D "$d" -C block_size
    ' | tail -n 1 | tr -d '[:space:]')"
    [[ "${actual}" == "${expected}" ]] || { echo "block_size verify failed pg=${pg} expected=${expected} actual=${actual}" >&2; exit 1; }
    echo "verified_pg${pg}k_block_size=${actual}"
done

# Validate the exact property the benchmark requires: RAM-backed PGDATA must
# survive docker restart within a measured run.
bash "${PREFLIGHT}"

for ((repeat=1; repeat<=REPEATS; repeat++)); do
    if (( repeat % 2 == 1 )); then order_list=(8 32); else order_list=(32 8); fi
    echo "=== REPEAT ${repeat}/${REPEATS} order=${order_list[*]} ==="
    order=0
    for pg in "${order_list[@]}"; do
        order=$((order + 1))
        expected=$((pg * 1024))
        run_dir="${RAM_ROOT}/repeat-${repeat}/pg-${pg}k"
        primary_dir="${run_dir}/.pgdata-primary"
        replica_dir="${run_dir}/.pgdata-replica"
        rm -rf "${primary_dir}" "${replica_dir}"
        mkdir -p "${run_dir}" "${primary_dir}" "${replica_dir}"
        sync
        sleep "${FOD_PG_BLOCK_TMPFS_IDLE_SECONDS:-5}"
        echo "--- repeat=${repeat} order=${order} pg=${pg}KiB ---"

        POSTGRES_BLOCK_SIZE_KB="${pg}" \
        FOD_EXPECTED_PG_BLOCK_SIZE_BYTES="${expected}" \
        FOD_PG_BLOCK_TMPFS_PRIMARY_DIR="${primary_dir}" \
        FOD_PG_BLOCK_TMPFS_REPLICA_DIR="${replica_dir}" \
        REPLICA_READ_COMPOSE_FILE="docker-compose.postgres-blocksize-tmpfs.yml" \
        FOD_REPLICA_READ_COMPOSE="${NO_BUILD_COMPOSE}" \
        FOD_CARGO_PROFILE="${CARGO_PROFILE}" \
        FOD_RUNTIME_PROFILE="${RUNTIME_PROFILE}" \
        FOD_STORAGE_BLOCK_SKIP_BUILD=1 \
        FOD_STORAGE_BLOCK_SIZES="${FOD_BLOCK_SIZE}" \
        FOD_STORAGE_BLOCK_FILE_SIZE="${FILE_SIZE}" \
        FOD_STORAGE_BLOCK_FIO_BLOCK_SIZE="${FIO_BLOCK_SIZE}" \
        FOD_STORAGE_BLOCK_PAYLOAD_MODE=random \
        FOD_STORAGE_BLOCK_ARTIFACT_DIR="${run_dir}" \
        FOD_PG_WRITE_PROFILE_WAL_EVERY=1 \
        FOD_REQUIRE_AC_POWER="${FOD_REQUIRE_AC_POWER:-1}" \
            bash "${MATRIX}"

        summary="${run_dir}/summary.tsv"
        [[ -s "${summary}" ]] || { echo "Missing ${summary}" >&2; exit 1; }
        row="$(awk -F '\t' 'NR==2{print}' "${summary}")"
        IFS=$'\t' read -r _storage _fio _size _payload write pread rread copy_calls _copy_rows copy_exec _copy_blks insert_calls _insert_rows insert_exec _insert_blks insert_wal profile_dir <<<"${row}"
        copy_mean="$(awk -v t="${copy_exec}" -v c="${copy_calls}" 'BEGIN{if(c>0)printf "%.3f\n",t/c;else print 0}')"
        insert_mean="$(awk -v t="${insert_exec}" -v c="${insert_calls}" 'BEGIN{if(c>0)printf "%.3f\n",t/c;else print 0}')"
        wal_file="${profile_dir}/wal.tsv"
        wal_bytes="$(wal_delta "${wal_file}" 4 int)"
        wal_sync="$(wal_delta "${wal_file}" 9 float)"
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "${repeat}" "${order}" "${pg}" "${write}" "${pread}" "${rread}" "${copy_mean}" "${insert_mean}" "${insert_wal}" "${wal_bytes}" "${wal_sync}" "${profile_dir}" >>"${RUNS}"

        # The integration test has already stopped the Compose project here.
        # Remove only the measured run's RAM-backed PGDATA so it is neither
        # reused by another variant nor copied into the final disk artifact.
        rm -rf "${primary_dir}" "${replica_dir}"
    done
done

for pg in 8 32; do
    n="$(awk -F '\t' -v pg="${pg}" 'NR>1&&$3==pg{n++}END{print n+0}' "${RUNS}")"
    medw="$(median_for "${pg}" 4)"; minw="$(min_for "${pg}" 4)"; maxw="$(max_for "${pg}" 4)"
    spread="$(awk -v a="${minw}" -v b="${maxw}" -v m="${medw}" 'BEGIN{if(m==0)print 0;else printf "%.2f\n",(b-a)/m*100}')"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "${pg}" "${n}" "${medw}" "${spread}" \
      "$(median_for "${pg}" 5)" "$(median_for "${pg}" 6)" "$(median_for "${pg}" 7)" "$(median_for "${pg}" 8)" \
      "$(median_for "${pg}" 9)" "$(median_for "${pg}" 10)" "$(median_for "${pg}" 11)" >>"${MEDIANS}"
done

mkdir -p "${FINAL_DIR}"
cp -a "${RAM_ROOT}/." "${FINAL_DIR}/"

echo
echo '=== TMPFS RUNS ==='
cat "${RUNS}"
echo
echo '=== TMPFS MEDIANS ==='
cat "${MEDIANS}"

w8="$(awk -F '\t' '$1==8{print $3}' "${MEDIANS}")"; w32="$(awk -F '\t' '$1==32{print $3}' "${MEDIANS}")"
pr8="$(awk -F '\t' '$1==8{print $5}' "${MEDIANS}")"; pr32="$(awk -F '\t' '$1==32{print $5}' "${MEDIANS}")"
rr8="$(awk -F '\t' '$1==8{print $6}' "${MEDIANS}")"; rr32="$(awk -F '\t' '$1==32{print $6}' "${MEDIANS}")"
cp8="$(awk -F '\t' '$1==8{print $7}' "${MEDIANS}")"; cp32="$(awk -F '\t' '$1==32{print $7}' "${MEDIANS}")"
in8="$(awk -F '\t' '$1==8{print $8}' "${MEDIANS}")"; in32="$(awk -F '\t' '$1==32{print $8}' "${MEDIANS}")"
iw8="$(awk -F '\t' '$1==8{print $9}' "${MEDIANS}")"; iw32="$(awk -F '\t' '$1==32{print $9}' "${MEDIANS}")"
wal8="$(awk -F '\t' '$1==8{print $10}' "${MEDIANS}")"; wal32="$(awk -F '\t' '$1==32{print $10}' "${MEDIANS}")"

echo "tmpfs_pg32_vs_8_primary_write_pct=$(pct_delta "${w8}" "${w32}")"
echo "tmpfs_pg32_vs_8_primary_read_pct=$(pct_delta "${pr8}" "${pr32}")"
echo "tmpfs_pg32_vs_8_replica_read_pct=$(pct_delta "${rr8}" "${rr32}")"
echo "tmpfs_pg32_vs_8_copy_mean_pct=$(pct_delta "${cp8}" "${cp32}")"
echo "tmpfs_pg32_vs_8_insert_mean_pct=$(pct_delta "${in8}" "${in32}")"
echo "tmpfs_pg32_vs_8_insert_wal_bytes_pct=$(pct_delta "${iw8}" "${iw32}")"
echo "tmpfs_pg32_vs_8_wal_bytes_pct=$(pct_delta "${wal8}" "${wal32}")"
echo "tmpfs_isolation_artifact_dir=${FINAL_DIR}"
echo 'OK: PostgreSQL block-size tmpfs isolation benchmark'
