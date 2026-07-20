SET search_path TO fod, public;

CREATE TABLE IF NOT EXISTS index_catalog_snapshots (
    id_catalog_snapshot SERIAL PRIMARY KEY,
    request_token TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL,
    source_filter TEXT,
    file_count BIGINT NOT NULL DEFAULT 0,
    total_bytes BIGINT NOT NULL DEFAULT 0,
    max_file_id BIGINT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS index_catalog_snapshot_files (
    id_catalog_snapshot INTEGER NOT NULL
        REFERENCES index_catalog_snapshots(id_catalog_snapshot) ON DELETE CASCADE,
    id_file INTEGER NOT NULL,
    id_index_source INTEGER NOT NULL,
    source_name TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    source_root TEXT NOT NULL,
    path TEXT NOT NULL,
    size BIGINT NOT NULL,
    mtime_ns BIGINT,
    inode BIGINT,
    device BIGINT,
    file_kind TEXT NOT NULL,
    scan_status TEXT NOT NULL,
    source_changed BOOLEAN NOT NULL,
    hash_algorithm TEXT,
    full_hash BYTEA,
    hash_status TEXT,
    id_scan_run INTEGER,
    file_created_at TIMESTAMP NOT NULL,
    file_updated_at TIMESTAMP NOT NULL,
    PRIMARY KEY (id_catalog_snapshot, id_file)
);

CREATE INDEX IF NOT EXISTS idx_index_catalog_snapshots_created
    ON index_catalog_snapshots (id_catalog_snapshot DESC);
CREATE INDEX IF NOT EXISTS idx_index_catalog_snapshot_files_source
    ON index_catalog_snapshot_files (id_catalog_snapshot, source_name, id_file);
CREATE INDEX IF NOT EXISTS idx_index_catalog_snapshot_files_path
    ON index_catalog_snapshot_files (id_catalog_snapshot, path);
CREATE INDEX IF NOT EXISTS idx_index_catalog_snapshot_files_hash_status
    ON index_catalog_snapshot_files (id_catalog_snapshot, hash_status, id_file);
