#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

HOST="${FOD_PG_WRITE_PROFILE_HOST:-127.0.0.1}"
PORT="${FOD_PG_WRITE_PROFILE_PORT:-55441}"
DB="${FOD_PG_WRITE_PROFILE_DB:-${POSTGRES_DB:-foddbname}}"
USER_NAME="${FOD_PG_WRITE_PROFILE_USER:-${POSTGRES_USER:-foduser}}"
PASSWORD="${FOD_PG_WRITE_PROFILE_PASSWORD:-${POSTGRES_PASSWORD:-cichosza}}"
POLL_SECONDS="${FOD_PG_WRITE_PROFILE_POLL_SECONDS:-0.25}"
WAL_EVERY="${FOD_PG_WRITE_PROFILE_WAL_EVERY:-4}"
PROCESS_PATTERN="${FOD_PG_WRITE_PROFILE_PROCESS_PATTERN:-/tmp/fod-primary-write\.}"
PRECHECK_ONLY="${FOD_PG_WRITE_PROFILE_PRECHECK_ONLY:-0}"

COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="${FOD_PG_WRITE_PROFILE_OUT:-$ROOT/artifacts/perf/$COMMIT/pg-primary-write-$STAMP}"

case "$WAL_EVERY" in
    ''|*[!0-9]*) echo "FOD_PG_WRITE_PROFILE_WAL_EVERY must be a positive integer" >&2; exit 2 ;;
esac
if [ "$WAL_EVERY" -lt 1 ]; then
    echo "FOD_PG_WRITE_PROFILE_WAL_EVERY must be >= 1" >&2
    exit 2
fi

printf '=== FOD PRIMARY WRITE POSTGRES PROFILE ===\n'
printf 'host=%s\nport=%s\ndatabase=%s\nuser=%s\npoll_seconds=%s\nwal_every=%s\nprocess_pattern=%s\nartifact_dir=%s\n' \
    "$HOST" "$PORT" "$DB" "$USER_NAME" "$POLL_SECONDS" "$WAL_EVERY" "$PROCESS_PATTERN" "$OUT"

if [ "$PRECHECK_ONLY" = "1" ]; then
    echo 'precheck_only=1'
    exit 0
fi

command -v psql >/dev/null 2>&1 || { echo 'psql is required' >&2; exit 1; }
command -v pgrep >/dev/null 2>&1 || { echo 'pgrep is required' >&2; exit 1; }
mkdir -p "$OUT"

export PGPASSWORD="$PASSWORD"
PG=(psql -X -h "$HOST" -p "$PORT" -U "$USER_NAME" -d "$DB")
ERROR_LOG="$OUT/sampler-errors.log"

until "${PG[@]}" -Atqc 'SELECT 1' >/dev/null 2>>"$ERROR_LOG"; do
    sleep 0.1
done

"${PG[@]}" -At -F $'\t' -c "
SELECT 'track_io_timing', current_setting('track_io_timing')
UNION ALL
SELECT 'track_wal_io_timing', current_setting('track_wal_io_timing')
UNION ALL
SELECT 'synchronous_commit', current_setting('synchronous_commit');
" > "$OUT/settings.tsv" 2>>"$ERROR_LOG" || true

echo 'Waiting for primary-write FOD process...'
while true; do
    PID="$(pgrep -af fod-rust-fuse | awk -v pattern="$PROCESS_PATTERN" '$0 ~ pattern {print $1}' | tail -1)"
    [ -n "$PID" ] && break
    sleep 0.05
done

echo "primary_write_pid=$PID"

: > "$OUT/activity.tsv"
: > "$OUT/wal.tsv"
: > "$OUT/io.tsv"

sample_activity() {
    "${PG[@]}" -At -F $'\t' -c "
SELECT
    clock_timestamp(),
    pid,
    application_name,
    state,
    coalesce(wait_event_type, '-'),
    coalesce(wait_event, '-'),
    backend_type,
    round(extract(epoch FROM (clock_timestamp() - query_start))::numeric, 6),
    left(regexp_replace(query, '\\s+', ' ', 'g'), 240)
FROM pg_stat_activity
WHERE datname = current_database()
  AND pid <> pg_backend_pid()
  AND backend_type = 'client backend'
ORDER BY pid;
" >> "$OUT/activity.tsv" 2>>"$ERROR_LOG"
}

sample_wal_io() {
    "${PG[@]}" -At -F $'\t' -c "
SELECT
    clock_timestamp(), wal_records, wal_fpi, wal_bytes, wal_buffers_full,
    wal_write, wal_sync, wal_write_time, wal_sync_time
FROM pg_stat_wal;
" >> "$OUT/wal.tsv" 2>>"$ERROR_LOG"

    "${PG[@]}" -At -F $'\t' -c "
SELECT
    clock_timestamp(), backend_type,
    sum(coalesce(reads, 0)),
    sum(coalesce(writes, 0)),
    sum(coalesce(writebacks, 0)),
    sum(coalesce(extends, 0)),
    sum(coalesce(hits, 0)),
    sum(coalesce(fsyncs, 0))
FROM pg_stat_io
WHERE backend_type IN ('client backend', 'checkpointer', 'background writer')
GROUP BY backend_type
ORDER BY backend_type;
" >> "$OUT/io.tsv" 2>>"$ERROR_LOG"
}

iteration=0
while kill -0 "$PID" 2>/dev/null; do
    sample_activity || true
    if [ $((iteration % WAL_EVERY)) -eq 0 ]; then
        sample_wal_io || true
    fi
    iteration=$((iteration + 1))
    sleep "$POLL_SECONDS"
done

# One final best-effort sample is intentionally not taken here. The benchmark may
# already be shutting primary down. The last successful continuous sample is the
# correct end-of-phase observation.

SUMMARY="$OUT/summary.txt"
{
    echo '=== WAIT EVENTS (all client-backend samples) ==='
    awk -F '\t' 'NF >= 6 {key=$4 "\t" $5 "\t" $6; count[key]++} END {for (key in count) print count[key], key}' "$OUT/activity.tsv" \
        | sort -nr | head -50
    echo
    echo '=== ACTIVE QUERY SAMPLES ==='
    awk -F '\t' '$4 == "active" && NF >= 9 {count[$9]++} END {for (query in count) print count[query], query}' "$OUT/activity.tsv" \
        | sort -nr | head -30
    echo
    echo '=== SETTINGS ==='
    cat "$OUT/settings.tsv"
    echo
    echo '=== WAL FIRST/LAST ==='
    head -1 "$OUT/wal.tsv" || true
    tail -1 "$OUT/wal.tsv" || true
    echo
    echo '=== IO FIRST/LAST ==='
    head -3 "$OUT/io.tsv" || true
    tail -3 "$OUT/io.tsv" || true
    echo
    echo "artifact_dir=$OUT"
} | tee "$SUMMARY"
