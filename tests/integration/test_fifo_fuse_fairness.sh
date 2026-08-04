#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

PYTHON_BIN="${PYTHON:-python3}"
WORKLOAD="${ROOT}/tests/integration/test_fifo_fuse_fairness.py"
LIMITS="${FIFO_FUSE_FAIRNESS_LIMITS:-0 1 2 4}"
REPEAT="${FIFO_FUSE_FAIRNESS_REPEAT:-3}"
LARGE_BYTES="${FIFO_FUSE_FAIRNESS_LARGE_BYTES:-4194304}"
LARGE_CHUNK_BYTES="${FIFO_FUSE_FAIRNESS_LARGE_CHUNK_BYTES:-4096}"
LARGE_PACE_MS="${FIFO_FUSE_FAIRNESS_LARGE_PACE_MS:-0.5}"
MINIMUM_LARGE_DURATION_MS="${FIFO_FUSE_FAIRNESS_MINIMUM_LARGE_DURATION_MS:-300}"
SMALL_FILES="${FIFO_FUSE_FAIRNESS_SMALL_FILES:-24}"
SMALL_BYTES="${FIFO_FUSE_FAIRNESS_SMALL_BYTES:-4096}"
OVERLAP_MIN_PERCENT="${FIFO_FUSE_FAIRNESS_OVERLAP_MIN_PERCENT:-90}"
TIMEOUT_SECONDS="${FIFO_FUSE_FAIRNESS_TIMEOUT_SECONDS:-120}"
RUN_ID="${FIFO_FUSE_FAIRNESS_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
COMMIT_SHORT="$(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
ARTIFACT_DIR="${FIFO_FUSE_FAIRNESS_ARTIFACT_DIR:-/tmp/fod-fifo-fuse-fairness/${COMMIT_SHORT}-${RUN_ID}}"

if ! command -v "${PYTHON_BIN}" >/dev/null 2>&1; then
  echo "Python is required for FIFO FUSE fairness testing" >&2
  exit 1
fi

case "${REPEAT}" in
  ''|*[!0-9]*)
    echo "FIFO_FUSE_FAIRNESS_REPEAT must be a positive integer: ${REPEAT}" >&2
    exit 1
    ;;
esac
if (( REPEAT < 1 )); then
  echo "FIFO_FUSE_FAIRNESS_REPEAT must be at least 1" >&2
  exit 1
fi

mkdir -p "${ARTIFACT_DIR}"
fod_test_setup "${ROOT}"
fod_test_init_schema

cleanup() {
  fod_test_cleanup
}
trap cleanup EXIT

run_files=()

for limit in ${LIMITS}; do
  case "${limit}" in
    ''|*[!0-9]*)
      echo "FIFO_FUSE_FAIRNESS_LIMITS contains a non-negative integer violation: ${limit}" >&2
      exit 1
      ;;
  esac

  export FOD_TASK_READ_ACTIVE_LIMIT=0
  export FOD_TASK_WRITE_ACTIVE_LIMIT="${limit}"
  export FOD_FOPEN_DIRECT_IO=1
  export FOD_PROFILE_IO=1
  export FOD_ENABLE_EXTENTS="${FIFO_FUSE_FAIRNESS_ENABLE_EXTENTS:-1}"
  export FOD_LOG_LEVEL="${FOD_LOG_LEVEL:-info}"

  fod_test_make_mountpoint "/tmp/fod-fifo-fairness-limit-${limit}"
  fod_test_start_mount "${MOUNTPOINT}"

  for run_index in $(seq 1 "${REPEAT}"); do
    output="${ARTIFACT_DIR}/limit-${limit}-run-${run_index}.json"
    "${PYTHON_BIN}" "${WORKLOAD}" run \
      --mountpoint "${MOUNTPOINT}" \
      --output "${output}" \
      --limit "${limit}" \
      --run-index "${run_index}" \
      --large-bytes "${LARGE_BYTES}" \
      --large-chunk-bytes "${LARGE_CHUNK_BYTES}" \
      --large-pace-ms "${LARGE_PACE_MS}" \
      --minimum-large-duration-ms "${MINIMUM_LARGE_DURATION_MS}" \
      --small-files "${SMALL_FILES}" \
      --small-bytes "${SMALL_BYTES}" \
      --overlap-min-percent "${OVERLAP_MIN_PERCENT}" \
      --timeout-seconds "${TIMEOUT_SECONDS}"
    run_files+=("${output}")
  done

  cp "${LOG_FILE}" "${ARTIFACT_DIR}/limit-${limit}-mount.log"
  fod_test_cleanup
done

"${PYTHON_BIN}" "${WORKLOAD}" summarize \
  --output-json "${ARTIFACT_DIR}/fifo-fuse-fairness-summary.json" \
  --output-md "${ARTIFACT_DIR}/fifo-fuse-fairness-summary.md" \
  "${run_files[@]}"

echo
echo "FIFO FUSE fairness artifacts: ${ARTIFACT_DIR}"
