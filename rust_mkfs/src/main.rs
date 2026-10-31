// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod config;
mod pg;
mod pg_config;
mod runtime;
mod schema_admin;
mod tls;
mod version;

use clap::{Parser, ValueEnum};
use serde_json::json;
use std::env;
use std::path::Path;

use config::{load_config_parser, load_runtime_config, resolve_config_path};
use pg::DbConn;
use pg_config::{make_conninfo, resolve_pg_connection_params};
use runtime::{
    compare_fod_client_versions, FOD_REQUIRED_SCHEMA_VERSION, FOD_SCHEMA_NAME, FOD_SEARCH_PATH,
    FOD_VERSION,
};
use schema_admin::{
    ensure_schema_admin_secret, format_schema_admin_source_message, schema_admin_secret_exists,
    schema_admin_secret_required_message, verify_existing_schema_admin_secret,
};
use tls::generate_client_tls_pair;

use version::FOD_VERSION_LABEL;
const SCHEMA_VERSION: u64 = 34;
const _: () = assert!(SCHEMA_VERSION as u32 == FOD_REQUIRED_SCHEMA_VERSION);
const STATUS_JSON_SCHEMA_VERSION: u32 = 1;
const MIGRATION_FILES: [&str; 34] = [
    "0001_base.sql",
    "0002_schema_admin.sql",
    "0003_schema_version_sql.sql",
    "0004_copy_block_crc.sql",
    "0005_data_objects.sql",
    "0006_data_objects_hash_dedupe.sql",
    "0007_copy_block_crc_object_key.sql",
    "0008_client_sessions.sql",
    "0009_client_session_lock_cleanup.sql",
    "0010_fod_schema.sql",
    "0011_rename_fod_schema.sql",
    "0012_storage_slot.sql",
    "0013_indexer.sql",
    "0014_indexer_request_tokens.sql",
    "0015_data_object_request_tokens.sql",
    "0016_hardlink_promotion_request_tokens.sql",
    "0017_data_object_payload_ownership.sql",
    "0018_payload_capacity_reservations.sql",
    "0019_index_catalog_snapshots.sql",
    "0020_storage_slot.sql",
    "0021_storage_layout_finalize.sql",
    "0022_monitor_session_stats.sql",
    "0023_drop_redundant_data_blocks_index.sql",
    "0024_write_ownership_leases.sql",
    "0025_open_unlinked_files.sql",
    "0026_file_content_generation.sql",
    "0027_explicit_file_content_generation.sql",
    "0028_namespace_parent_timestamps.sql",
    "0029_posix_lock_wait_graph.sql",
    "0030_posix_lock_wait_process_identity.sql",
    "0031_minimum_client_version.sql",
    "0032_schema_upgrade_state.sql",
    "0033_byte_length_name_limits.sql",
    "0034_align_numeric_column_ranges.sql",
];

const MIGRATION_DESCRIPTIONS: [&str; 34] = [
    "Base schema and initial FOD tables",
    "Schema admin secret table",
    "Schema version tracking table",
    "Copy block CRC cache table",
    "Data objects for copy-on-write and dedupe",
    "Data object hash+size dedupe index",
    "Copy block CRC keyed by data object",
    "Client session heartbeats and owner mapping",
    "PostgreSQL-side lock cleanup trigger for expired client sessions",
    "Move FOD objects into the dedicated fod schema",
    "Rename legacy FOD schema to fod",
    "Reserved storage migration slot",
    "Add fod-indexer metadata tables",
    "Make indexer run creation replay-safe",
    "Add request tokens for data object creation",
    "Add request tokens for hardlink promotion replay",
    "Make data objects own payload rows",
    "Add transactional payload capacity reservations",
    "Add immutable index catalogue snapshots",
    "Reserved storage migration slot",
    "Finalize canonical block storage layout",
    "Add shared monitor session statistics",
    "Drop redundant data_blocks prefix index and refresh planner statistics",
    "Add PostgreSQL-authoritative destination and file write ownership leases",
    "Preserve open file handles across unlink with global open leases",
    "Track transactional per-file content generations for cross-mount coherence",
    "Use explicit transactional content generation updates",
    "Update directory timestamps with namespace changes, including root",
    "Track blocking POSIX lock dependencies for cross-mount deadlock detection",
    "Use host and process identity for cross-mount POSIX lock wait graphs",
    "Persist the minimum compatible FOD client release",
    "Gate FOD mounts while schema migrations are incomplete",
    "Enforce Linux byte limits for filesystem and xattr names",
    "Align SQL numeric columns with Linux and Rust value ranges",
];

#[derive(Copy, Clone, Eq, PartialEq, ValueEnum)]
enum Action {
    Init,
    Upgrade,
    Clean,
    Status,
}

#[derive(Parser)]
#[command(name = "fod-mkfs", version = FOD_VERSION_LABEL, about = "Manage the fod schema.")]
struct Cli {
    #[arg(value_enum)]
    action: Action,
    #[arg(long, default_value_t = 32768)]
    block_size: u64,
    #[arg(long)]
    schema_admin_password: Option<String>,
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "1",
        default_value = "false",
        value_parser = parse_truthy_arg
    )]
    generate_client_tls_pair: bool,
    #[arg(long, default_value = ".fod/tls")]
    tls_material_dir: String,
    #[arg(
        long,
        default_value = "fod",
        help = "TLS common name for generated client material; allowed chars: ASCII letters, digits, dot, underscore, hyphen."
    )]
    tls_common_name: String,
    #[arg(long, default_value_t = 365)]
    tls_cert_days: i64,
    #[arg(long, default_value_t = false)]
    json: bool,
}

fn parse_truthy_arg(value: &str) -> Result<bool, String> {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err("expected 0/1, true/false, yes/no, or on/off".to_string()),
    }
}

fn current_uid_gid() -> (u32, u32) {
    #[cfg(unix)]
    {
        unsafe { (libc::getuid() as u32, libc::getgid() as u32) }
    }
    #[cfg(not(unix))]
    {
        (0, 0)
    }
}

fn load_schema_admin_password(cli: &Cli) -> (Option<String>, Option<String>) {
    if let Some(password) = &cli.schema_admin_password {
        (Some(password.clone()), Some("cli".to_string()))
    } else {
        (None, None)
    }
}

