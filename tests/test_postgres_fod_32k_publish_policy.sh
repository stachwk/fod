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
grep -F 'FOD_POSTGRES_MIN_EXTENSION_COUNT:-45' "${PUBLISH}" >/dev/null
grep -F 'FOD_POSTGRES_MIN_SYSTEM_LOCALE_COUNT:-100' "${PUBLISH}" >/dev/null
grep -F 'verified_system_locale_count=' "${PUBLISH}" >/dev/null
grep -F 'verified_icu_locales=' "${PUBLISH}" >/dev/null
grep -F 'pl_PL' "${PUBLISH}" >/dev/null
grep -F 'pl-PL' "${PUBLISH}" >/dev/null
grep -F 'zh_CN' "${PUBLISH}" >/dev/null
grep -F 'zh-CN' "${PUBLISH}" >/dev/null
grep -F 'ar_SA' "${PUBLISH}" >/dev/null
grep -F 'ar-SA' "${PUBLISH}" >/dev/null
grep -F 'docker push "${VERSION_TAG}"' "${PUBLISH}" >/dev/null

grep -F 'ARG POSTGRES_BASE_IMAGE=postgres:16.15-alpine' "${DOCKERFILE}" >/dev/null
grep -F 'pkgconf' "${DOCKERFILE}" >/dev/null
grep -F 'icu-dev' "${DOCKERFILE}" >/dev/null
grep -F -- '--with-icu' "${DOCKERFILE}" >/dev/null
grep -F 'musl-locales' "${DOCKERFILE}" >/dev/null
grep -F 'musl-locales-lang' "${DOCKERFILE}" >/dev/null
grep -F 'icu-data-full' "${DOCKERFILE}" >/dev/null
grep -F 'MUSL_LOCPATH="/usr/share/i18n/locales/musl"' "${DOCKERFILE}" >/dev/null
grep -F 'LANG="C.UTF-8"' "${DOCKERFILE}" >/dev/null
grep -F 'C.UTF-8 en_US en_GB de_DE' "${DOCKERFILE}" >/dev/null
grep -F 'pl_PL hu_HU uk_UA ja_JP ko_KR zh_CN ar_SA' "${DOCKERFILE}" >/dev/null
grep -F -- '--locale-provider=icu --icu-locale=pl-PL' "${DOCKERFILE}" >/dev/null
if grep -Fq 'ENV LC_ALL=' "${DOCKERFILE}"; then
    echo "LC_ALL must not be pinned because callers need to override LANG/LC_*" >&2
    exit 1
fi
if grep -Eq 'en_US\.UTF-8|de_DE\.UTF-8|cs_CZ\.UTF-8|pl_PL\.UTF-8' "${DOCKERFILE}" "${PUBLISH}"; then
    echo "musl locale names must be validated without a .UTF-8 suffix" >&2
    exit 1
fi
grep -F 'make -C contrib install' "${DOCKERFILE}" >/dev/null
grep -F 'PGVECTOR_VERSION=0.8.6' "${DOCKERFILE}" >/dev/null
grep -F 'PGAUDIT_VERSION=16.1' "${DOCKERFILE}" >/dev/null
grep -F 'PG_CRON_VERSION=1.6.7' "${DOCKERFILE}" >/dev/null
grep -F 'PG_REPACK_VERSION=1.5.3' "${DOCKERFILE}" >/dev/null
grep -F 'HYPOPG_VERSION=1.4.3' "${DOCKERFILE}" >/dev/null
grep -F 'PG_STAT_KCACHE_VERSION=2.3.2' "${DOCKERFILE}" >/dev/null
grep -F 'PG_HINT_PLAN_VERSION=1.6.2' "${DOCKERFILE}" >/dev/null
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
