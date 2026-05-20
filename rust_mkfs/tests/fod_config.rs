// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../src/config.rs"]
mod config;
#[path = "../src/pg_config.rs"]
mod pg_config;
#[path = "../src/version.rs"]
mod version;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

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

fn write_config(dir: &Path, password: &str, sslmode_line: &str) {
    let config_path = dir.join("fod_config.ini");
    let contents = format!(
        r#"
[database]
host = 127.0.0.1
port = 5432
dbname = foddbname
user = foduser
password = {password}
{sslmode_line}
sslrootcert = ca.crt
sslcert = client.crt
sslkey = client.key

[fod]
profile = bulk_write
pool_max_connections = 12
pg_visible_path = ./visible-pg
metadata_cache_ttl_seconds = 11
statfs_cache_ttl_seconds = 12
write_flush_threshold_bytes = 64MiB
max_fs_size_bytes = 10GiB
read_cache_blocks = 1024
read_ahead_blocks = 4
sequential_read_ahead_blocks = 8
small_file_read_threshold_blocks = 8
workers_read = 4
workers_read_min_blocks = 8
workers_write = 4
workers_write_min_blocks = 8
persist_buffer_chunk_blocks = 128
synchronous_commit = on
fopen_direct_io = false
fuse_writeback_cache = false
fuse_writeback_cache = false
copy_dedupe_enabled = true
copy_dedupe_min_blocks = 16
lock_heartbeat_interval_seconds = 7
selinux_context = system_u:object_r:fod_t:s0
selinux_fscontext = system_u:object_r:fod_fs_t:s0
selinux_defcontext = system_u:object_r:fod_def_t:s0
selinux_rootcontext = system_u:object_r:fod_root_t:s0

[fod.profile.bulk_write]
read_cache_blocks = 512
workers_write = 8
"#
    );
    fs::write(config_path, contents).unwrap();
}

