#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOCKERFILE="${ROOT}/docker/fod-client/Dockerfile"
DOCKERIGNORE="${ROOT}/docker/fod-client/Dockerfile.dockerignore"
PREFLIGHT="${ROOT}/docker/fod-client/fod-container-preflight.sh"
ENTRYPOINT="${ROOT}/docker/fod-client/fod-container-entrypoint.sh"
PUBLISH="${ROOT}/scripts/publish_fod_client.sh"
MAKEFILE="${ROOT}/Makefile"
README="${ROOT}/docker/fod-client/README.md"
VERSION="$(tr -d '[:space:]' < "${ROOT}/fod_version.txt")"

for file in "${DOCKERFILE}" "${DOCKERIGNORE}" "${PREFLIGHT}" "${ENTRYPOINT}" "${PUBLISH}" "${MAKEFILE}" "${README}"; do
    [[ -r "${file}" ]] || { echo "Missing ${file}" >&2; exit 1; }
done

bash -n "${PUBLISH}"
sh -n "${PREFLIGHT}"
sh -n "${ENTRYPOINT}"

for pattern in \
    'ARG FOD_BUILDER_IMAGE=rust:1.98.0-bookworm' \
    'ARG FOD_RUNTIME_IMAGE=debian:bookworm-slim' \
    'ARG FOD_IMAGE_VARIANT=fuse1' \
    'ARG POSTGRES_CLIENT_MAJOR=16' \
    'libpq-dev' \
    'libfuse3-dev' \
    'libcap2-bin' \
    'util-linux' \
    'package_root=/src/fod/target/packages-docker' \
    'FOD_PACKAGE_ROOT="${package_root}"' \
    'find "${package_root}/deb"' \
    'package-deb-build' \
    'COPY --from=builder /tmp/fod-client.deb /tmp/fod-client.deb' \
    'COPY docker/fod-client/fod-container-preflight.sh /usr/local/bin/fod-container-preflight' \
    'COPY docker/fod-client/fod-container-entrypoint.sh /usr/local/bin/fod-container-entrypoint' \
    'postgresql-client-${POSTGRES_CLIENT_MAJOR}' \
    'command -v fod-container-preflight' \
    'command -v capsh' \
    'command -v findmnt' \
    'command -v psql' \
    'command -v pg_isready' \
    'command -v pg_dump' \
    'command -v pg_restore' \
    'command -v vacuumdb' \
    'org.fod.container.role="client"' \
    'org.fod.container.variant="${FOD_IMAGE_VARIANT}"' \
    'org.fod.postgresql.runtime="client-tools"' \
    'org.fod.postgresql.client-major="${POSTGRES_CLIENT_MAJOR}"' \
    'org.fod.fuse.required-device="/dev/fuse"' \
    'org.fod.fuse.required-capability="SYS_ADMIN"' \
    'org.fod.fuse.mount-propagation="rshared"' \
    'org.fod.apparmor.policy="host-managed"' \
    'ENTRYPOINT ["/usr/local/bin/fod-container-entrypoint"]'; do
    grep -Fq -- "${pattern}" "${DOCKERFILE}" || { echo "Missing FOD client Docker policy: ${pattern}" >&2; exit 1; }
done

for pattern in \
    '/dev/fuse is not passed into the container' \
    'CAP_SYS_ADMIN is missing' \
    'docker-default' \
    'apparmor=unconfined' \
    'custom AppArmor profile active' \
    'fod-container-preflight --runtime'; do
    grep -Fq -- "${pattern}" "${PREFLIGHT}" "${ENTRYPOINT}" || { echo "Missing FUSE/AppArmor runtime guard: ${pattern}" >&2; exit 1; }
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
    'FOD_CLIENT_IMAGE_VARIANT:-fuse1' \
    'FOD_CLIENT_IMAGE_VERSION:-${BASE_VERSION}' \
    'FOD_CLIENT_POSTGRES_MAJOR:-16' \
    'docker/fod-client/Dockerfile' \
    'FOD_IMAGE_VARIANT=${VARIANT}' \
    'fod-container-preflight --image-only' \
    'org.fod.container.variant' \
    'org.fod.fuse.required-device' \
    'org.fod.fuse.required-capability' \
    'org.fod.apparmor.policy' \
    'command -v psql' \
    'command -v pg_isready' \
    'command -v pg_dump' \
    'org.fod.container.role' \
    'org.fod.postgresql.runtime' \
    'org.fod.postgresql.client-major'; do
    grep -Fq -- "${pattern}" "${PUBLISH}" || { echo "Missing FOD client publisher policy: ${pattern}" >&2; exit 1; }
done

for pattern in \
    'FOD_FORWARD_TARGET,docker-fod-client-build,docker-fod-client-build' \
    'FOD_FORWARD_TARGET,docker-fod-client-publish,docker-fod-client-publish' \
    'test-docker-fod-client-policy:'; do
    grep -Fq -- "${pattern}" "${MAKEFILE}" || { echo "Missing normalized FOD client Docker Make target: ${pattern}" >&2; exit 1; }
done

for obsolete in \
    'fod-client-build:' \
    'fod-client-publish:' \
    'docker-fod-client-test-policy:' \
    'test-fod-client-container-policy:'; do
    if grep -Fxq -- "${obsolete}" "${MAKEFILE}"; then
        echo "Obsolete FOD client Make target must not be restored: ${obsolete}" >&2
        exit 1
    fi
done

for pattern in \
    'make docker-fod-client-build' \
    'make test-docker-fod-client-policy' \
    'make docker-fod-client-publish' \
    "ghcr.io/stachwk/fod-client:${VERSION}" \
    '3.4.1-fuse1' \
    'apparmor=unconfined' \
    'fod-container-preflight --runtime'; do
    grep -Fq -- "${pattern}" "${README}" || { echo "Missing normalized FOD client/FUSE command in README: ${pattern}" >&2; exit 1; }
done

if grep -Eq 'make[[:space:]]+(fod-client-build|fod-client-publish|docker-fod-client-test-policy|test-fod-client-container-policy)([[:space:]]|$)' "${README}"; then
    echo 'FOD client README must not document obsolete Make targets' >&2
    exit 1
fi

for pattern in '.git' 'artifacts' 'target' '.venv' '*.local.ini' '*.db.ini'; do
    grep -Fxq -- "${pattern}" "${DOCKERIGNORE}" || { echo "Missing client Docker ignore pattern: ${pattern}" >&2; exit 1; }
done

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune' "${PUBLISH}"; then
    echo 'FOD client publisher must not perform global Docker pruning' >&2
    exit 1
fi

echo "OK: FOD client exact release image=${VERSION} is FUSE/AppArmor-aware, PostgreSQL-client-only and uses normalized Make targets"
