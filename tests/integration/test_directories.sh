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

not_a_directory="${dir}/rmdir-file-${suffix}"
printf 'rmdir target file\n' >"${not_a_directory}"
not_a_directory_link="${dir}/rmdir-symlink-${suffix}"
python3 - "${not_a_directory}" "${dir}" "${not_a_directory_link}" "${MOUNTPOINT}" <<'PY'
import errno
import os
import sys

file_path, nonempty_dir, symlink_path, mountpoint = sys.argv[1:]
os.symlink(nonempty_dir, symlink_path)
loop_a = os.path.join(nonempty_dir, "loop-a")
loop_b = os.path.join(nonempty_dir, "loop-b")
os.symlink("loop-b", loop_a)
os.symlink("loop-a", loop_b)
missing_parent = os.path.join(nonempty_dir, "missing-parent")
name_max = os.pathconf(nonempty_dir, "PC_NAME_MAX")
long_component = "n" * (name_max + 1)
for path, expected, label in (
    (nonempty_dir, errno.EISDIR, "unlink directory"),
    (os.path.join(nonempty_dir, "missing-unlink-target"), errno.ENOENT, "unlink missing path"),
    (os.path.join(nonempty_dir, "missing-rmdir-target"), errno.ENOENT, "rmdir missing path"),
    (os.path.join(missing_parent, "new-directory"), errno.ENOENT, "mkdir missing parent"),
    (os.path.join(missing_parent, "child"), errno.ENOENT, "rmdir missing parent"),
    (os.path.join(nonempty_dir, "."), errno.EINVAL, "rmdir final dot component"),
    (file_path, errno.ENOTDIR, "rmdir regular file"),
    (symlink_path, errno.ENOTDIR, "rmdir symlink to directory"),
    (os.path.join(file_path, "child"), errno.ENOTDIR, "mkdir parent is regular file"),
    (os.path.join(file_path, "child"), errno.ENOTDIR, "rmdir parent is regular file"),
    (os.path.join(loop_a, "child"), errno.ELOOP, "mkdir symlink loop in parent"),
    (os.path.join(loop_a, "child"), errno.ELOOP, "rmdir symlink loop in parent"),
    (os.path.join(nonempty_dir, long_component), errno.ENAMETOOLONG, "mkdir component over NAME_MAX"),
    (os.path.join(nonempty_dir, long_component), errno.ENAMETOOLONG, "rmdir component over NAME_MAX"),
    (mountpoint, errno.EBUSY, "rmdir mounted filesystem root"),
    (nonempty_dir, errno.ENOTEMPTY, "rmdir non-empty directory"),
    (nonempty_dir, errno.EEXIST, "mkdir existing directory"),
    (file_path, errno.EEXIST, "mkdir existing regular file"),
):
    try:
        if label.startswith("mkdir"):
            os.mkdir(path)
        elif label.startswith("unlink"):
            os.unlink(path)
        else:
            os.rmdir(path)
    except OSError as error:
        assert error.errno == expected, (label, error.errno, expected)
    else:
        raise AssertionError(f"{label} unexpectedly succeeded")

os.unlink(loop_a)
os.unlink(loop_b)
assert os.path.islink(symlink_path), "rmdir symlink unexpectedly removed the symlink"
assert os.path.isdir(nonempty_dir), "rmdir symlink unexpectedly affected its target"
os.unlink(symlink_path)

child_file = os.path.join(file_path, "child-file")
try:
    child_fd = os.open(child_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
except OSError as error:
    assert error.errno == errno.ENOTDIR, ("create below regular file", error.errno)
else:
    os.close(child_fd)
    raise AssertionError("file creation below a regular file unexpectedly succeeded")

for label, operation, child_name in (
    (
        "hard link below regular file",
        lambda child: os.link(file_path, child),
        "child-hardlink",
    ),
    (
        "symbolic link below regular file",
        lambda child: os.symlink("target", child),
        "child-symlink",
    ),
):
    child = os.path.join(file_path, child_name)
    try:
        operation(child)
    except OSError as error:
        assert error.errno == errno.ENOTDIR, (label, error.errno)
    else:
        raise AssertionError(f"{label} unexpectedly succeeded")
    assert not os.path.lexists(child), (label, "child entry appeared")

rename_source = os.path.join(nonempty_dir, "rename-source")
with open(rename_source, "wb") as source_file:
    source_file.write(b"rename source remains")
rename_target = os.path.join(file_path, "child-rename-target")
try:
    os.replace(rename_source, rename_target)
except OSError as error:
    assert error.errno == errno.ENOTDIR, ("rename destination below regular file", error.errno)
else:
    raise AssertionError("rename into a regular file unexpectedly succeeded")
with open(rename_source, "rb") as source_file:
    assert source_file.read() == b"rename source remains"
assert not os.path.lexists(rename_target)
os.unlink(rename_source)

with open(file_path, "rb") as existing_file:
    assert existing_file.read() == b"rmdir target file\n"

file_fd = os.open(file_path, os.O_RDONLY)
try:
    try:
        os.listdir(file_fd)
    except OSError as error:
        assert error.errno == errno.ENOTDIR, ("listdir regular fd", error.errno)
    else:
        raise AssertionError("listdir on regular-file descriptor unexpectedly succeeded")
finally:
    os.close(file_fd)

directory_fd = os.open(nonempty_dir, os.O_RDONLY | os.O_DIRECTORY)
first_listing = sorted(os.listdir(directory_fd))
second_listing = sorted(os.listdir(directory_fd))
assert first_listing == second_listing == [os.path.basename(file_path)]
os.close(directory_fd)
try:
    os.listdir(directory_fd)
except OSError as error:
    assert error.errno == errno.EBADF, ("listdir closed fd", error.errno)
else:
    raise AssertionError("listdir on closed descriptor unexpectedly succeeded")
PY
rm -f "${not_a_directory}" "${not_a_directory_link}"
rmdir "${dir}"

echo "OK directories/mkdir/rmdir/stat/ls/unlink-dir errno=EBADF,EBUSY,EEXIST,EISDIR,EINVAL,ELOOP,ENAMETOOLONG,ENOENT,ENOTDIR,ENOTEMPTY rmdir_symlink=ENOTDIR child_entry_parent_errors=1 directory_fd_repeat=1"
