#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-hardlink
trap fod_test_cleanup EXIT

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

suffix="$(date +%s%N)"
dir_path="${MOUNTPOINT}/hardlink_${suffix}"
source_path="${dir_path}/source.txt"
linked_path="${dir_path}/linked.txt"
renamed_path="${dir_path}/linked-renamed.txt"
payload="hardlink payload"

fod_rename() {
  perl -e 'use strict; use warnings; my ($src, $dst) = @ARGV; rename($src, $dst) or exit(($! + 0) || 1); exit(0);' \
    "$1" "$2"
}

mkdir -p "${dir_path}"
printf '%s\n' "${payload}" >"${source_path}"

source_inode="$(fod_stat "${source_path}" '%i')"
fod_assert_nonzero "${source_inode}" "source inode"

ln "${source_path}" "${linked_path}"

linked_inode="$(fod_stat "${linked_path}" '%i')"
linked_nlink="$(fod_stat "${linked_path}" '%h')"
fod_assert_eq "${linked_inode}" "${source_inode}" "hardlink inode mismatch"
fod_assert_eq "${linked_nlink}" 2 "hardlink count after link"

if [[ -z "$(ls -A "${dir_path}")" ]]; then
  echo "directory unexpectedly empty after hardlink creation"
  exit 1
fi

fod_assert_eq "$(cat "${linked_path}")" "${payload}" "hardlink payload mismatch"

# Use direct rename(2) here to match the original hardlink regression semantics.
fod_rename "${linked_path}" "${renamed_path}"
renamed_inode="$(fod_stat "${renamed_path}" '%i')"
renamed_nlink="$(fod_stat "${renamed_path}" '%h')"
fod_assert_eq "${renamed_inode}" "${source_inode}" "renamed hardlink inode mismatch"
fod_assert_eq "${renamed_nlink}" 2 "hardlink count after rename"

rm -f "${source_path}"
remaining_nlink="$(fod_stat "${renamed_path}" '%h')"
fod_assert_eq "${remaining_nlink}" 1 "hardlink count after source unlink"

if [[ -z "$(ls -A "${dir_path}")" ]]; then
  echo "directory unexpectedly empty after source unlink"
  exit 1
fi

fod_assert_eq "$(cat "${renamed_path}")" "${payload}" "renamed hardlink payload mismatch"

rm -f "${renamed_path}"
rmdir "${dir_path}"

dir_path_mv="${MOUNTPOINT}/hardlink_mv_${suffix}"
source_path_mv="${dir_path_mv}/source.txt"
linked_path_mv="${dir_path_mv}/linked.txt"
renamed_path_mv="${dir_path_mv}/linked-renamed.txt"

mkdir -p "${dir_path_mv}"
printf '%s\n' "${payload}" >"${source_path_mv}"

source_inode_mv="$(fod_stat "${source_path_mv}" '%i')"
fod_assert_nonzero "${source_inode_mv}" "source inode for mv case"

ln "${source_path_mv}" "${linked_path_mv}"

linked_inode_mv="$(fod_stat "${linked_path_mv}" '%i')"
linked_nlink_mv="$(fod_stat "${linked_path_mv}" '%h')"
fod_assert_eq "${linked_inode_mv}" "${source_inode_mv}" "mv hardlink inode mismatch"
fod_assert_eq "${linked_nlink_mv}" 2 "mv hardlink count after link"

if [[ -z "$(ls -A "${dir_path_mv}")" ]]; then
  echo "directory unexpectedly empty after mv hardlink creation"
  exit 1
fi

fod_assert_eq "$(cat "${linked_path_mv}")" "${payload}" "mv hardlink payload mismatch"

mv -T "${linked_path_mv}" "${renamed_path_mv}"
renamed_inode_mv="$(fod_stat "${renamed_path_mv}" '%i')"
renamed_nlink_mv="$(fod_stat "${renamed_path_mv}" '%h')"
fod_assert_eq "${renamed_inode_mv}" "${source_inode_mv}" "mv renamed hardlink inode mismatch"
fod_assert_eq "${renamed_nlink_mv}" 2 "mv hardlink count after rename"

if [[ ! -e "${source_path_mv}" ]]; then
  echo "source hardlink unexpectedly missing after mv"
  exit 1
fi

rm -f "${source_path_mv}"
remaining_nlink_mv="$(fod_stat "${renamed_path_mv}" '%h')"
fod_assert_eq "${remaining_nlink_mv}" 1 "mv hardlink count after source unlink"

if [[ -z "$(ls -A "${dir_path_mv}")" ]]; then
  echo "directory unexpectedly empty after mv source unlink"
  exit 1
fi

fod_assert_eq "$(cat "${renamed_path_mv}")" "${payload}" "mv renamed hardlink payload mismatch"

rm -f "${renamed_path_mv}"
rmdir "${dir_path_mv}"

echo "OK hardlink/backend"
