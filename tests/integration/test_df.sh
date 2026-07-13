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
  }
' /tmp/fod-df.phi

echo "OK df/Ph/Phi and POSIX st_blocks accounting"
