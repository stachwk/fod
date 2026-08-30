#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

POSTGRES_VERSION="${FOD_POSTGRES_IMAGE_VERSION:-16.15}"
REGISTRY="${FOD_CONTAINER_REGISTRY:-ghcr.io}"
NAMESPACE="${FOD_CONTAINER_NAMESPACE:-stachwk}"
REPOSITORY="${FOD_CONTAINER_REPOSITORY:-postgres-16-fod-32k}"
PUSH="${FOD_CONTAINER_PUSH:-0}"
TAG_MAJOR="${FOD_CONTAINER_TAG_MAJOR:-1}"
TAG_LATEST="${FOD_CONTAINER_TAG_LATEST:-0}"
SOURCE="${FOD_CONTAINER_SOURCE:-https://github.com/stachwk/fod}"
REVISION="$(git rev-parse HEAD)"
MIN_EXTENSION_COUNT="${FOD_POSTGRES_MIN_EXTENSION_COUNT:-45}"
MIN_SYSTEM_LOCALE_COUNT="${FOD_POSTGRES_MIN_SYSTEM_LOCALE_COUNT:-100}"
REQUIRED_EXTENSIONS=(
    pg_stat_statements
    pgcrypto
    hstore
    citext
    pg_trgm
    amcheck
    postgres_fdw
    uuid-ossp
    xml2
    vector
    pgaudit
    pg_cron
    pg_repack
    hypopg
    pg_stat_kcache
    pg_hint_plan
)
REQUIRED_SYSTEM_LOCALES=(
    C.UTF-8
    en_US
    en_GB
    de_DE
    fr_FR
    es_ES
    it_IT
    pt_BR
    ru_RU
    cs_CZ
    nl_NL
    sv_SE
    pl_PL
    hu_HU
    uk_UA
    ja_JP
    ko_KR
    zh_CN
    ar_SA
)
REQUIRED_ICU_LOCALES=(
    pl-PL
    cs-CZ
    de-DE
    en-US
    fr-FR
    es-ES
    it-IT
    pt-BR
    ru-RU
    uk-UA
    hu-HU
    ja-JP
    zh-CN
    ko-KR
    tr-TR
    ar-SA
)

case "${PUSH}:${TAG_MAJOR}:${TAG_LATEST}" in
    *[!01:]*) echo "FOD_CONTAINER_PUSH, FOD_CONTAINER_TAG_MAJOR and FOD_CONTAINER_TAG_LATEST must be 0 or 1" >&2; exit 2 ;;
esac
case "${MIN_EXTENSION_COUNT}:${MIN_SYSTEM_LOCALE_COUNT}" in
    *[!0-9:]*) echo "FOD_POSTGRES_MIN_EXTENSION_COUNT and FOD_POSTGRES_MIN_SYSTEM_LOCALE_COUNT must be integers" >&2; exit 2 ;;
esac

IMAGE_BASE="${REGISTRY}/${NAMESPACE}/${REPOSITORY}"
VERSION_TAG="${IMAGE_BASE}:${POSTGRES_VERSION}"
MAJOR_TAG="${IMAGE_BASE}:16"
LATEST_TAG="${IMAGE_BASE}:latest"

for cmd in docker git awk grep tail tr wc sort paste; do
    command -v "${cmd}" >/dev/null 2>&1 || { echo "Missing required command: ${cmd}" >&2; exit 2; }
done

printf '=== BUILD FOD POSTGRESQL 32K IMAGE ===\n'
printf 'postgres_version=%s\nregistry=%s\nrepository=%s\nrevision=%s\npush=%s\n' \
    "${POSTGRES_VERSION}" "${REGISTRY}" "${IMAGE_BASE}" "${REVISION}" "${PUSH}"

docker build \
    -f docker/postgres-blocksize/Dockerfile \
    --build-arg "POSTGRES_BASE_IMAGE=postgres:${POSTGRES_VERSION}-alpine" \
    --build-arg POSTGRES_BLOCK_SIZE_KB=32 \
    --build-arg "FOD_IMAGE_SOURCE=${SOURCE}" \
    --build-arg "FOD_IMAGE_REVISION=${REVISION}" \
    --build-arg "FOD_IMAGE_VERSION=${POSTGRES_VERSION}" \
    -t "${VERSION_TAG}" \
    .

actual_version="$(docker run --rm --entrypoint postgres "${VERSION_TAG}" --version | awk '{print $3}')"
if [[ "${actual_version}" != "${POSTGRES_VERSION}" ]]; then
    echo "PostgreSQL version verification failed expected=${POSTGRES_VERSION} actual=${actual_version}" >&2
    exit 1
