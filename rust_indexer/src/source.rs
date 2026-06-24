use crate::cli::SourceKind;
use crate::config;
use crate::model::IndexedFile;
use fod_rust_runtime::current_hostname;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const DEFAULT_IGNORED_COMPONENTS: &[&str] = &[
    "cache",
    "caches",
    "build",
    "dist",
    "coverage",
    "node_modules",
    "target",
    "tmp",
    "temp",
    "out",
    "__pycache__",
    "downloadcachemanager",
    "platformrequestcache",
    "serverrequestcache",
    "unitycache",
];

static INDEXER_SETTINGS_READY: OnceLock<()> = OnceLock::new();

#[derive(Debug, Clone)]
struct MountInfo {
    mount_point: PathBuf,
    source: String,
}

#[derive(Debug, Clone)]
pub struct AdbBrowseRoot {
    pub serial: String,
    pub remote_root: String,
    pub local_root: PathBuf,
}

pub fn resolve_source_name(
    explicit_name: Option<&str>,
    kind: SourceKind,
    root_path: &Path,
) -> Result<String, String> {
    if let Some(name) = explicit_name {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("source name cannot be empty".to_string());
        }
        return Ok(trimmed.to_string());
    }

    if let Some(suggested) = suggested_source_name(kind, root_path) {
        return Ok(suggested);
    }

    current_hostname().map_err(|err| format!("unable to determine a default source name: {err}"))
}

pub fn is_ignored_indexed_file(file: &IndexedFile) -> bool {
    ensure_indexer_settings_loaded();
    is_ignored_relative_path(Path::new(&file.path))
}

pub fn is_ignored_index_path(_root_path: &Path, relative_path: &str) -> bool {
    ensure_indexer_settings_loaded();
    is_ignored_relative_path(Path::new(relative_path))
}

pub fn is_ignored_source_path(root_path: &Path, entry_path: &Path) -> bool {
    ensure_indexer_settings_loaded();
    if entry_path == root_path {
        return false;
    }
    match entry_path.strip_prefix(root_path) {
        Ok(relative_path) => is_ignored_relative_path(relative_path),
        Err(_) => false,
    }
}

pub fn adb_browse_root() -> Result<AdbBrowseRoot, String> {
    let serial = adb_target_serial()?;
    let remote_root = adb_shell_storage_root(&serial)?;
    let runtime_root = runtime_user_dir()?;
    let local_root = find_android_mtp_browse_root(&runtime_root, Some(&serial))?
        .or_else(|| find_android_mtp_browse_root(&runtime_root, None).ok().flatten())
        .ok_or_else(|| {
            format!(
                "adb shell detected device {serial} with storage root {remote_root}, but no local gvfs MTP mount was found under {}",
                runtime_root.join("gvfs").display()
            )
        })?;

    Ok(AdbBrowseRoot {
        serial,
        remote_root,
        local_root,
    })
}

fn suggested_source_name(kind: SourceKind, root_path: &Path) -> Option<String> {
    match kind {
        SourceKind::Local => current_hostname().ok(),
        SourceKind::Smb | SourceKind::Qnap => {
            suggested_mount_name(root_path).or_else(|| current_hostname().ok())
        }
        SourceKind::Adb => suggested_adb_name().or_else(|| current_hostname().ok()),
        SourceKind::Github => suggested_github_name(root_path).or_else(|| current_hostname().ok()),
    }
}

fn is_ignored_relative_path(relative_path: &Path) -> bool {
    let settings = config::indexer_settings();
    let normalized = normalize_path(relative_path);
    if settings
        .skip_prefixes
        .iter()
        .any(|prefix| normalized == *prefix || normalized.starts_with(&format!("{prefix}/")))
    {
        return true;
    }

    relative_path.components().any(|component| match component {
        Component::Normal(value) => is_ignored_component(&value.to_string_lossy(), settings),
        _ => false,
    })
}

fn is_ignored_component(component: &str, settings: &config::IndexerSettings) -> bool {
    if component.is_empty() || component == "." || component == ".." {
        return false;
    }
    if component.starts_with('.') && settings.skip_hidden {
        return true;
    }

    let lowered = component.to_ascii_lowercase();
    DEFAULT_IGNORED_COMPONENTS
        .iter()
        .any(|candidate| lowered == *candidate)
        || settings.skip_components.contains(&lowered)
}

fn suggested_mount_name(root_path: &Path) -> Option<String> {
    let mount = find_mount_for_path(root_path)?;
    extract_mount_host(&mount.source)
        .or_else(|| {
            let source = mount.source.trim();
            if source.contains('/') || source.contains(':') || source.contains('@') {
                sanitize_label(source)
            } else {
                None
            }
        })
        .or_else(|| {
            mount
                .mount_point
                .file_name()
                .and_then(|value| sanitize_label(&value.to_string_lossy()))
        })
}

