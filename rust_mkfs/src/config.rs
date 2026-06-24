// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::collections::HashMap;

#[allow(unused_imports)]
pub use fod_rust_runtime::ini_config::{load_config_parser, resolve_config_path, IniConfig};
use fod_rust_runtime::{
    apply_runtime_env_from_map, env_var_with_legacy_alias,
    runtime_env_var_name as runtime_env_var_name_shared, RuntimeConfig,
};

#[allow(dead_code)]
pub fn load_runtime_config(config: &IniConfig) -> Result<RuntimeConfig, String> {
    let mut runtime = if let Some(section) = config.section("fod") {
        section.clone()
    } else {
        HashMap::new()
    };

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

    RuntimeConfig::from_runtime_map(&runtime)
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
