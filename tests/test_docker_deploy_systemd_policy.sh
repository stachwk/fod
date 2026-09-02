#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT}/scripts/docker_deploy_systemd.sh"
BOOT="${ROOT}/docker/systemd/fod-docker-deploy-boot.sh"
RELEASE_MK="${ROOT}/make/fod-deploy-release.mk"
GNU="${ROOT}/GNUmakefile"
VERSION="$(tr -d '[:space:]' < "${ROOT}/fod_version.txt")"
EXPECTED_IMAGE="ghcr.io/stachwk/fod-client:${VERSION}"

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT
mkdir -p "${tmp}/home" "${tmp}/state" "${tmp}/mount"

for file in "${SCRIPT}" "${BOOT}" "${RELEASE_MK}" "${GNU}"; do
  [[ -r "${file}" ]] || { echo "Missing Docker systemd policy file: ${file}" >&2; exit 1; }
done

bash -n "${SCRIPT}"

install_body="$(sed -n '/^install_systemd()/,/^}/p' "${SCRIPT}")"

for pattern in \
  'systemctl is-active --quiet "${UNIT_NAME}"' \
  'systemctl reload "${UNIT_NAME}"' \
  'systemctl start "${UNIT_NAME}"'; do
  grep -Fq -- "${pattern}" <<<"${install_body}" || {
    echo "Missing non-disruptive systemd install policy: ${pattern}" >&2
    exit 1
  }
done

if grep -Fq 'systemctl restart "${UNIT_NAME}"' <<<"${install_body}"; then
  echo 'Active systemd reinstall must not restart PostgreSQL deployment.' >&2
  exit 1
fi


bash -n "${BOOT}"

grep -Fq 'include make/fod-deploy-release.mk' "${GNU}"
grep -Fq 'FOD_RELEASE_VERSION := $(strip $(shell cat fod_version.txt))' "${RELEASE_MK}"
grep -Fq 'FOD_DOCKER_DEPLOY_CLIENT_IMAGE ?= ghcr.io/stachwk/fod-client:$(FOD_RELEASE_VERSION)' "${RELEASE_MK}"
grep -Fq 'FOD_CLIENT_IMAGE_VERSION ?= $(FOD_RELEASE_VERSION)' "${RELEASE_MK}"
grep -Fq 'export FOD_DOCKER_DEPLOY_CLIENT_IMAGE' "${RELEASE_MK}"
grep -Fq 'export FOD_CLIENT_IMAGE_VERSION' "${RELEASE_MK}"

for target in \
  docker-deploy-systemd-plan \
  docker-deploy-systemd-render \
  docker-deploy-systemd-install \
  docker-deploy-systemd-status \
  docker-deploy-systemd-smoke \
  docker-deploy-systemd-restart \
  docker-deploy-systemd-uninstall \
  test-docker-deploy-systemd-policy; do
  grep -Eq "^${target}:" "${RELEASE_MK}" || { echo "Missing systemd Make target: ${target}" >&2; exit 1; }
done

for pattern in \
  'install -m0755 scripts/docker_deploy.sh' \
  'install -m0755 scripts/docker_fod_install.sh' \
  'install -m0755 scripts/docker_fod_install_legacy.sh' \
  'install -m0755 scripts/docker_fod_start_guard.sh' \
  'install -m0755 docker/deploy/primary-init.sh' \
  'install -m0755 docker/deploy/replica-entrypoint.sh' \
  'user checkout is not executed by systemd' \
  'systemctl enable "${UNIT_NAME}"' \
  'systemctl is-active --quiet "${UNIT_NAME}"' \
  'systemctl reload "${UNIT_NAME}"' \
  'systemctl start "${UNIT_NAME}"' \
  'Docker volumes and deployment state were preserved'; do
  grep -Fq -- "${pattern}" "${SCRIPT}" || { echo "Missing systemd safety/lifecycle policy: ${pattern}" >&2; exit 1; }
done

for pattern in \
  'bash scripts/docker_fod_install.sh host-prepare' \
  'bash scripts/docker_deploy.sh up' \
  'bash scripts/docker_fod_start_guard.sh start' \
  'bash scripts/docker_deploy.sh smoke' \
  'bash scripts/docker_fod_install.sh smoke' \
  'bash scripts/docker_fod_install.sh down || true' \
  'bash scripts/docker_deploy.sh down || true'; do
  grep -Fq -- "${pattern}" "${BOOT}" || { echo "Missing boot lifecycle step: ${pattern}" >&2; exit 1; }
done

FOD_DOCKER_DEPLOY_SYSTEMD_HOME="${tmp}/home" \
FOD_DOCKER_DEPLOY_STATE_DIR="${tmp}/state" \
FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR="${tmp}/mount" \
FOD_DOCKER_DEPLOY_CLIENT_IMAGE="${EXPECTED_IMAGE}" \
MASTERS=1 SLAVES=2 \
  bash "${SCRIPT}" render >/dev/null

unit="${tmp}/state/systemd/fod-docker-deploy.service"
env_file="${tmp}/state/systemd/docker-deploy.env"
[[ -f "${unit}" && -f "${env_file}" ]] || { echo 'systemd render did not create staged files' >&2; exit 1; }

for pattern in \
  'Requires=docker.service' \
  'Wants=network-online.target' \
  'After=docker.service network-online.target' \
  'PartOf=docker.service' \
  'ConditionPathExists=/dev/fuse' \
  'Type=oneshot' \
  'RemainAfterExit=yes' \
  'ExecStart=/usr/local/libexec/fod-docker-deploy/boot.sh start' \
  'ExecStop=/usr/local/libexec/fod-docker-deploy/boot.sh stop' \
  'ExecReload=/usr/local/libexec/fod-docker-deploy/boot.sh start' \
  'WantedBy=multi-user.target'; do
  grep -Fq -- "${pattern}" "${unit}" || { echo "Missing rendered unit policy: ${pattern}" >&2; exit 1; }
done

grep -Fxq "FOD_DOCKER_DEPLOY_CLIENT_IMAGE=${EXPECTED_IMAGE}" "${env_file}"
grep -Fxq 'MASTERS=1' "${env_file}"
grep -Fxq 'SLAVES=2' "${env_file}"
grep -Fxq "FOD_DOCKER_DEPLOY_STATE_DIR=${tmp}/state" "${env_file}"
grep -Fxq "FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR=${tmp}/mount" "${env_file}"

if grep -Eq 'POSTGRES_PASSWORD|FOD_REPLICATION_PASSWORD|FOD_SCHEMA_ADMIN_PASSWORD' "${env_file}"; then
  echo 'systemd EnvironmentFile must not duplicate deployment secrets' >&2
  exit 1
fi

mode="$(stat -c '%a' "${env_file}")"
[[ "${mode}" == 600 ]] || { echo "systemd staged environment must be 0600, got ${mode}" >&2; exit 1; }

if MASTERS=2 SLAVES=1 FOD_DOCKER_DEPLOY_SYSTEMD_HOME="${tmp}/home" bash "${SCRIPT}" plan >/dev/null 2>&1; then
  echo 'systemd deployment must reject MASTERS>1' >&2
  exit 1
fi

make -C "${ROOT}" --no-print-directory -n docker-deploy-systemd-plan MASTERS=1 SLAVES=2 >/dev/null

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${SCRIPT}" "${BOOT}"; then
  echo 'systemd deployment must not perform global Docker pruning' >&2
  exit 1
fi

echo "OK docker-deploy-systemd-policy image=${EXPECTED_IMAGE}"
