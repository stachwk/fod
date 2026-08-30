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
PROCESS_MATCH="${FOD_PG_WRITE_PROFILE_PROCESS_MATCH:-${FOD_PG_WRITE_PROFILE_PROCESS_PATTERN:-/tmp/fod-primary-write.}}"
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
printf 'host=%s\nport=%s\ndatabase=%s\nuser=%s\npoll_seconds=%s\nwal_every=%s\nprocess_match=%s\nartifact_dir=%s\n' \
    "$HOST" "$PORT" "$DB" "$USER_NAME" "$POLL_SECONDS" "$WAL_EVERY" "$PROCESS_MATCH" "$OUT"

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

if ! "${PG[@]}" -v ON_ERROR_STOP=1 -At -F $'\t' -c "
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
SELECT clock_timestamp(), pg_stat_statements_reset();
" > "$OUT/pg-stat-statements-reset.tsv" 2>>"$ERROR_LOG"; then
    echo 'pg_stat_statements is required; check shared_preload_libraries and extension availability' >&2
    exit 1
fi

"${PG[@]}" -At -F $'\t' -c "
SELECT 'track_io_timing', current_setting('track_io_timing')
UNION ALL
SELECT 'track_wal_io_timing', current_setting('track_wal_io_timing')
UNION ALL
SELECT 'synchronous_commit', current_setting('synchronous_commit')
UNION ALL
SELECT 'shared_preload_libraries', current_setting('shared_preload_libraries')
UNION ALL
SELECT 'compute_query_id', current_setting('compute_query_id')
UNION ALL
SELECT 'pg_stat_statements.track', current_setting('pg_stat_statements.track')
UNION ALL
SELECT 'pg_stat_statements.track_utility', current_setting('pg_stat_statements.track_utility');
" > "$OUT/settings.tsv" 2>>"$ERROR_LOG" || true

find_primary_write_pid() {
    local processes
    processes="$(pgrep -af fod-rust-fuse 2>/dev/null || true)"
    printf '%s\n' "$processes" \
        | grep -F -- "$PROCESS_MATCH" \
        | awk '{print $1}' \
        | tail -1 \
        || true
}

echo 'Waiting for primary-write FOD process...'
while true; do
    PID="$(find_primary_write_pid)"
    [ -n "$PID" ] && break
    sleep 0.05
done

echo "primary_write_pid=$PID"

: > "$OUT/activity.tsv"
: > "$OUT/wal.tsv"
: > "$OUT/io.tsv"
: > "$OUT/statements.tsv"
: > "$OUT/relations.tsv"

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
    sum(coalesce(read_time, 0)),
    sum(coalesce(writes, 0)),
    sum(coalesce(write_time, 0)),
    sum(coalesce(writebacks, 0)),
    sum(coalesce(writeback_time, 0)),
    sum(coalesce(extends, 0)),
    sum(coalesce(extend_time, 0)),
    sum(coalesce(hits, 0)),
    sum(coalesce(fsyncs, 0)),
    sum(coalesce(fsync_time, 0))
FROM pg_stat_io
WHERE backend_type IN ('client backend', 'checkpointer', 'background writer')
GROUP BY backend_type
ORDER BY backend_type;
" >> "$OUT/io.tsv" 2>>"$ERROR_LOG"

    "${PG[@]}" -At -F $'\t' -c "
