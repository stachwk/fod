#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOCKERFILE="${ROOT}/docker/postgres-blocksize/Dockerfile"
COMPOSE="${ROOT}/docker-compose.postgres-blocksize.yml"
RUNNER="${ROOT}/scripts/perf/run_postgres_blocksize_comparison.sh"
REPLICA_ENTRYPOINT="${ROOT}/docker/replica-read/replica-entrypoint.sh"
PRODUCTION_COMPOSE="${ROOT}/docker-compose.replica-read.yml"

bash -n "${RUNNER}"

grep -F 'ARG POSTGRES_BLOCK_SIZE_KB=32' "${DOCKERFILE}" >/dev/null
grep -F -- '--with-blocksize="${POSTGRES_BLOCK_SIZE_KB}"' "${DOCKERFILE}" >/dev/null
grep -F 'POSTGRES_BLOCK_SIZE_KB must be one of 1,2,4,8,16,32' "${DOCKERFILE}" >/dev/null
grep -F -- '-C block_size' "${DOCKERFILE}" >/dev/null
grep -F 'make -C contrib install' "${DOCKERFILE}" >/dev/null
grep -F 'PGVECTOR_VERSION=0.8.6' "${DOCKERFILE}" >/dev/null
grep -F "listen_addresses = '*'" "${DOCKERFILE}" >/dev/null
grep -F -- '--no-sync --auth-local=trust --auth-host=trust' "${DOCKERFILE}" >/dev/null

grep -F 'POSTGRES_BLOCK_SIZE_KB: ${POSTGRES_BLOCK_SIZE_KB:-32}' "${COMPOSE}" >/dev/null
grep -F 'FOD_EXPECTED_PG_BLOCK_SIZE_BYTES: ${FOD_EXPECTED_PG_BLOCK_SIZE_BYTES:-32768}' "${COMPOSE}" >/dev/null
grep -F 'POSTGRES_INITDB_ARGS: "--auth-local=trust --auth-host=scram-sha-256"' "${COMPOSE}" >/dev/null
grep -F "'SHOW block_size'" "${COMPOSE}" >/dev/null
grep -F 'listen_addresses=*' "${COMPOSE}" >/dev/null
grep -F './docker/replica-read/replica-entrypoint.sh:/usr/local/bin/fod-replica-entrypoint.sh:ro' "${COMPOSE}" >/dev/null
grep -F 'postgres_blocksize_primary_data:' "${COMPOSE}" >/dev/null
grep -F 'postgres_blocksize_replica_data:' "${COMPOSE}" >/dev/null
grep -F -- "-c listen_addresses='*'" "${REPLICA_ENTRYPOINT}" >/dev/null
if grep -Eq -- 'POSTGRES_INITDB_ARGS:.*--auth-host=trust' "${COMPOSE}"; then
    echo 'Benchmark container host authentication must not use trust' >&2
    exit 1
fi

grep -F 'FOD_PG_BLOCK_COMPARISON_SIZES_KB:-8 32' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_REPEATS:-3' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_FOD_BLOCK_SIZE:-32768' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_PAYLOAD_MODE:-random' "${RUNNER}" >/dev/null
grep -F 'rotation=$(( (repeat - 1) % block_count ))' "${RUNNER}" >/dev/null
grep -F 'FOD_REPLICA_READ_COMPOSE="${NO_BUILD_COMPOSE}"' "${RUNNER}" >/dev/null
grep -F 'initdb --no-sync --auth-local=trust --auth-host=trust' "${RUNNER}" >/dev/null
grep -F 'verified_block_size_bytes' "${RUNNER}" >/dev/null
grep -F 'median_primary_write_mib_s' "${RUNNER}" >/dev/null
grep -F 'postgres_32k_vs_8k_primary_write_pct=' "${RUNNER}" >/dev/null

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${RUNNER}"; then
    echo "PostgreSQL block-size benchmark must not perform global Docker pruning" >&2
    exit 1
fi

# The normal replica-read lab must remain on the standard PostgreSQL image.
grep -F 'image: fod-replica-read-postgres:16-alpine' "${PRODUCTION_COMPOSE}" >/dev/null
if grep -Fq 'postgres-blocksize' "${PRODUCTION_COMPOSE}"; then
    echo "Benchmark-only PostgreSQL BLCKSZ image leaked into normal replica-read compose" >&2
    exit 1
fi

echo "OK: PostgreSQL block-size comparison policy"
