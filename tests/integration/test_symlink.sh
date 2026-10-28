#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-symlink
trap fod_test_cleanup EXIT

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

suffix="$(date +%s%N)"
payload="${MOUNTPOINT}/payload-${suffix}.txt"
link_path="${MOUNTPOINT}/payload-link-${suffix}"
link_renamed="${MOUNTPOINT}/payload-link-renamed-${suffix}"
orphaned_link="${MOUNTPOINT}/payload-orphaned-${suffix}"
loop_a="${MOUNTPOINT}/symlink-loop-a-${suffix}"
loop_b="${MOUNTPOINT}/symlink-loop-b-${suffix}"
payload_value="symlink smoke payload"

printf '%s\n' "${payload_value}" >"${payload}"
ln -s "${payload}" "${link_path}"

[[ -L "${link_path}" ]]
fod_assert_eq "$(readlink "${link_path}")" "${payload}" "symlink target mismatch"
fod_assert_eq "$(cat "${link_path}")" "${payload_value}" "symlink payload mismatch"

mv "${link_path}" "${link_renamed}"
[[ -L "${link_renamed}" ]]
fod_assert_eq "$(readlink "${link_renamed}")" "${payload}" "renamed symlink target mismatch"
fod_assert_eq "$(cat "${link_renamed}")" "${payload_value}" "renamed symlink payload mismatch"

rm -f "${link_renamed}"
[[ ! -e "${link_renamed}" ]]
fod_assert_eq "$(cat "${payload}")" "${payload_value}" "unlinking symlink removed its target"
rm -f "${payload}"

printf '%s\n' "${payload_value}" >"${payload}"
ln -s "${payload}" "${orphaned_link}"
rm -f "${payload}"

[[ -L "${orphaned_link}" ]]
[[ ! -e "${orphaned_link}" ]]
fod_assert_eq "$(readlink "${orphaned_link}")" "${payload}" "orphaned symlink target mismatch"
fod_assert_eq "$(cat "${orphaned_link}" 2>/dev/null || true)" "" "orphaned symlink should not resolve to content"
ls_orphaned_output="$(ls -al "${orphaned_link}")"
grep -F "${orphaned_link} -> ${payload}" <<<"${ls_orphaned_output}" >/dev/null

ln -s "${loop_b}" "${loop_a}"
ln -s "${loop_a}" "${loop_b}"
rm -f "${payload}"
python3 - "${orphaned_link}" "${loop_a}" "${loop_b}" <<'PY'
import errno
import os
import stat
import sys

orphaned, loop_a, loop_b = sys.argv[1:]
assert stat.S_ISLNK(os.lstat(orphaned).st_mode)
for operation in (lambda: os.stat(orphaned), lambda: os.open(orphaned, os.O_RDONLY)):
    try:
        operation()
    except OSError as error:
        assert error.errno == errno.ENOENT, error.errno
    else:
        raise AssertionError("following dangling symlink unexpectedly succeeded")

try:
    os.open(orphaned, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
except OSError as error:
    assert error.errno == errno.EEXIST, error.errno
else:
    raise AssertionError("O_CREAT|O_EXCL followed dangling symlink")
assert stat.S_ISLNK(os.lstat(orphaned).st_mode)

for path in (loop_a, loop_b):
    assert stat.S_ISLNK(os.lstat(path).st_mode)
    try:
        os.stat(path)
    except OSError as error:
        assert error.errno == errno.ELOOP, error.errno
    else:
        raise AssertionError("resolving symlink loop unexpectedly succeeded")
PY

echo "OK symlink/readlink unlink_preserves_target=1 dangling_stat_lstat_loop=1 dangling_excl=EEXIST"
