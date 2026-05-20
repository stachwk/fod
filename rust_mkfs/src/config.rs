// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use fod_rust_runtime::{
    apply_runtime_env_from_map, env_var_with_legacy_alias, expand_user,
    runtime_env_var_name as runtime_env_var_name_shared, RuntimeConfig,
};

#[allow(dead_code)]
const SYSTEM_CONFIG_PATH: &str = "/etc/fod/fod_config.ini";
#[allow(dead_code)]
const USER_CONFIG_PATH: &str = ".config/fod/fod_config.ini";
#[allow(dead_code)]
const LOCAL_CONFIG_NAME: &str = "fod_config.ini";
#[allow(dead_code)]
const ENV_CONFIG_VAR: &str = "FOD_CONFIG";

#[derive(Debug, Clone)]
pub struct IniConfig {
    #[allow(dead_code)]
    pub sections: HashMap<String, HashMap<String, String>>,
}

impl IniConfig {
    #[allow(dead_code)]
    pub fn section(&self, name: &str) -> Option<&HashMap<String, String>> {
        self.sections.get(&name.to_lowercase())
    }
}

#[allow(dead_code)]
fn strip_inline_comment(value: &str) -> &str {
    let mut prev_was_whitespace = true;
    for (idx, ch) in value.char_indices() {
        if (ch == '#' || ch == ';') && prev_was_whitespace {
            return value[..idx].trim_end();
        }
        prev_was_whitespace = ch.is_whitespace();
    }
    value
}

pub fn resolve_config_path(file_path: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(env_path) = env::var_os(ENV_CONFIG_VAR) {
        let config_path = expand_user(Path::new(&env_path));
        if !config_path.is_file() {
            return Err(format!(
                "{} is set to {}, but that file does not exist",
                ENV_CONFIG_VAR,
                config_path.display()
            ));
        }
        fs::File::open(&config_path).map_err(|e| {
            format!(
                "{} is set to {}, but that file is not readable: {}",
                ENV_CONFIG_VAR,
                config_path.display(),
                e
            )
        })?;
        return Ok(config_path);
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    let mut base_dir: Option<PathBuf> = None;
    if let Some(path) = file_path {
        let expanded = expand_user(path);
        if expanded.is_file() {
            return Ok(expanded);
        }
        if expanded.is_dir() {
            base_dir = Some(expanded);
        } else if expanded.parent() != Some(Path::new(".")) {
            base_dir = expanded.parent().map(|p| p.to_path_buf());
        }
    }

    candidates.push(PathBuf::from(SYSTEM_CONFIG_PATH));
    if let Some(home) = env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(USER_CONFIG_PATH));
    }

    let search_root = base_dir
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    candidates.push(search_root.join(LOCAL_CONFIG_NAME));

    if let Some(path) = file_path {
        let file_name = path.file_name().map(|name| name.to_owned());
        if let Some(file_name) = file_name {
            if file_name.to_string_lossy() != LOCAL_CONFIG_NAME {
                candidates.push(search_root.join(file_name));
            }
        }
    }

    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "FOD configuration file not found. Expected {} or {}/{}.",
        SYSTEM_CONFIG_PATH,
        search_root.display(),
        LOCAL_CONFIG_NAME
    ))
}

#[allow(dead_code)]
pub fn load_config_parser(file_path: Option<&Path>) -> Result<(IniConfig, PathBuf), String> {
    let config_path = resolve_config_path(file_path)?;
    let contents = fs::read_to_string(&config_path).map_err(|e| {
        format!(
            "Unable to read FOD configuration: {}: {}",
            config_path.display(),
            e
        )
    })?;

    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current_section = String::new();

    for raw_line in contents.lines() {
        let line = strip_inline_comment(raw_line).trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim().to_lowercase();
            sections.entry(current_section.clone()).or_default();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let section = sections.entry(current_section.clone()).or_default();
            section.insert(key.trim().to_lowercase(), value.trim().to_string());
        }
    }

    Ok((IniConfig { sections }, config_path))
}

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
