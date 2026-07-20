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

fn endpoint_config(config_path: &PathBuf) -> Output {
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
    command
        .args(["--config-path", config_path.to_str().unwrap(), "endpoint-config"])
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
    assert_eq!(payload["role_discovery_required"], true);
    assert_eq!(payload["unknown_count"], 2);
}

#[test]
fn rejects_ambiguous_role_configuration() {
    let config_path = write_config(
        "primary_hosts = db-a:15432\nreplica_hosts = db-r:15442\nhosts = db-x:15452",
    );
    let output = endpoint_config(&config_path);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ambiguous"));
}
