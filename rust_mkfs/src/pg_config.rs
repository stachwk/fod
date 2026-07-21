// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::env;

#[allow(unused_imports)]
pub use fod_rust_runtime::{make_conninfo, resolve_pg_connection_params};

const DEFAULT_PG_HOST: &str = "127.0.0.1";
const DEFAULT_PG_PORT: u16 = 5432;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgEndpointRole {
    Primary,
    Replica,
    Unknown,
}

impl PgEndpointRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Replica => "replica",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgEndpointMode {
    LegacySingle,
    ExplicitRoles,
    DiscoverRoles,
}

impl PgEndpointMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LegacySingle => "legacy-single",
            Self::ExplicitRoles => "explicit-roles",
            Self::DiscoverRoles => "discover-roles",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgEndpoint {
    pub host: String,
    pub port: u16,
    pub role: PgEndpointRole,
}

impl PgEndpoint {
    pub fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgEndpointConfig {
    pub mode: PgEndpointMode,
    pub role_discovery_required: bool,
    pub endpoints: Vec<PgEndpoint>,
}

impl PgEndpointConfig {
    pub fn primary_count(&self) -> usize {
        self.endpoints
            .iter()
            .filter(|endpoint| endpoint.role == PgEndpointRole::Primary)
            .count()
    }

    pub fn replica_count(&self) -> usize {
        self.endpoints
            .iter()
            .filter(|endpoint| endpoint.role == PgEndpointRole::Replica)
            .count()
    }

