#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/fod-fuse-cleanup-policy.XXXXXX")"
trap 'rm -rf "${tmpdir}" /tmp/fod-rust-fuse-cleanup-policy-$$' EXIT

fake_bin="${tmpdir}/bin"
state_file="${tmpdir}/mounted"
call_log="${tmpdir}/calls.log"
test_mount="/tmp/fod-rust-fuse-cleanup-policy-$$/mount"
mkdir -p "${fake_bin}" "${test_mount}"
touch "${state_file}"

cat >"${fake_bin}/findmnt" <<'EOF'
#!/usr/bin/env bash
set -eu
if [[ -e "${FOD_TEST_FAKE_STATE}" ]]; then
  printf '%s fuse.fod\n' "${FOD_TEST_FAKE_MOUNT}"
fi
printf '/tmp/not-fod/mount fuse.fod\n'
EOF

cat >"${fake_bin}/mountpoint" <<'EOF'
#!/usr/bin/env bash
set -eu
target="${!#}"
if [[ "${target}" == "${FOD_TEST_FAKE_MOUNT}" && -e "${FOD_TEST_FAKE_STATE}" ]]; then
  exit 0
fi
exit 1
EOF

cat >"${fake_bin}/fusermount3" <<'EOF'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >>"${FOD_TEST_FAKE_LOG}"
if [[ "${1:-}" == "-uz" ]]; then
  rm -f "${FOD_TEST_FAKE_STATE}"
  exit 0
fi
exit 1
EOF

chmod +x "${fake_bin}/findmnt" "${fake_bin}/mountpoint" "${fake_bin}/fusermount3"

FOD_TEST_FAKE_STATE="${state_file}" \
FOD_TEST_FAKE_MOUNT="${test_mount}" \
FOD_TEST_FAKE_LOG="${call_log}" \
FOD_FINDMNT="${fake_bin}/findmnt" \
FOD_MOUNTPOINT="${fake_bin}/mountpoint" \
FOD_FUSERMOUNT3="${fake_bin}/fusermount3" \
FOD_FUSERMOUNT="${tmpdir}/missing-fusermount" \
FOD_UMOUNT="${tmpdir}/missing-umount" \
FOD_TEST_FUSE_CLEANUP_RETRIES=2 \
FOD_TEST_FUSE_CLEANUP_SLEEP=0 \
bash "${repo_root}/scripts/fod-test-fuse-cleanup.sh" clean

if [[ -e "${state_file}" ]]; then
  echo "cleanup policy did not clear the simulated stale mount" >&2
  exit 1
fi
if [[ -d "$(dirname -- "${test_mount}")" ]]; then
  echo "cleanup policy did not remove the stale test workspace" >&2
  exit 1
fi

grep -Fq -- "-u ${test_mount}" "${call_log}"
grep -Fq -- "-uz ${test_mount}" "${call_log}"
if grep -Fq '/tmp/not-fod/mount' "${call_log}"; then
  echo "cleanup policy attempted to touch a non-FOD test mount" >&2
  exit 1
fi

echo "OK fuse-test-cleanup-policy"