fn migration_sql(version: u64) -> &'static str {
    match version {
        1 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0001_base.sql"
        )),
        2 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0002_schema_admin.sql"
        )),
        3 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0003_schema_version_sql.sql"
        )),
        4 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0004_copy_block_crc.sql"
        )),
        5 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0005_data_objects.sql"
        )),
        6 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0006_data_objects_hash_dedupe.sql"
        )),
        7 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0007_copy_block_crc_object_key.sql"
        )),
        8 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0008_client_sessions.sql"
        )),
        9 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0009_client_session_lock_cleanup.sql"
        )),
        10 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0010_fod_schema.sql"
        )),
        11 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0011_rename_fod_schema.sql"
        )),
        12 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0012_storage_slot.sql"
        )),
        13 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0013_indexer.sql"
        )),
        14 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0014_indexer_request_tokens.sql"
        )),
        15 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0015_data_object_request_tokens.sql"
        )),
        16 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0016_hardlink_promotion_request_tokens.sql"
        )),
        17 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0017_data_object_payload_ownership.sql"
        )),
        18 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0018_payload_capacity_reservations.sql"
        )),
        19 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0019_index_catalog_snapshots.sql"
        )),
        20 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0020_storage_slot.sql"
        )),
        21 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0021_storage_layout_finalize.sql"
        )),
        22 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0022_monitor_session_stats.sql"
        )),
        23 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0023_drop_redundant_data_blocks_index.sql"
        )),
        24 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0024_write_ownership_leases.sql"
        )),
        25 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0025_open_unlinked_files.sql"
        )),
        26 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0026_file_content_generation.sql"
        )),
        27 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0027_explicit_file_content_generation.sql"
        )),
        28 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0028_namespace_parent_timestamps.sql"
        )),
        29 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0029_posix_lock_wait_graph.sql"
        )),
        30 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0030_posix_lock_wait_process_identity.sql"
        )),
        31 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0031_minimum_client_version.sql"
        )),
        32 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0032_schema_upgrade_state.sql"
        )),
        33 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0033_byte_length_name_limits.sql"
        )),
        34 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0034_align_numeric_column_ranges.sql"
        )),
        _ => "",
    }
}

fn migration_description(version: u64) -> &'static str {
    match version {
        1 => MIGRATION_DESCRIPTIONS[0],
        2 => MIGRATION_DESCRIPTIONS[1],
        3 => MIGRATION_DESCRIPTIONS[2],
        4 => MIGRATION_DESCRIPTIONS[3],
        5 => MIGRATION_DESCRIPTIONS[4],
        6 => MIGRATION_DESCRIPTIONS[5],
        7 => MIGRATION_DESCRIPTIONS[6],
        8 => MIGRATION_DESCRIPTIONS[7],
        9 => MIGRATION_DESCRIPTIONS[8],
        10 => MIGRATION_DESCRIPTIONS[9],
        11 => MIGRATION_DESCRIPTIONS[10],
        12 => MIGRATION_DESCRIPTIONS[11],
        13 => MIGRATION_DESCRIPTIONS[12],
        14 => MIGRATION_DESCRIPTIONS[13],
        15 => MIGRATION_DESCRIPTIONS[14],
        16 => MIGRATION_DESCRIPTIONS[15],
        17 => MIGRATION_DESCRIPTIONS[16],
        18 => MIGRATION_DESCRIPTIONS[17],
        19 => MIGRATION_DESCRIPTIONS[18],
        20 => MIGRATION_DESCRIPTIONS[19],
        21 => MIGRATION_DESCRIPTIONS[20],
        22 => MIGRATION_DESCRIPTIONS[21],
        23 => MIGRATION_DESCRIPTIONS[22],
        24 => MIGRATION_DESCRIPTIONS[23],
        25 => MIGRATION_DESCRIPTIONS[24],
        26 => MIGRATION_DESCRIPTIONS[25],
        27 => MIGRATION_DESCRIPTIONS[26],
        28 => MIGRATION_DESCRIPTIONS[27],
        29 => MIGRATION_DESCRIPTIONS[28],
        30 => MIGRATION_DESCRIPTIONS[29],
        31 => MIGRATION_DESCRIPTIONS[30],
        32 => MIGRATION_DESCRIPTIONS[31],
        33 => MIGRATION_DESCRIPTIONS[32],
        34 => MIGRATION_DESCRIPTIONS[33],
        _ => "Migration",
    }
}

fn migration_filename(version: u64) -> &'static str {
    match version {
        1 => MIGRATION_FILES[0],
        2 => MIGRATION_FILES[1],
        3 => MIGRATION_FILES[2],
        4 => MIGRATION_FILES[3],
        5 => MIGRATION_FILES[4],
        6 => MIGRATION_FILES[5],
        7 => MIGRATION_FILES[6],
        8 => MIGRATION_FILES[7],
        9 => MIGRATION_FILES[8],
        10 => MIGRATION_FILES[9],
        11 => MIGRATION_FILES[10],
        12 => MIGRATION_FILES[11],
        13 => MIGRATION_FILES[12],
        14 => MIGRATION_FILES[13],
        15 => MIGRATION_FILES[14],
        16 => MIGRATION_FILES[15],
        17 => MIGRATION_FILES[16],
        18 => MIGRATION_FILES[17],
        19 => MIGRATION_FILES[18],
        20 => MIGRATION_FILES[19],
        21 => MIGRATION_FILES[20],
        22 => MIGRATION_FILES[21],
        23 => MIGRATION_FILES[22],
        24 => MIGRATION_FILES[23],
        25 => MIGRATION_FILES[24],
        26 => MIGRATION_FILES[25],
        27 => MIGRATION_FILES[26],
        28 => MIGRATION_FILES[27],
        29 => MIGRATION_FILES[28],
        30 => MIGRATION_FILES[29],
        31 => MIGRATION_FILES[30],
        32 => MIGRATION_FILES[31],
        33 => MIGRATION_FILES[32],
        34 => MIGRATION_FILES[33],
        _ => "unknown.sql",
    }
}

fn base_schema_sql() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../migrations/base_schema.sql"
    ))
}

fn latest_migration_version() -> u64 {
    SCHEMA_VERSION
}

fn migration_exists(version: u64) -> bool {
    (1..=latest_migration_version()).contains(&version)
}

fn migration_manifest() -> Vec<(u64, &'static str, &'static str)> {
    (1..=latest_migration_version())
        .map(|version| {
            (
                version,
                migration_filename(version),
                migration_description(version),
            )
        })
        .collect()
}

fn apply_migration(conn: &DbConn, version: u64) -> Result<(), String> {
    let sql = migration_sql(version);
    if sql.is_empty() {
        return Err(format!("Missing migration file for version {}", version));
    }
    conn.exec(sql)
}

