#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"

tmpdir="$(mktemp -d /tmp/fod-root-owned.XXXXXX)"
trap 'set +e; if [[ -n "${FOD_PID:-}" ]] && kill -0 "${FOD_PID}" >/dev/null 2>&1; then kill "${FOD_PID}" >/dev/null 2>&1 || true; wait "${FOD_PID}" >/dev/null 2>&1 || true; fi; if [[ -n "${MOUNTPOINT:-}" ]]; then fusermount3 -u "${MOUNTPOINT}" 2>/dev/null || fusermount -u "${MOUNTPOINT}" 2>/dev/null || umount "${MOUNTPOINT}" 2>/dev/null || true; fi; rm -rf "${tmpdir}"' EXIT

if ! grep -Eq '^[[:space:]]*user_allow_other[[:space:]]*$' /etc/fuse.conf 2>/dev/null; then
  echo "SKIP root-owned permissions (user_allow_other is disabled in /etc/fuse.conf)"
  exit 0
fi

local_dir="$(mktemp -d /home/wojtek/fod-root-owned-local.XXXXXX)"
local_file="${local_dir}/root-owned.txt"

mkdir -p "${local_dir}"
sudo -n install -m 0644 /dev/null "${local_file}"
local_stat="$(stat -c '%u:%g|%a' "${local_file}")"
rm -f "${local_file}"
rmdir "${local_dir}"

MOUNTPOINT="${tmpdir}/fod-root-owned"
mkdir -p "${MOUNTPOINT}"
POSTGRES_DB="${POSTGRES_DB}" POSTGRES_USER="${POSTGRES_USER}" POSTGRES_PASSWORD="${POSTGRES_PASSWORD}" \
  sudo -n env ${ADMP_TRACE_ENV:-} FOD_USE_FUSE_CONTEXT=1 /usr/local/sbin/mount.fod "${MOUNTPOINT}" \
  -o role=auto,selinux=off,acl=off,default_permissions,allow_other,profile=default \
  >/tmp/fod-root-owned.mount.log 2>&1 &
FOD_PID=$!

for _ in $(seq 1 60); do
  if mountpoint -q "${MOUNTPOINT}"; then
    break
  fi
  if ! kill -0 "${FOD_PID}" >/dev/null 2>&1; then
    cat /tmp/fod-root-owned.mount.log
    echo "FOD root mount failed"
    exit 1
  fi
  sleep 1
done

if ! mountpoint -q "${MOUNTPOINT}"; then
  cat /tmp/fod-root-owned.mount.log
  echo "FOD root mount did not become ready"
  exit 1
fi

fod_dir="${MOUNTPOINT}/root-owned"
mkdir -p "${fod_dir}"
fod_file="${fod_dir}/root-owned.txt"
sudo -n install -m 0644 /dev/null "${fod_file}"
fod_stat="$(stat -c '%u:%g|%a' "${fod_file}")"
rm -f "${fod_file}"
rmdir "${fod_dir}"

if [[ "${local_stat}" != "${fod_stat}" ]]; then
  echo "local root-owned stat: ${local_stat}"
  echo "fod root-owned stat:  ${fod_stat}"
  exit 1
fi

echo "OK root-owned-permissions ${local_stat}"
