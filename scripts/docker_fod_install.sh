#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

ACTION="${1:-plan}"
MASTERS="${MASTERS:-${FOD_DOCKER_DEPLOY_MASTERS:-1}}"
SLAVES="${SLAVES:-${FOD_DOCKER_DEPLOY_SLAVES:-1}}"
STATE_DIR="${FOD_DOCKER_DEPLOY_STATE_DIR:-${XDG_STATE_HOME:-${HOME}/.local/state}/fod/docker-deploy}"
PROJECT="${FOD_DOCKER_DEPLOY_PROJECT:-fod-deploy}"
MOUNT_DIR="${FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR:-${XDG_DATA_HOME:-${HOME}/.local/share}/fod/mount}"
CONTAINER_MOUNT="${FOD_DOCKER_DEPLOY_FOD_CONTAINER_MOUNT:-/mnt/fod}"
CONTAINER_NAME="${PROJECT}-fod"
LEGACY="${ROOT}/scripts/docker_fod_install_legacy.sh"
BASE_COMPOSE="${STATE_DIR}/compose.yml"
FOD_COMPOSE="${STATE_DIR}/compose-fod.yml"
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
    findmnt -T "${MOUNT_DIR}" -o ID,PARENT,MAJ:MIN,TARGET,SOURCE,FSTYPE,PROPAGATION,OPTIONS >&2 2>/dev/null || true
  fi
  exit "${rc}"
}
trap on_error ERR

legacy() {
  MASTERS="${MASTERS}" SLAVES="${SLAVES}" \
    FOD_DOCKER_DEPLOY_STATE_DIR="${STATE_DIR}" \
    FOD_DOCKER_DEPLOY_PROJECT="${PROJECT}" \
    FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR="${MOUNT_DIR}" \
    FOD_DOCKER_DEPLOY_FOD_CONTAINER_MOUNT="${CONTAINER_MOUNT}" \
    bash "${LEGACY}" "$@"
}

is_fuse_type() {
  case "$1" in
    fuse|fuse.*|fuse3) return 0 ;;
    *) return 1 ;;
  esac
}

host_fuse_rows() {
  command -v findmnt >/dev/null 2>&1 || return 0
  findmnt -rn -T "${MOUNT_DIR}" -o FSTYPE,MAJ:MIN 2>/dev/null \
    | awk '$1 == "fuse" || $1 == "fuse3" || $1 ~ /^fuse\./ {print $1, $2}'
}

host_fuse_row_count() {
  host_fuse_rows | awk 'NF {n++} END {print n+0}'
}

host_fuse_devices() {
  host_fuse_rows | awk 'NF >= 2 {print $2}' | sort -u
}

host_fuse_device_count() {
  host_fuse_devices | awk 'NF {n++} END {print n+0}'
}

container_fuse_rows() {
  command -v docker >/dev/null 2>&1 || return 0
  docker exec "${CONTAINER_NAME}" findmnt -rn -T "${CONTAINER_MOUNT}" -o FSTYPE,MAJ:MIN 2>/dev/null \
    | awk '$1 == "fuse" || $1 == "fuse3" || $1 ~ /^fuse\./ {print $1, $2}' || true
}

container_fuse_row_count() {
  container_fuse_rows | awk 'NF {n++} END {print n+0}'
}

container_fuse_devices() {
  container_fuse_rows | awk 'NF >= 2 {print $2}' | sort -u
}

container_fuse_device_count() {
  container_fuse_devices | awk 'NF {n++} END {print n+0}'
}

fod_container_healthy() {
  command -v docker >/dev/null 2>&1 || return 1
  [[ "$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "${CONTAINER_NAME}" 2>/dev/null || true)" == healthy ]]
}

active_fuse_identity_valid() {
  fod_container_healthy || return 1

  local host_rows host_devices container_rows container_devices host_dev container_dev
  host_rows="$(host_fuse_row_count)"
  host_devices="$(host_fuse_device_count)"
  container_rows="$(container_fuse_row_count)"
  container_devices="$(container_fuse_device_count)"

  (( host_rows >= 1 && host_devices == 1 )) || return 1
  (( container_rows >= 1 && container_devices == 1 )) || return 1

  host_dev="$(host_fuse_devices | head -1)"
  container_dev="$(container_fuse_devices | head -1)"
  [[ -n "${host_dev}" && "${host_dev}" == "${container_dev}" ]]
}

