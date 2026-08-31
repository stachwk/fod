#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PUBLIC="${ROOT}/Makefile"
GNU="${ROOT}/GNUmakefile"
ENTRY="${ROOT}/make/fod-internal-entry.mk"
INTERNAL="${ROOT}/make/fod-internal.mk"
EXTRA="${ROOT}/make/fod-extra-internal.mk"

public_make() {
    env -u FOD_INTERNAL_MAKE make -C "${ROOT}" --no-print-directory "$@"
}

for file in "${PUBLIC}" "${GNU}" "${ENTRY}" "${INTERNAL}" "${EXTRA}"; do
    [[ -r "${file}" ]] || { echo "Missing Make interface file: ${file}" >&2; exit 1; }
done

required_targets=(
    build-fod-runtime
    build-fod-debug
    rust-toolchain-production-check
    rust-profile-show
    target-cargo-info
    target-shm-status
    postgres-up
    postgres-down
    postgres-reset
    postgres-shell
    fod-init
    fod-mount
    fod-remote-mount
    runtime-config-list
    runtime-config-get
    runtime-config-set
    runtime-config-reload
    benchmark-all
    benchmark-postgres
    benchmark-postgres-local
    benchmark-postgres-qnap
    indexer-run
    indexer-materialize
    install-root
    uninstall-root
    docker-postgres-32k-build
    docker-postgres-32k-publish
    docker-fod-client-build
    docker-fod-client-publish
    package-deb-build
    package-rpm-build
    test-docker-postgres-policy
    test-docker-fod-client-policy
)

for target in "${required_targets[@]}"; do
    grep -Fq "FOD_FORWARD_TARGET,${target}," "${PUBLIC}" || \
        grep -Eq "^${target}:" "${PUBLIC}" || {
            echo "Missing canonical public Make target: ${target}" >&2
            exit 1
        }
done

obsolete_targets=(
    up down restart logs wait wait-client reset smoke db-shell init init-qnap
    mount mount-qnap mount-user demo unmount
    qnap-up qnap-down qnap-restart qnap-logs qnap-wait qnap-init qnap-smoke qnap-reset qnap-mount
    benchmark benchmarks postgres-benchmarks postgres-benchmarks-local postgres-benchmarks-qnap
    postgres-benchmarks-checkpoint postgres-benchmarks-compare postgres-benchmarks-wal-preset postgres-benchmarks-planner-preset
    cargo-profile-show cargo-target-info cargo-target-preflight shm-target-status shm-target-clean
    build-runtime build-debug build-runtime-shm build-debug-shm
    reload-runtime change-runtime change-runtime-sync change-runtime-list change-runtime-get change-runtime-set
    config-show indexer indexer-import
    install-config install-root-scripts install-on-root uninstall-on-root install-on-root-venv
    pip-build pip-install pip-install-editable
    rust-production-toolchain-check rust-msrv-check rust-candidate-check rust-candidate-clippy rust-toolchain-benchmark
    rocky-selinux-deps remote-rocky-selinux-sync remote-rocky-selinux-prepare
    package-info package-native package-ubuntu package-deb package-rocky package-redhat package-rpm
    fod-client-build fod-client-publish postgres-publish postgres-8k-publish postgres-32k-publish postgres-all-publish
)

for target in "${obsolete_targets[@]}"; do
    if grep -Eq "^${target}:" "${PUBLIC}"; then
        echo "Obsolete public Make target is still defined: ${target}" >&2
        exit 1
    fi
done

for target in \
    test-copy-dedupe-benchmark \
    test-ext4-vs-fod-permissions \
    test-fod-indexer-materialize \
    test-postgresql-requirements \
    test-fuse-test-cleanup \
    test-fuse-test-cleanup-policy \
    test-native-package-policy; do
    grep -Fq "${target}" "${PUBLIC}" || {
        echo "Missing explicit obsolete-test filter for ${target}" >&2
        exit 1
    }
done

grep -Fq 'FOD_INTERNAL_MAKE := 1' "${ENTRY}"
grep -Fq 'override MAKE :=' "${ENTRY}"
grep -Fq 'include make/fod-internal.mk' "${ENTRY}"
grep -Fq 'include make/fod-extra-internal.mk' "${ENTRY}"
grep -Fq 'include packaging/fod-packaging.mk' "${ENTRY}"
grep -Fq 'ifeq ($(FOD_INTERNAL_MAKE),1)' "${GNU}"
grep -Fq 'include Makefile' "${GNU}"

# The implementation remains available only through the internal dispatcher.
grep -Eq '^up:' "${INTERNAL}"
grep -Eq '^init:' "${INTERNAL}"
grep -Eq '^mount:' "${INTERNAL}"

# Public parsing must succeed regardless of whether a parent internal Make
# invocation exported FOD_INTERNAL_MAKE=1.
public_make -n help >/dev/null
public_make -n postgres-up >/dev/null
public_make -n fod-init >/dev/null
public_make -n package-deb-build >/dev/null

# Removed public aliases must fail to resolve.
for target in up qnap-up init mount benchmark change-runtime pip-build package-ubuntu; do
    if public_make -n "${target}" >/dev/null 2>&1; then
        echo "Obsolete Make target still resolves publicly: ${target}" >&2
        exit 1
    fi
done

echo 'OK: public Make target namespace is normalized and alias-free'
