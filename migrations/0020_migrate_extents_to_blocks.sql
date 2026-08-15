BEGIN;

SET search_path TO fod, public;

LOCK TABLE files, data_objects, data_blocks, data_extents, copy_block_crc
    IN ACCESS EXCLUSIVE MODE;

DO $$
DECLARE
    v_block_size BIGINT;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM data_extents) THEN
        RETURN;
    END IF;

    SELECT value
    INTO v_block_size
    FROM config
    WHERE key = 'block_size';

    IF v_block_size IS NULL OR v_block_size <= 0 OR v_block_size > 1073741824 THEN
        RAISE EXCEPTION 'extent migration requires config.block_size in range 1..1073741824, got %',
            v_block_size;
    END IF;

    CREATE TEMP TABLE fod_extent_migration_objects ON COMMIT DROP AS
    SELECT
        o.id_data_object AS data_object_id,
        o.file_size,
        CASE
            WHEN o.file_size = 0 THEN 0::NUMERIC
            ELSE floor((o.file_size::NUMERIC - 1) / v_block_size::NUMERIC) + 1
        END AS expected_blocks,
        COUNT(e.id_extent)::BIGINT AS extent_rows,
        COALESCE(SUM(e.block_count), 0)::NUMERIC AS extent_blocks,
        COALESCE(SUM(e.used_bytes), 0)::NUMERIC AS extent_bytes
    FROM data_objects o
    JOIN data_extents e ON e.data_object_id = o.id_data_object
    GROUP BY o.id_data_object, o.file_size;

    IF EXISTS (
        SELECT 1
        FROM data_extents e
        LEFT JOIN data_objects o ON o.id_data_object = e.data_object_id
        WHERE o.id_data_object IS NULL
    ) THEN
        RAISE EXCEPTION 'extent migration found orphaned data_extents.data_object_id values';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM fod_extent_migration_objects o
        JOIN files f ON f.data_object_id = o.data_object_id
        WHERE f.size <> o.file_size
    ) THEN
        RAISE EXCEPTION 'extent migration found file size values that differ from data_objects.file_size';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM fod_extent_migration_objects o
        WHERE o.file_size < 0
           OR o.expected_blocks > 2147483648::NUMERIC
           OR o.extent_bytes <> o.file_size::NUMERIC
           OR o.extent_blocks <> o.expected_blocks
    ) THEN
        RAISE EXCEPTION 'extent migration found extent coverage that does not match logical file size';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM data_extents e
        WHERE e.start_block < 0
           OR e.block_count <= 0
           OR e.used_bytes <= 0
           OR e.start_block > 2147483647
           OR e.block_count > 2147483648::BIGINT
           OR e.start_block > 2147483647::BIGINT - (e.block_count - 1)
           OR e.used_bytes::NUMERIC > e.block_count::NUMERIC * v_block_size::NUMERIC
           OR e.used_bytes::NUMERIC <= (e.block_count::NUMERIC - 1) * v_block_size::NUMERIC
           OR octet_length(e.payload)::NUMERIC <> e.used_bytes::NUMERIC
    ) THEN
        RAISE EXCEPTION 'extent migration found invalid extent row geometry or payload length';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM fod_extent_migration_objects o
        JOIN data_blocks b ON b.data_object_id = o.data_object_id
    ) THEN
        RAISE EXCEPTION 'extent migration refuses hybrid objects with both data_extents and data_blocks rows';
    END IF;

    CREATE TEMP TABLE fod_extent_migration_ordered ON COMMIT DROP AS
    SELECT
        e.id_extent,
        e.data_object_id,
        e.start_block,
        e.block_count,
        e.used_bytes,
        e.payload,
        o.file_size,
        COALESCE(
            SUM(e.block_count) OVER (
                PARTITION BY e.data_object_id
                ORDER BY e.start_block, e.id_extent
                ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING
            ),
            0
        )::NUMERIC AS expected_start_block
    FROM data_extents e
    JOIN fod_extent_migration_objects o ON o.data_object_id = e.data_object_id;

    IF EXISTS (
        SELECT 1
        FROM fod_extent_migration_ordered
        WHERE start_block::NUMERIC <> expected_start_block
    ) THEN
        RAISE EXCEPTION 'extent migration found a gap or overlap in extent block coverage';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM fod_extent_migration_ordered
        WHERE used_bytes::NUMERIC <> (
            LEAST(
                file_size::NUMERIC,
                (start_block::NUMERIC + block_count::NUMERIC) * v_block_size::NUMERIC
            ) - (start_block::NUMERIC * v_block_size::NUMERIC)
        )
    ) THEN
        RAISE EXCEPTION 'extent migration found extent used_bytes inconsistent with logical file size';
    END IF;

    CREATE TEMP TABLE fod_extent_migration_blocks ON COMMIT DROP AS
    SELECT
        e.data_object_id,
        (e.start_block + offsets.block_offset)::INTEGER AS _order,
        chunk.raw_data,
        CASE
            WHEN chunk.raw_data = decode(repeat('00', octet_length(chunk.raw_data)), 'hex') THEN NULL::BYTEA
            WHEN octet_length(chunk.raw_data) = v_block_size THEN chunk.raw_data
            ELSE chunk.raw_data ||
                decode(repeat('00', (v_block_size - octet_length(chunk.raw_data))::INTEGER), 'hex')
        END AS data
    FROM fod_extent_migration_ordered e
    CROSS JOIN LATERAL generate_series(
        0::BIGINT,
        e.block_count - 1
    ) AS offsets(block_offset)
    CROSS JOIN LATERAL (
        SELECT substring(
            e.payload
            FROM (offsets.block_offset * v_block_size + 1)::INTEGER
            FOR v_block_size::INTEGER
        ) AS raw_data
    ) AS chunk
    WHERE offsets.block_offset * v_block_size < e.used_bytes;

    IF EXISTS (
        SELECT 1
        FROM fod_extent_migration_blocks
        WHERE octet_length(raw_data) <= 0
           OR octet_length(raw_data) > v_block_size
           OR (data IS NOT NULL AND octet_length(data) <> v_block_size)
    ) THEN
        RAISE EXCEPTION 'extent migration generated invalid block candidates';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM (
            SELECT data_object_id, _order, COUNT(*) AS duplicate_count
            FROM fod_extent_migration_blocks
            GROUP BY data_object_id, _order
            HAVING COUNT(*) > 1
        ) duplicates
    ) THEN
        RAISE EXCEPTION 'extent migration generated duplicate data block positions';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM fod_extent_migration_objects o
        LEFT JOIN (
            SELECT data_object_id, COUNT(*)::NUMERIC AS block_count
            FROM fod_extent_migration_blocks
            GROUP BY data_object_id
        ) c ON c.data_object_id = o.data_object_id
        WHERE COALESCE(c.block_count, 0) <> o.expected_blocks
    ) THEN
        RAISE EXCEPTION 'extent migration generated block coverage that does not match logical file size';
    END IF;

    INSERT INTO data_blocks (data_object_id, _order, data)
    SELECT data_object_id, _order, data
    FROM fod_extent_migration_blocks
    WHERE data IS NOT NULL;

    IF EXISTS (
        SELECT 1
        FROM fod_extent_migration_blocks c
        WHERE c.data IS NOT NULL
          AND NOT EXISTS (
              SELECT 1
              FROM data_blocks b
              WHERE b.data_object_id = c.data_object_id
                AND b._order = c._order
                AND b.data = c.data
          )
    ) THEN
        RAISE EXCEPTION 'extent migration failed integrity validation for inserted data_blocks';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM fod_extent_migration_blocks c
        WHERE c.data IS NULL
          AND EXISTS (
              SELECT 1
              FROM data_blocks b
              WHERE b.data_object_id = c.data_object_id
                AND b._order = c._order
          )
    ) THEN
        RAISE EXCEPTION 'extent migration found persisted rows for canonical sparse zero blocks';
    END IF;

    DELETE FROM copy_block_crc c
    USING fod_extent_migration_objects o
    WHERE c.data_object_id = o.data_object_id;

    DELETE FROM data_extents e
    USING fod_extent_migration_objects o
    WHERE e.data_object_id = o.data_object_id;

    IF EXISTS (
        SELECT 1
        FROM data_extents e
        JOIN fod_extent_migration_objects o ON o.data_object_id = e.data_object_id
    ) THEN
        RAISE EXCEPTION 'extent migration failed to delete migrated data_extents rows';
    END IF;
END
$$;

COMMIT;
