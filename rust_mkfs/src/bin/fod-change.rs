// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

#[path = "../config.rs"]
mod config;
#[path = "../pg.rs"]
mod pg;
#[path = "../pg_config.rs"]
mod pg_config;
#[path = "../runtime.rs"]
mod runtime;
#[path = "../schema_admin.rs"]
mod schema_admin;
#[path = "../version.rs"]
mod version;

use clap::{ArgAction, Parser};
use config::{load_config_parser, load_runtime_config, resolve_config_path};
use fod_rust_runtime::{
    ordered_reloadable_snapshot, reloadable_snapshot_from_json, reloadable_snapshot_to_json,
    RuntimeConfig, FOD_SCHEMA_NAME,
};
use pg::DbConn;
use pg_config::{make_conninfo, resolve_pg_connection_params};
use schema_admin::verify_existing_schema_admin_secret;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "fod.change",
    version = version::FOD_VERSION_LABEL,
    about = "Store reloadable FOD runtime changes for the live change control plane."
)]
struct Cli {
    #[arg(long, help = "Path to the FOD config file or config directory.")]
    config_path: Option<PathBuf>,
    #[arg(long, help = "Schema-admin password required for --set.")]
    password: Option<String>,
    #[arg(
        long,
        value_name = "KEY",
        help = "Show the current value for a reloadable runtime key."
    )]
    get: Option<String>,
    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "List the current reloadable runtime snapshot."
    )]
    list: bool,
    #[arg(
        long = "sync-config",
        action = ArgAction::SetTrue,
        help = "Persist the reloadable snapshot derived from the current config."
    )]
    sync_config: bool,
    #[arg(
        long = "set",
        value_name = "KEY=VALUE",
        action = ArgAction::Append,
        help = "Reloadable runtime knob to change in the current live snapshot."
    )]
    changes: Vec<String>,
}

fn parse_key_value_pair(input: &str) -> Result<(String, String), String> {
    let (key, value) = input
        .split_once('=')
        .ok_or_else(|| format!("invalid change pair: {input}; expected KEY=VALUE"))?;
    let key = key.trim();
    if key.is_empty() {
        return Err(format!("invalid change pair: {input}; key is empty"));
    }
    Ok((key.to_string(), value.trim().to_string()))
}

fn load_runtime_override_snapshot(conn: &DbConn) -> Result<HashMap<String, String>, String> {
    let table_exists = conn.query_scalar_bool(&format!(
        "SELECT to_regclass('{}.runtime_overrides') IS NOT NULL",
        FOD_SCHEMA_NAME
    ))?;
    if !table_exists {
        return Ok(HashMap::new());
    }
    match conn.query_scalar_text(&format!(
        "SELECT payload_json FROM {}.runtime_overrides WHERE id = 1",
        FOD_SCHEMA_NAME
    ))? {
        Some(payload) => reloadable_snapshot_from_json(&payload),
        None => Ok(HashMap::new()),
    }
}

fn ensure_runtime_override_table(conn: &DbConn) -> Result<(), String> {
    let table_exists = conn.query_scalar_bool(&format!(
        "SELECT to_regclass('{}.runtime_overrides') IS NOT NULL",
        FOD_SCHEMA_NAME
    ))?;
    if table_exists {
        return Ok(());
    }
    conn.exec(&format!(
        "CREATE TABLE {}.runtime_overrides (\
            id INTEGER PRIMARY KEY CHECK (id = 1),\
            payload_json TEXT NOT NULL,\
            updated_at TIMESTAMP NOT NULL DEFAULT NOW()\
        )",
        FOD_SCHEMA_NAME
    ))
}

fn persist_runtime_override_snapshot(
    conn: &DbConn,
    snapshot: &HashMap<String, String>,
) -> Result<(), String> {
    ensure_runtime_override_table(conn)?;
    let payload = reloadable_snapshot_to_json(snapshot)?;
    let payload_sql = DbConn::quote_literal(&payload);
    conn.exec(&format!(
        "INSERT INTO {}.runtime_overrides (id, payload_json, updated_at) \
         VALUES (1, {}, NOW()) \
         ON CONFLICT (id) DO UPDATE SET \
            payload_json = EXCLUDED.payload_json, \
            updated_at = NOW()",
        FOD_SCHEMA_NAME, payload_sql
    ))
}

fn effective_runtime(
    runtime: &RuntimeConfig,
    persisted_snapshot: &HashMap<String, String>,
) -> Result<RuntimeConfig, String> {
    runtime.with_reloadable_overrides(persisted_snapshot)
}

