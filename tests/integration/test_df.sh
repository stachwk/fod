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

echo "OK df/Ph/Phi"
