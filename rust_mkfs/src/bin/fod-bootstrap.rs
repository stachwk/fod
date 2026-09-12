// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

#[path = "../config.rs"]
mod config;
#[path = "../pg_config.rs"]
mod pg_config;
#[path = "../version.rs"]
mod version;

use clap::{ArgAction, Parser};
use config::{
    apply_startup_passthrough_env, load_config_parser, load_runtime_config, resolve_config_path,
};
use fod_rust_runtime::{
    env_var_truthy_with_legacy_alias, env_var_with_legacy_alias, BootstrapOverrides,
};
use pg_config::{make_conninfo, resolve_pg_connection_params};
use std::env;
use std::ffi::CString;
use std::fs;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "fod-bootstrap", version = version::FOD_VERSION_LABEL, about = "Mount FOD through the Rust FUSE frontend.")]
struct Cli {
    #[arg(short = 'f', long = "mountpoint")]
    mountpoint: String,
    #[arg(long = "config", value_name = "INI")]
    config: PathBuf,
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
    #[arg(long = "no-default-permissions", action = ArgAction::SetTrue)]
    no_default_permissions: bool,
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

fn configured_rust_fuse_binary(
    configured: Option<std::ffi::OsString>,
) -> Result<Option<PathBuf>, String> {
    let Some(configured) = configured else {
        return Ok(None);
    };
    let candidate = PathBuf::from(configured);
    if is_executable_file(&candidate) {
        Ok(Some(candidate))
    } else {
        Err(format!(
            "FOD_RUST_FUSE_BIN points to a missing or non-executable file: {}",
            candidate.display()
        ))
    }
}

fn sibling_rust_fuse_binary(current_exe: &Path) -> Option<PathBuf> {
    current_exe
        .parent()
        .map(|dir| dir.join("fod-rust-fuse"))
        .filter(|candidate| is_executable_file(candidate))
}

fn rust_fuse_binary() -> Option<PathBuf> {
    if let Ok(current_exe) = env::current_exe() {
        if let Some(candidate) = sibling_rust_fuse_binary(&current_exe) {
            return Some(candidate);
        }
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let candidates = [
        root.join("target/release-lto/fod-rust-fuse"),
        root.join("rust_fuse/target/release-lto/fod-rust-fuse"),
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

const FUSE_HANG_GUARD_DEFAULT_TIMEOUT_SECONDS: u64 = 15;
const FUSE_HANG_GUARD_DEFAULT_INTERVAL_SECONDS: u64 = 5;
const FUSE_HANG_GUARD_POLL_INTERVAL: Duration = Duration::from_millis(100);
const FUSE_CONTROL_ROOT: &str = "/sys/fs/fuse/connections";

fn fuse_hang_guard_config() -> Option<(Duration, Duration)> {
    let timeout_seconds = env::var("FOD_FUSE_HANG_GUARD_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(FUSE_HANG_GUARD_DEFAULT_TIMEOUT_SECONDS);
    if timeout_seconds == 0 {
        return None;
    }

    let interval_seconds = env::var("FOD_FUSE_HANG_GUARD_INTERVAL_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(FUSE_HANG_GUARD_DEFAULT_INTERVAL_SECONDS)
        .max(1)
        .min(timeout_seconds.max(1));

    Some((
        Duration::from_secs(timeout_seconds),
        Duration::from_secs(interval_seconds),
    ))
}

fn decode_mountinfo_path(value: &str) -> String {
    value
        .replace(r"\040", " ")
        .replace(r"\011", "\t")
        .replace(r"\012", "\n")
        .replace(r"\134", "\\")
}

fn encoded_dev_from_mountinfo(contents: &str, mountpoint: &Path) -> Option<u64> {
    let wanted = mountpoint.to_string_lossy();
    for line in contents.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 7 {
            continue;
        }
        let Some(separator) = fields.iter().position(|field| *field == "-") else {
            continue;
        };
        if separator + 1 >= fields.len() {
            continue;
        }
        if !fields[separator + 1].starts_with("fuse") {
            continue;
        }
        if decode_mountinfo_path(fields[4]) != wanted {
            continue;
        }

        let (major_text, minor_text) = fields[2].split_once(':')?;
        let major = major_text.parse::<u32>().ok()?;
        let minor = minor_text.parse::<u32>().ok()?;
        let dev = libc::makedev(major, minor);
        return Some(dev as u64);
    }
    None
}

fn fuse_connection_id_for_mountpoint(mountpoint: &Path) -> Option<u64> {
    let contents = fs::read_to_string("/proc/self/mountinfo").ok()?;
    let connection_id = encoded_dev_from_mountinfo(&contents, mountpoint)?;
    let control_dir = Path::new(FUSE_CONTROL_ROOT).join(connection_id.to_string());
    control_dir.is_dir().then_some(connection_id)
}

fn fuse_connection_waiting(connection_id: u64) -> Option<u64> {
    fs::read_to_string(
        Path::new(FUSE_CONTROL_ROOT)
            .join(connection_id.to_string())
            .join("waiting"),
    )
    .ok()?
    .trim()
    .parse::<u64>()
    .ok()
}

fn abort_fuse_connection(connection_id: u64) -> Result<(), String> {
    let abort_path = Path::new(FUSE_CONTROL_ROOT)
        .join(connection_id.to_string())
        .join("abort");
    fs::write(&abort_path, b"1\n").map_err(|err| {
        format!(
            "Cannot abort FUSE connection {} through {}: {}",
            connection_id,
            abort_path.display(),
            err
        )
    })
}

fn detach_aborted_fuse_mount(mountpoint: &Path) -> Result<(), String> {
    let mut attempts = Vec::new();

    for (binary, args) in [
        ("fusermount3", vec!["-uz"]),
        ("fusermount", vec!["-uz"]),
        ("umount", vec!["-l"]),
    ] {
        let Some(path) = find_in_path(binary) else {
            attempts.push(format!("{binary}:not-found"));
            continue;
        };

        match Command::new(&path).args(&args).arg(mountpoint).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => attempts.push(format!(
                "{}:{}",
                path.display(),
                status.code().unwrap_or(-1)
            )),
            Err(err) => attempts.push(format!("{}:{err}", path.display())),
        }
    }

    Err(format!(
        "Cannot detach aborted FUSE mount {} ({})",
        mountpoint.display(),
        attempts.join(", ")
    ))
}

#[cfg(unix)]
fn spawn_statfs_probe(mountpoint: PathBuf) -> Receiver<Result<(), String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let path = CString::new(mountpoint.as_os_str().as_bytes()).map_err(|_| {
                format!(
                    "Mountpoint contains an embedded NUL byte: {}",
                    mountpoint.display()
                )
            })?;
            let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
            let rc = unsafe { libc::statvfs(path.as_ptr(), &mut stat) };
            if rc == 0 {
                Ok(())
            } else {
                Err(format!(
                    "statvfs({}) failed: {}",
                    mountpoint.display(),
                    std::io::Error::last_os_error()
                ))
            }
        })();
        let _ = sender.send(result);
    });
    receiver
}

