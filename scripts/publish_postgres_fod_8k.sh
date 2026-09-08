#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

POSTGRES_VERSION="${FOD_POSTGRES_IMAGE_VERSION:-16.15}"
case "${POSTGRES_VERSION}" in
    16.*) ;;
    *) echo "The compatibility 8K publisher supports PostgreSQL 16.x only" >&2; exit 2 ;;
esac

exec env \
    FOD_POSTGRES_IMAGE_VERSION="${POSTGRES_VERSION}" \
    FOD_POSTGRES_BLOCK_SIZE_KB=8 \
    FOD_CONTAINER_REPOSITORY="${FOD_CONTAINER_REPOSITORY:-postgres-16-fod-8k}" \
    bash "${ROOT}/scripts/publish_postgres_fod_32k.sh" "$@"
