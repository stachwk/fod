-- Capture real PostgreSQL table/index DML counters for storage profiling.
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

WITH table_names(relname) AS (
    VALUES
        ('data_blocks'),
        ('data_extents'),
        ('copy_block_crc'),
        ('files'),
        ('data_objects')
),
table_stats AS (
    SELECT
        t.relname,
        to_jsonb(s) AS stats
    FROM table_names t
    LEFT JOIN pg_stat_user_tables s
      ON s.schemaname = 'fod'
     AND s.relname = t.relname
),
metrics(relname, metric, value) AS (
    SELECT relname, 'seq_scan', COALESCE(stats->>'seq_scan', '0') FROM table_stats
    UNION ALL SELECT relname, 'seq_tup_read', COALESCE(stats->>'seq_tup_read', '0') FROM table_stats
    UNION ALL SELECT relname, 'idx_scan', COALESCE(stats->>'idx_scan', '0') FROM table_stats
    UNION ALL SELECT relname, 'idx_tup_fetch', COALESCE(stats->>'idx_tup_fetch', '0') FROM table_stats
    UNION ALL SELECT relname, 'n_tup_ins', COALESCE(stats->>'n_tup_ins', '0') FROM table_stats
    UNION ALL SELECT relname, 'n_tup_upd', COALESCE(stats->>'n_tup_upd', '0') FROM table_stats
    UNION ALL SELECT relname, 'n_tup_hot_upd', COALESCE(stats->>'n_tup_hot_upd', '0') FROM table_stats
    UNION ALL SELECT relname, 'n_tup_newpage_upd', COALESCE(stats->>'n_tup_newpage_upd', '0') FROM table_stats
    UNION ALL SELECT relname, 'n_tup_del', COALESCE(stats->>'n_tup_del', '0') FROM table_stats
    UNION ALL SELECT relname, 'n_live_tup', COALESCE(stats->>'n_live_tup', '0') FROM table_stats
    UNION ALL SELECT relname, 'n_dead_tup', COALESCE(stats->>'n_dead_tup', '0') FROM table_stats
    UNION ALL SELECT relname, 'n_mod_since_analyze', COALESCE(stats->>'n_mod_since_analyze', '0') FROM table_stats
    UNION ALL SELECT relname, 'n_ins_since_vacuum', COALESCE(stats->>'n_ins_since_vacuum', '0') FROM table_stats
    UNION ALL SELECT relname, 'vacuum_count', COALESCE(stats->>'vacuum_count', '0') FROM table_stats
    UNION ALL SELECT relname, 'autovacuum_count', COALESCE(stats->>'autovacuum_count', '0') FROM table_stats
    UNION ALL SELECT relname, 'analyze_count', COALESCE(stats->>'analyze_count', '0') FROM table_stats
    UNION ALL SELECT relname, 'autoanalyze_count', COALESCE(stats->>'autoanalyze_count', '0') FROM table_stats
)
SELECT relname || '_' || metric || '=' || value
FROM metrics
ORDER BY relname, metric;

WITH index_names(indexrelname) AS (
    VALUES
        ('idx_data_blocks_object_order'),
        ('idx_data_blocks_data_object_id'),
        ('idx_data_extents_object_start'),
        ('idx_data_extents_data_object_id'),
        ('idx_copy_block_crc_object_order'),
        ('idx_copy_block_crc_data_object_id')
),
index_stats AS (
    SELECT
        i.indexrelname,
        s.idx_scan,
        s.idx_tup_read,
        s.idx_tup_fetch
    FROM index_names i
    LEFT JOIN pg_stat_user_indexes s
      ON s.schemaname = 'fod'
     AND s.indexrelname = i.indexrelname
),
metrics(indexrelname, metric, value) AS (
    SELECT indexrelname, 'idx_scan', COALESCE(idx_scan::text, '0') FROM index_stats
    UNION ALL SELECT indexrelname, 'idx_tup_read', COALESCE(idx_tup_read::text, '0') FROM index_stats
    UNION ALL SELECT indexrelname, 'idx_tup_fetch', COALESCE(idx_tup_fetch::text, '0') FROM index_stats
)
SELECT indexrelname || '_' || metric || '=' || value
FROM metrics
ORDER BY indexrelname, metric;

WITH relation_names(relname) AS (
    VALUES
        ('data_blocks'),
        ('data_extents'),
        ('copy_block_crc'),
        ('files'),
        ('data_objects'),
        ('idx_data_blocks_object_order'),
        ('idx_data_blocks_data_object_id'),
        ('idx_data_extents_object_start'),
        ('idx_data_extents_data_object_id'),
        ('idx_copy_block_crc_object_order'),
        ('idx_copy_block_crc_data_object_id')
),
relation_sizes AS (
    SELECT
        r.relname,
        c.oid
    FROM relation_names r
    LEFT JOIN pg_namespace n
      ON n.nspname = 'fod'
    LEFT JOIN pg_class c
      ON c.relname = r.relname
     AND c.relnamespace = n.oid
)
SELECT relname || '_relation_size_bytes=' || COALESCE(pg_relation_size(oid)::text, '0')
FROM relation_sizes
ORDER BY relname;

WITH relation_names(relname) AS (
    VALUES
        ('data_blocks'),
        ('data_extents'),
        ('copy_block_crc'),
        ('files'),
        ('data_objects'),
        ('idx_data_blocks_object_order'),
        ('idx_data_blocks_data_object_id'),
        ('idx_data_extents_object_start'),
        ('idx_data_extents_data_object_id'),
        ('idx_copy_block_crc_object_order'),
        ('idx_copy_block_crc_data_object_id')
),
relation_sizes AS (
    SELECT
        r.relname,
        c.oid
    FROM relation_names r
    LEFT JOIN pg_namespace n
      ON n.nspname = 'fod'
    LEFT JOIN pg_class c
      ON c.relname = r.relname
     AND c.relnamespace = n.oid
)
SELECT relname || '_total_size_bytes=' || COALESCE(pg_total_relation_size(oid)::text, '0')
FROM relation_sizes
ORDER BY relname;
