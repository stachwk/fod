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

printf '[database]\n' >"${tmpdir}/fod_config.ini"

(
  cd "${tmpdir}"
  FOD_BOOTSTRAP_BIN="${tmpdir}/bin/fod-bootstrap" "${ROOT}/mount.fod" "${tmpdir}/mnt" -o role=auto,allow_other,profile=bulk_write,selinux=off,acl=off,default_permissions
) >"${tmpdir}/output.txt"

[[ -d "${tmpdir}/mnt" ]]
grep -Fq "FOD_CONFIG=${tmpdir}/fod_config.ini" "${tmpdir}/output.txt"
grep -Fq "FOD_ALLOW_OTHER=1" "${tmpdir}/output.txt"
grep -Fq "FOD_PROFILE=bulk_write" "${tmpdir}/output.txt"
grep -Fq "ARGS=-f ${tmpdir}/mnt --profile bulk_write" "${tmpdir}/output.txt"

(
  cd "${tmpdir}"
  FOD_BOOTSTRAP_BIN="${tmpdir}/bin/fod-bootstrap" "${ROOT}/mount.fod" "${tmpdir}/mnt-typo" -o rool=primary,_netdev,x-systemd.device-timeout=30,allow_other
) >"${tmpdir}/warn.txt" 2>&1

[[ -d "${tmpdir}/mnt-typo" ]]
grep -Fq "mount.fod: ignoring unrecognized option 'rool=primary'" "${tmpdir}/warn.txt"
grep -Fq "FOD_ALLOW_OTHER=1" "${tmpdir}/warn.txt"
if grep -Fq "_netdev" "${tmpdir}/warn.txt" || grep -Fq "x-systemd.device-timeout=30" "${tmpdir}/warn.txt"; then
  cat "${tmpdir}/warn.txt"
  echo "unexpected warning for system passthrough option"
  exit 1
fi

echo "OK mount-wrapper-options"
