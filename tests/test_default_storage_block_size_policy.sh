#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MKFS="${ROOT}/rust_mkfs/src/main.rs"
DOC="${ROOT}/docs/FOD_STORAGE_BLOCK_SIZE_SELECTION.md"
CURRENT="${ROOT}/docs/CURRENT_STATE.md"
RUNTIME="${ROOT}/docs/runtime-configuration.md"
README="${ROOT}/README.md"
README_PL="${ROOT}/README.pl"
CONFIG="${ROOT}/fod_config.ini"
CONFIG_EXAMPLE="${ROOT}/fod_config.example.ini"

for path in \
    "${MKFS}" "${DOC}" "${CURRENT}" "${RUNTIME}" "${README}" "${README_PL}" \
    "${CONFIG}" "${CONFIG_EXAMPLE}"; do
    [[ -f "${path}" ]]
done

grep -Fq '#[arg(long, default_value_t = 32768)]' "${MKFS}"
if grep -Fq '#[arg(long, default_value_t = 4096)]' "${MKFS}"; then
    echo "fod-rust-mkfs must not default new filesystems to 4 KiB blocks" >&2
    exit 1
fi

grep -Fq 'cli.block_size' "${MKFS}"
grep -Fq "INSERT INTO config (key, value) VALUES ('block_size', {})" "${MKFS}"

grep -Fq 'default storage block size for **newly initialized** FOD filesystems is **32 KiB (32768 bytes)**' "${DOC}"
grep -Fq '`fod-rust-mkfs init` still accepts `--block-size` as an explicit override' "${DOC}"
grep -Fq 'This decision does **not** migrate existing filesystems.' "${DOC}"
grep -Fq '**New general-purpose filesystem:** 32 KiB default.' "${DOC}"
grep -Fq '**Known large-I/O / streaming filesystem:** consider explicit 64 KiB.' "${DOC}"

grep -Fq '32 KiB by default for newly initialized filesystems' "${README}"
grep -Fq 'existing filesystems keep the `block_size` persisted in `fod.config`' "${README}"
grep -Fq '32 KiB domyslnie dla nowo inicjalizowanych filesystemow' "${README_PL}"
grep -Fq 'istniejacy filesystem zachowuje `block_size` utrwalony w `fod.config`' "${README_PL}"
grep -Fq 'The default logical FOD storage block for **newly initialized** filesystems is 32 KiB.' "${CURRENT}"
grep -Fq 'existing filesystems keep the value with which they were initialized' "${CURRENT}"
grep -Fq 'The default storage block size for **newly initialized** FOD filesystems is' "${RUNTIME}"
grep -Fq 'The selected size is persisted per filesystem in `fod.config`' "${RUNTIME}"

for config in "${CONFIG}" "${CONFIG_EXAMPLE}"; do
    grep -Fq 'New filesystems default to 32 KiB' "${config}"
    grep -Fq 'keep the block_size persisted in fod.config' "${config}"
    grep -Fq '128-block base persist chunk' "${config}"
    grep -Fq 'is 4 MiB at 32 KiB and scales with the persisted storage block size' "${config}"
done

for stale in \
    'FOD storage block: 4 KiB' \
    'The logical FOD storage block remains 4 KiB' \
    'Storage blocks remain schema-defined (normally 4 KiB)' \
    'does not change the 4 KiB storage'; do
    if grep -Fq "${stale}" \
        "${README}" "${CURRENT}" "${RUNTIME}" "${CONFIG}" "${CONFIG_EXAMPLE}"; then
        echo "Current documentation/config still contains stale 4 KiB default wording: ${stale}" >&2
        exit 1
    fi
done

if grep -Eq "UPDATE[[:space:]]+config[[:space:]]+SET[^;]*block_size" "${MKFS}"; then
    echo "Default block-size policy must not rewrite existing filesystem block_size" >&2
    exit 1
fi

echo "Default storage block size policy: OK (32768 bytes for new filesystems; persisted per filesystem)"
