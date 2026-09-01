#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

ACTION="${1:-start}"
MASTERS="${MASTERS:-${FOD_DOCKER_DEPLOY_MASTERS:-1}}"
SLAVES="${SLAVES:-${FOD_DOCKER_DEPLOY_SLAVES:-1}}"
STATE_DIR="${FOD_DOCKER_DEPLOY_STATE_DIR:-${XDG_STATE_HOME:-${HOME}/.local/state}/fod/docker-deploy}"
PROJECT="${FOD_DOCKER_DEPLOY_PROJECT:-fod-deploy}"
CLIENT_IMAGE="${FOD_DOCKER_DEPLOY_CLIENT_IMAGE:-ghcr.io/stachwk/fod-client:3.4}"
MOUNT_DIR="${FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR:-${XDG_DATA_HOME:-${HOME}/.local/share}/fod/mount}"
START_TIMEOUT="${FOD_DOCKER_DEPLOY_FOD_START_TIMEOUT_SECONDS:-30}"
HEALTH_TIMEOUT="${FOD_DOCKER_DEPLOY_FOD_HEALTH_TIMEOUT_SECONDS:-90}"
APPARMOR_MODE="${FOD_DOCKER_DEPLOY_FOD_APPARMOR:-auto}"
PULL_MODE="${FOD_DOCKER_DEPLOY_FOD_PULL_MODE:-always}"

BASE_COMPOSE="${STATE_DIR}/compose.yml"
FOD_COMPOSE="${STATE_DIR}/compose-fod.yml"
RUNTIME_COMPOSE="${STATE_DIR}/compose-fod-runtime.yml"
POSTGRES_ENV="${STATE_DIR}/postgres.env"
CONTAINER_NAME="${PROJECT}-fod"

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
  require_uint FOD_DOCKER_DEPLOY_FOD_START_TIMEOUT_SECONDS "${START_TIMEOUT}" 5 300
  require_uint FOD_DOCKER_DEPLOY_FOD_HEALTH_TIMEOUT_SECONDS "${HEALTH_TIMEOUT}" 10 600
  case "${APPARMOR_MODE}" in
    auto|unconfined|default) ;;
    *) fail "FOD_DOCKER_DEPLOY_FOD_APPARMOR must be auto, unconfined or default" ;;
  esac
  case "${PULL_MODE}" in
    always|if-missing|never) ;;
    *) fail "FOD_DOCKER_DEPLOY_FOD_PULL_MODE must be always, if-missing or never" ;;
  esac
}

base_action() {
  COMPOSE_IGNORE_ORPHANS=true \
    MASTERS="${MASTERS}" SLAVES="${SLAVES}" \
    FOD_DOCKER_DEPLOY_STATE_DIR="${STATE_DIR}" \
    FOD_DOCKER_DEPLOY_PROJECT="${PROJECT}" \
    FOD_DOCKER_DEPLOY_CLIENT_IMAGE="${CLIENT_IMAGE}" \
    bash "${ROOT}/scripts/docker_deploy.sh" "$@"
}

fod_action() {
  MASTERS="${MASTERS}" SLAVES="${SLAVES}" \
    FOD_DOCKER_DEPLOY_STATE_DIR="${STATE_DIR}" \
    FOD_DOCKER_DEPLOY_PROJECT="${PROJECT}" \
    FOD_DOCKER_DEPLOY_CLIENT_IMAGE="${CLIENT_IMAGE}" \
    FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR="${MOUNT_DIR}" \
    bash "${ROOT}/scripts/docker_fod_install.sh" "$@"
}

apparmor_enabled() {
  [[ -r /sys/module/apparmor/parameters/enabled ]] && grep -Eiq '^Y' /sys/module/apparmor/parameters/enabled
}

render_runtime_override() {
  rm -f "${RUNTIME_COMPOSE}"
  local use_unconfined=0
  case "${APPARMOR_MODE}" in
    unconfined) use_unconfined=1 ;;
    default) use_unconfined=0 ;;
    auto)
      if apparmor_enabled; then
        use_unconfined=1
      fi
      ;;
  esac

  if (( use_unconfined )); then
    cat > "${RUNTIME_COMPOSE}" <<'EOF'
services:
  fod:
    security_opt:
      - apparmor=unconfined
EOF
  else
    cat > "${RUNTIME_COMPOSE}" <<'EOF'
services:
  fod: {}
EOF
  fi
}

compose() {
  docker compose \
    -p "${PROJECT}" \
    -f "${BASE_COMPOSE}" \
    -f "${FOD_COMPOSE}" \
    -f "${RUNTIME_COMPOSE}" \
    "$@"
}

