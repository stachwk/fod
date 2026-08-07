#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

PYTHON_BIN="${PYTHON:-python3}"
WORKLOAD="${ROOT}/tests/integration/test_fifo_fuse_fairness.py"
MATRIX_TOOL="${ROOT}/tests/integration/test_fuse_admission_matrix.py"
CONFIRM_TOOL="${ROOT}/tests/integration/test_fuse_admission_confirmation.py"

CANDIDATES_TEXT="${FUSE_ADMISSION_CONFIRM_CANDIDATES:-8:4 16:4 4:8 8:0}"
REPEAT="${FUSE_ADMISSION_CONFIRM_REPEAT:-10}"
FIO_REPEAT="${FUSE_ADMISSION_CONFIRM_FIO_REPEAT:-3}"
CLONE_FD="${FUSE_ADMISSION_CONFIRM_CLONE_FD:-0}"
LARGE_WORKERS="${FUSE_ADMISSION_CONFIRM_LARGE_WORKERS:-16}"
LARGE_ITERATIONS="${FUSE_ADMISSION_CONFIRM_LARGE_ITERATIONS:-8}"
LARGE_WRITE_BYTES="${FUSE_ADMISSION_CONFIRM_LARGE_WRITE_BYTES:-262144}"
SMALL_FILES="${FUSE_ADMISSION_CONFIRM_SMALL_FILES:-32}"
SMALL_BYTES="${FUSE_ADMISSION_CONFIRM_SMALL_BYTES:-4096}"
INJECTION_DELAY_MS="${FUSE_ADMISSION_CONFIRM_INJECTION_DELAY_MS:-5}"
OVERLAP_MIN_PERCENT="${FUSE_ADMISSION_CONFIRM_OVERLAP_MIN_PERCENT:-90}"
MINIMUM_TAIL_LARGE_OPERATIONS="${FUSE_ADMISSION_CONFIRM_MINIMUM_TAIL_LARGE_OPERATIONS:-16}"
MINIMUM_PEAK_QUEUED="${FUSE_ADMISSION_CONFIRM_MINIMUM_PEAK_QUEUED:-2}"
MINIMUM_BASELINE_ACTIVE="${FUSE_ADMISSION_CONFIRM_MINIMUM_BASELINE_ACTIVE:-2}"
TIMEOUT_SECONDS="${FUSE_ADMISSION_CONFIRM_TIMEOUT_SECONDS:-240}"
MINIMUM_THROUGHPUT_GAIN_PERCENT="${FUSE_ADMISSION_CONFIRM_MINIMUM_THROUGHPUT_GAIN_PERCENT:-10}"
MINIMUM_LATENCY_IMPROVEMENT_PERCENT="${FUSE_ADMISSION_CONFIRM_MINIMUM_LATENCY_IMPROVEMENT_PERCENT:-10}"
MAXIMUM_STABILITY_CV_PERCENT="${FUSE_ADMISSION_CONFIRM_MAXIMUM_STABILITY_CV_PERCENT:-25}"
REQUIRE_CONFIRMED="${FUSE_ADMISSION_CONFIRM_REQUIRE_CONFIRMED:-0}"
RUN_ID="${FUSE_ADMISSION_CONFIRM_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
COMMIT_SHORT="$(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
ARTIFACT_DIR="${FUSE_ADMISSION_CONFIRM_ARTIFACT_DIR:-/tmp/fod-fuse-admission-confirmation/${COMMIT_SHORT}-${RUN_ID}}"
FIO_DIR="${ARTIFACT_DIR}/fio-8-4"

for value in "${REPEAT}" "${FIO_REPEAT}"; do
  case "${value}" in
    ''|*[!0-9]*)
      echo "repeat values must be positive integers" >&2
      exit 1
      ;;
  esac
  if (( value < 1 )); then
    echo "repeat values must be at least 1" >&2
    exit 1
  fi
done

if [[ "${CLONE_FD}" != "0" && "${CLONE_FD}" != "1" ]]; then
  echo "FUSE_ADMISSION_CONFIRM_CLONE_FD must be 0 or 1" >&2
  exit 1
fi
if [[ "${REQUIRE_CONFIRMED}" != "0" && "${REQUIRE_CONFIRMED}" != "1" ]]; then
  echo "FUSE_ADMISSION_CONFIRM_REQUIRE_CONFIRMED must be 0 or 1" >&2
  exit 1
fi