WITH target AS (
    SELECT
        CASE
            WHEN query ~* '^[[:space:]]*COPY[[:space:]]+fod_persist_block_stage'
                THEN 'copy_stage'
            WHEN query ~* '^[[:space:]]*INSERT[[:space:]]+INTO[[:space:]]+data_blocks'
             AND query ~* 'FROM[[:space:]]+fod_persist_block_stage'
             AND query ~* 'ON[[:space:]]+CONFLICT'
                THEN 'insert_on_conflict'
        END AS statement_kind,
        calls,
        total_exec_time,
        rows,
        shared_blks_hit,
        shared_blks_read,
        shared_blks_dirtied,
        shared_blks_written,
        local_blks_hit,
        local_blks_read,
        local_blks_dirtied,
        local_blks_written,
        temp_blks_read,
        temp_blks_written,
        blk_read_time,
        blk_write_time,
        temp_blk_read_time,
        temp_blk_write_time,
        wal_records,
        wal_fpi,
        wal_bytes
    FROM pg_stat_statements
    WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
)
SELECT
    statement_timestamp(),
    statement_kind,
    sum(calls),
    round(sum(total_exec_time)::numeric, 3),
    round((sum(total_exec_time) / NULLIF(sum(calls), 0))::numeric, 3),
    sum(rows),
    sum(shared_blks_hit),
    sum(shared_blks_read),
    sum(shared_blks_dirtied),
    sum(shared_blks_written),
    sum(local_blks_hit),
    sum(local_blks_read),
    sum(local_blks_dirtied),
    sum(local_blks_written),
    sum(temp_blks_read),
    sum(temp_blks_written),
    round(sum(blk_read_time)::numeric, 3),
    round(sum(blk_write_time)::numeric, 3),
    round(sum(temp_blk_read_time)::numeric, 3),
    round(sum(temp_blk_write_time)::numeric, 3),
    sum(wal_records),
    sum(wal_fpi),
    sum(wal_bytes)
FROM target
WHERE statement_kind IS NOT NULL
GROUP BY statement_kind
ORDER BY statement_kind;
" >> "$OUT/statements.tsv" 2>>"$ERROR_LOG"

    "${PG[@]}" -At -F $'\t' -c "
SELECT
    clock_timestamp(),
    c.relname,
    c.relpersistence,
    CASE c.relpersistence
        WHEN 'p' THEN 'permanent'
        WHEN 'u' THEN 'unlogged'
        WHEN 't' THEN 'temporary'
        ELSE c.relpersistence::text
    END,
    n.nspname
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relname IN ('fod_persist_block_stage', 'data_blocks')
ORDER BY c.relname, n.nspname;
" >> "$OUT/relations.tsv" 2>>"$ERROR_LOG"
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
    echo '=== RELATION PERSISTENCE OBSERVED ==='
    printf 'timestamp\trelname\trelpersistence\tpersistence_kind\tnamespace\n'
    if [ -s "$OUT/relations.tsv" ]; then
        awk -F '\t' '!seen[$2 FS $3 FS $4 FS $5]++ {print}' "$OUT/relations.tsv"
    else
        echo 'no target relations observed'
    fi
    echo
    echo '=== PG_STAT_STATEMENTS COLUMNS ==='
    printf 'timestamp\tstatement_kind\tcalls\ttotal_exec_time_ms\tmean_exec_time_ms\trows\tshared_blks_hit\tshared_blks_read\tshared_blks_dirtied\tshared_blks_written\tlocal_blks_hit\tlocal_blks_read\tlocal_blks_dirtied\tlocal_blks_written\ttemp_blks_read\ttemp_blks_written\tblk_read_time_ms\tblk_write_time_ms\ttemp_blk_read_time_ms\ttemp_blk_write_time_ms\twal_records\twal_fpi\twal_bytes\n'
    if [ -s "$OUT/statements.tsv" ]; then
        first_statement_ts="$(head -1 "$OUT/statements.tsv" | cut -f1)"
        last_statement_ts="$(tail -1 "$OUT/statements.tsv" | cut -f1)"
        echo
        echo '=== PG_STAT_STATEMENTS FIRST OBSERVED ==='
        awk -F '\t' -v ts="$first_statement_ts" '$1 == ts' "$OUT/statements.tsv"
        echo
        echo '=== PG_STAT_STATEMENTS LAST / DELTA SINCE RESET ==='
        awk -F '\t' -v ts="$last_statement_ts" '$1 == ts' "$OUT/statements.tsv"
    else
        echo 'no target pg_stat_statements rows captured'
    fi
    echo
    echo "artifact_dir=$OUT"
} | tee "$SUMMARY"
