// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

#[path = "../src/pg.rs"]
mod pg;

use pg::DbConn;
use std::env;
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static ENV_LOCK: Mutex<()> = Mutex::new(());
static DB_LOCK: Mutex<()> = Mutex::new(());
const SCHEMA_VERSION: u64 = 13;

fn conninfo_from_env() -> String {
    let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
    let dbname = env::var("POSTGRES_DB").unwrap_or_else(|_| "foddbname".to_string());
    let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "foduser".to_string());
    let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "cichosza".to_string());
    format!(
        "host='{}' port='{}' dbname='{}' user='{}' password='{}'",
        host, port, dbname, user, password
    )
}

fn unique_name(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}_{}_{}", std::process::id(), nanos)
}

fn db_guard() -> std::sync::MutexGuard<'static, ()> {
    DB_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

fn run_mkfs(action: &str, extra_args: &[&str], envs: &[(&str, String)]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fod-rust-mkfs"));
    command.arg(action);
    for arg in extra_args {
        command.arg(arg);
    }
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("failed to run fod-rust-mkfs")
}

fn postgres_envs() -> Vec<(&'static str, String)> {
    vec![
        (
            "POSTGRES_DB",
            env::var("POSTGRES_DB").unwrap_or_else(|_| "foddbname".to_string()),
        ),
        (
            "POSTGRES_USER",
            env::var("POSTGRES_USER").unwrap_or_else(|_| "foduser".to_string()),
        ),
        (
            "POSTGRES_PASSWORD",
            env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "cichosza".to_string()),
        ),
    ]
}

fn schema_admin_password() -> String {
    env::var("FOD_SCHEMA_ADMIN_PASSWORD")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("fod-{}", unique_name("schema").replace('-', "")))
}

fn assert_upgrade_message(output: &str) {
    if output.contains(&format!("Schema upgraded to version {}.", SCHEMA_VERSION))
        || output.contains(&format!("Schema already at version {}.", SCHEMA_VERSION))
    {
        return;
    }
    panic!("{output}");
}

fn assert_password_source(output: &str, source: &str) {
    let expected = format!(
        "Schema admin password source: {} (no prompt needed)",
        source
    );
    assert!(output.contains(&expected), "{output}");
}

fn table_exists(conn: &DbConn, schema: &str, table_name: &str) -> Result<bool, String> {
    conn.query_exists(&format!(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = '{}' AND table_name = '{}')",
        schema.replace('\'', "''"),
        table_name.replace('\'', "''")
    ))
}

fn schema_exists(conn: &DbConn, schema: &str) -> Result<bool, String> {
    conn.query_exists(&format!(
        "SELECT EXISTS (SELECT 1 FROM information_schema.schemata WHERE schema_name = '{}')",
        schema.replace('\'', "''")
    ))
}

