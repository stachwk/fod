// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
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

fn runtime_with_pool_limit(limit: u64) -> Result<RuntimeConfig, String> {
    let mut values = HashMap::new();
    values.insert("pool_max_connections".to_string(), limit.to_string());
    RuntimeConfig::from_runtime_map(&values)
        .map_err(|err| format!("direct write-lane runtime config failed: {err}"))
}

fn direct_write_lane_repo() -> Result<DbRepo, String> {
    let runtime = runtime_with_pool_limit(6)?;
    let conninfo = conninfo_from_config()
        .map_err(|err| format!("direct write-lane conninfo failed: {err}"))?;
    DbRepo::with_runtime(&conninfo, &runtime)
        .map_err(|err| format!("direct write-lane repo creation failed: {err}"))
}

fn direct_write_lane_create_preflight() -> Result<(), String> {
    let repo = direct_write_lane_repo()?;
    let suffix = unique_suffix();
    let directory_name = format!("pg-lanes-direct-{suffix}");
    let directory_path = format!("/{directory_name}");
    let file_path = format!("{directory_path}/source.txt");
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };

    let directory_id = repo
        .create_directory(None, &directory_name, 0o775, uid, gid, &directory_path)
        .map_err(|err| format!("direct write-lane create_directory failed: {err}"))?;
    let file_id = repo
        .create_file(
            Some(directory_id),
            "source.txt",
            0o100664,
            uid,
            gid,
            &file_path,
        )
        .map_err(|err| format!("direct write-lane create_file failed: {err}"))?;

    repo.purge_primary_file(file_id)
        .map_err(|err| format!("direct write-lane cleanup file failed: {err}"))?;
    repo.delete_directory_entry(directory_id)
        .map_err(|err| format!("direct write-lane cleanup directory failed: {err}"))?;

    Ok(())
}

fn failed_create_database_diagnostics(directory_path: &str, file_path: &str) -> String {
    let result = (|| -> Result<String, String> {
        let repo = direct_write_lane_repo()?;
        let directory = repo
            .resolve_path(directory_path)
            .map_err(|err| format!("resolve directory failed: {err}"))?;
        let file = repo
            .resolve_path(file_path)
            .map_err(|err| format!("resolve file failed: {err}"))?;
        let attrs_blob = repo
            .fetch_path_attrs_blob(file_path)
            .map_err(|err| format!("fetch file attrs failed: {err}"))?;
        let attrs_len = attrs_blob.as_ref().map(Vec::len);
        let attrs_fields = attrs_blob.as_ref().map(|blob| {
            blob.split(|byte| *byte == 0)
                .map(|field| String::from_utf8_lossy(field).to_string())
                .collect::<Vec<_>>()
        });
        let (file_link_count, special_metadata) = match file.entry_id {
            Some(file_id) => {
                let links = repo
                    .count_file_links(file_id)
                    .map(|count| format!("ok count={count}"))
                    .unwrap_or_else(|err| format!("error={err}"));
                let special = repo
                    .get_special_file_metadata(file_id)
                    .map(|value| format!("ok value={value:?}"))
                    .unwrap_or_else(|err| format!("error={err}"));
                (links, special)
            }
            None => (
                "skipped: missing file id".to_string(),
                "skipped: missing file id".to_string(),
            ),
        };

        let uid = unsafe { libc::geteuid() };
        let gid = unsafe { libc::getegid() };
        let probe_name = format!("direct-after-fuse-{}.txt", unique_suffix());
        let probe_path = format!("{directory_path}/{probe_name}");
        let direct_create = match directory.entry_id {
            Some(directory_id) => match repo.create_file(
                Some(directory_id),
                &probe_name,
                0o100664,
                uid,
                gid,
                &probe_path,
            ) {
                Ok(file_id) => {
                    let cleanup = repo
                        .purge_primary_file(file_id)
                        .map(|_| "ok".to_string())
                        .unwrap_or_else(|err| format!("error={err}"));
                    format!("ok file_id={file_id} cleanup={cleanup}")
                }
                Err(err) => format!("error={err}"),
            },
            None => "skipped: missing directory id".to_string(),
        };

        Ok(format!(
            "directory={directory:?}\nfile={file:?}\nfile_attrs_blob_len={attrs_len:?}\nfile_attrs_fields={attrs_fields:?}\nfile_link_count={file_link_count}\nspecial_metadata={special_metadata}\ndirect_create_after_fuse={direct_create}"
        ))
    })();

    result.unwrap_or_else(|err| format!("database diagnostics failed: {err}"))
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

    let directory_name = format!("pg-lanes-smoke-{}", unique_suffix());
    let directory_logical_path = format!("/{directory_name}");
    let source_logical_path = format!("{directory_logical_path}/source.txt");
    let directory = mounted.mountpoint.join(&directory_name);
    let source = directory.join("source.txt");
    let renamed = directory.join("renamed.txt");
    let payload = b"FOD PostgreSQL lane mounted smoke\n";

    with_mount_log(
        &mounted,
        fs::create_dir_all(&directory)
            .map_err(|err| format!("create_dir_all {} failed: {err}", directory.display())),
    )?;

    let mut created = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&source)
    {
        Ok(file) => file,
        Err(err) => {
            return Err(format!(
                "create {} failed: {err}\nPostgreSQL state after failed mounted create:\n{}\nFOD mount log (last 400 lines):\n{}",
                source.display(),
                failed_create_database_diagnostics(
                    &directory_logical_path,
                    &source_logical_path
                ),
                mounted.log_tail(400)
            ));
        }
    };

    if let Err(err) = created.write_all(payload) {
        return Err(format!(
            "write_all {} failed after successful create: {err}\nPostgreSQL state after failed mounted write:\n{}\nFOD mount log (last 400 lines):\n{}",
            source.display(),
            failed_create_database_diagnostics(
                &directory_logical_path,
                &source_logical_path
            ),
            mounted.log_tail(400)
        ));
    }

    if let Err(err) = created.sync_all() {
        return Err(format!(
            "sync_all {} failed after successful create and write: {err}\nPostgreSQL state after failed mounted fsync:\n{}\nFOD mount log (last 400 lines):\n{}",
            source.display(),
            failed_create_database_diagnostics(
                &directory_logical_path,
                &source_logical_path
            ),
            mounted.log_tail(400)
        ));
    }
    drop(created);

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
