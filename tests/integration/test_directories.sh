#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-dirs
trap fod_test_cleanup EXIT

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

suffix="$(date +%s%N)"
dir="${MOUNTPOINT}/alpha-${suffix}"

mkdir -p "${dir}"
fod_stat "${dir}" '%n|%F|%h|%a|%u|%g' >/tmp/fod-dirs.alpha.stat
fod_ls "${MOUNTPOINT}" /tmp/fod-dirs.ls-root
fod_ls "${dir}" /tmp/fod-dirs.ls-alpha
fod_find_sorted "${MOUNTPOINT}" 1 /tmp/fod-dirs.find

root_nlink="$(fod_stat "${MOUNTPOINT}" '%h')"
dir_nlink="$(fod_stat "${dir}" '%h')"
root_blocks="$(fod_stat "${MOUNTPOINT}" '%b')"
dir_blocks="$(fod_stat "${dir}" '%b')"
fod_assert_ge "${root_nlink}" 3 "root hard links"
fod_assert_ge "${dir_nlink}" 2 "directory hard links"
fod_assert_ge "${root_blocks}" 1 "root blocks"
fod_assert_ge "${dir_blocks}" 1 "directory blocks"

if unlink "${dir}" 2>/tmp/fod-dirs.unlink.err; then
    echo "expected unlink on directory to fail"
    exit 1
fi
if ! grep -q "Is a directory\|Permission denied\|Operation not permitted" /tmp/fod-dirs.unlink.err; then
    echo "unexpected unlink error for directory"
    cat /tmp/fod-dirs.unlink.err
    exit 1
fi

rmdir "${dir}"

echo "OK directories/mkdir/rmdir/stat/ls/unlink-dir"