    pub fn unknown_count(&self) -> usize {
        self.endpoints
            .iter()
            .filter(|endpoint| endpoint.role == PgEndpointRole::Unknown)
            .count()
    }
}

pub fn resolve_pg_endpoint_config(
    db_config: &HashMap<String, String>,
) -> Result<PgEndpointConfig, String> {
    let (primary_hosts, replica_hosts, discovery_hosts) = endpoint_mode_values(db_config)?;
    let has_explicit_roles = primary_hosts.is_some() || replica_hosts.is_some();

    if has_explicit_roles {
        let mut endpoints = Vec::new();
        if let Some(value) = primary_hosts.as_deref() {
            endpoints.extend(parse_endpoint_list(
                value,
                PgEndpointRole::Primary,
                "primary_hosts",
            )?);
        }
        if let Some(value) = replica_hosts.as_deref() {
            endpoints.extend(parse_endpoint_list(
                value,
                PgEndpointRole::Replica,
                "replica_hosts",
            )?);
        }
        if !endpoints
            .iter()
            .any(|endpoint| endpoint.role == PgEndpointRole::Primary)
        {
            return Err("primary_hosts must contain at least one endpoint".to_string());
        }
        validate_unique_endpoints(&endpoints)?;
        return Ok(PgEndpointConfig {
            mode: PgEndpointMode::ExplicitRoles,
            role_discovery_required: false,
            endpoints,
        });
    }

    if let Some(value) = discovery_hosts.as_deref() {
        let endpoints = parse_endpoint_list(value, PgEndpointRole::Unknown, "hosts")?;
        validate_unique_endpoints(&endpoints)?;
        return Ok(PgEndpointConfig {
            mode: PgEndpointMode::DiscoverRoles,
            role_discovery_required: true,
            endpoints,
        });
    }

    let host = configured_value("FOD_PG_HOST", db_config, "host")
        .unwrap_or_else(|| DEFAULT_PG_HOST.to_string());
    let port_text = configured_value("FOD_PG_PORT", db_config, "port")
        .unwrap_or_else(|| DEFAULT_PG_PORT.to_string());
    let port = parse_port(&port_text, "port")?;
    let host = host.trim();
    if host.is_empty() {
        return Err("database host must not be empty".to_string());
    }

    Ok(PgEndpointConfig {
        mode: PgEndpointMode::LegacySingle,
        role_discovery_required: true,
        endpoints: vec![PgEndpoint {
            host: host.to_string(),
            port,
            role: PgEndpointRole::Unknown,
        }],
    })
}

fn endpoint_mode_values(
    db_config: &HashMap<String, String>,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let env_primary = nonempty_env("FOD_PG_PRIMARY_HOSTS");
    let env_replica = nonempty_env("FOD_PG_REPLICA_HOSTS");
    let env_hosts = nonempty_env("FOD_PG_HOSTS");
    let env_explicit = env_primary.is_some() || env_replica.is_some();

    if env_explicit && env_hosts.is_some() {
        return Err(
            "PostgreSQL endpoint environment is ambiguous: use FOD_PG_PRIMARY_HOSTS/FOD_PG_REPLICA_HOSTS or FOD_PG_HOSTS, not both"
                .to_string(),
        );
    }
    if env_explicit {
        return Ok((
            env_primary.or_else(|| nonempty_config(db_config, "primary_hosts")),
            env_replica.or_else(|| nonempty_config(db_config, "replica_hosts")),
            None,
        ));
    }
    if env_hosts.is_some() {
        return Ok((None, None, env_hosts));
    }

    let primary_hosts = nonempty_config(db_config, "primary_hosts");
    let replica_hosts = nonempty_config(db_config, "replica_hosts");
    let discovery_hosts = nonempty_config(db_config, "hosts");
    if (primary_hosts.is_some() || replica_hosts.is_some()) && discovery_hosts.is_some() {
        return Err(
            "database endpoint configuration is ambiguous: use primary_hosts/replica_hosts or hosts, not both"
                .to_string(),
        );
    }
    Ok((primary_hosts, replica_hosts, discovery_hosts))
}

fn configured_value(
    env_name: &str,
    db_config: &HashMap<String, String>,
    key: &str,
) -> Option<String> {
    nonempty_env(env_name).or_else(|| nonempty_config(db_config, key))
}

fn nonempty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn nonempty_config(db_config: &HashMap<String, String>, key: &str) -> Option<String> {
    db_config
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_endpoint_list(
    value: &str,
    role: PgEndpointRole,
    key: &str,
) -> Result<Vec<PgEndpoint>, String> {
    let raw_entries = value.split(',').collect::<Vec<_>>();
    if raw_entries.is_empty() || raw_entries.iter().any(|entry| entry.trim().is_empty()) {
        return Err(format!(
            "{key} must contain non-empty comma-separated host:port endpoints"
        ));
    }
    raw_entries
        .into_iter()
        .map(|entry| parse_endpoint(entry.trim(), role, key))
        .collect()
}

fn parse_endpoint(entry: &str, role: PgEndpointRole, key: &str) -> Result<PgEndpoint, String> {
    let (host, port_text) = if let Some(rest) = entry.strip_prefix('[') {
        let close = rest
            .find(']')
            .ok_or_else(|| format!("invalid {key} endpoint `{entry}`: missing closing ]"))?;
        let host = &rest[..close];
        let suffix = &rest[close + 1..];
        let port = suffix
            .strip_prefix(':')
            .ok_or_else(|| format!("invalid {key} endpoint `{entry}`: expected [host]:port"))?;
        (host, port)
    } else {
        let (host, port) = entry
            .rsplit_once(':')
            .ok_or_else(|| format!("invalid {key} endpoint `{entry}`: expected host:port"))?;
        if host.contains(':') {
            return Err(format!(
                "invalid {key} endpoint `{entry}`: IPv6 addresses must use [address]:port"
            ));
        }
        (host, port)
    };

    let host = host.trim();
    if host.is_empty() {
        return Err(format!("invalid {key} endpoint `{entry}`: host is empty"));
    }
    let port = parse_port(port_text, key)?;
    Ok(PgEndpoint {
        host: host.to_string(),
        port,
        role,
    })
}

fn parse_port(value: &str, key: &str) -> Result<u16, String> {
    value
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| format!("invalid {key} port `{value}`: expected 1..65535"))
}

fn validate_unique_endpoints(endpoints: &[PgEndpoint]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for endpoint in endpoints {
        let identity = (endpoint.host.to_ascii_lowercase(), endpoint.port);
        if !seen.insert(identity) {
            return Err(format!(
                "duplicate PostgreSQL endpoint `{}` is not allowed",
                endpoint.authority()
            ));
        }
    }
    Ok(())
}
