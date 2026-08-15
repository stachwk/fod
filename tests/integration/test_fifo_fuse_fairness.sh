#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

PYTHON_BIN="${PYTHON:-python3}"
WORKLOAD="${ROOT}/tests/integration/test_fifo_fuse_fairness.py"
LIMITS_TEXT="${FIFO_FUSE_FAIRNESS_LIMITS:-0 1 2 4}"
REPEAT="${FIFO_FUSE_FAIRNESS_REPEAT:-3}"
LARGE_WORKERS="${FIFO_FUSE_FAIRNESS_LARGE_WORKERS:-8}"
LARGE_ITERATIONS="${FIFO_FUSE_FAIRNESS_LARGE_ITERATIONS:-8}"
LARGE_WRITE_BYTES="${FIFO_FUSE_FAIRNESS_LARGE_WRITE_BYTES:-262144}"
SMALL_FILES="${FIFO_FUSE_FAIRNESS_SMALL_FILES:-24}"
SMALL_BYTES="${FIFO_FUSE_FAIRNESS_SMALL_BYTES:-4096}"
INJECTION_DELAY_MS="${FIFO_FUSE_FAIRNESS_INJECTION_DELAY_MS:-5}"
OVERLAP_MIN_PERCENT="${FIFO_FUSE_FAIRNESS_OVERLAP_MIN_PERCENT:-90}"
MINIMUM_TAIL_LARGE_OPERATIONS="${FIFO_FUSE_FAIRNESS_MINIMUM_TAIL_LARGE_OPERATIONS:-8}"
MINIMUM_PEAK_QUEUED="${FIFO_FUSE_FAIRNESS_MINIMUM_PEAK_QUEUED:-2}"
MINIMUM_BASELINE_ACTIVE="${FIFO_FUSE_FAIRNESS_MINIMUM_BASELINE_ACTIVE:-2}"
TIMEOUT_SECONDS="${FIFO_FUSE_FAIRNESS_TIMEOUT_SECONDS:-180}"
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

read -r -a limits <<<"${LIMITS_TEXT}"
if (( ${#limits[@]} == 0 )); then
  echo "FIFO_FUSE_FAIRNESS_LIMITS must not be empty" >&2
  exit 1
fi
for limit in "${limits[@]}"; do
  case "${limit}" in
    ''|*[!0-9]*)
      echo "FIFO_FUSE_FAIRNESS_LIMITS contains an invalid value: ${limit}" >&2
      exit 1
      ;;
  esac
done

mkdir -p "${ARTIFACT_DIR}"
fod_test_setup "${ROOT}"
fod_test_init_schema

cleanup() {
  fod_test_cleanup
}
trap cleanup EXIT

run_files=()
limit_count="${#limits[@]}"

for run_index in $(seq 1 "${REPEAT}"); do
  offset=$(( (run_index - 1) % limit_count ))
  echo "FIFO FUSE fairness rotated run ${run_index}/${REPEAT}; offset=${offset}"

  for step in $(seq 0 $((limit_count - 1))); do
    index=$(( (offset + step) % limit_count ))
    limit="${limits[index]}"
    output="${ARTIFACT_DIR}/limit-${limit}-run-${run_index}.json"
    mount_log="${ARTIFACT_DIR}/limit-${limit}-run-${run_index}-mount.log"

    export FOD_TASK_READ_ACTIVE_LIMIT=0
    export FOD_TASK_WRITE_ACTIVE_LIMIT="${limit}"
    export FOD_FUSE_EVENT_THREADS="${FIFO_FUSE_FAIRNESS_EVENT_THREADS:-8}"
    export FOD_FUSE_CLONE_FD="${FIFO_FUSE_FAIRNESS_CLONE_FD:-0}"
    export FOD_TASK_OBSERVABILITY_INTERVAL_MS=100
    export FOD_FOPEN_DIRECT_IO=1
    export FOD_PROFILE_IO=1
    export FOD_LOG_LEVEL="${FOD_LOG_LEVEL:-info}"
    export FOD_TEST_LOG_ARCHIVE="${mount_log}"

    fod_test_make_mountpoint "/tmp/fod-fifo-saturation-limit-${limit}-run-${run_index}"
    fod_test_start_mount "${MOUNTPOINT}"

    set +e
    "${PYTHON_BIN}" "${WORKLOAD}" run \
      --mountpoint "${MOUNTPOINT}" \
      --output "${output}" \
      --limit "${limit}" \
      --run-index "${run_index}" \
      --large-workers "${LARGE_WORKERS}" \
      --large-iterations "${LARGE_ITERATIONS}" \
      --large-write-bytes "${LARGE_WRITE_BYTES}" \
      --small-files "${SMALL_FILES}" \
      --small-bytes "${SMALL_BYTES}" \
      --injection-delay-ms "${INJECTION_DELAY_MS}" \
      --overlap-min-percent "${OVERLAP_MIN_PERCENT}" \
      --minimum-tail-large-operations "${MINIMUM_TAIL_LARGE_OPERATIONS}" \
      --timeout-seconds "${TIMEOUT_SECONDS}"
    workload_rc=$?
    set -e

    fod_test_cleanup

    set +e
    "${PYTHON_BIN}" "${WORKLOAD}" annotate \
      --input "${output}" \
      --mount-log "${mount_log}" \
      --expected-limit "${limit}" \
      --minimum-peak-queued "${MINIMUM_PEAK_QUEUED}" \
      --minimum-baseline-active "${MINIMUM_BASELINE_ACTIVE}"
    annotate_rc=$?
    set -e

    run_files+=("${output}")
    unset FOD_TEST_LOG_ARCHIVE

    if (( workload_rc != 0 || annotate_rc != 0 )); then
      echo "FIFO FUSE fairness run recorded errors: limit=${limit} run=${run_index} workload_rc=${workload_rc} annotate_rc=${annotate_rc}" >&2
    fi
  done
done

"${PYTHON_BIN}" "${WORKLOAD}" summarize \
  --output-json "${ARTIFACT_DIR}/fifo-fuse-fairness-summary.json" \
  --output-md "${ARTIFACT_DIR}/fifo-fuse-fairness-summary.md" \
  "${run_files[@]}"

echo
echo "FIFO FUSE fairness artifacts: ${ARTIFACT_DIR}"