validate_active_fuse_identity() {
  CURRENT_STAGE=validate-active-fuse
  fod_container_healthy || fail "FOD container is not healthy: ${CONTAINER_NAME}"

  local host_rows host_devices container_rows container_devices host_dev container_dev
  host_rows="$(host_fuse_row_count)"
  host_devices="$(host_fuse_device_count)"
  container_rows="$(container_fuse_row_count)"
  container_devices="$(container_fuse_device_count)"

  (( host_rows >= 1 )) || fail "healthy FOD container has no propagated host FUSE mount at ${MOUNT_DIR}"
  (( host_devices == 1 )) || {
    findmnt -T "${MOUNT_DIR}" -o ID,PARENT,MAJ:MIN,TARGET,SOURCE,FSTYPE,PROPAGATION,OPTIONS >&2 2>/dev/null || true
    fail "host FUSE rows=${host_rows} refer to ${host_devices} distinct devices; expected one FOD filesystem identity"
  }
  (( container_rows >= 1 )) || fail "healthy FOD container has no FUSE mount at ${CONTAINER_MOUNT}"
  (( container_devices == 1 )) || fail "container FUSE rows=${container_rows} refer to ${container_devices} distinct devices"

  host_dev="$(host_fuse_devices | head -1)"
  container_dev="$(container_fuse_devices | head -1)"
  [[ "${host_dev}" == "${container_dev}" ]] || \
    fail "host/container FUSE device mismatch host=${host_dev:-none} container=${container_dev:-none}"

  echo "OK: FOD FUSE identity device=${host_dev} host_rows=${host_rows} container_rows=${container_rows}"
}

top_mount_fstype() {
  command -v findmnt >/dev/null 2>&1 || return 0
  findmnt -rn -T "${MOUNT_DIR}" -o FSTYPE 2>/dev/null | tail -1 | tr -d '[:space:]'
}

cleanup_stale_fuse_mounts() {
  local interactive_sudo="${1:-0}" before after top attempts=0
  CURRENT_STAGE=cleanup-stale-fuse
  before="$(host_fuse_row_count)"
  (( before > 0 )) || return 0

  if active_fuse_identity_valid; then
    echo "FOD container is healthy; preserving one FUSE filesystem identity across host propagation rows=${before} device=$(host_fuse_devices | head -1)."
    return 0
  fi

  if fod_container_healthy; then
    echo "FOD container is healthy but host/container FUSE identities disagree; recreating the FOD container and mount."
    docker rm -f "${CONTAINER_NAME}" >/dev/null 2>&1 || true
    sleep 1
    before="$(host_fuse_row_count)"
  fi

  (( before > 0 )) || return 0
  echo "Detected stale FUSE mount rows=${before} at ${MOUNT_DIR}; preserving the underlying non-FUSE/shared bind."

  while (( before > 0 )); do
    (( attempts += 1 ))
    (( attempts <= 64 )) || fail "too many stacked FUSE mount rows at ${MOUNT_DIR}"
    top="$(top_mount_fstype)"
    is_fuse_type "${top}" || fail "refusing to unmount non-FUSE top layer fstype=${top:-unknown} at ${MOUNT_DIR}"

    echo "Unmounting stale FUSE row(s), before=${before}: ${MOUNT_DIR} fstype=${top}"
    if command -v fusermount3 >/dev/null 2>&1; then
      fusermount3 -u "${MOUNT_DIR}" >/dev/null 2>&1 || true
    elif command -v fusermount >/dev/null 2>&1; then
      fusermount -u "${MOUNT_DIR}" >/dev/null 2>&1 || true
    fi

    after="$(host_fuse_row_count)"
    if (( after >= before )); then
      local sudo_cmd=()
      if (( EUID != 0 )); then
        command -v sudo >/dev/null 2>&1 || fail "sudo is required to remove stale root-owned FUSE mounts"
        if [[ "${interactive_sudo}" == 1 ]]; then
          sudo_cmd=(sudo)
        elif sudo -n true >/dev/null 2>&1; then
          sudo_cmd=(sudo -n)
        else
          fail "stale root-owned FUSE mount requires sudo; run make docker-deploy-fod-host-prepare MASTERS=${MASTERS} SLAVES=${SLAVES}"
        fi
      fi
      "${sudo_cmd[@]}" umount "${MOUNT_DIR}"
      after="$(host_fuse_row_count)"
    fi

    (( after < before )) || fail "failed to remove stale FUSE mount row at ${MOUNT_DIR}"
    before="${after}"
  done

  echo "OK: stale FUSE mounts removed; underlying mount retained:"
  findmnt -T "${MOUNT_DIR}" -o ID,PARENT,MAJ:MIN,TARGET,SOURCE,FSTYPE,PROPAGATION,OPTIONS 2>/dev/null || true
}

ensure_mount_dir() {
  CURRENT_STAGE=ensure-mount-dir
  (( $(host_fuse_row_count) > 0 )) && return 0
  mountpoint -q "${MOUNT_DIR}" 2>/dev/null && return 0
  if [[ -L "${MOUNT_DIR}" ]]; then
    local resolved
    resolved="$(readlink -f -- "${MOUNT_DIR}" 2>/dev/null || true)"
    [[ -n "${resolved}" && -d "${resolved}" ]] || fail "FOD mount path is a dangling/non-directory symlink: ${MOUNT_DIR}"
    return 0
  fi
  if [[ -e "${MOUNT_DIR}" ]]; then
    [[ -d "${MOUNT_DIR}" ]] || fail "FOD mount path exists but is not a directory: ${MOUNT_DIR}"
    return 0
  fi
  mkdir -p -- "${MOUNT_DIR}"
}