fn suggested_adb_name() -> Option<String> {
    for key in ["ANDROID_SERIAL", "ADB_SERIAL", "ADB_DEVICE_SERIAL"] {
        if let Ok(value) = env::var(key) {
            if let Some(label) = sanitize_label(&value) {
                return Some(label);
            }
        }
    }

    let output = Command::new("adb").arg("devices").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    for line in stdout.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split_whitespace();
        let serial = fields.next()?;
        let status = fields.next().unwrap_or("");
        if status == "device" {
            if let Some(label) = sanitize_label(serial) {
                return Some(label);
            }
        }
    }
    None
}

fn adb_target_serial() -> Result<String, String> {
    for key in ["ANDROID_SERIAL", "ADB_SERIAL", "ADB_DEVICE_SERIAL"] {
        if let Ok(value) = env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }

    let output = Command::new("adb")
        .arg("devices")
        .output()
        .map_err(|err| format!("unable to run `adb devices`: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "`adb devices` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|err| format!("`adb devices` returned non-utf8 output: {err}"))?;
    for line in stdout.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split_whitespace();
        let serial = fields.next().unwrap_or("");
        let status = fields.next().unwrap_or("");
        if status == "device" && !serial.is_empty() {
            return Ok(serial.to_string());
        }
    }

    Err("no authorized adb device found".to_string())
}

fn adb_shell_output(serial: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("adb")
        .arg("-s")
        .arg(serial)
        .arg("shell")
        .args(args)
        .output()
        .map_err(|err| format!("unable to run adb shell for {serial}: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "adb shell for device {serial} failed: {}",
            stderr.trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|err| format!("adb shell for device {serial} returned non-utf8 output: {err}"))
}

fn adb_shell_path_exists(serial: &str, path: &str) -> Result<bool, String> {
    let output = Command::new("adb")
        .arg("-s")
        .arg(serial)
        .arg("shell")
        .arg("ls")
        .arg("-d")
        .arg(path)
        .output()
        .map_err(|err| format!("unable to probe adb path {path} on device {serial}: {err}"))?;
    Ok(output.status.success())
}

fn adb_shell_storage_root(serial: &str) -> Result<String, String> {
    let mut candidates = Vec::new();
    let mut add_candidate = |candidate: &str| {
        let trimmed = candidate.trim();
        if !trimmed.is_empty() && !candidates.iter().any(|existing| existing == trimmed) {
            candidates.push(trimmed.to_string());
        }
    };

    for candidate in [
        adb_shell_output(serial, &["echo", "$EXTERNAL_STORAGE"])?,
        adb_shell_output(serial, &["echo", "$SECONDARY_STORAGE"])?,
    ] {
        for value in candidate.split(':') {
            add_candidate(value);
        }
    }

    for candidate in ["/sdcard", "/storage/emulated/0", "/storage/self/primary"] {
        add_candidate(candidate);
    }

    for candidate in candidates {
        if adb_shell_path_exists(serial, &candidate)? {
            return Ok(candidate);
        }
    }

    Err(format!(
        "unable to detect a browsable Android storage root for device {serial}"
    ))
}

fn runtime_user_dir() -> Result<PathBuf, String> {
    if let Ok(value) = env::var("XDG_RUNTIME_DIR") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let status = fs::read_to_string("/proc/self/status")
        .map_err(|err| format!("unable to read /proc/self/status for runtime dir lookup: {err}"))?;
    let uid = status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| "unable to determine effective uid for runtime dir lookup".to_string())?;
    Ok(PathBuf::from(format!("/run/user/{uid}")))
}

fn suggested_github_name(root_path: &Path) -> Option<String> {
    let top_level = git_output(root_path, &["rev-parse", "--show-toplevel"])?;
    let repo_root = PathBuf::from(top_level.trim());
    let remote = git_output(&repo_root, &["remote", "get-url", "origin"])
        .or_else(|| git_output(&repo_root, &["config", "--get", "remote.origin.url"]))?;

    parse_git_remote_slug(&remote)
        .and_then(|slug| sanitize_label(&slug))
        .or_else(|| {
            repo_root
                .file_name()
                .and_then(|value| sanitize_label(&value.to_string_lossy()))
        })
}

fn git_output(root_path: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root_path)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_git_remote_slug(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let mut candidate = trimmed;
    let mut removed_host = false;
    if let Some((_, rest)) = candidate.split_once("://") {
        candidate = rest;
    }
    if let Some((_, rest)) = candidate.split_once(':') {
        if candidate.starts_with("git@") || candidate.contains('@') {
            candidate = rest;
            removed_host = true;
        }
    }
    if !removed_host {
        if let Some((head, rest)) = candidate.split_once('/') {
            if head.contains('.') || head.contains('@') {
                candidate = rest;
            }
        }
    }

    let candidate = candidate.trim_end_matches(".git").trim_end_matches('/');
    let mut parts = candidate.split('/').filter(|segment| !segment.is_empty());
    let owner = parts.next()?;
    let repo = parts.next().unwrap_or(owner);
    sanitize_label(&format!("{owner}-{repo}"))
}

