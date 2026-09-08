#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

MATRIX="${TMP}/matrix"
CASE="${TMP}/case-512k"
mkdir -p "${MATRIX}" "${CASE}"

cat >"${MATRIX}/512k.log" <<EOF
artifact_dir=${CASE}
EOF

cat >"${CASE}/primary-write-mount.log" <<'EOF'
2026-01-01T00:00:01Z - INFO - FOD PostgreSQL lane observability: stage=periodic lane=shared operation_count=10 operation_failures=0 operation_micros_total=100 persist_operation_count=1 persist_micros_total=90
2026-01-01T00:00:02Z - INFO - FOD logical task observability: stage=shutdown lane=write operation=file-write admitted_tasks=2048 completed_bytes_per_second=70000000 elapsed_micros=15000000
2026-01-01T00:00:02Z - INFO - FOD PostgreSQL lane observability: stage=post-mount lane=shared operation_count=296 operation_failures=0 operation_micros_total=15000000 operation_micros_max=1000000 acquisition_wait_micros_total=3 acquisition_wait_micros_max=2 persist_operation_count=16 persist_input_bytes_total=1073741824 persist_input_bytes_max=67108864 persist_micros_total=14500000 persist_micros_max=1000000 persist_transaction_micros_total=14499900 persist_copy_stage_micros_total=5700000 persist_data_blocks_merge_micros_total=8600000 payload_peak_in_flight_bytes=67108864 write_transaction_backpressure_events=0
EOF

cat >"${CASE}/primary-read-mount.log" <<'EOF'
2026-01-01T00:00:03Z - INFO - FOD boundary profile:
2026-01-01T00:00:03Z - INFO -   fuse_read_total_us=420000
2026-01-01T00:00:03Z - INFO -   read_block_map_us=360000
2026-01-01T00:00:03Z - INFO -   read_fill_wait_count=3
2026-01-01T00:00:03Z - INFO -   read_fill_wait_us=120000
2026-01-01T00:00:03Z - INFO -   repo_fetch_block_range_us=280000
2026-01-01T00:00:03Z - INFO -   assemble_read_slice_us=10000
2026-01-01T00:00:03Z - INFO -   reply_data_us=50000
2026-01-01T00:00:03Z - INFO -   pg_prepared_statement name=fod_fetch_block_range_with_size count=4 total_us=200000 max_us=60000 avg_us=50000 params=12 param_bytes=24 result_rows=512 result_bytes=2097152 failures=0
2026-01-01T00:00:03Z - INFO -   pg_result_decode name=fod_fetch_block_range_with_size count=4 total_us=30000 max_us=9000 avg_us=7500 result_rows=512 result_bytes=2097152 failures=0
2026-01-01T00:00:03Z - INFO -   pg_prepared_statement name=fod_file_read_metadata count=8 total_us=80000 max_us=12000 avg_us=10000 params=8 param_bytes=8 result_rows=8 result_bytes=512 failures=0
2026-01-01T00:00:03Z - INFO - FOD logical task observability: stage=shutdown lane=read operation=file-read admitted_tasks=8 completed_bytes_per_second=480000000 elapsed_micros=2200000
2026-01-01T00:00:03Z - INFO - FOD PostgreSQL lane observability: stage=post-mount lane=shared operation_count=20 operation_failures=0 operation_micros_total=2600000 operation_micros_max=42000 acquisition_wait_micros_total=8 acquisition_wait_micros_max=8 persist_operation_count=0 persist_input_bytes_total=0 persist_input_bytes_max=0 persist_micros_total=0 persist_micros_max=0 persist_transaction_micros_total=0 persist_copy_stage_micros_total=0 persist_data_blocks_merge_micros_total=0 payload_peak_in_flight_bytes=0 write_transaction_backpressure_events=0
EOF

cat >"${CASE}/replica-read-mount.log" <<'EOF'
2026-01-01T00:00:04Z - INFO - FOD boundary profile:
2026-01-01T00:00:04Z - INFO -   fuse_read_total_us=440000
2026-01-01T00:00:04Z - INFO -   read_block_map_us=400000
2026-01-01T00:00:04Z - INFO -   read_fill_wait_count=5
2026-01-01T00:00:04Z - INFO -   read_fill_wait_us=150000
2026-01-01T00:00:04Z - INFO -   repo_fetch_block_range_us=300000
2026-01-01T00:00:04Z - INFO -   assemble_read_slice_us=12000
2026-01-01T00:00:04Z - INFO -   reply_data_us=48000
2026-01-01T00:00:04Z - INFO -   pg_prepared_statement name=fod_fetch_block_range count=4 total_us=240000 max_us=70000 avg_us=60000 params=12 param_bytes=24 result_rows=512 result_bytes=2097152 failures=0
2026-01-01T00:00:04Z - INFO -   pg_result_decode name=fod_fetch_block_range count=4 total_us=32000 max_us=10000 avg_us=8000 result_rows=512 result_bytes=2097152 failures=0
2026-01-01T00:00:04Z - INFO - FOD logical task observability: stage=shutdown lane=read operation=file-read admitted_tasks=8 completed_bytes_per_second=500000000 elapsed_micros=2100000
2026-01-01T00:00:04Z - INFO - FOD PostgreSQL lane observability: stage=post-mount lane=shared operation_count=22 operation_failures=0 operation_micros_total=2500000 operation_micros_max=47000 acquisition_wait_micros_total=4 acquisition_wait_micros_max=4 persist_operation_count=0 persist_input_bytes_total=0 persist_input_bytes_max=0 persist_micros_total=0 persist_micros_max=0 persist_transaction_micros_total=0 persist_copy_stage_micros_total=0 persist_data_blocks_merge_micros_total=0 payload_peak_in_flight_bytes=0 write_transaction_backpressure_events=0
EOF

