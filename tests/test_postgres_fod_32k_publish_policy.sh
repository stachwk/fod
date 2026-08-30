#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PUBLISH="${ROOT}/scripts/publish_postgres_fod_32k.sh"
DOCKERFILE="${ROOT}/docker/postgres-blocksize/Dockerfile"
DOC="${ROOT}/docs/FOD_POSTGRES_32K_CONTAINER.md"
COMPOSE="${ROOT}/docker-compose.postgres-blocksize.yml"

bash -n "${PUBLISH}"

grep -F 'FOD_POSTGRES_IMAGE_VERSION:-16.15' "${PUBLISH}" >/dev/null
grep -F 'FOD_CONTAINER_REGISTRY:-ghcr.io' "${PUBLISH}" >/dev/null
grep -F 'FOD_CONTAINER_NAMESPACE:-stachwk' "${PUBLISH}" >/dev/null
grep -F 'FOD_CONTAINER_REPOSITORY:-postgres-16-fod-32k' "${PUBLISH}" >/dev/null
grep -F 'FOD_CONTAINER_PUSH:-0' "${PUBLISH}" >/dev/null
grep -F 'POSTGRES_BLOCK_SIZE_KB=32' "${PUBLISH}" >/dev/null
grep -F 'actual_block_size' "${PUBLISH}" >/dev/null
grep -F '32768' "${PUBLISH}" >/dev/null
grep -F 'docker push "${VERSION_TAG}"' "${PUBLISH}" >/dev/null

grep -F 'ARG POSTGRES_BASE_IMAGE=postgres:16.15-alpine' "${DOCKERFILE}" >/dev/null
grep -F 'pkgconf' "${DOCKERFILE}" >/dev/null
grep -F 'icu-dev' "${DOCKERFILE}" >/dev/null
grep -F -- '--with-icu' "${DOCKERFILE}" >/dev/null
grep -F 'musl-locales' "${DOCKERFILE}" >/dev/null
grep -F "locale -a | grep -F 'en_US.UTF-8'" "${DOCKERFILE}" >/dev/null
grep -F 'org.opencontainers.image.source' "${DOCKERFILE}" >/dev/null
grep -F 'org.opencontainers.image.revision' "${DOCKERFILE}" >/dev/null
grep -F 'org.opencontainers.image.version' "${DOCKERFILE}" >/dev/null

grep -F 'postgres:16.15-alpine' "${COMPOSE}" >/dev/null
grep -F 'ghcr.io/stachwk/postgres-16-fod-32k:16.15' "${DOC}" >/dev/null
grep -F 'FOD_CONTAINER_REGISTRY=docker.io' "${DOC}" >/dev/null

if grep -Eq '(CR_PAT|password|token)=[A-Za-z0-9_./+-]{12,}' "${PUBLISH}" "${DOC}"; then
    echo "Publishing files must not contain embedded registry credentials" >&2
    exit 1
fi

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${PUBLISH}"; then
    echo "Publishing script must not perform global Docker pruning" >&2
    exit 1
fi

echo "OK: PostgreSQL 32K image publishing policy"
