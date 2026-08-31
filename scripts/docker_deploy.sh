#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

ACTION="${1:-plan}"
MASTERS="${MASTERS:-${FOD_DOCKER_DEPLOY_MASTERS:-1}}"
SLAVES="${SLAVES:-${FOD_DOCKER_DEPLOY_SLAVES:-1}}"
STATE_DIR="${FOD_DOCKER_DEPLOY_STATE_DIR:-${XDG_STATE_HOME:-${HOME}/.local/state}/fod/docker-deploy}"
PROJECT="${FOD_DOCKER_DEPLOY_PROJECT:-fod-deploy}"
NETWORK_NAME="${FOD_DOCKER_DEPLOY_NETWORK:-${PROJECT}-network}"
BIND_ADDRESS="${FOD_DOCKER_DEPLOY_BIND_ADDRESS:-127.0.0.1}"
CLIENT_HOST="${FOD_DOCKER_DEPLOY_CLIENT_HOST:-127.0.0.1}"
PRIMARY_PORT="${FOD_DOCKER_DEPLOY_PRIMARY_PORT:-55441}"
REPLICA_PORT_BASE="${FOD_DOCKER_DEPLOY_REPLICA_PORT_BASE:-55442}"
POSTGRES_IMAGE="${FOD_DOCKER_DEPLOY_POSTGRES_IMAGE:-ghcr.io/stachwk/postgres-16-fod-32k:16.15}"
CLIENT_IMAGE="${FOD_DOCKER_DEPLOY_CLIENT_IMAGE:-ghcr.io/stachwk/fod-client:3.4}"
REPLICA_READ_ROUTING="${FOD_DOCKER_DEPLOY_REPLICA_READ_ROUTING:-0}"
EXPECTED_BLOCK_SIZE=32768

COMPOSE_FILE="${STATE_DIR}/compose.yml"
POSTGRES_ENV="${STATE_DIR}/postgres.env"
ADMIN_ENV="${STATE_DIR}/fod-admin.env"
HOST_CONFIG="${STATE_DIR}/fod-host.ini"
CONTAINER_CONFIG="${STATE_DIR}/fod-container.ini"
SCRIPTS_DIR="${STATE_DIR}/scripts"

fail() {
  echo "ERROR: $*" >&2
  exit 2
}

require_uint() {
  local name="$1" value="$2" min="$3" max="$4"
  [[ "${value}" =~ ^[0-9]+$ ]] || fail "${name} must be an integer"
  (( value >= min && value <= max )) || fail "${name} must be in range ${min}..${max}"
}

validate() {
  require_uint MASTERS "${MASTERS}" 1 1
  require_uint SLAVES "${SLAVES}" 0 32
  require_uint FOD_DOCKER_DEPLOY_PRIMARY_PORT "${PRIMARY_PORT}" 1 65535
  require_uint FOD_DOCKER_DEPLOY_REPLICA_PORT_BASE "${REPLICA_PORT_BASE}" 1 65535
  local last_port=$((REPLICA_PORT_BASE + SLAVES - 1))
  (( SLAVES == 0 || last_port <= 65535 )) || fail "replica port range exceeds 65535"
  [[ "${REPLICA_READ_ROUTING}" == 0 || "${REPLICA_READ_ROUTING}" == 1 ]] || \
    fail "FOD_DOCKER_DEPLOY_REPLICA_READ_ROUTING must be 0 or 1"
  [[ "${PROJECT}" =~ ^[A-Za-z0-9][A-Za-z0-9_.-]*$ ]] || fail "invalid deployment project name"
}

random_secret() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 24
  else
    od -An -N24 -tx1 /dev/urandom | tr -d ' \n'
  fi
}

