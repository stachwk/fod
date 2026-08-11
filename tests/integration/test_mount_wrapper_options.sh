#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmpdir="$(mktemp -d /tmp/fod-mount-wrapper.XXXXXX)"
trap 'rm -rf "${tmpdir}"' EXIT

mkdir -p "${tmpdir}/bin"
cat >"${tmpdir}/bin/fod-bootstrap" <<'EOF'
#!/usr/bin/env bash
printf 'FOD_CONFIG=%s\n' "${FOD_CONFIG:-unset}"
printf 'FOD_ALLOW_OTHER=%s\n' "${FOD_ALLOW_OTHER:-unset}"
printf 'FOD_PROFILE=%s\n' "${FOD_PROFILE:-unset}"
printf 'ARGS=%s\n' "$*"
EOF
chmod +x "${tmpdir}/bin/fod-bootstrap"

db01_ini="${tmpdir}/fod.db01.ini"
db02_ini="${tmpdir}/fod.db02.ini"
printf '[database]\nhost = db01.example\n' >"${db01_ini}"
printf '[database]\nhost = db02.example\n' >"${db02_ini}"

# Missing explicit ini= must fail even when FOD_CONFIG exists in the caller.
set +e
FOD_CONFIG="${db01_ini}" \
FOD_BOOTSTRAP_BIN="${tmpdir}/bin/fod-bootstrap" \
  "${ROOT}/mount.fod" "${tmpdir}/mnt-missing" -o role=auto \
  >"${tmpdir}/missing.txt" 2>&1
missing_rc=$?
set -e
[[ "${missing_rc}" -ne 0 ]]
grep -Fq "missing required mount option 'ini=/absolute/path/to/fod.ini'" "${tmpdir}/missing.txt"
[[ ! -d "${tmpdir}/mnt-missing" ]]

# Relative paths are rejected so fstab/systemd mounts never depend on cwd.
set +e
FOD_BOOTSTRAP_BIN="${tmpdir}/bin/fod-bootstrap" \
  "${ROOT}/mount.fod" "${tmpdir}/mnt-relative" -o ini=fod.db01.ini \
  >"${tmpdir}/relative.txt" 2>&1
relative_rc=$?
set -e
[[ "${relative_rc}" -ne 0 ]]
grep -Fq "ini path must be absolute" "${tmpdir}/relative.txt"

# Each mount gets the exact INI selected by its own ini= option.
FOD_CONFIG="/should/not/win.ini" \
FOD_BOOTSTRAP_BIN="${tmpdir}/bin/fod-bootstrap" \
  "${ROOT}/mount.fod" none "${tmpdir}/mnt-db01" \
  -o "ini=${db01_ini},role=auto,allow_other,profile=bulk_write,selinux=off,acl=off,default_permissions" \
  >"${tmpdir}/db01.txt"

[[ -d "${tmpdir}/mnt-db01" ]]
grep -Fq "FOD_CONFIG=${db01_ini}" "${tmpdir}/db01.txt"
grep -Fq "FOD_ALLOW_OTHER=1" "${tmpdir}/db01.txt"
grep -Fq "FOD_PROFILE=bulk_write" "${tmpdir}/db01.txt"
grep -Fq "ARGS=-f ${tmpdir}/mnt-db01 --config ${db01_ini} --profile bulk_write" "${tmpdir}/db01.txt"

FOD_CONFIG="${db01_ini}" \
FOD_BOOTSTRAP_BIN="${tmpdir}/bin/fod-bootstrap" \
  "${ROOT}/mount.fod" none "${tmpdir}/mnt-db02" \
  -o "ini=${db02_ini},role=primary" \
  >"${tmpdir}/db02.txt"

[[ -d "${tmpdir}/mnt-db02" ]]
grep -Fq "FOD_CONFIG=${db02_ini}" "${tmpdir}/db02.txt"
grep -Fq "ARGS=-f ${tmpdir}/mnt-db02 --config ${db02_ini}" "${tmpdir}/db02.txt"
if grep -Fq "${db01_ini}" "${tmpdir}/db02.txt"; then
  cat "${tmpdir}/db02.txt"
  echo "db02 mount leaked db01 configuration"
  exit 1
fi

# Legacy explicit aliases remain temporarily supported, but warn.
FOD_BOOTSTRAP_BIN="${tmpdir}/bin/fod-bootstrap" \
  "${ROOT}/mount.fod" "${tmpdir}/mnt-legacy" -o "config=${db01_ini}" \
  >"${tmpdir}/legacy.txt" 2>&1
grep -Fq "option 'config=' is deprecated; use 'ini='" "${tmpdir}/legacy.txt"
grep -Fq "FOD_CONFIG=${db01_ini}" "${tmpdir}/legacy.txt"

# Unknown FOD options still warn, while system passthrough options stay quiet.
FOD_BOOTSTRAP_BIN="${tmpdir}/bin/fod-bootstrap" \
  "${ROOT}/mount.fod" "${tmpdir}/mnt-typo" \
  -o "ini=${db01_ini},rool=primary,_netdev,x-systemd.device-timeout=30,allow_other" \
  >"${tmpdir}/warn.txt" 2>&1

[[ -d "${tmpdir}/mnt-typo" ]]
grep -Fq "mount.fod: ignoring unrecognized option 'rool=primary'" "${tmpdir}/warn.txt"
grep -Fq "FOD_ALLOW_OTHER=1" "${tmpdir}/warn.txt"
if grep -Fq "_netdev" "${tmpdir}/warn.txt" || grep -Fq "x-systemd.device-timeout=30" "${tmpdir}/warn.txt"; then
  cat "${tmpdir}/warn.txt"
  echo "unexpected warning for system passthrough option"
  exit 1
fi

echo "OK mount-wrapper-options explicit-ini"
