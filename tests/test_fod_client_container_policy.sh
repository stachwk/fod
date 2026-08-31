#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOCKERFILE="${ROOT}/docker/fod-client/Dockerfile"
DOCKERIGNORE="${ROOT}/docker/fod-client/Dockerfile.dockerignore"
PUBLISH="${ROOT}/scripts/publish_fod_client.sh"
MAKEFILE="${ROOT}/GNUmakefile"
README="${ROOT}/docker/fod-client/README.md"

for file in "${DOCKERFILE}" "${DOCKERIGNORE}" "${PUBLISH}" "${MAKEFILE}" "${README}"; do
    [[ -r "${file}" ]] || { echo "Missing ${file}" >&2; exit 1; }
done

bash -n "${PUBLISH}"

for pattern in \
    'ARG FOD_BUILDER_IMAGE=rust:1.98.0-bookworm' \
    'ARG FOD_RUNTIME_IMAGE=debian:bookworm-slim' \
    'ARG POSTGRES_CLIENT_MAJOR=16' \
    'libpq-dev' \
    'libfuse3-dev' \
    'package_root=/src/fod/target/packages-docker' \
    'FOD_PACKAGE_ROOT="${package_root}"' \
    'find "${package_root}/deb"' \
    'package-ubuntu' \
    'COPY --from=builder /tmp/fod-client.deb /tmp/fod-client.deb' \
    'postgresql-client-${POSTGRES_CLIENT_MAJOR}' \
    'command -v psql' \
    'command -v pg_isready' \
    'command -v pg_dump' \
    'command -v pg_restore' \
    'command -v vacuumdb' \
    'org.fod.container.role="client"' \
    'org.fod.postgresql.runtime="client-tools"' \
    'org.fod.postgresql.client-major="${POSTGRES_CLIENT_MAJOR}"'; do
    grep -Fq -- "${pattern}" "${DOCKERFILE}" || { echo "Missing FOD client Docker policy: ${pattern}" >&2; exit 1; }
done

if grep -Fq 'FOD_PACKAGE_ROOT=/tmp/' "${DOCKERFILE}"; then
    echo 'FOD client package root must remain below the repository target directory' >&2
    exit 1
fi

for command in postgres initdb pg_ctl; do
    grep -Fq "command -v ${command}" "${DOCKERFILE}" || { echo "Missing negative server-command check: ${command}" >&2; exit 1; }
    grep -Fq "! command -v ${command}" "${PUBLISH}" || { echo "Missing publisher server-command check: ${command}" >&2; exit 1; }
done

if grep -Eq '^FROM[[:space:]]+.*postgres' "${DOCKERFILE}"; then
    echo 'FOD client runtime/build stages must not use a PostgreSQL server image' >&2
    exit 1
fi
if grep -Eq '^[[:space:]]+(postgresql|postgresql-[0-9]+|postgresql-all|postgresql-contrib)([[:space:]\\]|$)' "${DOCKERFILE}"; then
    echo 'FOD client image must not install a PostgreSQL server package' >&2
    exit 1
fi
if grep -Fq 'docker/replica-read' "${DOCKERFILE}"; then
    echo 'FOD client image must not copy PostgreSQL replication/server helpers' >&2
    exit 1
fi

for pattern in \
    'FOD_CLIENT_CONTAINER_REPOSITORY:-fod-client' \
    'FOD_CLIENT_IMAGE_VERSION:-$(tr -d' \
    'FOD_CLIENT_POSTGRES_MAJOR:-16' \
    'docker/fod-client/Dockerfile' \
    'command -v psql' \
    'command -v pg_isready' \
    'command -v pg_dump' \
    'org.fod.container.role' \
    'org.fod.postgresql.runtime' \
    'org.fod.postgresql.client-major'; do
    grep -Fq -- "${pattern}" "${PUBLISH}" || { echo "Missing FOD client publisher policy: ${pattern}" >&2; exit 1; }
done

for pattern in \
    'docker-fod-client-build:' \
    'FOD_CONTAINER_PUSH=0 bash scripts/publish_fod_client.sh' \
    'docker-fod-client-publish:' \
    'FOD_CONTAINER_PUSH=1 bash scripts/publish_fod_client.sh' \
    'docker-fod-client-test-policy:'; do
    grep -Fq -- "${pattern}" "${MAKEFILE}" || { echo "Missing normalized FOD client Docker Make target: ${pattern}" >&2; exit 1; }
done

for obsolete in \
    'fod-client-build:' \
    'fod-client-publish:' \
    'test-fod-client-container-policy:'; do
    if grep -Fxq -- "${obsolete}" "${MAKEFILE}"; then
        echo "Obsolete FOD client Make target must not be restored: ${obsolete}" >&2
        exit 1
    fi
done

for pattern in '.git' 'artifacts' 'target' '.venv' '*.local.ini' '*.db.ini'; do
    grep -Fxq -- "${pattern}" "${DOCKERIGNORE}" || { echo "Missing client Docker ignore pattern: ${pattern}" >&2; exit 1; }
done

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${PUBLISH}"; then
    echo 'FOD client publisher must not perform global Docker pruning' >&2
    exit 1
fi

echo 'OK: normalized FOD client Docker Make targets, PostgreSQL client tools and no PostgreSQL server'
