#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-df
trap fod_test_cleanup EXIT

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

python3 - "${MOUNTPOINT}" <<'PY'
from pathlib import Path
import sys

mountpoint = Path(sys.argv[1])
(mountpoint / "accounting-small.bin").write_bytes(b"a" * 4097)
(mountpoint / "accounting-large.bin").write_bytes(b"b" * 1048577)
PY
sync

for file in \
  "${MOUNTPOINT}/accounting-small.bin" \
  "${MOUNTPOINT}/accounting-large.bin"
do
  read -r logical_size stat_blocks stat_block_unit < <(stat -c '%s %b %B' "${file}")
  expected_blocks=$(((logical_size + 511) / 512))

  if [[ "${stat_block_unit}" -ne 512 ]]; then
    printf 'unexpected stat block unit for %s: got=%s expected=512\n' \
      "${file}" "${stat_block_unit}" >&2
    exit 1
  fi

  if [[ "${stat_blocks}" -ne "${expected_blocks}" ]]; then
    printf 'unexpected st_blocks for %s: size=%s got=%s expected=%s\n' \
      "${file}" "${logical_size}" "${stat_blocks}" "${expected_blocks}" >&2
    exit 1
  fi

  allocated_bytes="$(du -B1 "${file}" | awk '{print $1}')"
  apparent_bytes="$(du --apparent-size -B1 "${file}" | awk '{print $1}')"
  expected_allocated_bytes=$((expected_blocks * 512))

  if [[ "${allocated_bytes}" -ne "${expected_allocated_bytes}" ]]; then
    printf 'unexpected du allocation for %s: got=%s expected=%s\n' \
      "${file}" "${allocated_bytes}" "${expected_allocated_bytes}" >&2
    exit 1
  fi

  if [[ "${apparent_bytes}" -ne "${logical_size}" ]]; then
    printf 'unexpected apparent size for %s: got=%s expected=%s\n' \
      "${file}" "${apparent_bytes}" "${logical_size}" >&2
    exit 1
  fi
done

read -r fs_bsize fs_frsize fs_blocks fs_free_blocks fs_inodes fs_free_inodes fs_name_max < <(
  stat -f -c '%s %S %b %f %c %d %l' "${MOUNTPOINT}"
)

if [[ "${fs_bsize}" -ne "${fs_frsize}" ]]; then
  printf 'unexpected statfs block sizes: bsize=%s frsize=%s\n' \
    "${fs_bsize}" "${fs_frsize}" >&2
  exit 1
fi

if [[ "${fs_name_max}" -ne 255 ]]; then
  printf 'unexpected statfs name length: got=%s expected=255\n' \
    "${fs_name_max}" >&2
  exit 1
fi

if [[ "${fs_free_inodes}" -le 0 || "${fs_inodes}" -le "${fs_free_inodes}" ]]; then
  printf 'unexpected statfs inode capacity: inodes=%s ifree=%s\n' \
    "${fs_inodes}" "${fs_free_inodes}" >&2
  exit 1
fi

df -Ph "${MOUNTPOINT}" > /tmp/fod-df.ph
df -Phi "${MOUNTPOINT}" > /tmp/fod-df.phi

awk -v mount="${MOUNTPOINT}" '
  NR == 2 {
    if ($6 != mount) {
      print "unexpected mountpoint in df -Ph: " $6
      exit 1
    }
    if ($2 == "" || $3 == "" || $4 == "" || $5 == "") {
      print "missing df -Ph fields"
      exit 1
    }
  }
' /tmp/fod-df.ph

awk -v mount="${MOUNTPOINT}" '
  NR == 2 {
    if ($6 != mount) {
      print "unexpected mountpoint in df -Phi: " $6
      exit 1
    }
    if ($2 == "" || $3 == "" || $4 == "" || $5 == "") {
      print "missing df -Phi fields"
      exit 1
    }
    if ($5 == "100%") {
      print "df -Phi falsely reports inode exhaustion"
      exit 1
    }
  }
' /tmp/fod-df.phi

echo "OK df/Ph/Phi, statfs fields, inode headroom, and POSIX st_blocks accounting"
