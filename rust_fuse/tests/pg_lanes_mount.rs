// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::collections::HashMap;
use std::fs;
use std::thread::sleep;
use std::time::Duration;

use fod_rust_runtime::RuntimeConfig;
use rust_hotpath::pg::DbRepo;
use support::{conninfo_from_config, unique_suffix, MountedFs};

fn with_mount_log<T>(mounted: &MountedFs, result: Result<T, String>) -> Result<T, String> {
    result.map_err(|err| {
        format!(
            "{err}\nFOD mount log (last 400 lines):\n{}",
            mounted.log_tail(400)
        )
    })
}

fn direct_write_lane_create_preflight() -> Result<(), String> {
    let mut values = HashMap::new();
    values.insert("pool_max_connections".to_string(), "6".to_string());
    let runtime = RuntimeConfig::from_runtime_map(&values)
        .map_err(|err| format!("direct write-lane runtime config failed: {err}"))?;
    let conninfo = conninfo_from_config()
        .map_err(|err| format!("direct write-lane conninfo failed: {err}"))?;
    let repo = DbRepo::with_runtime(&conninfo, &runtime)
        .map_err(|err| format!("direct write-lane repo creation failed: {err}"))?;

    let suffix = unique_suffix();
    let directory_name = format!("pg-lanes-direct-{suffix}");
    let directory_path = format!("/{directory_name}");
    let file_path = format!("{directory_path}/source.txt");
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };

    let directory_id = repo
        .create_directory(None, &directory_name, 0o775, uid, gid, &directory_path)
        .map_err(|err| format!("direct write-lane create_directory failed: {err}"))?;
    repo.create_file(
        Some(directory_id),
        "source.txt",
        0o100664,
        uid,
        gid,
        &file_path,
    )
    .map_err(|err| format!("direct write-lane create_file failed: {err}"))?;

    Ok(())
}

fn wait_for_lane_diagnostics(mounted: &MountedFs) -> Result<String, String> {
    let required = [
        "FOD PostgreSQL lanes: opt_in_enabled=true dedicated_lanes_active=true mode=dedicated-lanes",
        "legacy_dsn_only=true routing_enabled=false",
        "FOD reading startup snapshot through control lane",
        "FOD PostgreSQL non-write lane keepalive count=3",
    ];

    for _ in 0..50 {
        let log = mounted.log_tail(400);
        if required.iter().all(|needle| log.contains(needle)) {
            return Ok(log);
        }
        sleep(Duration::from_millis(100));
    }

    Err(format!(
        "missing expected PostgreSQL lane diagnostics\n{}",
        mounted.log_tail(400)
    ))
}

#[test]
fn opt_in_pg_lanes_mount_and_serve_basic_filesystem_operations() -> Result<(), String> {
    direct_write_lane_create_preflight()?;

    let mounted = MountedFs::start_with_env(
        "pg-lanes-opt-in",
        &[
            ("FOD_PG_POOL_LANES_ENABLED", "1".to_string()),
            ("FOD_POOL_MAX_CONNECTIONS", "10".to_string()),
            ("FOD_LOG_LEVEL", "debug".to_string()),
        ],
    )?;

    wait_for_lane_diagnostics(&mounted)?;

    let directory = mounted
        .mountpoint
        .join(format!("pg-lanes-smoke-{}", unique_suffix()));
    let source = directory.join("source.txt");
    let renamed = directory.join("renamed.txt");
    let payload = b"FOD PostgreSQL lane mounted smoke\n";

    with_mount_log(
        &mounted,
        fs::create_dir_all(&directory)
            .map_err(|err| format!("create_dir_all {} failed: {err}", directory.display())),
    )?;
    with_mount_log(
        &mounted,
        fs::write(&source, payload)
            .map_err(|err| format!("write {} failed: {err}", source.display())),
    )?;

    let observed = with_mount_log(
        &mounted,
        fs::read(&source).map_err(|err| format!("read {} failed: {err}", source.display())),
    )?;
    if observed != payload {
        return Err(format!(
            "mounted PostgreSQL lane smoke payload mismatch\nFOD mount log (last 400 lines):\n{}",
            mounted.log_tail(400)
        ));
    }

    with_mount_log(
        &mounted,
        fs::rename(&source, &renamed).map_err(|err| {
            format!(
                "rename {} to {} failed: {err}",
                source.display(),
                renamed.display()
            )
        }),
    )?;

    let metadata = with_mount_log(
        &mounted,
        fs::metadata(&renamed)
            .map_err(|err| format!("metadata {} failed: {err}", renamed.display())),
    )?;
    if metadata.len() != payload.len() as u64 {
        return Err(format!(
            "unexpected renamed payload length: expected={} observed={}\nFOD mount log (last 400 lines):\n{}",
            payload.len(),
            metadata.len(),
            mounted.log_tail(400)
        ));
    }

    with_mount_log(
        &mounted,
        fs::remove_file(&renamed)
            .map_err(|err| format!("remove_file {} failed: {err}", renamed.display())),
    )?;
    with_mount_log(
        &mounted,
        fs::remove_dir(&directory)
            .map_err(|err| format!("remove_dir {} failed: {err}", directory.display())),
    )?;

    Ok(())
}
