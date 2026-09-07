#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Extract FOD internal observability from one primary/replica matrix artifact.
# Default output is compact and keeps final per-phase counters plus a read-path
# attribution summary assembled from the FOD boundary profile. Set
# FOD_PROFILE_EXTRACT_MODE=full to retain the verbose sampler/profile dump.

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

profile_field_line() {
    local phase_log="$1"
    local key="$2"
    grep -E "(^|[[:space:]])${key}=" "${phase_log}" | tail -n 1 || true
}

profile_named_line() {
    local phase_log="$1"
    local record="$2"
    local name="$3"
    grep "${record} name=${name} " "${phase_log}" | tail -n 1 || true
}

profile_named_line_any() {
    local phase_log="$1"
    local record="$2"
    shift 2

    local name line
    for name in "$@"; do
        line="$(profile_named_line "${phase_log}" "${record}" "${name}")"
        if [[ -n "${line}" ]]; then
            printf '%s' "${line}"
            return 0
        fi
    done
    return 0
}

ratio() {
    local numerator="${1:-0}"
    local denominator="${2:-0}"
    awk -v n="${numerator}" -v d="${denominator}" 'BEGIN {
        if ((d + 0) == 0) {
            printf "0"
        } else {
            printf "%.6f", (n + 0) / (d + 0)
        }
    }'
}

nonnegative_difference() {
    local left="${1:-0}"
    local right="${2:-0}"
    awk -v a="${left}" -v b="${right}" 'BEGIN {
        value = (a + 0) - (b + 0)
        if (value < 0) value = 0
        printf "%.0f", value
    }'
}