#[test]
fn schema_upgrade_non_destructive_password_protected() {
    let _guard = env_guard();
    let _db_guard = db_guard();
    let conninfo = conninfo_from_env();
    let conn = DbConn::connect(&conninfo).expect("connect");

    conn.exec("DROP SCHEMA IF EXISTS fod CASCADE")
        .expect("drop fod schema");
    conn.exec("DROP SCHEMA IF EXISTS fod CASCADE")
        .expect("drop fod schema");
    conn.exec("DROP SCHEMA IF EXISTS public CASCADE")
        .expect("drop schema");
    conn.exec("CREATE SCHEMA public").expect("create schema");

    let guard_table = unique_name("schema_upgrade_guard");
    conn.exec(&format!(
        "CREATE TABLE IF NOT EXISTS {}.{} (id INTEGER PRIMARY KEY, note TEXT NOT NULL)",
        DbConn::quote_identifier("public"),
        DbConn::quote_identifier(&guard_table)
    ))
    .expect("create guard table");
    conn.exec(&format!(
        "INSERT INTO {}.{} (id, note) VALUES (1, 'guard') ON CONFLICT (id) DO UPDATE SET note = EXCLUDED.note",
        DbConn::quote_identifier("public"),
        DbConn::quote_identifier(&guard_table)
    ))
    .expect("seed guard table");

    let envs = postgres_envs();
    let schema_password = schema_admin_password();

    let init_without_secret = run_mkfs("init", &[], &envs);
    assert_ne!(init_without_secret.status.code(), Some(0));
    let init_without_output = format!(
        "{}{}",
        String::from_utf8_lossy(&init_without_secret.stdout),
        String::from_utf8_lossy(&init_without_secret.stderr)
    );
    assert!(
        init_without_output
            .contains("Schema admin password is required for init; pass --schema-admin-password."),
        "{init_without_output}"
    );

    let init_with_secret = run_mkfs(
        "init",
        &["--schema-admin-password", &schema_password],
        &envs,
    );
    assert!(
        init_with_secret.status.success(),
        "{}",
        String::from_utf8_lossy(&init_with_secret.stdout)
    );
    assert_password_source(&String::from_utf8_lossy(&init_with_secret.stdout), "cli");
    assert!(
        String::from_utf8_lossy(&init_with_secret.stdout)
            .contains("Initialization completed successfully."),
        "{}",
        String::from_utf8_lossy(&init_with_secret.stdout)
    );
    assert!(
        conn.query_exists("SELECT to_regclass('fod.schema_admin') IS NOT NULL")
            .expect("fod schema_admin exists"),
        "schema_admin should be created in fod"
    );

    assert!(table_exists(&conn, "public", &guard_table).expect("table_exists"));
    let guard_note = conn
        .query_scalar_text(&format!(
            "SELECT note FROM {}.{} WHERE id = 1",
            DbConn::quote_identifier("public"),
            DbConn::quote_identifier(&guard_table)
        ))
        .expect("select guard");
    assert_eq!(guard_note.as_deref(), Some("guard"));

    let version = conn
        .query_scalar_u64("SELECT version FROM schema_version ORDER BY applied_at DESC LIMIT 1")
        .expect("version")
        .expect("schema version");
    assert_eq!(version, SCHEMA_VERSION);

    let admin_count = conn
        .query_scalar_u64("SELECT COUNT(*) FROM schema_admin WHERE id = 1")
        .expect("admin count")
        .expect("admin count row");
    assert_eq!(admin_count, 1);

    let upgrade_wrong = run_mkfs(
        "upgrade",
        &["--schema-admin-password", "wrong-password"],
        &envs,
    );
    assert_ne!(upgrade_wrong.status.code(), Some(0));
    let upgrade_wrong_output = format!(
        "{}{}",
        String::from_utf8_lossy(&upgrade_wrong.stdout),
        String::from_utf8_lossy(&upgrade_wrong.stderr)
    );
    assert!(
        upgrade_wrong_output.contains(
            "does not match the schema-admin secret currently stored in the FOD database"
        ),
        "{upgrade_wrong_output}"
    );

    conn.exec("DELETE FROM schema_version")
        .expect("delete schema_version");

    let upgrade_result = run_mkfs(
        "upgrade",
        &["--schema-admin-password", &schema_password],
        &envs,
    );
    assert!(
        upgrade_result.status.success(),
        "{}",
        String::from_utf8_lossy(&upgrade_result.stdout)
    );
    assert_password_source(&String::from_utf8_lossy(&upgrade_result.stdout), "cli");
    assert_upgrade_message(&String::from_utf8_lossy(&upgrade_result.stdout));

    assert!(table_exists(&conn, "public", &guard_table).expect("table_exists"));
    let guard_note = conn
        .query_scalar_text(&format!(
            "SELECT note FROM {}.{} WHERE id = 1",
            DbConn::quote_identifier("public"),
            DbConn::quote_identifier(&guard_table)
        ))
        .expect("select guard");
    assert_eq!(guard_note.as_deref(), Some("guard"));

    let version = conn
        .query_scalar_u64("SELECT version FROM schema_version ORDER BY applied_at DESC LIMIT 1")
        .expect("version")
        .expect("schema version");
    assert_eq!(version, SCHEMA_VERSION);

    let clean_missing_secret = run_mkfs("clean", &[], &envs);
    assert_ne!(clean_missing_secret.status.code(), Some(0));
    let clean_missing_output = format!(
        "{}{}",
        String::from_utf8_lossy(&clean_missing_secret.stdout),
        String::from_utf8_lossy(&clean_missing_secret.stderr)
    );
    assert!(
        clean_missing_output
            .contains("Schema admin password is required for clean; pass --schema-admin-password."),
        "{clean_missing_output}"
    );

    let clean_result = run_mkfs(
        "clean",
        &["--schema-admin-password", &schema_password],
        &envs,
    );
    assert!(
        clean_result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&clean_result.stdout),
        String::from_utf8_lossy(&clean_result.stderr)
    );
    assert_password_source(&String::from_utf8_lossy(&clean_result.stdout), "cli");
    assert!(
        String::from_utf8_lossy(&clean_result.stdout).contains("Cleanup completed."),
        "{}",
        String::from_utf8_lossy(&clean_result.stdout)
    );
    assert!(
        !schema_exists(&conn, "fod").expect("fod schema_exists after clean"),
        "fod schema should be dropped by clean"
    );

    let clean_again = run_mkfs(
        "clean",
        &["--schema-admin-password", &schema_password],
        &envs,
    );
    assert!(
        clean_again.status.success(),
        "{}",
        String::from_utf8_lossy(&clean_again.stdout)
    );
    assert!(
        String::from_utf8_lossy(&clean_again.stdout).contains("Cleanup completed."),
        "{}",
        String::from_utf8_lossy(&clean_again.stdout)
    );
    assert!(
        !schema_exists(&conn, "fod").expect("fod schema_exists after clean_again"),
        "fod schema should stay dropped after clean"
    );

    let init_after_clean = run_mkfs(
        "init",
        &["--schema-admin-password", &schema_password],
        &envs,
    );
    assert!(
        init_after_clean.status.success(),
        "{}",
        String::from_utf8_lossy(&init_after_clean.stdout)
    );
    assert_password_source(&String::from_utf8_lossy(&init_after_clean.stdout), "cli");
    assert!(
        String::from_utf8_lossy(&init_after_clean.stdout)
            .contains("Initialization completed successfully."),
        "{}",
        String::from_utf8_lossy(&init_after_clean.stdout)
    );
    assert!(
        schema_exists(&conn, "fod").expect("fod schema_exists after init"),
        "fod schema should be recreated by init"
    );

    let upgrade_after_clean = run_mkfs(
        "upgrade",
        &["--schema-admin-password", &schema_password],
        &envs,
    );
    assert!(
        upgrade_after_clean.status.success(),
        "{}",
        String::from_utf8_lossy(&upgrade_after_clean.stdout)
    );
    assert_password_source(&String::from_utf8_lossy(&upgrade_after_clean.stdout), "cli");
    assert_upgrade_message(&String::from_utf8_lossy(&upgrade_after_clean.stdout));

    conn.exec(&format!(
        "UPDATE {} SET version = {}",
        DbConn::quote_identifier("schema_version"),
        SCHEMA_VERSION - 1
    ))
    .expect("downgrade schema version");

    let upgrade_result = run_mkfs(
        "upgrade",
        &["--schema-admin-password", &schema_password],
        &envs,
    );
    assert!(
        upgrade_result.status.success(),
        "{}",
        String::from_utf8_lossy(&upgrade_result.stdout)
    );
    assert_password_source(&String::from_utf8_lossy(&upgrade_result.stdout), "cli");
    assert_upgrade_message(&String::from_utf8_lossy(&upgrade_result.stdout));

    conn.exec("DELETE FROM schema_admin")
        .expect("delete schema_admin");
    conn.exec("DELETE FROM lock_range_leases")
        .expect("delete lock_range_leases");
    conn.exec("UPDATE schema_version SET version = 1")
        .expect("downgrade to v1");

    let upgrade_result = run_mkfs(
        "upgrade",
        &["--schema-admin-password", &schema_password],
        &envs,
    );
    assert!(
        upgrade_result.status.success(),
        "{}",
        String::from_utf8_lossy(&upgrade_result.stdout)
    );
    assert_password_source(&String::from_utf8_lossy(&upgrade_result.stdout), "cli");
    assert!(
        String::from_utf8_lossy(&upgrade_result.stdout)
            .contains(&format!("Schema upgraded to version {}.", SCHEMA_VERSION)),
        "{}",
        String::from_utf8_lossy(&upgrade_result.stdout)
    );

    assert!(table_exists(&conn, "fod", "schema_admin").expect("schema_admin exists"));
    assert!(table_exists(&conn, "fod", "lock_range_leases").expect("lock_range_leases exists"));
    let version = conn
        .query_scalar_u64("SELECT version FROM schema_version ORDER BY applied_at DESC LIMIT 1")
        .expect("version")
        .expect("schema version");
    assert_eq!(version, SCHEMA_VERSION);

    conn.exec(&format!(
        "DROP TABLE IF EXISTS {}.{}",
        DbConn::quote_identifier("public"),
        DbConn::quote_identifier(&guard_table)
    ))
    .expect("drop guard table");

    println!("OK schema-upgrade/non-destructive/password-protected");
}

