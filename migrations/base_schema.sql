-- Fresh-install FOD bootstrap. Upgrade-only migrations stay in migrations/0001_*.sql.
CREATE SCHEMA IF NOT EXISTS fod;
SET search_path TO fod, public;

CREATE TABLE IF NOT EXISTS directories (
    id_directory BIGSERIAL PRIMARY KEY,
    id_parent BIGINT REFERENCES directories(id_directory),
    name VARCHAR(255) NOT NULL
        CONSTRAINT directories_name_octet_length_check CHECK (octet_length(name) <= 255),
    mode VARCHAR(6) NOT NULL
        CONSTRAINT directories_mode_octal_check CHECK (mode ~ '^(0o)?[0-7]{1,4}$'),
    uid BIGINT NOT NULL CONSTRAINT directories_uid_u32_check CHECK (uid BETWEEN 0 AND 4294967295),
    gid BIGINT NOT NULL CONSTRAINT directories_gid_u32_check CHECK (gid BETWEEN 0 AND 4294967295),
    inode_seed TEXT NOT NULL,
    modification_date TIMESTAMP NOT NULL,
    access_date TIMESTAMP NOT NULL,
    change_date TIMESTAMP NOT NULL,
    creation_date TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS data_objects (
    id_data_object BIGSERIAL PRIMARY KEY,
    file_size BIGINT NOT NULL DEFAULT 0
        CONSTRAINT data_objects_file_size_nonnegative_check CHECK (file_size >= 0),
    content_hash BYTEA,
    reference_count BIGINT NOT NULL DEFAULT 1
        CONSTRAINT data_objects_reference_count_nonnegative_check CHECK (reference_count >= 0),
    creation_date TIMESTAMP NOT NULL DEFAULT NOW(),
    modification_date TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS files (
    id_file BIGSERIAL PRIMARY KEY,
    data_object_id BIGINT NOT NULL REFERENCES data_objects(id_data_object),
    content_generation BIGINT NOT NULL DEFAULT 1,
    id_directory BIGINT REFERENCES directories(id_directory),
    name VARCHAR(255) NOT NULL
        CONSTRAINT files_name_octet_length_check CHECK (octet_length(name) <= 255),
    size BIGINT NOT NULL CONSTRAINT files_size_nonnegative_check CHECK (size >= 0),
    mode VARCHAR(6) NOT NULL
        CONSTRAINT files_mode_octal_check CHECK (mode ~ '^(0o)?[0-7]{1,4}$'),
    uid BIGINT NOT NULL CONSTRAINT files_uid_u32_check CHECK (uid BETWEEN 0 AND 4294967295),
    gid BIGINT NOT NULL CONSTRAINT files_gid_u32_check CHECK (gid BETWEEN 0 AND 4294967295),
    inode_seed TEXT NOT NULL,
    unlinked BOOLEAN NOT NULL DEFAULT FALSE,
    modification_date TIMESTAMP NOT NULL,
    access_date TIMESTAMP NOT NULL,
    change_date TIMESTAMP NOT NULL,
    creation_date TIMESTAMP NOT NULL
);

-- Keep fresh installs aligned with migrations 0026 and 0027: content changes
-- advance this value explicitly in their PostgreSQL transaction, while
-- metadata-only updates leave it unchanged.
CREATE OR REPLACE FUNCTION fod_bump_file_content_generation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.content_generation IS DISTINCT FROM OLD.content_generation THEN
        IF OLD.content_generation >= 9223372036854775807 THEN
            RAISE EXCEPTION 'FOD content generation exhausted for file %', OLD.id_file;
        END IF;
        IF NEW.content_generation <> OLD.content_generation + 1 THEN
            RAISE EXCEPTION 'FOD content generation must advance by exactly one for file %', OLD.id_file;
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_fod_file_content_generation ON files;
CREATE TRIGGER trg_fod_file_content_generation
BEFORE UPDATE ON files
FOR EACH ROW
EXECUTE FUNCTION fod_bump_file_content_generation();

CREATE TABLE IF NOT EXISTS special_files (
    id_file BIGINT PRIMARY KEY REFERENCES files(id_file) ON DELETE CASCADE,
    file_type VARCHAR(10) NOT NULL,
    rdev_major BIGINT NOT NULL DEFAULT 0
        CONSTRAINT special_files_rdev_major_u32_check CHECK (rdev_major BETWEEN 0 AND 4294967295),
    rdev_minor BIGINT NOT NULL DEFAULT 0
        CONSTRAINT special_files_rdev_minor_u32_check CHECK (rdev_minor BETWEEN 0 AND 4294967295)
);

CREATE TABLE IF NOT EXISTS hardlinks (
    id_hardlink BIGSERIAL PRIMARY KEY,
    id_file BIGINT NOT NULL REFERENCES files(id_file) ON DELETE CASCADE,
    id_directory BIGINT REFERENCES directories(id_directory),
    name VARCHAR(255) NOT NULL
        CONSTRAINT hardlinks_name_octet_length_check CHECK (octet_length(name) <= 255),
    uid BIGINT NOT NULL CONSTRAINT hardlinks_uid_u32_check CHECK (uid BETWEEN 0 AND 4294967295),
    gid BIGINT NOT NULL CONSTRAINT hardlinks_gid_u32_check CHECK (gid BETWEEN 0 AND 4294967295),
    modification_date TIMESTAMP NOT NULL,
    access_date TIMESTAMP NOT NULL,
    change_date TIMESTAMP NOT NULL,
    creation_date TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS symlinks (
    id_symlink BIGSERIAL PRIMARY KEY,
    id_parent BIGINT REFERENCES directories(id_directory),
    name VARCHAR(255) NOT NULL
        CONSTRAINT symlinks_name_octet_length_check CHECK (octet_length(name) <= 255),
    target TEXT NOT NULL,
    uid BIGINT NOT NULL CONSTRAINT symlinks_uid_u32_check CHECK (uid BETWEEN 0 AND 4294967295),
    gid BIGINT NOT NULL CONSTRAINT symlinks_gid_u32_check CHECK (gid BETWEEN 0 AND 4294967295),
    inode_seed TEXT NOT NULL,
    modification_date TIMESTAMP NOT NULL,
    access_date TIMESTAMP NOT NULL,
    change_date TIMESTAMP NOT NULL,
    creation_date TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS symlink_hardlinks (
    id_symlink_hardlink BIGSERIAL PRIMARY KEY,
    id_symlink BIGINT NOT NULL REFERENCES symlinks(id_symlink),
    id_parent BIGINT REFERENCES directories(id_directory),
    name VARCHAR(255) NOT NULL
        CONSTRAINT symlink_hardlinks_name_octet_length_check CHECK (octet_length(name) <= 255)
);

CREATE TABLE IF NOT EXISTS data_blocks (
    id_block BIGSERIAL PRIMARY KEY,
    data_object_id BIGINT NOT NULL REFERENCES data_objects(id_data_object) ON DELETE CASCADE,
    _order BIGINT NOT NULL CONSTRAINT data_blocks_order_nonnegative_check CHECK (_order >= 0),
    data BYTEA NOT NULL
);

CREATE TABLE IF NOT EXISTS config (
    key VARCHAR(50) PRIMARY KEY,
    value BIGINT
);

CREATE TABLE IF NOT EXISTS schema_version (
    schema_number INTEGER NOT NULL,
    applied_at TIMESTAMP NOT NULL,
    minimum_client_version TEXT NOT NULL DEFAULT '0.0.0',
    upgrade_in_progress BOOLEAN NOT NULL DEFAULT FALSE
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
    id_entry BIGSERIAL PRIMARY KEY,
    id_user BIGINT CONSTRAINT journal_user_u32_check CHECK (id_user BETWEEN 0 AND 4294967295),
    id_directory BIGINT REFERENCES directories(id_directory) ON DELETE SET NULL,
    id_file BIGINT REFERENCES files(id_file) ON DELETE SET NULL,
    action TEXT NOT NULL,
    date_time TIMESTAMP NOT NULL
);

CREATE TABLE IF NOT EXISTS xattrs (
    id BIGSERIAL PRIMARY KEY,
    owner_kind VARCHAR(20) NOT NULL,
    owner_id BIGINT NOT NULL,
    name VARCHAR(255) NOT NULL
        CONSTRAINT xattrs_name_octet_length_check CHECK (octet_length(name) <= 255),
    value BYTEA NOT NULL,
    UNIQUE(owner_kind, owner_id, name)
);

CREATE TABLE IF NOT EXISTS lock_leases (
    id_lock BIGSERIAL PRIMARY KEY,
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
CREATE UNIQUE INDEX IF NOT EXISTS uniq_directories_root_name ON directories (name) WHERE id_parent IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_directories_parent_name ON directories (id_parent, name) WHERE id_parent IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_files_root_name ON files (name) WHERE id_directory IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_files_parent_name ON files (id_directory, name) WHERE id_directory IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_hardlinks_root_name ON hardlinks (name) WHERE id_directory IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_hardlinks_parent_name ON hardlinks (id_directory, name) WHERE id_directory IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_symlinks_root_name ON symlinks (name) WHERE id_parent IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_symlinks_parent_name ON symlinks (id_parent, name) WHERE id_parent IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_symlink_hardlinks_root_name ON symlink_hardlinks (name) WHERE id_parent IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_symlink_hardlinks_parent_name ON symlink_hardlinks (id_parent, name) WHERE id_parent IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_directories_parent_id ON directories (id_parent);
CREATE INDEX IF NOT EXISTS idx_files_directory_id ON files (id_directory);
CREATE INDEX IF NOT EXISTS idx_hardlinks_directory_id ON hardlinks (id_directory);
CREATE INDEX IF NOT EXISTS idx_symlinks_parent_id ON symlinks (id_parent);
CREATE INDEX IF NOT EXISTS idx_symlink_hardlinks_parent_id ON symlink_hardlinks (id_parent);
CREATE INDEX IF NOT EXISTS idx_symlink_hardlinks_symlink_id ON symlink_hardlinks (id_symlink);
CREATE INDEX IF NOT EXISTS idx_hardlinks_file_id ON hardlinks (id_file);
CREATE INDEX IF NOT EXISTS idx_xattrs_owner ON xattrs (owner_kind, owner_id);
CREATE INDEX IF NOT EXISTS idx_lock_leases_resource ON lock_leases (resource_kind, resource_id, lease_kind);
CREATE INDEX IF NOT EXISTS idx_lock_leases_expires ON lock_leases (lease_expires_at);

CREATE TABLE IF NOT EXISTS lock_range_leases (
    id_lock BIGSERIAL PRIMARY KEY,
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

CREATE TABLE IF NOT EXISTS lock_wait_edges (
    waiter_session_id BIGINT NOT NULL,
    wait_id NUMERIC(20,0) NOT NULL,
    waiter_owner_key NUMERIC(20,0) NOT NULL,
    waiter_host_name VARCHAR(255) NOT NULL,
    waiter_pid BIGINT NOT NULL,
    blocked_session_id BIGINT NOT NULL,
    blocked_owner_key NUMERIC(20,0) NOT NULL,
    blocked_host_name VARCHAR(255) NOT NULL,
    blocked_pid BIGINT NOT NULL,
    lease_expires_at TIMESTAMP NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY (waiter_session_id, wait_id, blocked_session_id, blocked_owner_key),
    CHECK (waiter_host_name <> blocked_host_name OR waiter_pid <> blocked_pid)
);
CREATE INDEX IF NOT EXISTS idx_lock_wait_edges_waiter
    ON lock_wait_edges (waiter_host_name, waiter_pid, lease_expires_at);
CREATE INDEX IF NOT EXISTS idx_lock_wait_edges_blocked
    ON lock_wait_edges (blocked_host_name, blocked_pid, lease_expires_at);

CREATE TABLE IF NOT EXISTS copy_block_crc (
    data_object_id BIGINT NOT NULL REFERENCES data_objects(id_data_object) ON DELETE CASCADE,
    _order BIGINT NOT NULL CONSTRAINT copy_block_crc_order_nonnegative_check CHECK (_order >= 0),
    crc32 BIGINT NOT NULL,
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY (data_object_id, _order)
);

CREATE TABLE IF NOT EXISTS data_object_request_tokens (
    request_token TEXT PRIMARY KEY,
    id_data_object BIGINT NOT NULL REFERENCES data_objects(id_data_object) ON DELETE CASCADE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS hardlink_promotion_request_tokens (
    request_token TEXT PRIMARY KEY,
    id_file BIGINT NOT NULL REFERENCES files(id_file) ON DELETE CASCADE,
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

CREATE TABLE IF NOT EXISTS index_sources (
    id_index_source BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    root_path TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS index_scan_runs (
    id_scan_run BIGSERIAL PRIMARY KEY,
    id_index_source BIGINT NOT NULL REFERENCES index_sources(id_index_source) ON DELETE CASCADE,
    started_at TIMESTAMP NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMP,
    status TEXT NOT NULL,
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    request_token TEXT NOT NULL UNIQUE,
    error_message TEXT
);

CREATE TABLE IF NOT EXISTS index_files (
    id_file BIGSERIAL PRIMARY KEY,
    id_index_source BIGINT NOT NULL REFERENCES index_sources(id_index_source) ON DELETE CASCADE,
    id_scan_run BIGINT REFERENCES index_scan_runs(id_scan_run) ON DELETE SET NULL,
    path TEXT NOT NULL,
    size BIGINT NOT NULL CONSTRAINT index_files_size_nonnegative_check CHECK (size >= 0),
    mtime_ns BIGINT,
    inode BIGINT,
    device BIGINT,
    file_kind TEXT NOT NULL,
    scan_status TEXT NOT NULL,
    source_changed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_index_sources_name_md5
    ON index_sources (md5(name));
CREATE INDEX IF NOT EXISTS idx_index_files_source_path_md5
    ON index_files (id_index_source, md5(path));

CREATE TABLE IF NOT EXISTS index_file_hashes (
    id_file BIGINT PRIMARY KEY REFERENCES index_files(id_file) ON DELETE CASCADE,
    hash_algorithm TEXT NOT NULL,
    partial_hash BYTEA,
    full_hash BYTEA,
    hash_status TEXT NOT NULL,
    observed_size BIGINT NOT NULL
        CONSTRAINT index_file_hashes_observed_size_nonnegative_check CHECK (observed_size >= 0),
    observed_mtime_ns BIGINT,
    observed_inode BIGINT,
    observed_device BIGINT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS index_duplicate_sets (
    id_duplicate_set BIGSERIAL PRIMARY KEY,
    hash_algorithm TEXT NOT NULL,
    full_hash BYTEA NOT NULL,
    file_size BIGINT NOT NULL
        CONSTRAINT index_duplicate_sets_file_size_nonnegative_check CHECK (file_size >= 0),
    file_count BIGINT NOT NULL CONSTRAINT index_duplicate_sets_file_count_nonnegative_check CHECK (file_count >= 0),
    total_bytes BIGINT NOT NULL CONSTRAINT index_duplicate_sets_total_bytes_nonnegative_check CHECK (total_bytes >= 0),
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    UNIQUE(hash_algorithm, full_hash, file_size)
);

CREATE TABLE IF NOT EXISTS index_import_plans (
    id_import_plan BIGSERIAL PRIMARY KEY,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    status TEXT NOT NULL,
    request_token TEXT NOT NULL UNIQUE,
    dry_run BOOLEAN NOT NULL DEFAULT TRUE,
    source_filter TEXT,
    scanned_file_count BIGINT NOT NULL DEFAULT 0
        CONSTRAINT index_import_plans_scanned_files_nonnegative_check CHECK (scanned_file_count >= 0),
    candidate_group_count BIGINT NOT NULL DEFAULT 0
        CONSTRAINT index_import_plans_candidate_groups_nonnegative_check CHECK (candidate_group_count >= 0),
    confirmed_group_count BIGINT NOT NULL DEFAULT 0
        CONSTRAINT index_import_plans_confirmed_groups_nonnegative_check CHECK (confirmed_group_count >= 0),
    unique_payload_count BIGINT NOT NULL DEFAULT 0
        CONSTRAINT index_import_plans_unique_payloads_nonnegative_check CHECK (unique_payload_count >= 0),
    total_source_bytes BIGINT NOT NULL DEFAULT 0
        CONSTRAINT index_import_plans_source_bytes_nonnegative_check CHECK (total_source_bytes >= 0),
    estimated_import_bytes BIGINT NOT NULL DEFAULT 0
        CONSTRAINT index_import_plans_estimated_bytes_nonnegative_check CHECK (estimated_import_bytes >= 0),
    saved_bytes BIGINT NOT NULL DEFAULT 0
        CONSTRAINT index_import_plans_saved_bytes_nonnegative_check CHECK (saved_bytes >= 0)
);

CREATE TABLE IF NOT EXISTS index_import_plan_entries (
    id_import_plan_entry BIGSERIAL PRIMARY KEY,
    id_import_plan BIGINT NOT NULL REFERENCES index_import_plans(id_import_plan) ON DELETE CASCADE,
    id_file BIGINT NOT NULL REFERENCES index_files(id_file) ON DELETE CASCADE,
    id_duplicate_set BIGINT REFERENCES index_duplicate_sets(id_duplicate_set) ON DELETE SET NULL,
    action TEXT NOT NULL,
    canonical_file_id BIGINT REFERENCES index_files(id_file),
    logical_path TEXT NOT NULL,
    source_path TEXT NOT NULL,
    size BIGINT NOT NULL CONSTRAINT index_import_plan_entries_size_nonnegative_check CHECK (size >= 0),
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

CREATE OR REPLACE FUNCTION fod_reject_client_session_during_schema_upgrade()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF COALESCE((
        SELECT upgrade_in_progress
        FROM schema_version
        ORDER BY applied_at DESC
        LIMIT 1
    ), FALSE) THEN
        RAISE EXCEPTION 'FOD schema upgrade is in progress or was interrupted';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_fod_client_session_schema_upgrade ON client_sessions;
CREATE TRIGGER trg_fod_client_session_schema_upgrade
BEFORE INSERT ON client_sessions
FOR EACH ROW
EXECUTE FUNCTION fod_reject_client_session_during_schema_upgrade();

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

CREATE TABLE IF NOT EXISTS destination_write_leases (
    fencing_token BIGSERIAL PRIMARY KEY,
    parent_key BIGINT NOT NULL CHECK (parent_key >= 0),
    name TEXT NOT NULL
        CONSTRAINT destination_write_leases_name_octet_length_check CHECK (octet_length(name) <= 255),
    session_id BIGINT NOT NULL REFERENCES client_sessions(session_id) ON DELETE CASCADE,
    owner_key NUMERIC(20,0) NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    heartbeat_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_destination_write_leases_resource
    ON destination_write_leases (parent_key, name);
CREATE INDEX IF NOT EXISTS idx_destination_write_leases_session
    ON destination_write_leases (session_id);
CREATE INDEX IF NOT EXISTS idx_destination_write_leases_expires
    ON destination_write_leases (lease_expires_at);

CREATE TABLE IF NOT EXISTS file_write_leases (
    fencing_token BIGSERIAL PRIMARY KEY,
    file_id BIGINT NOT NULL REFERENCES files(id_file) ON DELETE CASCADE,
    session_id BIGINT NOT NULL REFERENCES client_sessions(session_id) ON DELETE CASCADE,
    owner_key NUMERIC(20,0) NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    heartbeat_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_file_write_leases_resource
    ON file_write_leases (file_id);
CREATE INDEX IF NOT EXISTS idx_file_write_leases_session
    ON file_write_leases (session_id);
CREATE INDEX IF NOT EXISTS idx_file_write_leases_expires
    ON file_write_leases (lease_expires_at);

CREATE TABLE IF NOT EXISTS file_open_leases (
    file_id BIGINT NOT NULL REFERENCES files(id_file) ON DELETE CASCADE,
    session_id BIGINT NOT NULL REFERENCES client_sessions(session_id) ON DELETE CASCADE,
    handle_id NUMERIC(20,0) NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    heartbeat_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (session_id, handle_id)
);

CREATE INDEX IF NOT EXISTS idx_file_open_leases_file
    ON file_open_leases (file_id);
CREATE INDEX IF NOT EXISTS idx_file_open_leases_expires
    ON file_open_leases (lease_expires_at);

CREATE TABLE IF NOT EXISTS monitor_session_stats (
    session_id BIGINT PRIMARY KEY REFERENCES client_sessions(session_id) ON DELETE CASCADE,
    fod_version VARCHAR(32) NOT NULL,
    sample_seq BIGINT NOT NULL DEFAULT 0,
    sampled_at TIMESTAMP NOT NULL DEFAULT NOW(),
    payload_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_monitor_session_stats_sampled
    ON monitor_session_stats (sampled_at);

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
    PERFORM pg_advisory_xact_lock(hashtextextended('fod-posix-lock-wait-graph', 0));

    DELETE FROM lock_leases
    WHERE session_id = OLD.session_id;

    DELETE FROM lock_range_leases
    WHERE session_id = OLD.session_id;

    DELETE FROM lock_wait_edges
    WHERE waiter_session_id = OLD.session_id
       OR blocked_session_id = OLD.session_id;

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
    id_catalog_snapshot BIGSERIAL PRIMARY KEY,
    request_token TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL,
    source_filter TEXT,
    file_count BIGINT NOT NULL DEFAULT 0
        CONSTRAINT index_catalog_snapshots_file_count_nonnegative_check CHECK (file_count >= 0),
    total_bytes BIGINT NOT NULL DEFAULT 0
        CONSTRAINT index_catalog_snapshots_total_bytes_nonnegative_check CHECK (total_bytes >= 0),
    max_file_id BIGINT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS index_catalog_snapshot_files (
    id_catalog_snapshot BIGINT NOT NULL
        REFERENCES index_catalog_snapshots(id_catalog_snapshot) ON DELETE CASCADE,
    id_file BIGINT NOT NULL,
    id_index_source BIGINT NOT NULL,
    source_name TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    source_root TEXT NOT NULL,
    path TEXT NOT NULL,
    size BIGINT NOT NULL CONSTRAINT index_catalog_snapshot_files_size_nonnegative_check CHECK (size >= 0),
    mtime_ns BIGINT,
    inode BIGINT,
    device BIGINT,
    file_kind TEXT NOT NULL,
    scan_status TEXT NOT NULL,
    source_changed BOOLEAN NOT NULL,
    hash_algorithm TEXT,
    full_hash BYTEA,
    hash_status TEXT,
    id_scan_run BIGINT,
    file_created_at TIMESTAMP NOT NULL,
    file_updated_at TIMESTAMP NOT NULL,
    PRIMARY KEY (id_catalog_snapshot, id_file)
);

CREATE INDEX IF NOT EXISTS idx_index_catalog_snapshots_created
    ON index_catalog_snapshots (id_catalog_snapshot DESC);
CREATE INDEX IF NOT EXISTS idx_index_catalog_snapshot_files_source
    ON index_catalog_snapshot_files (id_catalog_snapshot, md5(source_name), id_file);
CREATE INDEX IF NOT EXISTS idx_index_catalog_snapshot_files_hash_status
    ON index_catalog_snapshot_files (id_catalog_snapshot, hash_status, id_file);
-- Copyright (c) 2026 Wojciech Stach
-- Licensed under BSL 1.1
--
-- Keep parent directory mtime/ctime in the same PostgreSQL transaction as
-- namespace entry creation, removal, rename and movement. A NULL parent maps
-- to the durable logical root row.
CREATE OR REPLACE FUNCTION fod_touch_namespace_parent(p_parent_id BIGINT)
RETURNS VOID
LANGUAGE plpgsql
AS $$
DECLARE
    touched_at TIMESTAMP := clock_timestamp();
BEGIN
    IF p_parent_id IS NULL THEN
        UPDATE directories
        SET modification_date = touched_at, change_date = touched_at
        WHERE id_directory = (
            SELECT id_directory
            FROM directories
            WHERE id_parent IS NULL AND name = '/'
        );
    ELSE
        UPDATE directories
        SET modification_date = touched_at, change_date = touched_at
        WHERE id_directory = p_parent_id;
    END IF;
END;
$$;

CREATE OR REPLACE FUNCTION fod_touch_namespace_parents()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    parent_column TEXT;
    old_parent_id BIGINT;
    new_parent_id BIGINT;
    old_visible BOOLEAN := TRUE;
    new_visible BOOLEAN := TRUE;
    namespace_changed BOOLEAN := TRUE;
BEGIN
    IF TG_TABLE_NAME IN ('directories', 'symlinks', 'symlink_hardlinks') THEN
        parent_column := 'id_parent';
    ELSE
        parent_column := 'id_directory';
    END IF;

    IF TG_OP <> 'INSERT' THEN
        old_parent_id := NULLIF(to_jsonb(OLD) ->> parent_column, '')::BIGINT;
        IF TG_TABLE_NAME = 'files' THEN
            old_visible := NOT OLD.unlinked;
        END IF;
    END IF;
    IF TG_OP <> 'DELETE' THEN
        new_parent_id := NULLIF(to_jsonb(NEW) ->> parent_column, '')::BIGINT;
        IF TG_TABLE_NAME = 'files' THEN
            new_visible := NOT NEW.unlinked;
        END IF;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        namespace_changed :=
            old_parent_id IS DISTINCT FROM new_parent_id
            OR OLD.name IS DISTINCT FROM NEW.name;
        IF TG_TABLE_NAME = 'files' THEN
            namespace_changed := namespace_changed OR OLD.unlinked IS DISTINCT FROM NEW.unlinked;
        END IF;
    END IF;

    IF NOT namespace_changed THEN
        RETURN NEW;
    END IF;
    IF TG_OP <> 'INSERT' AND old_visible THEN
        PERFORM fod_touch_namespace_parent(old_parent_id);
    END IF;
    IF TG_OP <> 'DELETE' AND new_visible
       AND (TG_OP = 'INSERT' OR old_parent_id IS DISTINCT FROM new_parent_id OR NOT old_visible)
    THEN
        PERFORM fod_touch_namespace_parent(new_parent_id);
    END IF;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS fod_directories_touch_parent ON directories;
CREATE TRIGGER fod_directories_touch_parent
AFTER INSERT OR DELETE OR UPDATE OF id_parent, name ON directories
FOR EACH ROW EXECUTE FUNCTION fod_touch_namespace_parents();

DROP TRIGGER IF EXISTS fod_files_touch_parent ON files;
CREATE TRIGGER fod_files_touch_parent
AFTER INSERT OR DELETE OR UPDATE OF id_directory, name, unlinked ON files
FOR EACH ROW EXECUTE FUNCTION fod_touch_namespace_parents();

DROP TRIGGER IF EXISTS fod_hardlinks_touch_parent ON hardlinks;
CREATE TRIGGER fod_hardlinks_touch_parent
AFTER INSERT OR DELETE OR UPDATE OF id_directory, name ON hardlinks
FOR EACH ROW EXECUTE FUNCTION fod_touch_namespace_parents();

DROP TRIGGER IF EXISTS fod_symlinks_touch_parent ON symlinks;
CREATE TRIGGER fod_symlinks_touch_parent
AFTER INSERT OR DELETE OR UPDATE OF id_parent, name ON symlinks
FOR EACH ROW EXECUTE FUNCTION fod_touch_namespace_parents();

DROP TRIGGER IF EXISTS fod_symlink_hardlinks_touch_parent ON symlink_hardlinks;
CREATE TRIGGER fod_symlink_hardlinks_touch_parent
AFTER INSERT OR DELETE OR UPDATE OF id_parent, name ON symlink_hardlinks
FOR EACH ROW EXECUTE FUNCTION fod_touch_namespace_parents();
