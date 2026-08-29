#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Record an existing fod-rust-fuse process for a fixed interval. A fixed
# duration avoids incomplete perf.data files caused by an unclean manual stop.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PID="${1:-}"
LABEL="${2:-fod}"
DURATION_SECONDS="${3:-60}"
FREQ="${FOD_PERF_FREQ:-199}"
CALL_GRAPH="${FOD_PERF_CALL_GRAPH:-dwarf}"
HEAD_SHORT="$(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
OUT_DIR="${FOD_PERF_OUTPUT_DIR:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-perf-${RUN_ID}}"

if [[ -z "${PID}" || ! "${PID}" =~ ^[0-9]+$ ]]; then
    echo "Usage: $0 PID [LABEL] [SECONDS]" >&2
    exit 2
fi
if ! [[ "${DURATION_SECONDS}" =~ ^[1-9][0-9]*$ ]]; then
    echo "SECONDS must be a positive integer, got: ${DURATION_SECONDS}" >&2
    exit 2
fi
kill -0 "${PID}" 2>/dev/null || {
    echo "PID is not running: ${PID}" >&2
    exit 2
}
command -v perf >/dev/null 2>&1 || {
    echo "perf is not installed" >&2
    exit 2
}

mkdir -p "${OUT_DIR}"
DATA="${OUT_DIR}/perf-${LABEL}.data"
REPORT="${OUT_DIR}/perf-${LABEL}-report.txt"

echo "pid=${PID}"
echo "label=${LABEL}"
echo "seconds=${DURATION_SECONDS}"
echo "frequency=${FREQ}"
echo "call_graph=${CALL_GRAPH}"
echo "data=${DATA}"

sudo perf record \
    -F "${FREQ}" \
    -g \
    --call-graph "${CALL_GRAPH}" \
    -p "${PID}" \
    -o "${DATA}" \
    -- sleep "${DURATION_SECONDS}"

sudo perf report \
    --stdio \
    --sort comm,dso,symbol \
    -i "${DATA}" \
    >"${REPORT}"

echo "report=${REPORT}"