fn apply_base_schema(conn: &DbConn) -> Result<(), String> {
    let sql = base_schema_sql();
    if sql.trim().is_empty() {
        return Err("Missing base schema file".to_string());
    }
    conn.exec(sql)
}

// Build regclass / regprocedure lookups from quoted identifiers so schema
// names can later become configurable without opening a SQL injection path.
fn quote_schema_qualified_ident(schema: &str, object: &str) -> String {
    format!(
        "{}.{}",
        DbConn::quote_identifier(schema),
        DbConn::quote_identifier(object)
    )
}

fn quote_schema_regclass(schema: &str, object: &str) -> String {
    DbConn::quote_literal(&quote_schema_qualified_ident(schema, object))
}

fn quote_schema_regprocedure(schema: &str, object: &str, args: &str) -> String {
    DbConn::quote_literal(&format!(
        "{}({})",
        quote_schema_qualified_ident(schema, object),
        args
    ))
}

fn read_schema_version(conn: &DbConn) -> Result<Option<u64>, String> {
    if !conn.query_exists(&format!(
        "SELECT to_regclass({}) IS NOT NULL",
        quote_schema_regclass(FOD_SCHEMA_NAME, "schema_version")
    ))? {
        return Ok(None);
    }
    conn.query_scalar_text(&format!(
        "SELECT COALESCE(to_jsonb(version_row)->>'schema_number', to_jsonb(version_row)->>'version') FROM {} AS version_row ORDER BY applied_at DESC LIMIT 1",
        quote_schema_qualified_ident(FOD_SCHEMA_NAME, "schema_version")
    ))?
    .map(|value| {
        value
            .parse::<u64>()
            .map_err(|err| format!("invalid FOD schema version: {err}"))
    })
    .transpose()
}

fn read_minimum_client_version(conn: &DbConn) -> Result<Option<String>, String> {
    if !conn.query_exists(&format!(
        "SELECT to_regclass({}) IS NOT NULL",
        quote_schema_regclass(FOD_SCHEMA_NAME, "schema_version")
    ))? {
        return Ok(None);
    }

    conn.query_scalar_text(&format!(
        "SELECT to_jsonb(version_row)->>'minimum_client_version' FROM {} AS version_row ORDER BY applied_at DESC LIMIT 1",
        quote_schema_qualified_ident(FOD_SCHEMA_NAME, "schema_version")
    ))
}

fn read_schema_upgrade_in_progress(conn: &DbConn) -> Result<bool, String> {
    if !conn.query_exists(&format!(
        "SELECT to_regclass({}) IS NOT NULL",
        quote_schema_regclass(FOD_SCHEMA_NAME, "schema_version")
    ))? {
        return Ok(false);
    }
    conn.query_scalar_bool(
        "SELECT COALESCE((\
            SELECT (to_jsonb(version_row)->>'upgrade_in_progress')::boolean \
            FROM fod.schema_version AS version_row \
            ORDER BY applied_at DESC LIMIT 1\
        ), FALSE)",
    )
}

