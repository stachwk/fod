#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmpdir="$(mktemp -d /tmp/fod-mount-wrapper-path.XXXXXX)"
trap 'rm -rf "${tmpdir}"' EXIT

mkdir -p "${tmpdir}/bin"
cp "${ROOT}/mount.fod" "${tmpdir}/mount.fod"
chmod +x "${tmpdir}/mount.fod"

cat >"${tmpdir}/bin/fod-bootstrap" <<'EOF'
#!/usr/bin/env bash
printf 'FOD_ROLE=%s\n' "${FOD_ROLE:-unset}"
printf 'FOD_RUST_FUSE_READONLY=%s\n' "${FOD_RUST_FUSE_READONLY:-unset}"
printf 'FOD_CONFIG=%s\n' "${FOD_CONFIG:-unset}"
printf 'ARGS=%s\n' "$*"
EOF
chmod +x "${tmpdir}/bin/fod-bootstrap"

printf '[database]\n' >"${tmpdir}/fod_config.ini"

(
  cd "${tmpdir}"
  PATH="${tmpdir}/bin:${PATH}" ./mount.fod "${tmpdir}/mnt" -o role=auto,ro,allow_other
) >"${tmpdir}/output.txt"

[[ -d "${tmpdir}/mnt" ]]
grep -Fq "FOD_ROLE=auto" "${tmpdir}/output.txt"
grep -Fq "FOD_RUST_FUSE_READONLY=1" "${tmpdir}/output.txt"
grep -Fq "FOD_CONFIG=${tmpdir}/fod_config.ini" "${tmpdir}/output.txt"
grep -Fq "ARGS=-f ${tmpdir}/mnt" "${tmpdir}/output.txt"

echo "OK mount-wrapper-path-and-ro"
