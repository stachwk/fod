CREATE UNIQUE INDEX IF NOT EXISTS idx_data_objects_file_size_content_hash
    ON data_objects (file_size, content_hash);
