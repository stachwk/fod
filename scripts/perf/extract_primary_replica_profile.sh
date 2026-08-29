#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Extract FOD internal observability from one primary/replica matrix artifact.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 MATRIX_ARTIFACT_DIR" >&2
    exit 2
fi

MATRIX_DIR="$1"
if [[ ! -d "${MATRIX_DIR}" ]]; then
    echo "Matrix artifact directory does not exist: ${MATRIX_DIR}" >&2
    exit 2
fi

echo "matrix_artifact_dir=${MATRIX_DIR}"

shopt -s nullglob
logs=("${MATRIX_DIR}"/*.log)
if ((${#logs[@]} == 0)); then
    echo "No per-block matrix logs found under ${MATRIX_DIR}" >&2
    exit 1
fi

for matrix_log in "${logs[@]}"; do
    block_size="$(basename "${matrix_log}" .log)"
    case_artifact="$(
        sed -n 's/^artifact_dir=//p' "${matrix_log}" | tail -n 1
    )"

    echo
    echo "=== block_size=${block_size} ==="
    echo "case_artifact_dir=${case_artifact:-missing}"

    if [[ -z "${case_artifact}" || ! -d "${case_artifact}" ]]; then
        echo "missing_case_artifact=1"
        continue
    fi

    for phase in primary-write primary-read replica-read; do
        phase_log="${case_artifact}/${phase}-mount.log"
        echo
        echo "--- phase=${phase} log=${phase_log} ---"

        if [[ ! -f "${phase_log}" ]]; then
            echo "missing_phase_log=1"
            continue
        fi

        selected="$(
            grep -E \
                'FOD boundary profile:|FOD PostgreSQL lane observability|FOD logical task observability:|FOD persist|FOD read|FOD write|operation_failures=' \
                "${phase_log}" || true
        )"

        if [[ -n "${selected}" ]]; then
            printf '%s\n' "${selected}"
        else
            echo "No selected observability lines found; tail follows."
            tail -n 40 "${phase_log}"
        fi
    done
done
