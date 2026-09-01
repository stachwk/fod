#!/usr/bin/env bash
set -Eeuo pipefail

ENV_FILE="${FOD_DOCKER_SYSTEMD_ENV_FILE:-/etc/fod/docker-deploy.env}"
[[ -r "${ENV_FILE}" ]] || { echo "FOD systemd environment missing: ${ENV_FILE}" >&2; exit 2; }
# shellcheck disable=SC1090
source "${ENV_FILE}"

: "${FOD_RUNTIME_ROOT:?missing FOD_RUNTIME_ROOT}"
: "${HOME:?missing HOME}"
: "${MASTERS:?missing MASTERS}"
: "${SLAVES:?missing SLAVES}"
: "${FOD_DOCKER_DEPLOY_STATE_DIR:?missing FOD_DOCKER_DEPLOY_STATE_DIR}"
: "${FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR:?missing FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR}"
: "${FOD_DOCKER_DEPLOY_CLIENT_IMAGE:?missing FOD_DOCKER_DEPLOY_CLIENT_IMAGE}"

export HOME MASTERS SLAVES
export FOD_DOCKER_DEPLOY_STATE_DIR FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR
export FOD_DOCKER_DEPLOY_CLIENT_IMAGE
export FOD_DOCKER_DEPLOY_PROJECT="${FOD_DOCKER_DEPLOY_PROJECT:-fod-deploy}"
export FOD_DOCKER_DEPLOY_FOD_PULL_MODE="${FOD_DOCKER_DEPLOY_FOD_PULL_MODE:-if-missing}"
export FOD_DOCKER_DEPLOY_FOD_APPARMOR="${FOD_DOCKER_DEPLOY_FOD_APPARMOR:-auto}"

cd "${FOD_RUNTIME_ROOT}"

case "${1:-start}" in
  start)
    bash scripts/docker_fod_install.sh host-prepare
    bash scripts/docker_deploy.sh up
    bash scripts/docker_fod_start_guard.sh start
    bash scripts/docker_deploy.sh smoke
    bash scripts/docker_fod_install.sh smoke
    ;;
  stop)
    bash scripts/docker_fod_install.sh down || true
    bash scripts/docker_deploy.sh down || true
    ;;
  smoke)
    bash scripts/docker_deploy.sh smoke
    bash scripts/docker_fod_install.sh smoke
    ;;
  status)
    bash scripts/docker_deploy.sh status
    bash scripts/docker_fod_install.sh status
    ;;
  *)
    echo "Unknown FOD systemd boot action: ${1:-}" >&2
    exit 2
    ;;
esac