fn find_mount_for_path(root_path: &Path) -> Option<MountInfo> {
    let content = fs::read_to_string("/proc/self/mountinfo").ok()?;
    let mut best_match: Option<MountInfo> = None;
    for line in content.lines() {
        let mount = parse_mountinfo_line(line)?;
        if !root_path.starts_with(&mount.mount_point) {
            continue;
        }
        let replace = best_match
            .as_ref()
            .map(|current| {
                mount.mount_point.components().count() > current.mount_point.components().count()
            })
            .unwrap_or(true);
        if replace {
            best_match = Some(mount);
        }
    }
    best_match
}

fn parse_mountinfo_line(line: &str) -> Option<MountInfo> {
    let (left, right) = line.split_once(" - ")?;
    let mut left_fields = left.split_whitespace();
    left_fields.next()?;
    left_fields.next()?;
    left_fields.next()?;
    left_fields.next()?;
    let mount_point = PathBuf::from(left_fields.next()?);

    let mut right_fields = right.split_whitespace();
    right_fields.next()?;
    let source = right_fields.next().unwrap_or("").to_string();
    Some(MountInfo {
        mount_point,
        source,
    })
}

fn extract_mount_host(source: &str) -> Option<String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("//") {
        let host = rest.split(['/', '\\']).next().unwrap_or("");
        return sanitize_label(host);
    }

    if let Some((prefix, _)) = trimmed.split_once(':') {
        let host = prefix.rsplit('@').next().unwrap_or(prefix);
        return sanitize_label(host);
    }

    None
}

fn sanitize_label(value: &str) -> Option<String> {
    let mut label = String::with_capacity(value.len());
    let mut previous_dash = false;

    for ch in value.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' {
            Some(ch)
        } else {
            None
        };

        match mapped {
            Some(ch) => {
                label.push(ch);
                previous_dash = false;
            }
            None if !previous_dash => {
                label.push('-');
                previous_dash = true;
            }
            None => {}
        }
    }

    let sanitized = label.trim_matches('-').to_string();
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

fn normalize_path(relative_path: &Path) -> String {
    relative_path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn ensure_indexer_settings_loaded() {
    INDEXER_SETTINGS_READY.get_or_init(|| {
        let _ = config::initialize_indexer_settings();
    });
}

fn find_android_mtp_browse_root(
    runtime_root: &Path,
    serial_hint: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let gvfs_root = runtime_root.join("gvfs");
    let metadata = match fs::metadata(&gvfs_root) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(None),
    };
    if !metadata.is_dir() {
        return Ok(None);
    }

    let mut mounts = Vec::new();
    let serial_hint = serial_hint.map(|value| value.to_ascii_lowercase());
    let read_dir = fs::read_dir(&gvfs_root).map_err(|err| {
        format!(
            "Android MTP browse root {} is not readable: {err}",
            gvfs_root.display()
        )
    })?;
    for item in read_dir {
        let entry = match item {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!(
                    "FOD indexer source list warning for {}: {err}",
                    gvfs_root.display()
                );
                continue;
            }
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("mtp:host=") {
            continue;
        }
        if let Some(serial_hint) = serial_hint.as_deref() {
            if !name.to_ascii_lowercase().contains(serial_hint) {
                continue;
            }
        }
        match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => mounts.push(path),
            Ok(_) => continue,
            Err(err) => {
                eprintln!(
                    "FOD indexer source list warning for {}: {err}",
                    path.display()
                );
            }
        }
    }

    mounts.sort();
    for mount in mounts {
        if let Some(root) = choose_android_storage_root(&mount)? {
            return Ok(Some(root));
        }
    }

    Ok(None)
}

fn choose_android_storage_root(mount_root: &Path) -> Result<Option<PathBuf>, String> {
    let read_dir = match fs::read_dir(mount_root) {
        Ok(read_dir) => read_dir,
        Err(err) => {
            return Err(format!(
                "Android MTP mount {} is not readable: {err}",
                mount_root.display()
            ))
        }
    };

    let mut directories = Vec::new();
    for item in read_dir {
        let entry = match item {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!(
                    "FOD indexer source list warning for {}: {err}",
                    mount_root.display()
                );
                continue;
            }
        };
        let path = entry.path();
        match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => directories.push(path),
            Ok(_) => continue,
            Err(err) => {
                eprintln!(
                    "FOD indexer source list warning for {}: {err}",
                    path.display()
                );
            }
        }
    }

    directories.sort();
    if directories.is_empty() {
        return Ok(Some(mount_root.to_path_buf()));
    }

    let preferred = directories.iter().find(|path| {
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        name.contains("pamięć")
            || name.contains("wewn")
            || name.contains("internal")
            || name.contains("shared")
            || name.contains("phone")
            || name.contains("storage")
    });

    if let Some(path) = preferred {
        return Ok(Some(path.clone()));
    }

    if directories.len() == 1 {
        return Ok(Some(directories[0].clone()));
    }

    Ok(Some(mount_root.to_path_buf()))
}
