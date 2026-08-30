#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Remove Docker resources created by the isolated FOD primary/replica
# performance benchmark without pruning unrelated Docker data.

set -euo pipefail

PROJECT_PREFIX="${FOD_DOCKER_PERF_PROJECT_PREFIX:-fod-replica-read-}"
IMAGE_NAME="${FOD_DOCKER_PERF_IMAGE:-fod-replica-read-postgres:16-alpine}"
FORCE="${FOD_DOCKER_PERF_CLEAN_FORCE:-0}"
PRUNE_BUILD_CACHE="${FOD_DOCKER_PERF_PRUNE_BUILD_CACHE:-0}"

command -v docker >/dev/null 2>&1 || {
    echo "Missing required command: docker" >&2
    exit 2
}

mapfile -t RUNNING_CONTAINERS < <(
    docker ps --format '{{.Names}}' \
        | awk -v prefix="${PROJECT_PREFIX}" 'index($0, prefix) == 1 { print }'
)

if ((${#RUNNING_CONTAINERS[@]} > 0)) && [[ "${FORCE}" != "1" ]]; then
    echo "Refusing Docker perf cleanup: active benchmark containers exist:" >&2
    printf '  %s\n' "${RUNNING_CONTAINERS[@]}" >&2
    echo "Re-run with FOD_DOCKER_PERF_CLEAN_FORCE=1 only if these benchmark containers may be stopped." >&2
    exit 2
fi

echo "=== Docker disk usage before FOD perf cleanup ==="
docker system df

mapfile -t CONTAINER_IDS < <(
    docker ps -a --format '{{.ID}} {{.Names}}' \
        | awk -v prefix="${PROJECT_PREFIX}" 'index($2, prefix) == 1 { print $1 }'
)
if ((${#CONTAINER_IDS[@]} > 0)); then
    echo "Removing FOD perf containers: ${#CONTAINER_IDS[@]}"
    docker rm -f "${CONTAINER_IDS[@]}"
else
    echo "FOD perf containers: none"
fi

mapfile -t VOLUMES < <(
    docker volume ls --format '{{.Name}}' \
        | awk -v prefix="${PROJECT_PREFIX}" 'index($0, prefix) == 1 { print }'
)
if ((${#VOLUMES[@]} > 0)); then
    echo "Removing FOD perf volumes: ${#VOLUMES[@]}"
    docker volume rm "${VOLUMES[@]}"
else
    echo "FOD perf volumes: none"
fi

mapfile -t NETWORK_IDS < <(
    docker network ls --format '{{.ID}} {{.Name}}' \
        | awk -v prefix="${PROJECT_PREFIX}" 'index($2, prefix) == 1 { print $1 }'
)
if ((${#NETWORK_IDS[@]} > 0)); then
    echo "Removing FOD perf networks: ${#NETWORK_IDS[@]}"
    docker network rm "${NETWORK_IDS[@]}"
else
    echo "FOD perf networks: none"
fi

if docker image inspect "${IMAGE_NAME}" >/dev/null 2>&1; then
    echo "Removing FOD perf image: ${IMAGE_NAME}"
    docker image rm -f "${IMAGE_NAME}"
else
    echo "FOD perf image: not present (${IMAGE_NAME})"
fi

if [[ "${PRUNE_BUILD_CACHE}" == "1" ]]; then
    echo "Pruning all unused Docker builder cache (explicit opt-in)."
    docker builder prune -af
else
    echo "Docker builder cache not pruned because it is shared across projects."
    echo "To prune all unused builder cache too: make docker-perf-clean FOD_DOCKER_PERF_PRUNE_BUILD_CACHE=1"
fi

echo "=== Docker disk usage after FOD perf cleanup ==="
docker system df