load_or_create_secrets() {
  mkdir -p "${STATE_DIR}"
  chmod 700 "${STATE_DIR}"
  umask 077

  local requested_pg_password="${POSTGRES_PASSWORD:-}"
  local requested_repl_password="${FOD_REPLICATION_PASSWORD:-}"
  local requested_admin_password="${FOD_SCHEMA_ADMIN_PASSWORD:-}"
  local requested_db="${POSTGRES_DB:-}"
  local requested_user="${POSTGRES_USER:-}"

  if [[ -f "${POSTGRES_ENV}" ]]; then
    # shellcheck disable=SC1090
    source "${POSTGRES_ENV}"
    [[ -z "${requested_pg_password}" || "${requested_pg_password}" == "${POSTGRES_PASSWORD}" ]] || \
      fail "existing deployment uses a different POSTGRES_PASSWORD; secret rotation is not automatic"
    [[ -z "${requested_repl_password}" || "${requested_repl_password}" == "${FOD_REPLICATION_PASSWORD}" ]] || \
      fail "existing deployment uses a different FOD_REPLICATION_PASSWORD"
    [[ -z "${requested_db}" || "${requested_db}" == "${POSTGRES_DB}" ]] || fail "existing deployment uses a different POSTGRES_DB"
    [[ -z "${requested_user}" || "${requested_user}" == "${POSTGRES_USER}" ]] || fail "existing deployment uses a different POSTGRES_USER"
  else
    POSTGRES_DB="${requested_db:-foddbname}"
    POSTGRES_USER="${requested_user:-foduser}"
    POSTGRES_PASSWORD="${requested_pg_password:-$(random_secret)}"
    FOD_REPLICATION_USER="${FOD_REPLICATION_USER:-fod_repl}"
    FOD_REPLICATION_PASSWORD="${requested_repl_password:-$(random_secret)}"
    case "${POSTGRES_USER}:${FOD_REPLICATION_USER}" in
      *[!A-Za-z0-9_:]*) fail "PostgreSQL user names must contain only letters, digits and underscore" ;;
    esac
    cat > "${POSTGRES_ENV}" <<EOF
POSTGRES_DB=${POSTGRES_DB}
POSTGRES_USER=${POSTGRES_USER}
POSTGRES_PASSWORD=${POSTGRES_PASSWORD}
FOD_REPLICATION_USER=${FOD_REPLICATION_USER}
FOD_REPLICATION_PASSWORD=${FOD_REPLICATION_PASSWORD}
EOF
  fi

  if [[ -f "${ADMIN_ENV}" ]]; then
    # shellcheck disable=SC1090
    source "${ADMIN_ENV}"
    [[ -z "${requested_admin_password}" || "${requested_admin_password}" == "${FOD_SCHEMA_ADMIN_PASSWORD}" ]] || \
      fail "existing deployment uses a different FOD_SCHEMA_ADMIN_PASSWORD"
  else
    FOD_SCHEMA_ADMIN_PASSWORD="${requested_admin_password:-$(random_secret)}"
    printf 'FOD_SCHEMA_ADMIN_PASSWORD=%s\n' "${FOD_SCHEMA_ADMIN_PASSWORD}" > "${ADMIN_ENV}"
  fi
  chmod 600 "${POSTGRES_ENV}" "${ADMIN_ENV}"
}

replica_host_list() {
  local mode="$1" result="" i host port
  for ((i=1; i<=SLAVES; i++)); do
    if [[ "${mode}" == host ]]; then
      host="${CLIENT_HOST}"
      port=$((REPLICA_PORT_BASE + i - 1))
    else
      host="replica${i}"
      port=5432
    fi
    [[ -z "${result}" ]] || result+="," 
    result+="${host}:${port}"
  done
  printf '%s' "${result}"
}

