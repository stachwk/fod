SET search_path TO fod, public;

CREATE TABLE IF NOT EXISTS index_sources (
    id_index_source SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    root_path TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS index_scan_runs (
    id_scan_run SERIAL PRIMARY KEY,
    id_index_source INTEGER NOT NULL REFERENCES index_sources(id_index_source) ON DELETE CASCADE,
    started_at TIMESTAMP NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMP,
    status TEXT NOT NULL,
    error_message TEXT
);

CREATE TABLE IF NOT EXISTS index_files (
    id_file SERIAL PRIMARY KEY,
    id_index_source INTEGER NOT NULL REFERENCES index_sources(id_index_source) ON DELETE CASCADE,
    id_scan_run INTEGER REFERENCES index_scan_runs(id_scan_run) ON DELETE SET NULL,
    path TEXT NOT NULL,
    size BIGINT NOT NULL,
    mtime_ns BIGINT,
    inode BIGINT,
    device BIGINT,
    file_kind TEXT NOT NULL,
    scan_status TEXT NOT NULL,
    source_changed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    UNIQUE(id_index_source, path)
);

CREATE TABLE IF NOT EXISTS index_file_hashes (
    id_file INTEGER PRIMARY KEY REFERENCES index_files(id_file) ON DELETE CASCADE,
    hash_algorithm TEXT NOT NULL,
    partial_hash BYTEA,
    full_hash BYTEA,
    hash_status TEXT NOT NULL,
    observed_size BIGINT NOT NULL,
    observed_mtime_ns BIGINT,
    observed_inode BIGINT,
    observed_device BIGINT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS index_duplicate_sets (
    id_duplicate_set SERIAL PRIMARY KEY,
    hash_algorithm TEXT NOT NULL,
    full_hash BYTEA NOT NULL,
    file_size BIGINT NOT NULL,
    file_count INTEGER NOT NULL,
    total_bytes BIGINT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    UNIQUE(hash_algorithm, full_hash, file_size)
);

CREATE TABLE IF NOT EXISTS index_import_plans (
    id_import_plan SERIAL PRIMARY KEY,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    status TEXT NOT NULL,
    dry_run BOOLEAN NOT NULL DEFAULT TRUE,
    source_filter TEXT,
    scanned_file_count INTEGER NOT NULL DEFAULT 0,
    candidate_group_count INTEGER NOT NULL DEFAULT 0,
    confirmed_group_count INTEGER NOT NULL DEFAULT 0,
    unique_payload_count INTEGER NOT NULL DEFAULT 0,
    total_source_bytes BIGINT NOT NULL DEFAULT 0,
    estimated_import_bytes BIGINT NOT NULL DEFAULT 0,
    saved_bytes BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS index_import_plan_entries (
    id_import_plan_entry SERIAL PRIMARY KEY,
    id_import_plan INTEGER NOT NULL REFERENCES index_import_plans(id_import_plan) ON DELETE CASCADE,
    id_file INTEGER NOT NULL REFERENCES index_files(id_file) ON DELETE CASCADE,
    id_duplicate_set INTEGER REFERENCES index_duplicate_sets(id_duplicate_set) ON DELETE SET NULL,
    action TEXT NOT NULL,
    canonical_file_id INTEGER REFERENCES index_files(id_file),
    logical_path TEXT NOT NULL,
    source_path TEXT NOT NULL,
    size BIGINT NOT NULL,
    mtime_ns BIGINT,
    source_changed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_index_scan_runs_source ON index_scan_runs (id_index_source, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_index_files_source_status ON index_files (id_index_source, scan_status);
CREATE INDEX IF NOT EXISTS idx_index_files_source_size ON index_files (id_index_source, size);
CREATE INDEX IF NOT EXISTS idx_index_file_hashes_full ON index_file_hashes (hash_algorithm, full_hash);
CREATE INDEX IF NOT EXISTS idx_index_duplicate_sets_full ON index_duplicate_sets (hash_algorithm, full_hash, file_size);
CREATE INDEX IF NOT EXISTS idx_index_import_plans_status ON index_import_plans (status);
CREATE INDEX IF NOT EXISTS idx_index_import_plan_entries_plan ON index_import_plan_entries (id_import_plan);
CREATE INDEX IF NOT EXISTS idx_index_import_plan_entries_duplicate_set ON index_import_plan_entries (id_duplicate_set);
