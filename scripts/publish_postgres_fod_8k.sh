#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

exec env \
    FOD_POSTGRES_BLOCK_SIZE_KB=8 \
    FOD_CONTAINER_REPOSITORY="${FOD_CONTAINER_REPOSITORY:-postgres-16-fod-8k}" \
    bash "${ROOT}/scripts/publish_postgres_fod_32k.sh" "$@"
