// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub mod ini_config;

pub use ini_config::{
    load_config_parser, resolve_config_path, resolve_config_path_optional, IniConfig,
};

pub const DEFAULT_METADATA_TTL: Duration = Duration::from_secs(1);
pub const DEFAULT_STATFS_TTL: Duration = Duration::from_secs(2);
pub const DEFAULT_LOCK_LEASE_TTL: Duration = Duration::from_secs(30);
pub const DEFAULT_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(50);
// Canonical FOD schema name. Keep it away from `public` so FOD objects do
// not collide with unrelated application tables in the same database.
pub const FOD_SCHEMA_NAME: &str = "fod";
pub const FOD_SEARCH_PATH: &str = "fod";
// Version is sourced from ../fod_version.txt via rust_runtime/build.rs.
pub const FOD_VERSION_LABEL: &str = env!("FOD_VERSION_LABEL");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountRole {
    Auto,
    Primary,
    Replica,
}

impl MountRole {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "primary" => Ok(Self::Primary),
            "replica" => Ok(Self::Replica),
            other => Err(format!("invalid mount role: {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Primary => "primary",
            Self::Replica => "replica",
        }
    }

    pub fn is_read_only(self, is_in_recovery: bool) -> bool {
        match self {
            Self::Auto => is_in_recovery,
            Self::Primary => false,
            Self::Replica => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtimePolicy {
    Default,
    NoAtime,
    Nodiratime,
    Relatime,
    StrictAtime,
}

pub trait AtimeStat {
    fn atime(&self) -> std::time::SystemTime;
    fn mtime(&self) -> std::time::SystemTime;
}

pub fn parse_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => Err(format!("invalid boolean value: {other}")),
    }
}

pub fn duration_to_micros(elapsed: Duration) -> u64 {
    elapsed.as_micros().min(u128::from(u64::MAX)) as u64
}

#[cfg(unix)]
pub fn current_hostname() -> Result<String, String> {
    let mut buffer = [0u8; 256];
    let rc = unsafe {
        libc::gethostname(
            buffer.as_mut_ptr() as *mut libc::c_char,
            buffer.len() as libc::size_t,
        )
    };
    if rc != 0 {
        return Err(format!(
            "failed to read hostname: {}",
            std::io::Error::last_os_error()
        ));
    }

    let len = buffer
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(buffer.len());
    let hostname = String::from_utf8(buffer[..len].to_vec())
        .map_err(|err| format!("hostname is not valid UTF-8: {err}"))?
        .trim()
        .to_string();
    if hostname.is_empty() {
        Err("hostname is empty".to_string())
    } else {
        Ok(hostname)
    }
}

#[cfg(not(unix))]
pub fn current_hostname() -> Result<String, String> {
    Err("hostname lookup is unavailable on this platform".to_string())
}

pub fn reloadable_snapshot_from_json(payload: &str) -> Result<HashMap<String, String>, String> {
    serde_json::from_str(payload)
        .map_err(|err| format!("runtime_overrides payload is malformed: {err}"))
}

pub fn reloadable_snapshot_to_json(snapshot: &HashMap<String, String>) -> Result<String, String> {
    serde_json::to_string(snapshot).map_err(|err| err.to_string())
}

pub fn ordered_reloadable_snapshot(snapshot: &HashMap<String, String>) -> Vec<(String, String)> {
    RuntimeConfig::reloadable_setting_keys()
        .iter()
        .filter_map(|key| {
            snapshot
                .get(*key)
                .map(|value| ((*key).to_string(), value.clone()))
        })
        .collect()
}

pub fn parse_size_bytes(value: &str) -> Result<u64, String> {
    let text = value.trim();
    if text.is_empty() {
        return Err("size value is empty".to_string());
    }
    let lower = text.to_ascii_lowercase();
    let (number_text, multiplier) = if let Some(stripped) = lower.strip_suffix("kib") {
        (stripped, 1024u64)
    } else if let Some(stripped) = lower.strip_suffix("mib") {
        (stripped, 1024u64.pow(2))
    } else if let Some(stripped) = lower.strip_suffix("gib") {
        (stripped, 1024u64.pow(3))
    } else if let Some(stripped) = lower.strip_suffix("tib") {
        (stripped, 1024u64.pow(4))
    } else if let Some(stripped) = lower.strip_suffix("kb") {
        (stripped, 1000u64)
    } else if let Some(stripped) = lower.strip_suffix("mb") {
        (stripped, 1000u64.pow(2))
    } else if let Some(stripped) = lower.strip_suffix("gb") {
        (stripped, 1000u64.pow(3))
    } else if let Some(stripped) = lower.strip_suffix("tb") {
        (stripped, 1000u64.pow(4))
    } else if let Some(stripped) = lower.strip_suffix('b') {
        (stripped, 1u64)
    } else {
        (lower.as_str(), 1u64)
    };

    let number = number_text
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("invalid size value: {value}"))?;
    number
        .checked_mul(multiplier)
        .ok_or_else(|| format!("size value overflows u64: {value}"))
}

#[cfg(unix)]
pub fn statvfs_total_bytes(path: &Path) -> Result<u64, String> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("path contains NUL byte: {}", path.display()))?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) };
    if rc != 0 {
        return Err(format!("statvfs failed for {}", path.display()));
    }
    let stats = unsafe { stats.assume_init() };
    Ok((stats.f_frsize as u64).saturating_mul(stats.f_blocks as u64))
}

#[cfg(not(unix))]
pub fn statvfs_total_bytes(_path: &Path) -> Result<u64, String> {
    Err("statvfs is unavailable on this platform".to_string())
}

pub fn resolve_max_fs_size_bytes(
    max_fs_size_bytes: Option<&str>,
    pg_visible_path: Option<&Path>,
    default_max_fs_size_bytes: u64,
) -> Result<u64, String> {
    let requested = match max_fs_size_bytes {
        Some(value) if !value.trim().is_empty() => parse_size_bytes(value)?,
        _ => default_max_fs_size_bytes,
    };
    if let Some(path) = pg_visible_path {
        let visible_total = statvfs_total_bytes(path)?;
        Ok(requested.min(visible_total))
    } else {
        Ok(requested)
    }
}

pub fn validate_runtime_tuning(runtime: &HashMap<String, String>) -> Result<(), String> {
    validate_runtime_tuning_lookup(&|key| runtime.get(key).cloned())
}

impl AtimePolicy {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "default" => Ok(Self::Default),
            "noatime" => Ok(Self::NoAtime),
            "nodiratime" => Ok(Self::Nodiratime),
            "relatime" => Ok(Self::Relatime),
            "strictatime" => Ok(Self::StrictAtime),
            other => Err(format!("invalid atime policy: {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::NoAtime => "noatime",
            Self::Nodiratime => "nodiratime",
            Self::Relatime => "relatime",
            Self::StrictAtime => "strictatime",
        }
    }

