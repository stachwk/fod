// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

#![allow(dead_code)]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fod_rust_runtime::{make_conninfo, resolve_pg_connection_params};
use rust_hotpath::pg::DbRepo;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rust_fuse lives inside the repo root")
        .to_path_buf()
}

pub fn config_path() -> PathBuf {
    repo_root().join("fod_config.ini")
}

pub fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn binary_from_env_or_candidates(
    env_var: &str,
    candidates: &[PathBuf],
    error_message: &str,
) -> PathBuf {
    if let Ok(path) = env::var(env_var) {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return candidate;
        }
    }
    for candidate in candidates {
        if candidate.is_file() {
            return candidate.clone();
        }
    }
    panic!("{}", error_message);
}

pub fn create_workspace(name: &str) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let base = env::temp_dir().join(format!("fod-rust-fuse-{}-{name}", unique_suffix()));
    let mountpoint = base.join("mount");
    let log_path = base.join("mount.log");
    fs::create_dir_all(&mountpoint).map_err(|err| err.to_string())?;
    Ok((base, mountpoint, log_path))
}

pub fn parse_database_section(config_text: &str) -> Result<Vec<(String, String)>, String> {
    let mut in_database = false;
    let mut pairs = Vec::new();

    for raw_line in config_text.lines() {
        let line = raw_line.split(['#', ';']).next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_database = line[1..line.len() - 1]
                .trim()
                .eq_ignore_ascii_case("database");
            continue;
        }
        if !in_database {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            pairs.push((key.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }

    if pairs.is_empty() {
        return Err("missing [database] section in fod_config.ini".to_string());
    }
    Ok(pairs)
}

pub fn conninfo_from_config() -> Result<String, String> {
    let config_path = config_path();
    let config = fs::read_to_string(&config_path)
        .map_err(|err| format!("failed to read fod_config.ini: {err}"))?;
    let db_config = parse_database_section(&config)?
        .into_iter()
        .collect::<HashMap<_, _>>();
    let params =
        resolve_pg_connection_params(&db_config, config_path.parent().unwrap_or(Path::new(".")));
    Ok(make_conninfo(&params))
}

pub fn db_repo() -> Result<DbRepo, String> {
    DbRepo::new(&conninfo_from_config()?)
}

pub fn admp_trace_env_pairs() -> Result<Vec<(String, String)>, String> {
    let raw = match env::var("ADMP_TRACE_ENV") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return Ok(Vec::new()),
    };

    let mut pairs = Vec::new();
    for entry in raw.split_whitespace() {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("invalid ADMP_TRACE_ENV entry: {entry}"))?;
        if key.is_empty() {
            return Err("invalid ADMP_TRACE_ENV entry: empty key".to_string());
        }
        pairs.push((key.to_string(), value.to_string()));
    }
    Ok(pairs)
}

pub fn block_size_from_config() -> Result<usize, String> {
    let repo = db_repo()?;
    let snapshot = repo.startup_snapshot()?;
    Ok(snapshot.block_size.unwrap_or(4096) as usize)
}

