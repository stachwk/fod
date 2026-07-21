pub mod pg_endpoints;
pub mod pg_runtime;

pub use pg_endpoints::{
    pg_connection_params_for_endpoint, resolve_pg_endpoint_config, PgEndpoint, PgEndpointConfig,
    PgEndpointMode, PgEndpointProbe, PgEndpointRole, PgObservedEndpointRole,
};
pub use pg_runtime::{
    PgConnectionPurpose, PgEndpointHealthRegistry, PgEndpointHealthSnapshot,
    PgEndpointHealthState, PgPoolIsolationMode, PgPoolPlan,
    DEFAULT_PG_HEALTH_FAILURE_THRESHOLD,
};

use crate::expand_user;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const SYSTEM_CONFIG_PATH: &str = "/etc/fod/fod_config.ini";
const USER_CONFIG_PATH: &str = ".config/fod/fod_config.ini";
const LOCAL_CONFIG_NAME: &str = "fod_config.ini";
const ENV_CONFIG_VAR: &str = "FOD_CONFIG";

#[derive(Debug, Clone)]
pub struct IniConfig {
    pub sections: HashMap<String, HashMap<String, String>>,
}

impl IniConfig {
    pub fn section(&self, name: &str) -> Option<&HashMap<String, String>> {
        self.sections.get(&name.to_lowercase())
    }
}

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

pub fn resolve_config_path_optional(file_path: Option<&Path>) -> Result<Option<PathBuf>, String> {
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
        return Ok(Some(config_path));
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    let mut base_dir: Option<PathBuf> = None;
    if let Some(path) = file_path {
        let expanded = expand_user(path);
        if expanded.is_file() {
            return Ok(Some(expanded));
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
            return Ok(Some(candidate));
        }
    }

    Ok(None)
}

pub fn resolve_config_path(file_path: Option<&Path>) -> Result<PathBuf, String> {
    resolve_config_path_optional(file_path)?.ok_or_else(|| {
        format!(
            "FOD configuration file not found. Expected {} or {}/{}.",
            SYSTEM_CONFIG_PATH,
            env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .display(),
            LOCAL_CONFIG_NAME
        )
    })
}

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