fn latest_schema_shape_matches(conn: &DbConn) -> Result<bool, String> {
    conn.query_exists(
        "SELECT
            NOT EXISTS (
                SELECT 1
                FROM (VALUES
                    ('directories'),
                    ('data_objects'),
                    ('files'),
                    ('special_files'),
                    ('hardlinks'),
                    ('symlinks'),
                    ('data_blocks'),
                    ('config'),
                    ('schema_version'),
                    ('schema_admin'),
                    ('journal'),
                    ('xattrs'),
                    ('lock_leases'),
                    ('lock_range_leases'),
                    ('copy_block_crc'),
                    ('data_object_request_tokens'),
                    ('hardlink_promotion_request_tokens'),
                    ('payload_capacity_reservations'),
                    ('index_sources'),
                    ('index_scan_runs'),
                    ('index_files'),
                    ('index_file_hashes'),
                    ('index_duplicate_sets'),
                    ('index_import_plans'),
                    ('index_import_plan_entries'),
                    ('index_catalog_snapshots'),
                    ('index_catalog_snapshot_files'),
                    ('client_sessions'),
                    ('client_session_owner_keys'),
                    ('destination_write_leases'),
                    ('file_write_leases'),
                    ('file_open_leases'),
                    ('monitor_session_stats')
                ) AS required_relations(name)
                WHERE to_regclass(format('fod.%I', name)) IS NULL
            )
            AND NOT EXISTS (
                SELECT 1
                FROM (VALUES
                    ('directories', 'directories_name_octet_length_check'),
                    ('files', 'files_name_octet_length_check'),
                    ('hardlinks', 'hardlinks_name_octet_length_check'),
                    ('symlinks', 'symlinks_name_octet_length_check'),
                    ('xattrs', 'xattrs_name_octet_length_check'),
                    ('destination_write_leases', 'destination_write_leases_name_octet_length_check')
                ) AS required_checks(table_name, constraint_name)
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM pg_constraint c
                    WHERE c.conrelid = to_regclass(format('fod.%I', table_name))
                      AND c.conname = constraint_name
                      AND c.contype = 'c'
                )
            )
            AND NOT EXISTS (
                SELECT 1
                FROM (VALUES
                    ('directories', 'uid'), ('directories', 'gid'),
                    ('files', 'uid'), ('files', 'gid'), ('files', 'size'),
                    ('hardlinks', 'uid'), ('hardlinks', 'gid'),
                    ('symlinks', 'uid'), ('symlinks', 'gid'),
                    ('special_files', 'rdev_major'), ('special_files', 'rdev_minor'),
                    ('journal', 'id_user'),
                    ('data_objects', 'file_size'), ('data_objects', 'reference_count'),
                    ('data_blocks', '_order'), ('copy_block_crc', '_order'),
                    ('index_duplicate_sets', 'file_count'),
                    ('index_import_plans', 'scanned_file_count'),
                    ('index_import_plans', 'candidate_group_count'),
                    ('index_import_plans', 'confirmed_group_count'),
                    ('index_import_plans', 'unique_payload_count')
                ) AS required_bigint_columns(table_name, column_name)
                WHERE NOT EXISTS (
                    SELECT 1 FROM information_schema.columns c
                    WHERE c.table_schema = 'fod'
                      AND c.table_name = required_bigint_columns.table_name
                      AND c.column_name = required_bigint_columns.column_name
                      AND c.data_type = 'bigint'
                )
            )
            AND NOT EXISTS (
                SELECT 1
                FROM (VALUES
                    ('directories', 'directories_uid_u32_check'),
                    ('directories', 'directories_gid_u32_check'),
                    ('files', 'files_uid_u32_check'),
                    ('files', 'files_gid_u32_check'),
                    ('files', 'files_size_nonnegative_check'),
                    ('hardlinks', 'hardlinks_uid_u32_check'),
                    ('hardlinks', 'hardlinks_gid_u32_check'),
                    ('symlinks', 'symlinks_uid_u32_check'),
                    ('symlinks', 'symlinks_gid_u32_check'),
                    ('special_files', 'special_files_rdev_major_u32_check'),
                    ('special_files', 'special_files_rdev_minor_u32_check'),
                    ('journal', 'journal_user_u32_check'),
                    ('data_objects', 'data_objects_file_size_nonnegative_check'),
                    ('data_objects', 'data_objects_reference_count_nonnegative_check'),
                    ('data_blocks', 'data_blocks_order_nonnegative_check'),
                    ('copy_block_crc', 'copy_block_crc_order_nonnegative_check'),
                    ('index_files', 'index_files_size_nonnegative_check'),
                    ('index_file_hashes', 'index_file_hashes_observed_size_nonnegative_check'),
                    ('index_duplicate_sets', 'index_duplicate_sets_file_size_nonnegative_check'),
                    ('index_duplicate_sets', 'index_duplicate_sets_file_count_nonnegative_check'),
                    ('index_duplicate_sets', 'index_duplicate_sets_total_bytes_nonnegative_check'),
                    ('index_import_plans', 'index_import_plans_scanned_files_nonnegative_check'),
                    ('index_import_plans', 'index_import_plans_candidate_groups_nonnegative_check'),
                    ('index_import_plans', 'index_import_plans_confirmed_groups_nonnegative_check'),
                    ('index_import_plans', 'index_import_plans_unique_payloads_nonnegative_check'),
                    ('index_import_plans', 'index_import_plans_source_bytes_nonnegative_check'),
                    ('index_import_plans', 'index_import_plans_estimated_bytes_nonnegative_check'),
                    ('index_import_plans', 'index_import_plans_saved_bytes_nonnegative_check'),
                    ('index_import_plan_entries', 'index_import_plan_entries_size_nonnegative_check'),
                    ('index_catalog_snapshots', 'index_catalog_snapshots_file_count_nonnegative_check'),
                    ('index_catalog_snapshots', 'index_catalog_snapshots_total_bytes_nonnegative_check'),
                    ('index_catalog_snapshot_files', 'index_catalog_snapshot_files_size_nonnegative_check')
                ) AS required_checks(table_name, constraint_name)
                WHERE NOT EXISTS (
                    SELECT 1 FROM pg_constraint c
                    WHERE c.conrelid = to_regclass(format('fod.%I', required_checks.table_name))
                      AND c.conname = required_checks.constraint_name
                      AND c.contype = 'c'
                )
            )
            AND to_regprocedure('fod.fod_prune_client_session_lock_leases()') IS NOT NULL
            AND EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'fod'
                  AND table_name = 'schema_version'
                  AND column_name = 'schema_number'
                  AND is_nullable = 'NO'
            )
            AND EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'fod'
                  AND table_name = 'schema_version'
                  AND column_name = 'minimum_client_version'
                  AND is_nullable = 'NO'
            )
            AND EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = 'fod'
                  AND table_name = 'schema_version'
                  AND column_name = 'upgrade_in_progress'
                  AND is_nullable = 'NO'
            )
            AND to_regprocedure('fod.fod_reject_client_session_during_schema_upgrade()') IS NOT NULL
            AND EXISTS (
                SELECT 1 FROM pg_trigger
                WHERE tgrelid = 'fod.client_sessions'::regclass
                  AND tgname = 'trg_fod_client_session_schema_upgrade'
                  AND NOT tgisinternal
            )
            AND (
                SELECT COUNT(*)
                FROM information_schema.columns
                WHERE table_schema = 'fod'
                  AND table_name IN ('files', 'data_blocks', 'copy_block_crc')
                  AND column_name = 'data_object_id'
                  AND is_nullable = 'NO'
            ) = 3
            AND NOT EXISTS (
                SELECT 1
                FROM information_schema.columns
                WHERE table_schema = 'fod'
                  AND table_name IN ('data_blocks', 'copy_block_crc')
                  AND column_name = 'id_file'
            )
            AND EXISTS (
                SELECT 1
                FROM information_schema.columns
                WHERE table_schema = 'fod'
                  AND table_name = 'files'
                  AND column_name = 'unlinked'
                  AND is_nullable = 'NO'
            )
            AND EXISTS (
                SELECT 1
                FROM information_schema.columns
                WHERE table_schema = 'fod'
                  AND table_name = 'files'
                  AND column_name = 'content_generation'
                  AND is_nullable = 'NO'
                  AND column_default = '1'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_trigger
                WHERE tgrelid = 'fod.files'::regclass
                  AND tgname = 'trg_fod_file_content_generation'
                  AND NOT tgisinternal
            )
            AND to_regprocedure('fod.fod_bump_file_content_generation()') IS NOT NULL
            AND EXISTS (
                SELECT 1
                FROM pg_indexes
                WHERE schemaname = 'fod'
                  AND indexname = 'idx_monitor_session_stats_sampled'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_indexes
                WHERE schemaname = 'fod'
                  AND indexname = 'idx_destination_write_leases_resource'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_indexes
                WHERE schemaname = 'fod'
                  AND indexname = 'idx_file_write_leases_resource'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_indexes
                WHERE schemaname = 'fod'
                  AND indexname = 'idx_file_open_leases_file'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_indexes
                WHERE schemaname = 'fod'
                  AND indexname = 'idx_data_blocks_object_order'
            )
            AND NOT EXISTS (
                SELECT 1
                FROM pg_indexes
                WHERE schemaname = 'fod'
                  AND indexname = 'idx_data_blocks_data_object_id'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_constraint
                WHERE conname = 'files_data_object_id_fkey'
                  AND conrelid = 'fod.files'::regclass
                  AND confrelid = 'fod.data_objects'::regclass
                  AND contype = 'f'
                  AND confdeltype = 'a'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_constraint
                WHERE conname = 'data_blocks_data_object_id_fkey'
                  AND conrelid = 'fod.data_blocks'::regclass
                  AND confrelid = 'fod.data_objects'::regclass
                  AND contype = 'f'
                  AND confdeltype = 'c'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_constraint
                WHERE conname = 'copy_block_crc_pkey'
                  AND conrelid = 'fod.copy_block_crc'::regclass
                  AND contype = 'p'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_constraint
                WHERE conname = 'copy_block_crc_data_object_id_fkey'
                  AND conrelid = 'fod.copy_block_crc'::regclass
                  AND confrelid = 'fod.data_objects'::regclass
                  AND contype = 'f'
                  AND confdeltype = 'c'
            )",
    )
}

