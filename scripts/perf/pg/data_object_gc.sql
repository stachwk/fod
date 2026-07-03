SET search_path TO fod, public;

CREATE TEMP TABLE fod_gc_data_objects ON COMMIT DROP AS
SELECT d.id_data_object
FROM data_objects d
WHERE NOT EXISTS (
    SELECT 1
    FROM files f
    WHERE f.data_object_id = d.id_data_object
)
ORDER BY d.id_data_object
LIMIT :gc_limit;

SELECT 'candidate_data_objects=' || COUNT(*) FROM fod_gc_data_objects;

WITH deleted AS (
    DELETE FROM copy_block_crc c
    USING fod_gc_data_objects g
    WHERE c.data_object_id = g.id_data_object
    RETURNING 1
)
SELECT 'deleted_copy_block_crc=' || COUNT(*) FROM deleted;

WITH deleted AS (
    DELETE FROM data_extents e
    USING fod_gc_data_objects g
    WHERE e.data_object_id = g.id_data_object
    RETURNING 1
)
SELECT 'deleted_data_extents=' || COUNT(*) FROM deleted;

WITH deleted AS (
    DELETE FROM data_blocks b
    USING fod_gc_data_objects g
    WHERE b.data_object_id = g.id_data_object
    RETURNING 1
)
SELECT 'deleted_data_blocks=' || COUNT(*) FROM deleted;

WITH deleted AS (
    DELETE FROM data_objects d
    USING fod_gc_data_objects g
    WHERE d.id_data_object = g.id_data_object
      AND NOT EXISTS (
          SELECT 1
          FROM files f
          WHERE f.data_object_id = d.id_data_object
      )
    RETURNING 1
)
SELECT 'deleted_data_objects=' || COUNT(*) FROM deleted;
