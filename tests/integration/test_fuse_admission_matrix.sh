#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

PYTHON_BIN="${PYTHON:-python3}"
WORKLOAD="${ROOT}/tests/integration/test_fifo_fuse_fairness.py"
MATRIX_TOOL="${ROOT}/tests/integration/test_fuse_admission_matrix.py"

THREADS_TEXT="${FUSE_ADMISSION_MATRIX_EVENT_THREADS:-2 4 8 16}"
LIMITS_TEXT="${FUSE_ADMISSION_MATRIX_LIMITS:-0 1 2 4 8}"
REPEAT="${FUSE_ADMISSION_MATRIX_REPEAT:-3}"
CLONE_FD="${FUSE_ADMISSION_MATRIX_CLONE_FD:-0}"
LARGE_WORKERS="${FUSE_ADMISSION_MATRIX_LARGE_WORKERS:-16}"
LARGE_ITERATIONS="${FUSE_ADMISSION_MATRIX_LARGE_ITERATIONS:-8}"
LARGE_WRITE_BYTES="${FUSE_ADMISSION_MATRIX_LARGE_WRITE_BYTES:-262144}"
SMALL_FILES="${FUSE_ADMISSION_MATRIX_SMALL_FILES:-32}"
SMALL_BYTES="${FUSE_ADMISSION_MATRIX_SMALL_BYTES:-4096}"
INJECTION_DELAY_MS="${FUSE_ADMISSION_MATRIX_INJECTION_DELAY_MS:-5}"
OVERLAP_MIN_PERCENT="${FUSE_ADMISSION_MATRIX_OVERLAP_MIN_PERCENT:-90}"
MINIMUM_TAIL_LARGE_OPERATIONS="${FUSE_ADMISSION_MATRIX_MINIMUM_TAIL_LARGE_OPERATIONS:-16}"
MINIMUM_PEAK_QUEUED="${FUSE_ADMISSION_MATRIX_MINIMUM_PEAK_QUEUED:-2}"
MINIMUM_BASELINE_ACTIVE="${FUSE_ADMISSION_MATRIX_MINIMUM_BASELINE_ACTIVE:-2}"
TIMEOUT_SECONDS="${FUSE_ADMISSION_MATRIX_TIMEOUT_SECONDS:-240}"
RUN_ID="${FUSE_ADMISSION_MATRIX_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
COMMIT_SHORT="$(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
ARTIFACT_DIR="${FUSE_ADMISSION_MATRIX_ARTIFACT_DIR:-/tmp/fod-fuse-admission-matrix/${COMMIT_SHORT}-${RUN_ID}}"

if ! command -v "${PYTHON_BIN}" >/dev/null 2>&1; then
  echo "Python is required for FUSE admission matrix testing" >&2
  exit 1
fi

case "${REPEAT}" in
  ''|*[!0-9]*)
    echo "FUSE_ADMISSION_MATRIX_REPEAT must be a positive integer: ${REPEAT}" >&2
    exit 1
    ;;
esac
if (( REPEAT < 1 )); then
  echo "FUSE_ADMISSION_MATRIX_REPEAT must be at least 1" >&2
  exit 1
fi

read -r -a event_threads <<<"${THREADS_TEXT}"
read -r -a limits <<<"${LIMITS_TEXT}"