fn schema_objects_exist(conn: &DbConn, schema: &str, prune_function: &str) -> Result<bool, String> {
    conn.query_exists(&format!(
        "SELECT \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regclass({}) IS NOT NULL OR \
            to_regprocedure({}) IS NOT NULL",
        quote_schema_regclass(schema, "directories"),
        quote_schema_regclass(schema, "files"),
        quote_schema_regclass(schema, "special_files"),
        quote_schema_regclass(schema, "hardlinks"),
        quote_schema_regclass(schema, "symlinks"),
        quote_schema_regclass(schema, "data_blocks"),
        quote_schema_regclass(schema, "config"),
        quote_schema_regclass(schema, "schema_version"),
        quote_schema_regclass(schema, "schema_admin"),
        quote_schema_regclass(schema, "journal"),
        quote_schema_regclass(schema, "xattrs"),
        quote_schema_regclass(schema, "lock_leases"),
        quote_schema_regclass(schema, "lock_range_leases"),
        quote_schema_regclass(schema, "data_objects"),
        quote_schema_regclass(schema, "data_object_request_tokens"),
        quote_schema_regclass(schema, "copy_block_crc"),
        quote_schema_regclass(schema, "client_sessions"),
        quote_schema_regclass(schema, "client_session_owner_keys"),
        quote_schema_regprocedure(schema, prune_function, "")
    ))
}

fn fod_objects_exist(conn: &DbConn) -> Result<bool, String> {
    schema_objects_exist(
        conn,
        FOD_SCHEMA_NAME,
        "fod_prune_client_session_lock_leases",
    )
}

fn schema_version_label(version: Option<u64>) -> String {
    version
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn read_storage_config_value(conn: &DbConn, key: &str) -> Result<Option<u64>, String> {
    if !conn.query_exists(&format!(
        "SELECT to_regclass({}) IS NOT NULL",
        quote_schema_regclass(FOD_SCHEMA_NAME, "config")
    ))? {
        return Ok(None);
    }

    conn.query_scalar_u64(&format!(
        "SELECT value FROM {} WHERE key = {}",
        quote_schema_qualified_ident(FOD_SCHEMA_NAME, "config"),
        DbConn::quote_literal(key)
    ))
}

fn ensure_fod_privileges(conn: &DbConn, db_user: &str) -> Result<(), String> {
    run_sql_commands_user(
        conn,
        "GRANT USAGE ON SCHEMA fod TO {}; \
         GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA fod TO {}; \
         GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA fod TO {}",
        db_user,
    )
}

fn acquire_schema_upgrade_lock(conn: &DbConn) -> Result<(), String> {
    let acquired = conn
        .query_scalar_text("SELECT pg_try_advisory_lock(1180124246)::text")?
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "t" | "true" | "1"
            )
        })
        .unwrap_or(false);
    if acquired {
        Ok(())
    } else {
        Err("another mkfs.fod schema upgrade is already running".to_string())
    }
}

fn begin_schema_upgrade(conn: &DbConn) -> Result<(), String> {
    conn.exec("BEGIN")?;
    let result = (|| {
        let has_legacy_version_column = conn.query_exists(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_schema = 'fod' AND table_name = 'schema_version' AND column_name = 'version')",
        )?;
        let has_schema_number_column = conn.query_exists(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_schema = 'fod' AND table_name = 'schema_version' AND column_name = 'schema_number')",
        )?;
        if has_legacy_version_column && !has_schema_number_column {
            conn.exec("ALTER TABLE fod.schema_version RENAME COLUMN version TO schema_number")?;
        }
        conn.exec(
            "ALTER TABLE fod.schema_version \
             ADD COLUMN IF NOT EXISTS minimum_client_version TEXT NOT NULL DEFAULT '0.0.0'",
        )?;
        conn.exec(
            "ALTER TABLE fod.schema_version \
             ADD COLUMN IF NOT EXISTS upgrade_in_progress BOOLEAN NOT NULL DEFAULT FALSE",
        )?;

        if conn.query_exists("SELECT to_regclass('fod.client_sessions') IS NOT NULL")? {
            let active_sessions = || {
                conn.query_scalar_u64(
                    "SELECT COUNT(*) FROM fod.client_sessions WHERE lease_expires_at > NOW()",
                )
                .map(|count| count.unwrap_or(0))
            };
            let active_count_before_guard = active_sessions()?;
            if active_count_before_guard != 0 {
                return Err(format!(
                    "cannot upgrade FOD schema while {active_count_before_guard} client session(s) are active; unmount every FOD mount and retry"
                ));
            }
            conn.exec(
                "CREATE OR REPLACE FUNCTION fod.fod_reject_client_session_during_schema_upgrade() \
                 RETURNS trigger LANGUAGE plpgsql AS $$ \
                 BEGIN \
                    IF COALESCE((SELECT upgrade_in_progress FROM fod.schema_version ORDER BY applied_at DESC LIMIT 1), FALSE) THEN \
                        RAISE EXCEPTION 'FOD schema upgrade is in progress or was interrupted'; \
                    END IF; \
                    RETURN NEW; \
                 END; \
                 $$; \
                 DROP TRIGGER IF EXISTS trg_fod_client_session_schema_upgrade ON fod.client_sessions; \
                 CREATE TRIGGER trg_fod_client_session_schema_upgrade \
                 BEFORE INSERT ON fod.client_sessions FOR EACH ROW \
                 EXECUTE FUNCTION fod.fod_reject_client_session_during_schema_upgrade()",
            )?;
            let active_count_after_guard = active_sessions()?;
            if active_count_after_guard != 0 {
                return Err(format!(
                    "cannot upgrade FOD schema while {active_count_after_guard} client session(s) are active; unmount every FOD mount and retry"
                ));
            }
        }

        if let Some(minimum_client_version) = conn.query_scalar_text(
            "SELECT minimum_client_version FROM fod.schema_version ORDER BY applied_at DESC LIMIT 1",
        )? {
            if compare_fod_client_versions(FOD_VERSION, &minimum_client_version)?
                == std::cmp::Ordering::Less
            {
                return Err(format!(
                    "FOD client {FOD_VERSION} cannot lower database minimum client version {minimum_client_version}"
                ));
            }
        }
        conn.exec(&format!(
            "UPDATE fod.schema_version \
             SET minimum_client_version = {}, upgrade_in_progress = TRUE",
            DbConn::quote_literal(FOD_VERSION)
        ))?;
        Ok(())
    })();

    match result {
        Ok(()) => conn.exec("COMMIT"),
        Err(err) => {
            let _ = conn.exec("ROLLBACK");
            Err(err)
        }
    }
}

