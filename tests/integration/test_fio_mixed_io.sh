#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

if ! command -v fio >/dev/null 2>&1; then
  echo "SKIP fio/mixed (fio is not installed)"
  exit 0
fi

fod_test_setup "${ROOT}"

size_to_bytes() {
  local value="$1"
  case "${value}" in
    *K|*k) echo $(( ${value%[Kk]} * 1024 )) ;;
    *M|*m) echo $(( ${value%[Mm]} * 1024 * 1024 )) ;;
    *G|*g) echo $(( ${value%[Gg]} * 1024 * 1024 * 1024 )) ;;
    *) echo "${value}" ;;
  esac
}

cleanup() {
  fod_test_cleanup
}

run_case() {
  local label="$1"
  local enable_extents="$2"
  local rw_mode="${FIO_RW_MODE:-rw}"
  local block_size="${FIO_BLOCK_SIZE:-4k}"
  local file_size="${FIO_FILE_SIZE:-4M}"
  local file_size_slug="${file_size//[^[:alnum:]]/}"
  local rw_mode_slug="${rw_mode//[^[:alnum:]]/}"
  local expected_size
  expected_size="$(size_to_bytes "${file_size}")"

  export FOD_ENABLE_EXTENTS="${enable_extents}"
  export FOD_LOG_LEVEL="${FOD_LOG_LEVEL:-debug}"

  fod_test_make_mountpoint "/tmp/fod-fio-mixed-${label}"
  fod_test_init_schema
  fod_test_start_mount "${MOUNTPOINT}"

  local file="${MOUNTPOINT}/fio-mixed-${label}-${rw_mode_slug}-${file_size_slug}.bin"
  local fio_args=(
    --name="mix-${label}"
    --filename="${file}"
    --ioengine=sync
    --rw="${rw_mode}"
    --bs="${block_size}"
    --size="${file_size}"
    --numjobs=1
    --group_reporting=1
    --direct=0
    --buffer_pattern=0x5a
    --output-format=normal
  )

  if [[ "${rw_mode}" == "randrw" ]]; then
    fio_args+=(--rwmixread="${FIO_RWMIXREAD:-50}")
  fi

  fio "${fio_args[@]}"

  local actual_size
  actual_size="$(fod_stat "${file}" '%s')"
  fod_assert_eq "${actual_size}" "${expected_size}" "fio ${label} mixed file size"

  if [[ "${enable_extents}" == "1" ]]; then
    fod_assert_contains "${LOG_FILE}" "enable_extents=true"
  else
    if grep -Fq "FOD extent PoC execution" "${LOG_FILE}"; then
      echo "unexpected extent PoC log in block-storage mode"
      return 1
    fi
  fi

  echo "OK fio/mixed ${label} rw=${rw_mode} extents=${enable_extents} size=${expected_size} block_size=${block_size}"
  fod_test_cleanup
}

trap cleanup EXIT

case "${FIO_CASES:-both}" in
  both)
    run_case block 0
    run_case extent 1
    ;;
  block)
    run_case block 0
    ;;
  extent)
    run_case extent 1
    ;;
  *)
    echo "Unsupported FIO_CASES=${FIO_CASES}; use both, block, or extent" >&2
    exit 2
    ;;
esac
