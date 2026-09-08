#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN_COMPOSE="${ROOT}/docker-compose.yml"
BENCH_COMPOSE="${ROOT}/docker-compose.postgres-blocksize.yml"
TMPFS_COMPOSE="${ROOT}/docker-compose.postgres-blocksize-tmpfs.yml"
DOCKERFILE="${ROOT}/docker/postgres-blocksize/Dockerfile"
STANDALONE_INIT="${ROOT}/docker/postgres-blocksize/standalone-init.sh"
MAKEFILE="${ROOT}/Makefile"
INTERNAL_MAKE="${ROOT}/make/fod-internal.mk"
DECISION="${ROOT}/docs/FOD_POSTGRES_BLCKSZ_32K_DEFAULT_DECISION.md"

for file in "${MAIN_COMPOSE}" "${BENCH_COMPOSE}" "${TMPFS_COMPOSE}" "${DOCKERFILE}" "${STANDALONE_INIT}" "${MAKEFILE}" "${INTERNAL_MAKE}" "${DECISION}"; do
    [[ -r "${file}" ]] || { echo "Missing ${file}" >&2; exit 1; }
done

grep -F 'image: ${POSTGRES_IMAGE:-ghcr.io/stachwk/postgres-16-fod-32k:16}' "${MAIN_COMPOSE}" >/dev/null
grep -F 'FOD_EXPECTED_PG_BLOCK_SIZE_BYTES: ${FOD_EXPECTED_PG_BLOCK_SIZE_BYTES:-32768}' "${MAIN_COMPOSE}" >/dev/null
grep -F "SHOW block_size" "${MAIN_COMPOSE}" >/dev/null
grep -F 'POSTGRES_INITDB_ARGS: "--auth-local=trust --auth-host=scram-sha-256"' "${MAIN_COMPOSE}" >/dev/null
if grep -Fq './docker/postgres-blocksize/standalone-init.sh:' "${MAIN_COMPOSE}"; then
    echo 'Default FOD compose must not bind-mount repository-local PostgreSQL init hooks; remote Docker/QNAP cannot resolve client-side paths' >&2
    exit 1
fi

if grep -Fq 'image: postgres:16-alpine' "${MAIN_COMPOSE}"; then
    echo 'Standard PostgreSQL 8K image must not be the default FOD compose image' >&2
    exit 1
fi

grep -F 'ARG POSTGRES_BLOCK_SIZE_KB=32' "${DOCKERFILE}" >/dev/null
if grep -Fq 'COPY docker/replica-read/' "${DOCKERFILE}"; then
    echo 'Published PostgreSQL image must not bake benchmark replication helpers' >&2
    exit 1
fi
if grep -Fq 'fod-replica-entrypoint.sh' "${DOCKERFILE}"; then
    echo 'Published PostgreSQL image must stay deployment-neutral' >&2
    exit 1
fi

if grep -Eq 'CREATE[[:space:]]+ROLE|fod_repl|replication[[:space:]]+fod_repl' "${STANDALONE_INIT}"; then
    echo 'Standalone init hook must not create benchmark replication state' >&2
    exit 1
fi

for compose in "${BENCH_COMPOSE}" "${TMPFS_COMPOSE}"; do
    grep -F './docker/replica-read/primary-init.sh:/docker-entrypoint-initdb.d/10-fod-replication.sh:ro' "${compose}" >/dev/null
    grep -F './docker/replica-read/replica-entrypoint.sh:/usr/local/bin/fod-replica-entrypoint.sh:ro' "${compose}" >/dev/null
done

grep -F 'FOD_FORWARD_TARGET,docker-postgres-32k-build,docker-postgres-32k-build' "${MAKEFILE}" >/dev/null
grep -F 'FOD_FORWARD_TARGET,docker-postgres-32k-publish,docker-postgres-32k-publish' "${MAKEFILE}" >/dev/null
grep -F 'test-docker-postgres-policy:' "${MAKEFILE}" >/dev/null

# QNAP PostgreSQL profile must stay explicit and reproducible after a full
# container/volume rebuild. The profile targets the current 8 GB / 2 CPU / HDD
# QNAP host while the local PostgreSQL path keeps its normal defaults.
for pattern in \
    'QNAP_POSTGRES_SHARED_BUFFERS ?= 1GB' \
    'QNAP_POSTGRES_WORK_MEM ?= 4MB' \
    'QNAP_POSTGRES_MAX_CONNECTIONS ?= 64' \
    'QNAP_POSTGRES_MAX_WAL_SIZE ?= 2GB' \
    'QNAP_POSTGRES_CHECKPOINT_TIMEOUT ?= 15min' \
    'QNAP_POSTGRES_CHECKPOINT_COMPLETION_TARGET ?= 0.9' \
    'QNAP_POSTGRES_WAL_COMPRESSION ?= off' \
    'QNAP_POSTGRES_RANDOM_PAGE_COST ?= 4' \
    'QNAP_POSTGRES_EFFECTIVE_CACHE_SIZE ?= 4GB' \
    'QNAP_POSTGRES_EFFECTIVE_IO_CONCURRENCY ?= 1' \
    'QNAP_POSTGRES_MAINTENANCE_WORK_MEM ?= 256MB' \
    'QNAP_POSTGRES_AUTOVACUUM_MAX_WORKERS ?= 2' \
    'QNAP_POSTGRES_AUTOVACUUM_WORK_MEM ?= 128MB' \
    'QNAP_POSTGRES_MAX_PARALLEL_WORKERS ?= 2' \
    'QNAP_POSTGRES_MAX_PARALLEL_WORKERS_PER_GATHER ?= 1'; do
    grep -F "${pattern}" "${INTERNAL_MAKE}" >/dev/null
