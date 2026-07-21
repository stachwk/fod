// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let mut path = env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.push(format!("fod-{prefix}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_config(database_lines: &str) -> PathBuf {
    let dir = unique_temp_dir("pg-endpoints");
    let config_path = dir.join("fod_config.ini");
    fs::write(
        &config_path,
        format!(
            "[database]\n{database_lines}\ndbname = foddbname\nuser = foduser\npassword = test\n\n[fod]\npool_max_connections = 4\n"
        ),
    )
    .unwrap();
    config_path
}

fn write_live_config() -> PathBuf {
    let dir = unique_temp_dir("pg-endpoint-probe");
    let config_path = dir.join("fod_config.ini");
    let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
    let dbname = env::var("POSTGRES_DB").unwrap_or_else(|_| "foddbname".to_string());
    let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "foduser".to_string());
    let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "cichosza".to_string());
    fs::write(
        &config_path,
        format!(
            "[database]\nhost = {host}\nport = {port}\ndbname = {dbname}\nuser = {user}\npassword = {password}\n\n[fod]\npool_max_connections = 4\n"
        ),
    )
    .unwrap();
    config_path
}

fn endpoint_config(config_path: &PathBuf) -> Output {
    endpoint_config_with_env(config_path, &[])
}

fn endpoint_config_with_env(config_path: &PathBuf, overrides: &[(&str, &str)]) -> Output {
    config_command_with_env(config_path, "endpoint-config", overrides)
}

fn config_command_with_env(
    config_path: &PathBuf,
    subcommand: &str,
    overrides: &[(&str, &str)],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fod-config"));
    for key in [
        "FOD_PG_HOST",
        "FOD_PG_PORT",
        "FOD_PG_PRIMARY_HOSTS",
        "FOD_PG_REPLICA_HOSTS",
        "FOD_PG_HOSTS",
    ] {
        command.env_remove(key);
    }
    for (key, value) in overrides {
        command.env(key, value);
    }
    command
        .args(["--config-path", config_path.to_str().unwrap(), subcommand])
        .output()
        .unwrap()
}

#[test]
fn reports_legacy_single_endpoint_without_changing_connection_compatibility() {
    let config_path = write_config("host = db.internal\nport = 15432");
    let output = endpoint_config(&config_path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["mode"], "legacy-single");
    assert_eq!(payload["routing_enabled"], false);
    assert_eq!(payload["role_discovery_required"], true);
    assert_eq!(payload["unknown_count"], 1);
    assert_eq!(payload["endpoints"][0]["authority"], "db.internal:15432");
}

#[test]
fn reports_explicit_primary_and_replica_roles() {
    let config_path = write_config(
        "primary_hosts = 127.0.0.1:15432,127.0.0.1:15433\nreplica_hosts = 127.0.0.1:15442,[::1]:15443",
    );
    let output = endpoint_config(&config_path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["mode"], "explicit-roles");
    assert_eq!(payload["routing_enabled"], false);
    assert_eq!(payload["role_discovery_required"], false);
    assert_eq!(payload["primary_count"], 2);
    assert_eq!(payload["replica_count"], 2);
    assert_eq!(payload["endpoints"][3]["authority"], "[::1]:15443");
}

#[test]
fn reports_transitional_hosts_as_discovery_required() {
    let config_path = write_config("hosts = db-a:15432,db-b:15442");
    let output = endpoint_config(&config_path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["mode"], "discover-roles");
    assert_eq!(payload["routing_enabled"], false);
    assert_eq!(payload["role_discovery_required"], true);
    assert_eq!(payload["unknown_count"], 2);
}

#[test]
fn environment_selects_the_endpoint_mode_over_the_config_file() {
    let discovery_config = write_config("hosts = config-unknown:15432");
    let explicit_output = endpoint_config_with_env(
        &discovery_config,
        &[
            ("FOD_PG_PRIMARY_HOSTS", "env-primary:25432"),
            ("FOD_PG_REPLICA_HOSTS", "env-replica:25442"),
        ],
    );
    assert!(
        explicit_output.status.success(),
        "{}",
        String::from_utf8_lossy(&explicit_output.stderr)
    );
    let explicit: serde_json::Value = serde_json::from_slice(&explicit_output.stdout).unwrap();
    assert_eq!(explicit["mode"], "explicit-roles");
    assert_eq!(explicit["routing_enabled"], false);
    assert_eq!(explicit["endpoints"][0]["authority"], "env-primary:25432");

    let explicit_config = write_config("primary_hosts = config-primary:15432");
    let discovery_output =
        endpoint_config_with_env(&explicit_config, &[("FOD_PG_HOSTS", "env-unknown:35432")]);
    assert!(
        discovery_output.status.success(),
        "{}",
        String::from_utf8_lossy(&discovery_output.stderr)
    );
    let discovery: serde_json::Value = serde_json::from_slice(&discovery_output.stdout).unwrap();
    assert_eq!(discovery["mode"], "discover-roles");
    assert_eq!(discovery["routing_enabled"], false);
    assert_eq!(discovery["endpoints"][0]["authority"], "env-unknown:35432");
}

#[test]
fn rejects_ambiguous_or_incomplete_role_configuration() {
    let ambiguous_config =
        write_config("primary_hosts = db-a:15432\nreplica_hosts = db-r:15442\nhosts = db-x:15452");
    let ambiguous = endpoint_config(&ambiguous_config);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("ambiguous"));

    let replica_only_config = write_config("replica_hosts = db-r:15442");
    let replica_only = endpoint_config(&replica_only_config);
    assert!(!replica_only.status.success());
    assert!(String::from_utf8_lossy(&replica_only.stderr).contains("at least one"));
}

#[test]
fn rejects_duplicates_invalid_ports_empty_entries_and_unbracketed_ipv6() {
    for (database_lines, expected_error) in [
        (
            "primary_hosts = db-a:15432\nreplica_hosts = DB-A:15432",
            "duplicate",
        ),
        ("primary_hosts = db-a:70000", "1..65535"),
        ("primary_hosts = db-a:15432,,db-b:15433", "non-empty"),
        ("primary_hosts = ::1:15432", "IPv6"),
    ] {
        let config_path = write_config(database_lines);
        let output = endpoint_config(&config_path);
        assert!(!output.status.success(), "{database_lines}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected_error),
            "expected {expected_error:?} for {database_lines:?}, stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn endpoint_probe_reports_live_server_without_enabling_routing() {
    let config_path = write_live_config();
    let output = config_command_with_env(&config_path, "endpoint-probe", &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["mode"], "legacy-single");
    assert_eq!(payload["routing_enabled"], false);
    assert_eq!(payload["probe_only"], true);
    assert_eq!(payload["endpoint_count"], 1);
    assert_eq!(payload["reachable_count"], 1);
    assert_eq!(payload["failed_count"], 0);
    assert_eq!(payload["all_probes_succeeded"], true);
    assert_eq!(payload["endpoints"][0]["connected"], true);
    assert!(payload["endpoints"][0]["observed_role"].is_string());
    assert!(payload["endpoints"][0]["role_matches_config"].is_null());
}