if (( ${#event_threads[@]} == 0 || ${#limits[@]} == 0 )); then
  echo "FUSE admission matrix axes must not be empty" >&2
  exit 1
fi

for threads in "${event_threads[@]}"; do
  case "${threads}" in
    ''|*[!0-9]*)
      echo "Invalid event-thread value: ${threads}" >&2
      exit 1
      ;;
  esac
  if (( threads < 1 || threads > 256 )); then
    echo "Event-thread value outside 1..256: ${threads}" >&2
    exit 1
  fi
done

for limit in "${limits[@]}"; do
  case "${limit}" in
    ''|*[!0-9]*)
      echo "Invalid admission-limit value: ${limit}" >&2
      exit 1
      ;;
  esac
done

if [[ "${CLONE_FD}" != "0" && "${CLONE_FD}" != "1" ]]; then
  echo "FUSE_ADMISSION_MATRIX_CLONE_FD must be 0 or 1" >&2
  exit 1
fi

mkdir -p "${ARTIFACT_DIR}"
fod_test_setup "${ROOT}"
fod_test_init_schema

cleanup() {
  fod_test_cleanup
}
trap cleanup EXIT

cells=()
for threads in "${event_threads[@]}"; do
  for limit in "${limits[@]}"; do
    cells+=("${threads}:${limit}")
  done
done

run_files=()
cell_count="${#cells[@]}"
stride=7

for run_index in $(seq 1 "${REPEAT}"); do
  offset=$(( ((run_index - 1) * stride) % cell_count ))
  echo "FUSE admission matrix repeat ${run_index}/${REPEAT}; offset=${offset}"

  for step in $(seq 0 $((cell_count - 1))); do
    cell_index=$(( (offset + step) % cell_count ))
    cell="${cells[cell_index]}"
    threads="${cell%%:*}"
    limit="${cell##*:}"

    output="${ARTIFACT_DIR}/threads-${threads}-limit-${limit}-run-${run_index}.json"
    mount_log="${ARTIFACT_DIR}/threads-${threads}-limit-${limit}-run-${run_index}-mount.log"

    export FOD_TASK_READ_ACTIVE_LIMIT=0
    export FOD_TASK_WRITE_ACTIVE_LIMIT="${limit}"
    export FOD_FUSE_EVENT_THREADS="${threads}"
    export FOD_FUSE_CLONE_FD="${CLONE_FD}"
    export FOD_TASK_OBSERVABILITY_INTERVAL_MS=100
    export FOD_FOPEN_DIRECT_IO=1
    export FOD_PROFILE_IO=1
    export FOD_ENABLE_EXTENTS="${FUSE_ADMISSION_MATRIX_ENABLE_EXTENTS:-1}"
    export FOD_LOG_LEVEL="${FOD_LOG_LEVEL:-info}"
    export FOD_TEST_LOG_ARCHIVE="${mount_log}"

    fod_test_make_mountpoint \
      "/tmp/fod-admission-matrix-t${threads}-l${limit}-r${run_index}"
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

    annotate_args=(
      annotate
      --input "${output}"
      --mount-log "${mount_log}"
      --event-threads "${threads}"
      --limit "${limit}"
      --minimum-peak-queued "${MINIMUM_PEAK_QUEUED}"
      --minimum-baseline-active "${MINIMUM_BASELINE_ACTIVE}"
    )
    if [[ "${CLONE_FD}" == "1" ]]; then
      annotate_args+=(--clone-fd)
    fi

    set +e
    "${PYTHON_BIN}" "${MATRIX_TOOL}" "${annotate_args[@]}"
    annotate_rc=$?
    set -e

    run_files+=("${output}")
    unset FOD_TEST_LOG_ARCHIVE

    if (( workload_rc != 0 || annotate_rc != 0 )); then
      echo \
        "FUSE admission matrix cell recorded errors: threads=${threads} limit=${limit} run=${run_index} workload_rc=${workload_rc} annotate_rc=${annotate_rc}" \
        >&2
    fi
  done
done

"${PYTHON_BIN}" "${MATRIX_TOOL}" summarize \
  --output-json "${ARTIFACT_DIR}/fuse-admission-matrix-summary.json" \
  --output-md "${ARTIFACT_DIR}/fuse-admission-matrix-summary.md" \
  --output-csv "${ARTIFACT_DIR}/fuse-admission-matrix-summary.csv" \
  "${run_files[@]}"

echo
echo "FUSE admission matrix artifacts: ${ARTIFACT_DIR}"
