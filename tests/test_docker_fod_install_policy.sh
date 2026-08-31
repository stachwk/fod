#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT}/scripts/docker_fod_install.sh"
GUARD="${ROOT}/scripts/docker_fod_start_guard.sh"
PUBLIC="${ROOT}/make/fod-deploy-public.mk"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/fod-docker-fod-install-policy.XXXXXX")"
trap 'rm -rf "${tmpdir}"' EXIT

for file in "${SCRIPT}" "${GUARD}"; do
  [[ -r "${file}" ]] || { echo "Missing FOD Docker script: ${file}" >&2; exit 1; }
  bash -n "${file}"
done

state="${tmpdir}/state"
mount_dir="${tmpdir}/mount"
common_env=(
  FOD_DOCKER_DEPLOY_STATE_DIR="${state}"
  FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR="${mount_dir}"
  POSTGRES_PASSWORD=postgres_policy_secret
  FOD_REPLICATION_PASSWORD=replication_policy_secret
  FOD_SCHEMA_ADMIN_PASSWORD=admin_policy_secret
)

env "${common_env[@]}" MASTERS=1 SLAVES=2 bash "${SCRIPT}" render >/dev/null

base_compose="${state}/compose.yml"
fod_compose="${state}/compose-fod.yml"
config="${state}/fod-container.ini"

for file in "${base_compose}" "${fod_compose}" "${config}"; do
  [[ -s "${file}" ]] || { echo "Missing FOD Docker artifact: ${file}" >&2; exit 1; }
done

grep -Fq 'image: ghcr.io/stachwk/fod-client:3.4' "${fod_compose}"
grep -Fq 'container_name: fod-deploy-fod' "${fod_compose}"
grep -Fq '/dev/fuse:/dev/fuse' "${fod_compose}"
grep -Fq 'SYS_ADMIN' "${fod_compose}"
grep -Fq 'propagation: rshared' "${fod_compose}"
grep -Fq "source: ${mount_dir}" "${fod_compose}"
grep -Fq 'target: /mnt/fod' "${fod_compose}"
grep -Fq './fod-container.ini:/etc/fod/fod.ini:ro' "${fod_compose}"
grep -Fq 'ini=/etc/fod/fod.ini,role=auto,selinux=off,acl=off' "${fod_compose}"
grep -Fq "grep -Fq ' /mnt/fod ' /proc/mounts" "${fod_compose}"
grep -Fq 'condition: service_healthy' "${fod_compose}"

for target in \
  docker-deploy-fod-plan \
  docker-deploy-fod-render \
  docker-deploy-fod-preflight \
  docker-deploy-fod-host-prepare \
  docker-deploy-fod-install \
  docker-deploy-fod-up \
  docker-deploy-fod-down \
  docker-deploy-fod-status \
  docker-deploy-fod-smoke \
  docker-deploy-fod-logs \
  docker-deploy-fod-shell \
  docker-deploy-fod-diagnostics \
  test-docker-fod-install-policy; do
  grep -Eq "^${target}:" "${PUBLIC}" || {
    echo "Missing public FOD Docker target: ${target}" >&2
    exit 1
  }
done

grep -A3 '^docker-deploy-install:' "${PUBLIC}" | grep -Fq 'scripts/docker_deploy.sh install'
grep -A3 '^docker-deploy-install:' "${PUBLIC}" | grep -Fq 'scripts/docker_fod_start_guard.sh start'
grep -A2 '^docker-deploy-fod-install:' "${PUBLIC}" | grep -Fq 'scripts/docker_fod_start_guard.sh install'
grep -A2 '^docker-deploy-fod-up:' "${PUBLIC}" | grep -Fq 'scripts/docker_fod_start_guard.sh up'
grep -Fq 'FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR' "${SCRIPT}"
grep -Fq 'mount --make-rshared' "${SCRIPT}"
grep -Fq 'FOD schema is not initialized' "${SCRIPT}"

# Startup must never hang indefinitely in compose up. The guard bypasses
# already-validated dependencies, refreshes the mutable series image tag,
# applies an AppArmor override when appropriate, and emits diagnostics on
# timeout/failure.
grep -Fq 'FOD_DOCKER_DEPLOY_FOD_START_TIMEOUT_SECONDS' "${GUARD}"
grep -Fq 'FOD_DOCKER_DEPLOY_FOD_HEALTH_TIMEOUT_SECONDS' "${GUARD}"
grep -Fq 'docker pull "${CLIENT_IMAGE}"' "${GUARD}"
grep -Fq 'timeout --foreground' "${GUARD}"
grep -Fq 'up -d --no-deps fod' "${GUARD}"
grep -Fq 'apparmor=unconfined' "${GUARD}"
grep -Fq 'FOD CONTAINER START DIAGNOSTICS' "${GUARD}"
grep -Fq 'docker inspect' "${GUARD}"
grep -Fq 'findmnt -T' "${GUARD}"

if MASTERS=2 SLAVES=1 FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR="${mount_dir}" bash "${SCRIPT}" plan >/dev/null 2>&1; then
  echo 'FOD Docker install must reject MASTERS>1 together with the database deployment.' >&2
  exit 1
fi

echo 'OK docker-fod-install-policy'