dump_diagnostics() {
  echo >&2
  echo '=== FOD CONTAINER START DIAGNOSTICS ===' >&2
  docker ps -a --filter "name=^/${CONTAINER_NAME}$" --no-trunc >&2 || true
  docker inspect "${CONTAINER_NAME}" --format \
    'status={{.State.Status}} running={{.State.Running}} restarting={{.State.Restarting}} exit={{.State.ExitCode}} error={{.State.Error}} health={{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' >&2 2>/dev/null || true
  docker logs --tail=120 "${CONTAINER_NAME}" >&2 2>/dev/null || true
  echo '--- /dev/fuse ---' >&2
  ls -l /dev/fuse >&2 2>/dev/null || true
  echo '--- host mount stack ---' >&2
  if command -v findmnt >/dev/null 2>&1; then
    findmnt -T "${MOUNT_DIR}" -o TARGET,SOURCE,FSTYPE,PROPAGATION,OPTIONS >&2 2>/dev/null || true
  fi
  if apparmor_enabled; then
    echo '--- AppArmor ---' >&2
    command -v aa-status >/dev/null 2>&1 && aa-status >&2 2>/dev/null | head -80 || true
  fi
  echo '=======================================' >&2
}

reconcile_stale_fod() {
  local state="" health=""
  state="$(docker inspect --format '{{.State.Status}}' "${CONTAINER_NAME}" 2>/dev/null || true)"
  health="$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "${CONTAINER_NAME}" 2>/dev/null || true)"

  if [[ -n "${state}" && ! ( "${state}" == running && "${health}" == healthy ) ]]; then
    echo "Removing stale FOD container: ${CONTAINER_NAME} status=${state} health=${health:-none}"
    docker rm -f "${CONTAINER_NAME}" >/dev/null 2>&1 || true
  fi

  FOD_DOCKER_DEPLOY_FOD_INTERACTIVE_SUDO=1 fod_action cleanup-stale-mounts
}

schema_exists() {
  [[ -f "${POSTGRES_ENV}" ]] || return 1
  # shellcheck disable=SC1090
  source "${POSTGRES_ENV}"
  docker compose -p "${PROJECT}" -f "${BASE_COMPOSE}" exec -T primary \
    psql -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Atqc \
    "SELECT to_regclass('fod.config') IS NOT NULL" 2>/dev/null | tr -d '\r' | grep -Fxq t
}

prepare() {
  reconcile_stale_fod
  fod_action render >/dev/null
  fod_action preflight
  schema_exists || fail "FOD schema is not initialized; run make docker-deploy-schema-init first"
  render_runtime_override
  compose config --quiet
}

pull_client_image() {
  case "${PULL_MODE}" in
    always)
      echo "Pulling FOD client image: ${CLIENT_IMAGE}"
      docker pull "${CLIENT_IMAGE}"
      ;;
    if-missing)
      if docker image inspect "${CLIENT_IMAGE}" >/dev/null 2>&1; then
        echo "Using cached exact FOD client image: ${CLIENT_IMAGE}"
      else
        echo "FOD client image is not cached; pulling: ${CLIENT_IMAGE}"
        docker pull "${CLIENT_IMAGE}"
      fi
      ;;
    never)
      docker image inspect "${CLIENT_IMAGE}" >/dev/null 2>&1 || \
        fail "FOD client image is not cached and pull mode is never: ${CLIENT_IMAGE}"
      echo "Using cached FOD client image without registry access: ${CLIENT_IMAGE}"
      ;;
  esac
}

start_container() {
  command -v timeout >/dev/null 2>&1 || fail "GNU timeout is required for guarded Docker FOD startup"
  pull_client_image
  prepare

  local current=""
  current="$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "${CONTAINER_NAME}" 2>/dev/null || true)"
  if [[ "${current}" == healthy ]]; then
    echo "FOD container is already healthy: ${CONTAINER_NAME}"
    return 0
  fi

  echo "Starting FOD container with timeout=${START_TIMEOUT}s apparmor=${APPARMOR_MODE}..."
  set +e
  timeout --foreground "${START_TIMEOUT}s" \
    docker compose \
      -p "${PROJECT}" \
      -f "${BASE_COMPOSE}" \
      -f "${FOD_COMPOSE}" \
      -f "${RUNTIME_COMPOSE}" \
      up -d --no-deps fod
  local rc=$?
  set -e
  if (( rc != 0 )); then
    dump_diagnostics
    if (( rc == 124 || rc == 137 )); then
      fail "Docker timed out while moving FOD from Created to Started after ${START_TIMEOUT}s"
    fi
    fail "Docker failed to start FOD container (rc=${rc})"
  fi

  local deadline=$((SECONDS + HEALTH_TIMEOUT)) status="" last=""
  while (( SECONDS < deadline )); do
    status="$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "${CONTAINER_NAME}" 2>/dev/null || true)"
    if [[ "${status}" != "${last}" ]]; then
      echo "FOD container status=${status:-missing}"
      last="${status}"
    fi
    [[ "${status}" == healthy ]] && return 0
    case "${status}" in
      exited|dead)
        dump_diagnostics
        fail "FOD container exited before becoming healthy"
        ;;
    esac
    sleep 1
  done

  dump_diagnostics
  fail "FOD container did not become healthy within ${HEALTH_TIMEOUT}s (status=${status:-missing})"
}

validate

case "${ACTION}" in
  start)
    start_container
    fod_action smoke
    ;;
  install)
    base_action up
    base_action fod-init
    start_container
    fod_action smoke
    ;;
  up)
    base_action up
    start_container
    fod_action smoke
    ;;
  diagnostics)
    fod_action render >/dev/null || true
    render_runtime_override
    dump_diagnostics
    ;;
  *)
    fail "unknown guarded FOD startup action: ${ACTION}"
    ;;
esac
