-- Inspect data_blocks.id_file semantics before optimizing the merge path.
-- This script is read-only and is intended for:
--
--     make profile-pg-data-blocks-semantics PROFILE_CAPTURE_LABEL=...
--
-- It answers whether data_blocks.id_file appears to be a required owner pointer,
-- a representative file id for a data_object, or a stale/legacy column.

-- FOD runtime tables are created in the `fod` schema on the standard local setup.
-- Keep the probe read-only, but run all unqualified table references against FOD.
SET search_path TO fod, public;

SELECT now() AS captured_at;

SELECT
    current_database() AS database_name,
    current_schema() AS current_schema,
    current_setting('search_path') AS search_path;

SELECT 'data_blocks columns' AS section;

SELECT
    ordinal_position,
    column_name,
    data_type,
    is_nullable,
    column_default
FROM information_schema.columns
WHERE table_name = 'data_blocks'
ORDER BY ordinal_position;

SELECT 'data_blocks constraints' AS section;

SELECT
    con.conname,
    con.contype,
    pg_get_constraintdef(con.oid) AS definition
FROM pg_constraint con
JOIN pg_class rel ON rel.oid = con.conrelid
WHERE rel.relname = 'data_blocks'
ORDER BY con.conname;

SELECT 'data_blocks indexes' AS section;

SELECT
    indexname,
    indexdef
FROM pg_indexes
WHERE tablename = 'data_blocks'
ORDER BY indexname;

SELECT 'row counts' AS section;

SELECT
    (SELECT count(*) FROM data_blocks) AS data_blocks_rows,
    (SELECT count(DISTINCT data_object_id) FROM data_blocks) AS data_blocks_distinct_data_objects,
    (SELECT count(DISTINCT id_file) FROM data_blocks) AS data_blocks_distinct_id_files,
    (SELECT count(*) FROM files) AS files_rows,
    (SELECT count(DISTINCT data_object_id) FROM files WHERE data_object_id IS NOT NULL) AS files_distinct_data_objects;

SELECT 'block ownership consistency' AS section;

SELECT
    count(*) AS data_blocks_without_matching_file_owner
FROM data_blocks db
WHERE NOT EXISTS (
    SELECT 1
    FROM files f
    WHERE f.id_file = db.id_file
      AND f.data_object_id = db.data_object_id
);

SELECT
    count(*) AS data_blocks_with_matching_file_owner
FROM data_blocks db
WHERE EXISTS (
    SELECT 1
    FROM files f
    WHERE f.id_file = db.id_file
      AND f.data_object_id = db.data_object_id
);

SELECT 'data_objects shared by multiple files' AS section;

WITH file_owners AS (
    SELECT
        data_object_id,
        count(*) AS file_count
    FROM files
    WHERE data_object_id IS NOT NULL
    GROUP BY data_object_id
)
SELECT
    count(*) AS data_objects_with_multiple_files,
    coalesce(sum(file_count), 0) AS total_files_on_shared_objects
FROM file_owners
WHERE file_count > 1;

SELECT 'data_blocks id_file distribution per data_object' AS section;

WITH per_object AS (
    SELECT
        data_object_id,
        count(*) AS block_rows,
        count(DISTINCT id_file) AS block_id_file_count,
        min(id_file) AS min_block_id_file,
        max(id_file) AS max_block_id_file
    FROM data_blocks
    GROUP BY data_object_id
)
SELECT
    count(*) AS data_objects_with_blocks,
    count(*) FILTER (WHERE block_id_file_count = 1) AS objects_with_single_block_id_file,
    count(*) FILTER (WHERE block_id_file_count > 1) AS objects_with_multiple_block_id_files,
    max(block_rows) AS max_blocks_per_object,
    max(block_id_file_count) AS max_block_id_file_count_per_object
FROM per_object;

SELECT 'sample shared data_objects and block id_file mapping' AS section;

WITH file_owners AS (
    SELECT
        data_object_id,
        count(*) AS file_count,
        min(id_file) AS min_file_id,
        max(id_file) AS max_file_id
    FROM files
    WHERE data_object_id IS NOT NULL
    GROUP BY data_object_id
),
block_owners AS (
    SELECT
        data_object_id,
        count(*) AS block_rows,
        count(DISTINCT id_file) AS block_id_file_count,
        min(id_file) AS min_block_id_file,
        max(id_file) AS max_block_id_file
    FROM data_blocks
    GROUP BY data_object_id
)
SELECT
    f.data_object_id,
    f.file_count,
    f.min_file_id,
    f.max_file_id,
    b.block_rows,
    b.block_id_file_count,
    b.min_block_id_file,
    b.max_block_id_file,
    (b.min_block_id_file BETWEEN f.min_file_id AND f.max_file_id) AS block_id_file_in_file_id_range
FROM file_owners f
JOIN block_owners b USING (data_object_id)
WHERE f.file_count > 1
ORDER BY f.file_count DESC, b.block_rows DESC, f.data_object_id
LIMIT 25;

SELECT 'candidate optimization safety hints' AS section;

WITH file_owners AS (
    SELECT
        data_object_id,
        count(*) AS file_count
    FROM files
    WHERE data_object_id IS NOT NULL
    GROUP BY data_object_id
),
block_owners AS (
    SELECT
        data_object_id,
        count(DISTINCT id_file) AS block_id_file_count
    FROM data_blocks
    GROUP BY data_object_id
)
SELECT
    count(*) FILTER (WHERE f.file_count > 1) AS shared_data_objects,
    count(*) FILTER (WHERE f.file_count > 1 AND b.block_id_file_count = 1) AS shared_objects_with_single_block_id_file,
    count(*) FILTER (WHERE f.file_count > 1 AND b.block_id_file_count > 1) AS shared_objects_with_multiple_block_id_files
FROM file_owners f
LEFT JOIN block_owners b USING (data_object_id);

SELECT 'interpretation guide' AS section;

SELECT
    'If shared_data_objects > 0 and shared_objects_with_single_block_id_file is also high, data_blocks.id_file likely behaves like a representative/stale owner rather than a strict owner for every file. In that case, DO NOTHING or data-only updates may be possible, but only after correctness tests prove semantics.' AS note
UNION ALL
SELECT
    'If data_blocks_without_matching_file_owner > 0, data_blocks.id_file already does not strictly match the current files owner mapping and should not be used as the only correctness guard.'
UNION ALL
SELECT
    'If objects_with_multiple_block_id_files > 0, id_file varies inside a single data_object block set, so removing or ignoring id_file in merge semantics is higher risk.';
