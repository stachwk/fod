#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

if ! command -v fio >/dev/null 2>&1; then
  echo "SKIP fio/sequential (fio is not installed)"
  exit 0
fi
if [[ "${FOD_STRACE:-0}" =~ ^(1|true|True|yes|on)$ ]] && ! command -v strace >/dev/null 2>&1; then
  echo "SKIP fio/sequential strace (strace is not installed)"
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
  local block_size="${FIO_BLOCK_SIZE:-4k}"
  local file_size="${FIO_FILE_SIZE:-64k}"
  local file_size_slug="${file_size//[^[:alnum:]]/}"
  local expected_size
  expected_size="$(size_to_bytes "${file_size}")"

  export FOD_ENABLE_EXTENTS="${enable_extents}"
  export FOD_LOG_LEVEL="${FOD_LOG_LEVEL:-debug}"
  if [[ "${FOD_STRACE:-0}" =~ ^(1|true|True|yes|on)$ ]]; then
    export FOD_STRACE_LABEL="${label}"
    export FOD_STRACE_SUMMARY_FILE="$(mktemp "/tmp/fod-fio-${label}.XXXXXX.strace")"
  else
    unset FOD_STRACE_LABEL FOD_STRACE_SUMMARY_FILE || true
  fi

  fod_test_make_mountpoint "/tmp/fod-fio-${label}"
  fod_test_init_schema
  fod_test_start_mount "${MOUNTPOINT}"

  local file="${MOUNTPOINT}/fio-${label}-${file_size_slug}-${BASHPID}.bin"

  fio \
    --name="seq-write-${label}" \
    --filename="${file}" \
    --ioengine=sync \
    --rw=write \
    --bs="${block_size}" \
    --size="${file_size}" \
    --numjobs=1 \
    --group_reporting=1 \
    --direct=0 \
    --fsync_on_close=1 \
    --buffer_pattern=0x5a \
    --output-format=normal

  fio \
    --name="seq-read-${label}" \
    --filename="${file}" \
    --ioengine=sync \
    --rw=read \
    --bs="${block_size}" \
    --size="${file_size}" \
    --numjobs=1 \
    --group_reporting=1 \
    --direct=0 \
    --output-format=normal

  actual_size="$(fod_stat "${file}" '%s')"
  fod_assert_eq "${actual_size}" "${expected_size}" "fio ${label} file size"

  if [[ "${enable_extents}" == "1" ]]; then
    fod_assert_contains "${LOG_FILE}" "enable_extents=true"
    fod_assert_contains "${LOG_FILE}" "FOD sequential segment state entered"
    fod_assert_contains "${LOG_FILE}" "write_state_mode=sequential_segment"
    fod_assert_contains "${LOG_FILE}" "persist_write_class=new_object_sequential"
    fod_assert_contains "${LOG_FILE}" "FOD direct segment persistence"
  else
    if grep -Fq "FOD extent PoC execution" "${LOG_FILE}"; then
      echo "unexpected extent PoC log in block-storage mode"
      return 1
    fi
    if grep -Fq "FOD sequential segment state entered" "${LOG_FILE}"; then
      echo "unexpected sequential segment state in block-storage mode"
      return 1
    fi
    if grep -Fq "FOD direct segment persistence" "${LOG_FILE}"; then
      echo "unexpected direct segment persistence in block-storage mode"
      return 1
    fi
    fod_assert_contains "${LOG_FILE}" "FOD write_state_mode=block"
    fod_assert_contains "${LOG_FILE}" "persist_write_class=existing_object_patch"
  fi

  echo "OK fio/sequential ${label} extents=${enable_extents} size=${expected_size} block_size=${block_size}"
  rm -f "${file}"
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
