-- Inspect real data_blocks and related table/index size and churn signals.
--
-- This script is read-only. It does not estimate bloat mathematically; it
-- captures the PostgreSQL statistics and relation sizes needed before deciding
-- whether a deeper bloat extension/query is worth adding.

SET search_path TO fod, public;

SELECT now() AS captured_at;

SELECT
    current_database() AS database_name,
    current_schema() AS current_schema,
    current_setting('search_path') AS search_path;

SELECT 'table tuple statistics' AS section;

SELECT
    schemaname,
    relname,
    n_live_tup,
    n_dead_tup,
    n_mod_since_analyze,
    last_vacuum,
    last_autovacuum,
    last_analyze,
    last_autoanalyze
FROM pg_stat_user_tables
WHERE relname IN ('data_blocks', 'copy_block_crc', 'files', 'data_objects')
ORDER BY relname;

SELECT 'index usage statistics' AS section;

SELECT
    schemaname,
    relname,
    indexrelname,
    idx_scan,
    idx_tup_read,
    idx_tup_fetch
FROM pg_stat_user_indexes
WHERE relname IN ('data_blocks', 'copy_block_crc')
ORDER BY relname, indexrelname;

SELECT 'relation sizes' AS section;

SELECT
    c.relname,
    pg_size_pretty(pg_relation_size(c.oid)) AS relation_size,
    pg_size_pretty(pg_total_relation_size(c.oid)) AS total_size
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = 'fod'
  AND c.relname IN (
      'data_blocks',
      'idx_data_blocks_data_object_id',
      'idx_data_blocks_object_order',
      'copy_block_crc'
  )
ORDER BY pg_total_relation_size(c.oid) DESC;
