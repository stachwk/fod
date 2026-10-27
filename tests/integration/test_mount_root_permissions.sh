#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-mount-root-permissions
trap fod_test_cleanup EXIT

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

root_stat="$(stat -c '%F|%a|%u|%g' "${MOUNTPOINT}")"
current_uid="$(id -u)"
current_gid="$(id -g)"
root_times() {
  python3 -c 'import os, sys; st = os.stat(sys.argv[1]); print(st.st_mtime_ns, st.st_ctime_ns)' "${MOUNTPOINT}"
}
read -r root_mtime_before root_ctime_before <<<"$(root_times)"

fod_assert_contains_text "${root_stat}" "directory"

dir="${MOUNTPOINT}/root-perms"
mkdir "${dir}"
read -r root_mtime_after_mkdir root_ctime_after_mkdir <<<"$(root_times)"
if (( root_mtime_after_mkdir <= root_mtime_before || root_ctime_after_mkdir <= root_ctime_before )); then
  echo "Expected creating a root child to advance root mtime and ctime" >&2
  exit 1
fi
chmod 750 "${dir}"
chown "${current_uid}:${current_gid}" "${dir}"

dir_stat="$(stat -c '%n|%F|%a|%u|%g|%s' "${dir}")"
fod_assert_contains_text "${dir_stat}" "${dir}|directory|750|${current_uid}|${current_gid}|0"

test -r "${dir}"
test -x "${dir}"
test -w "${dir}"

payload="${dir}/nested.txt"
printf 'root-permissions\n' >"${payload}"
test -f "${payload}"

fod_ls "${MOUNTPOINT}" /tmp/fod-mount-root-permissions.ls
fod_assert_contains /tmp/fod-mount-root-permissions.ls 'root-perms'

rm -f "${payload}"
rmdir "${dir}"
read -r root_mtime_after_rmdir root_ctime_after_rmdir <<<"$(root_times)"
if (( root_mtime_after_rmdir <= root_mtime_after_mkdir || root_ctime_after_rmdir <= root_ctime_after_mkdir )); then
  echo "Expected removing a root child to advance root mtime and ctime" >&2
  exit 1
fi

echo "OK mount/root-permissions"
