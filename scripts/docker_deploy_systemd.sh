#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

ACTION="${1:-plan}"
MASTERS="${MASTERS:-${FOD_DOCKER_DEPLOY_MASTERS:-1}}"
SLAVES="${SLAVES:-${FOD_DOCKER_DEPLOY_SLAVES:-1}}"
PROJECT="${FOD_DOCKER_DEPLOY_PROJECT:-fod-deploy}"
DEPLOY_USER="${FOD_DOCKER_DEPLOY_SYSTEMD_USER:-${SUDO_USER:-${USER:-}}}"
VERSION="$(tr -d '[:space:]' < fod_version.txt)"
CLIENT_IMAGE="${FOD_DOCKER_DEPLOY_CLIENT_IMAGE:-ghcr.io/stachwk/fod-client:${VERSION}}"
UNIT_NAME="${FOD_DOCKER_DEPLOY_SYSTEMD_UNIT:-fod-docker-deploy.service}"
ENV_FILE="${FOD_DOCKER_DEPLOY_SYSTEMD_ENV_FILE:-/etc/fod/docker-deploy.env}"
RUNTIME_ROOT="${FOD_DOCKER_DEPLOY_SYSTEMD_RUNTIME_ROOT:-/usr/local/libexec/fod-docker-deploy}"
START_NOW="${FOD_DOCKER_DEPLOY_SYSTEMD_START_NOW:-1}"

fail() { echo "ERROR: $*" >&2; exit 2; }

require_uint() {
  local name="$1" value="$2" min="$3" max="$4"
  [[ "${value}" =~ ^[0-9]+$ ]] || fail "${name} must be an integer"
  (( value >= min && value <= max )) || fail "${name} must be in range ${min}..${max}"
}

validate_plain_value() {
  local name="$1" value="$2"
  [[ -n "${value}" ]] || fail "${name} must not be empty"
  case "${value}" in
    *[[:space:]]*) fail "${name} must not contain whitespace" ;;
    *\"*) fail "${name} must not contain double quotes" ;;
    *\'*) fail "${name} must not contain single quotes" ;;
  esac
}

validate() {
  require_uint MASTERS "${MASTERS}" 1 1
  require_uint SLAVES "${SLAVES}" 0 32
  [[ "${START_NOW}" == 0 || "${START_NOW}" == 1 ]] || fail "FOD_DOCKER_DEPLOY_SYSTEMD_START_NOW must be 0 or 1"
  [[ "${UNIT_NAME}" =~ ^[A-Za-z0-9_.@-]+\.service$ ]] || fail "invalid systemd unit name: ${UNIT_NAME}"
  validate_plain_value FOD_DOCKER_DEPLOY_SYSTEMD_RUNTIME_ROOT "${RUNTIME_ROOT}"
  validate_plain_value FOD_DOCKER_DEPLOY_SYSTEMD_ENV_FILE "${ENV_FILE}"
  validate_plain_value FOD_DOCKER_DEPLOY_CLIENT_IMAGE "${CLIENT_IMAGE}"
}

resolve_home() {
  local value=""
  if [[ -n "${FOD_DOCKER_DEPLOY_SYSTEMD_HOME:-}" ]]; then
    value="${FOD_DOCKER_DEPLOY_SYSTEMD_HOME}"
  elif [[ -n "${DEPLOY_USER}" ]] && command -v getent >/dev/null 2>&1; then
    value="$(getent passwd "${DEPLOY_USER}" 2>/dev/null | awk -F: 'NR==1 {print $6}')"
  fi
  [[ -n "${value}" ]] || value="${HOME}"
  printf '%s\n' "${value}"
}

DEPLOY_HOME="$(resolve_home)"
STATE_DIR="${FOD_DOCKER_DEPLOY_STATE_DIR:-${DEPLOY_HOME}/.local/state/fod/docker-deploy}"
MOUNT_DIR="${FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR:-${DEPLOY_HOME}/.local/share/fod/mount}"
STAGING_DIR="${STATE_DIR}/systemd"
UNIT_FILE="${STAGING_DIR}/${UNIT_NAME}"
STAGED_ENV="${STAGING_DIR}/docker-deploy.env"
SYSTEM_UNIT_FILE="/etc/systemd/system/${UNIT_NAME}"

