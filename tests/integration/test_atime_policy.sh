#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-atime
trap fod_test_cleanup EXIT

policy="${FOD_ATIME_POLICY:-default}"
fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

file="${MOUNTPOINT}/atime-${policy}.txt"
printf '%s\n' "atime smoke" > "${file}"

case "${policy}" in
  noatime)
    touch -a -d '2 days ago' "${file}"
    before="$(fod_stat "${file}" '%X')"
    cat "${file}" >/dev/null
    after="$(fod_stat "${file}" '%X')"
    fod_assert_eq "${after}" "${before}" "atime changed under noatime"

    dir="${MOUNTPOINT}/atime-${policy}-dir"
    mkdir -p "${dir}"
    printf '%s\n' "entry" > "${dir}/entry.txt"
    touch -a -d '2 days ago' "${dir}"
    before="$(fod_stat "${dir}" '%X')"
    shopt -s nullglob
    entries=( "${dir}"/* )
    fod_assert_eq "${#entries[@]}" "1" "unexpected directory entry count under noatime"
    after="$(fod_stat "${dir}" '%X')"
    fod_assert_eq "${after}" "${before}" "directory atime changed under noatime"
    ;;
  nodiratime)
    touch -a -d '2 days ago' "${file}"
    before="$(fod_stat "${file}" '%X')"
    cat "${file}" >/dev/null
    after="$(fod_stat "${file}" '%X')"
    if (( after <= before )); then
      echo "expected file atime to advance under nodiratime: before=${before} after=${after}"
      exit 1
    fi

    dir="${MOUNTPOINT}/atime-${policy}-dir"
    mkdir -p "${dir}"
    printf '%s\n' "entry" > "${dir}/entry.txt"
    touch -a -d '2 days ago' "${dir}"
    before="$(fod_stat "${dir}" '%X')"
    shopt -s nullglob
    entries=( "${dir}"/* )
    fod_assert_eq "${#entries[@]}" "1" "unexpected directory entry count under nodiratime"
    after="$(fod_stat "${dir}" '%X')"
    fod_assert_eq "${after}" "${before}" "directory atime changed under nodiratime"
    ;;
  relatime)
    touch -a -d '2 days ago' "${file}"
    before="$(fod_stat "${file}" '%X')"
    cat "${file}" >/dev/null
    after="$(fod_stat "${file}" '%X')"
    if (( after <= before )); then
      echo "expected atime to advance under relatime: before=${before} after=${after}"
      exit 1
    fi
    ;;
  *)
    echo "unsupported ATIME policy: ${policy}"
    exit 1
    ;;
esac

echo "OK atime/${policy}"
