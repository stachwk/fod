-- FOD exposes entity IDs as u64 in Rust and FUSE-facing repository APIs.
-- Widen the SQL identity graph and owned serial sequences together so the
-- signed 32-bit SERIAL ceiling does not stop long-lived filesystems at 2^31-1.
CREATE TEMP TABLE fod_migration_0037_foreign_keys ON COMMIT DROP AS
SELECT conrelid AS child_table,
       conname AS constraint_name,
       pg_get_constraintdef(oid) AS constraint_definition
FROM pg_constraint
WHERE contype = 'f'
  AND connamespace = 'fod'::regnamespace;

CREATE TEMP TABLE fod_migration_0037_triggers ON COMMIT DROP AS
SELECT tgrelid AS trigger_table,
       tgname AS trigger_name,
       pg_get_triggerdef(tg.oid) AS trigger_definition
FROM pg_trigger AS tg
JOIN pg_class AS cls ON cls.oid = tg.tgrelid
WHERE NOT tg.tgisinternal
  AND cls.relnamespace = 'fod'::regnamespace;

DO $$
DECLARE
    item RECORD;
BEGIN
    FOR item IN
        SELECT trigger_table, trigger_name
        FROM fod_migration_0037_triggers
        ORDER BY trigger_table, trigger_name
    LOOP
        EXECUTE format(
            'DROP TRIGGER %I ON %s',
            item.trigger_name,
            item.trigger_table::regclass
        );
    END LOOP;

    FOR item IN
        SELECT child_table, constraint_name
        FROM fod_migration_0037_foreign_keys
        ORDER BY child_table, constraint_name
    LOOP
        EXECUTE format(
            'ALTER TABLE %s DROP CONSTRAINT %I',
            item.child_table::regclass,
            item.constraint_name
        );
    END LOOP;

    FOR item IN
        SELECT *
        FROM (VALUES
            ('directories', 'id_directory'),
            ('directories', 'id_parent'),
            ('data_objects', 'id_data_object'),
            ('files', 'id_file'),
            ('files', 'data_object_id'),
            ('files', 'id_directory'),
            ('special_files', 'id_file'),
            ('hardlinks', 'id_hardlink'),
            ('hardlinks', 'id_file'),
            ('hardlinks', 'id_directory'),
            ('symlinks', 'id_symlink'),
            ('symlinks', 'id_parent'),
            ('data_blocks', 'id_block'),
            ('data_blocks', 'data_object_id'),
            ('copy_block_crc', 'data_object_id'),
            ('data_object_request_tokens', 'id_data_object'),
            ('hardlink_promotion_request_tokens', 'id_file'),
            ('file_open_leases', 'file_id'),
            ('file_write_leases', 'file_id'),
            ('journal', 'id_entry'),
            ('journal', 'id_directory'),
            ('journal', 'id_file'),
            ('xattrs', 'id'),
            ('xattrs', 'owner_id'),
            ('lock_leases', 'id_lock'),
            ('lock_range_leases', 'id_lock'),
            ('index_sources', 'id_index_source'),
            ('index_scan_runs', 'id_scan_run'),
            ('index_scan_runs', 'id_index_source'),
            ('index_files', 'id_file'),
            ('index_files', 'id_index_source'),
            ('index_files', 'id_scan_run'),
            ('index_file_hashes', 'id_file'),
            ('index_duplicate_sets', 'id_duplicate_set'),
            ('index_import_plans', 'id_import_plan'),
            ('index_import_plan_entries', 'id_import_plan_entry'),
            ('index_import_plan_entries', 'id_import_plan'),
            ('index_import_plan_entries', 'id_file'),
            ('index_import_plan_entries', 'id_duplicate_set'),
            ('index_import_plan_entries', 'canonical_file_id'),
            ('index_catalog_snapshots', 'id_catalog_snapshot'),
            ('index_catalog_snapshot_files', 'id_catalog_snapshot'),
            ('index_catalog_snapshot_files', 'id_file'),
            ('index_catalog_snapshot_files', 'id_index_source'),
            ('index_catalog_snapshot_files', 'id_scan_run')
        ) AS ids(table_name, column_name)
        ORDER BY table_name, column_name
    LOOP
        EXECUTE format(
            'ALTER TABLE fod.%I ALTER COLUMN %I TYPE BIGINT USING %I::BIGINT',
            item.table_name,
            item.column_name,
            item.column_name
        );
    END LOOP;

    FOR item IN
        SELECT seq.oid::regclass AS sequence_name
        FROM pg_class AS seq
        JOIN pg_depend AS dep
          ON dep.classid = 'pg_class'::regclass
         AND dep.objid = seq.oid
         AND dep.refclassid = 'pg_class'::regclass
         AND dep.deptype IN ('a', 'i')
        JOIN pg_class AS tbl ON tbl.oid = dep.refobjid
        WHERE seq.relkind = 'S'
          AND tbl.relnamespace = 'fod'::regnamespace
        ORDER BY seq.oid
    LOOP
        EXECUTE format('ALTER SEQUENCE %s AS BIGINT', item.sequence_name);
    END LOOP;

    FOR item IN
        SELECT trigger_definition
        FROM fod_migration_0037_triggers
        ORDER BY trigger_table, trigger_name
    LOOP
        EXECUTE item.trigger_definition;
    END LOOP;

    FOR item IN
        SELECT child_table, constraint_name, constraint_definition
        FROM fod_migration_0037_foreign_keys
        ORDER BY child_table, constraint_name
    LOOP
        EXECUTE format(
            'ALTER TABLE %s ADD CONSTRAINT %I %s',
            item.child_table::regclass,
            item.constraint_name,
            item.constraint_definition
        );
    END LOOP;
END;
$$;
