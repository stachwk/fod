#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-throughput

pattern_file=""

cleanup_local() {
  # Usuwa plik tymczasowy z danymi testowymi.
  if [[ -n "${pattern_file:-}" ]]; then
    rm -f "${pattern_file}" 2>/dev/null || true
  fi

  # Sprzata mountpoint i proces FOD.
  fod_test_cleanup
}

trap cleanup_local EXIT

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

file="${MOUNTPOINT}/throughput.bin"
block_size="${THROUGHPUT_BLOCK_SIZE:-1M}"
count="${THROUGHPUT_COUNT:-1}"
sync_mode="${THROUGHPUT_SYNC:-0}"
throughput_source="${THROUGHPUT_SOURCE:-zero}"

block_size_to_bytes() {
  local value="$1"

  case "${value}" in
    *K|*k) echo $(( ${value%[Kk]} * 1024 )) ;;
    *M|*m) echo $(( ${value%[Mm]} * 1024 * 1024 )) ;;
    *G|*g) echo $(( ${value%[Gg]} * 1024 * 1024 * 1024 )) ;;
    *) echo "${value}" ;;
  esac
}

prepare_input_file() {
  local source_mode="$1"
  local expected_size="$2"
  local block_size_value="$3"
  local count_value="$4"

  case "${source_mode}" in
    zero)
      # Stare zachowanie: testuje sciezke zer/sparse.
      echo "/dev/zero"
      ;;

    pattern)
      # Tworzy niezerowy, powtarzalny wzorzec poza pomiarem czasu.
      # Dzieki temu test mierzy FOD, a nie generator danych.
      pattern_file="$(mktemp "${TMPDIR:-/tmp}/fod-throughput-pattern.XXXXXX")"

      dd if=/dev/zero bs="${block_size_value}" count="${count_value}" status=none \
        | tr '\000' '\001' > "${pattern_file}"

      echo "${pattern_file}"
      ;;

    random)
      # Tworzy losowy plik poza pomiarem czasu.
      # Uwaga: dla duzych testow przygotowanie moze chwile trwac.
      pattern_file="$(mktemp "${TMPDIR:-/tmp}/fod-throughput-random.XXXXXX")"

      dd if=/dev/urandom of="${pattern_file}" bs="${block_size_value}" count="${count_value}" status=none

      echo "${pattern_file}"
      ;;

    file:*)
      # Uzywa wskazanego pliku jako zrodla danych.
      local source_file="${source_mode#file:}"

      if [[ ! -r "${source_file}" ]]; then
        echo "Input file is not readable: ${source_file}" >&2
        exit 1
      fi

      local source_size
      source_size="$(fod_stat "${source_file}" '%s' 2>/dev/null || stat -c '%s' "${source_file}")"

      if (( source_size < expected_size )); then
        echo "Input file is too small: ${source_file}, size=${source_size}, expected=${expected_size}" >&2
        exit 1
      fi

      echo "${source_file}"
      ;;

    *)
      echo "Unsupported THROUGHPUT_SOURCE=${source_mode}" >&2
      echo "Supported: zero, pattern, random, file:/path/to/file" >&2
      exit 1
      ;;
  esac
}

run_dd_write() {
  local input_file_value="$1"
  local output_file_value="$2"
  local block_size_value="$3"
  local count_value="$4"
  local sync_value="$5"

  # Opcjonalny strace pozwala sprawdzic, czy czas siedzi w write(), close() albo fsync().
  # Uzycie:
  #   THROUGHPUT_STRACE=1 make test-throughput
  if [[ "${THROUGHPUT_STRACE:-0}" =~ ^(1|true|True|yes|on)$ ]]; then
    local trace_file="${THROUGHPUT_STRACE_FILE:-/tmp/fod-throughput-dd.strace}"

    if ! command -v strace >/dev/null 2>&1; then
      echo "THROUGHPUT_STRACE=1 requested, but strace is not installed or not in PATH" >&2
      exit 1
    fi

    rm -f "${trace_file}" "${trace_file}".* 2>/dev/null || true

    if [[ "${sync_value}" =~ ^(0|false|False|no|off)$ ]]; then
      strace -ff -ttt -T \
        -e trace=write,close,fsync,fdatasync \
        -o "${trace_file}" \
        dd if="${input_file_value}" of="${output_file_value}" bs="${block_size_value}" count="${count_value}" status=none
    else
      strace -ff -ttt -T \
        -e trace=write,close,fsync,fdatasync \
        -o "${trace_file}" \
        dd if="${input_file_value}" of="${output_file_value}" bs="${block_size_value}" count="${count_value}" conv=fsync status=none
    fi

    echo "Strace file: ${trace_file}.*"    echo "Strace file: ${trace_file}.*"

    echo "Strace syscall summary:"
    awk '
      match($0, /([a-z_]+)\(.*<([0-9.]+)>/, m) {
        op=m[1]
        t=m[2]
        cnt[op]++
        sum[op]+=t
        if (t > max[op]) max[op]=t
      }
      END {
        for (op in cnt) {
          printf "%-12s count=%-8d sum=%10.6f max=%10.6f avg=%10.6f\n", op, cnt[op], sum[op], max[op], sum[op]/cnt[op]
        }
      }
    ' "${trace_file}".*

  else
    if [[ "${sync_value}" =~ ^(0|false|False|no|off)$ ]]; then
      dd if="${input_file_value}" of="${output_file_value}" bs="${block_size_value}" count="${count_value}" status=none
    else
      dd if="${input_file_value}" of="${output_file_value}" bs="${block_size_value}" count="${count_value}" conv=fsync status=none
    fi
  fi
}

block_bytes="$(block_size_to_bytes "${block_size}")"
expected_size=$((count * block_bytes))
input_file="$(prepare_input_file "${throughput_source}" "${expected_size}" "${block_size}" "${count}")"

echo "Throughput source: ${throughput_source}"
echo "Throughput block_size=${block_size} count=${count} expected_size=${expected_size}"

start_ns="$(date +%s%N)"
run_dd_write "${input_file}" "${file}" "${block_size}" "${count}" "${sync_mode}"
end_ns="$(date +%s%N)"

actual_size=""
for _ in $(seq 1 50); do
  actual_size="$(fod_stat "${file}" '%s' 2>/dev/null || echo 0)"

  if [[ "${actual_size}" == "${expected_size}" ]]; then
    break
  fi

  sleep 0.1
done

fod_assert_eq "${actual_size}" "${expected_size}" "throughput file size"

elapsed_ns=$((end_ns - start_ns))
if (( elapsed_ns <= 0 )); then
  echo "Invalid elapsed time"
  exit 1
fi

elapsed_s="$(awk "BEGIN { printf \"%.3f\", ${elapsed_ns} / 1000000000 }")"
throughput_mb_s="$(awk "BEGIN { printf \"%.2f\", ${expected_size} / 1024 / 1024 / (${elapsed_ns} / 1000000000) }")"

echo "OK throughput/write ${expected_size} bytes in ${elapsed_s}s (${throughput_mb_s} MiB/s)"

if [[ "${sync_mode}" =~ ^(0|false|False|no|off)$ ]]; then
  echo "Tip: set THROUGHPUT_SYNC=1 to force fsync-backed throughput measurement."
fi
