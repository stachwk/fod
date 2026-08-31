#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAKEFILE="${ROOT}/GNUmakefile"
PUBLISH_8="${ROOT}/scripts/publish_postgres_fod_8k.sh"
PUBLISH="${ROOT}/scripts/publish_postgres_fod_32k.sh"
COMPOSE_DISK="${ROOT}/docker-compose.postgres-blocksize.yml"
COMPOSE_TMPFS="${ROOT}/docker-compose.postgres-blocksize-tmpfs.yml"
DOCKERFILE="${ROOT}/docker/postgres-blocksize/Dockerfile"

for file in "${MAKEFILE}" "${PUBLISH_8}" "${PUBLISH}" "${COMPOSE_DISK}" "${COMPOSE_TMPFS}" "${DOCKERFILE}"; do
    [[ -r "${file}" ]] || { echo "Missing ${file}" >&2; exit 1; }
done

bash -n "${PUBLISH_8}"
bash -n "${PUBLISH}"

grep -F 'postgres-8k-publish:' "${MAKEFILE}" >/dev/null
grep -F 'FOD_CONTAINER_PUSH=1 bash scripts/publish_postgres_fod_8k.sh' "${MAKEFILE}" >/dev/null
grep -F 'postgres-32k-publish:' "${MAKEFILE}" >/dev/null
grep -F 'FOD_CONTAINER_PUSH=1 bash scripts/publish_postgres_fod_32k.sh' "${MAKEFILE}" >/dev/null
grep -F 'postgres-all-publish: postgres-8k-publish postgres-32k-publish' "${MAKEFILE}" >/dev/null

grep -F 'FOD_POSTGRES_BLOCK_SIZE_KB=8' "${PUBLISH_8}" >/dev/null
grep -F 'postgres-16-fod-8k' "${PUBLISH_8}" >/dev/null
grep -F 'FOD_POSTGRES_BLOCK_SIZE_KB:-32' "${PUBLISH}" >/dev/null
grep -F 'postgres-16-fod-${BLOCK_SIZE_KB}k' "${PUBLISH}" >/dev/null
grep -F 'FOD_CONTAINER_TAG_LATEST:-0' "${PUBLISH}" >/dev/null
grep -F 'MAJOR_TAG="${IMAGE_BASE}:16"' "${PUBLISH}" >/dev/null

for compose in "${COMPOSE_DISK}" "${COMPOSE_TMPFS}"; do
    grep -F 'POSTGRES_INITDB_ARGS: "--auth-local=trust --auth-host=scram-sha-256"' "${compose}" >/dev/null
    if grep -Eq -- 'POSTGRES_INITDB_ARGS:.*--auth-host=trust' "${compose}"; then
        echo "Host authentication must remain SCRAM in ${compose}" >&2
        exit 1
    fi
done

grep -F -- '--no-sync --auth-local=trust --auth-host=trust' "${DOCKERFILE}" >/dev/null
grep -F 'initdb --no-sync --auth-local=trust --auth-host=trust' "${PUBLISH}" >/dev/null

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${PUBLISH_8}" "${PUBLISH}"; then
    echo 'PostgreSQL publishers must not perform global Docker pruning' >&2
    exit 1
fi

echo 'OK: PostgreSQL 8K/32K publish targets and explicit initdb auth policy'