generate_fod_config() {
  local path="$1" mode="$2" primary_host primary_port replicas replica_routing
  if [[ "${mode}" == host ]]; then
    primary_host="${CLIENT_HOST}"
    primary_port="${PRIMARY_PORT}"
  else
    primary_host="primary"
    primary_port=5432
  fi
  replicas="$(replica_host_list "${mode}")"
  replica_routing=false
  [[ "${REPLICA_READ_ROUTING}" == 1 && "${SLAVES}" -gt 0 ]] && replica_routing=true

  cat > "${path}" <<EOF
[database]
host = ${primary_host}
port = ${primary_port}
primary_hosts = ${primary_host}:${primary_port}
EOF
  if [[ -n "${replicas}" ]]; then
    printf 'replica_hosts = %s\n' "${replicas}" >> "${path}"
  fi
  cat >> "${path}" <<EOF
dbname = ${POSTGRES_DB}
user = ${POSTGRES_USER}
password = ${POSTGRES_PASSWORD}

[fod]
role = auto
force_read_only = false
pg_endpoint_routing_enabled = true
pg_runtime_failover_enabled = true
pg_replica_read_routing_enabled = ${replica_routing}
pool_max_connections = 10
synchronous_commit = on
EOF
  chmod 600 "${path}"
}

generate_compose() {
  mkdir -p "${SCRIPTS_DIR}"
  cp "${ROOT}/docker/deploy/primary-init.sh" "${SCRIPTS_DIR}/primary-init.sh"
  cp "${ROOT}/docker/deploy/replica-entrypoint.sh" "${SCRIPTS_DIR}/replica-entrypoint.sh"
  chmod 755 "${SCRIPTS_DIR}/primary-init.sh" "${SCRIPTS_DIR}/replica-entrypoint.sh"

  local wal_senders=$((SLAVES + 4))
  local slots=$((SLAVES + 4))
  cat > "${COMPOSE_FILE}" <<EOF
services:
  primary:
    image: ${POSTGRES_IMAGE}
    restart: unless-stopped
    env_file:
      - ./postgres.env
    environment:
      PGDATA: /var/lib/postgresql/data/pgdata
      POSTGRES_INITDB_ARGS: "--auth-local=trust --auth-host=scram-sha-256"
      FOD_REPLICA_COUNT: "${SLAVES}"
    command:
      - postgres
      - -c
      - wal_level=replica
      - -c
      - max_wal_senders=${wal_senders}
      - -c
      - max_replication_slots=${slots}
      - -c
      - max_slot_wal_keep_size=4GB
      - -c
      - hot_standby=on
      - -c
      - synchronous_commit=on
      - -c
      - track_io_timing=on
      - -c
      - track_wal_io_timing=on
      - -c
      - shared_preload_libraries=pg_stat_statements
    ports:
      - "${BIND_ADDRESS}:${PRIMARY_PORT}:5432"
    volumes:
      - primary_data:/var/lib/postgresql/data
      - ./scripts/primary-init.sh:/docker-entrypoint-initdb.d/20-fod-replication.sh:ro
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U \"\$\$POSTGRES_USER\" -d \"\$\$POSTGRES_DB\" && test \"\$\$(psql -U \"\$\$POSTGRES_USER\" -d \"\$\$POSTGRES_DB\" -Atqc 'SHOW block_size')\" = \"${EXPECTED_BLOCK_SIZE}\""]
      interval: 2s
      timeout: 5s
      retries: 60
      start_period: 5s
EOF

  local i port
  for ((i=1; i<=SLAVES; i++)); do
    port=$((REPLICA_PORT_BASE + i - 1))
    cat >> "${COMPOSE_FILE}" <<EOF

  replica${i}:
    image: ${POSTGRES_IMAGE}
    restart: unless-stopped
    depends_on:
      primary:
        condition: service_healthy
    env_file:
      - ./postgres.env
    environment:
      PGDATA: /var/lib/postgresql/data/pgdata
      PRIMARY_HOST: primary
      PRIMARY_PORT: "5432"
      FOD_REPLICATION_SLOT: fod_replica_${i}
      FOD_REPLICATION_APPLICATION_NAME: fod_replica_${i}
    entrypoint: ["/bin/sh", "/usr/local/bin/fod-replica-entrypoint.sh"]
    ports:
      - "${BIND_ADDRESS}:${port}:5432"
    volumes:
      - replica${i}_data:/var/lib/postgresql/data
      - ./scripts/replica-entrypoint.sh:/usr/local/bin/fod-replica-entrypoint.sh:ro
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U \"\$\$POSTGRES_USER\" -d \"\$\$POSTGRES_DB\" && test \"\$\$(psql -U \"\$\$POSTGRES_USER\" -d \"\$\$POSTGRES_DB\" -Atqc 'SELECT pg_is_in_recovery()')\" = t && test \"\$\$(psql -U \"\$\$POSTGRES_USER\" -d \"\$\$POSTGRES_DB\" -Atqc 'SHOW block_size')\" = \"${EXPECTED_BLOCK_SIZE}\""]
      interval: 2s
      timeout: 5s
      retries: 90
      start_period: 5s
