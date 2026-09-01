#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

BASE_VERSION="$(tr -d '[:space:]' < fod_version.txt)"
VARIANT="${FOD_CLIENT_IMAGE_VARIANT:-fuse1}"
VERSION="${FOD_CLIENT_IMAGE_VERSION:-${BASE_VERSION}}"
POSTGRES_CLIENT_MAJOR="${FOD_CLIENT_POSTGRES_MAJOR:-16}"
REGISTRY="${FOD_CONTAINER_REGISTRY:-ghcr.io}"
NAMESPACE="${FOD_CONTAINER_NAMESPACE:-stachwk}"
REPOSITORY="${FOD_CLIENT_CONTAINER_REPOSITORY:-fod-client}"
PUSH="${FOD_CONTAINER_PUSH:-0}"
TAG_SERIES="${FOD_CLIENT_TAG_SERIES:-1}"
TAG_LATEST="${FOD_CONTAINER_TAG_LATEST:-0}"
SOURCE="${FOD_CONTAINER_SOURCE:-https://github.com/stachwk/fod}"
REVISION="$(git rev-parse HEAD)"

case "${PUSH}:${TAG_SERIES}:${TAG_LATEST}" in
    *[!01:]*) echo "FOD_CONTAINER_PUSH, FOD_CLIENT_TAG_SERIES and FOD_CONTAINER_TAG_LATEST must be 0 or 1" >&2; exit 2 ;;
esac
case "${POSTGRES_CLIENT_MAJOR}" in
    ''|*[!0-9]*) echo "FOD_CLIENT_POSTGRES_MAJOR must be an integer" >&2; exit 2 ;;
esac
case "${VARIANT}" in
    ''|*[!A-Za-z0-9_.-]*) echo "FOD_CLIENT_IMAGE_VARIANT contains invalid characters" >&2; exit 2 ;;
esac

[[ -n "${BASE_VERSION}" ]] || { echo "FOD base version must not be empty" >&2; exit 2; }
[[ -n "${VERSION}" ]] || { echo "FOD client image version must not be empty" >&2; exit 2; }
SERIES_TAG_VALUE="${BASE_VERSION%.*}"
if [[ "${SERIES_TAG_VALUE}" == "${BASE_VERSION}" ]]; then
    SERIES_TAG_VALUE="${BASE_VERSION}"
fi

IMAGE_BASE="${REGISTRY}/${NAMESPACE}/${REPOSITORY}"
VERSION_TAG="${IMAGE_BASE}:${VERSION}"
SERIES_TAG="${IMAGE_BASE}:${SERIES_TAG_VALUE}"
LATEST_TAG="${IMAGE_BASE}:latest"

for cmd in docker git grep tr; do
    command -v "${cmd}" >/dev/null 2>&1 || { echo "Missing required command: ${cmd}" >&2; exit 2; }
done

printf '=== BUILD FOD CLIENT IMAGE ===\n'
printf 'base_version=%s\nversion=%s\nvariant=%s\npostgres_client_major=%s\nregistry=%s\nrepository=%s\nrevision=%s\npush=%s\n' \
    "${BASE_VERSION}" "${VERSION}" "${VARIANT}" "${POSTGRES_CLIENT_MAJOR}" "${REGISTRY}" "${IMAGE_BASE}" "${REVISION}" "${PUSH}"

docker build \
    -f docker/fod-client/Dockerfile \
    --build-arg "FOD_IMAGE_SOURCE=${SOURCE}" \
    --build-arg "FOD_IMAGE_REVISION=${REVISION}" \
    --build-arg "FOD_IMAGE_VERSION=${VERSION}" \
    --build-arg "FOD_IMAGE_VARIANT=${VARIANT}" \
    --build-arg "POSTGRES_CLIENT_MAJOR=${POSTGRES_CLIENT_MAJOR}" \
    -t "${VERSION_TAG}" \
    .
docker run --rm --entrypoint /bin/sh "${VERSION_TAG}" -ceu '
    command -v fod-bootstrap >/dev/null
    command -v fod-rust-fuse >/dev/null
    command -v mkfs.fod >/dev/null
    command -v mount.fod >/dev/null
    command -v fod-container-preflight >/dev/null
    command -v capsh >/dev/null
    command -v findmnt >/dev/null
    fod-container-preflight --image-only >/dev/null
    dpkg-query -W libpq5 >/dev/null
    command -v psql >/dev/null
    command -v pg_isready >/dev/null
    command -v pg_dump >/dev/null
    command -v pg_restore >/dev/null
    command -v createdb >/dev/null
    command -v dropdb >/dev/null
    command -v reindexdb >/dev/null
    command -v vacuumdb >/dev/null
    ! command -v postgres >/dev/null 2>&1
    ! command -v initdb >/dev/null 2>&1
    ! command -v pg_ctl >/dev/null 2>&1
    for binary in /usr/bin/fod-bootstrap /usr/bin/fod-change /usr/bin/fod-indexer /usr/bin/fod-monitor /usr/bin/fod-rust-fuse /usr/sbin/mkfs.fod; do
        ldd "$binary" | grep -F "not found" && exit 1 || true
    done
