#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

COMPOSE="docker-compose.postgres-blocksize-tmpfs.yml"
RUNNER="scripts/perf/run_postgres_blocksize_tmpfs_isolation.sh"

[[ -r "${COMPOSE}" ]] || { echo "Missing ${COMPOSE}" >&2; exit 1; }
[[ -r "${RUNNER}" ]] || { echo "Missing ${RUNNER}" >&2; exit 1; }

for pattern in \
    'type: tmpfs' \
    'postgres_blocksize_primary_tmpfs' \
    'postgres_blocksize_replica_tmpfs' \
    'synchronous_commit=on' \
    'wal_level=replica' \
    'shared_preload_libraries=pg_stat_statements'; do
    grep -Fq -- "${pattern}" "${COMPOSE}" || { echo "Missing tmpfs compose policy: ${pattern}" >&2; exit 1; }
done

for pattern in \
    'warning=tmpfs_lab_not_durability_benchmark' \
    'FOD_PG_BLOCK_TMPFS_FILE_SIZE:-512M' \
    '/dev/shm/fod-postgres-blocksize-' \
    'REPLICA_READ_COMPOSE_FILE="docker-compose.postgres-blocksize-tmpfs.yml"' \
    'FOD_STORAGE_BLOCK_PAYLOAD_MODE=random' \
    'FOD_STORAGE_BLOCK_SKIP_BUILD=1' \
    'cp -a "${RAM_ROOT}/." "${FINAL_DIR}/"' \
    'tmpfs_pg32_vs_8_primary_write_pct=' \
    'tmpfs_pg32_vs_8_insert_wal_bytes_pct='; do
    grep -Fq -- "${pattern}" "${RUNNER}" || { echo "Missing tmpfs runner policy: ${pattern}" >&2; exit 1; }
done

# The isolation lab must not weaken PostgreSQL durability settings merely to
# manufacture a throughput result. tmpfs itself is the explicit isolation.
if grep -Eq -- 'synchronous_commit=(off|local)|fsync=off|full_page_writes=off' "${COMPOSE}" "${RUNNER}"; then
    echo 'Tmpfs isolation must not disable PostgreSQL durability settings' >&2
    exit 1
fi

# It must not mutate or globally clean the user's Docker/storage state.
if grep -Eq -- 'docker[[:space:]]+(system|volume)[[:space:]]+prune|fstrim|blkdiscard|rm[[:space:]]+-rf[[:space:]]+/docker' "${RUNNER}"; then
    echo 'Tmpfs isolation runner contains destructive/global storage cleanup' >&2
    exit 1
fi

echo 'OK: PostgreSQL block-size tmpfs isolation policy'