done

for pattern in \
    'work_mem=${POSTGRES_WORK_MEM}' \
    'effective_io_concurrency=${POSTGRES_EFFECTIVE_IO_CONCURRENCY}' \
    'max_parallel_workers=${POSTGRES_MAX_PARALLEL_WORKERS}' \
    'max_parallel_workers_per_gather=${POSTGRES_MAX_PARALLEL_WORKERS_PER_GATHER}'; do
    grep -F "${pattern}" "${MAIN_COMPOSE}" >/dev/null
done

# Remove ambient tuning variables from this policy check. QNAP must resolve its
# own profile while the local backend must stay on defaults.
CLEAN_ENV=(
    env
    -u POSTGRES_SHARED_BUFFERS
    -u POSTGRES_WORK_MEM
    -u POSTGRES_MAX_CONNECTIONS
    -u POSTGRES_MAX_WAL_SIZE
    -u POSTGRES_CHECKPOINT_TIMEOUT
    -u POSTGRES_CHECKPOINT_COMPLETION_TARGET
    -u POSTGRES_WAL_COMPRESSION
    -u POSTGRES_RANDOM_PAGE_COST
    -u POSTGRES_EFFECTIVE_CACHE_SIZE
    -u POSTGRES_EFFECTIVE_IO_CONCURRENCY
    -u POSTGRES_MAINTENANCE_WORK_MEM
    -u POSTGRES_AUTOVACUUM_MAX_WORKERS
    -u POSTGRES_AUTOVACUUM_WORK_MEM
    -u POSTGRES_MAX_PARALLEL_WORKERS
    -u POSTGRES_MAX_PARALLEL_WORKERS_PER_GATHER
)

qnap_config="$("${CLEAN_ENV[@]}" make --no-print-directory -s -C "${ROOT}" QNAP=1 postgres-qnap-config-show)"
local_config="$("${CLEAN_ENV[@]}" make --no-print-directory -s -C "${ROOT}" QNAP=0 postgres-config-show)"

grep -F 'POSTGRES_SHARED_BUFFERS=1GB' <<<"${qnap_config}" >/dev/null
grep -F 'POSTGRES_WORK_MEM=4MB' <<<"${qnap_config}" >/dev/null
grep -F 'POSTGRES_RANDOM_PAGE_COST=4' <<<"${qnap_config}" >/dev/null
grep -F 'POSTGRES_EFFECTIVE_IO_CONCURRENCY=1' <<<"${qnap_config}" >/dev/null
grep -F 'POSTGRES_MAX_PARALLEL_WORKERS=2' <<<"${qnap_config}" >/dev/null
grep -F 'POSTGRES_MAX_PARALLEL_WORKERS_PER_GATHER=1' <<<"${qnap_config}" >/dev/null

grep -F 'POSTGRES_SHARED_BUFFERS=<default>' <<<"${local_config}" >/dev/null
grep -F 'POSTGRES_WORK_MEM=<default>' <<<"${local_config}" >/dev/null
grep -F 'POSTGRES_EFFECTIVE_IO_CONCURRENCY=<default>' <<<"${local_config}" >/dev/null
grep -F 'POSTGRES_MAX_PARALLEL_WORKERS=<default>' <<<"${local_config}" >/dev/null

# Explicit command-line tuning must still override the fixed QNAP default.
qnap_override="$("${CLEAN_ENV[@]}" make --no-print-directory -s -C "${ROOT}" \
    QNAP=1 POSTGRES_SHARED_BUFFERS=2GB postgres-qnap-config-show)"
grep -F 'POSTGRES_SHARED_BUFFERS=2GB' <<<"${qnap_override}" >/dev/null

grep -F 'PostgreSQL compiled with `BLCKSZ=32K` is the default and target PostgreSQL variant.' "${DECISION}" >/dev/null
grep -F 'It is not the default or target configuration for new FOD deployments.' "${DECISION}" >/dev/null

echo 'OK: PostgreSQL BLCKSZ=32K is the guarded FOD default and target with normalized Docker Make targets'