'

psql_version="$(docker run --rm --entrypoint psql "${VERSION_TAG}" --version)"
if ! grep -Eq "^psql \(PostgreSQL\) ${POSTGRES_CLIENT_MAJOR}\\." <<<"${psql_version}"; then
    echo "Unexpected psql version expected_major=${POSTGRES_CLIENT_MAJOR} actual=${psql_version}" >&2
    exit 1
fi

role_label="$(docker inspect --format '{{ index .Config.Labels "org.fod.container.role" }}' "${VERSION_TAG}")"
variant_label="$(docker inspect --format '{{ index .Config.Labels "org.fod.container.variant" }}' "${VERSION_TAG}")"
pg_runtime_label="$(docker inspect --format '{{ index .Config.Labels "org.fod.postgresql.runtime" }}' "${VERSION_TAG}")"
pg_client_major_label="$(docker inspect --format '{{ index .Config.Labels "org.fod.postgresql.client-major" }}' "${VERSION_TAG}")"
fuse_device_label="$(docker inspect --format '{{ index .Config.Labels "org.fod.fuse.required-device" }}' "${VERSION_TAG}")"
fuse_cap_label="$(docker inspect --format '{{ index .Config.Labels "org.fod.fuse.required-capability" }}' "${VERSION_TAG}")"
apparmor_label="$(docker inspect --format '{{ index .Config.Labels "org.fod.apparmor.policy" }}' "${VERSION_TAG}")"
[[ "${role_label}" == "client" ]] || { echo "Unexpected client role label: ${role_label}" >&2; exit 1; }
[[ "${variant_label}" == "${VARIANT}" ]] || { echo "Unexpected client variant label: ${variant_label}" >&2; exit 1; }
[[ "${pg_runtime_label}" == "client-tools" ]] || { echo "Unexpected PostgreSQL runtime label: ${pg_runtime_label}" >&2; exit 1; }
[[ "${pg_client_major_label}" == "${POSTGRES_CLIENT_MAJOR}" ]] || { echo "Unexpected PostgreSQL client-major label: ${pg_client_major_label}" >&2; exit 1; }
[[ "${fuse_device_label}" == "/dev/fuse" ]] || { echo "Unexpected FUSE device label: ${fuse_device_label}" >&2; exit 1; }
[[ "${fuse_cap_label}" == "SYS_ADMIN" ]] || { echo "Unexpected FUSE capability label: ${fuse_cap_label}" >&2; exit 1; }
[[ "${apparmor_label}" == "host-managed" ]] || { echo "Unexpected AppArmor policy label: ${apparmor_label}" >&2; exit 1; }

if [[ "${TAG_SERIES}" == "1" ]]; then
    docker tag "${VERSION_TAG}" "${SERIES_TAG}"
fi
if [[ "${TAG_LATEST}" == "1" ]]; then
    docker tag "${VERSION_TAG}" "${LATEST_TAG}"
fi

printf 'verified_role=%s\nverified_variant=%s\nverified_postgresql_runtime=%s\nverified_postgresql_client_major=%s\nverified_fuse_device=%s\nverified_fuse_capability=%s\nverified_apparmor_policy=%s\nverified_psql=%s\n' \
    "${role_label}" "${variant_label}" "${pg_runtime_label}" "${pg_client_major_label}" \
    "${fuse_device_label}" "${fuse_cap_label}" "${apparmor_label}" "${psql_version}"
printf 'image=%s\n' "${VERSION_TAG}"
[[ "${TAG_SERIES}" == "1" ]] && printf 'alias=%s\n' "${SERIES_TAG}"
[[ "${TAG_LATEST}" == "1" ]] && printf 'alias=%s\n' "${LATEST_TAG}"

if [[ "${PUSH}" == "1" ]]; then
    docker push "${VERSION_TAG}"
    [[ "${TAG_SERIES}" == "1" ]] && docker push "${SERIES_TAG}"
    [[ "${TAG_LATEST}" == "1" ]] && docker push "${LATEST_TAG}"
    echo "published=1"
else
    echo "published=0"
    echo "Set FOD_CONTAINER_PUSH=1 after docker login to publish."
fi
