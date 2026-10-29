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
python3 - "${not_a_directory}" "${dir}" <<'PY'
import errno
import os
import sys

file_path, nonempty_dir = sys.argv[1:]
missing_parent = os.path.join(nonempty_dir, "missing-parent")
for path, expected, label in (
    (nonempty_dir, errno.EISDIR, "unlink directory"),
    (os.path.join(nonempty_dir, "missing-unlink-target"), errno.ENOENT, "unlink missing path"),
    (os.path.join(nonempty_dir, "missing-rmdir-target"), errno.ENOENT, "rmdir missing path"),
    (os.path.join(missing_parent, "new-directory"), errno.ENOENT, "mkdir missing parent"),
    (os.path.join(missing_parent, "child"), errno.ENOENT, "rmdir missing parent"),
    (file_path, errno.ENOTDIR, "rmdir regular file"),
    (os.path.join(file_path, "child"), errno.ENOTDIR, "mkdir parent is regular file"),
    (os.path.join(file_path, "child"), errno.ENOTDIR, "rmdir parent is regular file"),
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
rm -f "${not_a_directory}"
rmdir "${dir}"

echo "OK directories/mkdir/rmdir/stat/ls/unlink-dir errno=EBADF,EEXIST,EISDIR,ENOENT,ENOTDIR,ENOTEMPTY mkdir_rmdir_parent_errors=1 directory_fd_repeat=1"