as_root() {
  if (( EUID == 0 )); then
    "$@"
  else
    command -v sudo >/dev/null 2>&1 || fail "sudo is required for systemd installation"
    sudo "$@"
  fi
}

render_runtime_state() {
  mkdir -p "${STAGING_DIR}"
  MASTERS="${MASTERS}" SLAVES="${SLAVES}" \
    FOD_DOCKER_DEPLOY_STATE_DIR="${STATE_DIR}" \
    FOD_DOCKER_DEPLOY_PROJECT="${PROJECT}" \
    FOD_DOCKER_DEPLOY_CLIENT_IMAGE="${CLIENT_IMAGE}" \
    bash scripts/docker_deploy.sh render >/dev/null
  MASTERS="${MASTERS}" SLAVES="${SLAVES}" \
    FOD_DOCKER_DEPLOY_STATE_DIR="${STATE_DIR}" \
    FOD_DOCKER_DEPLOY_PROJECT="${PROJECT}" \
    FOD_DOCKER_DEPLOY_CLIENT_IMAGE="${CLIENT_IMAGE}" \
    FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR="${MOUNT_DIR}" \
    bash scripts/docker_fod_install.sh render >/dev/null
}

render() {
  validate_plain_value FOD_DOCKER_DEPLOY_SYSTEMD_HOME "${DEPLOY_HOME}"
  validate_plain_value FOD_DOCKER_DEPLOY_STATE_DIR "${STATE_DIR}"
  validate_plain_value FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR "${MOUNT_DIR}"
  validate_plain_value FOD_DOCKER_DEPLOY_PROJECT "${PROJECT}"
  render_runtime_state

  cat > "${STAGED_ENV}" <<EOF
HOME=${DEPLOY_HOME}
MASTERS=${MASTERS}
SLAVES=${SLAVES}
FOD_RUNTIME_ROOT=${RUNTIME_ROOT}
FOD_DOCKER_DEPLOY_STATE_DIR=${STATE_DIR}
FOD_DOCKER_DEPLOY_PROJECT=${PROJECT}
FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR=${MOUNT_DIR}
FOD_DOCKER_DEPLOY_CLIENT_IMAGE=${CLIENT_IMAGE}
FOD_DOCKER_DEPLOY_FOD_APPARMOR=auto
EOF
  chmod 600 "${STAGED_ENV}"

  cat > "${UNIT_FILE}" <<EOF
[Unit]
Description=FOD Docker deployment (PostgreSQL 32K + replicas + FOD/FUSE)
Requires=docker.service
Wants=network-online.target
After=docker.service network-online.target
PartOf=docker.service
ConditionPathExists=/dev/fuse

[Service]
Type=oneshot
RemainAfterExit=yes
EnvironmentFile=${ENV_FILE}
ExecStart=${RUNTIME_ROOT}/boot.sh start
ExecStop=${RUNTIME_ROOT}/boot.sh stop
ExecReload=${RUNTIME_ROOT}/boot.sh start
TimeoutStartSec=600
TimeoutStopSec=180

[Install]
WantedBy=multi-user.target
EOF
  chmod 644 "${UNIT_FILE}"

  printf 'systemd_unit=%s\nenvironment=%s\nruntime_root=%s\nclient_image=%s\nmasters=%s\nslaves=%s\nstate_dir=%s\nmount_dir=%s\n' \
    "${UNIT_NAME}" "${ENV_FILE}" "${RUNTIME_ROOT}" "${CLIENT_IMAGE}" "${MASTERS}" "${SLAVES}" "${STATE_DIR}" "${MOUNT_DIR}"
}