read -r -a candidates <<<"${CANDIDATES_TEXT}"
expected_candidates=("8:4" "16:4" "4:8" "8:0")
if (( ${#candidates[@]} != ${#expected_candidates[@]} )); then
  echo "confirmation suite requires exactly four candidates" >&2
  exit 1
fi
for expected in "${expected_candidates[@]}"; do
  found=0
  for candidate in "${candidates[@]}"; do
    if [[ "${candidate}" == "${expected}" ]]; then
      found=1
      break
    fi
  done
  if (( found == 0 )); then
    echo "missing required candidate ${expected}" >&2
    exit 1
  fi
done

mkdir -p "${ARTIFACT_DIR}" "${FIO_DIR}"
fod_test_setup "${ROOT}"
fod_test_init_schema

cleanup() {
  fod_test_cleanup
}
trap cleanup EXIT

run_files=()
candidate_count="${#candidates[@]}"

for run_index in $(seq 1 "${REPEAT}"); do
  offset=$(( (run_index - 1) % candidate_count ))
  echo "FUSE admission confirmation repeat ${run_index}/${REPEAT}; offset=${offset}"

  for step in $(seq 0 $((candidate_count - 1))); do
    candidate_index=$(( (offset + step) % candidate_count ))
    candidate="${candidates[candidate_index]}"
    threads="${candidate%%:*}"
    limit="${candidate##*:}"

    output="${ARTIFACT_DIR}/threads-${threads}-limit-${limit}-run-${run_index}.json"
    mount_log="${ARTIFACT_DIR}/threads-${threads}-limit-${limit}-run-${run_index}-mount.log"

    export FOD_TASK_READ_ACTIVE_LIMIT=0
    export FOD_TASK_WRITE_ACTIVE_LIMIT="${limit}"
    export FOD_FUSE_EVENT_THREADS="${threads}"
    export FOD_FUSE_CLONE_FD="${CLONE_FD}"
    export FOD_TASK_OBSERVABILITY_INTERVAL_MS=100
    export FOD_FOPEN_DIRECT_IO=1
    export FOD_PROFILE_IO=1
    export FOD_ENABLE_EXTENTS="${FUSE_ADMISSION_CONFIRM_ENABLE_EXTENTS:-1}"
    export FOD_LOG_LEVEL="${FOD_LOG_LEVEL:-info}"
    export FOD_TEST_LOG_ARCHIVE="${mount_log}"

    fod_test_make_mountpoint \
      "/tmp/fod-admission-confirm-t${threads}-l${limit}-r${run_index}"
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
        "confirmation candidate recorded errors: threads=${threads} limit=${limit} run=${run_index} workload_rc=${workload_rc} annotate_rc=${annotate_rc}" \
        >&2
    fi
  done
done

"${PYTHON_BIN}" "${MATRIX_TOOL}" summarize \
  --output-json "${ARTIFACT_DIR}/confirmation-matrix-summary.json" \
  --output-md "${ARTIFACT_DIR}/confirmation-matrix-summary.md" \
  --output-csv "${ARTIFACT_DIR}/confirmation-matrix-summary.csv" \
  "${run_files[@]}"

echo
echo "Running fio/strace confirmation for threads=8 limit=4"

for profile_run in $(seq 1 "${FIO_REPEAT}"); do
  common_env=(
    FOD_FUSE_EVENT_THREADS=8
    FOD_FUSE_CLONE_FD=0
    FOD_TASK_READ_ACTIVE_LIMIT=0
    FOD_TASK_WRITE_ACTIVE_LIMIT=4
    FOD_PROFILE_IO=1
  )

  env "${common_env[@]}" \
    make --no-print-directory test-fio-sequential-io \
    2>&1 | tee "${FIO_DIR}/fio-sequential-run-${profile_run}.log"

  env "${common_env[@]}" \
    make --no-print-directory test-fio-sequential-io-strace \
    2>&1 | tee "${FIO_DIR}/fio-sequential-strace-run-${profile_run}.log"
done

confirm_args=(
  --matrix-summary "${ARTIFACT_DIR}/confirmation-matrix-summary.json"
  --output-json "${ARTIFACT_DIR}/admission-confirmation-summary.json"
  --output-md "${ARTIFACT_DIR}/admission-confirmation-summary.md"
  --fio-artifact-dir "${FIO_DIR}"
  --expected-runs "${REPEAT}"
  --fio-repeat "${FIO_REPEAT}"
  --minimum-throughput-gain-percent "${MINIMUM_THROUGHPUT_GAIN_PERCENT}"
  --minimum-latency-improvement-percent "${MINIMUM_LATENCY_IMPROVEMENT_PERCENT}"
  --maximum-stability-cv-percent "${MAXIMUM_STABILITY_CV_PERCENT}"
)
if [[ "${REQUIRE_CONFIRMED}" == "1" ]]; then
  confirm_args+=(--require-confirmed)
fi

"${PYTHON_BIN}" "${CONFIRM_TOOL}" "${confirm_args[@]}"

echo
echo "FUSE admission confirmation artifacts: ${ARTIFACT_DIR}"