mount_propagation() {
  command -v findmnt >/dev/null 2>&1 || return 0
  findmnt -rn -o PROPAGATION -T "${MOUNT_DIR}" 2>/dev/null | tail -1 | tr -d '[:space:]'
}

ensure_rendered() {
  [[ -f "${BASE_COMPOSE}" && -f "${FOD_COMPOSE}" ]] || legacy render >/dev/null
}

preflight() {
  CURRENT_STAGE=preflight
  [[ -e /dev/fuse ]] || fail "/dev/fuse does not exist on the Docker host"
  ensure_rendered
  ensure_mount_dir

  local rows propagation
  rows="$(host_fuse_row_count)"
  if (( rows > 0 )); then
    if fod_container_healthy; then
      validate_active_fuse_identity
    else
      fail "stale FUSE mount rows=${rows} remain at ${MOUNT_DIR}; run make docker-deploy-fod-host-prepare MASTERS=${MASTERS} SLAVES=${SLAVES}"
    fi
  fi

  propagation="$(mount_propagation || true)"
  case "${propagation}" in
    shared|rshared) ;;
    "") echo "WARNING: cannot determine mount propagation for ${MOUNT_DIR}; continuing." >&2 ;;
    *) fail "${MOUNT_DIR} is on a '${propagation}' mount; run make docker-deploy-fod-host-prepare MASTERS=${MASTERS} SLAVES=${SLAVES}" ;;
  esac
  echo "OK: FOD Docker host preflight"
}

host_prepare() {
  CURRENT_STAGE=host-prepare-cleanup
  cleanup_stale_fuse_mounts 1
  if active_fuse_identity_valid; then
    preflight
    echo "OK: FOD host mount is already active and correctly propagated: ${MOUNT_DIR}"
    return 0
  fi

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
  legacy render >/dev/null
  preflight
  echo "OK: FOD host mount propagation prepared for current boot: ${MOUNT_DIR}"
}

compose() {
  docker compose -p "${PROJECT}" -f "${BASE_COMPOSE}" -f "${FOD_COMPOSE}" "$@"
}

smoke() {
  CURRENT_STAGE=smoke
  ensure_rendered
  [[ -f "${POSTGRES_ENV}" ]] || fail "missing ${POSTGRES_ENV}; render/install the database deployment first"
  fod_container_healthy || fail "FOD container is not healthy: ${CONTAINER_NAME}"

  docker exec "${CONTAINER_NAME}" sh -ceu "
    grep -Fq ' ${CONTAINER_MOUNT} ' /proc/mounts
    test -d '${CONTAINER_MOUNT}'
    pg_isready -h primary -p 5432 -U \"\$POSTGRES_USER\" -d \"\$POSTGRES_DB\" >/dev/null
  "
  validate_active_fuse_identity
  printf 'OK: FOD Docker client healthy host_mount=%s\n' "${MOUNT_DIR}"
}

down() {
  CURRENT_STAGE=down
  ensure_rendered
  compose stop fod || true
  compose rm -f fod || true
  cleanup_stale_fuse_mounts 1
  ensure_mount_dir
}

validate_inputs() {
  [[ "${MASTERS}" =~ ^[0-9]+$ && "${MASTERS}" -eq 1 ]] || fail "MASTERS must be exactly 1"
  [[ "${SLAVES}" =~ ^[0-9]+$ && "${SLAVES}" -ge 0 && "${SLAVES}" -le 32 ]] || fail "SLAVES must be in range 0..32"
  [[ "${MOUNT_DIR}" == /* ]] || fail "FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR must be absolute"
  [[ "${CONTAINER_MOUNT}" == /* ]] || fail "FOD_DOCKER_DEPLOY_FOD_CONTAINER_MOUNT must be absolute"
  [[ -r "${LEGACY}" ]] || fail "missing legacy renderer/lifecycle helper: ${LEGACY}"
}

validate_inputs
mkdir -p -- "${STATE_DIR}"

case "${ACTION}" in
  plan|render|status|logs|shell)
    exec bash "${LEGACY}" "${ACTION}"
    ;;
  preflight)
    legacy render >/dev/null
    preflight
    ;;
  cleanup-stale-mounts)
    cleanup_stale_fuse_mounts "${FOD_DOCKER_DEPLOY_FOD_INTERACTIVE_SUDO:-0}"
    ensure_mount_dir
    ;;
  host-prepare)
    host_prepare
    ;;
  smoke)
    smoke
    ;;
  down)
    down
    ;;
  start|install|up)
    fail "direct action '${ACTION}' is handled by scripts/docker_fod_start_guard.sh; use the public make docker-deploy-fod-* target"
    ;;
  *)
    fail "unknown FOD Docker action: ${ACTION}"
    ;;
esac
