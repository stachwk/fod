-- Keep indexer path uniqueness while avoiding oversized text keys in B-tree.
-- Digest indexes only narrow candidates; Rust/PostgreSQL transactions always
-- compare the complete source name or path before updating/inserting.
ALTER TABLE index_sources
    DROP CONSTRAINT IF EXISTS index_sources_name_key;
ALTER TABLE index_files
    DROP CONSTRAINT IF EXISTS index_files_id_index_source_path_key;

CREATE INDEX IF NOT EXISTS idx_index_sources_name_md5
    ON index_sources (md5(name));
CREATE INDEX IF NOT EXISTS idx_index_files_source_path_md5
    ON index_files (id_index_source, md5(path));

-- Snapshot filtering searches path substrings and is paged by file ID, so the
-- previous full-path B-tree did not serve those queries and rejected long paths.
DROP INDEX IF EXISTS idx_index_catalog_snapshot_files_path;
DROP INDEX IF EXISTS idx_index_catalog_snapshot_files_source;
CREATE INDEX IF NOT EXISTS idx_index_catalog_snapshot_files_source
    ON index_catalog_snapshot_files (id_catalog_snapshot, md5(source_name), id_file);