fn write_schema_version(conn: &DbConn, version: u64) -> Result<(), String> {
    conn.exec("BEGIN")?;
    let result = (|| {
        // Serialize concurrent mkfs writers before reading the current floor.
        conn.query_scalar_text("SELECT pg_advisory_xact_lock(1180124246)::text")?;
        if let Some(minimum_client_version) = conn.query_scalar_text(
            "SELECT minimum_client_version FROM schema_version ORDER BY applied_at DESC LIMIT 1",
        )? {
            if compare_fod_client_versions(FOD_VERSION, &minimum_client_version)?
                == std::cmp::Ordering::Less
            {
                return Err(format!(
                    "FOD client {FOD_VERSION} cannot lower database minimum client version {minimum_client_version}"
                ));
            }
        }
        conn.exec(&format!(
            "DELETE FROM schema_version; \
             INSERT INTO schema_version \
                (schema_number, applied_at, minimum_client_version, upgrade_in_progress) \
             VALUES ({}, NOW(), {}, FALSE)",
            version,
            DbConn::quote_literal(FOD_VERSION)
        ))
    })();

    match result {
        Ok(()) => conn.exec("COMMIT"),
        Err(err) => {
            let _ = conn.exec("ROLLBACK");
            Err(err)
        }
    }
}

fn run_sql_commands_user(conn: &DbConn, sql_commands: &str, db_user: &str) -> Result<(), String> {
    let sql = sql_commands.replace("{}", &DbConn::quote_identifier(db_user));
    conn.exec(&sql)
}

fn set_search_path(conn: &DbConn, search_path: &str) -> Result<(), String> {
    conn.exec(&format!("SET search_path TO {}", search_path))
}

