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
use std::env;
use std::path::Path;

use config::{load_config_parser, resolve_config_path};
use pg::DbConn;
use pg_config::{make_conninfo, resolve_pg_connection_params};
use runtime::{FOD_SCHEMA_NAME, FOD_SEARCH_PATH};
use schema_admin::{
    ensure_schema_admin_secret, format_schema_admin_source_message, schema_admin_secret_exists,
    schema_admin_secret_required_message, verify_existing_schema_admin_secret,
};
use tls::generate_client_tls_pair;

use version::FOD_VERSION_LABEL;
const SCHEMA_VERSION: u64 = 13;
const MIGRATION_FILES: [&str; 13] = [
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
    "0012_data_extents.sql",
    "0013_indexer.sql",
];

const MIGRATION_DESCRIPTIONS: [&str; 13] = [
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
    "Introduce native extent storage",
    "Add fod-indexer metadata tables",
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
    #[arg(long, default_value_t = 4096)]
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
            "/../migrations/0012_data_extents.sql"
        )),
        13 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0013_indexer.sql"
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
    conn.query_scalar_u64(&format!(
        "SELECT version FROM {} ORDER BY applied_at DESC LIMIT 1",
        quote_schema_qualified_ident(FOD_SCHEMA_NAME, "schema_version")
    ))
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

fn ensure_fod_privileges(conn: &DbConn, db_user: &str) -> Result<(), String> {
    run_sql_commands_user(
        conn,
        "GRANT USAGE ON SCHEMA fod TO {}; \
         GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA fod TO {}; \
         GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA fod TO {}",
        db_user,
    )
}

fn write_schema_version(conn: &DbConn, version: u64) -> Result<(), String> {
    conn.exec("DELETE FROM schema_version")?;
    conn.exec(&format!(
        "INSERT INTO schema_version (version, applied_at) VALUES ({}, NOW())",
        version
    ))
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
            let start_version = current_version.unwrap_or(0);
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
            let ready = fod_exists && current_version == Some(latest_version) && secret_present;
            println!("FOD version: {}", FOD_VERSION_LABEL);
            println!("FOD schema name: {}", FOD_SCHEMA_NAME);
            println!(
                "FOD schema version: {}",
                schema_version_label(current_version)
            );
            println!("Canonical FOD storage schema: {}", FOD_SCHEMA_NAME);
            println!("Active schema: {}", if fod_exists { "fod" } else { "none" });
            println!("fod objects: {}", if fod_exists { "yes" } else { "no" });
            println!("Latest migration version: {}", latest_version);
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

#[cfg(test)]
mod tests {
    use super::{base_schema_sql, quote_schema_regclass, quote_schema_regprocedure};

    #[test]
    fn base_schema_sql_excludes_legacy_upgrade_migrations() {
        let sql = base_schema_sql();
        assert!(sql.contains("CREATE SCHEMA IF NOT EXISTS fod;"));
        assert!(sql.contains("SET search_path TO fod, public;"));
        assert!(!sql.contains("ALTER SCHEMA fod RENAME TO fod"));
        assert!(!sql.contains("ALTER TABLE IF EXISTS public."));
        assert!(!sql.contains("ALTER SEQUENCE IF EXISTS public."));
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
}