EOF
  done

  cat >> "${COMPOSE_FILE}" <<EOF

volumes:
  primary_data:
EOF
  for ((i=1; i<=SLAVES; i++)); do
    printf '  replica%s_data:\n' "${i}" >> "${COMPOSE_FILE}"
  done
  cat >> "${COMPOSE_FILE}" <<EOF

networks:
  default:
    name: ${NETWORK_NAME}
EOF
}

render() {
  load_or_create_secrets
  generate_fod_config "${HOST_CONFIG}" host
  generate_fod_config "${CONTAINER_CONFIG}" container
  generate_compose
  if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" config --quiet
  fi
  printf 'rendered_state_dir=%s\ncompose=%s\nhost_config=%s\ncontainer_config=%s\n' \
    "${STATE_DIR}" "${COMPOSE_FILE}" "${HOST_CONFIG}" "${CONTAINER_CONFIG}"
}

compose() {
  docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" "$@"
}

require_docker() {
  command -v docker >/dev/null 2>&1 || fail "docker is required"
  docker compose version >/dev/null 2>&1 || fail "Docker Compose v2 is required"
}

wait_for_cluster() {
  # shellcheck disable=SC1090
  source "${POSTGRES_ENV}"
  local attempt i recovery block
  for attempt in $(seq 1 90); do
    if compose exec -T primary pg_isready -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" >/dev/null 2>&1; then
      block="$(compose exec -T primary psql -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Atqc 'SHOW block_size' 2>/dev/null | tr -d '\r')"
      [[ "${block}" == "${EXPECTED_BLOCK_SIZE}" ]] && break
    fi
    sleep 1
  done
  [[ "${block:-}" == "${EXPECTED_BLOCK_SIZE}" ]] || fail "primary did not become healthy with block_size=${EXPECTED_BLOCK_SIZE}"

  for ((i=1; i<=SLAVES; i++)); do
    recovery=""
    for attempt in $(seq 1 120); do
      recovery="$(compose exec -T "replica${i}" psql -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Atqc 'SELECT pg_is_in_recovery()' 2>/dev/null | tr -d '\r' || true)"
      [[ "${recovery}" == t ]] && break
      sleep 1
    done
    [[ "${recovery}" == t ]] || fail "replica${i} did not enter recovery"
  done
}

smoke() {
  # shellcheck disable=SC1090
  source "${POSTGRES_ENV}"
  local block recovery streaming i
  block="$(compose exec -T primary psql -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Atqc 'SHOW block_size' | tr -d '\r')"
  [[ "${block}" == "${EXPECTED_BLOCK_SIZE}" ]] || fail "unexpected primary block_size=${block}"
  recovery="$(compose exec -T primary psql -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Atqc 'SELECT pg_is_in_recovery()' | tr -d '\r')"
  [[ "${recovery}" == f ]] || fail "primary is unexpectedly in recovery"
  for ((i=1; i<=SLAVES; i++)); do
    recovery="$(compose exec -T "replica${i}" psql -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Atqc 'SELECT pg_is_in_recovery()' | tr -d '\r')"
    [[ "${recovery}" == t ]] || fail "replica${i} is not in recovery"
  done
  streaming="$(compose exec -T primary psql -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Atqc "SELECT count(*) FROM pg_stat_replication WHERE state = 'streaming'" | tr -d '\r')"
  (( streaming >= SLAVES )) || fail "expected at least ${SLAVES} streaming replicas, observed ${streaming}"
  printf 'OK: docker deployment primary=1 replicas=%s block_size=%s streaming=%s\n' "${SLAVES}" "${block}" "${streaming}"
}

