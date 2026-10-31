-- Linux/FUSE supplies ownership and device-number fields as unsigned 32-bit
-- values. PostgreSQL INTEGER is signed, so BIGINT plus explicit range checks
-- preserves the complete input domain. File/block counters use the same
-- non-negative 64-bit domain as the Rust runtime.
ALTER TABLE directories ALTER COLUMN uid TYPE BIGINT;
ALTER TABLE directories ALTER COLUMN gid TYPE BIGINT;
ALTER TABLE directories
    ADD CONSTRAINT directories_uid_u32_check CHECK (uid BETWEEN 0 AND 4294967295);
ALTER TABLE directories
    ADD CONSTRAINT directories_gid_u32_check CHECK (gid BETWEEN 0 AND 4294967295);

ALTER TABLE files ALTER COLUMN uid TYPE BIGINT;
ALTER TABLE files ALTER COLUMN gid TYPE BIGINT;
ALTER TABLE files
    ALTER COLUMN size TYPE BIGINT,
    ADD CONSTRAINT files_uid_u32_check CHECK (uid BETWEEN 0 AND 4294967295),
    ADD CONSTRAINT files_gid_u32_check CHECK (gid BETWEEN 0 AND 4294967295),
    ADD CONSTRAINT files_size_nonnegative_check CHECK (size >= 0);

ALTER TABLE hardlinks ALTER COLUMN uid TYPE BIGINT;
ALTER TABLE hardlinks ALTER COLUMN gid TYPE BIGINT;
ALTER TABLE hardlinks
    ADD CONSTRAINT hardlinks_uid_u32_check CHECK (uid BETWEEN 0 AND 4294967295),
    ADD CONSTRAINT hardlinks_gid_u32_check CHECK (gid BETWEEN 0 AND 4294967295);

ALTER TABLE symlinks ALTER COLUMN uid TYPE BIGINT;
ALTER TABLE symlinks ALTER COLUMN gid TYPE BIGINT;
ALTER TABLE symlinks
    ADD CONSTRAINT symlinks_uid_u32_check CHECK (uid BETWEEN 0 AND 4294967295),
    ADD CONSTRAINT symlinks_gid_u32_check CHECK (gid BETWEEN 0 AND 4294967295);

ALTER TABLE special_files
    ALTER COLUMN rdev_major TYPE BIGINT,
    ALTER COLUMN rdev_minor TYPE BIGINT,
    ADD CONSTRAINT special_files_rdev_major_u32_check CHECK (rdev_major BETWEEN 0 AND 4294967295),
    ADD CONSTRAINT special_files_rdev_minor_u32_check CHECK (rdev_minor BETWEEN 0 AND 4294967295);

ALTER TABLE journal ALTER COLUMN id_user TYPE BIGINT;
ALTER TABLE journal
    ADD CONSTRAINT journal_user_u32_check CHECK (id_user BETWEEN 0 AND 4294967295);

ALTER TABLE data_objects
    ALTER COLUMN file_size TYPE BIGINT,
    ALTER COLUMN reference_count TYPE BIGINT,
    ADD CONSTRAINT data_objects_file_size_nonnegative_check CHECK (file_size >= 0),
    ADD CONSTRAINT data_objects_reference_count_nonnegative_check CHECK (reference_count >= 0);

ALTER TABLE data_blocks
    ALTER COLUMN _order TYPE BIGINT,
    ADD CONSTRAINT data_blocks_order_nonnegative_check CHECK (_order >= 0);
ALTER TABLE copy_block_crc
    ALTER COLUMN _order TYPE BIGINT,
    ADD CONSTRAINT copy_block_crc_order_nonnegative_check CHECK (_order >= 0);

ALTER TABLE index_files
    ADD CONSTRAINT index_files_size_nonnegative_check CHECK (size >= 0);
ALTER TABLE index_file_hashes
    ADD CONSTRAINT index_file_hashes_observed_size_nonnegative_check CHECK (observed_size >= 0);
ALTER TABLE index_duplicate_sets
    ALTER COLUMN file_count TYPE BIGINT,
    ADD CONSTRAINT index_duplicate_sets_file_size_nonnegative_check CHECK (file_size >= 0),
    ADD CONSTRAINT index_duplicate_sets_file_count_nonnegative_check CHECK (file_count >= 0),
    ADD CONSTRAINT index_duplicate_sets_total_bytes_nonnegative_check CHECK (total_bytes >= 0);
ALTER TABLE index_import_plans
    ALTER COLUMN scanned_file_count TYPE BIGINT,
    ALTER COLUMN candidate_group_count TYPE BIGINT,
    ALTER COLUMN confirmed_group_count TYPE BIGINT,
    ALTER COLUMN unique_payload_count TYPE BIGINT,
    ADD CONSTRAINT index_import_plans_scanned_files_nonnegative_check CHECK (scanned_file_count >= 0),
    ADD CONSTRAINT index_import_plans_candidate_groups_nonnegative_check CHECK (candidate_group_count >= 0),
    ADD CONSTRAINT index_import_plans_confirmed_groups_nonnegative_check CHECK (confirmed_group_count >= 0),
    ADD CONSTRAINT index_import_plans_unique_payloads_nonnegative_check CHECK (unique_payload_count >= 0),
    ADD CONSTRAINT index_import_plans_source_bytes_nonnegative_check CHECK (total_source_bytes >= 0),
    ADD CONSTRAINT index_import_plans_estimated_bytes_nonnegative_check CHECK (estimated_import_bytes >= 0),
    ADD CONSTRAINT index_import_plans_saved_bytes_nonnegative_check CHECK (saved_bytes >= 0);
ALTER TABLE index_import_plan_entries
    ADD CONSTRAINT index_import_plan_entries_size_nonnegative_check CHECK (size >= 0);
ALTER TABLE index_catalog_snapshots
    ADD CONSTRAINT index_catalog_snapshots_file_count_nonnegative_check CHECK (file_count >= 0),
    ADD CONSTRAINT index_catalog_snapshots_total_bytes_nonnegative_check CHECK (total_bytes >= 0);
ALTER TABLE index_catalog_snapshot_files
    ADD CONSTRAINT index_catalog_snapshot_files_size_nonnegative_check CHECK (size >= 0);
