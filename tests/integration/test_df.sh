#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-df
OUTPUT_DIR="$(mktemp -d /tmp/fod-df-output.XXXXXX)"
export FOD_PG_VISIBLE_PATH="${OUTPUT_DIR}"
export FOD_MAX_FS_SIZE_BYTES=1125899906842624
read -r host_fragment_size host_available_blocks < <(
  stat -f -c '%S %a' "${FOD_PG_VISIBLE_PATH}"
)
host_available_bytes=$((host_fragment_size * host_available_blocks))

cleanup() {
  rm -rf "${OUTPUT_DIR}"
  fod_test_cleanup
}

trap cleanup EXIT

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"
rm -f "${MOUNTPOINT}/accounting-link"

python3 - "${MOUNTPOINT}" <<'PY'
from pathlib import Path
import sys

mountpoint = Path(sys.argv[1])
(mountpoint / "accounting-small.bin").write_bytes(b"a" * 4097)
(mountpoint / "accounting-large.bin").write_bytes(b"b" * 1048577)
with (mountpoint / "accounting-sparse.bin").open("wb") as sparse:
    sparse.seek(1048576)
    sparse.write(b"c")
PY
sync

for file in \
  "${MOUNTPOINT}/accounting-small.bin" \
  "${MOUNTPOINT}/accounting-large.bin"
do
  read -r logical_size stat_blocks stat_block_unit io_block_size < <(
    stat -c '%s %b %B %o' "${file}"
  )
  expected_blocks=$((((logical_size + io_block_size - 1) / io_block_size) * io_block_size / 512))

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

sparse_file="${MOUNTPOINT}/accounting-sparse.bin"
read -r sparse_size sparse_blocks < <(stat -c '%s %b' "${sparse_file}")
sparse_logical_blocks=$(((sparse_size + 511) / 512))
if [[ "${sparse_blocks}" -le 0 || "${sparse_blocks}" -ge "${sparse_logical_blocks}" ]]; then
  printf 'unexpected sparse allocation: size=%s blocks=%s logical_blocks=%s\n' \
    "${sparse_size}" "${sparse_blocks}" "${sparse_logical_blocks}" >&2
  exit 1
fi

read -r fs_bsize fs_frsize fs_blocks fs_free_blocks fs_inodes fs_free_inodes fs_name_max < <(
  stat -f -c '%s %S %b %f %c %d %l' "${MOUNTPOINT}"
)

if [[ "${fs_bsize}" -ne "${fs_frsize}" ]]; then
  printf 'unexpected statfs block sizes: bsize=%s frsize=%s\n' \
    "${fs_bsize}" "${fs_frsize}" >&2
  exit 1
fi

reported_available_bytes=$((fs_free_blocks * fs_frsize))
if [[ "${reported_available_bytes}" -gt "${host_available_bytes}" ]]; then
  printf 'statfs exceeds host available bytes: reported=%s host_before_mount=%s\n' \
    "${reported_available_bytes}" "${host_available_bytes}" >&2
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

ln -s accounting-small.bin "${MOUNTPOINT}/accounting-link"
read -r symlink_fs_inodes symlink_fs_free_inodes < <(
  stat -f -c '%c %d' "${MOUNTPOINT}"
)
if [[ "${symlink_fs_inodes}" -ne $((fs_inodes + 1)) ]]; then
  printf 'symlink did not increment statfs inode count: before=%s after=%s\n' \
    "${fs_inodes}" "${symlink_fs_inodes}" >&2
  exit 1
fi
if [[ "${symlink_fs_free_inodes}" -ne "${fs_free_inodes}" ]]; then
  printf 'symlink unexpectedly changed virtual inode headroom: before=%s after=%s\n' \
    "${fs_free_inodes}" "${symlink_fs_free_inodes}" >&2
  exit 1
fi

DF_BYTES_OUTPUT="${OUTPUT_DIR}/df.ph"
DF_INODES_OUTPUT="${OUTPUT_DIR}/df.phi"
df -Ph "${MOUNTPOINT}" > "${DF_BYTES_OUTPUT}"
df -Phi "${MOUNTPOINT}" > "${DF_INODES_OUTPUT}"

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
' "${DF_BYTES_OUTPUT}"

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
' "${DF_INODES_OUTPUT}"

echo "OK df/Ph/Phi, statfs fields, symlink inode accounting, and sparse POSIX st_blocks accounting"
