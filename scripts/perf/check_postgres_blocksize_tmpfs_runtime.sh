#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

COMPOSE="${ROOT}/docker-compose.postgres-blocksize-tmpfs.yml"
PROJECT="fod-pg-tmpfs-preflight-${BASHPID}"
WAIT_SECONDS="${FOD_PG_BLOCK_TMPFS_PREFLIGHT_WAIT_SECONDS:-60}"

for cmd in docker sleep; do
    command -v "${cmd}" >/dev/null 2>&1 || { echo "Missing required command: ${cmd}" >&2; exit 2; }
done
[[ -r "${COMPOSE}" ]] || { echo "Missing tmpfs compose: ${COMPOSE}" >&2; exit 2; }

cleanup() {
    local rc=$?
    set +e
    COMPOSE_PROJECT_NAME="${PROJECT}" docker compose -f "${COMPOSE}" down -v --remove-orphans >/dev/null 2>&1 || true
    exit "${rc}"
}
trap cleanup EXIT INT TERM

compose() {
    COMPOSE_PROJECT_NAME="${PROJECT}" \
    POSTGRES_BLOCK_SIZE_KB=8 \
    FOD_EXPECTED_PG_BLOCK_SIZE_BYTES=8192 \
    docker compose -f "${COMPOSE}" "$@"
}

echo '=== POSTGRESQL TMPFS RUNTIME PREFLIGHT ==='
compose down -v --remove-orphans >/dev/null 2>&1 || true
compose up -d primary

container="$(compose ps -q primary)"
[[ -n "${container}" ]] || { echo 'Primary container id is empty' >&2; exit 1; }

for ((second=0; second<WAIT_SECONDS; second++)); do
    state="$(docker inspect -f '{{.State.Status}}' "${container}" 2>/dev/null || echo missing)"
    health="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "${container}" 2>/dev/null || echo missing)"
    if [[ "${state}" == "exited" || "${state}" == "dead" ]]; then
        echo "Primary exited during tmpfs preflight state=${state}" >&2
        compose logs --no-color primary >&2 || true
        exit 1
    fi
    if [[ "${health}" == "healthy" ]]; then
        break
    fi
    sleep 1
done

state="$(docker inspect -f '{{.State.Status}}' "${container}")"
health="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "${container}")"
if [[ "${state}" != "running" || "${health}" != "healthy" ]]; then
    echo "Primary did not become healthy state=${state} health=${health}" >&2
    compose logs --no-color primary >&2 || true
    exit 1
fi

docker exec "${container}" /bin/sh -ceu '
    echo "runtime_uid=$(id -u) runtime_gid=$(id -g)"
    stat -c "data_owner=%u:%g data_mode=%a" /var/lib/postgresql/data
    test "$(stat -c %u /var/lib/postgresql/data)" = "70"
    test "$(stat -c %g /var/lib/postgresql/data)" = "70"
    su-exec postgres sh -ceu '\''touch /var/lib/postgresql/data/.fod-tmpfs-writecheck; rm -f /var/lib/postgresql/data/.fod-tmpfs-writecheck'\''
    test "$(psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atqc "SHOW block_size")" = "8192"
'

echo 'OK: PostgreSQL tmpfs runtime preflight'
