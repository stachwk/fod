#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAKEFILE="${ROOT}/Makefile"
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

for pattern in \
    'FOD_FORWARD_TARGET,docker-postgres-8k-build,docker-postgres-8k-build' \
    'FOD_FORWARD_TARGET,docker-postgres-8k-publish,docker-postgres-8k-publish' \
    'FOD_FORWARD_TARGET,docker-postgres-32k-build,docker-postgres-32k-build' \
    'FOD_FORWARD_TARGET,docker-postgres-32k-publish,docker-postgres-32k-publish' \
    'FOD_FORWARD_TARGET,docker-postgres-all-build,docker-postgres-all-build' \
    'FOD_FORWARD_TARGET,docker-postgres-all-publish,docker-postgres-all-publish' \
    'test-docker-postgres-policy:'; do
    grep -Fq -- "${pattern}" "${MAKEFILE}" || { echo "Missing normalized PostgreSQL Docker Make target: ${pattern}" >&2; exit 1; }
done

for obsolete in \
    'postgres-publish:' \
    'postgres-8k-publish:' \
    'postgres-32k-publish:' \
    'postgres-all-publish:' \
    'docker-postgres-test-policy:' \
    'test-postgres-container-publish-policy:' \
    'test-postgres-32k-default-policy:'; do
    if grep -Fxq -- "${obsolete}" "${MAKEFILE}"; then
        echo "Obsolete PostgreSQL Make target must not be restored: ${obsolete}" >&2
        exit 1
    fi
done

grep -F 'FOD_POSTGRES_BLOCK_SIZE_KB=8' "${PUBLISH_8}" >/dev/null
grep -F 'postgres-16-fod-8k' "${PUBLISH_8}" >/dev/null
grep -F 'FOD_POSTGRES_BLOCK_SIZE_KB:-32' "${PUBLISH}" >/dev/null
grep -F 'POSTGRES_MAJOR="${POSTGRES_VERSION%%.*}"' "${PUBLISH}" >/dev/null
grep -F 'postgres-${POSTGRES_MAJOR}-fod-${BLOCK_SIZE_KB}k' "${PUBLISH}" >/dev/null
grep -F 'DEFAULT_PGAUDIT_VERSION="17.1"' "${PUBLISH}" >/dev/null
grep -F 'DEFAULT_PGAUDIT_VERSION="18.0"' "${PUBLISH}" >/dev/null
grep -F 'DEFAULT_PG_HINT_PLAN_TAG="REL17_1_7_1"' "${PUBLISH}" >/dev/null
grep -F 'DEFAULT_PG_HINT_PLAN_TAG="REL18_1_8_0"' "${PUBLISH}" >/dev/null
grep -F 'FOD_CONTAINER_TAG_LATEST:-0' "${PUBLISH}" >/dev/null
grep -F 'MAJOR_TAG="${IMAGE_BASE}:${POSTGRES_MAJOR}"' "${PUBLISH}" >/dev/null
grep -F 'ARG PG_HINT_PLAN_TAG=REL16_1_6_2' "${DOCKERFILE}" >/dev/null
grep -F 'refs/tags/${PG_HINT_PLAN_TAG}.tar.gz' "${DOCKERFILE}" >/dev/null

for compose in "${COMPOSE_DISK}" "${COMPOSE_TMPFS}"; do
    grep -F 'POSTGRES_INITDB_ARGS: "--auth-local=trust --auth-host=scram-sha-256"' "${compose}" >/dev/null
    if grep -Eq -- 'POSTGRES_INITDB_ARGS:.*--auth-host=trust' "${compose}"; then
        echo "Host authentication must remain SCRAM in ${compose}" >&2
        exit 1
    fi
done

grep -F -- '--no-sync --auth-local=trust --auth-host=trust' "${DOCKERFILE}" >/dev/null
grep -F 'initdb --no-sync --auth-local=trust --auth-host=trust' "${PUBLISH}" >/dev/null
grep -F 'gosu postgres' "${DOCKERFILE}" >/dev/null
grep -F 'gosu postgres' "${PUBLISH}" >/dev/null
if grep -Fq 'su-exec postgres' "${DOCKERFILE}" "${PUBLISH}"; then
    echo 'PostgreSQL Alpine images must use gosu, not su-exec' >&2
    exit 1
fi

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${PUBLISH_8}" "${PUBLISH}"; then
    echo 'PostgreSQL publishers must not perform global Docker pruning' >&2
    exit 1
fi

echo 'OK: normalized PostgreSQL Docker Make targets, 8K/32K publishers and explicit initdb auth policy'
