// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::collections::HashMap;
use std::env;

#[allow(unused_imports)]
pub use fod_rust_runtime::ini_config::{load_config_parser, resolve_config_path, IniConfig};
use fod_rust_runtime::{
    apply_runtime_env_from_map, env_var_with_legacy_alias, parse_bool,
    runtime_env_var_name as runtime_env_var_name_shared, RuntimeConfig,
};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
enum StartupPassthroughKind {
    Bool,
    U64 { min: u64, max: Option<u64> },
    NonNegativeFloat,
}

#[allow(dead_code)]
const STARTUP_PASSTHROUGH_SPECS: &[(&str, &str, StartupPassthroughKind)] = &[
    (
        "fuse_event_threads",
        "FOD_FUSE_EVENT_THREADS",
        StartupPassthroughKind::U64 {
            min: 1,
            max: Some(256),
        },
    ),
    (
        "fuse_clone_fd",
        "FOD_FUSE_CLONE_FD",
        StartupPassthroughKind::Bool,
    ),
    (
        "task_read_active_limit",
        "FOD_TASK_READ_ACTIVE_LIMIT",
        StartupPassthroughKind::U64 { min: 0, max: None },
    ),
    (
        "task_write_active_limit",
        "FOD_TASK_WRITE_ACTIVE_LIMIT",
        StartupPassthroughKind::U64 { min: 0, max: None },
    ),
    (
        "pg_write_transaction_limit",
        "FOD_PG_WRITE_TRANSACTION_LIMIT",
        StartupPassthroughKind::U64 { min: 0, max: None },
    ),
    (
        "pg_control_transaction_limit",
        "FOD_PG_CONTROL_TRANSACTION_LIMIT",
        StartupPassthroughKind::U64 { min: 0, max: None },
    ),
    (
        "allow_other",
        "FOD_ALLOW_OTHER",
        StartupPassthroughKind::Bool,
    ),
    (
        "entry_timeout_seconds",
        "FOD_ENTRY_TIMEOUT_SECONDS",
        StartupPassthroughKind::NonNegativeFloat,
    ),
    (
        "attr_timeout_seconds",
        "FOD_ATTR_TIMEOUT_SECONDS",
        StartupPassthroughKind::NonNegativeFloat,
    ),
    (
        "negative_timeout_seconds",
        "FOD_NEGATIVE_TIMEOUT_SECONDS",
        StartupPassthroughKind::NonNegativeFloat,
    ),
];

fn resolved_runtime_map(config: &IniConfig) -> HashMap<String, String> {
    let mut runtime = config.section("fod").cloned().unwrap_or_default();
    let profile_name =
        env_var_with_legacy_alias("FOD_PROFILE").or_else(|| runtime.get("profile").cloned());
    if let Some(profile_name) = profile_name {
        for section_name in [
            format!("fod.profile.{}", profile_name),
            format!("fod.profile:{}", profile_name),
        ] {
            if let Some(section) = config.section(&section_name) {
                runtime.extend(section.clone());
                runtime.insert("profile".to_string(), profile_name.clone());
                break;
            }
        }
    }
    runtime
}

#[allow(dead_code)]
fn validate_startup_passthrough_value(
    key: &str,
    value: &str,
    kind: StartupPassthroughKind,
) -> Result<(), String> {
    match kind {
        StartupPassthroughKind::Bool => parse_bool(value)
            .map(|_| ())
            .map_err(|err| format!("{key}: {err}")),
        StartupPassthroughKind::U64 { min, max } => {
            let parsed = value
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("invalid {key}: {value}"))?;
            if parsed < min || max.is_some_and(|upper| parsed > upper) {
                return Err(format!("invalid {key}: {value}"));
            }
            Ok(())
        }
        StartupPassthroughKind::NonNegativeFloat => {
            let parsed = value
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("invalid {key}: {value}"))?;
            if !parsed.is_finite() || parsed < 0.0 {
                return Err(format!("invalid {key}: {value}"));
            }
            Ok(())
        }
    }
}

#[allow(dead_code)]
pub fn startup_passthrough_runtime_map(
    config: &IniConfig,
) -> Result<HashMap<String, String>, String> {
    let runtime = resolved_runtime_map(config);
    let mut result = HashMap::new();
    for (key, env_name, kind) in STARTUP_PASSTHROUGH_SPECS {
        let value = env::var(env_name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| runtime.get(*key).cloned());
        if let Some(value) = value {
            validate_startup_passthrough_value(key, &value, *kind)?;
            result.insert((*key).to_string(), value);
        }
    }
    Ok(result)
}

#[allow(dead_code)]
pub fn apply_startup_passthrough_env(
    config: &IniConfig,
) -> Result<HashMap<String, String>, String> {
    let values = startup_passthrough_runtime_map(config)?;
    for (key, env_name, _) in STARTUP_PASSTHROUGH_SPECS {
        if let Some(value) = values.get(*key) {
            env::set_var(env_name, value);
        }
    }
    Ok(values)
}

#[allow(dead_code)]
pub fn load_runtime_config(config: &IniConfig) -> Result<RuntimeConfig, String> {
    RuntimeConfig::from_runtime_map(&resolved_runtime_map(config))
}

#[allow(dead_code)]
pub fn runtime_env_var_name(key: &str) -> Option<String> {
    runtime_env_var_name_shared(key)
}

#[allow(dead_code)]
pub fn apply_runtime_env(runtime: &HashMap<String, String>) {
    apply_runtime_env_from_map(runtime);
}

#[cfg(test)]
mod tests {
    use super::{apply_runtime_env, runtime_env_var_name};
    use std::collections::HashMap;
    use std::env;

    #[test]
    fn maps_runtime_keys_to_fod_env_names() {
        let cases = [
            ("copy_dedupe_enabled", Some("FOD_COPY_DEDUPE_ENABLED")),
            ("fopen_direct_io", Some("FOD_FOPEN_DIRECT_IO")),
            ("fuse_writeback_cache", Some("FOD_FUSE_WRITEBACK_CACHE")),
            (
                "write_flush_threshold_bytes",
                Some("FOD_WRITE_FLUSH_THRESHOLD_BYTES"),
            ),
            (
                "lock_heartbeat_interval_seconds",
                Some("FOD_LOCK_HEARTBEAT_INTERVAL_SECONDS"),
            ),
            ("", None),
        ];

        for (key, expected) in cases {
            assert_eq!(
                runtime_env_var_name(key),
                expected.map(|value| value.to_string()),
                "runtime env mapping mismatch for key={}",
                key
            );
        }
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
        apply_runtime_env(&runtime);
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
}
