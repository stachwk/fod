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
create_target="${MOUNTPOINT}/payload-created-through-link-${suffix}"
create_link="${MOUNTPOINT}/payload-create-link-${suffix}"
loop_a="${MOUNTPOINT}/symlink-loop-a-${suffix}"
loop_b="${MOUNTPOINT}/symlink-loop-b-${suffix}"
payload_value="symlink smoke payload"

printf '%s\n' "${payload_value}" >"${payload}"
ln -s "${payload}" "${link_path}"

[[ -L "${link_path}" ]]
fod_assert_eq "$(readlink "${link_path}")" "${payload}" "symlink target mismatch"
fod_assert_eq "$(cat "${link_path}")" "${payload_value}" "symlink payload mismatch"
python3 - "${link_path}" "${payload}" <<'PY'
import errno
import os
import stat
import sys
import time

link, target = sys.argv[1:]
link_stat = os.lstat(link)
target_stat = os.stat(target)
followed_stat = os.stat(link)
assert stat.S_ISLNK(link_stat.st_mode)
assert followed_stat.st_ino == target_stat.st_ino
assert link_stat.st_ino != target_stat.st_ino

link_mtime_before = link_stat.st_mtime_ns
os.chmod(link, 0o640)
assert stat.S_IMODE(os.stat(target).st_mode) == 0o640
target_atime_ns = (time.time_ns() - 21_000_000_000) // 1000 * 1000
target_mtime_ns = (time.time_ns() - 20_000_000_000) // 1000 * 1000
os.utime(link, ns=(target_atime_ns, target_mtime_ns))
updated_target_stat = os.stat(target)
assert updated_target_stat.st_atime_ns == target_atime_ns, (updated_target_stat.st_atime_ns, target_atime_ns)
assert updated_target_stat.st_mtime_ns == target_mtime_ns, (updated_target_stat.st_mtime_ns, target_mtime_ns)
assert os.lstat(link).st_mtime_ns == link_mtime_before

fd = os.open(link, os.O_WRONLY | os.O_TRUNC)
try:
    assert os.fstat(fd).st_ino == target_stat.st_ino
    os.write(fd, b"symlink smoke payload\n")
finally:
    os.close(fd)
with open(target, "rb") as target_file:
    assert target_file.read() == b"symlink smoke payload\n"
assert stat.S_ISLNK(os.lstat(link).st_mode)

try:
    os.readlink(target)
except OSError as error:
    assert error.errno == errno.EINVAL, error.errno
else:
    raise AssertionError("readlink on a regular file unexpectedly succeeded")
PY

python3 - "${link_path}" <<'PY'
import errno
import os
import sys

existing_link = os.fsencode(sys.argv[1])
parent = os.path.dirname(existing_link)
valid_target = parent + b"/target-\xef\xbf\xbd"
invalid_target = b"target-\xff"
invalid_link = existing_link + b"-invalid-target"

with open(valid_target, "wb") as target_file:
    target_file.write(b"replacement-character target")

try:
    os.symlink(invalid_target, invalid_link)
except OSError as exc:
    assert exc.errno == errno.EILSEQ, exc
else:
    raise AssertionError("symlink with invalid UTF-8 target unexpectedly succeeded")

assert not os.path.lexists(invalid_link)
assert os.path.exists(valid_target)
with open(valid_target, "rb") as target_file:
    assert target_file.read() == b"replacement-character target"
PY

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
ln -s "${create_target}" "${create_link}"
rm -f "${payload}"
python3 - "${orphaned_link}" "${loop_a}" "${loop_b}" "${create_link}" "${create_target}" <<'PY'
import ctypes
import errno
import os
import stat
import sys

orphaned, loop_a, loop_b, create_link, create_target = sys.argv[1:]
orphaned_stat = os.lstat(orphaned)
target = os.fsencode(os.readlink(orphaned))
assert stat.S_ISLNK(orphaned_stat.st_mode)
assert orphaned_stat.st_size == len(target), (orphaned_stat.st_size, len(target))

libc = ctypes.CDLL(None, use_errno=True)
libc.readlink.argtypes = [ctypes.c_char_p, ctypes.c_void_p, ctypes.c_size_t]
libc.readlink.restype = ctypes.c_ssize_t
assert len(target) > 8
buffer = ctypes.create_string_buffer(b"\xa5" * 9, 9)
read_count = libc.readlink(os.fsencode(orphaned), buffer, 8)
assert read_count == 8, read_count
assert buffer.raw[:8] == target[:8], (buffer.raw[:8], target[:8])
assert buffer.raw[8] == 0xA5, "readlink unexpectedly NUL-terminated the buffer"

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

assert not os.path.lexists(create_target)
fd = os.open(create_link, os.O_CREAT | os.O_WRONLY, 0o600)
try:
    os.write(fd, b"created through dangling symlink\n")
finally:
    os.close(fd)
assert stat.S_ISLNK(os.lstat(create_link).st_mode)
assert os.readlink(create_link) == create_target
assert os.stat(create_link).st_ino == os.stat(create_target).st_ino
with open(create_target, "rb") as created_file:
    assert created_file.read() == b"created through dangling symlink\n"

for path in (loop_a, loop_b):
    assert stat.S_ISLNK(os.lstat(path).st_mode)
    try:
        os.stat(path)
    except OSError as error:
        assert error.errno == errno.ELOOP, error.errno
    else:
        raise AssertionError("resolving symlink loop unexpectedly succeeded")
PY

echo "OK symlink/readlink regular_path=EINVAL stat_lstat_follow=1 chmod_utimens_follow=1 open_trunc_follows=1 dangling_create_follows=1 unlink_preserves_target=1 readlink_shortbuf=1 dangling_lstat_size=1 dangling_stat_loop=1 dangling_excl=EEXIST"
