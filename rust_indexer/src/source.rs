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
];

static INDEXER_SETTINGS_READY: OnceLock<()> = OnceLock::new();

#[derive(Debug, Clone)]
struct MountInfo {
    mount_point: PathBuf,
    source: String,
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

pub fn adb_browse_root() -> Result<PathBuf, String> {
    let root = runtime_user_dir()?.join("adb");
    let metadata = fs::metadata(&root).map_err(|err| {
        format!(
            "ADB browse root {} is not accessible: {err}",
            root.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "ADB browse root {} is not a directory",
            root.display()
        ));
    }
    Ok(root)
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