print_compact_phase() {
    local phase="$1"
    local phase_log="$2"
    local logical_line lane_line
    local fuse_read_line read_block_map_line repo_fetch_line assemble_line reply_data_line
    local fetch_line decode_line metadata_line fetch_statement_name
    local admitted_tasks operation_count fetch_count fetch_rows fetch_bytes fetch_total_us
    local decode_total_us decode_rows decode_bytes metadata_count metadata_total_us

    logical_line="$(grep 'FOD logical task observability: stage=shutdown' "${phase_log}" | tail -n 1 || true)"
    lane_line="$(grep 'FOD PostgreSQL lane observability: stage=post-mount lane=' "${phase_log}" | tail -n 1 || true)"

    if [[ -z "${logical_line}" && -z "${lane_line}" ]]; then
        echo "phase=${phase} no_final_observability=1"
        return
    fi

    fuse_read_line="$(profile_field_line "${phase_log}" fuse_read_total_us)"
    read_block_map_line="$(profile_field_line "${phase_log}" read_block_map_us)"
    repo_fetch_line="$(profile_field_line "${phase_log}" repo_fetch_block_range_us)"
    assemble_line="$(profile_field_line "${phase_log}" assemble_read_slice_us)"
    reply_data_line="$(profile_field_line "${phase_log}" reply_data_us)"
    fetch_line="$(profile_named_line_any "${phase_log}" pg_prepared_statement \
        fod_fetch_block_range_with_size fod_fetch_block_range)"
    decode_line="$(profile_named_line_any "${phase_log}" pg_result_decode \
        fod_fetch_block_range_with_size fod_fetch_block_range)"
    metadata_line="$(profile_named_line "${phase_log}" pg_prepared_statement \
        fod_file_read_metadata)"

    admitted_tasks="$(field_value "${logical_line}" admitted_tasks)"
    operation_count="$(field_value "${lane_line}" operation_count)"
    fetch_statement_name="$(field_value "${fetch_line}" name)"
    fetch_count="$(field_value "${fetch_line}" count)"
    fetch_rows="$(field_value "${fetch_line}" result_rows)"
    fetch_bytes="$(field_value "${fetch_line}" result_bytes)"
    fetch_total_us="$(field_value "${fetch_line}" total_us)"
    decode_total_us="$(field_value "${decode_line}" total_us)"
    decode_rows="$(field_value "${decode_line}" result_rows)"
    decode_bytes="$(field_value "${decode_line}" result_bytes)"
    metadata_count="$(field_value "${metadata_line}" count)"
    metadata_total_us="$(field_value "${metadata_line}" total_us)"

    printf 'phase=%s' "${phase}"
    printf ' completed_bytes_per_second=%s' "$(field_value "${logical_line}" completed_bytes_per_second)"
    printf ' elapsed_micros=%s' "$(field_value "${logical_line}" elapsed_micros)"
    printf ' admitted_tasks=%s' "${admitted_tasks}"
    printf ' operation_count=%s' "${operation_count}"
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

    printf ' profile_attribution_available=%s' "$([[ -n "${fetch_line}" ]] && echo 1 || echo 0)"
    printf ' fetch_statement_name=%s' "${fetch_statement_name}"
    printf ' fuse_read_total_us=%s' "$(field_value "${fuse_read_line}" fuse_read_total_us)"
    printf ' read_block_map_us=%s' "$(field_value "${read_block_map_line}" read_block_map_us)"
    printf ' repo_fetch_block_range_us=%s' "$(field_value "${repo_fetch_line}" repo_fetch_block_range_us)"
    printf ' assemble_read_slice_us=%s' "$(field_value "${assemble_line}" assemble_read_slice_us)"
    printf ' reply_data_us=%s' "$(field_value "${reply_data_line}" reply_data_us)"
    printf ' fetch_block_range_calls=%s' "${fetch_count}"
    printf ' fetch_block_range_total_us=%s' "${fetch_total_us}"
    printf ' fetch_block_range_result_rows=%s' "${fetch_rows}"
    printf ' fetch_block_range_result_bytes=%s' "${fetch_bytes}"
    printf ' fetch_block_range_failures=%s' "$(field_value "${fetch_line}" failures)"
    printf ' fetch_block_range_decode_total_us=%s' "${decode_total_us}"
    printf ' fetch_block_range_decode_rows=%s' "${decode_rows}"
    printf ' fetch_block_range_decode_bytes=%s' "${decode_bytes}"
    printf ' fetch_block_range_decode_failures=%s' "$(field_value "${decode_line}" failures)"
    printf ' file_read_metadata_calls=%s' "${metadata_count}"
    printf ' file_read_metadata_total_us=%s' "${metadata_total_us}"

    printf ' pg_operations_per_callback=%s' "$(ratio "${operation_count}" "${admitted_tasks}")"
    printf ' fetch_calls_per_callback=%s' "$(ratio "${fetch_count}" "${admitted_tasks}")"
    printf ' fetch_rows_per_call=%s' "$(ratio "${fetch_rows}" "${fetch_count}")"
    printf ' fetch_bytes_per_call=%s' "$(ratio "${fetch_bytes}" "${fetch_count}")"
    printf ' fetch_bytes_per_callback=%s' "$(ratio "${fetch_bytes}" "${admitted_tasks}")"
    printf ' read_block_map_us_per_callback=%s' "$(ratio "$(field_value "${read_block_map_line}" read_block_map_us)" "${admitted_tasks}")"
    printf ' repo_fetch_block_range_us_per_callback=%s' "$(ratio "$(field_value "${repo_fetch_line}" repo_fetch_block_range_us)" "${admitted_tasks}")"
    printf ' pg_fetch_us_per_callback=%s' "$(ratio "${fetch_total_us}" "${admitted_tasks}")"
    printf ' pg_decode_us_per_callback=%s' "$(ratio "${decode_total_us}" "${admitted_tasks}")"
    printf ' file_read_metadata_us_per_callback=%s' "$(ratio "${metadata_total_us}" "${admitted_tasks}")"
    printf ' non_fetch_operation_count=%s' "$(nonnegative_difference "${operation_count}" "${fetch_count}")"
    printf '\n'
}

print_full_phase() {
    local phase="$1"
    local phase_log="$2"

    echo
    echo "--- phase=${phase} log=${phase_log} ---"
    awk '
        /FOD boundary profile:/ {
            in_boundary = 1
            print
            next
        }
        in_boundary && / - [A-Z]+ -   / {
            print
            next
        }
        in_boundary {
            in_boundary = 0
        }
        /FOD PostgreSQL lane observability/ ||
        /FOD logical task observability:/ ||
        /FOD persist/ ||
        /FOD read/ ||
        /FOD write/ ||
        /operation_failures=/ {
            print
        }
    ' "${phase_log}"
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
        else
            print_full_phase "${phase}" "${phase_log}"
        fi
    done
done