    pub fn should_update<T: AtimeStat>(self, is_dir: bool, attrs: &T) -> bool {
        match self {
            Self::Default | Self::StrictAtime => true,
            Self::NoAtime => false,
            Self::Nodiratime => !is_dir,
            Self::Relatime => {
                let stale_by_time = std::time::SystemTime::now()
                    .duration_since(attrs.atime())
                    .map(|age| age >= Duration::from_secs(24 * 60 * 60))
                    .unwrap_or(false);
                stale_by_time || attrs.atime() < attrs.mtime()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockBackend {
    Memory,
    PostgresLease,
}

impl LockBackend {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "memory" => Ok(Self::Memory),
            "postgres_lease" => Ok(Self::PostgresLease),
            other => Err(format!("invalid lock backend: {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::PostgresLease => "postgres_lease",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistBlockTransport {
    CopyBinaryStaging,
    BinaryBytea,
    LegacyHex,
}

impl PersistBlockTransport {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "copy_binary_staging" => Ok(Self::CopyBinaryStaging),
            "binary_bytea" => Ok(Self::BinaryBytea),
            "legacy_hex" => Ok(Self::LegacyHex),
            other => Err(format!("invalid persist block transport: {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CopyBinaryStaging => "copy_binary_staging",
            Self::BinaryBytea => "binary_bytea",
            Self::LegacyHex => "legacy_hex",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BootstrapOverrides {
    pub profile: Option<String>,
    pub role: String,
    pub selinux: String,
    pub acl: String,
    pub atime_policy: String,
    pub default_permissions: bool,
    pub lazytime: bool,
    pub sync: bool,
    pub dirsync: bool,
    pub debug: bool,
    pub log_level: Option<String>,
    pub force_read_only: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub profile: Option<String>,
    pub role: MountRole,
    pub force_read_only: bool,
    pub selinux: String,
    pub acl: String,
    pub atime_policy: AtimePolicy,
    pub default_permissions: bool,
    pub lazytime: bool,
    pub sync: bool,
    pub dirsync: bool,
    pub log_level: String,
    pub use_fuse_context: bool,
    pub fopen_direct_io: bool,
    pub fuse_writeback_cache: bool,
    pub use_rust_fuse: bool,
    pub pool_max_connections: u64,
    pub read_cache_blocks: u64,
    pub read_cache_eviction_policy: String,
    pub read_ahead_blocks: u64,
    pub sequential_read_ahead_blocks: u64,
    pub small_file_read_threshold_blocks: u64,
    pub workers_read: u64,
    pub workers_read_min_blocks: u64,
    pub workers_write: u64,
    pub workers_write_min_blocks: u64,
    pub persist_buffer_chunk_blocks: u64,
    pub persist_block_transport: PersistBlockTransport,
    pub synchronous_commit: String,
    pub write_flush_threshold_bytes: u64,
    pub max_fs_size_bytes: Option<u64>,
    pub pg_visible_path: Option<PathBuf>,
    pub metadata_cache_ttl: Duration,
    pub statfs_cache_ttl: Duration,
    pub lock_backend: LockBackend,
    pub lock_lease_ttl: Duration,
    pub lock_heartbeat_interval: Duration,
    pub lock_poll_interval: Duration,
    pub copy_dedupe_enabled: bool,
    pub copy_dedupe_min_blocks: u64,
    pub copy_dedupe_max_blocks: u64,
    pub copy_dedupe_crc_table: bool,
    pub enable_extents: bool,
    pub selinux_context: Option<String>,
    pub selinux_fscontext: Option<String>,
    pub selinux_defcontext: Option<String>,
    pub selinux_rootcontext: Option<String>,
}

/// General startup and runtime control values.
#[derive(Debug, Clone)]
pub struct RuntimeCoreSettings {
    pub profile: Option<String>,
    pub role: MountRole,
    pub force_read_only: bool,
    pub log_level: String,
    pub use_rust_fuse: bool,
    pub pool_max_connections: u64,
}

/// FUSE mount behavior and VFS semantics.
#[derive(Debug, Clone)]
pub struct RuntimeMountSettings {
    pub read_only: bool,
    pub default_permissions: bool,
    pub lazytime: bool,
    pub sync: bool,
    pub dirsync: bool,
    pub atime_policy: AtimePolicy,
    pub use_fuse_context: bool,
    pub fopen_direct_io: bool,
    pub fuse_writeback_cache: bool,
}

/// SELinux/ACL controls and mount contexts.
#[derive(Debug, Clone)]
pub struct RuntimeSecuritySettings {
    pub selinux_enabled: bool,
    pub acl_enabled: bool,
    pub selinux_context: Option<String>,
    pub selinux_fscontext: Option<String>,
    pub selinux_defcontext: Option<String>,
    pub selinux_rootcontext: Option<String>,
}

/// Lock backend selection and lease timing.
#[derive(Debug, Clone)]
pub struct RuntimeLockSettings {
    pub lock_backend: LockBackend,
    pub lock_lease_ttl: Duration,
    pub lock_heartbeat_interval: Duration,
    pub lock_poll_interval: Duration,
}

/// Cache TTLs and read-ahead tuning.
#[derive(Debug, Clone)]
pub struct RuntimeCacheSettings {
    pub metadata_cache_ttl: Duration,
    pub statfs_cache_ttl: Duration,
    pub read_cache_blocks: u64,
    pub read_cache_eviction_policy: String,
    pub read_ahead_blocks: u64,
    pub sequential_read_ahead_blocks: u64,
    pub small_file_read_threshold_blocks: u64,
}

/// Storage and hot-path tuning.
#[derive(Debug, Clone)]
pub struct RuntimeStorageSettings {
    pub workers_read: u64,
    pub workers_read_min_blocks: u64,
    pub workers_write: u64,
    pub workers_write_min_blocks: u64,
    pub persist_buffer_chunk_blocks: u64,
    pub persist_block_transport: PersistBlockTransport,
    pub synchronous_commit: String,
    pub write_flush_threshold_bytes: u64,
    pub max_fs_size_bytes: Option<u64>,
    pub pg_visible_path: Option<PathBuf>,
    pub copy_dedupe_enabled: bool,
    pub copy_dedupe_min_blocks: u64,
    pub copy_dedupe_max_blocks: u64,
    pub copy_dedupe_crc_table: bool,
    pub enable_extents: bool,
}

/// Runtime knobs that can be refreshed without remounting FOD.
/// Keep mount semantics and backend selection out of this set for now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeReloadableSettings {
    pub profile: Option<String>,
    pub log_level: String,
    pub metadata_cache_ttl: Duration,
    pub statfs_cache_ttl: Duration,
    pub read_cache_blocks: u64,
    pub read_ahead_blocks: u64,
    pub sequential_read_ahead_blocks: u64,
    pub small_file_read_threshold_blocks: u64,
    pub workers_read: u64,
    pub workers_read_min_blocks: u64,
    pub workers_write: u64,
    pub workers_write_min_blocks: u64,
    pub persist_buffer_chunk_blocks: u64,
    pub copy_dedupe_enabled: bool,
    pub copy_dedupe_min_blocks: u64,
    pub copy_dedupe_max_blocks: u64,
    pub copy_dedupe_crc_table: bool,
}

pub const RELOADABLE_RUNTIME_KEYS: &[&str] = &[
    "profile",
    "log_level",
    "metadata_cache_ttl_seconds",
    "statfs_cache_ttl_seconds",
    "read_cache_blocks",
    "read_ahead_blocks",
    "sequential_read_ahead_blocks",
    "small_file_read_threshold_blocks",
    "workers_read",
    "workers_read_min_blocks",
    "workers_write",
    "workers_write_min_blocks",
    "persist_buffer_chunk_blocks",
    "copy_dedupe_enabled",
    "copy_dedupe_min_blocks",
    "copy_dedupe_max_blocks",
    "copy_dedupe_crc_table",
];

fn runtime_env_var_name_internal(key: &str) -> Option<String> {
    let normalized = match key {
        "force_read_only" => "RUST_FUSE_READONLY".to_string(),
        _ => key
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string(),
    };
    if normalized.is_empty() {
        None
    } else {
        Some(format!("FOD_{}", normalized))
    }
}

pub fn runtime_env_var_name(key: &str) -> Option<String> {
    runtime_env_var_name_internal(key)
}

pub fn env_var_with_legacy_alias(name: &str) -> Option<String> {
    env::var(name).ok()
}

pub fn env_var_os_with_legacy_alias(name: &str) -> Option<std::ffi::OsString> {
    env::var_os(name)
}

pub fn env_var_truthy_with_legacy_alias(name: &str, default: bool) -> bool {
    parse_bool_or_default(env_var_with_legacy_alias(name), default)
}

pub fn env_var_truthy(name: &str) -> bool {
    env_var_truthy_with_legacy_alias(name, false)
}

pub fn expand_user(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(raw.as_ref())
}

pub fn resolve_path(value: &str, config_dir: &Path) -> String {
    let path = expand_user(Path::new(value));
    if path.is_absolute() {
        path.display().to_string()
    } else {
        config_dir.join(path).display().to_string()
    }
}

fn runtime_env_var_value_internal(key: &str) -> Option<String> {
    runtime_env_var_name_internal(key).and_then(|env_key| env_var_with_legacy_alias(&env_key))
}

fn map_value_or_default(map: &HashMap<String, String>, key: &str, default: &str) -> String {
    map.get(key).cloned().unwrap_or_else(|| default.to_string())
}

fn env_or_config_value(
    env_name: &str,
    map: &HashMap<String, String>,
    key: &str,
    default: &str,
) -> String {
    env_var_with_legacy_alias(env_name)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| map_value_or_default(map, key, default))
}

pub fn resolve_pg_connection_params(
    db_config: &HashMap<String, String>,
    config_dir: &Path,
) -> HashMap<String, String> {
    let mut params = HashMap::new();
    // Allow remote PostgreSQL endpoints to be overridden without editing the config file.
    params.insert(
        "host".to_string(),
        env_or_config_value("FOD_PG_HOST", db_config, "host", "127.0.0.1"),
    );
    params.insert(
        "port".to_string(),
        env_or_config_value("FOD_PG_PORT", db_config, "port", "5432"),
    );
    params.insert(
        "dbname".to_string(),
        env_or_config_value("FOD_PG_DBNAME", db_config, "dbname", "foddbname"),
    );
    params.insert(
        "user".to_string(),
        env_or_config_value("FOD_PG_USER", db_config, "user", "foduser"),
    );
    params.insert(
        "password".to_string(),
        env_or_config_value("FOD_PG_PASSWORD", db_config, "password", ""),
    );

    let sslmode = env_var_with_legacy_alias("FOD_PG_SSLMODE")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| map_value_or_default(db_config, "sslmode", "disable"));
    if !sslmode.is_empty() && sslmode != "disable" {
        params.insert("sslmode".to_string(), sslmode);
    }

    let sslrootcert = env_var_with_legacy_alias("FOD_PG_SSLROOTCERT")
        .unwrap_or_else(|| map_value_or_default(db_config, "sslrootcert", ""));
    if !sslrootcert.trim().is_empty() {
        params.insert(
            "sslrootcert".to_string(),
            resolve_path(&sslrootcert, config_dir),
        );
    }
    let sslcert = env_var_with_legacy_alias("FOD_PG_SSLCERT")
        .unwrap_or_else(|| map_value_or_default(db_config, "sslcert", ""));
    if !sslcert.trim().is_empty() {
        params.insert("sslcert".to_string(), resolve_path(&sslcert, config_dir));
    }
    let sslkey = env_var_with_legacy_alias("FOD_PG_SSLKEY")
        .unwrap_or_else(|| map_value_or_default(db_config, "sslkey", ""));
    if !sslkey.trim().is_empty() {
        params.insert("sslkey".to_string(), resolve_path(&sslkey, config_dir));
    }

    params
}

pub fn make_conninfo(params: &HashMap<String, String>) -> String {
    let mut parts = Vec::new();
    for key in [
        "host",
        "port",
        "dbname",
        "user",
        "password",
        "sslmode",
        "sslrootcert",
        "sslcert",
        "sslkey",
    ] {
        if let Some(value) = params.get(key) {
            if value.is_empty() {
                continue;
            }
            let escaped = value.replace('\'', "''");
            parts.push(format!("{}='{}'", key, escaped));
        }
    }
    parts.join(" ")
}

fn lookup_value<F>(lookup: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_bool_or_default(value: Option<String>, default: bool) -> bool {
    value
        .as_deref()
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "" | "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(default)
}

fn parse_u64(value: Option<String>, default: u64) -> u64 {
    value
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn parse_duration_secs(value: Option<String>, default_secs: f64) -> Duration {
    match value.as_deref().and_then(|value| value.parse::<f64>().ok()) {
        Some(value) if value.is_finite() && value >= 0.0 => Duration::from_secs_f64(value),
        _ => Duration::from_secs_f64(default_secs),
    }
}

fn lookup_bool<F>(lookup: &F, key: &str, default: bool) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    parse_bool_or_default(lookup_value(lookup, key), default)
}

fn lookup_u64<F>(lookup: &F, key: &str, default: u64) -> u64
where
    F: Fn(&str) -> Option<String>,
{
    parse_u64(lookup_value(lookup, key), default)
}

fn lookup_size_u64<F>(lookup: &F, key: &str, default: u64) -> u64
where
    F: Fn(&str) -> Option<String>,
{
    lookup_value(lookup, key)
        .and_then(|value| parse_size_bytes(&value).ok())
        .unwrap_or(default)
}

fn lookup_duration<F>(lookup: &F, key: &str, default_secs: f64) -> Duration
where
    F: Fn(&str) -> Option<String>,
{
    parse_duration_secs(lookup_value(lookup, key), default_secs)
}

fn lookup_path<F>(lookup: &F, key: &str) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    lookup_value(lookup, key).map(PathBuf::from)
}

fn lookup_string<F>(lookup: &F, key: &str, default: &str) -> String
where
    F: Fn(&str) -> Option<String>,
{
    lookup_value(lookup, key).unwrap_or_else(|| default.to_string())
}

fn lookup_optional_string<F>(lookup: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup_value(lookup, key)
}

fn validate_bool_value<F>(lookup: &F, key: &str) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(value) = lookup_value(lookup, key) {
        parse_bool(&value).map_err(|err| format!("{}: {}", key, err))?;
    }
    Ok(())
}

fn validate_selinux_value<F>(lookup: &F, key: &str) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(value) = lookup_value(lookup, key) {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized != "auto" && parse_bool(&normalized).is_err() {
            return Err(format!("invalid {}: {}", key, value));
        }
    }
    Ok(())
}

fn validate_u64_value<F>(lookup: &F, key: &str, allow_zero: bool) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(value) = lookup_value(lookup, key) {
        let parsed = value
            .parse::<u64>()
            .map_err(|_| format!("invalid {}: {}", key, value))?;
        if !allow_zero && parsed == 0 {
            return Err(format!("{} must be greater than zero", key));
        }
    }
    Ok(())
}

fn validate_duration_value<F>(lookup: &F, key: &str, allow_zero: bool) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(value) = lookup_value(lookup, key) {
        let parsed = value
            .parse::<f64>()
            .map_err(|_| format!("invalid {}: {}", key, value))?;
        if !parsed.is_finite() || (!allow_zero && parsed <= 0.0) || (allow_zero && parsed < 0.0) {
            let comparator = if allow_zero {
                "greater than or equal to zero"
            } else {
                "greater than zero"
            };
            return Err(format!("{} must be {}", key, comparator));
        }
    }
    Ok(())
}

fn validate_choice_value<F>(lookup: &F, key: &str, allowed: &[&str]) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(value) = lookup_value(lookup, key) {
        let normalized = value.trim().to_ascii_lowercase();
        if !allowed.iter().any(|candidate| *candidate == normalized) {
            return Err(format!("invalid {}: {}", key, value));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeValidationTarget {
    Tuning,
    RuntimeEnvOverrides,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeValueKind {
    Bool,
    Selinux,
    Choice(&'static [&'static str]),
    U64 { allow_zero: bool },
    Duration { allow_zero: bool },
    SizeBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeValueSpec {
    key: &'static str,
    kind: RuntimeValueKind,
    validate_in_tuning: bool,
    validate_in_runtime_env_overrides: bool,
}

impl RuntimeValueSpec {
    const fn tuning_only(key: &'static str, kind: RuntimeValueKind) -> Self {
        Self {
            key,
            kind,
            validate_in_tuning: true,
            validate_in_runtime_env_overrides: false,
        }
    }

    const fn tuning_and_runtime_env(key: &'static str, kind: RuntimeValueKind) -> Self {
        Self {
            key,
            kind,
            validate_in_tuning: true,
            validate_in_runtime_env_overrides: true,
        }
    }

    const fn applies_to(self, target: RuntimeValidationTarget) -> bool {
        match target {
            RuntimeValidationTarget::Tuning => self.validate_in_tuning,
            RuntimeValidationTarget::RuntimeEnvOverrides => self.validate_in_runtime_env_overrides,
        }
    }
}

fn validate_runtime_value_spec<F>(lookup: &F, spec: &RuntimeValueSpec) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    match spec.kind {
        RuntimeValueKind::Bool => validate_bool_value(lookup, spec.key),
        RuntimeValueKind::Selinux => validate_selinux_value(lookup, spec.key),
        RuntimeValueKind::Choice(allowed) => validate_choice_value(lookup, spec.key, allowed),
        RuntimeValueKind::U64 { allow_zero } => validate_u64_value(lookup, spec.key, allow_zero),
        RuntimeValueKind::Duration { allow_zero } => {
            validate_duration_value(lookup, spec.key, allow_zero)
        }
        RuntimeValueKind::SizeBytes => {
            if let Some(value) = lookup_value(lookup, spec.key) {
                parse_size_bytes(&value).map_err(|err| format!("{}: {}", spec.key, err))?;
            }
            Ok(())
        }
    }
}

fn validate_runtime_value_specs<F>(
    lookup: &F,
    target: RuntimeValidationTarget,
) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    for spec in RUNTIME_VALUE_SPECS {
        if spec.applies_to(target) {
            validate_runtime_value_spec(lookup, spec)?;
        }
    }
    Ok(())
}

const RUNTIME_VALUE_SPECS: &[RuntimeValueSpec] = &[
    RuntimeValueSpec::tuning_only(
        "role",
        RuntimeValueKind::Choice(&["auto", "primary", "replica"]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env("force_read_only", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only("selinux", RuntimeValueKind::Selinux),
    RuntimeValueSpec::tuning_only("acl", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only(
        "atime_policy",
        RuntimeValueKind::Choice(&[
            "default",
            "noatime",
            "nodiratime",
            "relatime",
            "strictatime",
        ]),
    ),
    RuntimeValueSpec::tuning_only("default_permissions", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only("lazytime", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only("sync", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only("dirsync", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only(
        "log_level",
        RuntimeValueKind::Choice(&["off", "error", "warn", "info", "debug", "trace"]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env("use_fuse_context", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env("fopen_direct_io", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env("fuse_writeback_cache", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env("use_rust_fuse", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env(
        "pool_max_connections",
        RuntimeValueKind::U64 { allow_zero: false },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "read_cache_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "read_cache_eviction_policy",
        RuntimeValueKind::Choice(&["fifo", "lru"]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "read_ahead_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "sequential_read_ahead_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "small_file_read_threshold_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "workers_read",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "workers_read_min_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "workers_write",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "workers_write_min_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "persist_buffer_chunk_blocks",
        RuntimeValueKind::U64 { allow_zero: false },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "persist_block_transport",
        RuntimeValueKind::Choice(&["copy_binary_staging", "binary_bytea", "legacy_hex"]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "write_flush_threshold_bytes",
        RuntimeValueKind::SizeBytes,
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "synchronous_commit",
        RuntimeValueKind::Choice(&[
            "on",
            "off",
            "local",
            "remote_write",
            "remote_apply",
            "true",
            "false",
        ]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "lock_backend",
        RuntimeValueKind::Choice(&["memory", "postgres_lease"]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "metadata_cache_ttl_seconds",
        RuntimeValueKind::Duration { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "statfs_cache_ttl_seconds",
        RuntimeValueKind::Duration { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "lock_lease_ttl_seconds",
        RuntimeValueKind::Duration { allow_zero: false },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "lock_heartbeat_interval_seconds",
        RuntimeValueKind::Duration { allow_zero: false },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "lock_poll_interval_seconds",
        RuntimeValueKind::Duration { allow_zero: false },
    ),
    RuntimeValueSpec::tuning_and_runtime_env("copy_dedupe_enabled", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env(
        "copy_dedupe_min_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "copy_dedupe_max_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env("copy_dedupe_crc_table", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env("enable_extents", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env("max_fs_size_bytes", RuntimeValueKind::SizeBytes),
];

fn validate_runtime_tuning_lookup<F>(lookup: &F) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    validate_runtime_value_specs(lookup, RuntimeValidationTarget::Tuning)
}

fn validate_runtime_env_overrides_lookup<F>(lookup: &F) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    validate_runtime_value_specs(lookup, RuntimeValidationTarget::RuntimeEnvOverrides)
}

impl RuntimeConfig {
    fn from_lookup<F>(lookup: F) -> Result<Self, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        validate_runtime_tuning_lookup(&lookup)?;
        let profile = lookup_optional_string(&lookup, "profile");
        let role =
            MountRole::parse(&lookup_string(&lookup, "role", "auto")).unwrap_or(MountRole::Auto);
        let force_read_only = lookup_bool(&lookup, "force_read_only", false);
        let selinux = lookup_string(&lookup, "selinux", "off");
        let acl = lookup_string(&lookup, "acl", "off");
        let atime_policy = AtimePolicy::parse(&lookup_string(&lookup, "atime_policy", "default"))
            .unwrap_or(AtimePolicy::Default);
        let default_permissions = lookup_bool(&lookup, "default_permissions", true);
        let lazytime = lookup_bool(&lookup, "lazytime", false);
        let sync = lookup_bool(&lookup, "sync", false);
        let dirsync = lookup_bool(&lookup, "dirsync", false);
        let log_level = lookup_string(&lookup, "log_level", "INFO");
        let use_fuse_context = lookup_bool(&lookup, "use_fuse_context", true);
        let fopen_direct_io = lookup_bool(&lookup, "fopen_direct_io", false);
        let fuse_writeback_cache = lookup_bool(&lookup, "fuse_writeback_cache", false);
        let use_rust_fuse = lookup_bool(&lookup, "use_rust_fuse", true);
        let pool_max_connections = lookup_u64(&lookup, "pool_max_connections", 10);
        let read_cache_blocks = lookup_u64(&lookup, "read_cache_blocks", 1024);
        let read_cache_eviction_policy =
            lookup_string(&lookup, "read_cache_eviction_policy", "fifo");
        let read_ahead_blocks = lookup_u64(&lookup, "read_ahead_blocks", 4);
        let sequential_read_ahead_blocks = lookup_u64(&lookup, "sequential_read_ahead_blocks", 8);
        let small_file_read_threshold_blocks =
            lookup_u64(&lookup, "small_file_read_threshold_blocks", 8);
        let workers_read = lookup_u64(&lookup, "workers_read", 4);
        let workers_read_min_blocks = lookup_u64(&lookup, "workers_read_min_blocks", 8);
        let workers_write = lookup_u64(&lookup, "workers_write", 4);
        let workers_write_min_blocks = lookup_u64(&lookup, "workers_write_min_blocks", 8);
        let persist_buffer_chunk_blocks = lookup_u64(&lookup, "persist_buffer_chunk_blocks", 128);
        let persist_block_transport = PersistBlockTransport::parse(&lookup_string(
            &lookup,
            "persist_block_transport",
            "copy_binary_staging",
        ))?;
        let synchronous_commit = lookup_string(&lookup, "synchronous_commit", "on");
        let write_flush_threshold_bytes =
            lookup_size_u64(&lookup, "write_flush_threshold_bytes", 0);
        let max_fs_size_bytes = lookup_value(&lookup, "max_fs_size_bytes")
            .and_then(|value| parse_size_bytes(&value).ok());
        let pg_visible_path = lookup_path(&lookup, "pg_visible_path");
        let metadata_cache_ttl = lookup_duration(
            &lookup,
            "metadata_cache_ttl_seconds",
            DEFAULT_METADATA_TTL.as_secs_f64(),
        );
        let statfs_cache_ttl = lookup_duration(
            &lookup,
            "statfs_cache_ttl_seconds",
            DEFAULT_STATFS_TTL.as_secs_f64(),
        );
        let lock_backend =
            LockBackend::parse(&lookup_string(&lookup, "lock_backend", "postgres_lease"))
                .unwrap_or(LockBackend::PostgresLease);
        let lock_lease_ttl = lookup_duration(
            &lookup,
            "lock_lease_ttl_seconds",
            DEFAULT_LOCK_LEASE_TTL.as_secs_f64(),
        );
        let lock_heartbeat_interval =
            lookup_duration(&lookup, "lock_heartbeat_interval_seconds", 10.0);
        let lock_poll_interval = lookup_duration(
            &lookup,
            "lock_poll_interval_seconds",
            DEFAULT_LOCK_POLL_INTERVAL.as_secs_f64(),
        );
        let copy_dedupe_enabled = lookup_bool(&lookup, "copy_dedupe_enabled", false);
        let copy_dedupe_min_blocks = lookup_u64(&lookup, "copy_dedupe_min_blocks", 0);
        let copy_dedupe_max_blocks = lookup_u64(&lookup, "copy_dedupe_max_blocks", 0);
        let copy_dedupe_crc_table = lookup_bool(&lookup, "copy_dedupe_crc_table", false);
        let enable_extents = lookup_bool(&lookup, "enable_extents", false);
        let selinux_context = lookup_optional_string(&lookup, "selinux_context");
        let selinux_fscontext = lookup_optional_string(&lookup, "selinux_fscontext");
        let selinux_defcontext = lookup_optional_string(&lookup, "selinux_defcontext");
        let selinux_rootcontext = lookup_optional_string(&lookup, "selinux_rootcontext");

        Ok(Self {
            profile,
            role,
            force_read_only,
            selinux,
            acl,
            atime_policy,
            default_permissions,
            lazytime,
            sync,
            dirsync,
            log_level,
            use_fuse_context,
            fopen_direct_io,
            fuse_writeback_cache,
            use_rust_fuse,
            pool_max_connections,
            read_cache_blocks,
            read_cache_eviction_policy,
            read_ahead_blocks,
            sequential_read_ahead_blocks,
            small_file_read_threshold_blocks,
            workers_read,
            workers_read_min_blocks,
            workers_write,
            workers_write_min_blocks,
            persist_buffer_chunk_blocks,
            persist_block_transport,
            synchronous_commit,
            write_flush_threshold_bytes,
            max_fs_size_bytes,
            pg_visible_path,
            metadata_cache_ttl,
            statfs_cache_ttl,
            lock_backend,
            lock_lease_ttl,
            lock_heartbeat_interval,
            lock_poll_interval,
            copy_dedupe_enabled,
            copy_dedupe_min_blocks,
            copy_dedupe_max_blocks,
            copy_dedupe_crc_table,
            enable_extents,
            selinux_context,
            selinux_fscontext,
            selinux_defcontext,
            selinux_rootcontext,
        })
    }

    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(runtime_env_var_value_internal)
    }

    pub fn from_runtime_map(runtime: &HashMap<String, String>) -> Result<Self, String> {
        Self::from_lookup(|key| runtime.get(key).cloned())
    }

    pub fn from_bootstrap(
        runtime: &HashMap<String, String>,
        overrides: &BootstrapOverrides,
    ) -> Result<Self, String> {
        Self::from_runtime_map(runtime)?.with_bootstrap_overrides(overrides)
    }

    pub fn with_bootstrap_overrides(
        mut self,
        overrides: &BootstrapOverrides,
    ) -> Result<Self, String> {
        self.apply_bootstrap_overrides(overrides)?;
        Ok(self)
    }

    pub fn core_settings(&self) -> RuntimeCoreSettings {
        RuntimeCoreSettings {
            profile: self.profile.clone(),
            role: self.role,
            force_read_only: self.force_read_only,
            log_level: self.log_level.clone(),
            use_rust_fuse: self.use_rust_fuse,
            pool_max_connections: self.pool_max_connections,
        }
    }

    pub fn mount_settings(&self, read_only: bool) -> RuntimeMountSettings {
        RuntimeMountSettings {
            read_only,
            default_permissions: self.default_permissions,
            lazytime: self.lazytime,
            sync: self.sync,
            dirsync: self.dirsync,
            atime_policy: self.atime_policy,
            use_fuse_context: self.use_fuse_context,
            fopen_direct_io: self.fopen_direct_io,
            fuse_writeback_cache: self.fuse_writeback_cache,
        }
    }

    pub fn security_settings(&self) -> RuntimeSecuritySettings {
        RuntimeSecuritySettings {
            selinux_enabled: self.selinux_enabled(),
            acl_enabled: self.acl_enabled(),
            selinux_context: self.selinux_context.clone(),
            selinux_fscontext: self.selinux_fscontext.clone(),
            selinux_defcontext: self.selinux_defcontext.clone(),
            selinux_rootcontext: self.selinux_rootcontext.clone(),
        }
    }

    pub fn lock_settings(&self, read_only: bool) -> RuntimeLockSettings {
        RuntimeLockSettings {
            lock_backend: self.lock_backend_for(read_only),
            lock_lease_ttl: self.lock_lease_ttl,
            lock_heartbeat_interval: self.lock_heartbeat_interval,
            lock_poll_interval: self.lock_poll_interval,
        }
    }

    pub fn cache_settings(&self) -> RuntimeCacheSettings {
        RuntimeCacheSettings {
            metadata_cache_ttl: self.metadata_cache_ttl,
            statfs_cache_ttl: self.statfs_cache_ttl,
            read_cache_blocks: self.read_cache_blocks,
            read_cache_eviction_policy: self.read_cache_eviction_policy.clone(),
            read_ahead_blocks: self.read_ahead_blocks,
            sequential_read_ahead_blocks: self.sequential_read_ahead_blocks,
            small_file_read_threshold_blocks: self.small_file_read_threshold_blocks,
        }
    }

    pub fn storage_settings(&self) -> RuntimeStorageSettings {
        RuntimeStorageSettings {
            workers_read: self.workers_read,
            workers_read_min_blocks: self.workers_read_min_blocks,
            workers_write: self.workers_write,
            workers_write_min_blocks: self.workers_write_min_blocks,
            persist_buffer_chunk_blocks: self.persist_buffer_chunk_blocks,
            persist_block_transport: self.persist_block_transport,
            synchronous_commit: self.synchronous_commit.clone(),
            write_flush_threshold_bytes: self.write_flush_threshold_bytes,
            max_fs_size_bytes: self.max_fs_size_bytes,
            pg_visible_path: self.pg_visible_path.clone(),
            copy_dedupe_enabled: self.copy_dedupe_enabled,
            copy_dedupe_min_blocks: self.copy_dedupe_min_blocks,
            copy_dedupe_max_blocks: self.copy_dedupe_max_blocks,
            copy_dedupe_crc_table: self.copy_dedupe_crc_table,
            enable_extents: self.enable_extents,
        }
    }

    pub fn reloadable_settings(&self) -> RuntimeReloadableSettings {
        RuntimeReloadableSettings {
            profile: self.profile.clone(),
            log_level: self.log_level.clone(),
            metadata_cache_ttl: self.metadata_cache_ttl,
            statfs_cache_ttl: self.statfs_cache_ttl,
            read_cache_blocks: self.read_cache_blocks,
            read_ahead_blocks: self.read_ahead_blocks,
            sequential_read_ahead_blocks: self.sequential_read_ahead_blocks,
            small_file_read_threshold_blocks: self.small_file_read_threshold_blocks,
            workers_read: self.workers_read,
            workers_read_min_blocks: self.workers_read_min_blocks,
            workers_write: self.workers_write,
            workers_write_min_blocks: self.workers_write_min_blocks,
            persist_buffer_chunk_blocks: self.persist_buffer_chunk_blocks,
            copy_dedupe_enabled: self.copy_dedupe_enabled,
            copy_dedupe_min_blocks: self.copy_dedupe_min_blocks,
            copy_dedupe_max_blocks: self.copy_dedupe_max_blocks,
            copy_dedupe_crc_table: self.copy_dedupe_crc_table,
        }
    }

    pub fn reloadable_setting_keys() -> &'static [&'static str] {
        RELOADABLE_RUNTIME_KEYS
    }

    pub fn reloadable_runtime_map(&self) -> HashMap<String, String> {
        let mut runtime = self.to_runtime_map();
        runtime.retain(|key, _| {
            Self::reloadable_setting_keys()
                .iter()
                .any(|candidate| *candidate == key.as_str())
        });
        runtime
    }

    pub fn with_reloadable_overrides(
        &self,
        overrides: &HashMap<String, String>,
    ) -> Result<Self, String> {
        for key in overrides.keys() {
            if !Self::reloadable_setting_keys()
                .iter()
                .any(|candidate| *candidate == key.as_str())
            {
                return Err(format!(
                    "{} is not reloadable; restart FOD to change it.",
                    key
                ));
            }
        }
        let mut runtime = self.to_runtime_map();
        for (key, value) in overrides {
            runtime.insert(key.clone(), value.clone());
        }
        Self::from_runtime_map(&runtime)
    }

    fn apply_bootstrap_overrides(&mut self, overrides: &BootstrapOverrides) -> Result<(), String> {
        validate_runtime_env_overrides_lookup(&runtime_env_var_value_internal)?;
        self.apply_runtime_env_overrides();
        if let Some(profile) = &overrides.profile {
            self.profile = Some(profile.clone());
        }
        self.role = MountRole::parse(&overrides.role)?;
        let selinux_value = overrides.selinux.trim().to_ascii_lowercase();
        if selinux_value != "auto" && parse_bool(&selinux_value).is_err() {
            return Err(format!("invalid selinux: {}", overrides.selinux));
        }
        if let Err(err) = parse_bool(&overrides.acl) {
            return Err(format!("acl: {}", err));
        }
        if let Some(level) = &overrides.log_level {
            match level.trim().to_ascii_lowercase().as_str() {
                "off" | "error" | "warn" | "info" | "debug" | "trace" => {}
                other => return Err(format!("invalid log_level: {other}")),
            }
        }
        self.force_read_only = overrides.force_read_only;
        self.selinux = overrides.selinux.clone();
        self.acl = overrides.acl.clone();
        self.atime_policy = AtimePolicy::parse(&overrides.atime_policy)?;
        self.default_permissions = overrides.default_permissions;
        self.lazytime = overrides.lazytime;
        self.sync = overrides.sync;
        self.dirsync = overrides.dirsync;
        self.log_level = overrides
            .log_level
            .clone()
            .or_else(|| {
                if overrides.debug {
                    Some("DEBUG".to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "INFO".to_string());
        self.use_fuse_context = true;
        self.use_rust_fuse = true;
        Ok(())
    }

    fn apply_runtime_env_overrides(&mut self) {
        self.fopen_direct_io = parse_bool_or_default(
            runtime_env_var_value_internal("fopen_direct_io"),
            self.fopen_direct_io,
        );
        self.fuse_writeback_cache = parse_bool_or_default(
            runtime_env_var_value_internal("fuse_writeback_cache"),
            self.fuse_writeback_cache,
        );

        self.force_read_only =
            env_var_truthy_with_legacy_alias("FOD_RUST_FUSE_READONLY", self.force_read_only);
        self.use_fuse_context =
            env_var_truthy_with_legacy_alias("FOD_USE_FUSE_CONTEXT", self.use_fuse_context);
        self.use_rust_fuse =
            env_var_truthy_with_legacy_alias("FOD_USE_RUST_FUSE", self.use_rust_fuse);
        self.pool_max_connections = parse_u64(
            env_var_with_legacy_alias("FOD_POOL_MAX_CONNECTIONS"),
            self.pool_max_connections,
        );
        self.read_cache_blocks = parse_u64(
            env_var_with_legacy_alias("FOD_READ_CACHE_BLOCKS"),
            self.read_cache_blocks,
        );
        self.read_cache_eviction_policy =
            env_var_with_legacy_alias("FOD_READ_CACHE_EVICTION_POLICY")
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| matches!(value.as_str(), "fifo" | "lru"))
                .unwrap_or_else(|| self.read_cache_eviction_policy.clone());
        self.read_ahead_blocks = parse_u64(
            env_var_with_legacy_alias("FOD_READ_AHEAD_BLOCKS"),
            self.read_ahead_blocks,
        );
        self.sequential_read_ahead_blocks = parse_u64(
            env_var_with_legacy_alias("FOD_SEQUENTIAL_READ_AHEAD_BLOCKS"),
            self.sequential_read_ahead_blocks,
        );
        self.small_file_read_threshold_blocks = parse_u64(
            env_var_with_legacy_alias("FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS"),
            self.small_file_read_threshold_blocks,
        );
        self.workers_read = parse_u64(
            env_var_with_legacy_alias("FOD_WORKERS_READ"),
            self.workers_read,
        );
        self.workers_read_min_blocks = parse_u64(
            env_var_with_legacy_alias("FOD_WORKERS_READ_MIN_BLOCKS"),
            self.workers_read_min_blocks,
        );
        self.workers_write = parse_u64(
            env_var_with_legacy_alias("FOD_WORKERS_WRITE"),
            self.workers_write,
        );
        self.workers_write_min_blocks = parse_u64(
            env_var_with_legacy_alias("FOD_WORKERS_WRITE_MIN_BLOCKS"),
            self.workers_write_min_blocks,
        );
        self.persist_buffer_chunk_blocks = parse_u64(
            env_var_with_legacy_alias("FOD_PERSIST_BUFFER_CHUNK_BLOCKS"),
            self.persist_buffer_chunk_blocks,
        );
        self.persist_block_transport = env_var_with_legacy_alias("FOD_PERSIST_BLOCK_TRANSPORT")
            .and_then(|value| PersistBlockTransport::parse(&value).ok())
            .unwrap_or(self.persist_block_transport);
        self.synchronous_commit = env_var_with_legacy_alias("FOD_SYNCHRONOUS_COMMIT")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.synchronous_commit.clone());
        self.write_flush_threshold_bytes =
            env_var_with_legacy_alias("FOD_WRITE_FLUSH_THRESHOLD_BYTES")
                .and_then(|value| parse_size_bytes(&value).ok())
                .unwrap_or(self.write_flush_threshold_bytes);
        self.max_fs_size_bytes = match env_var_with_legacy_alias("FOD_MAX_FS_SIZE_BYTES")
            .and_then(|value| parse_size_bytes(&value).ok())
        {
            Some(0) => None,
            Some(value) => Some(value),
            None => self.max_fs_size_bytes,
        };
        self.pg_visible_path = env_var_with_legacy_alias("FOD_PG_VISIBLE_PATH")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or(self.pg_visible_path.clone());
        self.metadata_cache_ttl = parse_duration_secs(
            env_var_with_legacy_alias("FOD_METADATA_CACHE_TTL_SECONDS"),
            self.metadata_cache_ttl.as_secs_f64(),
        );
        self.statfs_cache_ttl = parse_duration_secs(
            env_var_with_legacy_alias("FOD_STATFS_CACHE_TTL_SECONDS"),
            self.statfs_cache_ttl.as_secs_f64(),
        );
        self.lock_backend = env_var_with_legacy_alias("FOD_LOCK_BACKEND")
            .and_then(|value| LockBackend::parse(&value).ok())
            .unwrap_or(self.lock_backend);
        self.lock_lease_ttl = parse_duration_secs(
            env_var_with_legacy_alias("FOD_LOCK_LEASE_TTL_SECONDS"),
            self.lock_lease_ttl.as_secs_f64(),
        );
        self.lock_heartbeat_interval = parse_duration_secs(
            env_var_with_legacy_alias("FOD_LOCK_HEARTBEAT_INTERVAL_SECONDS"),
            self.lock_heartbeat_interval.as_secs_f64(),
        );
        self.lock_poll_interval = parse_duration_secs(
            env_var_with_legacy_alias("FOD_LOCK_POLL_INTERVAL_SECONDS"),
            self.lock_poll_interval.as_secs_f64(),
        );
        self.copy_dedupe_enabled = parse_bool_or_default(
            env_var_with_legacy_alias("FOD_COPY_DEDUPE_ENABLED"),
            self.copy_dedupe_enabled,
        );
        self.copy_dedupe_min_blocks = parse_u64(
            env_var_with_legacy_alias("FOD_COPY_DEDUPE_MIN_BLOCKS"),
            self.copy_dedupe_min_blocks,
        );
        self.copy_dedupe_max_blocks = parse_u64(
            env_var_with_legacy_alias("FOD_COPY_DEDUPE_MAX_BLOCKS"),
            self.copy_dedupe_max_blocks,
        );
        self.copy_dedupe_crc_table = env_var_truthy_with_legacy_alias(
            "FOD_COPY_DEDUPE_CRC_TABLE",
            self.copy_dedupe_crc_table,
        );
        self.enable_extents = parse_bool_or_default(
            env_var_with_legacy_alias("FOD_ENABLE_EXTENTS"),
            self.enable_extents,
        );
        self.selinux_context = env_var_with_legacy_alias("FOD_SELINUX_CONTEXT")
            .filter(|value| !value.trim().is_empty())
            .or(self.selinux_context.clone());
        self.selinux_fscontext = env_var_with_legacy_alias("FOD_SELINUX_FSCONTEXT")
            .filter(|value| !value.trim().is_empty())
            .or(self.selinux_fscontext.clone());
        self.selinux_defcontext = env_var_with_legacy_alias("FOD_SELINUX_DEFCONTEXT")
            .filter(|value| !value.trim().is_empty())
            .or(self.selinux_defcontext.clone());
        self.selinux_rootcontext = env_var_with_legacy_alias("FOD_SELINUX_ROOTCONTEXT")
            .filter(|value| !value.trim().is_empty())
            .or(self.selinux_rootcontext.clone());
    }

    pub fn effective_read_only(&self, cli_readonly: bool, is_in_recovery: bool) -> bool {
        cli_readonly
            || self.force_read_only
            || is_in_recovery
            || matches!(self.role, MountRole::Replica)
    }

    pub fn lock_backend_for(&self, read_only: bool) -> LockBackend {
        if read_only {
            LockBackend::Memory
        } else {
            self.lock_backend
        }
    }

    pub fn selinux_enabled(&self) -> bool {
        parse_bool_or_default(Some(self.selinux.clone()), false)
    }

    pub fn acl_enabled(&self) -> bool {
        parse_bool_or_default(Some(self.acl.clone()), false)
    }

    pub fn apply_env(&self) {
        set_string("FOD_PROFILE", self.profile.as_deref());
        set_string("FOD_ROLE", Some(self.role.as_str()));
        set_bool("FOD_RUST_FUSE_READONLY", self.force_read_only);
        set_string("FOD_SELINUX", Some(self.selinux.as_str()));
        set_string("FOD_ACL", Some(self.acl.as_str()));
        set_string("FOD_ATIME_POLICY", Some(self.atime_policy.as_str()));
        set_bool("FOD_DEFAULT_PERMISSIONS", self.default_permissions);
        set_bool("FOD_LAZYTIME", self.lazytime);
        set_bool("FOD_SYNC", self.sync);
        set_bool("FOD_DIRSYNC", self.dirsync);
        set_string("FOD_LOG_LEVEL", Some(self.log_level.as_str()));
        set_bool("FOD_USE_FUSE_CONTEXT", self.use_fuse_context);
        set_bool("FOD_FOPEN_DIRECT_IO", self.fopen_direct_io);
        set_bool("FOD_FUSE_WRITEBACK_CACHE", self.fuse_writeback_cache);
        set_bool("FOD_USE_RUST_FUSE", self.use_rust_fuse);
        set_u64("FOD_POOL_MAX_CONNECTIONS", self.pool_max_connections);
        set_u64("FOD_READ_CACHE_BLOCKS", self.read_cache_blocks);
        set_string(
            "FOD_READ_CACHE_EVICTION_POLICY",
            Some(self.read_cache_eviction_policy.as_str()),
        );
        set_u64("FOD_READ_AHEAD_BLOCKS", self.read_ahead_blocks);
        set_u64(
            "FOD_SEQUENTIAL_READ_AHEAD_BLOCKS",
            self.sequential_read_ahead_blocks,
        );
        set_u64(
            "FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS",
            self.small_file_read_threshold_blocks,
        );
        set_u64("FOD_WORKERS_READ", self.workers_read);
        set_u64("FOD_WORKERS_READ_MIN_BLOCKS", self.workers_read_min_blocks);
        set_u64("FOD_WORKERS_WRITE", self.workers_write);
        set_u64(
            "FOD_WORKERS_WRITE_MIN_BLOCKS",
            self.workers_write_min_blocks,
        );
        set_u64(
            "FOD_PERSIST_BUFFER_CHUNK_BLOCKS",
            self.persist_buffer_chunk_blocks,
        );
        set_string(
            "FOD_PERSIST_BLOCK_TRANSPORT",
            Some(self.persist_block_transport.as_str()),
        );
        set_string(
            "FOD_SYNCHRONOUS_COMMIT",
            Some(self.synchronous_commit.as_str()),
        );
        set_u64(
            "FOD_WRITE_FLUSH_THRESHOLD_BYTES",
            self.write_flush_threshold_bytes,
        );
        set_opt_u64("FOD_MAX_FS_SIZE_BYTES", self.max_fs_size_bytes);
        set_opt_path("FOD_PG_VISIBLE_PATH", self.pg_visible_path.as_ref());
        set_duration("FOD_METADATA_CACHE_TTL_SECONDS", self.metadata_cache_ttl);
        set_duration("FOD_STATFS_CACHE_TTL_SECONDS", self.statfs_cache_ttl);
        set_string("FOD_LOCK_BACKEND", Some(self.lock_backend.as_str()));
        set_duration("FOD_LOCK_LEASE_TTL_SECONDS", self.lock_lease_ttl);
        set_duration(
            "FOD_LOCK_HEARTBEAT_INTERVAL_SECONDS",
            self.lock_heartbeat_interval,
        );
        set_duration("FOD_LOCK_POLL_INTERVAL_SECONDS", self.lock_poll_interval);
        set_bool("FOD_COPY_DEDUPE_ENABLED", self.copy_dedupe_enabled);
        set_u64("FOD_COPY_DEDUPE_MIN_BLOCKS", self.copy_dedupe_min_blocks);
        set_u64("FOD_COPY_DEDUPE_MAX_BLOCKS", self.copy_dedupe_max_blocks);
        set_bool("FOD_COPY_DEDUPE_CRC_TABLE", self.copy_dedupe_crc_table);
        set_bool("FOD_ENABLE_EXTENTS", self.enable_extents);
        set_opt_string("FOD_SELINUX_CONTEXT", self.selinux_context.as_ref());
        set_opt_string("FOD_SELINUX_FSCONTEXT", self.selinux_fscontext.as_ref());
        set_opt_string("FOD_SELINUX_DEFCONTEXT", self.selinux_defcontext.as_ref());
        set_opt_string("FOD_SELINUX_ROOTCONTEXT", self.selinux_rootcontext.as_ref());
    }

    pub fn to_runtime_map(&self) -> HashMap<String, String> {
        let mut runtime = HashMap::new();
        set_map_string(&mut runtime, "profile", self.profile.as_deref());
        set_map_string(&mut runtime, "role", Some(self.role.as_str()));
        set_map_bool(&mut runtime, "force_read_only", self.force_read_only);
        set_map_string(&mut runtime, "selinux", Some(self.selinux.as_str()));
        set_map_string(&mut runtime, "acl", Some(self.acl.as_str()));
        set_map_string(
            &mut runtime,
            "atime_policy",
            Some(self.atime_policy.as_str()),
        );
        set_map_bool(
            &mut runtime,
            "default_permissions",
            self.default_permissions,
        );
        set_map_bool(&mut runtime, "lazytime", self.lazytime);
        set_map_bool(&mut runtime, "sync", self.sync);
        set_map_bool(&mut runtime, "dirsync", self.dirsync);
        set_map_string(&mut runtime, "log_level", Some(self.log_level.as_str()));
        set_map_bool(&mut runtime, "use_fuse_context", self.use_fuse_context);
        set_map_bool(&mut runtime, "fopen_direct_io", self.fopen_direct_io);
        set_map_bool(
            &mut runtime,
            "fuse_writeback_cache",
            self.fuse_writeback_cache,
        );
        set_map_bool(&mut runtime, "use_rust_fuse", self.use_rust_fuse);
        set_map_u64(
            &mut runtime,
            "pool_max_connections",
            self.pool_max_connections,
        );
        set_map_u64(&mut runtime, "read_cache_blocks", self.read_cache_blocks);
        set_map_string(
            &mut runtime,
            "read_cache_eviction_policy",
            Some(self.read_cache_eviction_policy.as_str()),
        );
        set_map_u64(&mut runtime, "read_ahead_blocks", self.read_ahead_blocks);
        set_map_u64(
            &mut runtime,
            "sequential_read_ahead_blocks",
            self.sequential_read_ahead_blocks,
        );
        set_map_u64(
            &mut runtime,
            "small_file_read_threshold_blocks",
            self.small_file_read_threshold_blocks,
        );
        set_map_u64(&mut runtime, "workers_read", self.workers_read);
        set_map_u64(
            &mut runtime,
            "workers_read_min_blocks",
            self.workers_read_min_blocks,
        );
        set_map_u64(&mut runtime, "workers_write", self.workers_write);
        set_map_u64(
            &mut runtime,
            "workers_write_min_blocks",
            self.workers_write_min_blocks,
        );
        set_map_u64(
            &mut runtime,
            "persist_buffer_chunk_blocks",
            self.persist_buffer_chunk_blocks,
        );
        set_map_string(
            &mut runtime,
            "persist_block_transport",
            Some(self.persist_block_transport.as_str()),
        );
        set_map_string(
            &mut runtime,
            "synchronous_commit",
            Some(self.synchronous_commit.as_str()),
        );
        set_map_u64(
            &mut runtime,
            "write_flush_threshold_bytes",
            self.write_flush_threshold_bytes,
        );
        set_map_opt_u64(&mut runtime, "max_fs_size_bytes", self.max_fs_size_bytes);
        set_map_opt_path(
            &mut runtime,
            "pg_visible_path",
            self.pg_visible_path.as_ref(),
        );
        set_map_duration(
            &mut runtime,
            "metadata_cache_ttl_seconds",
            self.metadata_cache_ttl,
        );
        set_map_duration(
            &mut runtime,
            "statfs_cache_ttl_seconds",
            self.statfs_cache_ttl,
        );
        set_map_string(
            &mut runtime,
            "lock_backend",
            Some(self.lock_backend.as_str()),
        );
        set_map_duration(&mut runtime, "lock_lease_ttl_seconds", self.lock_lease_ttl);
        set_map_duration(
            &mut runtime,
            "lock_heartbeat_interval_seconds",
            self.lock_heartbeat_interval,
        );
        set_map_duration(
            &mut runtime,
            "lock_poll_interval_seconds",
            self.lock_poll_interval,
        );
        set_map_bool(
            &mut runtime,
            "copy_dedupe_enabled",
            self.copy_dedupe_enabled,
        );
        set_map_u64(
            &mut runtime,
            "copy_dedupe_min_blocks",
            self.copy_dedupe_min_blocks,
        );
        set_map_u64(
            &mut runtime,
            "copy_dedupe_max_blocks",
            self.copy_dedupe_max_blocks,
        );
        set_map_bool(
            &mut runtime,
            "copy_dedupe_crc_table",
            self.copy_dedupe_crc_table,
        );
        set_map_bool(&mut runtime, "enable_extents", self.enable_extents);
        set_map_opt_string(
            &mut runtime,
            "selinux_context",
            self.selinux_context.as_ref(),
        );
        set_map_opt_string(
            &mut runtime,
            "selinux_fscontext",
            self.selinux_fscontext.as_ref(),
        );
        set_map_opt_string(
            &mut runtime,
            "selinux_defcontext",
            self.selinux_defcontext.as_ref(),
        );
        set_map_opt_string(
            &mut runtime,
            "selinux_rootcontext",
            self.selinux_rootcontext.as_ref(),
        );
        runtime
    }
}

fn set_string(name: &str, value: Option<&str>) {
    match value {
        Some(value) => env::set_var(name, value),
        None => env::remove_var(name),
    }
}

fn set_opt_string(name: &str, value: Option<&String>) {
    match value {
        Some(value) => env::set_var(name, value),
        None => env::remove_var(name),
    }
}

fn set_bool(name: &str, value: bool) {
    env::set_var(name, if value { "1" } else { "0" });
}

fn set_u64(name: &str, value: u64) {
    env::set_var(name, value.to_string());
}

fn set_opt_u64(name: &str, value: Option<u64>) {
    match value {
        Some(value) => env::set_var(name, value.to_string()),
        None => env::remove_var(name),
    }
}

fn set_opt_path(name: &str, value: Option<&PathBuf>) {
    match value {
        Some(value) => env::set_var(name, value),
        None => env::remove_var(name),
    }
}

fn set_duration(name: &str, value: Duration) {
    env::set_var(name, value.as_secs_f64().to_string());
}

fn set_map_string(runtime: &mut HashMap<String, String>, key: &str, value: Option<&str>) {
    match value {
        Some(value) => {
            runtime.insert(key.to_string(), value.to_string());
        }
        None => {
            runtime.remove(key);
        }
    }
}

fn set_map_opt_string(runtime: &mut HashMap<String, String>, key: &str, value: Option<&String>) {
    match value {
        Some(value) => {
            runtime.insert(key.to_string(), value.clone());
        }
        None => {
            runtime.remove(key);
        }
    }
}

fn set_map_bool(runtime: &mut HashMap<String, String>, key: &str, value: bool) {
    runtime.insert(
        key.to_string(),
        if value { "true" } else { "false" }.to_string(),
    );
}

fn set_map_u64(runtime: &mut HashMap<String, String>, key: &str, value: u64) {
    runtime.insert(key.to_string(), value.to_string());
}

fn set_map_opt_u64(runtime: &mut HashMap<String, String>, key: &str, value: Option<u64>) {
    match value {
        Some(value) => {
            runtime.insert(key.to_string(), value.to_string());
        }
        None => {
            runtime.remove(key);
        }
    }
}

fn set_map_opt_path(runtime: &mut HashMap<String, String>, key: &str, value: Option<&PathBuf>) {
    match value {
        Some(value) => {
            runtime.insert(key.to_string(), value.display().to_string());
        }
        None => {
            runtime.remove(key);
        }
    }
}

fn set_map_duration(runtime: &mut HashMap<String, String>, key: &str, value: Duration) {
    runtime.insert(key.to_string(), value.as_secs_f64().to_string());
}

pub fn apply_runtime_env_from_map(runtime: &HashMap<String, String>) {
    for (key, value) in runtime {
        if let Some(env_key) = runtime_env_var_name(key) {
            env::set_var(env_key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_runtime_env_from_map, parse_bool, parse_size_bytes, resolve_max_fs_size_bytes,
        runtime_env_var_name, validate_runtime_tuning, BootstrapOverrides, LockBackend, MountRole,
        RuntimeConfig, FOD_SCHEMA_NAME, FOD_SEARCH_PATH,
    };
    use std::collections::HashMap;
    use std::env;
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn maps_runtime_keys_to_fod_env_names() {
        assert_eq!(
            runtime_env_var_name("copy_dedupe_enabled"),
            Some("FOD_COPY_DEDUPE_ENABLED".to_string())
        );
        assert_eq!(
            runtime_env_var_name("write_flush_threshold_bytes"),
            Some("FOD_WRITE_FLUSH_THRESHOLD_BYTES".to_string())
        );
        assert_eq!(
            runtime_env_var_name("lock_heartbeat_interval_seconds"),
            Some("FOD_LOCK_HEARTBEAT_INTERVAL_SECONDS".to_string())
        );
        assert_eq!(
            runtime_env_var_name("pool_max_connections"),
            Some("FOD_POOL_MAX_CONNECTIONS".to_string())
        );
        assert_eq!(
            runtime_env_var_name("force_read_only"),
            Some("FOD_RUST_FUSE_READONLY".to_string())
        );
        assert_eq!(runtime_env_var_name(""), None);
    }

    #[test]
    fn applies_runtime_env_to_process_environment() {
        let mut runtime = HashMap::new();
        runtime.insert("copy_dedupe_enabled".to_string(), "true".to_string());
        runtime.insert(
            "write_flush_threshold_bytes".to_string(),
            "12345".to_string(),
        );
        runtime.insert(
            "lock_heartbeat_interval_seconds".to_string(),
            "7".to_string(),
        );
        apply_runtime_env_from_map(&runtime);
        assert_eq!(env::var("FOD_COPY_DEDUPE_ENABLED").unwrap(), "true");
        assert_eq!(
            env::var("FOD_WRITE_FLUSH_THRESHOLD_BYTES").unwrap(),
            "12345"
        );
        assert_eq!(
            env::var("FOD_LOCK_HEARTBEAT_INTERVAL_SECONDS").unwrap(),
            "7"
        );
        env::remove_var("FOD_COPY_DEDUPE_ENABLED");
        env::remove_var("FOD_WRITE_FLUSH_THRESHOLD_BYTES");
        env::remove_var("FOD_LOCK_HEARTBEAT_INTERVAL_SECONDS");
    }

    #[test]
    fn resolves_role_and_read_only() {
        assert!(MountRole::Replica.is_read_only(false));
        assert!(MountRole::Auto.is_read_only(true));
        assert!(!MountRole::Primary.is_read_only(true));
    }

    #[test]
    fn resolves_auto_and_replica_lock_roles() {
        let runtime = HashMap::new();
        let auto = RuntimeConfig::from_bootstrap(
            &runtime,
            &BootstrapOverrides {
                profile: None,
                role: "auto".to_string(),
                selinux: "off".to_string(),
                acl: "off".to_string(),
                atime_policy: "default".to_string(),
                default_permissions: true,
                lazytime: false,
                sync: false,
                dirsync: false,
                debug: false,
                log_level: None,
                force_read_only: false,
            },
        )
        .unwrap();
        assert_eq!(auto.role, MountRole::Auto);
        assert!(!auto.effective_read_only(false, false));
        assert_eq!(auto.lock_backend_for(false), LockBackend::PostgresLease);

        let replica = RuntimeConfig::from_bootstrap(
            &runtime,
            &BootstrapOverrides {
                profile: None,
                role: "replica".to_string(),
                selinux: "off".to_string(),
                acl: "off".to_string(),
                atime_policy: "default".to_string(),
                default_permissions: true,
                lazytime: false,
                sync: false,
                dirsync: false,
                debug: false,
                log_level: None,
                force_read_only: false,
            },
        )
        .unwrap();
        assert_eq!(replica.role, MountRole::Replica);
        assert!(replica.effective_read_only(false, false));
        assert_eq!(replica.lock_backend_for(true), LockBackend::Memory);
    }

    #[test]
    fn debug_bootstrap_forces_debug_log_level() {
        let runtime = HashMap::new();
        let config = RuntimeConfig::from_bootstrap(
            &runtime,
            &BootstrapOverrides {
                profile: None,
                role: "auto".to_string(),
                selinux: "off".to_string(),
                acl: "off".to_string(),
                atime_policy: "default".to_string(),
                default_permissions: true,
                lazytime: false,
                sync: false,
                dirsync: false,
                debug: true,
                log_level: None,
                force_read_only: false,
            },
        )
        .unwrap();
        assert_eq!(config.log_level, "DEBUG");
    }

    #[test]
    fn builds_runtime_config_from_zero_range_inputs() {
        let mut runtime = HashMap::new();
        runtime.insert("role".to_string(), "auto".to_string());
        runtime.insert("force_read_only".to_string(), "off".to_string());
        runtime.insert("selinux".to_string(), "off".to_string());
        runtime.insert("acl".to_string(), "off".to_string());
        runtime.insert("atime_policy".to_string(), "default".to_string());
        runtime.insert("default_permissions".to_string(), "on".to_string());
        runtime.insert("lazytime".to_string(), "off".to_string());
        runtime.insert("sync".to_string(), "off".to_string());
        runtime.insert("dirsync".to_string(), "off".to_string());
        runtime.insert("log_level".to_string(), "info".to_string());
        runtime.insert("use_fuse_context".to_string(), "on".to_string());
        runtime.insert("use_rust_fuse".to_string(), "on".to_string());
        runtime.insert("pool_max_connections".to_string(), "1".to_string());
        runtime.insert("read_cache_blocks".to_string(), "0".to_string());
        runtime.insert("read_ahead_blocks".to_string(), "0".to_string());
        runtime.insert("sequential_read_ahead_blocks".to_string(), "0".to_string());
        runtime.insert(
            "small_file_read_threshold_blocks".to_string(),
            "0".to_string(),
        );
        runtime.insert("workers_read".to_string(), "0".to_string());
        runtime.insert("workers_read_min_blocks".to_string(), "0".to_string());
        runtime.insert("workers_write".to_string(), "0".to_string());
        runtime.insert("workers_write_min_blocks".to_string(), "0".to_string());
        runtime.insert("persist_buffer_chunk_blocks".to_string(), "1".to_string());
        runtime.insert("synchronous_commit".to_string(), "on".to_string());
        runtime.insert("write_flush_threshold_bytes".to_string(), "0".to_string());
        runtime.insert("max_fs_size_bytes".to_string(), "0".to_string());
        runtime.insert("metadata_cache_ttl_seconds".to_string(), "0".to_string());
        runtime.insert("statfs_cache_ttl_seconds".to_string(), "0".to_string());
        runtime.insert("lock_backend".to_string(), "postgres_lease".to_string());
        runtime.insert("lock_lease_ttl_seconds".to_string(), "1".to_string());
        runtime.insert(
            "lock_heartbeat_interval_seconds".to_string(),
            "1".to_string(),
        );
        runtime.insert("lock_poll_interval_seconds".to_string(), "0.05".to_string());
        runtime.insert("copy_dedupe_enabled".to_string(), "off".to_string());
        runtime.insert("copy_dedupe_min_blocks".to_string(), "0".to_string());
        runtime.insert("copy_dedupe_max_blocks".to_string(), "0".to_string());
        runtime.insert("copy_dedupe_crc_table".to_string(), "off".to_string());
        let config = RuntimeConfig::from_runtime_map(&runtime).unwrap();
        assert_eq!(config.role, MountRole::Auto);
        assert!(!config.force_read_only);
        assert_eq!(config.pool_max_connections, 1);
        assert_eq!(config.read_cache_blocks, 0);
        assert_eq!(config.read_ahead_blocks, 0);
        assert_eq!(config.sequential_read_ahead_blocks, 0);
        assert_eq!(config.small_file_read_threshold_blocks, 0);
        assert_eq!(config.workers_read, 0);
        assert_eq!(config.workers_read_min_blocks, 0);
        assert_eq!(config.workers_write, 0);
        assert_eq!(config.workers_write_min_blocks, 0);
        assert_eq!(config.persist_buffer_chunk_blocks, 1);
        assert_eq!(config.write_flush_threshold_bytes, 0);
        assert_eq!(config.max_fs_size_bytes, Some(0));
        assert_eq!(config.metadata_cache_ttl, Duration::ZERO);
        assert_eq!(config.statfs_cache_ttl, Duration::ZERO);
        assert_eq!(config.lock_backend, LockBackend::PostgresLease);
        assert_eq!(config.lock_lease_ttl, Duration::from_secs(1));
        assert_eq!(config.lock_heartbeat_interval, Duration::from_secs(1));
        assert_eq!(config.lock_poll_interval, Duration::from_millis(50));
        assert!(!config.copy_dedupe_enabled);
        assert_eq!(config.copy_dedupe_min_blocks, 0);
        assert_eq!(config.copy_dedupe_max_blocks, 0);
        assert!(!config.copy_dedupe_crc_table);
    }

    #[test]
    fn rejects_invalid_runtime_config_ranges() {
        let mut runtime = HashMap::new();
        runtime.insert("pool_max_connections".to_string(), "0".to_string());
        runtime.insert("persist_buffer_chunk_blocks".to_string(), "0".to_string());
        runtime.insert(
            "lock_heartbeat_interval_seconds".to_string(),
            "0".to_string(),
        );
        runtime.insert("lock_poll_interval_seconds".to_string(), "0".to_string());

        assert!(RuntimeConfig::from_runtime_map(&runtime).is_err());
    }

    #[test]
    fn builds_runtime_config_from_bootstrap_inputs() {
        let mut runtime = HashMap::new();
        runtime.insert("lock_backend".to_string(), "memory".to_string());
        runtime.insert("copy_dedupe_enabled".to_string(), "true".to_string());
        runtime.insert(
            "lock_heartbeat_interval_seconds".to_string(),
            "7".to_string(),
        );
        runtime.insert("read_cache_blocks".to_string(), "2048".to_string());
        runtime.insert("read_ahead_blocks".to_string(), "4".to_string());
        runtime.insert("sequential_read_ahead_blocks".to_string(), "8".to_string());
        runtime.insert(
            "small_file_read_threshold_blocks".to_string(),
            "16".to_string(),
        );
        runtime.insert("workers_read".to_string(), "2".to_string());
        runtime.insert("workers_read_min_blocks".to_string(), "16".to_string());
        runtime.insert("workers_write".to_string(), "8".to_string());
        runtime.insert("workers_write_min_blocks".to_string(), "16".to_string());
        runtime.insert("persist_buffer_chunk_blocks".to_string(), "512".to_string());
        runtime.insert("synchronous_commit".to_string(), "off".to_string());
        runtime.insert("pool_max_connections".to_string(), "12".to_string());
        runtime.insert(
            "write_flush_threshold_bytes".to_string(),
            "64MiB".to_string(),
        );
        runtime.insert("max_fs_size_bytes".to_string(), "10GiB".to_string());
        let config = RuntimeConfig::from_bootstrap(
            &runtime,
            &BootstrapOverrides {
                profile: Some("bulk_write".to_string()),
                role: "primary".to_string(),
                selinux: "off".to_string(),
                acl: "off".to_string(),
                atime_policy: "relatime".to_string(),
                default_permissions: true,
                lazytime: false,
                sync: false,
                dirsync: false,
                debug: false,
                log_level: None,
                force_read_only: false,
            },
        )
        .unwrap();
        assert_eq!(config.profile.as_deref(), Some("bulk_write"));
        assert_eq!(config.role, MountRole::Primary);
        assert_eq!(config.lock_backend, super::LockBackend::Memory);
        assert_eq!(config.lock_heartbeat_interval, Duration::from_secs(7));
        assert_eq!(config.pool_max_connections, 12);
        assert!(config.copy_dedupe_enabled);
        assert_eq!(config.read_cache_blocks, 2048);
        assert_eq!(config.read_ahead_blocks, 4);
        assert_eq!(config.sequential_read_ahead_blocks, 8);
        assert_eq!(config.small_file_read_threshold_blocks, 16);
        assert_eq!(config.workers_read, 2);
        assert_eq!(config.workers_read_min_blocks, 16);
        assert_eq!(config.workers_write, 8);
        assert_eq!(config.workers_write_min_blocks, 16);
        assert_eq!(config.persist_buffer_chunk_blocks, 512);
        assert_eq!(config.synchronous_commit, "off");
        assert_eq!(config.write_flush_threshold_bytes, 64 * 1024 * 1024);
        assert_eq!(config.max_fs_size_bytes, Some(10 * 1024 * 1024 * 1024));
    }

    #[test]
    fn parses_generic_runtime_helpers() {
        assert!(parse_bool("1").unwrap());
        assert!(parse_bool("on").unwrap());
        assert!(!parse_bool("0").unwrap());
        assert!(parse_bool("maybe").is_err());

        assert_eq!(parse_size_bytes("50GiB").unwrap(), 50 * 1024u64.pow(3));
        assert_eq!(parse_size_bytes("1TiB").unwrap(), 1024u64.pow(4));
        assert_eq!(parse_size_bytes("4096").unwrap(), 4096);
    }

    #[test]
    fn validates_runtime_tuning_values() {
        let mut runtime = HashMap::new();
        runtime.insert("role".to_string(), "primary".to_string());
        runtime.insert("force_read_only".to_string(), "off".to_string());
        runtime.insert("selinux".to_string(), "auto".to_string());
        runtime.insert("acl".to_string(), "off".to_string());
        runtime.insert("default_permissions".to_string(), "on".to_string());
        runtime.insert("lazytime".to_string(), "off".to_string());
        runtime.insert("sync".to_string(), "off".to_string());
        runtime.insert("dirsync".to_string(), "off".to_string());
        runtime.insert("log_level".to_string(), "info".to_string());
        runtime.insert("use_fuse_context".to_string(), "on".to_string());
        runtime.insert("use_rust_fuse".to_string(), "on".to_string());
        runtime.insert("pool_max_connections".to_string(), "10".to_string());
        runtime.insert("read_cache_blocks".to_string(), "0".to_string());
        runtime.insert("read_ahead_blocks".to_string(), "0".to_string());
        runtime.insert("sequential_read_ahead_blocks".to_string(), "0".to_string());
        runtime.insert(
            "small_file_read_threshold_blocks".to_string(),
            "0".to_string(),
        );
        runtime.insert("workers_read".to_string(), "0".to_string());
        runtime.insert("workers_read_min_blocks".to_string(), "0".to_string());
        runtime.insert("workers_write".to_string(), "0".to_string());
        runtime.insert("workers_write_min_blocks".to_string(), "0".to_string());
        runtime.insert("persist_buffer_chunk_blocks".to_string(), "1".to_string());
        runtime.insert(
            "lock_heartbeat_interval_seconds".to_string(),
            "1".to_string(),
        );
        runtime.insert("lock_poll_interval_seconds".to_string(), "0.05".to_string());
        runtime.insert("lock_lease_ttl_seconds".to_string(), "30".to_string());
        runtime.insert("copy_dedupe_enabled".to_string(), "off".to_string());
        runtime.insert("copy_dedupe_min_blocks".to_string(), "0".to_string());
        runtime.insert("copy_dedupe_max_blocks".to_string(), "0".to_string());
        runtime.insert("write_flush_threshold_bytes".to_string(), "0".to_string());
        runtime.insert("metadata_cache_ttl_seconds".to_string(), "0".to_string());
        runtime.insert("statfs_cache_ttl_seconds".to_string(), "0".to_string());
        runtime.insert("max_fs_size_bytes".to_string(), "1TiB".to_string());
        runtime.insert("copy_dedupe_crc_table".to_string(), "off".to_string());
        runtime.insert("lock_backend".to_string(), "postgres_lease".to_string());
        runtime.insert("atime_policy".to_string(), "relatime".to_string());
        runtime.insert("synchronous_commit".to_string(), "on".to_string());
        validate_runtime_tuning(&runtime).unwrap();

        runtime.insert("pool_max_connections".to_string(), "0".to_string());
        runtime.insert("persist_buffer_chunk_blocks".to_string(), "0".to_string());
        assert!(validate_runtime_tuning(&runtime).is_err());
    }

    #[test]
    fn clamps_size_to_visible_fs() {
        let temp_dir = std::env::temp_dir();
        let result = resolve_max_fs_size_bytes(Some("1TiB"), Some(Path::new(&temp_dir)), 4096);
        assert!(result.is_ok());
        assert!(result.unwrap() > 0);
    }

    #[test]
    fn runtime_uses_fod_schema_not_public() {
        assert_eq!(FOD_SCHEMA_NAME, "fod");
        assert_eq!(FOD_SEARCH_PATH, "fod");
        assert_ne!(FOD_SCHEMA_NAME, "public");
        assert_ne!(FOD_SEARCH_PATH, "public");
    }
}
