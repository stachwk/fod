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
2026-01-01T00:00:03Z - INFO - FOD logical task observability: stage=shutdown lane=read operation=file-read admitted_tasks=2048 completed_bytes_per_second=480000000 elapsed_micros=2200000
2026-01-01T00:00:03Z - INFO - FOD PostgreSQL lane observability: stage=post-mount lane=shared operation_count=2182 operation_failures=0 operation_micros_total=2600000 operation_micros_max=42000 acquisition_wait_micros_total=8 acquisition_wait_micros_max=8 persist_operation_count=0 persist_input_bytes_total=0 persist_input_bytes_max=0 persist_micros_total=0 persist_micros_max=0 persist_transaction_micros_total=0 persist_copy_stage_micros_total=0 persist_data_blocks_merge_micros_total=0 payload_peak_in_flight_bytes=0 write_transaction_backpressure_events=0
EOF

cat >"${CASE}/replica-read-mount.log" <<'EOF'
2026-01-01T00:00:04Z - INFO - FOD logical task observability: stage=shutdown lane=read operation=file-read admitted_tasks=2048 completed_bytes_per_second=500000000 elapsed_micros=2100000
2026-01-01T00:00:04Z - INFO - FOD PostgreSQL lane observability: stage=post-mount lane=shared operation_count=2189 operation_failures=0 operation_micros_total=2500000 operation_micros_max=47000 acquisition_wait_micros_total=4 acquisition_wait_micros_max=4 persist_operation_count=0 persist_input_bytes_total=0 persist_input_bytes_max=0 persist_micros_total=0 persist_micros_max=0 persist_transaction_micros_total=0 persist_copy_stage_micros_total=0 persist_data_blocks_merge_micros_total=0 payload_peak_in_flight_bytes=0 write_transaction_backpressure_events=0
EOF

OUT="$("${ROOT}/scripts/perf/extract_primary_replica_profile.sh" "${MATRIX}")"

grep -Fq 'extract_mode=compact' <<<"${OUT}"
grep -Fq 'phase=primary-write completed_bytes_per_second=70000000' <<<"${OUT}"
grep -Fq 'persist_operation_count=16' <<<"${OUT}"
grep -Fq 'persist_input_bytes_max=67108864' <<<"${OUT}"
grep -Fq 'persist_copy_stage_micros_total=5700000' <<<"${OUT}"
grep -Fq 'persist_data_blocks_merge_micros_total=8600000' <<<"${OUT}"
grep -Fq 'phase=primary-read completed_bytes_per_second=480000000' <<<"${OUT}"
grep -Fq 'phase=replica-read completed_bytes_per_second=500000000' <<<"${OUT}"
if grep -Fq 'stage=periodic' <<<"${OUT}"; then
    echo "compact mode unexpectedly emitted periodic sampler output" >&2
    exit 1
fi

FULL="$(FOD_PROFILE_EXTRACT_MODE=full "${ROOT}/scripts/perf/extract_primary_replica_profile.sh" "${MATRIX}")"
grep -Fq 'extract_mode=full' <<<"${FULL}"
grep -Fq 'stage=periodic' <<<"${FULL}"

if FOD_PROFILE_EXTRACT_MODE=invalid "${ROOT}/scripts/perf/extract_primary_replica_profile.sh" "${MATRIX}" >/dev/null 2>&1; then
    echo "invalid extract mode unexpectedly succeeded" >&2
    exit 1
fi

echo "OK primary-replica-profile-extract"
