#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT}/scripts/docker_deploy.sh"
PUBLIC_DEPLOY="${ROOT}/make/fod-deploy-public.mk"
PRIMARY_INIT="${ROOT}/docker/deploy/primary-init.sh"
REPLICA_ENTRY="${ROOT}/docker/deploy/replica-entrypoint.sh"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/fod-docker-deploy-policy.XXXXXX")"
trap 'rm -rf "${tmpdir}"' EXIT

for file in "${SCRIPT}" "${PUBLIC_DEPLOY}" "${PRIMARY_INIT}" "${REPLICA_ENTRY}"; do
  [[ -r "${file}" ]] || { echo "Missing deployment file: ${file}" >&2; exit 1; }
done
for file in "${SCRIPT}" "${PRIMARY_INIT}" "${REPLICA_ENTRY}"; do
  bash -n "${file}"
done

state="${tmpdir}/state"
common_env=(
  FOD_DOCKER_DEPLOY_STATE_DIR="${state}"
  POSTGRES_PASSWORD=postgres_policy_secret
  FOD_REPLICATION_PASSWORD=replication_policy_secret
  FOD_SCHEMA_ADMIN_PASSWORD=admin_policy_secret
)

env "${common_env[@]}" MASTERS=1 SLAVES=2 bash "${SCRIPT}" render >/dev/null

compose="${state}/compose.yml"
host_ini="${state}/fod-host.ini"
container_ini="${state}/fod-container.ini"

for file in "${compose}" "${host_ini}" "${container_ini}" "${state}/postgres.env" "${state}/fod-admin.env"; do
  [[ -s "${file}" ]] || { echo "Missing rendered deployment artifact: ${file}" >&2; exit 1; }
done

grep -Fq 'image: ghcr.io/stachwk/postgres-16-fod-32k:16.15' "${compose}"
grep -Fq '  primary:' "${compose}"
grep -Fq '  replica1:' "${compose}"
grep -Fq '  replica2:' "${compose}"
grep -Fq 'FOD_REPLICA_COUNT: "2"' "${compose}"
grep -Fq 'max_slot_wal_keep_size=4GB' "${compose}"
grep -Fq 'replica1_data:' "${compose}"
grep -Fq 'replica2_data:' "${compose}"
if grep -Fq 'postgres_policy_secret' "${compose}" || grep -Fq 'replication_policy_secret' "${compose}"; then
  echo 'Deployment compose must not embed database secrets.' >&2
  exit 1
fi

grep -Fq 'primary_hosts = 127.0.0.1:55441' "${host_ini}"
grep -Fq 'replica_hosts = 127.0.0.1:55442,127.0.0.1:55443' "${host_ini}"
grep -Fq 'primary_hosts = primary:5432' "${container_ini}"
grep -Fq 'replica_hosts = replica1:5432,replica2:5432' "${container_ini}"
grep -Fq 'pg_endpoint_routing_enabled = true' "${container_ini}"

mode_pg="$(stat -c '%a' "${state}/postgres.env")"
mode_admin="$(stat -c '%a' "${state}/fod-admin.env")"
[[ "${mode_pg}" == 600 && "${mode_admin}" == 600 ]] || {
  echo "Deployment secret files must be mode 600: postgres=${mode_pg} admin=${mode_admin}" >&2
  exit 1
}

single_state="${tmpdir}/single"
env \
  FOD_DOCKER_DEPLOY_STATE_DIR="${single_state}" \
  POSTGRES_PASSWORD=postgres_policy_secret \
  FOD_REPLICATION_PASSWORD=replication_policy_secret \
  FOD_SCHEMA_ADMIN_PASSWORD=admin_policy_secret \
  MASTERS=1 SLAVES=0 bash "${SCRIPT}" render >/dev/null
if grep -Eq '^  replica[0-9]+:' "${single_state}/compose.yml"; then
  echo 'SLAVES=0 unexpectedly rendered a replica service.' >&2
  exit 1
fi
if grep -Fq 'replica_hosts' "${single_state}/fod-container.ini"; then
  echo 'SLAVES=0 unexpectedly rendered replica_hosts.' >&2
  exit 1
fi

if MASTERS=2 SLAVES=1 bash "${SCRIPT}" plan >/dev/null 2>&1; then
  echo 'MASTERS>1 must be rejected because multi-master PostgreSQL is unsupported.' >&2
  exit 1
fi

for target in \
  docker-deploy-plan \
  docker-deploy-render \
  docker-deploy-install \
  docker-deploy-up \
  docker-deploy-down \
  docker-deploy-status \
  docker-deploy-smoke \
  docker-deploy-logs \
  docker-deploy-fod-init \
  docker-deploy-fod-upgrade \
  docker-deploy-destroy \
  docker-deploy-single-install \
  docker-deploy-one-replica-install \
  docker-deploy-two-replicas-install; do
  grep -Eq "^${target}:" "${PUBLIC_DEPLOY}" || {
    echo "Missing public deployment target: ${target}" >&2
    exit 1
  }
done

echo 'OK docker-deploy-policy'