#[test]
fn version_matches_bootstrap_and_mkfs() {
    let _guard = env_guard();
    let bootstrap = Command::new(env!("CARGO_BIN_EXE_fod-bootstrap"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(bootstrap.status.success());
    let bootstrap_version = String::from_utf8(bootstrap.stdout).unwrap();
    assert!(bootstrap_version.contains(version::FOD_VERSION_LABEL));

    let mkfs = Command::new(env!("CARGO_BIN_EXE_fod-rust-mkfs"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(mkfs.status.success());
    let mkfs_version = String::from_utf8(mkfs.stdout).unwrap();
    assert!(mkfs_version.contains(version::FOD_VERSION_LABEL));
}

#[test]
fn resolve_path_and_runtime_config_and_connection_params() {
    let _guard = env_guard();
    let temp_dir = unique_temp_dir("config");
    write_config(&temp_dir, "cichosza", "sslmode = require ; inline comment");
    let config_path = temp_dir.join("fod_config.ini");
    let _old_config = env::var_os("FOD_CONFIG");
    env::set_var("FOD_CONFIG", &config_path);

    let resolve = Command::new(env!("CARGO_BIN_EXE_fod-config"))
        .arg("resolve-path")
        .output()
        .unwrap();
    assert!(resolve.status.success());
    assert_eq!(
        String::from_utf8(resolve.stdout).unwrap().trim(),
        config_path.display().to_string()
    );

    let connection = Command::new(env!("CARGO_BIN_EXE_fod-config"))
        .arg("connection-params")
        .output()
        .unwrap();
    assert!(connection.status.success());
    let params: serde_json::Value = serde_json::from_slice(&connection.stdout).unwrap();
    assert_eq!(params["host"], "127.0.0.1");
    assert_eq!(params["port"], "5432");
    assert_eq!(params["dbname"], "foddbname");
    assert_eq!(params["user"], "foduser");
    assert_eq!(params["password"], "cichosza");
    assert_eq!(params["sslmode"], "require");
    assert_eq!(
        params["sslrootcert"],
        temp_dir.join("ca.crt").display().to_string()
    );
    assert_eq!(
        params["sslcert"],
        temp_dir.join("client.crt").display().to_string()
    );
    assert_eq!(
        params["sslkey"],
        temp_dir.join("client.key").display().to_string()
    );

    let runtime = Command::new(env!("CARGO_BIN_EXE_fod-config"))
        .env("FOD_PROFILE", "bulk_write")
        .arg("runtime-config")
        .output()
        .unwrap();
    assert!(runtime.status.success());
    let runtime: serde_json::Value = serde_json::from_slice(&runtime.stdout).unwrap();
    assert_eq!(runtime["profile"], "bulk_write");
    assert_eq!(runtime["role"], "auto");
    assert_eq!(runtime["pool_max_connections"], "12");
    assert_eq!(runtime["lock_backend"], "postgres_lease");
    assert_eq!(runtime["atime_policy"], "default");
    assert_eq!(runtime["default_permissions"], "true");
    assert_eq!(runtime["use_fuse_context"], "true");
    assert_eq!(runtime["fopen_direct_io"], "false");
    assert_eq!(runtime["fuse_writeback_cache"], "false");
    assert_eq!(runtime["use_rust_fuse"], "true");
    assert_eq!(runtime["pg_visible_path"], "./visible-pg");
    assert_eq!(runtime["metadata_cache_ttl_seconds"], "11");
    assert_eq!(runtime["statfs_cache_ttl_seconds"], "12");
    assert_eq!(runtime["read_cache_blocks"], "512");
    assert_eq!(runtime["read_ahead_blocks"], "4");
    assert_eq!(runtime["sequential_read_ahead_blocks"], "8");
    assert_eq!(runtime["small_file_read_threshold_blocks"], "8");
    assert_eq!(runtime["workers_read"], "4");
    assert_eq!(runtime["workers_read_min_blocks"], "8");
    assert_eq!(runtime["workers_write"], "8");
    assert_eq!(runtime["workers_write_min_blocks"], "8");
    assert_eq!(runtime["persist_buffer_chunk_blocks"], "128");
    assert_eq!(runtime["synchronous_commit"], "on");
    assert_eq!(runtime["copy_dedupe_enabled"], "true");
    assert_eq!(runtime["copy_dedupe_min_blocks"], "16");
    assert_eq!(runtime["lock_heartbeat_interval_seconds"], "7");
    assert_eq!(runtime["write_flush_threshold_bytes"], "67108864");
    assert_eq!(runtime["max_fs_size_bytes"], "10737418240");
    assert_eq!(runtime["selinux_context"], "system_u:object_r:fod_t:s0");
    assert_eq!(
        runtime["selinux_fscontext"],
        "system_u:object_r:fod_fs_t:s0"
    );
    assert_eq!(
        runtime["selinux_defcontext"],
        "system_u:object_r:fod_def_t:s0"
    );
    assert_eq!(
        runtime["selinux_rootcontext"],
        "system_u:object_r:fod_root_t:s0"
    );

    match _old_config {
        Some(value) => env::set_var("FOD_CONFIG", value),
        None => env::remove_var("FOD_CONFIG"),
    }
}

#[test]
fn resolve_path_fails_fast_for_missing_fod_config() {
    let _guard = env_guard();
    let temp_dir = unique_temp_dir("missing-config");
    let missing_config = temp_dir.join("fod_config.ini");
    let _old_config = env::var_os("FOD_CONFIG");
    env::set_var("FOD_CONFIG", &missing_config);

    let resolve = Command::new(env!("CARGO_BIN_EXE_fod-config"))
        .arg("resolve-path")
        .output()
        .unwrap();
    assert!(!resolve.status.success());
    let stderr = String::from_utf8(resolve.stderr).unwrap();
    assert!(stderr.contains("FOD_CONFIG"));
    assert!(stderr.contains("does not exist"));

    match _old_config {
        Some(value) => env::set_var("FOD_CONFIG", value),
        None => env::remove_var("FOD_CONFIG"),
    }
}

#[test]
fn inline_comment_markers_survive_inside_values() {
    let _guard = env_guard();
    let temp_dir = unique_temp_dir("inline-comment");
    write_config(&temp_dir, "abc#123", "sslmode = require ; inline comment");
    let config_path = temp_dir.join("fod_config.ini");
    let _old_config = env::var_os("FOD_CONFIG");
    env::set_var("FOD_CONFIG", &config_path);

    let connection = Command::new(env!("CARGO_BIN_EXE_fod-config"))
        .arg("connection-params")
        .output()
        .unwrap();
    assert!(connection.status.success());
    let params: serde_json::Value = serde_json::from_slice(&connection.stdout).unwrap();
    assert_eq!(params["password"], "abc#123");
    assert_eq!(params["sslmode"], "require");

    match _old_config {
        Some(value) => env::set_var("FOD_CONFIG", value),
        None => env::remove_var("FOD_CONFIG"),
    }
}

#[test]
fn generate_tls_command_creates_client_pair_and_reuses_existing_files() {
    let _guard = env_guard();
    let temp_dir = unique_temp_dir("generate-tls");
    write_config(&temp_dir, "cichosza", "sslmode = require");
    let config_path = temp_dir.join("fod_config.ini");
    let material_dir = temp_dir.join("tls-material");

    let output = Command::new(env!("CARGO_BIN_EXE_fod-config"))
        .args([
            "--config-path",
            config_path.to_str().unwrap(),
            "generate-tls",
            "--material-dir",
        ])
        .arg(&material_dir)
        .args(["--common-name", "fod-test", "--days", "1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first_payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let cert_path = PathBuf::from(first_payload["cert_path"].as_str().unwrap());
    let key_path = PathBuf::from(first_payload["key_path"].as_str().unwrap());
    assert_eq!(cert_path, material_dir.join("client.crt"));
    assert_eq!(key_path, material_dir.join("client.key"));
    assert!(cert_path.exists());
    assert!(key_path.exists());
    let cert_bytes = fs::read(&cert_path).unwrap();
    let key_bytes = fs::read(&key_path).unwrap();

    let second_output = Command::new(env!("CARGO_BIN_EXE_fod-config"))
        .args([
            "--config-path",
            config_path.to_str().unwrap(),
            "generate-tls",
            "--material-dir",
        ])
        .arg(&material_dir)
        .args(["--common-name", "fod-test", "--days", "1"])
        .output()
        .unwrap();
    assert!(
        second_output.status.success(),
        "{}",
        String::from_utf8_lossy(&second_output.stderr)
    );
    let second_payload: serde_json::Value = serde_json::from_slice(&second_output.stdout).unwrap();
    assert_eq!(second_payload["cert_path"], first_payload["cert_path"]);
    assert_eq!(second_payload["key_path"], first_payload["key_path"]);
    assert_eq!(fs::read(&cert_path).unwrap(), cert_bytes);
    assert_eq!(fs::read(&key_path).unwrap(), key_bytes);
}
