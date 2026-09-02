#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT}/scripts/docker_fod_install.sh"
LEGACY="${ROOT}/scripts/docker_fod_install_legacy.sh"
GUARD="${ROOT}/scripts/docker_fod_start_guard.sh"
PUBLIC="${ROOT}/make/fod-deploy-public.mk"

for file in "${SCRIPT}" "${LEGACY}" "${GUARD}" "${PUBLIC}"; do
  [[ -r "${file}" ]] || { echo "Missing FOD Docker policy file: ${file}" >&2; exit 1; }
done
bash -n "${SCRIPT}"
bash -n "${LEGACY}"
bash -n "${GUARD}"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/fod-docker-fod-install-policy.XXXXXX")"
trap 'rm -rf "${tmpdir}"' EXIT
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
for file in "${state}/compose.yml" "${state}/compose-fod.yml" "${state}/fod-container.ini"; do
  [[ -s "${file}" ]] || { echo "Missing rendered FOD Docker artifact: ${file}" >&2; exit 1; }
done

grep -Fq 'image: ghcr.io/stachwk/fod-client:3.4' "${state}/compose-fod.yml"
grep -Fq '/dev/fuse:/dev/fuse' "${state}/compose-fod.yml"
grep -Fq 'SYS_ADMIN' "${state}/compose-fod.yml"
grep -Fq 'propagation: rshared' "${state}/compose-fod.yml"

# The active host mount is validated by kernel filesystem identity, not by the
# number of propagated mount rows. rshared/bind propagation can expose several
# findmnt rows for one FUSE filesystem; MAJ:MIN must remain unique and match the
# FUSE device visible inside the container.
for pattern in \
  'host_fuse_rows()' \
  'host_fuse_devices()' \
  'host_fuse_device_count()' \
  'container_fuse_rows()' \
  'container_fuse_devices()' \
  'container_fuse_device_count()' \
  'active_fuse_identity_valid()' \
  'validate_active_fuse_identity()' \
  'MAJ:MIN' \
  'one FUSE filesystem identity across host propagation rows=' \
  'host/container FUSE device mismatch' \
  'OK: FOD FUSE identity device='; do
  grep -Fq -- "${pattern}" "${SCRIPT}" || { echo "Missing FUSE identity policy: ${pattern}" >&2; exit 1; }
done

if grep -Fq 'expected exactly 1' "${SCRIPT}"; then
  echo 'Public FOD Docker lifecycle must not require exactly one propagated host FUSE row.' >&2
  exit 1
fi

# Stale cleanup is still row-oriented after the container is gone: remove every
# top fuse* mount while retaining the underlying shared ext4/xfs bind.
for pattern in \
  'cleanup_stale_fuse_mounts()' \
  'Detected stale FUSE mount rows=' \
  'Unmounting stale FUSE row(s)' \
  'fusermount3 -u "${MOUNT_DIR}"' \
  '"${sudo_cmd[@]}" umount "${MOUNT_DIR}"' \
  'refusing to unmount non-FUSE top layer' \
  'underlying mount retained'; do
  grep -Fq -- "${pattern}" "${SCRIPT}" || { echo "Missing stale FUSE cleanup policy: ${pattern}" >&2; exit 1; }
done

# Down must remove the container first, then reconcile all propagated FUSE rows.
grep -A8 '^down()' "${SCRIPT}" | grep -Fq 'compose stop fod || true'
grep -A8 '^down()' "${SCRIPT}" | grep -Fq 'compose rm -f fod || true'
grep -A8 '^down()' "${SCRIPT}" | grep -Fq 'cleanup_stale_fuse_mounts 1'

# Guarded startup refreshes the mutable image, performs stale reconciliation,
# has bounded startup and applies AppArmor override where required.
for pattern in \
  'docker pull "${CLIENT_IMAGE}"' \
  'docker inspect' \
  "'{{.Image}}'" \
  'docker image inspect' \
  "'{{.Id}}'" \
  '"${container_image_id}" == "${expected_image_id}"' \
  'Replacing healthy FOD container because image changed:' \
  'docker rm -f "${CONTAINER_NAME}"' \
  'timeout --foreground' \
  'up -d --no-deps fod' \
  'apparmor=unconfined' \
  'FOD CONTAINER START DIAGNOSTICS' \
  'FOD_DOCKER_DEPLOY_FOD_INTERACTIVE_SUDO=1 fod_action cleanup-stale-mounts'; do
  grep -Fq -- "${pattern}" "${GUARD}" || { echo "Missing guarded startup policy: ${pattern}" >&2; exit 1; }
done

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
  grep -Eq "^${target}:" "${PUBLIC}" || { echo "Missing public FOD Docker target: ${target}" >&2; exit 1; }
done

if env "${common_env[@]}" MASTERS=2 SLAVES=1 bash "${SCRIPT}" plan >/dev/null 2>&1; then
  echo 'FOD Docker deployment must reject MASTERS>1.' >&2
  exit 1
fi

echo 'OK docker-fod-install-policy'
