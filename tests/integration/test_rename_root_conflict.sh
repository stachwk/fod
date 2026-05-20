#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-rename-root
trap 'cleanup_artifacts; fod_test_cleanup' EXIT

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

suffix="$$-$(date +%s%N)"
source="${MOUNTPOINT}/rename_${suffix}_a.txt"
occupied="${MOUNTPOINT}/rename_${suffix}_b.txt"
target="${MOUNTPOINT}/rename_${suffix}_c.txt"
payload="rename root"
occupied_payload="occupied root"
cross_parent_src_dir="${MOUNTPOINT}/rename_${suffix}_cross_src"
cross_parent_dst_dir="${MOUNTPOINT}/rename_${suffix}_cross_dst"
cross_parent_src="${cross_parent_src_dir}/source.txt"
cross_parent_dst="${cross_parent_dst_dir}/target.txt"
source_dir="${MOUNTPOINT}/rename_${suffix}_dir_a"
occupied_dir="${MOUNTPOINT}/rename_${suffix}_dir_b"
cycle_dir="${MOUNTPOINT}/rename_${suffix}_cycle"
cycle_child="${cycle_dir}/child"
root_conflict_source="${MOUNTPOINT}/rename_${suffix}_root_conflict.txt"
workdir="$(mktemp -d /tmp/fod-rename-root-work.XXXXXX)"

cleanup_artifacts() {
  if [[ -n "${workdir:-}" ]]; then
    rm -rf "${workdir}"
  fi
}

expect_rename_errno() {
  local expected_re="$1"
  local description="$2"
  local stdout_file="${workdir}/${description}.out"
  local stderr_file="${workdir}/${description}.err"
  local trace_file="${workdir}/${description}.trace"
  shift 2

  if strace -qq -e trace=rename,renameat,renameat2 -o "${trace_file}" \
    perl -e 'use strict; use warnings; my ($src, $dst) = @ARGV; rename($src, $dst) or exit(($! + 0) || 1); exit(0);' \
    "$@" >"${stdout_file}" 2>"${stderr_file}"; then
    echo "${description}: expected rename failure"
    cat "${stdout_file}"
    cat "${stderr_file}"
    cat "${trace_file}"
    exit 1
  fi

  if ! grep -Eq "${expected_re}" "${trace_file}"; then
    echo "${description}: unexpected rename errno"
    cat "${stdout_file}"
    cat "${stderr_file}"
    cat "${trace_file}"
    exit 1
  fi
}

fod_rename() {
  mv -T "$1" "$2"
}

printf '%s' "${payload}" > "${source}"
printf '%s' "${occupied_payload}" > "${occupied}"

fod_rename "${source}" "${target}"
fod_assert_eq "$(cat "${target}")" "${payload}" "rename root read mismatch"

fod_rename "${target}" "${occupied}"
fod_assert_eq "$(cat "${occupied}")" "${payload}" "rename replace mismatch"
if [[ -e "${source}" ]]; then
  echo "old path still exists after replace rename"
  exit 1
fi

mkdir -p "${cross_parent_src_dir}" "${cross_parent_dst_dir}"
printf '%s' "cross-parent" > "${cross_parent_src}"
fod_rename "${cross_parent_src}" "${cross_parent_dst}"
fod_assert_eq "$(cat "${cross_parent_dst}")" "cross-parent" "cross-parent rename mismatch"
if [[ -e "${cross_parent_src}" ]]; then
  echo "cross-parent old path still exists after rename"
  exit 1
fi

mkdir -p "${source_dir}" "${occupied_dir}"
fod_rename "${source_dir}" "${occupied_dir}"
if [[ ! -d "${occupied_dir}" ]]; then
  echo "directory rename did not produce directory"
  exit 1
fi
if [[ -e "${source_dir}" ]]; then
  echo "old directory path still exists after replace rename"
  exit 1
fi

mkdir -p "${cycle_child}"
expect_rename_errno '(EINVAL|EBUSY)' "rename-into-descendant" "${cycle_dir}" "${cycle_child}/inner"

printf '%s' "root conflict" > "${root_conflict_source}"
expect_rename_errno 'EXDEV' "rename-to-root" "${root_conflict_source}" "${MOUNTPOINT}"
expect_rename_errno 'EXDEV' "rename-from-root" "${MOUNTPOINT}" "${occupied}"

echo "OK rename/root-conflict"
