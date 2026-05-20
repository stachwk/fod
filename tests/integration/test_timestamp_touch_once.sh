#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

FOD_ATIME_POLICY="${FOD_ATIME_POLICY:-relatime}"
if [[ "${FOD_ATIME_POLICY}" != "relatime" ]]; then
  echo "unsupported ATIME policy: ${FOD_ATIME_POLICY}"
  exit 1
fi
fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-touch-once
trap fod_test_cleanup EXIT

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

suffix="$$-$(date +%s%N)"
dir_path="${MOUNTPOINT}/timestamp-touch-${suffix}"
file_path="${dir_path}/payload.txt"

mkdir -p "${dir_path}"
printf '%s\n' "touch-once" > "${file_path}"

file_atime="$(fod_stat "${file_path}" '%X')"
touch -a -d "@$((file_atime - 86400))" "${file_path}"

first_read="$(head -c 4 "${file_path}")"
fod_assert_eq "${first_read}" "touc" "unexpected first read payload"
after_first="$(fod_stat "${file_path}" '%X')"

sleep 1.1

second_read="$(dd if="${file_path}" bs=1 skip=4 count=4 status=none 2>/dev/null)"
fod_assert_eq "${second_read}" "h-on" "unexpected second read payload"
after_second="$(fod_stat "${file_path}" '%X')"
fod_assert_eq "${after_second}" "${after_first}" "file atime changed on second read"

dir_atime="$(fod_stat "${dir_path}" '%X')"
touch -a -d "@$((dir_atime - 86400))" "${dir_path}"

shopt -s nullglob
first_entries=( "${dir_path}"/* )
fod_assert_eq "${#first_entries[@]}" "1" "unexpected first directory entry count"
fod_assert_eq "${first_entries[0]##*/}" "payload.txt" "unexpected first directory entry"
dir_after_first="$(fod_stat "${dir_path}" '%X')"

sleep 1.1

second_entries=( "${dir_path}"/* )
fod_assert_eq "${#second_entries[@]}" "1" "unexpected second directory entry count"
fod_assert_eq "${second_entries[0]##*/}" "payload.txt" "unexpected second directory entry"
dir_after_second="$(fod_stat "${dir_path}" '%X')"
fod_assert_eq "${dir_after_second}" "${dir_after_first}" "dir atime changed on second listing"

echo "OK timestamp-touch-once"