#[test]
fn schema_status_reports_version_secret_and_pending_migrations() {
    let _guard = env_guard();
    let _db_guard = db_guard();
    let conninfo = conninfo_from_env();
    let conn = DbConn::connect(&conninfo).expect("connect");

    conn.exec("DROP SCHEMA IF EXISTS fod CASCADE")
        .expect("drop fod schema");
    conn.exec("DROP SCHEMA IF EXISTS fod CASCADE")
        .expect("drop fod schema");
    conn.exec("DROP SCHEMA IF EXISTS public CASCADE")
        .expect("drop schema");
    conn.exec("CREATE SCHEMA public").expect("create schema");

    let envs = postgres_envs();
    let schema_password = schema_admin_password();
    let init = run_mkfs(
        "init",
        &["--schema-admin-password", &schema_password],
        &envs,
    );
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stdout)
    );
    assert_password_source(&String::from_utf8_lossy(&init.stdout), "cli");

    let schema_admin_hash = conn
        .query_scalar_text("SELECT password_hash FROM schema_admin WHERE id = 1")
        .expect("schema admin hash")
        .expect("schema admin hash row");
    let schema_admin_salt = conn
        .query_scalar_text("SELECT password_salt FROM schema_admin WHERE id = 1")
        .expect("schema admin salt")
        .expect("schema admin salt row");
    let schema_admin_iterations = conn
        .query_scalar_u64("SELECT password_iterations FROM schema_admin WHERE id = 1")
        .expect("schema admin iterations")
        .expect("schema admin iterations row");
    let schema_admin_created_at = conn
        .query_scalar_text("SELECT created_at::text FROM schema_admin WHERE id = 1")
        .expect("schema admin created_at")
        .expect("schema admin created_at row");
    let schema_admin_updated_at = conn
        .query_scalar_text("SELECT updated_at::text FROM schema_admin WHERE id = 1")
        .expect("schema admin updated_at")
        .expect("schema admin updated_at row");

    let status_after_init =
        String::from_utf8(run_mkfs("status", &[], &envs).stdout).expect("status output");
    for needle in [
        "FOD version: FOD ",
        "FOD schema name: fod",
        "Canonical FOD storage schema: fod",
        "FOD schema version: 13",
        "Active schema: fod",
        "fod objects: yes",
        "Latest migration version: 13",
        "Schema admin secret: present",
        "FOD ready: yes",
        "Pending migrations: none",
        "0001: 0001_base.sql",
        "0002: 0002_schema_admin.sql",
        "0003: 0003_schema_version_sql.sql",
        "0004: 0004_copy_block_crc.sql",
        "0005: 0005_data_objects.sql",
        "0006: 0006_data_objects_hash_dedupe.sql",
        "0007: 0007_copy_block_crc_object_key.sql",
        "0008: 0008_client_sessions.sql",
        "0009: 0009_client_session_lock_cleanup.sql",
        "0010: 0010_fod_schema.sql",
        "0011: 0011_rename_fod_schema.sql",
        "0012: 0012_data_extents.sql",
        "0013: 0013_indexer.sql",
    ] {
        assert!(
            status_after_init.contains(needle),
            "missing needle in status_after_init: {needle}\n\n{status_after_init}"
        );
    }

    conn.exec("DELETE FROM schema_version")
        .expect("delete schema_version");
    let status_without_version =
        String::from_utf8(run_mkfs("status", &[], &envs).stdout).expect("status output");
    for needle in [
        "FOD version: FOD ",
        "FOD schema name: fod",
        "FOD schema version: none",
        "Canonical FOD storage schema: fod",
        "Active schema: fod",
        "fod objects: yes",
        "Latest migration version: 13",
        "Schema admin secret: present",
        "FOD ready: no",
        "Pending migrations: 0001, 0002, 0003, 0004, 0005, 0006, 0007, 0008, 0009, 0010, 0011, 0012, 0013",
    ] {
        assert!(
            status_without_version.contains(needle),
            "missing needle={needle:?}\n{status_without_version}"
        );
    }

    conn.exec("DELETE FROM schema_admin")
        .expect("delete schema_admin");
    let status_without_secret =
        String::from_utf8(run_mkfs("status", &[], &envs).stdout).expect("status output");
    for needle in ["Schema admin secret: missing", "FOD ready: no"] {
        assert!(
            status_without_secret.contains(needle),
            "{status_without_secret}"
        );
    }

    conn.exec(&format!(
        "INSERT INTO schema_admin (id, password_hash, password_salt, password_iterations, created_at, updated_at) VALUES (1, {}, {}, {}, {}, {})",
        DbConn::quote_literal(&schema_admin_hash),
        DbConn::quote_literal(&schema_admin_salt),
        schema_admin_iterations,
        DbConn::quote_literal(&schema_admin_created_at),
        DbConn::quote_literal(&schema_admin_updated_at),
    ))
    .expect("restore schema_admin");
    conn.exec(&format!(
        "INSERT INTO schema_version (version, applied_at) VALUES ({}, NOW())",
        SCHEMA_VERSION
    ))
    .expect("restore schema_version");

    println!("OK schema-status");
}

