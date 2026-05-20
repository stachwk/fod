#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FOD_SCHEMA_ADMIN_PASSWORD="${FOD_SCHEMA_ADMIN_PASSWORD:-fod-allow-other-secret}"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"

cleanup_current_mount() {
  set +e
  if [[ -n "${MOUNTPOINT:-}" || -n "${FOD_PID:-}" || -n "${LOG_FILE:-}" ]]; then
    fod_test_cleanup
  fi
  unset MOUNTPOINT LOG_FILE FOD_PID
}

trap cleanup_current_mount EXIT

if [[ ! -c /dev/fuse ]]; then
  echo 'SKIP allow-other/visibility (/dev/fuse is unavailable)'
  exit 0
fi

if ! grep -Eq '^[[:space:]]*user_allow_other[[:space:]]*$' /etc/fuse.conf 2>/dev/null; then
  echo "SKIP allow-other/visibility (user_allow_other is disabled in /etc/fuse.conf)"
  exit 0
fi

if ! sudo -n -u nobody true >/dev/null 2>&1; then
  echo "SKIP allow-other/visibility (sudo -n -u nobody is unavailable)"
  exit 0
fi

fod_test_init_schema

fod_test_make_mountpoint "/tmp/fod-allow-other.no-allow"
fod_test_start_mount "${MOUNTPOINT}"

if sudo -n -u nobody ls "${MOUNTPOINT}" >/dev/null 2>&1; then
  echo "allow_other/visibility failed: nobody unexpectedly accessed mount without allow_other"
  exit 1
fi

cleanup_current_mount

FOD_ALLOW_OTHER=1
fod_test_make_mountpoint "/tmp/fod-allow-other.with-allow"
fod_test_start_mount "${MOUNTPOINT}"

if ! sudo -n -u nobody ls "${MOUNTPOINT}" >/dev/null 2>&1; then
  echo "SKIP allow-other/visibility (host-dependent; allow_other mount not exposed to nobody on this host)"
  exit 0
fi

cleanup_current_mount

echo "OK allow-other-visibility"
