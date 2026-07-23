// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::fs;
use std::thread::sleep;
use std::time::Duration;

use support::{unique_suffix, MountedFs};

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
    let mounted = MountedFs::start_with_env(
        "pg-lanes-opt-in",
        &[
            ("FOD_PG_POOL_LANES_ENABLED", "1".to_string()),
            ("FOD_POOL_MAX_CONNECTIONS", "10".to_string()),
            ("FOD_LOG_LEVEL", "info".to_string()),
        ],
    )?;

    let directory = mounted
        .mountpoint
        .join(format!("pg-lanes-smoke-{}", unique_suffix()));
    let source = directory.join("source.txt");
    let renamed = directory.join("renamed.txt");
    let payload = b"FOD PostgreSQL lane mounted smoke\n";

    fs::create_dir_all(&directory)
        .map_err(|err| format!("create_dir_all {} failed: {err}", directory.display()))?;
    fs::write(&source, payload)
        .map_err(|err| format!("write {} failed: {err}", source.display()))?;

    let observed = fs::read(&source)
        .map_err(|err| format!("read {} failed: {err}", source.display()))?;
    if observed != payload {
        return Err("mounted PostgreSQL lane smoke payload mismatch".to_string());
    }

    fs::rename(&source, &renamed).map_err(|err| {
        format!(
            "rename {} to {} failed: {err}",
            source.display(),
            renamed.display()
        )
    })?;

    let metadata = fs::metadata(&renamed)
        .map_err(|err| format!("metadata {} failed: {err}", renamed.display()))?;
    if metadata.len() != payload.len() as u64 {
        return Err(format!(
            "unexpected renamed payload length: expected={} observed={}",
            payload.len(),
            metadata.len()
        ));
    }

    let _log = wait_for_lane_diagnostics(&mounted)?;

    fs::remove_file(&renamed)
        .map_err(|err| format!("remove_file {} failed: {err}", renamed.display()))?;
    fs::remove_dir(&directory)
        .map_err(|err| format!("remove_dir {} failed: {err}", directory.display()))?;

    Ok(())
}
