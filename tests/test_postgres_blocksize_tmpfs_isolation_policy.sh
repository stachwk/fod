#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

COMPOSE="docker-compose.postgres-blocksize-tmpfs.yml"
RUNNER="scripts/perf/run_postgres_blocksize_tmpfs_isolation.sh"
PREFLIGHT="scripts/perf/check_postgres_blocksize_tmpfs_runtime.sh"

[[ -r "${COMPOSE}" ]] || { echo "Missing ${COMPOSE}" >&2; exit 1; }
[[ -r "${RUNNER}" ]] || { echo "Missing ${RUNNER}" >&2; exit 1; }
[[ -r "${PREFLIGHT}" ]] || { echo "Missing ${PREFLIGHT}" >&2; exit 1; }

bash -n "${RUNNER}"
bash -n "${PREFLIGHT}"

for pattern in \
    'type: bind' \
    'source: ${FOD_PG_BLOCK_TMPFS_PRIMARY_DIR:' \
    'source: ${FOD_PG_BLOCK_TMPFS_REPLICA_DIR:' \
    'synchronous_commit=on' \
    'wal_level=replica' \
    'shared_preload_libraries=pg_stat_statements'; do
    grep -Fq -- "${pattern}" "${COMPOSE}" || { echo "Missing tmpfs compose policy: ${pattern}" >&2; exit 1; }
done

if grep -Eq -- 'type:[[:space:]]*tmpfs|driver_opts:|postgres_blocksize_(primary|replica)_tmpfs:' "${COMPOSE}"; then
    echo 'Tmpfs isolation must use host /dev/shm bind directories, not restart-volatile Docker tmpfs volumes' >&2
    exit 1
fi

for pattern in \
    'warning=tmpfs_lab_not_durability_benchmark' \
    'FOD_PG_BLOCK_TMPFS_FILE_SIZE:-512M' \
    '/dev/shm/fod-postgres-blocksize-' \
    'FOD_PG_BLOCK_TMPFS_PRIMARY_DIR="${primary_dir}"' \
    'FOD_PG_BLOCK_TMPFS_REPLICA_DIR="${replica_dir}"' \
    'REPLICA_READ_COMPOSE_FILE="docker-compose.postgres-blocksize-tmpfs.yml"' \
    'FOD_STORAGE_BLOCK_PAYLOAD_MODE=random' \
    'FOD_STORAGE_BLOCK_SKIP_BUILD=1' \
    'bash "${PREFLIGHT}"' \
    'cleanup_ram_tree "${primary_dir}"' \
    'cleanup_ram_tree "${replica_dir}"' \
    'docker run --rm --entrypoint /bin/sh' \
    'cp -a "${RAM_ROOT}/." "${FINAL_DIR}/"' \
    'tmpfs_pg32_vs_8_primary_write_pct=' \
    'tmpfs_pg32_vs_8_insert_wal_bytes_pct='; do
    grep -Fq -- "${pattern}" "${RUNNER}" || { echo "Missing tmpfs runner policy: ${pattern}" >&2; exit 1; }
done

for pattern in \
    'RAM_ROOT="/dev/shm/${PROJECT}"' \
    'FOD_PG_BLOCK_TMPFS_PRIMARY_DIR="${PRIMARY_DIR}"' \
    'FOD_PG_BLOCK_TMPFS_REPLICA_DIR="${REPLICA_DIR}"' \
    'POSTGRES_BLOCK_SIZE_KB=8' \
    'FOD_EXPECTED_PG_BLOCK_SIZE_BYTES=8192' \
    'postgres_uid=$(id -u postgres)' \
    'pgdata_owner=%u:%g' \
    'test "$(stat -c %u "$PGDATA")" = "$(id -u postgres)"' \
    'test "$(stat -c %a "$PGDATA")" = "700"' \
    '$PGDATA/.fod-tmpfs-writecheck' \
    '$PGDATA/.fod-tmpfs-restart-persistence' \
    'compose restart primary' \
    'restart_persistence=ok' \
    'cleanup_ram_root' \
    'SHOW block_size' \
    'compose logs --no-color primary'; do
    grep -Fq -- "${pattern}" "${PREFLIGHT}" || { echo "Missing tmpfs preflight policy: ${pattern}" >&2; exit 1; }
done

if grep -Fq -- 'test "$(stat -c %g "$PGDATA")" = "70"' "${PREFLIGHT}"; then
    echo 'Tmpfs preflight must not require a fixed PostgreSQL group id' >&2
    exit 1
fi

# The isolation lab must not weaken PostgreSQL durability settings merely to
# manufacture a throughput result. RAM storage itself is the explicit isolation.
if grep -Eq -- 'synchronous_commit=(off|local)|fsync=off|full_page_writes=off' "${COMPOSE}" "${RUNNER}" "${PREFLIGHT}"; then
    echo 'Tmpfs isolation must not disable PostgreSQL durability settings' >&2
    exit 1
fi

# It must not mutate or globally clean the user's Docker/storage state.
if grep -Eq -- 'docker[[:space:]]+(system|volume)[[:space:]]+prune|fstrim|blkdiscard|rm[[:space:]]+-rf[[:space:]]+/docker' "${RUNNER}" "${PREFLIGHT}"; then
    echo 'Tmpfs isolation contains destructive/global storage cleanup' >&2
    exit 1
fi

echo 'OK: PostgreSQL block-size tmpfs isolation policy'
