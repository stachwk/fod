SET search_path TO fod, public;

CREATE TABLE IF NOT EXISTS data_extents (
    id_extent SERIAL PRIMARY KEY,
    id_file INTEGER NOT NULL REFERENCES files(id_file),
    data_object_id INTEGER NOT NULL,
    start_block BIGINT NOT NULL,
    block_count BIGINT NOT NULL,
    used_bytes BIGINT NOT NULL,
    payload BYTEA NOT NULL,
    creation_date TIMESTAMP NOT NULL DEFAULT NOW(),
    modification_date TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_data_extents_object_start ON data_extents (data_object_id, start_block);
CREATE INDEX IF NOT EXISTS idx_data_extents_data_object_id ON data_extents (data_object_id);
