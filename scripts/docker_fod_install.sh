#!/usr/bin/env bash
set -Eeuo pipefail

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
CONTAINER_NAME="${PROJECT}-fod"

BASE_COMPOSE="${STATE_DIR}/compose.yml"
FOD_COMPOSE="${STATE_DIR}/compose-fod.yml"
CONTAINER_CONFIG="${STATE_DIR}/fod-container.ini"
POSTGRES_ENV="${STATE_DIR}/postgres.env"
CURRENT_STAGE=init

fail() {
  echo "ERROR: $*" >&2
  exit 2
}

on_error() {
  local rc=$? line="${BASH_LINENO[0]:-?}" cmd="${BASH_COMMAND:-?}"
  echo "ERROR: docker_fod_install stage=${CURRENT_STAGE} rc=${rc} line=${line} command=${cmd}" >&2
  echo "ERROR: mount_dir=${MOUNT_DIR}" >&2
  if command -v findmnt >/dev/null 2>&1; then
    findmnt -T "${MOUNT_DIR}" -o TARGET,SOURCE,FSTYPE,PROPAGATION,OPTIONS >&2 2>/dev/null || true
  fi
  ls -ld -- "${MOUNT_DIR}" >&2 2>/dev/null || true
  exit "${rc}"
}
trap on_error ERR

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

mount_fstypes() {
  command -v findmnt >/dev/null 2>&1 || return 0
  findmnt -rn -T "${MOUNT_DIR}" -o FSTYPE 2>/dev/null || true
}

fuse_mount_count() {
  local count="0"
  count="$(mount_fstypes | grep -Ec '^(fuse|fuse\..*|fuse3)$' || true)"
  printf '%s\n' "${count:-0}"
}

top_mount_fstype() {
  mount_fstypes | tail -1 | tr -d '[:space:]'
}

fod_container_healthy() {
  command -v docker >/dev/null 2>&1 || return 1
  [[ "$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "${CONTAINER_NAME}" 2>/dev/null || true)" == healthy ]]
}

cleanup_stale_fuse_mounts() {
  local interactive_sudo="${1:-0}" before="0" after="0" top="" attempts=0
  CURRENT_STAGE=cleanup-stale-fuse

  before="$(fuse_mount_count)"
  (( before > 0 )) || return 0

  if fod_container_healthy; then
    echo "FOD container is healthy; preserving active FUSE mount layers=${before}: ${MOUNT_DIR}"
    return 0
  fi

  echo "Detected stale FUSE mount layers=${before} at ${MOUNT_DIR}; preserving the underlying non-FUSE/shared bind."
  while (( before > 0 )); do
    (( attempts += 1 ))
    (( attempts <= 64 )) || fail "too many stacked FUSE mount layers at ${MOUNT_DIR}"

    top="$(top_mount_fstype)"
    case "${top}" in
      fuse|fuse.*|fuse3) ;;
      *)
        fail "stale FUSE layer exists below unexpected top mount fstype=${top:-unknown}; refusing to unmount a non-FUSE layer"
        ;;
    esac

    echo "Unmounting stale FUSE layer ${before}: ${MOUNT_DIR} fstype=${top}"
    if command -v fusermount3 >/dev/null 2>&1; then
      fusermount3 -u "${MOUNT_DIR}" >/dev/null 2>&1 || true
    elif command -v fusermount >/dev/null 2>&1; then
      fusermount -u "${MOUNT_DIR}" >/dev/null 2>&1 || true
    fi

    after="$(fuse_mount_count)"
    if (( after >= before )); then
      local sudo_cmd=()
      if (( EUID != 0 )); then
        command -v sudo >/dev/null 2>&1 || \
          fail "root is required to unmount stale root-owned FUSE layer; run sudo umount '${MOUNT_DIR}' once per FUSE layer"
        if [[ "${interactive_sudo}" == 1 ]]; then
          sudo_cmd=(sudo)
        elif sudo -n true >/dev/null 2>&1; then
          sudo_cmd=(sudo -n)
        else
          fail "stale root-owned FUSE mount requires sudo; run: make docker-deploy-fod-host-prepare MASTERS=${MASTERS} SLAVES=${SLAVES}"
        fi
      fi
      "${sudo_cmd[@]}" umount "${MOUNT_DIR}"
      after="$(fuse_mount_count)"
    fi

    (( after < before )) || fail "failed to remove stale FUSE layer at ${MOUNT_DIR}"
    before="${after}"
  done

  echo "OK: stale FUSE layers removed; underlying mount retained:"
  if command -v findmnt >/dev/null 2>&1; then
    findmnt -T "${MOUNT_DIR}" -o TARGET,SOURCE,FSTYPE,PROPAGATION,OPTIONS 2>/dev/null || true
  fi
}

