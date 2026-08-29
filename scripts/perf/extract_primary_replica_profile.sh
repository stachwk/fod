#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Extract FOD internal observability from one primary/replica matrix artifact.
# Default output is compact and keeps only final per-phase counters. Set
# FOD_PROFILE_EXTRACT_MODE=full to retain the historical verbose sampler dump.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 MATRIX_ARTIFACT_DIR" >&2
    exit 2
fi

MATRIX_DIR="$1"
MODE="${FOD_PROFILE_EXTRACT_MODE:-compact}"

if [[ ! -d "${MATRIX_DIR}" ]]; then
    echo "Matrix artifact directory does not exist: ${MATRIX_DIR}" >&2
    exit 2
fi

case "${MODE}" in
    compact|full) ;;
    *)
        echo "FOD_PROFILE_EXTRACT_MODE must be compact or full, got: ${MODE}" >&2
        exit 2
        ;;
esac

field_value() {
    local line="$1"
    local key="$2"
    local value
    value="$(printf '%s\n' "${line}" | tr ' ' '\n' | sed -n "s/^${key}=//p" | tail -n 1)"
    printf '%s' "${value:-0}"
}

print_compact_phase() {
    local phase="$1"
    local phase_log="$2"
    local logical_line lane_line

    logical_line="$(grep 'FOD logical task observability: stage=shutdown' "${phase_log}" | tail -n 1 || true)"
    lane_line="$(grep 'FOD PostgreSQL lane observability: stage=post-mount lane=' "${phase_log}" | tail -n 1 || true)"

    if [[ -z "${logical_line}" && -z "${lane_line}" ]]; then
        echo "phase=${phase} no_final_observability=1"
        return
    fi

    printf 'phase=%s' "${phase}"
    printf ' completed_bytes_per_second=%s' "$(field_value "${logical_line}" completed_bytes_per_second)"
    printf ' elapsed_micros=%s' "$(field_value "${logical_line}" elapsed_micros)"
    printf ' admitted_tasks=%s' "$(field_value "${logical_line}" admitted_tasks)"
    printf ' operation_count=%s' "$(field_value "${lane_line}" operation_count)"
    printf ' operation_failures=%s' "$(field_value "${lane_line}" operation_failures)"
    printf ' operation_micros_total=%s' "$(field_value "${lane_line}" operation_micros_total)"
    printf ' operation_micros_max=%s' "$(field_value "${lane_line}" operation_micros_max)"
    printf ' acquisition_wait_micros_total=%s' "$(field_value "${lane_line}" acquisition_wait_micros_total)"
    printf ' acquisition_wait_micros_max=%s' "$(field_value "${lane_line}" acquisition_wait_micros_max)"
    printf ' persist_operation_count=%s' "$(field_value "${lane_line}" persist_operation_count)"
    printf ' persist_input_bytes_total=%s' "$(field_value "${lane_line}" persist_input_bytes_total)"
    printf ' persist_input_bytes_max=%s' "$(field_value "${lane_line}" persist_input_bytes_max)"
    printf ' persist_micros_total=%s' "$(field_value "${lane_line}" persist_micros_total)"
    printf ' persist_micros_max=%s' "$(field_value "${lane_line}" persist_micros_max)"
    printf ' persist_transaction_micros_total=%s' "$(field_value "${lane_line}" persist_transaction_micros_total)"
    printf ' persist_copy_stage_micros_total=%s' "$(field_value "${lane_line}" persist_copy_stage_micros_total)"
    printf ' persist_data_blocks_merge_micros_total=%s' "$(field_value "${lane_line}" persist_data_blocks_merge_micros_total)"
    printf ' payload_peak_in_flight_bytes=%s' "$(field_value "${lane_line}" payload_peak_in_flight_bytes)"
    printf ' write_transaction_backpressure_events=%s' "$(field_value "${lane_line}" write_transaction_backpressure_events)"
    printf '\n'
}

echo "matrix_artifact_dir=${MATRIX_DIR}"
echo "extract_mode=${MODE}"

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

        if [[ ! -f "${phase_log}" ]]; then
            echo "phase=${phase} missing_phase_log=1 log=${phase_log}"
            continue
        fi

        if [[ "${MODE}" == "compact" ]]; then
            print_compact_phase "${phase}" "${phase_log}"
            continue
        fi

        echo
        echo "--- phase=${phase} log=${phase_log} ---"
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
