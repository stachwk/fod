#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-dirhooks
trap fod_test_cleanup EXIT

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

suffix="$$-$(date +%s%N)"
dir_path="${MOUNTPOINT}/dirhooks_${suffix}"
file_path="${dir_path}/payload.txt"

mkdir -p "${dir_path}"
exec {fd}>"${file_path}"
exec {fd}>&-

if [[ ! -f "${file_path}" ]]; then
  echo "dirhooks file was not created"
  exit 1
fi

rm -f "${file_path}"
rmdir "${dir_path}"

echo "OK dirhooks"