#[test]
fn schema_clean_requires_existing_schema_admin_secret() {
    let _guard = env_guard();
    let _db_guard = db_guard();
    let conninfo = conninfo_from_env();
    let conn = DbConn::connect(&conninfo).expect("connect");

    conn.exec("DROP SCHEMA IF EXISTS fod CASCADE")
        .expect("drop fod schema");
    conn.exec("DROP SCHEMA IF EXISTS fod CASCADE")
        .expect("drop fod schema");
    conn.exec("DROP SCHEMA IF EXISTS public CASCADE")
        .expect("drop public schema");
    conn.exec("CREATE SCHEMA public").expect("create public");

    let envs = postgres_envs();
    let schema_password = schema_admin_password();

    let init_result = run_mkfs(
        "init",
        &["--schema-admin-password", &schema_password],
        &envs,
    );
    assert!(
        init_result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&init_result.stdout),
        String::from_utf8_lossy(&init_result.stderr)
    );
    assert!(
        schema_exists(&conn, "fod").expect("fod schema_exists after init"),
        "fod schema should exist after init"
    );

    conn.exec("DELETE FROM schema_admin")
        .expect("delete schema_admin");

    let clean_result = run_mkfs(
        "clean",
        &["--schema-admin-password", &schema_password],
        &envs,
    );
    assert_ne!(clean_result.status.code(), Some(0));
    let clean_output = format!(
        "{}{}",
        String::from_utf8_lossy(&clean_result.stdout),
        String::from_utf8_lossy(&clean_result.stderr)
    );
    assert!(
        clean_output.contains(
            "Schema admin secret is missing from the FOD schema; run fod-mkfs init or upgrade first."
        ),
        "{clean_output}"
    );
    assert!(
        schema_exists(&conn, "fod").expect("fod schema_exists after failed clean"),
        "clean must not recreate or drop fod when the schema-admin secret is missing"
    );

    conn.exec("DROP SCHEMA IF EXISTS fod CASCADE")
        .expect("final cleanup of fod");
    println!("OK schema-clean-secret");
}
