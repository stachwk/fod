#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-statfs-use-ino

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

suffix="$(python3 - <<'PY'
import uuid
print(uuid.uuid4().hex[:8])
PY
)"
dir_path="${MOUNTPOINT}/statfs-${suffix}"
file_path="${dir_path}/payload.txt"

cleanup() {
  fod_test_cleanup
}
trap cleanup EXIT

mkdir -p "${dir_path}"
printf '%s\n' "statfs use_ino" > "${file_path}"

file_ino="$(stat -c '%i' "${file_path}")"
dir_ino="$(stat -c '%i' "${dir_path}")"
mount_statfs="$(stat -f -c 'bsize=%S frsize=%s blocks=%b bfree=%f bavail=%a files=%c ffree=%d' "${MOUNTPOINT}")"
mount_df="$(df -Ph "${MOUNTPOINT}")"

if [[ "${file_ino}" == "0" || "${dir_ino}" == "0" ]]; then
  echo "inode numbers must be non-zero"
  exit 1
fi

if [[ "${mount_statfs}" != *"blocks="* || "${mount_statfs}" != *"files="* ]]; then
  echo "statfs output is incomplete: ${mount_statfs}"
  exit 1
fi

if ! grep -Fq -- "${MOUNTPOINT}" <<<"${mount_df}"; then
  echo "df output does not mention mountpoint"
  exit 1
fi

echo "OK statfs/use_ino"
