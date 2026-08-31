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
CLIENT_IMAGE="${FOD_DOCKER_DEPLOY_CLIENT_IMAGE:-ghcr.io/stachwk/fod-client:3.4}"
MOUNT_DIR="${FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR:-${XDG_DATA_HOME:-${HOME}/.local/share}/fod/mount}"
CONTAINER_MOUNT="${FOD_DOCKER_DEPLOY_FOD_CONTAINER_MOUNT:-/mnt/fod}"

BASE_COMPOSE="${STATE_DIR}/compose.yml"
FOD_COMPOSE="${STATE_DIR}/compose-fod.yml"
CONTAINER_CONFIG="${STATE_DIR}/fod-container.ini"
POSTGRES_ENV="${STATE_DIR}/postgres.env"

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
  [[ "${CONTAINER_MOUNT}" == /* ]] || fail "FOD_DOCKER_DEPLOY_FOD_CONTAINER_MOUNT must be absolute"
  [[ "${MOUNT_DIR}" == /* ]] || fail "FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR must be absolute"
}

ensure_mount_dir() {
  # A running FOD instance may already have a FUSE mount propagated onto this
  # path. Do not call mkdir on an active mountpoint: some FUSE states report
  # EEXIST instead of behaving like a normal directory during mkdir -p.
  if mountpoint -q "${MOUNT_DIR}" 2>/dev/null; then
    return 0
  fi
  if [[ -e "${MOUNT_DIR}" ]]; then
    [[ -d "${MOUNT_DIR}" ]] || fail "FOD mount path exists but is not a directory: ${MOUNT_DIR}"
    return 0
  fi
  mkdir -p -- "${MOUNT_DIR}"
}

base_action() {
  MASTERS="${MASTERS}" SLAVES="${SLAVES}" \
    FOD_DOCKER_DEPLOY_STATE_DIR="${STATE_DIR}" \
    FOD_DOCKER_DEPLOY_PROJECT="${PROJECT}" \
    FOD_DOCKER_DEPLOY_NETWORK="${NETWORK_NAME}" \
    FOD_DOCKER_DEPLOY_CLIENT_IMAGE="${CLIENT_IMAGE}" \
    bash "${ROOT}/scripts/docker_deploy.sh" "$@"
}

compose() {
  docker compose \
    -p "${PROJECT}" \
    -f "${BASE_COMPOSE}" \
    -f "${FOD_COMPOSE}" \
    "$@"
}

require_docker() {
  command -v docker >/dev/null 2>&1 || fail "docker is required"
  docker compose version >/dev/null 2>&1 || fail "Docker Compose v2 is required"
}

render() {
  base_action render >/dev/null
  ensure_mount_dir
  if ! mountpoint -q "${MOUNT_DIR}" 2>/dev/null; then
    chmod 0755 "${MOUNT_DIR}"
  fi

  cat > "${FOD_COMPOSE}" <<EOF
services:
  fod:
    image: ${CLIENT_IMAGE}
    container_name: ${PROJECT}-fod
    restart: unless-stopped
    depends_on:
      primary:
        condition: service_healthy
    env_file:
      - ./postgres.env
    devices:
      - /dev/fuse:/dev/fuse
    cap_add:
      - SYS_ADMIN
    volumes:
      - ./fod-container.ini:/etc/fod/fod.ini:ro
      - type: bind
        source: ${MOUNT_DIR}
        target: ${CONTAINER_MOUNT}
        bind:
          propagation: rshared
    command:
      - mount.fod
      - none
      - ${CONTAINER_MOUNT}
      - -o
      - ini=/etc/fod/fod.ini,role=auto,selinux=off,acl=off
    healthcheck:
      test:
        - CMD-SHELL
        - grep -Fq ' ${CONTAINER_MOUNT} ' /proc/mounts
      interval: 2s
      timeout: 5s
      retries: 60
      start_period: 5s
EOF

  if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    compose config --quiet
  fi

  printf 'fod_compose=%s\nfod_image=%s\nfod_host_mount=%s\nfod_container_mount=%s\n' \
    "${FOD_COMPOSE}" "${CLIENT_IMAGE}" "${MOUNT_DIR}" "${CONTAINER_MOUNT}"
}

mount_propagation() {
  if command -v findmnt >/dev/null 2>&1; then
    findmnt -n -o PROPAGATION -T "${MOUNT_DIR}" 2>/dev/null | tr -d '[:space:]'
  fi
}

preflight() {
  [[ -e /dev/fuse ]] || fail "/dev/fuse does not exist on the Docker host"
  [[ -r /dev/fuse && -w /dev/fuse ]] || {
    echo "WARNING: current user cannot directly read/write /dev/fuse; Docker daemon may still be able to pass it through." >&2
  }

  ensure_mount_dir
  local propagation=""
  propagation="$(mount_propagation || true)"
  case "${propagation}" in
    shared|rshared)
      ;;
    "")
      echo "WARNING: cannot determine mount propagation for ${MOUNT_DIR}; continuing." >&2
      ;;
    *)
      cat >&2 <<EOF
ERROR: ${MOUNT_DIR} is on a '${propagation}' mount.
A FUSE submount created inside the container will not propagate back to the host.

Prepare the host mountpoint once, then retry:
  sudo mount --bind '${MOUNT_DIR}' '${MOUNT_DIR}'
  sudo mount --make-rshared '${MOUNT_DIR}'

For persistence across reboot, configure an equivalent systemd mount/unit or host mount policy.
EOF
      exit 2
      ;;
  esac
}

host_prepare() {
  ensure_mount_dir
  local sudo_cmd=()
  if (( EUID != 0 )); then
    command -v sudo >/dev/null 2>&1 || fail "sudo is required to prepare host mount propagation"
    sudo_cmd=(sudo)
  fi

  if ! mountpoint -q "${MOUNT_DIR}" 2>/dev/null; then
    "${sudo_cmd[@]}" mount --bind "${MOUNT_DIR}" "${MOUNT_DIR}"
  fi
  "${sudo_cmd[@]}" mount --make-rshared "${MOUNT_DIR}"
  preflight
  echo "OK: FOD host mount propagation prepared for current boot: ${MOUNT_DIR}"
}

schema_exists() {
  [[ -f "${POSTGRES_ENV}" ]] || return 1
  source "${POSTGRES_ENV}"
  docker compose -p "${PROJECT}" -f "${BASE_COMPOSE}" exec -T primary \
    psql -U "${POSTGRES_USER}" -d "${POSTGRES_DB}" -Atqc \
    "SELECT to_regclass('fod.config') IS NOT NULL" 2>/dev/null | tr -d '\r' | grep -Fxq t
}

wait_for_fod() {
  local attempt status=""
  for attempt in $(seq 1 90); do
    status="$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' \
      "${PROJECT}-fod" 2>/dev/null || true)"
    [[ "${status}" == healthy ]] && break
    [[ "${status}" == exited || "${status}" == dead ]] && break
    sleep 1
  done
  [[ "${status}" == healthy ]] || {
    compose logs --tail=100 fod >&2 || true
    fail "FOD container did not become healthy; status=${status:-missing}"
  }
}

smoke() {
  [[ -f "${POSTGRES_ENV}" ]] || fail "missing ${POSTGRES_ENV}; render/install the database deployment first"
  source "${POSTGRES_ENV}"

  compose exec -T fod sh -ceu "
    grep -Fq ' ${CONTAINER_MOUNT} ' /proc/mounts
    test -d '${CONTAINER_MOUNT}'
    pg_isready -h primary -p 5432 -U \"\$POSTGRES_USER\" -d \"\$POSTGRES_DB\" >/dev/null
  "

  if ! mountpoint -q "${MOUNT_DIR}" 2>/dev/null; then
    if command -v findmnt >/dev/null 2>&1; then
      findmnt -T "${MOUNT_DIR}" -t fuse,fuse.fod,fuse3 >/dev/null 2>&1 || \
        echo "WARNING: host cannot positively identify ${MOUNT_DIR} as a FUSE mount; container health is OK." >&2
    fi
  fi

  printf 'OK: FOD Docker client healthy image=%s host_mount=%s\n' "${CLIENT_IMAGE}" "${MOUNT_DIR}"
}

start_fod() {
  render >/dev/null
  preflight
  schema_exists || fail "FOD schema is not initialized; run docker-deploy-fod-install or docker-deploy-install"
  compose up -d fod
  wait_for_fod
}

install_fod() {
  require_docker
  base_action up
  base_action fod-init
  render >/dev/null
  preflight
  docker pull "${CLIENT_IMAGE}"
  compose up -d fod
  wait_for_fod
  smoke
}

plan() {
  printf '%s\n' \
    '=== FOD DOCKER CLIENT PLAN ===' \
    "masters=${MASTERS}" \
    "slaves=${SLAVES}" \
    "client_image=${CLIENT_IMAGE}" \
    "network=${NETWORK_NAME}" \
    "host_mount=${MOUNT_DIR}" \
    "container_mount=${CONTAINER_MOUNT}" \
    "config=${CONTAINER_CONFIG}" \
    'requires=/dev/fuse + SYS_ADMIN + rshared host mount propagation'
}

validate

case "${ACTION}" in
  plan)
    plan
    ;;
  render)
    render
    ;;
  preflight)
    render >/dev/null
    preflight
    echo "OK: FOD Docker host preflight"
    ;;
  host-prepare)
    render >/dev/null
    host_prepare
    ;;
  start)
    require_docker
    start_fod
    ;;
  install)
    install_fod
    ;;
  up)
    require_docker
    base_action up
    start_fod
    smoke
    ;;
  down)
    require_docker
    [[ -f "${FOD_COMPOSE}" ]] || render >/dev/null
    compose stop fod
    compose rm -f fod
    ;;
  status)
    require_docker
    [[ -f "${FOD_COMPOSE}" ]] || render >/dev/null
    compose ps fod
    ;;
  logs)
    require_docker
    [[ -f "${FOD_COMPOSE}" ]] || render >/dev/null
    compose logs -f fod
    ;;
  smoke)
    require_docker
    [[ -f "${FOD_COMPOSE}" ]] || render >/dev/null
    wait_for_fod
    smoke
    ;;
  shell)
    require_docker
    [[ -f "${FOD_COMPOSE}" ]] || render >/dev/null
    compose exec fod /bin/sh
    ;;
  *)
    fail "unknown FOD Docker action: ${ACTION}"
    ;;
esac
