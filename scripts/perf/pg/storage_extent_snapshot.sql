-- Capture the current physical block/extent representation after a storage profile.

\pset pager off
\pset tuples_only on
\pset format unaligned
\pset footer off

SET search_path TO fod, public;

SELECT 'captured_at=' || now()::text;
SELECT 'data_blocks_rows=' || COUNT(*)::text FROM data_blocks;
SELECT 'data_blocks_payload_bytes=' || COALESCE(SUM(OCTET_LENGTH(data)), 0)::text
FROM data_blocks;
SELECT 'data_extents_rows=' || COUNT(*)::text FROM data_extents;
SELECT 'data_extents_payload_bytes=' || COALESCE(SUM(OCTET_LENGTH(payload)), 0)::text
FROM data_extents;
SELECT 'data_extents_max_payload_bytes=' || COALESCE(MAX(OCTET_LENGTH(payload)), 0)::text
FROM data_extents;

WITH relation_names(relname) AS (
    VALUES
        ('data_blocks'),
        ('data_extents'),
        ('idx_data_blocks_object_order'),
        ('idx_data_blocks_data_object_id'),
        ('idx_data_extents_object_start'),
        ('idx_data_extents_data_object_id')
),
relations AS (
    SELECT r.relname, c.oid
    FROM relation_names r
    LEFT JOIN pg_namespace n ON n.nspname = 'fod'
    LEFT JOIN pg_class c
      ON c.relname = r.relname
     AND c.relnamespace = n.oid
)
SELECT relname || '_relation_size_bytes=' || COALESCE(pg_relation_size(oid)::text, '0')
FROM relations
ORDER BY relname;

WITH relation_names(relname) AS (
    VALUES
        ('data_blocks'),
        ('data_extents'),
        ('idx_data_blocks_object_order'),
        ('idx_data_blocks_data_object_id'),
        ('idx_data_extents_object_start'),
        ('idx_data_extents_data_object_id')
),
relations AS (
    SELECT r.relname, c.oid
    FROM relation_names r
    LEFT JOIN pg_namespace n ON n.nspname = 'fod'
    LEFT JOIN pg_class c
      ON c.relname = r.relname
     AND c.relnamespace = n.oid
)
SELECT relname || '_total_size_bytes=' || COALESCE(pg_total_relation_size(oid)::text, '0')
FROM relations
ORDER BY relname;
