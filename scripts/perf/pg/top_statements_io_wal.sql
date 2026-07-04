-- Capture pg_stat_statements with shared/local/temp buffers and per-statement WAL.
--
-- PostgreSQL exposes wal_records, wal_fpi, and wal_bytes per statement here.
-- wal_buffers_full is available from pg_stat_wal, not pg_stat_statements.

SELECT
    queryid,
    calls,
    round(total_exec_time::numeric, 3) AS total_exec_ms,
    round(mean_exec_time::numeric, 3) AS mean_exec_ms,
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
    round(blk_read_time::numeric, 3) AS blk_read_ms,
    round(blk_write_time::numeric, 3) AS blk_write_ms,
    round(temp_blk_read_time::numeric, 3) AS temp_blk_read_ms,
    round(temp_blk_write_time::numeric, 3) AS temp_blk_write_ms,
    wal_records,
    wal_fpi,
    wal_bytes,
    left(regexp_replace(query, '\s+', ' ', 'g'), 240) AS query
FROM pg_stat_statements
ORDER BY total_exec_time DESC
LIMIT 30;
