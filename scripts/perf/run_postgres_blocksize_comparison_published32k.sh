#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Run the existing FOD 32K / PostgreSQL BLCKSZ comparison while reusing the
# published PostgreSQL 32K image. The 8K control is still built locally from the
# same Dockerfile so compiler options, contrib modules and external extensions
# stay comparable. Only the expensive 32K rebuild is skipped.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

PUBLISHED_IMAGE="${FOD_PG32_PUBLISHED_IMAGE:-ghcr.io/stachwk/postgres-16-fod-32k:16.15}"
EXPECTED_VERSION="${FOD_PG32_EXPECTED_VERSION:-16.15}"
EXPECTED_BLOCK_SIZE="${FOD_PG32_EXPECTED_BLOCK_SIZE:-32768}"
MIN_EXTENSION_COUNT="${FOD_PG32_MIN_EXTENSION_COUNT:-45}"
PULL_IMAGE="${FOD_PG32_PULL_IMAGE:-1}"
LOCAL_IMAGE="fod-postgres-blocksize:16-32k"
BASE_RUNNER="${ROOT}/scripts/perf/run_postgres_blocksize_comparison.sh"

case "${PULL_IMAGE}" in
    0|1) ;;
    *) echo "FOD_PG32_PULL_IMAGE must be 0 or 1" >&2; exit 2 ;;
esac
case "${EXPECTED_BLOCK_SIZE}:${MIN_EXTENSION_COUNT}" in
    *[!0-9:]*) echo "Expected block size and extension count must be integers" >&2; exit 2 ;;
esac

for cmd in docker awk tail tr grep mktemp; do
    command -v "${cmd}" >/dev/null 2>&1 || {
        echo "Missing required command: ${cmd}" >&2
        exit 2
    }
done
[[ -x "${BASE_RUNNER}" ]] || { echo "Missing comparison runner: ${BASE_RUNNER}" >&2; exit 2; }

REAL_DOCKER="$(command -v docker)"

printf '=== PREPARE PUBLISHED POSTGRESQL 32K IMAGE ===\n'
printf 'published_image=%s\nexpected_version=%s\nexpected_block_size=%s\npull=%s\n' \
    "${PUBLISHED_IMAGE}" "${EXPECTED_VERSION}" "${EXPECTED_BLOCK_SIZE}" "${PULL_IMAGE}"

if [[ "${PULL_IMAGE}" == "1" ]]; then
    docker pull "${PUBLISHED_IMAGE}"
fi

actual_version="$(docker run --rm --entrypoint postgres "${PUBLISHED_IMAGE}" --version | awk '{print $3}')"
if [[ "${actual_version}" != "${EXPECTED_VERSION}" ]]; then
    echo "Published image PostgreSQL version mismatch expected=${EXPECTED_VERSION} actual=${actual_version}" >&2
    exit 1
fi

actual_block_size="$(docker run --rm --entrypoint /bin/sh "${PUBLISHED_IMAGE}" -ceu '
    dir="$(mktemp -d)"
    chown postgres:postgres "${dir}"
    su-exec postgres initdb --no-sync -D "${dir}" >/dev/null 2>&1
    su-exec postgres postgres -D "${dir}" -C block_size
' | tail -n 1 | tr -d '[:space:]')"
if [[ "${actual_block_size}" != "${EXPECTED_BLOCK_SIZE}" ]]; then
    echo "Published image block_size mismatch expected=${EXPECTED_BLOCK_SIZE} actual=${actual_block_size}" >&2
    exit 1
fi

extension_count="$(docker run --rm --entrypoint /bin/sh "${PUBLISHED_IMAGE}" -ceu '
    awk "NF {n++} END {print n + 0}" /opt/postgresql-custom/share/fod/available-extensions.txt
')"
if (( extension_count < MIN_EXTENSION_COUNT )); then
    echo "Published image extension bundle too small expected_at_least=${MIN_EXTENSION_COUNT} actual=${extension_count}" >&2
    exit 1
fi
for extension in pg_stat_statements pgcrypto vector pgaudit pg_cron pg_repack hypopg pg_stat_kcache pg_hint_plan; do
    if ! docker run --rm --entrypoint /bin/sh "${PUBLISHED_IMAGE}" -ceu \
        'grep -Fx "$1" /opt/postgresql-custom/share/fod/available-extensions.txt >/dev/null' sh "${extension}"; then
        echo "Published image missing required extension: ${extension}" >&2
        exit 1
    fi
done

docker tag "${PUBLISHED_IMAGE}" "${LOCAL_IMAGE}"
printf 'verified_postgres_version=%s\nverified_block_size=%s\nverified_extension_count=%s\nlocal_test_tag=%s\n' \
    "${actual_version}" "${actual_block_size}" "${extension_count}" "${LOCAL_IMAGE}"

# The base runner intentionally builds every BLCKSZ variant before measurement.
# Put a very small docker shim first in PATH so only the 32K compose build is a
# no-op. All other docker operations, including the 8K build and measured compose
# runs, are delegated unchanged to the real Docker CLI.
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/fod-pg32-published.XXXXXX")"
cleanup() {
    local rc=$?
    rm -rf "${TMP_DIR}"
    exit "${rc}"
}
trap cleanup EXIT INT TERM

cat >"${TMP_DIR}/docker" <<'SHIM'
#!/usr/bin/env bash
set -euo pipefail

REAL="${FOD_PG32_REAL_DOCKER:?FOD_PG32_REAL_DOCKER is required}"
if [[ "${POSTGRES_BLOCK_SIZE_KB:-}" == "32" && "${1:-}" == "compose" ]]; then
    for arg in "$@"; do
        if [[ "${arg}" == "build" ]]; then
            echo "Published PostgreSQL 32K image already prepared; skipping local 32K compose build."
            exit 0
        fi
    done
fi
exec "${REAL}" "$@"
SHIM
chmod 0700 "${TMP_DIR}/docker"

printf '\n=== START FOD 32K: POSTGRESQL 8K VS 32K ===\n'
printf '32k_source=published\n8k_source=local_same_dockerfile\n'

set +e
PATH="${TMP_DIR}:${PATH}" \
FOD_PG32_REAL_DOCKER="${REAL_DOCKER}" \
FOD_PG_BLOCK_COMPARISON_SIZES_KB="${FOD_PG_BLOCK_COMPARISON_SIZES_KB:-8 32}" \
FOD_PG_BLOCK_COMPARISON_FOD_BLOCK_SIZE="${FOD_PG_BLOCK_COMPARISON_FOD_BLOCK_SIZE:-32768}" \
FOD_PG_BLOCK_COMPARISON_REPEATS="${FOD_PG_BLOCK_COMPARISON_REPEATS:-3}" \
FOD_PG_BLOCK_COMPARISON_FILE_SIZE="${FOD_PG_BLOCK_COMPARISON_FILE_SIZE:-1G}" \
FOD_PG_BLOCK_COMPARISON_FIO_BLOCK_SIZE="${FOD_PG_BLOCK_COMPARISON_FIO_BLOCK_SIZE:-512k}" \
FOD_PG_BLOCK_COMPARISON_PAYLOAD_MODE="${FOD_PG_BLOCK_COMPARISON_PAYLOAD_MODE:-random}" \
FOD_CARGO_PROFILE="${FOD_CARGO_PROFILE:-profiling}" \
FOD_RUNTIME_PROFILE="${FOD_RUNTIME_PROFILE:-profiling}" \
FOD_REQUIRE_AC_POWER="${FOD_REQUIRE_AC_POWER:-1}" \
    bash "${BASE_RUNNER}"
status=$?
set -e

exit "${status}"
