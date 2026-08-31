#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN_COMPOSE="${ROOT}/docker-compose.yml"
BENCH_COMPOSE="${ROOT}/docker-compose.postgres-blocksize.yml"
TMPFS_COMPOSE="${ROOT}/docker-compose.postgres-blocksize-tmpfs.yml"
DOCKERFILE="${ROOT}/docker/postgres-blocksize/Dockerfile"
STANDALONE_INIT="${ROOT}/docker/postgres-blocksize/standalone-init.sh"
MAKEFILE="${ROOT}/GNUmakefile"
DECISION="${ROOT}/docs/FOD_POSTGRES_BLCKSZ_32K_DEFAULT_DECISION.md"

for file in "${MAIN_COMPOSE}" "${BENCH_COMPOSE}" "${TMPFS_COMPOSE}" "${DOCKERFILE}" "${STANDALONE_INIT}" "${MAKEFILE}" "${DECISION}"; do
    [[ -r "${file}" ]] || { echo "Missing ${file}" >&2; exit 1; }
done

grep -F 'image: ${POSTGRES_IMAGE:-ghcr.io/stachwk/postgres-16-fod-32k:16}' "${MAIN_COMPOSE}" >/dev/null
grep -F 'FOD_EXPECTED_PG_BLOCK_SIZE_BYTES: ${FOD_EXPECTED_PG_BLOCK_SIZE_BYTES:-32768}' "${MAIN_COMPOSE}" >/dev/null
grep -F "SHOW block_size" "${MAIN_COMPOSE}" >/dev/null
grep -F 'POSTGRES_INITDB_ARGS: "--auth-local=trust --auth-host=scram-sha-256"' "${MAIN_COMPOSE}" >/dev/null
grep -F './docker/postgres-blocksize/standalone-init.sh:/docker-entrypoint-initdb.d/10-fod-replication.sh:ro' "${MAIN_COMPOSE}" >/dev/null

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

grep -F 'docker-postgres-32k-build:' "${MAKEFILE}" >/dev/null
grep -F 'docker-postgres-32k-publish:' "${MAKEFILE}" >/dev/null
grep -F 'docker-postgres-test-policy:' "${MAKEFILE}" >/dev/null

grep -F 'PostgreSQL compiled with `BLCKSZ=32K` is the default and target PostgreSQL variant.' "${DECISION}" >/dev/null
grep -F 'It is not the default or target configuration for new FOD deployments.' "${DECISION}" >/dev/null

echo 'OK: PostgreSQL BLCKSZ=32K is the guarded FOD default and target with normalized Docker Make targets'
