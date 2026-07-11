BEGIN;

SET search_path TO fod, public;

LOCK TABLE files, data_objects, data_blocks, data_extents, copy_block_crc
    IN ACCESS EXCLUSIVE MODE;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM files f
        LEFT JOIN data_objects o ON o.id_data_object = f.data_object_id
        WHERE o.id_data_object IS NULL
    ) OR EXISTS (
        SELECT 1
        FROM data_blocks b
        LEFT JOIN data_objects o ON o.id_data_object = b.data_object_id
        WHERE o.id_data_object IS NULL
    ) OR EXISTS (
        SELECT 1
        FROM data_extents e
        LEFT JOIN data_objects o ON o.id_data_object = e.data_object_id
        WHERE o.id_data_object IS NULL
    ) OR EXISTS (
        SELECT 1
        FROM copy_block_crc c
        LEFT JOIN data_objects o ON o.id_data_object = c.data_object_id
        WHERE o.id_data_object IS NULL
    ) THEN
        RAISE EXCEPTION 'payload ownership migration found orphaned data_object_id values';
    END IF;
END
$$;

ALTER TABLE files
    DROP CONSTRAINT IF EXISTS files_data_object_id_fkey;
ALTER TABLE files
    ADD CONSTRAINT files_data_object_id_fkey
    FOREIGN KEY (data_object_id)
    REFERENCES data_objects(id_data_object);

ALTER TABLE data_blocks
    DROP CONSTRAINT IF EXISTS data_blocks_id_file_fkey;
ALTER TABLE data_extents
    DROP CONSTRAINT IF EXISTS data_extents_id_file_fkey;
ALTER TABLE copy_block_crc
    DROP CONSTRAINT IF EXISTS copy_block_crc_id_file_fkey;
ALTER TABLE data_blocks
    DROP CONSTRAINT IF EXISTS data_blocks_data_object_id_fkey;
ALTER TABLE data_extents
    DROP CONSTRAINT IF EXISTS data_extents_data_object_id_fkey;
ALTER TABLE copy_block_crc
    DROP CONSTRAINT IF EXISTS copy_block_crc_data_object_id_fkey;

ALTER TABLE data_blocks
    DROP COLUMN IF EXISTS id_file;
ALTER TABLE data_extents
    DROP COLUMN IF EXISTS id_file;
ALTER TABLE copy_block_crc
    DROP COLUMN IF EXISTS id_file;

ALTER TABLE data_blocks
    ADD CONSTRAINT data_blocks_data_object_id_fkey
    FOREIGN KEY (data_object_id)
    REFERENCES data_objects(id_data_object)
    ON DELETE CASCADE;

ALTER TABLE data_extents
    ADD CONSTRAINT data_extents_data_object_id_fkey
    FOREIGN KEY (data_object_id)
    REFERENCES data_objects(id_data_object)
    ON DELETE CASCADE;

ALTER TABLE copy_block_crc
    ADD CONSTRAINT copy_block_crc_data_object_id_fkey
    FOREIGN KEY (data_object_id)
    REFERENCES data_objects(id_data_object)
    ON DELETE CASCADE;

CREATE UNIQUE INDEX IF NOT EXISTS idx_copy_block_crc_object_order
    ON copy_block_crc (data_object_id, _order);

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'copy_block_crc'::regclass
          AND contype = 'p'
    ) THEN
        DROP INDEX IF EXISTS idx_copy_block_crc_object_order;
    ELSE
        ALTER TABLE copy_block_crc
            ADD CONSTRAINT copy_block_crc_pkey
            PRIMARY KEY USING INDEX idx_copy_block_crc_object_order;
    END IF;
END
$$;

COMMIT;