fn main() {
    let cli = Cli::parse();
    if cli.json && cli.action != Action::Status {
        eprintln!("--json is only supported with the status action");
        std::process::exit(2);
    }
    if cli.block_size % 1024 != 0 {
        eprintln!("block_size must be a multiple of 1024");
        std::process::exit(1);
    }

    let config_path = match resolve_config_path(None) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let (config, config_dir) = match load_config_parser(Some(&config_path)) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let runtime = match load_runtime_config(&config) {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let db_section = match config.section("database") {
        Some(section) => section.clone(),
        None => {
            eprintln!("Missing [database] section in FOD configuration");
            std::process::exit(1);
        }
    };
    let params = resolve_pg_connection_params(&db_section, &config_dir);
    let conninfo = make_conninfo(&params);
    let conn = match DbConn::connect(&conninfo) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let postgres_requirements =
        match conn.postgres_runtime_requirements(runtime.pool_max_connections) {
            Ok(requirements) => requirements,
            Err(err) => {
                eprintln!("PostgreSQL runtime requirements validation failed: {err}");
                std::process::exit(1);
            }
        };
    let postgres_configuration_warnings =
        match postgres_requirements.server_configuration_warnings() {
            Ok(warnings) => warnings,
            Err(err) => {
                eprintln!("PostgreSQL runtime requirements validation failed: {err}");
                std::process::exit(1);
            }
        };
    for warning in &postgres_configuration_warnings {
        eprintln!("FOD PostgreSQL instance configuration requires attention: {warning}");
    }

    let (schema_admin_password, schema_admin_source) = load_schema_admin_password(&cli);
    let uid_gid = current_uid_gid();

    match cli.action {
        Action::Init => {
            if schema_admin_password.is_none() {
                eprintln!("{}", schema_admin_secret_required_message("init"));
                std::process::exit(1);
            }
            let existing_fod = match fod_objects_exist(&conn) {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("{}", err);
                    std::process::exit(1);
                }
            };
            if existing_fod {
                eprintln!(
                    "fod schema objects already exist in this database; run fod-mkfs upgrade instead of init."
                );
                std::process::exit(1);
            }
            if cli.generate_client_tls_pair {
                if let Err(err) = generate_client_tls_pair(
                    Path::new(&cli.tls_material_dir),
                    &cli.tls_common_name,
                    cli.tls_cert_days,
                ) {
                    eprintln!("{}", err);
                    std::process::exit(1);
                }
            }
            println!(
                "{}",
                format_schema_admin_source_message(schema_admin_source.as_deref().unwrap_or("cli"))
            );
            if let Err(err) = apply_base_schema(&conn) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            if let Err(err) = set_search_path(&conn, FOD_SEARCH_PATH) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            if let Err(err) = ensure_schema_admin_secret(&conn, schema_admin_password.as_deref()) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            if let Err(err) = conn.exec(
                "CREATE TABLE IF NOT EXISTS config (key VARCHAR(50) PRIMARY KEY, value BIGINT)",
            ) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            if let Err(err) = conn.exec(&format!(
                "INSERT INTO config (key, value) VALUES ('max_fs_size_bytes', {}) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
                10_u64 * 1024 * 1024 * 1024
            )) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            if let Err(err) = conn.exec(&format!(
                "INSERT INTO config (key, value) VALUES ('block_size', {}) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
                cli.block_size
            )) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            if let Err(err) = write_schema_version(&conn, SCHEMA_VERSION) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            let (uid, gid) = uid_gid;
            if let Err(err) = conn.exec(&format!(
                "UPDATE directories SET uid = {}, gid = {} WHERE name IN ('/', '.Trash-1000') AND id_parent IS NULL",
                uid, gid
            )) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            let db_user = params
                .get("user")
                .cloned()
                .unwrap_or_else(|| "foduser".to_string());
            if let Err(err) = ensure_fod_privileges(&conn, &db_user) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            println!("Initialization completed successfully.");
        }
        Action::Upgrade => {
            if schema_admin_password.is_none() {
                eprintln!("{}", schema_admin_secret_required_message("upgrade"));
                std::process::exit(1);
            }
            println!(
                "{}",
                format_schema_admin_source_message(schema_admin_source.as_deref().unwrap_or("cli"))
            );
            let fod_exists = match fod_objects_exist(&conn) {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("{}", err);
                    std::process::exit(1);
                }
            };
            if !fod_exists {
                eprintln!(
                    "fod schema objects do not exist in this database; run fod-mkfs init instead of upgrade."
                );
                std::process::exit(1);
            }
            if let Err(err) = set_search_path(&conn, FOD_SCHEMA_NAME) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            if let Err(err) = ensure_schema_admin_secret(&conn, schema_admin_password.as_deref()) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            if let Err(err) = acquire_schema_upgrade_lock(&conn) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            let current_version = match read_schema_version(&conn) {
                Ok(version) => version,
                Err(err) => {
                    eprintln!("{}", err);
                    std::process::exit(1);
                }
            };
            if let Some(version) = current_version {
                if version > SCHEMA_VERSION {
                    eprintln!(
                        "Unsupported schema version {}; expected {}.",
                        version, SCHEMA_VERSION
                    );
                    std::process::exit(1);
                }
            }
            if let Err(err) = begin_schema_upgrade(&conn) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            let recovered_latest_shape = if current_version.is_none() {
                match latest_schema_shape_matches(&conn) {
                    Ok(matches) => matches,
                    Err(err) => {
                        eprintln!("{}", err);
                        std::process::exit(1);
                    }
                }
            } else {
                false
            };
            let start_version = if recovered_latest_shape {
                SCHEMA_VERSION
            } else {
                current_version.unwrap_or(0)
            };
            for version in (start_version + 1)..=SCHEMA_VERSION {
                if !migration_exists(version) {
                    eprintln!("Missing migration file for version {}", version);
                    std::process::exit(1);
                }
                if let Err(err) = apply_migration(&conn, version) {
                    eprintln!("{}", err);
                    std::process::exit(1);
                }
            }
            if let Err(err) = set_search_path(&conn, FOD_SEARCH_PATH) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            if let Err(err) = write_schema_version(&conn, SCHEMA_VERSION) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            let db_user = params
                .get("user")
                .cloned()
                .unwrap_or_else(|| "foduser".to_string());
            if let Err(err) = ensure_fod_privileges(&conn, &db_user) {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            if current_version == Some(SCHEMA_VERSION) {
                println!("Schema already at version {}.", SCHEMA_VERSION);
            } else {
                if recovered_latest_shape {
                    println!(
                        "Schema version row was missing; recovered version {} from the verified schema shape.",
                        SCHEMA_VERSION
                    );
                }
                println!("Schema upgraded to version {}.", SCHEMA_VERSION);
            }
        }
        Action::Clean => {
            let exists = match fod_objects_exist(&conn) {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("{}", err);
                    std::process::exit(1);
                }
            };
            if !exists {
                println!("Cleanup completed.");
                return;
            }
            if schema_admin_password.is_none() {
                eprintln!("{}", schema_admin_secret_required_message("clean"));
                std::process::exit(1);
            }
            println!(
                "{}",
                format_schema_admin_source_message(schema_admin_source.as_deref().unwrap_or("cli"))
            );
            if let Err(err) =
                verify_existing_schema_admin_secret(&conn, schema_admin_password.as_deref())
            {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            if let Err(err) = conn.exec("DROP SCHEMA IF EXISTS fod CASCADE") {
                eprintln!("{}", err);
                std::process::exit(1);
            }
            println!("Cleanup completed.");
        }
        Action::Status => {
            let fod_exists = match fod_objects_exist(&conn) {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("{}", err);
                    std::process::exit(1);
                }
            };
            let fod_version = match read_schema_version(&conn) {
                Ok(version) => version,
                Err(err) => {
                    eprintln!("{}", err);
                    std::process::exit(1);
                }
            };
            let current_version = fod_version;
            let minimum_client_version = match read_minimum_client_version(&conn) {
                Ok(version) => version,
                Err(err) => {
                    eprintln!("FOD client compatibility diagnostics unavailable: {}", err);
                    std::process::exit(1);
                }
            };
            let upgrade_in_progress = match read_schema_upgrade_in_progress(&conn) {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("FOD schema upgrade diagnostics unavailable: {}", err);
                    std::process::exit(1);
                }
            };
            let latest_version = latest_migration_version();
            let pending_versions: Vec<u64> =
                ((current_version.unwrap_or(0) + 1)..=latest_version).collect();
            let manifest = migration_manifest();
            let secret_present = match schema_admin_secret_exists(&conn) {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("{}", err);
                    std::process::exit(1);
                }
            };
            let postgres_versions = match conn.postgres_version_diagnostics() {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("PostgreSQL runtime diagnostics unavailable: {}", err);
                    std::process::exit(1);
                }
            };
            let latest_shape = if fod_exists && current_version == Some(latest_version) {
                match latest_schema_shape_matches(&conn) {
                    Ok(matches) => matches,
                    Err(err) => {
                        eprintln!("{}", err);
                        std::process::exit(1);
                    }
                }
            } else {
                false
            };
            let ready = fod_exists
                && current_version == Some(latest_version)
                && !upgrade_in_progress
                && secret_present
                && latest_shape;
            let block_size_bytes = match read_storage_config_value(&conn, "block_size") {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("FOD storage block-size diagnostics unavailable: {}", err);
                    std::process::exit(1);
                }
            };
            let max_fs_size_bytes = match read_storage_config_value(&conn, "max_fs_size_bytes") {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("FOD storage max-size diagnostics unavailable: {}", err);
                    std::process::exit(1);
                }
            };

            if cli.json {
                let migration_path = manifest
                    .iter()
                    .map(|(version, filename, description)| {
                        json!({
                            "version": version,
                            "filename": filename,
                            "description": description,
                        })
                    })
                    .collect::<Vec<_>>();
                let postgres_settings = postgres_requirements
                    .settings
                    .values()
                    .map(|setting| {
                        json!({
                            "name": setting.name,
                            "setting": setting.setting,
                            "unit": setting.unit,
                            "context": setting.context,
                            "pending_restart": setting.pending_restart,
                        })
                    })
                    .collect::<Vec<_>>();
                let session_configuration_errors =
                    postgres_requirements.session_configuration_errors();

                let payload = json!({
                    "schema_version": STATUS_JSON_SCHEMA_VERSION,
                    "fod_version": env!("CARGO_PKG_VERSION"),
                    "postgresql": {
                        "libpq_runtime": {
                            "version": postgres_versions.libpq_version,
                            "version_num": postgres_versions.libpq_version_num,
                        },
                        "server_runtime": {
                            "version": postgres_versions.server_version,
                            "version_num": postgres_versions.server_version_num,
                        },
                        "client_server_major_relation": postgres_versions.major_relation,
                        "compatibility": postgres_versions.compatibility_label(),
                        "requirements": {
                            "minimum_server_version_num": postgres_requirements.minimum_server_version_num,
                            "pool_max_connections": postgres_requirements.pool_max_connections,
                            "required_max_connections": postgres_requirements.required_max_connections,
                            "server_configuration_warnings": postgres_configuration_warnings,
                            "session_configuration_errors": session_configuration_errors,
                            "settings": postgres_settings,
                        },
                    },
                    "schema": {
                        "name": FOD_SCHEMA_NAME,
                        "version": current_version,
                        "minimum_client_version": minimum_client_version,
                        "upgrade_in_progress": upgrade_in_progress,
                        "latest_migration_version": latest_version,
                        "objects_present": fod_exists,
                        "latest_shape": latest_shape,
                        "admin_secret_present": secret_present,
                        "ready": ready,
                        "pending_migrations": pending_versions,
                        "migration_path": migration_path,
                    },
                    "storage_format": {
                        "canonical_schema": FOD_SCHEMA_NAME,
                        "active_schema": if fod_exists { FOD_SCHEMA_NAME } else { "none" },
                        "block_size_bytes": block_size_bytes,
                        "max_fs_size_bytes": max_fs_size_bytes,
                    },
                });

                match serde_json::to_string_pretty(&payload) {
                    Ok(payload) => println!("{}", payload),
                    Err(err) => {
                        eprintln!("unable to serialize FOD status JSON: {}", err);
                        std::process::exit(1);
                    }
                }
            } else {
                println!("FOD version: {}", FOD_VERSION_LABEL);
                println!(
                    "PostgreSQL libpq runtime: {} ({})",
                    postgres_versions.libpq_version, postgres_versions.libpq_version_num
                );
                println!(
                    "PostgreSQL server runtime: {} ({})",
                    postgres_versions.server_version, postgres_versions.server_version_num
                );
                println!(
                    "PostgreSQL client/server major relation: {}",
                    postgres_versions.major_relation
                );
                println!(
                    "PostgreSQL client/server compatibility: {}",
                    postgres_versions.compatibility_label()
                );
                println!("FOD schema name: {}", FOD_SCHEMA_NAME);
                println!(
                    "FOD minimum client version: {}",
                    minimum_client_version.as_deref().unwrap_or("unavailable")
                );
                println!(
                    "FOD schema upgrade in progress: {}",
                    if upgrade_in_progress { "yes" } else { "no" }
                );
                println!(
                    "FOD schema version: {}",
                    schema_version_label(current_version)
                );
                println!("Canonical FOD storage schema: {}", FOD_SCHEMA_NAME);
                println!("Active schema: {}", if fod_exists { "fod" } else { "none" });
                println!(
                    "FOD storage block size: {}",
                    block_size_bytes
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unavailable".to_string())
                );
                println!(
                    "FOD max filesystem size: {}",
                    max_fs_size_bytes
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unavailable".to_string())
                );
                println!("fod objects: {}", if fod_exists { "yes" } else { "no" });
                println!("Latest migration version: {}", latest_version);
                println!(
                    "Latest schema shape: {}",
                    if latest_shape { "yes" } else { "no" }
                );
                println!(
                    "Schema admin secret: {}",
                    if secret_present { "present" } else { "missing" }
                );
                println!("FOD ready: {}", if ready { "yes" } else { "no" });
                if pending_versions.is_empty() {
                    println!("Pending migrations: none");
                } else {
                    let joined = pending_versions
                        .iter()
                        .map(|version| format!("{:04}", version))
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("Pending migrations: {}", joined);
                }
                println!("Migration path:");
                for (version, filename, description) in manifest {
                    println!("  - {:04}: {} :: {}", version, filename, description);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        base_schema_sql, migration_manifest, quote_schema_regclass, quote_schema_regprocedure,
        SCHEMA_VERSION, STATUS_JSON_SCHEMA_VERSION,
    };
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn status_json_schema_version_is_stable() {
        assert_eq!(STATUS_JSON_SCHEMA_VERSION, 1);
    }

    #[test]
    fn base_schema_sql_excludes_legacy_upgrade_migrations() {
        let sql = base_schema_sql();
        assert!(sql.contains("CREATE SCHEMA IF NOT EXISTS fod;"));
        assert!(sql.contains("SET search_path TO fod, public;"));
        assert!(!sql.contains("ALTER SCHEMA fod RENAME TO fod"));
        assert!(!sql.contains("ALTER TABLE IF EXISTS public."));
        assert!(!sql.contains("ALTER SEQUENCE IF EXISTS public."));
        assert!(sql.contains(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_data_blocks_object_order ON data_blocks (data_object_id, _order);"
        ));
        assert!(!sql.contains("idx_data_blocks_data_object_id"));
    }

    #[test]
    fn quote_schema_regclass_escapes_identifiers_for_sql_literal() {
        assert_eq!(
            quote_schema_regclass("fo'd", r#"ab"c"#),
            "'\"fo''d\".\"ab\"\"c\"'"
        );
    }

    #[test]
    fn quote_schema_regprocedure_escapes_identifiers_for_sql_literal() {
        assert_eq!(
            quote_schema_regprocedure("fo'd", r#"pr"oc"#, ""),
            "'\"fo''d\".\"pr\"\"oc\"()'"
        );
    }

    #[test]
    fn migration_manifest_matches_numbered_migration_files() {
        let migrations_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("rust_mkfs must live below the repository root")
            .join("migrations");
        let mut disk_files = fs::read_dir(&migrations_dir)
            .expect("read migrations directory")
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| {
                name.len() > 5
                    && name.as_bytes()[0..4]
                        .iter()
                        .all(|byte| byte.is_ascii_digit())
                    && name.as_bytes()[4] == b'_'
                    && name.ends_with(".sql")
            })
            .collect::<Vec<_>>();
        disk_files.sort();

        let manifest = migration_manifest();
        let manifest_files = manifest
            .iter()
            .map(|(_, filename, _)| (*filename).to_string())
            .collect::<Vec<_>>();

        assert_eq!(manifest.len() as u64, SCHEMA_VERSION);
        assert_eq!(manifest.last().map(|entry| entry.0), Some(SCHEMA_VERSION));
        assert_eq!(disk_files, manifest_files);
    }
}