install_runtime() {
  as_root install -d -m0755 "${RUNTIME_ROOT}/scripts" "${RUNTIME_ROOT}/docker/deploy" "$(dirname "${ENV_FILE}")"
  as_root install -m0755 docker/systemd/fod-docker-deploy-boot.sh "${RUNTIME_ROOT}/boot.sh"
  as_root install -m0755 scripts/docker_deploy.sh "${RUNTIME_ROOT}/scripts/docker_deploy.sh"
  as_root install -m0755 scripts/docker_fod_install.sh "${RUNTIME_ROOT}/scripts/docker_fod_install.sh"
  as_root install -m0755 scripts/docker_fod_install_legacy.sh "${RUNTIME_ROOT}/scripts/docker_fod_install_legacy.sh"
  as_root install -m0755 scripts/docker_fod_start_guard.sh "${RUNTIME_ROOT}/scripts/docker_fod_start_guard.sh"
  as_root install -m0755 docker/deploy/primary-init.sh "${RUNTIME_ROOT}/docker/deploy/primary-init.sh"
  as_root install -m0755 docker/deploy/replica-entrypoint.sh "${RUNTIME_ROOT}/docker/deploy/replica-entrypoint.sh"
  as_root install -m0600 "${STAGED_ENV}" "${ENV_FILE}"
  as_root install -m0644 "${UNIT_FILE}" "${SYSTEM_UNIT_FILE}"
}

install_systemd() {
  command -v systemctl >/dev/null 2>&1 || fail "systemctl is required"
  render
  install_runtime
  as_root systemctl daemon-reload
  as_root systemctl enable "${UNIT_NAME}"
  if [[ "${START_NOW}" == 1 ]]; then
    if as_root systemctl is-active --quiet "${UNIT_NAME}"; then
      as_root systemctl reload "${UNIT_NAME}"
    else
      as_root systemctl start "${UNIT_NAME}"
    fi
  fi
  echo "OK: installed ${UNIT_NAME}; exact FOD image=${CLIENT_IMAGE}"
}

status_systemd() {
  command -v systemctl >/dev/null 2>&1 || fail "systemctl is required"
  systemctl --no-pager --full status "${UNIT_NAME}"
}

smoke_systemd() {
  [[ -x "${RUNTIME_ROOT}/boot.sh" ]] || fail "systemd runtime is not installed: ${RUNTIME_ROOT}"
  as_root env FOD_DOCKER_SYSTEMD_ENV_FILE="${ENV_FILE}" "${RUNTIME_ROOT}/boot.sh" smoke
}

restart_systemd() {
  command -v systemctl >/dev/null 2>&1 || fail "systemctl is required"
  as_root systemctl restart "${UNIT_NAME}"
  smoke_systemd
}

uninstall_systemd() {
  command -v systemctl >/dev/null 2>&1 || fail "systemctl is required"
  as_root systemctl disable --now "${UNIT_NAME}" >/dev/null 2>&1 || true
  as_root rm -f "${SYSTEM_UNIT_FILE}" "${ENV_FILE}"
  as_root rm -rf "${RUNTIME_ROOT}"
  as_root systemctl daemon-reload
  echo "OK: removed ${UNIT_NAME}; Docker volumes and deployment state were preserved"
}

plan() {
  printf '%s\n' \
    '=== FOD DOCKER SYSTEMD PLAN ===' \
    "unit=${UNIT_NAME}" \
    "masters=${MASTERS}" \
    "slaves=${SLAVES}" \
    "client_image=${CLIENT_IMAGE}" \
    "runtime_root=${RUNTIME_ROOT}" \
    "environment=${ENV_FILE}" \
    "state_dir=${STATE_DIR}" \
    "mount_dir=${MOUNT_DIR}" \
    'boot_order=docker -> rshared host mount -> PostgreSQL -> FOD -> smoke' \
    'runtime_scripts=root-owned copy; user checkout is not executed by systemd'
}

validate
case "${ACTION}" in
  plan) plan ;;
  render) render ;;
  install) install_systemd ;;
  status) status_systemd ;;
  smoke) smoke_systemd ;;
  restart) restart_systemd ;;
  uninstall) uninstall_systemd ;;
  *) fail "unknown systemd deployment action: ${ACTION}" ;;
esac
