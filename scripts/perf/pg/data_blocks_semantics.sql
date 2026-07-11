-- Inspect object-owned payload semantics after the storage ownership migration.
-- This script is read-only and is intended for:
--
--     make profile-pg-data-blocks-semantics PROFILE_CAPTURE_LABEL=...

SET search_path TO fod, public;

SELECT now() AS captured_at;

SELECT
    current_database() AS database_name,
    current_schema() AS current_schema,
    current_setting('search_path') AS search_path;

SELECT 'payload columns' AS section;

SELECT
    table_name,
    ordinal_position,
    column_name,
    data_type,
    is_nullable,
    column_default
FROM information_schema.columns
WHERE table_schema = 'fod'
  AND table_name IN ('data_blocks', 'data_extents', 'copy_block_crc')
ORDER BY table_name, ordinal_position;

SELECT 'payload constraints' AS section;

SELECT
    rel.relname AS table_name,
    con.conname,
    con.contype,
    pg_get_constraintdef(con.oid) AS definition
FROM pg_constraint con
JOIN pg_class rel ON rel.oid = con.conrelid
JOIN pg_namespace ns ON ns.oid = rel.relnamespace
WHERE ns.nspname = 'fod'
  AND rel.relname IN ('data_blocks', 'data_extents', 'copy_block_crc')
ORDER BY rel.relname, con.conname;

SELECT 'payload indexes' AS section;

SELECT
    tablename,
    indexname,
    indexdef
FROM pg_indexes
WHERE schemaname = 'fod'
  AND tablename IN ('data_blocks', 'data_extents', 'copy_block_crc')
ORDER BY tablename, indexname;

SELECT 'row counts' AS section;

SELECT
    (SELECT count(*) FROM data_objects) AS data_objects_rows,
    (SELECT count(*) FROM files) AS files_rows,
    (SELECT count(*) FROM data_blocks) AS data_blocks_rows,
    (SELECT count(DISTINCT data_object_id) FROM data_blocks) AS data_blocks_objects,
    (SELECT count(*) FROM data_extents) AS data_extents_rows,
    (SELECT count(DISTINCT data_object_id) FROM data_extents) AS data_extents_objects,
    (SELECT count(*) FROM copy_block_crc) AS copy_block_crc_rows,
    (SELECT count(DISTINCT data_object_id) FROM copy_block_crc) AS copy_block_crc_objects;

SELECT 'orphan checks' AS section;

SELECT
    (SELECT count(*)
     FROM files f
     LEFT JOIN data_objects o ON o.id_data_object = f.data_object_id
     WHERE o.id_data_object IS NULL) AS orphan_files,
    (SELECT count(*)
     FROM data_blocks b
     LEFT JOIN data_objects o ON o.id_data_object = b.data_object_id
     WHERE o.id_data_object IS NULL) AS orphan_blocks,
    (SELECT count(*)
     FROM data_extents e
     LEFT JOIN data_objects o ON o.id_data_object = e.data_object_id
     WHERE o.id_data_object IS NULL) AS orphan_extents,
    (SELECT count(*)
     FROM copy_block_crc c
     LEFT JOIN data_objects o ON o.id_data_object = c.data_object_id
     WHERE o.id_data_object IS NULL) AS orphan_crc_rows;

SELECT 'reference counts' AS section;

WITH actual_references AS (
    SELECT data_object_id, count(*)::bigint AS file_count
    FROM files
    GROUP BY data_object_id
)
SELECT
    count(*) FILTER (WHERE coalesce(r.file_count, 0) = 0) AS unreferenced_objects,
    count(*) FILTER (
        WHERE o.reference_count::bigint IS DISTINCT FROM coalesce(r.file_count, 0)
    ) AS reference_count_mismatches,
    count(*) FILTER (WHERE coalesce(r.file_count, 0) > 1) AS shared_objects,
    max(coalesce(r.file_count, 0)) AS max_files_per_object
FROM data_objects o
LEFT JOIN actual_references r ON r.data_object_id = o.id_data_object;

SELECT 'payload layout per object' AS section;

WITH block_rows AS (
    SELECT data_object_id, count(*) AS row_count
    FROM data_blocks
    GROUP BY data_object_id
),
extent_rows AS (
    SELECT data_object_id, count(*) AS row_count
    FROM data_extents
    GROUP BY data_object_id
)
SELECT
    count(*) FILTER (WHERE coalesce(b.row_count, 0) > 0) AS block_objects,
    count(*) FILTER (WHERE coalesce(e.row_count, 0) > 0) AS extent_objects,
    count(*) FILTER (
        WHERE coalesce(b.row_count, 0) > 0 AND coalesce(e.row_count, 0) > 0
    ) AS hybrid_objects,
    max(coalesce(b.row_count, 0)) AS max_blocks_per_object,
    max(coalesce(e.row_count, 0)) AS max_extents_per_object
FROM data_objects o
LEFT JOIN block_rows b ON b.data_object_id = o.id_data_object
LEFT JOIN extent_rows e ON e.data_object_id = o.id_data_object;

SELECT 'interpretation guide' AS section;

SELECT
    'All orphan counters must be zero; PostgreSQL foreign keys should enforce this after schema version 17.' AS note
UNION ALL
SELECT
    'reference_count_mismatches should be investigated before object GC or dedupe decisions.'
UNION ALL
SELECT
    'hybrid_objects should stay zero because reads prefer extents and block patches convert the complete object before removing extent rows.';
