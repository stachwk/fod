-- Fresh-install FOD bootstrap. Upgrade-only migrations stay in migrations/0001_*.sql.
CREATE SCHEMA IF NOT EXISTS fod;
SET search_path TO fod, public;

CREATE TABLE IF NOT EXISTS directories (
    id_directory SERIAL PRIMARY KEY,
    id_parent INTEGER REFERENCES directories(id_directory),
    name VARCHAR(255) NOT NULL,
    mode VARCHAR(6) NOT NULL,
    uid INTEGER NOT NULL,
    gid INTEGER NOT NULL,
    inode_seed TEXT NOT NULL,
    modification_date TIMESTAMP NOT NULL,
    access_date TIMESTAMP NOT NULL,
    change_date TIMESTAMP NOT NULL,
    creation_date TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS data_objects (
    id_data_object SERIAL PRIMARY KEY,
    file_size BIGINT NOT NULL DEFAULT 0,
    content_hash BYTEA,
    reference_count INTEGER NOT NULL DEFAULT 1,
    creation_date TIMESTAMP NOT NULL DEFAULT NOW(),
    modification_date TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS files (
    id_file SERIAL PRIMARY KEY,
    data_object_id INTEGER NOT NULL REFERENCES data_objects(id_data_object),
    id_directory INTEGER REFERENCES directories(id_directory),
    name VARCHAR(255) NOT NULL,
    size BIGINT NOT NULL,
    mode VARCHAR(6) NOT NULL,
    uid INTEGER NOT NULL,
    gid INTEGER NOT NULL,
    inode_seed TEXT NOT NULL,
    modification_date TIMESTAMP NOT NULL,
    access_date TIMESTAMP NOT NULL,
    change_date TIMESTAMP NOT NULL,
    creation_date TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS special_files (
    id_file INTEGER PRIMARY KEY REFERENCES files(id_file) ON DELETE CASCADE,
    file_type VARCHAR(10) NOT NULL,
    rdev_major INTEGER NOT NULL DEFAULT 0,
    rdev_minor INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS hardlinks (
    id_hardlink SERIAL PRIMARY KEY,
    id_file INTEGER NOT NULL REFERENCES files(id_file) ON DELETE CASCADE,
    id_directory INTEGER REFERENCES directories(id_directory),
    name VARCHAR(255) NOT NULL,
    uid INTEGER NOT NULL,
    gid INTEGER NOT NULL,
    modification_date TIMESTAMP NOT NULL,
    access_date TIMESTAMP NOT NULL,
    change_date TIMESTAMP NOT NULL,
    creation_date TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS symlinks (
    id_symlink SERIAL PRIMARY KEY,
    id_parent INTEGER REFERENCES directories(id_directory),
    name VARCHAR(255) NOT NULL,
    target TEXT NOT NULL,
    uid INTEGER NOT NULL,
    gid INTEGER NOT NULL,
    inode_seed TEXT NOT NULL,
    modification_date TIMESTAMP NOT NULL,
    access_date TIMESTAMP NOT NULL,
    change_date TIMESTAMP NOT NULL,
    creation_date TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS data_blocks (
    id_block SERIAL PRIMARY KEY,
    data_object_id INTEGER NOT NULL REFERENCES data_objects(id_data_object) ON DELETE CASCADE,
    _order INTEGER NOT NULL,
    data BYTEA NOT NULL
);

CREATE TABLE IF NOT EXISTS data_extents (
    id_extent SERIAL PRIMARY KEY,
    data_object_id INTEGER NOT NULL REFERENCES data_objects(id_data_object) ON DELETE CASCADE,
    start_block BIGINT NOT NULL,
    block_count BIGINT NOT NULL,
    used_bytes BIGINT NOT NULL,
    payload BYTEA NOT NULL,
    creation_date TIMESTAMP NOT NULL DEFAULT NOW(),
    modification_date TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS config (
    key VARCHAR(50) PRIMARY KEY,
    value BIGINT
);

CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL,
    applied_at TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS schema_admin (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    password_hash TEXT NOT NULL,
    password_salt TEXT NOT NULL,
    password_iterations INTEGER NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS journal (
    id_entry SERIAL PRIMARY KEY,
    id_user INTEGER,
    id_directory INTEGER REFERENCES directories(id_directory) ON DELETE SET NULL,
    id_file INTEGER REFERENCES files(id_file) ON DELETE SET NULL,
    action TEXT NOT NULL,
    date_time TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS xattrs (
    id SERIAL PRIMARY KEY,
    owner_kind VARCHAR(20) NOT NULL,
    owner_id INTEGER NOT NULL,
    name VARCHAR(255) NOT NULL,
    value BYTEA NOT NULL,
    UNIQUE(owner_kind, owner_id, name)
);

CREATE TABLE IF NOT EXISTS lock_leases (
    id_lock SERIAL PRIMARY KEY,
    resource_kind VARCHAR(20) NOT NULL,
    resource_id BIGINT NOT NULL,
    owner_key NUMERIC(20,0) NOT NULL,
    lease_kind VARCHAR(20) NOT NULL,
    lock_type INTEGER NOT NULL,
    lease_expires_at TIMESTAMP NOT NULL,
    heartbeat_at TIMESTAMP NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    UNIQUE(resource_kind, resource_id, owner_key, lease_kind)
);

INSERT INTO directories (id_parent, name, mode, uid, gid, inode_seed, modification_date, access_date, change_date, creation_date)
SELECT NULL, '/', '755', 0, 0, 'root', NOW(), NOW(), NOW(), NOW()
WHERE NOT EXISTS (
    SELECT 1 FROM directories WHERE id_parent IS NULL AND name = '/'
);

INSERT INTO directories (id_parent, name, mode, uid, gid, inode_seed, modification_date, access_date, change_date, creation_date)
SELECT NULL, '.Trash-1000', '755', 0, 0, 'pseudo:.Trash-1000', NOW(), NOW(), NOW(), NOW()
WHERE NOT EXISTS (
    SELECT 1 FROM directories WHERE id_parent IS NULL AND name = '.Trash-1000'
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_data_blocks_object_order ON data_blocks (data_object_id, _order);
CREATE UNIQUE INDEX IF NOT EXISTS idx_data_extents_object_start ON data_extents (data_object_id, start_block);
CREATE INDEX IF NOT EXISTS idx_data_extents_data_object_id ON data_extents (data_object_id);
CREATE UNIQUE INDEX IF NOT EXISTS uniq_directories_root_name ON directories (name) WHERE id_parent IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_directories_parent_name ON directories (id_parent, name) WHERE id_parent IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_files_root_name ON files (name) WHERE id_directory IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_files_parent_name ON files (id_directory, name) WHERE id_directory IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_hardlinks_root_name ON hardlinks (name) WHERE id_directory IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_hardlinks_parent_name ON hardlinks (id_directory, name) WHERE id_directory IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_symlinks_root_name ON symlinks (name) WHERE id_parent IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_symlinks_parent_name ON symlinks (id_parent, name) WHERE id_parent IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_directories_parent_id ON directories (id_parent);
CREATE INDEX IF NOT EXISTS idx_files_directory_id ON files (id_directory);
CREATE INDEX IF NOT EXISTS idx_hardlinks_directory_id ON hardlinks (id_directory);
CREATE INDEX IF NOT EXISTS idx_symlinks_parent_id ON symlinks (id_parent);
CREATE INDEX IF NOT EXISTS idx_hardlinks_file_id ON hardlinks (id_file);
CREATE INDEX IF NOT EXISTS idx_xattrs_owner ON xattrs (owner_kind, owner_id);
CREATE INDEX IF NOT EXISTS idx_lock_leases_resource ON lock_leases (resource_kind, resource_id, lease_kind);
CREATE INDEX IF NOT EXISTS idx_lock_leases_expires ON lock_leases (lease_expires_at);

CREATE TABLE IF NOT EXISTS lock_range_leases (
    id_lock SERIAL PRIMARY KEY,
    resource_kind VARCHAR(20) NOT NULL,
    resource_id BIGINT NOT NULL,
    owner_key NUMERIC(20,0) NOT NULL,
    lock_type INTEGER NOT NULL,
    range_start BIGINT NOT NULL,
    range_end BIGINT NULL,
    lease_expires_at TIMESTAMP NOT NULL,
    heartbeat_at TIMESTAMP NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_lock_range_leases_resource ON lock_range_leases (resource_kind, resource_id);
CREATE INDEX IF NOT EXISTS idx_lock_range_leases_expires ON lock_range_leases (lease_expires_at);

CREATE TABLE IF NOT EXISTS copy_block_crc (
    data_object_id INTEGER NOT NULL REFERENCES data_objects(id_data_object) ON DELETE CASCADE,
    _order INTEGER NOT NULL,
    crc32 BIGINT NOT NULL,
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY (data_object_id, _order)
);

CREATE TABLE IF NOT EXISTS data_object_request_tokens (
    request_token TEXT PRIMARY KEY,
    id_data_object INTEGER NOT NULL REFERENCES data_objects(id_data_object) ON DELETE CASCADE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS hardlink_promotion_request_tokens (
    request_token TEXT PRIMARY KEY,
    id_file INTEGER NOT NULL REFERENCES files(id_file) ON DELETE CASCADE,
    did_promote BOOLEAN NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS payload_capacity_reservations (
    request_token TEXT PRIMARY KEY,
    reserved_bytes BIGINT NOT NULL CHECK (reserved_bytes > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_payload_capacity_reservations_expires
    ON payload_capacity_reservations (expires_at);

CREATE INDEX IF NOT EXISTS idx_files_data_object_id ON files (data_object_id);
CREATE INDEX IF NOT EXISTS idx_data_blocks_data_object_id ON data_blocks (data_object_id);

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
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    request_token TEXT NOT NULL UNIQUE,
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
    request_token TEXT NOT NULL UNIQUE,
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
CREATE INDEX IF NOT EXISTS idx_copy_block_crc_data_object_id ON copy_block_crc (data_object_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_data_objects_file_size_content_hash
    ON data_objects (file_size, content_hash);

CREATE TABLE IF NOT EXISTS client_sessions (
    session_id BIGSERIAL PRIMARY KEY,
    host_name VARCHAR(255) NOT NULL,
    mountpoint TEXT NOT NULL,
    mount_mode VARCHAR(20) NOT NULL,
    lock_backend VARCHAR(20) NOT NULL,
    pid BIGINT NOT NULL,
    lease_expires_at TIMESTAMP NOT NULL,
    heartbeat_at TIMESTAMP NOT NULL,
    last_lock_at TIMESTAMP NULL,
    last_write_at TIMESTAMP NULL,
    started_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_client_sessions_expires
    ON client_sessions (lease_expires_at);

CREATE TABLE IF NOT EXISTS client_session_owner_keys (
    session_id BIGINT NOT NULL REFERENCES client_sessions(session_id) ON DELETE CASCADE,
    owner_key NUMERIC(20,0) NOT NULL,
    first_seen_at TIMESTAMP NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY(session_id, owner_key)
);

CREATE INDEX IF NOT EXISTS idx_client_session_owner_keys_owner
    ON client_session_owner_keys (owner_key);

CREATE INDEX IF NOT EXISTS idx_client_session_owner_keys_last_seen
    ON client_session_owner_keys (last_seen_at);

ALTER TABLE IF EXISTS lock_leases
    ADD COLUMN IF NOT EXISTS session_id BIGINT DEFAULT 0;

UPDATE lock_leases
SET session_id = 0
WHERE session_id IS NULL;

ALTER TABLE IF EXISTS lock_leases
    ALTER COLUMN session_id SET DEFAULT 0;

ALTER TABLE IF EXISTS lock_leases
    ALTER COLUMN session_id SET NOT NULL;

ALTER TABLE IF EXISTS lock_leases
    DROP CONSTRAINT IF EXISTS lock_leases_resource_kind_resource_id_owner_key_lease_kind_key;

CREATE INDEX IF NOT EXISTS idx_lock_leases_session
    ON lock_leases (session_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_lock_leases_identity
    ON lock_leases (resource_kind, resource_id, session_id, owner_key, lease_kind);

ALTER TABLE IF EXISTS lock_range_leases
    ADD COLUMN IF NOT EXISTS session_id BIGINT DEFAULT 0;

UPDATE lock_range_leases
SET session_id = 0
WHERE session_id IS NULL;

ALTER TABLE IF EXISTS lock_range_leases
    ALTER COLUMN session_id SET DEFAULT 0;

ALTER TABLE IF EXISTS lock_range_leases
    ALTER COLUMN session_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_lock_range_leases_session
    ON lock_range_leases (session_id);

CREATE OR REPLACE FUNCTION fod_prune_client_session_lock_leases()
RETURNS trigger AS $$
BEGIN
    DELETE FROM lock_leases
    WHERE session_id = OLD.session_id;

    DELETE FROM lock_range_leases
    WHERE session_id = OLD.session_id;

    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS fod_client_sessions_prune_lock_leases ON client_sessions;

CREATE TRIGGER fod_client_sessions_prune_lock_leases
BEFORE DELETE ON client_sessions
FOR EACH ROW
EXECUTE FUNCTION fod_prune_client_session_lock_leases();

UPDATE lock_leases
SET session_id = mapping.session_id
FROM (
    SELECT DISTINCT ON (owner_key)
        owner_key,
        session_id
    FROM client_session_owner_keys
    ORDER BY owner_key, last_seen_at DESC, session_id DESC
) AS mapping
WHERE lock_leases.session_id = 0
  AND lock_leases.owner_key = mapping.owner_key;

UPDATE lock_range_leases
SET session_id = mapping.session_id
FROM (
    SELECT DISTINCT ON (owner_key)
        owner_key,
        session_id
    FROM client_session_owner_keys
    ORDER BY owner_key, last_seen_at DESC, session_id DESC
) AS mapping
WHERE lock_range_leases.session_id = 0
  AND lock_range_leases.owner_key = mapping.owner_key;

DELETE FROM lock_leases
WHERE session_id = 0
  AND lease_expires_at <= NOW();

DELETE FROM lock_range_leases
WHERE session_id = 0
  AND lease_expires_at <= NOW();

ALTER TABLE IF EXISTS lock_leases
    ALTER COLUMN session_id DROP DEFAULT;

ALTER TABLE IF EXISTS lock_range_leases
    ALTER COLUMN session_id DROP DEFAULT;

CREATE TABLE IF NOT EXISTS index_catalog_snapshots (
    id_catalog_snapshot SERIAL PRIMARY KEY,
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
