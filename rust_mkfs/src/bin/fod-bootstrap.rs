// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

#[path = "../config.rs"]
mod config;
#[path = "../pg_config.rs"]
mod pg_config;
#[path = "../version.rs"]
mod version;

use clap::Parser;
use config::{load_config_parser, load_runtime_config, resolve_config_path};
use fod_rust_runtime::{
    env_var_truthy_with_legacy_alias, env_var_with_legacy_alias, BootstrapOverrides,
};
use pg_config::{make_conninfo, resolve_pg_connection_params};
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser)]
#[command(name = "fod-bootstrap", version = version::FOD_VERSION_LABEL, about = "Mount FOD through the Rust FUSE frontend.")]
struct Cli {
    #[arg(short = 'f', long = "mountpoint")]
    mountpoint: String,
    #[arg(long, default_value = "auto")]
    role: String,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long, default_value = "off")]
    selinux: String,
    #[arg(long, default_value = "off")]
    acl: String,
    #[arg(long, default_value = "default")]
    atime_policy: String,
    #[arg(long, default_value_t = true)]
    default_permissions: bool,
    #[arg(long, default_value_t = false)]
    lazytime: bool,
    #[arg(long, default_value_t = false)]
    sync: bool,
    #[arg(long, default_value_t = false)]
    dirsync: bool,
    #[arg(long, default_value_t = false)]
    debug: bool,
    #[arg(long)]
    log_level: Option<String>,
}

fn validate_mountpoint(mountpoint: &Path) -> Result<(), String> {
    if !mountpoint.exists() {
        return Err(format!(
            "Mountpoint {} does not exist. Create an empty directory first.",
            mountpoint.display()
        ));
    }
    if !mountpoint.is_dir() {
        return Err(format!(
            "Mountpoint {} is not a directory.",
            mountpoint.display()
        ));
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(mountpoint)
        .map_err(|e| format!("Cannot inspect mountpoint {}: {}", mountpoint.display(), e))?
    {
        let entry = entry
            .map_err(|e| format!("Cannot inspect mountpoint {}: {}", mountpoint.display(), e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name != "." && name != ".." {
            entries.push(name);
        }
    }
    if !entries.is_empty() {
        let preview = entries
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if entries.len() <= 5 {
            String::new()
        } else {
            format!(" (+{} more)", entries.len() - 5)
        };
        return Err(format!(
            "Mountpoint {} is not empty ({} entries: {}{}). Please use an empty directory.",
            mountpoint.display(),
            entries.len(),
            preview,
            suffix
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
        && fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn find_in_paths<I>(binary_name: &str, search_paths: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    search_paths
        .into_iter()
        .map(|dir| dir.join(binary_name))
        .find(|candidate| is_executable_file(candidate))
}

fn find_in_path(binary_name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| find_in_paths(binary_name, env::split_paths(&path)))
}

fn rust_fuse_binary() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let candidates = [
        root.join("target/debug/fod-rust-fuse"),
        root.join("target/release/fod-rust-fuse"),
        root.join("rust_fuse/target/debug/fod-rust-fuse"),
        root.join("rust_fuse/target/release/fod-rust-fuse"),
    ];
    if let Some(candidate) = candidates
        .into_iter()
        .find(|candidate| is_executable_file(candidate))
    {
        return Some(candidate);
    }
    if let Some(candidate) = find_in_path("fod-rust-fuse") {
        return Some(candidate);
    }
    let local_install = PathBuf::from("/usr/local/bin/fod-rust-fuse");
    if is_executable_file(&local_install) {
        return Some(local_install);
    }
    None
}

fn main() {
    let cli = Cli::parse();
    let rust_fuse = match rust_fuse_binary() {
        Some(path) => path,
        None => {
            eprintln!(
                "Rust FUSE binary is unavailable; build target/debug/fod-rust-fuse first, install fod-rust-fuse on PATH, or place it in /usr/local/bin."
            );
            std::process::exit(1);
        }
    };
    if let Some(profile) = &cli.profile {
        env::set_var("FOD_PROFILE", profile);
    }
    let env_log_level =
        env_var_with_legacy_alias("FOD_LOG_LEVEL").filter(|value| !value.trim().is_empty());
    let env_debug = env_var_truthy_with_legacy_alias("FOD_DEBUG", false);
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
    let runtime = match load_runtime_config(&config) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let log_level = cli.log_level.clone().or_else(|| {
        if cli.debug || env_debug {
            Some("DEBUG".to_string())
        } else {
            env_log_level.clone()
        }
    });
    let bootstrap_runtime = match runtime.with_bootstrap_overrides(&BootstrapOverrides {
        profile: cli.profile.clone(),
        role: cli.role.clone(),
        selinux: cli.selinux.clone(),
        acl: cli.acl.clone(),
        atime_policy: cli.atime_policy.clone(),
        default_permissions: cli.default_permissions,
        lazytime: cli.lazytime,
        sync: cli.sync,
        dirsync: cli.dirsync,
        debug: cli.debug || env_debug,
        log_level,
        force_read_only: env_var_truthy_with_legacy_alias("FOD_RUST_FUSE_READONLY", false),
    }) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    bootstrap_runtime.apply_env();
    let db_section = match config.section("database") {
        Some(section) => section.clone(),
        None => {
            eprintln!("Missing [database] section in FOD configuration");
            std::process::exit(1);
        }
    };
    let params = resolve_pg_connection_params(&db_section, &config_dir);
    let conninfo = make_conninfo(&params);
    env::set_var("FOD_DSN_CONNINFO", conninfo);
    let mountpoint = PathBuf::from(&cli.mountpoint);
    if let Err(err) = validate_mountpoint(&mountpoint) {
        eprintln!("{}", err);
        std::process::exit(1);
    }
    let readonly = bootstrap_runtime.effective_read_only(false, false);
    let mut command = Command::new(&rust_fuse);
    command.arg("-f").arg(&cli.mountpoint);
    if readonly {
        command.arg("--readonly");
    }
    let status = command.status();
    match status {
        Ok(status) if status.success() => std::process::exit(0),
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(err) => {
            eprintln!("Failed to launch Rust FUSE frontend: {}", err);
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn find_in_path_prefers_first_executable_candidate() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let base = env::temp_dir().join(format!("fod-bootstrap-path-{unique}"));
        let dir = base.join("bin");
        let candidate = dir.join("fod-rust-fuse");
        fs::create_dir_all(&dir).expect("create temp dir");
        fs::write(&candidate, b"#!/bin/sh\n").expect("write candidate");
        #[cfg(unix)]
        let mut perms = fs::metadata(&candidate)
            .expect("stat candidate")
            .permissions();
        #[cfg(unix)]
        perms.set_mode(0o755);
        #[cfg(unix)]
        fs::set_permissions(&candidate, perms).expect("chmod candidate");

        let found = find_in_paths("fod-rust-fuse", [dir.clone()]).expect("find candidate");
        assert_eq!(found, candidate);

        let _ = fs::remove_file(&candidate);
        let _ = fs::remove_dir_all(&base);
    }
}