fn validate_reloadable_key(key: &str) -> Result<(), String> {
    if RuntimeConfig::reloadable_setting_keys()
        .iter()
        .any(|candidate| *candidate == key)
    {
        Ok(())
    } else {
        Err(format!(
            "{} is not reloadable; restart FOD to change it.",
            key
        ))
    }
}

fn main() {
    let cli = Cli::parse();
    let mode_count = u8::from(cli.list)
        + u8::from(cli.sync_config)
        + u8::from(cli.get.is_some())
        + u8::from(!cli.changes.is_empty());
    if mode_count != 1 {
        eprintln!("Use exactly one of --set, --get, --list, or --sync-config.");
        std::process::exit(1);
    }
    if !cli.changes.is_empty() && cli.password.is_none() {
        eprintln!("--password is required when using --set.");
        std::process::exit(1);
    }
    let config_path = match resolve_config_path(cli.config_path.as_deref()) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let (config, config_path) = match load_config_parser(Some(&config_path)) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let runtime = match load_runtime_config(&config) {
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
    let params =
        resolve_pg_connection_params(&db_section, &config_path.parent().unwrap_or(Path::new(".")));
    let conninfo = make_conninfo(&params);
    let conn = match DbConn::connect(&conninfo) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };

    let persisted_snapshot = match load_runtime_override_snapshot(&conn) {
        Ok(snapshot) => snapshot,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let live_runtime = match effective_runtime(&runtime, &persisted_snapshot) {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let live_snapshot = live_runtime.reloadable_runtime_map();

    if cli.sync_config {
        let config_snapshot = runtime.reloadable_runtime_map();
        if let Err(err) = persist_runtime_override_snapshot(&conn, &config_snapshot) {
            eprintln!("{}", err);
            std::process::exit(1);
        }
        let entries = ordered_reloadable_snapshot(&config_snapshot)
            .into_iter()
            .map(|(key, value)| json!({"key": key, "value": value}))
            .collect::<Vec<_>>();
        println!(
            "{}",
            json!({
                "status": "synced",
                "live_snapshot": entries,
                "note": "The reloadable snapshot from the current config was persisted to PostgreSQL so the running FUSE process can consume it without remounting."
            })
        );
        return;
    }

    if cli.list {
        let entries = ordered_reloadable_snapshot(&live_snapshot)
            .into_iter()
            .map(|(key, value)| json!({"key": key, "value": value}))
            .collect::<Vec<_>>();
        println!(
            "{}",
            json!({
                "status": "ok",
                "current_snapshot": entries,
            })
        );
        return;
    }

    if let Some(key) = cli.get.as_deref() {
        if let Err(err) = validate_reloadable_key(key) {
            eprintln!("{}", err);
            std::process::exit(1);
        }
        let Some(value) = live_snapshot.get(key) else {
            eprintln!("{} is not present in the current reloadable snapshot.", key);
            std::process::exit(1);
        };
        println!("{}", json!({"status": "ok", "key": key, "value": value}));
        return;
    }

    let Some(password) = cli.password.as_deref() else {
        eprintln!("--password is required when using --set.");
        std::process::exit(1);
    };
    if let Err(err) = verify_existing_schema_admin_secret(&conn, Some(password)) {
        eprintln!("{}", err);
        std::process::exit(1);
    }

    let mut requested = HashMap::new();
    for change in &cli.changes {
        let (key, value) = match parse_key_value_pair(change) {
            Ok(pair) => pair,
            Err(err) => {
                eprintln!("{}", err);
                std::process::exit(1);
            }
        };
        requested.insert(key, value);
    }

    let updated_runtime = match live_runtime.with_reloadable_overrides(&requested) {
        Ok(snapshot) => snapshot,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let updated_snapshot = updated_runtime.reloadable_runtime_map();
    if let Err(err) = persist_runtime_override_snapshot(&conn, &updated_snapshot) {
        eprintln!("{}", err);
        std::process::exit(1);
    }
    let entries = ordered_reloadable_snapshot(&updated_snapshot)
        .into_iter()
        .map(|(key, value)| json!({"key": key, "value": value}))
        .collect::<Vec<_>>();
    println!(
        "{}",
        json!({
            "status": "stored",
            "live_snapshot": entries,
            "note": "The live reload consumer is wired into the running FUSE process; this stores the canonical reloadable snapshot in PostgreSQL."
        })
    );
}