OUT="$("${ROOT}/scripts/perf/extract_primary_replica_profile.sh" "${MATRIX}")"

primary_line="$(grep '^phase=primary-read ' <<<"${OUT}")"
replica_line="$(grep '^phase=replica-read ' <<<"${OUT}")"

grep -Fq 'extract_mode=compact' <<<"${OUT}"
grep -Fq 'phase=primary-write completed_bytes_per_second=70000000' <<<"${OUT}"
grep -Fq 'persist_operation_count=16' <<<"${OUT}"
grep -Fq 'phase=primary-read completed_bytes_per_second=480000000' <<<"${OUT}"
grep -Fq 'profile_attribution_available=1' <<<"${primary_line}"
grep -Fq 'lane_observability_available=1' <<<"${primary_line}"
grep -Fq 'fetch_statement_name=fod_fetch_block_range_with_size' <<<"${primary_line}"
grep -Fq 'fetch_block_range_calls=4' <<<"${primary_line}"
grep -Fq 'fetch_block_range_result_bytes=2097152' <<<"${primary_line}"
grep -Fq 'pg_operations_per_callback=2.500000' <<<"${primary_line}"
grep -Fq 'fetch_calls_per_callback=0.500000' <<<"${primary_line}"
grep -Fq 'fetch_rows_per_call=128.000000' <<<"${primary_line}"
grep -Fq 'fetch_bytes_per_call=524288.000000' <<<"${primary_line}"
grep -Fq 'fetch_bytes_per_callback=262144.000000' <<<"${primary_line}"
grep -Fq 'read_block_map_us_per_callback=45000.000000' <<<"${primary_line}"
grep -Fq 'read_fill_wait_count=3' <<<"${primary_line}"
grep -Fq 'read_fill_wait_us=120000' <<<"${primary_line}"
grep -Fq 'repo_fetch_block_range_us_per_callback=35000.000000' <<<"${primary_line}"
grep -Fq 'pg_fetch_us_per_callback=25000.000000' <<<"${primary_line}"
grep -Fq 'pg_decode_us_per_callback=3750.000000' <<<"${primary_line}"
grep -Fq 'file_read_metadata_calls=8' <<<"${primary_line}"
grep -Fq 'file_read_metadata_total_us=80000' <<<"${primary_line}"
grep -Fq 'file_read_metadata_us_per_callback=10000.000000' <<<"${primary_line}"
grep -Fq 'non_fetch_operation_count=16' <<<"${primary_line}"
grep -Fq 'phase=replica-read completed_bytes_per_second=500000000' <<<"${OUT}"
grep -Fq 'profile_attribution_available=1' <<<"${replica_line}"
grep -Fq 'lane_observability_available=1' <<<"${replica_line}"
grep -Fq 'fetch_statement_name=fod_fetch_block_range' <<<"${replica_line}"
grep -Fq 'read_fill_wait_count=5' <<<"${replica_line}"
grep -Fq 'read_fill_wait_us=150000' <<<"${replica_line}"
grep -Fq 'file_read_metadata_calls=0' <<<"${replica_line}"
grep -Fq 'file_read_metadata_us_per_callback=0.000000' <<<"${replica_line}"
if grep -Fq 'stage=periodic' <<<"${OUT}"; then
    echo "compact mode unexpectedly emitted periodic sampler output" >&2
    exit 1
fi

FULL="$(FOD_PROFILE_EXTRACT_MODE=full "${ROOT}/scripts/perf/extract_primary_replica_profile.sh" "${MATRIX}")"
grep -Fq 'extract_mode=full' <<<"${FULL}"
grep -Fq 'stage=periodic' <<<"${FULL}"
grep -Fq 'pg_prepared_statement name=fod_fetch_block_range_with_size count=4' <<<"${FULL}"
grep -Fq 'pg_result_decode name=fod_fetch_block_range_with_size count=4' <<<"${FULL}"
grep -Fq 'pg_prepared_statement name=fod_fetch_block_range count=4' <<<"${FULL}"
grep -Fq 'pg_result_decode name=fod_fetch_block_range count=4' <<<"${FULL}"
grep -Fq 'read_block_map_us=360000' <<<"${FULL}"

if FOD_PROFILE_EXTRACT_MODE=invalid "${ROOT}/scripts/perf/extract_primary_replica_profile.sh" "${MATRIX}" >/dev/null 2>&1; then
    echo "invalid extract mode unexpectedly succeeded" >&2
    exit 1
fi

echo "OK primary-replica-profile-extract"
