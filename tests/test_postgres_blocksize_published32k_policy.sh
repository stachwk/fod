#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WRAPPER="${ROOT}/scripts/perf/run_postgres_blocksize_comparison_published32k.sh"
BASE="${ROOT}/scripts/perf/run_postgres_blocksize_comparison.sh"

bash -n "${WRAPPER}"
bash -n "${BASE}"

grep -F 'FOD_PG32_PUBLISHED_IMAGE:-ghcr.io/stachwk/postgres-16-fod-32k:16.15' "${WRAPPER}" >/dev/null
grep -F 'FOD_PG32_EXPECTED_VERSION:-16.15' "${WRAPPER}" >/dev/null
grep -F 'FOD_PG32_EXPECTED_BLOCK_SIZE:-32768' "${WRAPPER}" >/dev/null
grep -F 'FOD_PG32_MIN_EXTENSION_COUNT:-45' "${WRAPPER}" >/dev/null
grep -F 'docker pull "${PUBLISHED_IMAGE}"' "${WRAPPER}" >/dev/null
grep -F 'docker tag "${PUBLISHED_IMAGE}" "${LOCAL_IMAGE}"' "${WRAPPER}" >/dev/null
grep -F 'LOCAL_IMAGE="fod-postgres-blocksize:16-32k"' "${WRAPPER}" >/dev/null
grep -F '[[ -r "${BASE_RUNNER}" ]]' "${WRAPPER}" >/dev/null
if grep -Fq '[[ -x "${BASE_RUNNER}" ]]' "${WRAPPER}"; then
    echo "Base comparison runner is invoked via bash and must not require executable mode" >&2
    exit 1
fi
grep -F 'Published PostgreSQL 32K image already prepared; skipping local 32K compose build.' "${WRAPPER}" >/dev/null
grep -F 'POSTGRES_BLOCK_SIZE_KB:-' "${WRAPPER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_SIZES_KB:-8 32' "${WRAPPER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_FOD_BLOCK_SIZE:-32768' "${WRAPPER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_REPEATS:-3' "${WRAPPER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_FILE_SIZE:-1G' "${WRAPPER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_FIO_BLOCK_SIZE:-512k' "${WRAPPER}" >/dev/null
grep -F 'FOD_PG_BLOCK_COMPARISON_PAYLOAD_MODE:-random' "${WRAPPER}" >/dev/null

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${WRAPPER}"; then
    echo "Published-image benchmark wrapper must not perform global Docker pruning" >&2
    exit 1
fi

if grep -Fq '.github/workflows' "${WRAPPER}"; then
    echo "Published-image benchmark wrapper must not modify GitHub Actions" >&2
    exit 1
fi

echo "OK: published PostgreSQL 32K comparison wrapper policy"