pub fn ensure_schema_initialized() -> Result<(), String> {
    let config = config_path();
    let conninfo = conninfo_from_config().map_err(|err| format!("conninfo_from_config: {err}"))?;
    let repo = DbRepo::new(&conninfo).map_err(|err| format!("DbRepo::new: {err}"))?;
    let snapshot = repo
        .startup_snapshot()
        .map_err(|err| format!("startup_snapshot: {err}"))?;
    if snapshot.schema_is_initialized && snapshot.schema_version.is_some() {
        return Ok(());
    }

    let root = repo_root();
    let schema_password = env::var("FOD_SCHEMA_ADMIN_PASSWORD")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("fod-{}", unique_suffix().replace('-', "")));
    let mkfs = mkfs_binary();

    let mut command = Command::new(&mkfs);
    command
        .current_dir(&root)
        .arg("init")
        .arg("--schema-admin-password")
        .arg(&schema_password)
        .env("FOD_CONFIG", &config)
        .env(
            "POSTGRES_DB",
            env::var("POSTGRES_DB").unwrap_or_else(|_| "foddbname".to_string()),
        )
        .env(
            "POSTGRES_USER",
            env::var("POSTGRES_USER").unwrap_or_else(|_| "foduser".to_string()),
        )
        .env(
            "POSTGRES_PASSWORD",
            env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "cichosza".to_string()),
        );
    apply_admp_trace_env(&mut command)?;
    let output = command
        .output()
        .map_err(|err| format!("failed to initialize schema: {err}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "fod-rust-mkfs init failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn apply_admp_trace_env(command: &mut Command) -> Result<(), String> {
    for (key, value) in admp_trace_env_pairs()? {
        command.env(key, value);
    }
    Ok(())
}

fn logical_mount_path(mountpoint: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(mountpoint).map_err(|_| {
        format!(
            "path {} is not under mountpoint {}",
            path.display(),
            mountpoint.display()
        )
    })?;
    let mut logical = String::from("/");
    logical.push_str(&relative.to_string_lossy().replace('\\', "/"));
    Ok(logical.trim_end_matches('/').to_string())
}

pub fn resolve_file_id(repo: &DbRepo, mountpoint: &Path, path: &Path) -> Result<u64, String> {
    let logical_path = logical_mount_path(mountpoint, path)?;
    let resolved = repo.resolve_path(&logical_path)?;
    match (resolved.kind.as_deref(), resolved.entry_id) {
        (Some("hardlink"), Some(hardlink_id)) => repo
            .get_hardlink_file_id(hardlink_id)?
            .ok_or_else(|| "missing file id".to_string()),
        (Some("file"), Some(file_id)) => Ok(file_id),
        other => Err(format!("unexpected resolved path: {:?}", other)),
    }
}

pub fn bootstrap_binary() -> PathBuf {
    let root = repo_root();
    binary_from_env_or_candidates(
        "FOD_BOOTSTRAP_BIN",
        &[
            root.join("target/debug/fod-bootstrap"),
            root.join("target/release/fod-bootstrap"),
            root.join("rust_mkfs/target/debug/fod-bootstrap"),
            root.join("rust_mkfs/target/release/fod-bootstrap"),
            PathBuf::from("/usr/local/bin/fod-bootstrap"),
        ],
        "fod-bootstrap binary not found; build the workspace first",
    )
}

pub fn mkfs_binary() -> PathBuf {
    let root = repo_root();
    binary_from_env_or_candidates(
        "FOD_MKFS_BIN",
        &[
            root.join("target/debug/fod-rust-mkfs"),
            root.join("target/release/fod-rust-mkfs"),
            root.join("rust_mkfs/target/debug/fod-rust-mkfs"),
            root.join("rust_mkfs/target/release/fod-rust-mkfs"),
            PathBuf::from("/usr/local/bin/fod-rust-mkfs"),
        ],
        "fod-rust-mkfs binary not found; build the workspace first",
    )
}

pub fn mountpoint_ready(path: &Path) -> bool {
    Command::new("mountpoint")
        .arg("-q")
        .arg(path)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn try_unmount(path: &Path) {
    for program in ["fusermount3", "fusermount", "umount"] {
        let mut command = Command::new(program);
        if program == "umount" {
            command.arg(path);
        } else {
            command.arg("-u").arg(path);
        }
        let _ = command.status();
        if !mountpoint_ready(path) {
            break;
        }
    }
}

pub fn tail_log(path: &Path, max_lines: usize) -> String {
    let contents = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = contents.lines().collect();
    if lines.len() <= max_lines {
        return contents;
    }
    lines[lines.len() - max_lines..].join("\n")
}

fn env_var_truthy(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn should_print_profile_io_line(line: &str) -> bool {
    line.contains("FOD I/O profile:")
        && (line.contains("pg.copy_put_data.aggregate")
            || line.contains("pg.copy_put_end")
            || line.contains("pg.copy_get_result"))
}

fn print_profile_io_summaries_if_enabled(log_path: &Path) {
    if !env_var_truthy("FOD_PROFILE_IO") {
        return;
    }

    let Ok(contents) = fs::read_to_string(log_path) else {
        return;
    };

    for line in contents
        .lines()
        .filter(|line| should_print_profile_io_line(line))
    {
        eprintln!("{line}");
    }
}

pub struct MountedFs {
    pub workspace: PathBuf,
    pub mountpoint: PathBuf,
    pub log_path: PathBuf,
    child: Child,
}

impl MountedFs {
    pub fn start(name: &str) -> Result<Self, String> {
        Self::start_without_init(name)
    }

    pub fn start_without_init(name: &str) -> Result<Self, String> {
        Self::start_with_role_noinit(name, "auto", &[])
    }

    pub fn start_with_env(name: &str, extra_env: &[(&str, String)]) -> Result<Self, String> {
        Self::start_with_role_noinit(name, "auto", extra_env)
    }

    pub fn start_with_role(
        name: &str,
        role: &str,
        extra_env: &[(&str, String)],
    ) -> Result<Self, String> {
        Self::start_with_role_noinit(name, role, extra_env)
    }

    fn start_with_role_noinit(
        name: &str,
        role: &str,
        extra_env: &[(&str, String)],
    ) -> Result<Self, String> {
        let (workspace, mountpoint, log_path) = create_workspace(name)?;
        let log_file = fs::File::create(&log_path).map_err(|err| err.to_string())?;
        let bootstrap = bootstrap_binary();
        let conninfo = conninfo_from_config()?;
        let mut command = Command::new(bootstrap);
        command
            .current_dir(repo_root())
            .arg("--debug")
            .arg("--role")
            .arg(role)
            .arg("-f")
            .arg(&mountpoint)
            .env("FOD_CONFIG", config_path())
            .env("FOD_DSN_CONNINFO", conninfo)
            .env("FOD_USE_RUST_FUSE", "1")
            .env("FOD_USE_FUSE_CONTEXT", "1")
            .env("FOD_SELINUX", "off")
            .env("FOD_ACL", "off")
            .env("FOD_DEFAULT_PERMISSIONS", "1")
            .env("FOD_ATIME_POLICY", "default");
        apply_admp_trace_env(&mut command)?;
        command
            .stdout(Stdio::from(
                log_file.try_clone().map_err(|err| err.to_string())?,
            ))
            .stderr(Stdio::from(log_file));
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let mut child = command.spawn().map_err(|err| err.to_string())?;

        for _ in 0..60 {
            if mountpoint_ready(&mountpoint) {
                return Ok(Self {
                    workspace,
                    mountpoint,
                    log_path,
                    child,
                });
            }
            if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
                return Err(format!(
                    "fod-bootstrap exited too early with status {status:?}\n{}",
                    tail_log(&log_path, 200)
                ));
            }
            sleep(Duration::from_secs(1));
        }

        Err(format!(
            "mountpoint did not become ready\n{}",
            tail_log(&log_path, 200)
        ))
    }
}

impl MountedFs {
    pub fn log_tail(&self, max_lines: usize) -> String {
        tail_log(&self.log_path, max_lines)
    }
}

impl Drop for MountedFs {
    fn drop(&mut self) {
        try_unmount(&self.mountpoint);
        let _ = self.child.kill();
        let _ = self.child.wait();
        print_profile_io_summaries_if_enabled(&self.log_path);
        let _ = fs::remove_dir_all(&self.workspace);
    }
}
