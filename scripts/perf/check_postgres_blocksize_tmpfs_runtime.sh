#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

COMPOSE="${ROOT}/docker-compose.postgres-blocksize-tmpfs.yml"
PROJECT="fod-pg-tmpfs-preflight-${BASHPID}"
WAIT_SECONDS="${FOD_PG_BLOCK_TMPFS_PREFLIGHT_WAIT_SECONDS:-60}"
RAM_ROOT="/dev/shm/${PROJECT}"
PRIMARY_DIR="${RAM_ROOT}/primary"
REPLICA_DIR="${RAM_ROOT}/replica"

for cmd in docker sleep mkdir rm; do
    command -v "${cmd}" >/dev/null 2>&1 || { echo "Missing required command: ${cmd}" >&2; exit 2; }
done
[[ -r "${COMPOSE}" ]] || { echo "Missing tmpfs compose: ${COMPOSE}" >&2; exit 2; }
[[ -d /dev/shm && -w /dev/shm ]] || { echo "/dev/shm must be writable" >&2; exit 2; }

mkdir -p "${PRIMARY_DIR}" "${REPLICA_DIR}"

cleanup() {
    local rc=$?
    set +e
    compose down -v --remove-orphans >/dev/null 2>&1 || true
    rm -rf "${RAM_ROOT}"
    exit "${rc}"
}
trap cleanup EXIT INT TERM

compose() {
    COMPOSE_PROJECT_NAME="${PROJECT}" \
    POSTGRES_BLOCK_SIZE_KB=8 \
    FOD_EXPECTED_PG_BLOCK_SIZE_BYTES=8192 \
    FOD_PG_BLOCK_TMPFS_PRIMARY_DIR="${PRIMARY_DIR}" \
    FOD_PG_BLOCK_TMPFS_REPLICA_DIR="${REPLICA_DIR}" \
    docker compose -f "${COMPOSE}" "$@"
}

wait_primary_healthy() {
    local container="$1"
    local state health
    for ((second=0; second<WAIT_SECONDS; second++)); do
        state="$(docker inspect -f '{{.State.Status}}' "${container}" 2>/dev/null || echo missing)"
        health="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "${container}" 2>/dev/null || echo missing)"
        if [[ "${state}" == "exited" || "${state}" == "dead" ]]; then
            echo "Primary exited during tmpfs preflight state=${state}" >&2
            compose logs --no-color primary >&2 || true
            return 1
        fi
        if [[ "${state}" == "running" && "${health}" == "healthy" ]]; then
            return 0
        fi
        sleep 1
    done
    echo "Primary did not become healthy within ${WAIT_SECONDS}s" >&2
    compose logs --no-color primary >&2 || true
    return 1
}

echo '=== POSTGRESQL TMPFS RUNTIME PREFLIGHT ==='
compose down -v --remove-orphans >/dev/null 2>&1 || true
compose up -d primary

container="$(compose ps -q primary)"
[[ -n "${container}" ]] || { echo 'Primary container id is empty' >&2; exit 1; }
wait_primary_healthy "${container}"

docker exec "${container}" /bin/sh -ceu '
    echo "runtime_uid=$(id -u) runtime_gid=$(id -g)"
    stat -c "data_owner=%u:%g data_mode=%a" /var/lib/postgresql/data
    su-exec postgres sh -ceu '\''touch /var/lib/postgresql/data/.fod-tmpfs-writecheck; rm -f /var/lib/postgresql/data/.fod-tmpfs-writecheck'\''
    test "$(psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atqc "SHOW block_size")" = "8192"
    touch /var/lib/postgresql/data/.fod-tmpfs-restart-persistence
'

compose restart primary >/dev/null
container="$(compose ps -q primary)"
[[ -n "${container}" ]] || { echo 'Primary container id is empty after restart' >&2; exit 1; }
wait_primary_healthy "${container}"

docker exec "${container}" /bin/sh -ceu '
    test -f /var/lib/postgresql/data/.fod-tmpfs-restart-persistence
    test -s "$PGDATA/PG_VERSION"
    test "$(psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atqc "SHOW block_size")" = "8192"
    rm -f /var/lib/postgresql/data/.fod-tmpfs-restart-persistence
'

echo "primary_bind_ram_dir=${PRIMARY_DIR}"
echo 'restart_persistence=ok'
echo 'OK: PostgreSQL tmpfs runtime preflight'