path_entry_exists() {
  [[ -e "${MOUNT_DIR}" || -L "${MOUNT_DIR}" ]] && return 0
  mountpoint -q "${MOUNT_DIR}" 2>/dev/null && return 0
  if command -v findmnt >/dev/null 2>&1; then
    findmnt -n -T "${MOUNT_DIR}" >/dev/null 2>&1 && [[ -e "${MOUNT_DIR}" || -L "${MOUNT_DIR}" ]] && return 0
  fi
  return 1
}

ensure_mount_dir() {
  CURRENT_STAGE=ensure-mount-dir

  if mountpoint -q "${MOUNT_DIR}" 2>/dev/null; then
    return 0
  fi

  if [[ -L "${MOUNT_DIR}" ]]; then
    local resolved=""
    resolved="$(readlink -f -- "${MOUNT_DIR}" 2>/dev/null || true)"
    [[ -n "${resolved}" && -d "${resolved}" ]] || \
      fail "FOD mount path is a dangling/non-directory symlink: ${MOUNT_DIR}"
    return 0
  fi

  if [[ -e "${MOUNT_DIR}" ]]; then
    [[ -d "${MOUNT_DIR}" ]] || fail "FOD mount path exists but is not a directory: ${MOUNT_DIR}"
    return 0
  fi

  set +e
  mkdir -p -- "${MOUNT_DIR}" 2>"${STATE_DIR}/.fod-mkdir.err"
  local mkdir_rc=$?
  set -e
  if (( mkdir_rc != 0 )); then
    if mountpoint -q "${MOUNT_DIR}" 2>/dev/null || [[ -d "${MOUNT_DIR}" ]]; then
      rm -f "${STATE_DIR}/.fod-mkdir.err"
      return 0
    fi
    if [[ -L "${MOUNT_DIR}" ]]; then
      local resolved_after=""
      resolved_after="$(readlink -f -- "${MOUNT_DIR}" 2>/dev/null || true)"
      if [[ -n "${resolved_after}" && -d "${resolved_after}" ]]; then
        rm -f "${STATE_DIR}/.fod-mkdir.err"
        return 0
      fi
    fi
    echo "ERROR: cannot prepare FOD mount directory: ${MOUNT_DIR}" >&2
    cat "${STATE_DIR}/.fod-mkdir.err" >&2 2>/dev/null || true
    rm -f "${STATE_DIR}/.fod-mkdir.err"
    if command -v findmnt >/dev/null 2>&1; then
      findmnt -T "${MOUNT_DIR}" -o TARGET,SOURCE,FSTYPE,PROPAGATION,OPTIONS >&2 2>/dev/null || true
    fi
    ls -ld -- "$(dirname -- "${MOUNT_DIR}")" "${MOUNT_DIR}" >&2 2>/dev/null || true
    fail "mount directory preparation failed; inspect the path diagnostics above"
  fi
  rm -f "${STATE_DIR}/.fod-mkdir.err"
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
  CURRENT_STAGE=render-base
  base_action render >/dev/null
  mkdir -p -- "${STATE_DIR}"
  CURRENT_STAGE=render-mount-dir
  ensure_mount_dir
  if ! mountpoint -q "${MOUNT_DIR}" 2>/dev/null && [[ -d "${MOUNT_DIR}" && ! -L "${MOUNT_DIR}" ]]; then
    chmod 0755 "${MOUNT_DIR}"
  fi

  CURRENT_STAGE=render-compose
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
    findmnt -rn -o PROPAGATION -T "${MOUNT_DIR}" 2>/dev/null | tail -1 | tr -d '[:space:]'
  fi
}

preflight() {
  CURRENT_STAGE=preflight
  [[ -e /dev/fuse ]] || fail "/dev/fuse does not exist on the Docker host"
  [[ -r /dev/fuse && -w /dev/fuse ]] || {
    echo "WARNING: current user cannot directly read/write /dev/fuse; Docker daemon may still be able to pass it through." >&2
  }

  ensure_mount_dir
  local stale_count="0"
  stale_count="$(fuse_mount_count)"
  if (( stale_count > 0 )) && ! fod_container_healthy; then
    fail "stale FUSE mount layers=${stale_count} remain at ${MOUNT_DIR}; run make docker-deploy-fod-host-prepare MASTERS=${MASTERS} SLAVES=${SLAVES}"
  fi

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
  CURRENT_STAGE=host-prepare-cleanup
  cleanup_stale_fuse_mounts 1
  CURRENT_STAGE=host-prepare
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
  CURRENT_STAGE=wait-for-fod
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
  CURRENT_STAGE=smoke
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
mkdir -p -- "${STATE_DIR}"

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
  cleanup-stale-mounts)
    cleanup_stale_fuse_mounts 0
    ensure_mount_dir
    ;;
  host-prepare)
    host_prepare
    render >/dev/null
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