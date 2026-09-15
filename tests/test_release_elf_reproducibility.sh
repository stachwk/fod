#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -u -o pipefail

repo_root="${FOD_TEST_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null)}"
[[ -n "${repo_root}" && -f "${repo_root}/Cargo.toml" ]] || {
    echo "cannot resolve FOD repository root" >&2
    exit 2
}

for tool in make sha256sum cmp readelf rustc cargo; do
    command -v "${tool}" >/dev/null 2>&1 || {
        echo "missing required tool: ${tool}" >&2
        exit 2
    }
done

gate_root="${repo_root}/target/reproducibility/elf"
target_a="${gate_root}/build-a"
target_b="${gate_root}/build-b"

rm -rf -- "${gate_root}"
mkdir -p -- "${gate_root}"

build_once() {
    local target_dir="$1"
    (
        cd "${repo_root}" || exit 1
        CARGO_TARGET_DIR="${target_dir}" \
        FOD_CARGO_PROFILE=release-lto \
        make --no-print-directory \
            -f "${repo_root}/make/fod-internal-entry.mk" \
            package-artifacts
    )
}

echo "=== release ELF reproducibility: build A ==="
build_once "${target_a}" || exit $?

echo "=== release ELF reproducibility: build B ==="
build_once "${target_b}" || exit $?

artifacts=(
    fod-bootstrap
    fod-rust-mkfs
    fod-config
    fod-change
    fod-indexer
    fod-monitor
    fod-rust-fuse
    libfod.so
)

failed=0
checked=0
for artifact in "${artifacts[@]}"; do
    a="${target_a}/release-lto/${artifact}"
    b="${target_b}/release-lto/${artifact}"

    if [[ ! -f "${a}" || ! -f "${b}" ]]; then
        echo "missing release artifact: ${artifact}" >&2
        failed=1
        continue
    fi

    readelf -h "${a}" >/dev/null 2>&1 || {
        echo "artifact A is not readable ELF: ${a}" >&2
        failed=1
        continue
    }
    readelf -h "${b}" >/dev/null 2>&1 || {
        echo "artifact B is not readable ELF: ${b}" >&2
        failed=1
        continue
    }

    sha_a="$(sha256sum "${a}" | awk '{print $1}')"
    sha_b="$(sha256sum "${b}" | awk '{print $1}')"
    printf 'ELF_REPRO artifact=%s sha_a=%s sha_b=%s\n' \
        "${artifact}" "${sha_a}" "${sha_b}"

    if ! cmp -s "${a}" "${b}"; then
        echo "ELF reproducibility mismatch: ${artifact}" >&2
        echo "--- build A notes ---" >&2
        readelf -n "${a}" >&2 || true
        echo "--- build B notes ---" >&2
        readelf -n "${b}" >&2 || true
        failed=1
    fi
    checked=$((checked + 1))
done

if [[ ${failed} -ne 0 ]]; then
    echo "FAIL release-elf-reproducibility checked=${checked}" >&2
    exit 1
fi

echo "OK release-elf-reproducibility checked=${checked} profile=release-lto isolated_targets=2"
