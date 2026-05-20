#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"

fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-atime-bench
trap fod_test_cleanup EXIT

policy="${FOD_ATIME_POLICY:-default}"
kind="${ATIME_BENCH_KIND:-file}"
iterations="${ATIME_BENCH_ITERATIONS:-50}"
entries="${ATIME_BENCH_ENTRIES:-64}"

fod_test_init_schema
fod_test_start_mount "${MOUNTPOINT}"

start_ns="$(date +%s%N)"

case "${kind}" in
  file)
    file="${MOUNTPOINT}/bench.txt"
    printf '%s\n' "atime benchmark" > "${file}"
    for _ in $(seq 1 "${iterations}"); do
      cat "${file}" >/dev/null
    done
    ;;
  dir)
    dir="${MOUNTPOINT}/bench-dir"
    mkdir -p "${dir}"
    for i in $(seq 1 "${entries}"); do
      printf '%s\n' "${i}" > "${dir}/entry-${i}.txt"
    done
    for _ in $(seq 1 "${iterations}"); do
      ls -1 "${dir}" >/dev/null
    done
    ;;
  *)
    echo "unsupported ATIME bench kind: ${kind}"
    exit 1
    ;;
esac

end_ns="$(date +%s%N)"
elapsed_ns=$((end_ns - start_ns))
elapsed_ms=$((elapsed_ns / 1000000))
echo "OK atime-benchmark/${kind}/${policy} elapsed_ms=${elapsed_ms} iterations=${iterations}"
