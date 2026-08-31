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
    'libpq-dev' \
    'libfuse3-dev' \
    'FOD_PACKAGE_ROOT=/tmp/fod-packages' \
    'package-ubuntu' \
    'COPY --from=builder /tmp/fod-client.deb /tmp/fod-client.deb' \
    'apt-get install -y --no-install-recommends ca-certificates /tmp/fod-client.deb' \
    "dpkg-query -W -f='\${db:Status-Abbrev}\\n' libpq5" \
    'org.fod.container.role="client"' \
    'org.fod.postgresql.runtime="libpq-only"'; do
    grep -Fq -- "${pattern}" "${DOCKERFILE}" || { echo "Missing FOD client Docker policy: ${pattern}" >&2; exit 1; }
done

for command in postgres initdb pg_ctl psql; do
    grep -Fq "command -v ${command}" "${DOCKERFILE}" || { echo "Missing negative server-command check: ${command}" >&2; exit 1; }
    grep -Fq "! command -v ${command}" "${PUBLISH}" || { echo "Missing publisher server-command check: ${command}" >&2; exit 1; }
done

if grep -Eq '^FROM[[:space:]]+.*postgres' "${DOCKERFILE}"; then
    echo 'FOD client runtime/build stages must not use a PostgreSQL server image' >&2
    exit 1
fi
if grep -Eq 'apt-get[[:space:]]+install[^\n]*postgresql([[:space:]-]|$)' "${DOCKERFILE}"; then
    echo 'FOD client image must not install PostgreSQL server/client packages' >&2
    exit 1
fi
if grep -Fq 'docker/replica-read' "${DOCKERFILE}"; then
    echo 'FOD client image must not copy PostgreSQL replication/server helpers' >&2
    exit 1
fi

for pattern in \
    'FOD_CLIENT_CONTAINER_REPOSITORY:-fod-client' \
    'FOD_CLIENT_IMAGE_VERSION:-$(tr -d' \
    'docker/fod-client/Dockerfile' \
    'dpkg-query -W libpq5' \
    'org.fod.container.role' \
    'org.fod.postgresql.runtime'; do
    grep -Fq -- "${pattern}" "${PUBLISH}" || { echo "Missing FOD client publisher policy: ${pattern}" >&2; exit 1; }
done

for pattern in \
    'fod-client-build:' \
    'FOD_CONTAINER_PUSH=0 bash scripts/publish_fod_client.sh' \
    'fod-client-publish:' \
    'FOD_CONTAINER_PUSH=1 bash scripts/publish_fod_client.sh' \
    'test-fod-client-container-policy:'; do
    grep -Fq -- "${pattern}" "${MAKEFILE}" || { echo "Missing FOD client make target: ${pattern}" >&2; exit 1; }
done

for pattern in '.git' 'artifacts' 'target' '.venv' '*.local.ini' '*.db.ini'; do
    grep -Fxq -- "${pattern}" "${DOCKERIGNORE}" || { echo "Missing client Docker ignore pattern: ${pattern}" >&2; exit 1; }
done

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${PUBLISH}"; then
    echo 'FOD client publisher must not perform global Docker pruning' >&2
    exit 1
fi

echo 'OK: FOD client container is libpq-only and contains no PostgreSQL server'
