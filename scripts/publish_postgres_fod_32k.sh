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

case "${PUSH}:${TAG_MAJOR}:${TAG_LATEST}" in
    *[!01:]*) echo "FOD_CONTAINER_PUSH, FOD_CONTAINER_TAG_MAJOR and FOD_CONTAINER_TAG_LATEST must be 0 or 1" >&2; exit 2 ;;
esac

IMAGE_BASE="${REGISTRY}/${NAMESPACE}/${REPOSITORY}"
VERSION_TAG="${IMAGE_BASE}:${POSTGRES_VERSION}"
MAJOR_TAG="${IMAGE_BASE}:16"
LATEST_TAG="${IMAGE_BASE}:latest"

for cmd in docker git awk grep tail tr; do
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
    su-exec postgres initdb -D "${dir}" >/dev/null
    su-exec postgres postgres -D "${dir}" -C block_size
' | tail -n 1 | tr -d '[:space:]')"
if [[ "${actual_block_size}" != "32768" ]]; then
    echo "PostgreSQL block_size verification failed expected=32768 actual=${actual_block_size}" >&2
    exit 1
fi

if [[ "${TAG_MAJOR}" == "1" ]]; then
    docker tag "${VERSION_TAG}" "${MAJOR_TAG}"
fi
if [[ "${TAG_LATEST}" == "1" ]]; then
    docker tag "${VERSION_TAG}" "${LATEST_TAG}"
fi

printf 'verified_postgres_version=%s\nverified_block_size=%s\n' "${actual_version}" "${actual_block_size}"
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
