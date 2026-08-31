#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT}/scripts/perf/clean_replica_read_docker.sh"
MAKEFILE="${ROOT}/Makefile"

require_line() {
    local needle="$1"
    local file="$2"
    grep -Fq -- "${needle}" "${file}" || {
        echo "Missing required cleanup policy line in ${file}: ${needle}" >&2
        exit 1
    }
}

require_line 'PROJECT_PREFIX="${FOD_DOCKER_PERF_PROJECT_PREFIX:-fod-replica-read-}"' "${SCRIPT}"
require_line 'IMAGE_NAME="${FOD_DOCKER_PERF_IMAGE:-fod-replica-read-postgres:16-alpine}"' "${SCRIPT}"
require_line 'FOD_DOCKER_PERF_CLEAN_FORCE' "${SCRIPT}"
require_line 'FOD_DOCKER_PERF_PRUNE_BUILD_CACHE' "${SCRIPT}"
require_line 'docker ps --format' "${SCRIPT}"
require_line 'docker volume ls --format' "${SCRIPT}"
require_line 'docker network ls --format' "${SCRIPT}"
require_line 'docker image rm -f "${IMAGE_NAME}"' "${SCRIPT}"
require_line 'docker builder prune -af' "${SCRIPT}"
require_line 'Docker builder cache not pruned because it is shared across projects.' "${SCRIPT}"
require_line 'FOD_FORWARD_TARGET,docker-perf-clean,docker-perf-clean' "${MAKEFILE}"

if grep -Fq 'docker system prune' "${SCRIPT}"; then
    echo "Unsafe global docker system prune is forbidden in FOD perf cleanup." >&2
    exit 1
fi

if grep -Fq 'docker volume prune' "${SCRIPT}"; then
    echo "Unsafe global docker volume prune is forbidden in FOD perf cleanup." >&2
    exit 1
fi

echo "Docker perf cleanup policy: OK"