#[cfg(not(unix))]
fn spawn_statfs_probe(_mountpoint: PathBuf) -> Receiver<Result<(), String>> {
    let (sender, receiver) = mpsc::channel();
    let _ = sender.send(Ok(()));
    receiver
}

fn run_fuse_with_hang_guard(
    command: &mut Command,
    mountpoint: &Path,
) -> Result<ExitStatus, String> {
    let mut child = command
        .spawn()
        .map_err(|err| format!("Failed to launch Rust FUSE frontend: {err}"))?;

    let Some((probe_timeout, probe_interval)) = fuse_hang_guard_config() else {
        return child
            .wait()
            .map_err(|err| format!("Failed waiting for Rust FUSE frontend: {err}"));
    };

    let mountpoint = fs::canonicalize(mountpoint).unwrap_or_else(|_| mountpoint.to_path_buf());
    let discovery_started = Instant::now();
    let mut connection_id: Option<u64> = None;
    let mut discovery_warning_emitted = false;
    let mut next_probe_at = Instant::now();
    let mut active_probe: Option<(Receiver<Result<(), String>>, Instant)> = None;

    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|err| format!("Failed checking Rust FUSE frontend status: {err}"))?
        {
            return Ok(status);
        }

        if connection_id.is_none() {
            connection_id = fuse_connection_id_for_mountpoint(&mountpoint);
            if connection_id.is_none()
                && !discovery_warning_emitted
                && discovery_started.elapsed() >= Duration::from_secs(10)
            {
                eprintln!(
                    "WARNING: FOD FUSE hang guard cannot resolve fusectl connection for {}; \
                     df/statfs hang protection is unavailable for this mount",
                    mountpoint.display()
                );
                discovery_warning_emitted = true;
            }
        }

        if let Some((receiver, probe_started)) = active_probe.as_ref() {
            match receiver.try_recv() {
                Ok(Ok(())) => {
                    active_probe = None;
                    next_probe_at = Instant::now() + probe_interval;
                }
                Ok(Err(err)) => {
                    eprintln!("WARNING: FOD FUSE hang guard statfs probe returned: {err}");
                    active_probe = None;
                    next_probe_at = Instant::now() + probe_interval;
                }
                Err(TryRecvError::Disconnected) => {
                    active_probe = None;
                    next_probe_at = Instant::now() + probe_interval;
                }
                Err(TryRecvError::Empty) if probe_started.elapsed() >= probe_timeout => {
                    let waiting = connection_id.and_then(fuse_connection_waiting);
                    eprintln!(
                        "ERROR: FOD FUSE hang guard timeout for {} after {:.3}s; \
                         connection={:?} waiting={:?}; aborting mount",
                        mountpoint.display(),
                        probe_started.elapsed().as_secs_f64(),
                        connection_id,
                        waiting
                    );

                    if let Some(connection_id) = connection_id {
                        if let Err(err) = abort_fuse_connection(connection_id) {
                            eprintln!("WARNING: {err}");
                        }
                    } else {
                        eprintln!(
                            "WARNING: fusectl connection is unknown; falling back to killing FUSE frontend"
                        );
                    }

                    if let Err(err) = child.kill() {
                        eprintln!("WARNING: failed to kill hung Rust FUSE frontend: {err}");
                    }

                    let status = child.wait().map_err(|err| {
                        format!("Failed waiting for aborted Rust FUSE frontend: {err}")
                    })?;

                    if let Err(err) = detach_aborted_fuse_mount(&mountpoint) {
                        eprintln!("WARNING: {err}");
                    }

                    return Ok(status);
                }
                Err(TryRecvError::Empty) => {}
            }
        } else if connection_id.is_some() && Instant::now() >= next_probe_at {
            active_probe = Some((spawn_statfs_probe(mountpoint.clone()), Instant::now()));
        }

        thread::sleep(FUSE_HANG_GUARD_POLL_INTERVAL);
    }
}

