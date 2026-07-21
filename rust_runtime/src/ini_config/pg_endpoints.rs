// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::collections::{HashMap, HashSet};
use std::env;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgObservedEndpointRole {
    PrimaryWritable,
    PrimaryReadOnly,
    Replica,
    InconsistentRecoveryWritable,
}

impl PgObservedEndpointRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrimaryWritable => "primary-writable",
            Self::PrimaryReadOnly => "primary-read-only",
            Self::Replica => "replica",
            Self::InconsistentRecoveryWritable => "inconsistent-recovery-writable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PgEndpointProbe {
    pub pg_is_in_recovery: bool,
    pub transaction_read_only: bool,
    pub observed_role: PgObservedEndpointRole,
}

impl PgEndpointProbe {
    pub fn from_flags(pg_is_in_recovery: bool, transaction_read_only: bool) -> Self {
        let observed_role = match (pg_is_in_recovery, transaction_read_only) {
            (false, false) => PgObservedEndpointRole::PrimaryWritable,
            (false, true) => PgObservedEndpointRole::PrimaryReadOnly,
            (true, true) => PgObservedEndpointRole::Replica,
            (true, false) => PgObservedEndpointRole::InconsistentRecoveryWritable,
        };
        Self {
            pg_is_in_recovery,
            transaction_read_only,
            observed_role,
        }
    }

    pub fn parse_row(value: &str) -> Result<Self, String> {
        let (recovery, read_only) = value.trim().split_once('|').ok_or_else(|| {
            format!(
                "invalid PostgreSQL endpoint probe result `{value}`: expected recovery|transaction_read_only"
            )
        })?;
        Ok(Self::from_flags(
            parse_postgres_bool(recovery, "pg_is_in_recovery")?,
            parse_postgres_bool(read_only, "transaction_read_only")?,
        ))
    }

    pub fn write_capable(self) -> bool {
        self.observed_role == PgObservedEndpointRole::PrimaryWritable
    }

    pub fn is_consistent(self) -> bool {
        self.observed_role != PgObservedEndpointRole::InconsistentRecoveryWritable
    }

    pub fn configured_role_matches(self, configured: PgEndpointRole) -> Option<bool> {
        match configured {
            PgEndpointRole::Unknown => None,
            PgEndpointRole::Primary => Some(matches!(
                self.observed_role,
                PgObservedEndpointRole::PrimaryWritable | PgObservedEndpointRole::PrimaryReadOnly
            )),
            PgEndpointRole::Replica => Some(self.observed_role == PgObservedEndpointRole::Replica),
        }
    }
}

pub fn pg_connection_params_for_endpoint(
    base_params: &HashMap<String, String>,
    endpoint: &PgEndpoint,
) -> HashMap<String, String> {
    let mut params = base_params.clone();
    params.insert("host".to_string(), endpoint.host.clone());
    params.insert("port".to_string(), endpoint.port.to_string());
    params
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

fn parse_postgres_bool(value: &str, label: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "t" | "true" | "1" | "on" | "yes" => Ok(true),
        "f" | "false" | "0" | "off" | "no" => Ok(false),
        other => Err(format!(
            "invalid PostgreSQL endpoint probe {label} value `{other}`"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_all_probe_flag_combinations() {
        let writable = PgEndpointProbe::from_flags(false, false);
        assert_eq!(
            writable.observed_role,
            PgObservedEndpointRole::PrimaryWritable
        );
        assert!(writable.write_capable());
        assert!(writable.is_consistent());

        let primary_read_only = PgEndpointProbe::from_flags(false, true);
        assert_eq!(
            primary_read_only.observed_role,
            PgObservedEndpointRole::PrimaryReadOnly
        );
        assert!(!primary_read_only.write_capable());
        assert_eq!(
            primary_read_only.configured_role_matches(PgEndpointRole::Primary),
            Some(true)
        );

        let replica = PgEndpointProbe::from_flags(true, true);
        assert_eq!(replica.observed_role, PgObservedEndpointRole::Replica);
        assert_eq!(
            replica.configured_role_matches(PgEndpointRole::Replica),
            Some(true)
        );

        let inconsistent = PgEndpointProbe::from_flags(true, false);
        assert_eq!(
            inconsistent.observed_role,
            PgObservedEndpointRole::InconsistentRecoveryWritable
        );
        assert!(!inconsistent.is_consistent());
    }

    #[test]
    fn parses_probe_rows_and_rejects_malformed_values() {
        assert_eq!(
            PgEndpointProbe::parse_row("false|off").unwrap(),
            PgEndpointProbe::from_flags(false, false)
        );
        assert_eq!(
            PgEndpointProbe::parse_row("t|on").unwrap(),
            PgEndpointProbe::from_flags(true, true)
        );
        assert!(PgEndpointProbe::parse_row("false").is_err());
        assert!(PgEndpointProbe::parse_row("maybe|off").is_err());
    }

    #[test]
    fn endpoint_params_override_only_host_and_port() {
        let base = HashMap::from([
            ("host".to_string(), "old-host".to_string()),
            ("port".to_string(), "5432".to_string()),
            ("dbname".to_string(), "foddbname".to_string()),
            ("sslmode".to_string(), "require".to_string()),
        ]);
        let endpoint = PgEndpoint {
            host: "db-new".to_string(),
            port: 15432,
            role: PgEndpointRole::Primary,
        };
        let resolved = pg_connection_params_for_endpoint(&base, &endpoint);
        assert_eq!(resolved.get("host").map(String::as_str), Some("db-new"));
        assert_eq!(resolved.get("port").map(String::as_str), Some("15432"));
        assert_eq!(
            resolved.get("dbname").map(String::as_str),
            Some("foddbname")
        );
        assert_eq!(resolved.get("sslmode").map(String::as_str), Some("require"));
    }
}
