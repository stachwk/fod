-- Machine-readable WAL/checkpointer snapshot for before/after deltas.
--
-- Output format:
--
--     metric<TAB>value
--
-- Keep this intentionally narrow and stable so Makefile can compute deltas
-- without parsing the human-readable wal_checkpointer.sql output.

\set QUIET on
\pset format unaligned
\pset tuples_only on
\pset fieldsep '\t'
\set QUIET off

SELECT metric, value
FROM (
    SELECT 'wal_records' AS metric, wal_records::numeric AS value FROM pg_stat_wal
    UNION ALL
    SELECT 'wal_fpi', wal_fpi::numeric FROM pg_stat_wal
    UNION ALL
    SELECT 'wal_bytes', wal_bytes::numeric FROM pg_stat_wal
    UNION ALL
    SELECT 'wal_buffers_full', wal_buffers_full::numeric FROM pg_stat_wal
    UNION ALL
    SELECT 'wal_write', wal_write::numeric FROM pg_stat_wal
    UNION ALL
    SELECT 'wal_sync', wal_sync::numeric FROM pg_stat_wal
    UNION ALL
    SELECT 'buffers_checkpoint', buffers_checkpoint::numeric FROM pg_stat_bgwriter
    UNION ALL
    SELECT 'buffers_backend', buffers_backend::numeric FROM pg_stat_bgwriter
    UNION ALL
    SELECT 'buffers_backend_fsync', buffers_backend_fsync::numeric FROM pg_stat_bgwriter
) AS snapshot
ORDER BY metric;
