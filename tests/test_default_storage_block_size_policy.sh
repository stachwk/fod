#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MKFS="${ROOT}/rust_mkfs/src/main.rs"
DOC="${ROOT}/docs/FOD_STORAGE_BLOCK_SIZE_SELECTION.md"

[[ -f "${MKFS}" ]]
[[ -f "${DOC}" ]]

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

# A change of the default is an init-time format policy. Do not turn it into a
# blind UPDATE of existing filesystem metadata.
if grep -Eq "UPDATE[[:space:]]+config[[:space:]]+SET[^;]*block_size" "${MKFS}"; then
    echo "Default block-size policy must not rewrite existing filesystem block_size" >&2
    exit 1
fi

echo "Default storage block size policy: OK (32768 bytes)"