fod_schema_exists() {
  # shellcheck disable=SC1090
  source "${POSTGRES_ENV}"
  [[ "$(compose exec -T primary psql -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Atqc "SELECT to_regclass('fod.config') IS NOT NULL" | tr -d '\r')" == t ]]
}

run_mkfs() {
  local operation="$1"
  # shellcheck disable=SC1090
  source "${ADMIN_ENV}"
  if [[ "${operation}" == init ]] && fod_schema_exists; then
    echo "FOD schema already exists; init skipped."
    return 0
  fi
  docker run --rm \
    --network "${NETWORK_NAME}" \
    -e FOD_CONFIG=/work/fod.ini \
    -v "${CONTAINER_CONFIG}:/work/fod.ini:ro" \
    "${CLIENT_IMAGE}" \
    mkfs.fod "${operation}" \
      --schema-admin-password "${FOD_SCHEMA_ADMIN_PASSWORD}" \
      --block-size "${EXPECTED_BLOCK_SIZE}"
}

plan() {
  printf '%s\n' \
    '=== FOD DOCKER DEPLOYMENT PLAN ===' \
    "masters=${MASTERS}" \
    "slaves=${SLAVES}" \
    "postgres_image=${POSTGRES_IMAGE}" \
    "client_image=${CLIENT_IMAGE}" \
    "primary=${BIND_ADDRESS}:${PRIMARY_PORT}" \
    "state_dir=${STATE_DIR}" \
    "network=${NETWORK_NAME}"
  local i
  for ((i=1; i<=SLAVES; i++)); do
    printf 'replica%s=%s:%s slot=fod_replica_%s\n' "${i}" "${BIND_ADDRESS}" "$((REPLICA_PORT_BASE + i - 1))" "${i}"
  done
  echo 'PostgreSQL multi-master is intentionally unsupported; MASTERS must equal 1.'
}

validate

case "${ACTION}" in
  plan)
    plan
    ;;
  render)
    render
    ;;
  pull)
    require_docker
    render
    compose pull
    docker pull "${CLIENT_IMAGE}"
    ;;
  up)
    require_docker
    render
    compose up -d
    wait_for_cluster
    ;;
  install)
    require_docker
    render
    compose pull
    docker pull "${CLIENT_IMAGE}"
    compose up -d
    wait_for_cluster
    run_mkfs init
    smoke
    ;;
  status)
    require_docker
    render
    compose ps
    ;;
  smoke)
    require_docker
    render
    wait_for_cluster
    smoke
    ;;
  logs)
    require_docker
    render
    compose logs -f
    ;;
  down)
    require_docker
    [[ -f "${COMPOSE_FILE}" ]] || render
    compose down --remove-orphans
    ;;
  destroy)
    require_docker
    [[ "${DESTROY:-}" == YES ]] || fail "destructive removal requires DESTROY=YES"
    [[ -f "${COMPOSE_FILE}" ]] || render
    compose down -v --remove-orphans
    if [[ "${FOD_DOCKER_DEPLOY_PURGE_STATE:-0}" == 1 ]]; then
      rm -rf "${STATE_DIR}"
    fi
    ;;
  fod-init)
    require_docker
    render
    wait_for_cluster
    run_mkfs init
    ;;
  fod-upgrade)
    require_docker
    render
    wait_for_cluster
    run_mkfs upgrade
    ;;
  *)
    fail "unknown action: ${ACTION}"
    ;;
esac
