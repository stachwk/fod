#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-files
trap fod_test_cleanup EXIT

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

file="${MOUNTPOINT}/files.bin"
renamed="${MOUNTPOINT}/files-renamed.bin"
expected_size=0

dd if=/dev/urandom of="${file}" bs=1K count=1 status=none
expected_size=$((expected_size + 1024))
for block_k in 2 5 3 7; do
  dd if=/dev/urandom of="${file}" bs="${block_k}K" count=1 oflag=append conv=notrunc status=none
  expected_size=$((expected_size + block_k * 1024))
  actual_size="$(fod_stat "${file}" '%s')"
  fod_assert_eq "${actual_size}" "${expected_size}" "unexpected file size after append"
done

file_inode="$(fod_stat "${file}" '%i')"
file_nlink="$(fod_stat "${file}" '%h')"
file_blksize="$(fod_stat "${file}" '%o')"
fod_assert_nonzero "${file_inode}" "file inode"
fod_assert_eq "${file_nlink}" 1 "file hard links"
fod_assert_ge "${file_blksize}" 512 "file block size"

mv "${file}" "${renamed}"
fod_assert_eq "$(fod_stat "${renamed}" '%i')" "${file_inode}" "inode changed after rename"
fod_assert_eq "$(fod_stat "${renamed}" '%s')" "${expected_size}" "file size changed after rename"

rm -f "${renamed}"

echo "OK files/create/write/truncate/rename/unlink"
