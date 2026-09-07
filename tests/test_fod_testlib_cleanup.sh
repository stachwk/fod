#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=integration/fod_testlib.sh
source "${ROOT}/tests/integration/fod_testlib.sh"

TMP="$(mktemp -d /tmp/fod-testlib-cleanup.XXXXXX)"
cleanup_tmp() {
    rm -rf "${TMP}"
}
trap cleanup_tmp EXIT

MOUNTPOINT="${TMP}/mount"
LOG_FILE="${TMP}/live.log"
FOD_TEST_LOG_ARCHIVE="${TMP}/archive.log"
FOD_PROFILE_IO=0
FOD_STRACE_SUMMARY_FILE=""
FOD_STRACE_LABEL=""

mkdir -p "${MOUNTPOINT}"
: >"${LOG_FILE}"

(
    sleep 0.05
    printf '%s\n' \
        'FOD PostgreSQL lane observability: stage=post-mount lane=shared operation_count=7' \
        >>"${LOG_FILE}"
) &
FOD_PID=$!

fod_test_cleanup

if [[ ! -f "${TMP}/archive.log" ]]; then
    echo "cleanup did not preserve the requested log archive" >&2
    exit 1
fi

if ! grep -Fq \
    'FOD PostgreSQL lane observability: stage=post-mount lane=shared operation_count=7' \
    "${TMP}/archive.log"; then
    echo "cleanup archived the log before final process output was stable" >&2
    cat "${TMP}/archive.log" >&2 || true
    exit 1
fi

echo "OK fod-testlib-cleanup-final-log"
