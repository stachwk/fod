#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"

if [[ "$(getenforce 2>/dev/null || true)" != "Enforcing" ]]; then
  echo "SKIP Rocky SELinux operational test requires SELinux Enforcing"
  exit 0
fi

for tool in curl fusermount3 getsebool httpd ls mountpoint ps setsebool systemctl; do
  command -v "${tool}" >/dev/null || {
    echo "missing_tool=${tool}" >&2
    exit 1
  }
done

BASE="${FOD_ROCKY_SELINUX_HTTPD_BASE:-/srv/fod-httpd-fuse-proof}"
MOUNTPOINT="${BASE}/mnt"
HTTPD_CONF="/etc/httpd/conf.d/fod-fuse-proof.conf"
FOD_LOG="${FOD_ROCKY_SELINUX_HTTPD_FOD_LOG:-/tmp/fod-httpd-systemd-proof-fod.log}"
HTTPD_ERROR_LOG="${FOD_ROCKY_SELINUX_HTTPD_ERROR_LOG:-/tmp/fod-httpd-systemd-proof-error.log}"
HTTPD_ACCESS_LOG="${FOD_ROCKY_SELINUX_HTTPD_ACCESS_LOG:-/tmp/fod-httpd-systemd-proof-access.log}"
OFF_BODY="/tmp/fod-httpd-systemd-proof-off.body"
ON_BODY="/tmp/fod-httpd-systemd-proof-on.body"
FOD_PID=""
HTTPD_WAS_ACTIVE=0
INITIAL_HTTPD_USE_FUSEFS="$(getsebool httpd_use_fusefs | awk '{print $3}')"

cleanup() {
  local rc=$?
  set +e
  sudo systemctl stop httpd >/dev/null 2>&1
  sudo rm -f "${HTTPD_CONF}"
  if [[ "${HTTPD_WAS_ACTIVE}" == "1" ]]; then
    sudo systemctl restart httpd >/dev/null 2>&1
  fi
  if [[ "${INITIAL_HTTPD_USE_FUSEFS}" == "on" ]]; then
    sudo setsebool httpd_use_fusefs on >/dev/null 2>&1
  else
    sudo setsebool httpd_use_fusefs off >/dev/null 2>&1
  fi
  fusermount3 -u "${MOUNTPOINT}" >/dev/null 2>&1
  if [[ -n "${FOD_PID}" ]]; then
    wait "${FOD_PID}" >/dev/null 2>&1
  fi
  exit "${rc}"
}
trap cleanup EXIT

systemctl is-active --quiet httpd && HTTPD_WAS_ACTIVE=1
sudo systemctl stop httpd >/dev/null 2>&1 || true
sudo rm -f "${HTTPD_CONF}"
sudo mkdir -p "${MOUNTPOINT}"
sudo chown -R "$(id -u):$(id -g)" "${BASE}"
sudo chmod 0755 /srv "${BASE}" "${MOUNTPOINT}"
fusermount3 -u "${MOUNTPOINT}" >/dev/null 2>&1 || true
rm -f "${FOD_LOG}" "${HTTPD_ERROR_LOG}" "${HTTPD_ACCESS_LOG}" "${OFF_BODY}" "${ON_BODY}"

FOD_ALLOW_OTHER=1 \
FOD_LOG_LEVEL="${FOD_LOG_LEVEL:-INFO}" \
FOD_SELINUX=on \
FOD_ACL=on \
FOD_DEFAULT_PERMISSIONS=1 \
"${FOD_BOOTSTRAP_BIN}" \
  --config "${FOD_CONFIG}" \
  --role "${FOD_ROLE}" \
  --selinux on \
  --acl on \
  --default-permissions \
  -f "${MOUNTPOINT}" >"${FOD_LOG}" 2>&1 &
FOD_PID=$!

for _ in $(seq 1 80); do
  if mountpoint -q "${MOUNTPOINT}"; then
    break
  fi
  sleep 0.25
done
if ! mountpoint -q "${MOUNTPOINT}"; then
  cat "${FOD_LOG}" >&2 || true
  echo "FOD mount did not become ready" >&2
  exit 1
fi

printf 'fod-httpd-ok\n' >"${MOUNTPOINT}/index.txt"
chmod 0755 "${MOUNTPOINT}"
chmod 0644 "${MOUNTPOINT}/index.txt"

mount_label="$(ls -Zd "${MOUNTPOINT}")"
file_label="$(ls -Z "${MOUNTPOINT}/index.txt")"
if [[ "${mount_label}" != *":fusefs_t:"* ]] || [[ "${file_label}" != *":fusefs_t:"* ]]; then
  echo "Expected FOD mount and file to be labeled fusefs_t" >&2
  echo "${mount_label}" >&2
  echo "${file_label}" >&2
  exit 1
fi

sudo tee "${HTTPD_CONF}" >/dev/null <<EOF
<VirtualHost 127.0.0.1:80>
    ServerName 127.0.0.1
    DocumentRoot "${MOUNTPOINT}"
    ErrorLog ${HTTPD_ERROR_LOG}
    CustomLog ${HTTPD_ACCESS_LOG} combined
    <Directory "${MOUNTPOINT}">
        Options None
        AllowOverride None
        Require all granted
    </Directory>
</VirtualHost>
EOF

sudo httpd -t

sudo setsebool httpd_use_fusefs off
sudo systemctl start httpd
sleep 1
httpd_contexts="$(ps -eZ | grep '[h]ttpd' || true)"
echo "${httpd_contexts}" | grep -q 'system_u:system_r:httpd_t:s0' || {
  echo "Expected systemd-started httpd_t processes" >&2
  echo "${httpd_contexts}" >&2
  exit 1
}
off_code="$(curl -sS -o "${OFF_BODY}" -w '%{http_code}' http://127.0.0.1/index.txt || true)"
sudo systemctl stop httpd
if [[ "${off_code}" != "403" ]]; then
  echo "Expected httpd_use_fusefs=off to deny FOD content with 403, got ${off_code}" >&2
  cat "${OFF_BODY}" >&2 || true
  exit 1
fi

sudo setsebool httpd_use_fusefs on
sudo systemctl start httpd
sleep 1
on_code="$(curl -sS -o "${ON_BODY}" -w '%{http_code}' http://127.0.0.1/index.txt || true)"
sudo systemctl stop httpd
if [[ "${on_code}" != "200" ]]; then
  echo "Expected httpd_use_fusefs=on to allow FOD content with 200, got ${on_code}" >&2
  cat "${ON_BODY}" >&2 || true
  exit 1
fi
grep -qx 'fod-httpd-ok' "${ON_BODY}"

echo "OK Rocky SELinux operational FOD fusefs_t/httpd_t proof"
