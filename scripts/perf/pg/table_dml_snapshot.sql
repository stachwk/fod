-- Capture real PostgreSQL table/index DML counters for data_blocks profiling.
--
-- This script is read-only. It is intended for before/after snapshots around a
-- real workload, so the delta shows INSERT/UPDATE/HOT/dead-tuple behavior from
-- the production path instead of a synthetic EXPLAIN probe.

\pset pager off
\pset tuples_only on
\pset format unaligned
\pset footer off

DO $$
BEGIN
    PERFORM pg_stat_force_next_flush();
EXCEPTION
    WHEN undefined_function THEN
        NULL;
END $$;

SELECT 'captured_at=' || now()::text;
SELECT 'database_name=' || current_database();
SELECT 'source=pg_stat_user_tables_pg_stat_user_indexes_pg_class';
SELECT 'database_stats_reset=' || COALESCE(stats_reset::text, '')
FROM pg_stat_database
WHERE datname = current_database();

WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_seq_scan=' || COALESCE((SELECT stats->>'seq_scan' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_seq_tup_read=' || COALESCE((SELECT stats->>'seq_tup_read' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_idx_scan=' || COALESCE((SELECT stats->>'idx_scan' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_idx_tup_fetch=' || COALESCE((SELECT stats->>'idx_tup_fetch' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_n_tup_ins=' || COALESCE((SELECT stats->>'n_tup_ins' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_n_tup_upd=' || COALESCE((SELECT stats->>'n_tup_upd' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_n_tup_hot_upd=' || COALESCE((SELECT stats->>'n_tup_hot_upd' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_n_tup_newpage_upd=' || COALESCE((SELECT stats->>'n_tup_newpage_upd' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_n_tup_del=' || COALESCE((SELECT stats->>'n_tup_del' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_n_live_tup=' || COALESCE((SELECT stats->>'n_live_tup' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_n_dead_tup=' || COALESCE((SELECT stats->>'n_dead_tup' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_n_mod_since_analyze=' || COALESCE((SELECT stats->>'n_mod_since_analyze' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_n_ins_since_vacuum=' || COALESCE((SELECT stats->>'n_ins_since_vacuum' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_vacuum_count=' || COALESCE((SELECT stats->>'vacuum_count' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_autovacuum_count=' || COALESCE((SELECT stats->>'autovacuum_count' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_analyze_count=' || COALESCE((SELECT stats->>'analyze_count' FROM table_stats), '0');
WITH table_stats AS (
    SELECT to_jsonb(s) AS stats
    FROM pg_stat_user_tables s
    WHERE s.schemaname = 'fod'
      AND s.relname = 'data_blocks'
)
SELECT 'data_blocks_autoanalyze_count=' || COALESCE((SELECT stats->>'autoanalyze_count' FROM table_stats), '0');

WITH index_stats AS (
    SELECT *
    FROM pg_stat_user_indexes
    WHERE schemaname = 'fod'
      AND relname = 'data_blocks'
      AND indexrelname = 'idx_data_blocks_object_order'
)
SELECT 'idx_data_blocks_object_order_idx_scan=' || COALESCE((SELECT idx_scan::text FROM index_stats), '0');
WITH index_stats AS (
    SELECT *
    FROM pg_stat_user_indexes
    WHERE schemaname = 'fod'
      AND relname = 'data_blocks'
      AND indexrelname = 'idx_data_blocks_object_order'
)
SELECT 'idx_data_blocks_object_order_idx_tup_read=' || COALESCE((SELECT idx_tup_read::text FROM index_stats), '0');
WITH index_stats AS (
    SELECT *
    FROM pg_stat_user_indexes
    WHERE schemaname = 'fod'
      AND relname = 'data_blocks'
      AND indexrelname = 'idx_data_blocks_object_order'
)
SELECT 'idx_data_blocks_object_order_idx_tup_fetch=' || COALESCE((SELECT idx_tup_fetch::text FROM index_stats), '0');

WITH index_stats AS (
    SELECT *
    FROM pg_stat_user_indexes
    WHERE schemaname = 'fod'
      AND relname = 'data_blocks'
      AND indexrelname = 'idx_data_blocks_data_object_id'
)
SELECT 'idx_data_blocks_data_object_id_idx_scan=' || COALESCE((SELECT idx_scan::text FROM index_stats), '0');
WITH index_stats AS (
    SELECT *
    FROM pg_stat_user_indexes
    WHERE schemaname = 'fod'
      AND relname = 'data_blocks'
      AND indexrelname = 'idx_data_blocks_data_object_id'
)
SELECT 'idx_data_blocks_data_object_id_idx_tup_read=' || COALESCE((SELECT idx_tup_read::text FROM index_stats), '0');
WITH index_stats AS (
    SELECT *
    FROM pg_stat_user_indexes
    WHERE schemaname = 'fod'
      AND relname = 'data_blocks'
      AND indexrelname = 'idx_data_blocks_data_object_id'
)
SELECT 'idx_data_blocks_data_object_id_idx_tup_fetch=' || COALESCE((SELECT idx_tup_fetch::text FROM index_stats), '0');

SELECT 'data_blocks_relation_size_bytes=' || COALESCE((
    SELECT pg_relation_size(c.oid)::text
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'fod'
      AND c.relname = 'data_blocks'
), '0');
SELECT 'data_blocks_total_size_bytes=' || COALESCE((
    SELECT pg_total_relation_size(c.oid)::text
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'fod'
      AND c.relname = 'data_blocks'
), '0');
SELECT 'idx_data_blocks_object_order_relation_size_bytes=' || COALESCE((
    SELECT pg_relation_size(c.oid)::text
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'fod'
      AND c.relname = 'idx_data_blocks_object_order'
), '0');
SELECT 'idx_data_blocks_data_object_id_relation_size_bytes=' || COALESCE((
    SELECT pg_relation_size(c.oid)::text
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'fod'
      AND c.relname = 'idx_data_blocks_data_object_id'
), '0');
