#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOCKERFILE="${ROOT}/docker/postgres-blocksize/Dockerfile"
COMPOSE="${ROOT}/docker-compose.postgres-blocksize.yml"
RUNNER="${ROOT}/scripts/perf/run_postgres_blocksize_comparison.sh"
PRODUCTION_COMPOSE="${ROOT}/docker-compose.replica-read.yml"

bash -n "${RUNNER}"

grep -F 'ARG POSTGRES_BLOCK_SIZE_KB=32' "${DOCKERFILE}" >/dev/null
grep -F -- '--with-blocksize="${POSTGRES_BLOCK_SIZE_KB}"' "${DOCKERFILE}" >/dev/null
grep -F 'POSTGRES_BLOCK_SIZE_KB must be one of 1,2,4,8,16,32' "${DOCKERFILE}" >/dev/null
grep -F 'postgres -D /tmp/pg-blocksize-verify -C block_size' "${DOCKERFILE}" >/dev/null
grep -F 'make -C contrib/pg_stat_statements install' "${DOCKERFILE}" >/dev/null

grep -F 'POSTGRES_BLOCK_SIZE_KB: ${POSTGRES_BLOCK_SIZE_KB:-32}' "${COMPOSE}" >/dev/null
grep -F 'FOD_EXPECTED_PG_BLOCK_SIZE_BYTES: ${FOD_EXPECTED_PG_BLOCK_SIZE_BYTES:-32768}' "${COMPOSE}" >/dev/null
grep -F "'SHOW block_size'" "${COMPOSE}" >/dev/null
grep -F 'postgres_blocksize_primary_data:' "${COMPOSE}" >/dev/null
grep -F 'postgres_blocksize_replica_data:' "${COMPOSE}" >/dev/null

grep -F 'FOD_PG_BLOCK_COMPARISON_SIZES_KB:-8 32' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_REPEATS:-3' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_FOD_BLOCK_SIZE:-32768' "${RUNNER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_PAYLOAD_MODE:-random' "${RUNNER}" >/dev/null
grep -F 'rotation=$(( (repeat - 1) % block_count ))' "${RUNNER}" >/dev/null
grep -F 'FOD_REPLICA_READ_COMPOSE="${NO_BUILD_COMPOSE}"' "${RUNNER}" >/dev/null
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