const PG_ENDPOINT_ENV_KEYS: &[(&str, &str)] = &[
    ("primary_hosts", "FOD_PG_PRIMARY_HOSTS"),
    ("replica_hosts", "FOD_PG_REPLICA_HOSTS"),
    ("hosts", "FOD_PG_HOSTS"),
];

fn configured_pg_endpoint_env(
    db_section: &std::collections::HashMap<String, String>,
) -> Vec<(&'static str, String)> {
    PG_ENDPOINT_ENV_KEYS
        .iter()
        .filter_map(|(key, env_name)| {
            db_section
                .get(*key)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(|value| (*env_name, value.to_string()))
        })
        .collect()
}

fn apply_pg_endpoint_env(db_section: &std::collections::HashMap<String, String>) {
    for (env_name, value) in configured_pg_endpoint_env(db_section) {
        if env::var_os(env_name).is_none() {
            env::set_var(env_name, value);
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let rust_fuse = match configured_rust_fuse_binary(env::var_os("FOD_RUST_FUSE_BIN")) {
        Ok(Some(path)) => path,
        Ok(None) => match rust_fuse_binary() {
            Some(path) => path,
            None => {
                eprintln!(
                    "Rust FUSE binary is unavailable; build target/release-lto/fod-rust-fuse, set FOD_RUST_FUSE_BIN, install fod-rust-fuse on PATH, or place it in /usr/local/bin."
                );
                std::process::exit(1);
            }
        },
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    if let Some(profile) = &cli.profile {
        env::set_var("FOD_PROFILE", profile);
    }
    let env_log_level =
        env_var_with_legacy_alias("FOD_LOG_LEVEL").filter(|value| !value.trim().is_empty());
    let env_debug = env_var_truthy_with_legacy_alias("FOD_DEBUG", false);
    // Mount configuration is deliberately explicit. Override any inherited
    // FOD_CONFIG selector with the CLI-selected INI before resolving it.
    env::set_var("FOD_CONFIG", &cli.config);
    let config_path = match resolve_config_path(Some(&cli.config)) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    env::set_var("FOD_CONFIG", &config_path);
    let (config, config_dir) = match load_config_parser(Some(&config_path)) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    if let Err(err) = apply_startup_passthrough_env(&config) {
        eprintln!("{}", err);
        std::process::exit(1);
    }
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
    let default_permissions = cli.default_permissions && !cli.no_default_permissions;
    let bootstrap_runtime = match runtime.with_bootstrap_overrides(&BootstrapOverrides {
        profile: cli.profile.clone(),
        role: cli.role.clone(),
        selinux: cli.selinux.clone(),
        acl: cli.acl.clone(),
        atime_policy: cli.atime_policy.clone(),
        default_permissions,
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
    apply_pg_endpoint_env(&db_section);
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
    let status = run_fuse_with_hang_guard(&mut command, &mountpoint);
    match status {
        Ok(status) if status.success() => std::process::exit(0),
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(err) => {
            eprintln!("{}", err);
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
    fn decodes_mountinfo_escaped_mountpoint() {
        assert_eq!(
            decode_mountinfo_path(r"/tmp/fod\040with\040space"),
            "/tmp/fod with space"
        );
        assert_eq!(decode_mountinfo_path(r"/tmp/fod\134name"), r"/tmp/fod\name");
    }

    #[test]
    fn maps_fuse_mountinfo_device_to_fusectl_id() {
        let mountpoint = Path::new("/tmp/fod-test");
        let contents =
            "101 42 0:168 / /tmp/fod-test rw,nosuid,nodev - fuse.fod fod rw,user_id=1000\n";
        assert_eq!(
            encoded_dev_from_mountinfo(contents, mountpoint),
            Some(libc::makedev(0, 168) as u64)
        );
    }

    #[test]
    fn extracts_endpoint_lists_for_fuse_environment() {
        let db = std::collections::HashMap::from([
            (
                "primary_hosts".to_string(),
                "db-a:5432,db-b:5432".to_string(),
            ),
            ("replica_hosts".to_string(), "db-r:5432".to_string()),
            ("host".to_string(), "legacy".to_string()),
        ]);
        assert_eq!(
            configured_pg_endpoint_env(&db),
            vec![
                ("FOD_PG_PRIMARY_HOSTS", "db-a:5432,db-b:5432".to_string()),
                ("FOD_PG_REPLICA_HOSTS", "db-r:5432".to_string()),
            ]
        );
    }

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

    #[test]
    fn prefers_fuse_binary_sibling_of_selected_bootstrap() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let base = env::temp_dir().join(format!("fod-bootstrap-sibling-{unique}"));
        let profile_dir = base.join("release-lto");
        let bootstrap = profile_dir.join("fod-bootstrap");
        let fuse = profile_dir.join("fod-rust-fuse");
        fs::create_dir_all(&profile_dir).expect("create profile dir");
        fs::write(&bootstrap, b"#!/bin/sh\n").expect("write bootstrap");
        fs::write(&fuse, b"#!/bin/sh\n").expect("write fuse");
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&fuse).expect("stat fuse").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&fuse, perms).expect("chmod fuse");
        }

        assert_eq!(sibling_rust_fuse_binary(&bootstrap), Some(fuse.clone()));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn validates_explicit_rust_fuse_binary_override() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let base = env::temp_dir().join(format!("fod-bootstrap-explicit-{unique}"));
        let candidate = base.join("fod-rust-fuse");
        fs::create_dir_all(&base).expect("create temp dir");
        fs::write(&candidate, b"#!/bin/sh\n").expect("write candidate");
        #[cfg(unix)]
        let mut perms = fs::metadata(&candidate)
            .expect("stat candidate")
            .permissions();
        #[cfg(unix)]
        perms.set_mode(0o755);
        #[cfg(unix)]
        fs::set_permissions(&candidate, perms).expect("chmod candidate");

        let selected = configured_rust_fuse_binary(Some(candidate.clone().into_os_string()))
            .expect("valid explicit binary")
            .expect("explicit binary selected");
        assert_eq!(selected, candidate);

        let missing = base.join("missing-fod-rust-fuse");
        let err = configured_rust_fuse_binary(Some(missing.into_os_string()))
            .expect_err("missing explicit binary must fail");
        assert!(err.contains("FOD_RUST_FUSE_BIN"));

        let _ = fs::remove_file(&candidate);
        let _ = fs::remove_dir_all(&base);
    }
}
