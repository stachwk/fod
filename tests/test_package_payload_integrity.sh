#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -u -o pipefail

repo_root="${FOD_TEST_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null)}"
[[ -n "${repo_root}" && -f "${repo_root}/Cargo.toml" ]] || {
    echo "cannot resolve FOD repository root" >&2
    exit 2
}

for tool in make sha256sum cmp; do
    command -v "${tool}" >/dev/null 2>&1 || {
        echo "missing required tool: ${tool}" >&2
        exit 2
    }
done

gate_root="${repo_root}/target/reproducibility/package-payload"
build_root="${gate_root}/build"
package_root="${gate_root}/packages"
extract_root="${gate_root}/extract"

rm -rf -- "${gate_root}"
mkdir -p -- "${build_root}" "${package_root}" "${extract_root}"

plan="$(
    cd "${repo_root}" &&
    FOD_PACKAGE_ROOT="${package_root}" \
    bash scripts/fod-native-package.sh plan native
)" || exit $?

format="$(printf '%s\n' "${plan}" | awk -F= '$1=="resolved_format" {print $2}')"
case "${format}" in
    deb|rpm) ;;
    *)
        echo "unsupported native package format: ${format:-unknown}" >&2
        exit 2
        ;;
esac

echo "=== package payload integrity: build format=${format} ==="
(
    cd "${repo_root}" || exit 1
    CARGO_TARGET_DIR="${build_root}" \
    FOD_CARGO_PROFILE=release-lto \
    FOD_PACKAGE_ROOT="${package_root}" \
    make --no-print-directory \
        -f "${repo_root}/make/fod-internal-entry.mk" \
        package-native
) || exit $?

if [[ "${format}" == "deb" ]]; then
    command -v dpkg-deb >/dev/null 2>&1 || {
        echo "dpkg-deb is required for package payload gate" >&2
        exit 2
    }
    package_file="$(find "${package_root}/deb" -maxdepth 1 -type f -name '*.deb' -print | sort | tail -n1)"
    [[ -n "${package_file}" ]] || {
        echo "no DEB package produced" >&2
        exit 1
    }
    dpkg-deb -x "${package_file}" "${extract_root}" || exit $?
else
    for tool in rpm2cpio cpio; do
        command -v "${tool}" >/dev/null 2>&1 || {
            echo "${tool} is required for RPM payload gate" >&2
            exit 2
        }
    done
    package_file="$(find "${package_root}/rpm" -type f -name '*.rpm' ! -name '*.src.rpm' -print | sort | tail -n1)"
    [[ -n "${package_file}" ]] || {
        echo "no RPM package produced" >&2
        exit 1
    }
    (
        cd "${extract_root}" || exit 1
        rpm2cpio "${package_file}" | cpio -idm --quiet
    ) || exit $?
fi

source_paths=(
    "${build_root}/release-lto/fod-bootstrap"
    "${build_root}/release-lto/fod-rust-mkfs"
    "${build_root}/release-lto/fod-change"
    "${build_root}/release-lto/fod-indexer"
    "${build_root}/release-lto/fod-monitor"
    "${build_root}/release-lto/fod-rust-fuse"
)
payload_paths=(
    "${extract_root}/usr/bin/fod-bootstrap"
    "${extract_root}/usr/sbin/mkfs.fod"
    "${extract_root}/usr/bin/fod-change"
    "${extract_root}/usr/bin/fod-indexer"
    "${extract_root}/usr/bin/fod-monitor"
    "${extract_root}/usr/bin/fod-rust-fuse"
)

failed=0
checked=0

for index in "${!source_paths[@]}"; do
    source_file="${source_paths[$index]}"
    payload_file="${payload_paths[$index]}"
    label="$(basename "${source_file}")"

    if [[ ! -f "${source_file}" || ! -f "${payload_file}" ]]; then
        echo "missing payload comparison input: ${label}" >&2
        failed=1
        continue
    fi

    sha_source="$(sha256sum "${source_file}" | awk '{print $1}')"
    sha_payload="$(sha256sum "${payload_file}" | awk '{print $1}')"
    printf 'PACKAGE_PAYLOAD artifact=%s source_sha=%s payload_sha=%s\n' \
        "${label}" "${sha_source}" "${sha_payload}"

    if ! cmp -s "${source_file}" "${payload_file}"; then
        echo "package changed ELF bytes: ${label}" >&2
        failed=1
    fi
    checked=$((checked + 1))
done

source_lib="${build_root}/release-lto/libfod.so"
payload_lib="$(find "${extract_root}/usr" -type f -name 'libfod.so' -print | sort | head -n1)"
if [[ ! -f "${source_lib}" || -z "${payload_lib}" || ! -f "${payload_lib}" ]]; then
    echo "missing libfod.so payload comparison input" >&2
    failed=1
else
    sha_source="$(sha256sum "${source_lib}" | awk '{print $1}')"
    sha_payload="$(sha256sum "${payload_lib}" | awk '{print $1}')"
    printf 'PACKAGE_PAYLOAD artifact=libfod.so source_sha=%s payload_sha=%s\n' \
        "${sha_source}" "${sha_payload}"
    if ! cmp -s "${source_lib}" "${payload_lib}"; then
        echo "package changed ELF bytes: libfod.so" >&2
        failed=1
    fi
    checked=$((checked + 1))
fi

if [[ ${failed} -ne 0 ]]; then
    echo "FAIL package-payload-integrity format=${format} checked=${checked}" >&2
    exit 1
fi

echo "OK package-payload-integrity format=${format} checked=${checked} exact_elf_bytes=1"