fi

actual_block_size="$(docker run --rm --entrypoint /bin/sh "${VERSION_TAG}" -ceu '
    dir="$(mktemp -d)"
    chown postgres:postgres "${dir}"
    su-exec postgres initdb --no-sync -D "${dir}" >/dev/null
    su-exec postgres postgres -D "${dir}" -C block_size
' | tail -n 1 | tr -d '[:space:]')"
if [[ "${actual_block_size}" != "32768" ]]; then
    echo "PostgreSQL block_size verification failed expected=32768 actual=${actual_block_size}" >&2
    exit 1
fi

extension_list="$(docker run --rm --entrypoint /bin/sh "${VERSION_TAG}" -ceu \
    'cat /opt/postgresql-custom/share/fod/available-extensions.txt')"
extension_count="$(printf '%s\n' "${extension_list}" | awk 'NF {n++} END {print n + 0}')"
if (( extension_count < MIN_EXTENSION_COUNT )); then
    echo "Extension bundle verification failed expected_at_least=${MIN_EXTENSION_COUNT} actual=${extension_count}" >&2
    printf '%s\n' "${extension_list}" >&2
    exit 1
fi
for extension in "${REQUIRED_EXTENSIONS[@]}"; do
    if ! printf '%s\n' "${extension_list}" | grep -Fx "${extension}" >/dev/null; then
        echo "Required extension missing from image: ${extension}" >&2
        exit 1
    fi
done

system_locale_list="$(docker run --rm --entrypoint /bin/sh "${VERSION_TAG}" -ceu 'locale -a | sort -u')"
system_locale_count="$(printf '%s\n' "${system_locale_list}" | awk 'NF {n++} END {print n + 0}')"
if (( system_locale_count < MIN_SYSTEM_LOCALE_COUNT )); then
    echo "System locale verification failed expected_at_least=${MIN_SYSTEM_LOCALE_COUNT} actual=${system_locale_count}" >&2
    printf '%s\n' "${system_locale_list}" >&2
    exit 1
fi
for locale_name in "${REQUIRED_SYSTEM_LOCALES[@]}"; do
    if ! printf '%s\n' "${system_locale_list}" | grep -Fx "${locale_name}" >/dev/null; then
        echo "Required system locale missing from image: ${locale_name}" >&2
        exit 1
    fi
done

icu_locale_csv="$(IFS=,; printf '%s' "${REQUIRED_ICU_LOCALES[*]}")"
docker run --rm --entrypoint /bin/sh "${VERSION_TAG}" -ceu '
    old_ifs="$IFS"
    IFS=,
    for locale_name in $1; do
        dir="$(mktemp -d)"
        chown postgres:postgres "${dir}"
        su-exec postgres initdb --no-sync --locale-provider=icu --icu-locale="${locale_name}" -D "${dir}" >/dev/null
        rm -rf "${dir}"
    done
    IFS="$old_ifs"
' sh "${icu_locale_csv}"

if [[ "${TAG_MAJOR}" == "1" ]]; then
    docker tag "${VERSION_TAG}" "${MAJOR_TAG}"
fi
if [[ "${TAG_LATEST}" == "1" ]]; then
    docker tag "${VERSION_TAG}" "${LATEST_TAG}"
fi

printf 'verified_postgres_version=%s\nverified_block_size=%s\n' "${actual_version}" "${actual_block_size}"
printf 'verified_extension_count=%s\n' "${extension_count}"
printf 'verified_external_extensions=%s\n' 'vector,pgaudit,pg_cron,pg_repack,hypopg,pg_stat_kcache,pg_hint_plan'
printf 'verified_system_locale_count=%s\n' "${system_locale_count}"
printf 'verified_system_locales=%s\n' "$(IFS=,; printf '%s' "${REQUIRED_SYSTEM_LOCALES[*]}")"
printf 'verified_icu_locales=%s\n' "${icu_locale_csv}"
printf 'image=%s\n' "${VERSION_TAG}"
[[ "${TAG_MAJOR}" == "1" ]] && printf 'alias=%s\n' "${MAJOR_TAG}"
[[ "${TAG_LATEST}" == "1" ]] && printf 'alias=%s\n' "${LATEST_TAG}"

if [[ "${PUSH}" == "1" ]]; then
    docker push "${VERSION_TAG}"
    [[ "${TAG_MAJOR}" == "1" ]] && docker push "${MAJOR_TAG}"
    [[ "${TAG_LATEST}" == "1" ]] && docker push "${LATEST_TAG}"
    echo "published=1"
else
    echo "published=0"
    echo "Set FOD_CONTAINER_PUSH=1 after docker login to publish."
fi
