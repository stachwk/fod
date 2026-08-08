// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_monitor::{
    log_lane_observability, DbRepoPayloadObservability, LaneObservabilitySampler,
    LaneObservabilitySource,
};
use fod_rust_runtime::ini_config::{
    resolve_pg_endpoint_config, PgConnectionPurpose, PgEndpoint, PgEndpointConfig,
    PgEndpointHealthRegistry, PgEndpointHealthSnapshot, PgEndpointMode, PgEndpointProbe,
    PgEndpointRole, PgObservedEndpointRole, PgPoolIsolationMode, PgPoolPlan,
};
use fod_rust_runtime::{
    env_var_truthy_with_legacy_alias, MountRole, RuntimeConfig, RuntimeEndpointRoutingSettings,
    RuntimePayloadSettings,
};
use rust_hotpath::pg::DbRepo;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

pub const PG_POOL_LANES_ENV: &str = "FOD_PG_POOL_LANES_ENABLED";
pub const PG_OBSERVABILITY_INTERVAL_MS_ENV: &str = "FOD_PG_OBSERVABILITY_INTERVAL_MS";
const LEGACY_DSN_AUTHORITY: &str = "legacy-dsn";
const DEFAULT_PG_OBSERVABILITY_INTERVAL_MS: u64 = 5_000;
const MIN_PG_OBSERVABILITY_INTERVAL_MS: u64 = 100;
const MAX_PG_OBSERVABILITY_INTERVAL_MS: u64 = 3_600_000;
const CONNECTION_OK: c_int = 0;
const PGRES_TUPLES_OK: c_int = 2;

#[repr(C)]
struct PGconn {
    _private: [u8; 0],
}

#[repr(C)]
struct PGresult {
    _private: [u8; 0],
}

#[link(name = "pq")]
unsafe extern "C" {
    fn PQconnectdb(conninfo: *const c_char) -> *mut PGconn;
    fn PQstatus(conn: *const PGconn) -> c_int;
    fn PQerrorMessage(conn: *const PGconn) -> *const c_char;
    fn PQexec(conn: *mut PGconn, command: *const c_char) -> *mut PGresult;
    fn PQresultStatus(res: *const PGresult) -> c_int;
    fn PQntuples(res: *const PGresult) -> c_int;
    fn PQnfields(res: *const PGresult) -> c_int;
    fn PQgetvalue(res: *const PGresult, row_number: c_int, field_number: c_int) -> *const c_char;
    fn PQclear(res: *mut PGresult);
    fn PQfinish(conn: *mut PGconn);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbRepoLaneDiagnostics {
    pub opt_in_enabled: bool,
    pub dedicated_lanes_active: bool,
    pub mode: PgPoolIsolationMode,
    pub total_limit: usize,
    pub read_limit: usize,
    pub write_limit: usize,
    pub control_limit: usize,
    pub lease_limit: usize,
    pub legacy_dsn_only: bool,
    pub routing_enabled: bool,
    pub endpoint_mode: PgEndpointMode,
    pub routing_candidate_count: usize,
    pub selected_authority: Option<String>,
    pub startup_failovers: usize,
    pub selected_read_only: bool,
}

#[derive(Debug, Clone)]
struct EndpointRoutingSelection {
    endpoint: PgEndpoint,
    health: PgEndpointHealthSnapshot,
    startup_failovers: usize,
    read_only: bool,
}

struct EndpointRoutingResolution {
    conninfo: String,
    health: PgEndpointHealthRegistry,
    endpoint_mode: PgEndpointMode,
    candidate_count: usize,
    selected: Option<EndpointRoutingSelection>,
}

fn quote_conninfo_value(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

fn conninfo_for_endpoint(base_conninfo: &str, endpoint: &PgEndpoint) -> String {
    format!(
        "{} host={} port={}",
        base_conninfo.trim(),
        quote_conninfo_value(&endpoint.host),
        endpoint.port
    )
}

fn health_for_endpoint(
    endpoint: &PgEndpoint,
    snapshots: &[PgEndpointHealthSnapshot],
) -> Option<PgEndpointHealthSnapshot> {
    let authority = endpoint.authority();
    snapshots
        .iter()
        .find(|snapshot| snapshot.authority == authority)
        .cloned()
}

fn choose_endpoint(
    config: &PgEndpointConfig,
    snapshots: &[PgEndpointHealthSnapshot],
    purpose: PgConnectionPurpose,
    roles: &[PgEndpointRole],
    observed_replica_only: bool,
) -> Option<EndpointRoutingSelection> {
    let mut failed_candidates = 0usize;
    for endpoint in &config.endpoints {
        if !roles.contains(&endpoint.role) {
            continue;
        }
        let eligible = health_for_endpoint(endpoint, snapshots).filter(|snapshot| {
            snapshot.eligible_for(purpose)
                && (!observed_replica_only
                    || snapshot.observed_role == Some(PgObservedEndpointRole::Replica))
        });
        if let Some(health) = eligible {
            return Some(EndpointRoutingSelection {
                endpoint: endpoint.clone(),
                health,
                startup_failovers: failed_candidates,
                read_only: purpose == PgConnectionPurpose::Read,
            });
        }
        failed_candidates = failed_candidates.saturating_add(1);
    }
    None
}

fn effective_mount_role(runtime: &RuntimeConfig) -> Result<MountRole, String> {
    match std::env::var("FOD_ROLE") {
        Ok(value) if !value.trim().is_empty() => MountRole::parse(&value),
        Ok(_) | Err(std::env::VarError::NotPresent) => Ok(runtime.role),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("FOD_ROLE must contain valid UTF-8".to_string())
        }
    }
}

fn select_endpoint_for_mount(
    config: &PgEndpointConfig,
    mount_role: MountRole,
    snapshots: &[PgEndpointHealthSnapshot],
) -> Result<EndpointRoutingSelection, String> {
    let primary_roles = [PgEndpointRole::Primary, PgEndpointRole::Unknown];
    let replica_roles = [PgEndpointRole::Replica, PgEndpointRole::Unknown];

    let selected = match mount_role {
        MountRole::Primary => choose_endpoint(
            config,
            snapshots,
            PgConnectionPurpose::Write,
            &primary_roles,
            false,
        ),
        MountRole::Replica => choose_endpoint(
            config,
            snapshots,
            PgConnectionPurpose::Read,
            &replica_roles,
            true,
        ),
        MountRole::Auto => choose_endpoint(
            config,
            snapshots,
            PgConnectionPurpose::Write,
            &primary_roles,
            false,
        )
        .or_else(|| {
            choose_endpoint(
                config,
                snapshots,
                PgConnectionPurpose::Read,
                &[
                    PgEndpointRole::Replica,
                    PgEndpointRole::Primary,
                    PgEndpointRole::Unknown,
                ],
                false,
            )
        }),
    };

    selected.ok_or_else(|| {
        let states = snapshots
            .iter()
            .map(|snapshot| format!("{}:{}", snapshot.authority, snapshot.state.as_str()))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "no eligible PostgreSQL endpoint for mount role {} (endpoint states: {})",
            mount_role.as_str(),
            states
        )
    })
}

fn resolve_endpoint_routing(
    base_conninfo: &str,
    runtime: &RuntimeConfig,
) -> Result<EndpointRoutingResolution, String> {
    let settings = RuntimeEndpointRoutingSettings::from_env();
    let config = resolve_pg_endpoint_config(&HashMap::new())?;
    if !settings.enabled || config.mode == PgEndpointMode::LegacySingle {
        return Ok(EndpointRoutingResolution {
            conninfo: base_conninfo.to_string(),
            health: PgEndpointHealthRegistry::default(),
            endpoint_mode: config.mode,
            candidate_count: config.endpoints.len(),
            selected: None,
        });
    }

    let health = PgEndpointHealthRegistry::default();
    let mut snapshots = Vec::with_capacity(config.endpoints.len());
    for endpoint in &config.endpoints {
        let endpoint_conninfo = conninfo_for_endpoint(base_conninfo, endpoint);
        snapshots.push(health.record_probe_result(
            &endpoint.authority(),
            endpoint.role,
            postgres_endpoint_probe(&endpoint_conninfo),
        )?);
    }

    let mount_role = effective_mount_role(runtime)?;
    let selected = select_endpoint_for_mount(&config, mount_role, &snapshots)?;
    let conninfo = conninfo_for_endpoint(base_conninfo, &selected.endpoint);
    Ok(EndpointRoutingResolution {
        conninfo,
        health,
        endpoint_mode: config.mode,
        candidate_count: config.endpoints.len(),
        selected: Some(selected),
    })
}

enum DbRepoLaneStorage {
    Shared(DbRepo),
    Dedicated {
        read: DbRepo,
        write: DbRepo,
        control: DbRepo,
        lease: DbRepo,
    },
}

pub struct DbRepoLaneKeepalive {
    repositories: Vec<DbRepo>,
}

impl DbRepoLaneKeepalive {
    pub fn active_lane_count(&self) -> usize {
        self.repositories.len()
    }
}

pub struct DbRepoLanes {
    storage: DbRepoLaneStorage,
    plan: PgPoolPlan,
    opt_in_enabled: bool,
    health: PgEndpointHealthRegistry,
    endpoint_mode: PgEndpointMode,
    routing_candidate_count: usize,
    selected_endpoint: Option<PgEndpoint>,
    selected_health: Option<PgEndpointHealthSnapshot>,
    startup_failovers: usize,
    selected_read_only: bool,
}

impl DbRepoLanes {
    pub fn from_runtime(conninfo: &str, runtime: &RuntimeConfig) -> Result<Self, String> {
        let enabled = env_var_truthy_with_legacy_alias(PG_POOL_LANES_ENV, false);
        Self::with_opt_in(conninfo, runtime, enabled)
    }

    pub fn with_opt_in(
        conninfo: &str,
        runtime: &RuntimeConfig,
        opt_in_enabled: bool,
    ) -> Result<Self, String> {
        let mut plan = PgPoolPlan::from_total_limit(runtime.pool_max_connections);
        let routing = resolve_endpoint_routing(conninfo, runtime)?;
        let effective_conninfo = routing.conninfo.as_str();
        let dedicated = opt_in_enabled && plan.mode == PgPoolIsolationMode::DedicatedLanes;
        plan.routing_enabled = routing.selected.is_some();
        let payload_settings = RuntimePayloadSettings::from_env()?;
        let global_payload_observability =
            Arc::new(DbRepoPayloadObservability::with_in_flight_limit_bytes(
                payload_settings.in_flight_limit_bytes,
            ));

        let storage = if dedicated {
            DbRepoLaneStorage::Dedicated {
                read: build_lane_repo(
                    effective_conninfo,
                    runtime,
                    &plan,
                    PgConnectionPurpose::Read,
                    Arc::clone(&global_payload_observability),
                )?,
                write: build_lane_repo(
                    effective_conninfo,
                    runtime,
                    &plan,
                    PgConnectionPurpose::Write,
                    Arc::clone(&global_payload_observability),
                )?,
                control: build_lane_repo(
                    effective_conninfo,
                    runtime,
                    &plan,
                    PgConnectionPurpose::Control,
                    Arc::clone(&global_payload_observability),
                )?,
                lease: build_lane_repo(
                    effective_conninfo,
                    runtime,
                    &plan,
                    PgConnectionPurpose::Lease,
                    global_payload_observability,
                )?,
            }
        } else {
            DbRepoLaneStorage::Shared(DbRepo::with_runtime_and_global_payload_observability(
                effective_conninfo,
                runtime,
                global_payload_observability,
            )?)
        };

        let selected_endpoint = routing
            .selected
            .as_ref()
            .map(|selection| selection.endpoint.clone());
        let selected_health = routing
            .selected
            .as_ref()
            .map(|selection| selection.health.clone());
        let startup_failovers = routing
            .selected
            .as_ref()
            .map(|selection| selection.startup_failovers)
            .unwrap_or(0);
        let selected_read_only = routing
            .selected
            .as_ref()
            .is_some_and(|selection| selection.read_only);

        Ok(Self {
            storage,
            plan,
            opt_in_enabled,
            health: routing.health,
            endpoint_mode: routing.endpoint_mode,
            routing_candidate_count: routing.candidate_count,
            selected_endpoint,
            selected_health,
            startup_failovers,
            selected_read_only,
        })
    }

    pub fn repo_for(&self, purpose: PgConnectionPurpose) -> &DbRepo {
        match &self.storage {
            DbRepoLaneStorage::Shared(repo) => repo,
            DbRepoLaneStorage::Dedicated {
                read,
                write,
                control,
                lease,
            } => match purpose {
                PgConnectionPurpose::Read => read,
                PgConnectionPurpose::Write => write,
                PgConnectionPurpose::Control => control,
                PgConnectionPurpose::Lease => lease,
            },
        }
    }

    fn observability_repositories(
        &self,
    ) -> Vec<(&'static str, Arc<dyn LaneObservabilitySource + Send + Sync>)> {
        match &self.storage {
            DbRepoLaneStorage::Shared(repo) => vec![("shared", observability_source(repo))],
            DbRepoLaneStorage::Dedicated {
                read,
                write,
                control,
                lease,
            } => vec![
                (
                    PgConnectionPurpose::Read.as_str(),
                    observability_source(read),
                ),
                (
                    PgConnectionPurpose::Write.as_str(),
                    observability_source(write),
                ),
                (
                    PgConnectionPurpose::Control.as_str(),
                    observability_source(control),
                ),
                (
                    PgConnectionPurpose::Lease.as_str(),
                    observability_source(lease),
                ),
            ],
        }
    }

    pub fn into_mount_repo(self) -> (DbRepo, DbRepoLaneKeepalive) {
        match self.storage {
            DbRepoLaneStorage::Shared(repo) => (
                repo,
                DbRepoLaneKeepalive {
                    repositories: Vec::new(),
                },
            ),
            DbRepoLaneStorage::Dedicated {
                read,
                write,
                control,
                lease,
            } => (
                write,
                DbRepoLaneKeepalive {
                    repositories: vec![read, control, lease],
                },
            ),
        }
    }

    pub fn diagnostics(&self) -> DbRepoLaneDiagnostics {
        DbRepoLaneDiagnostics {
            opt_in_enabled: self.opt_in_enabled,
            dedicated_lanes_active: self.opt_in_enabled
                && self.plan.mode == PgPoolIsolationMode::DedicatedLanes,
            mode: self.plan.mode,
            total_limit: self.plan.total_limit,
            read_limit: self.plan.read_limit,
            write_limit: self.plan.write_limit,
            control_limit: self.plan.control_limit,
            lease_limit: self.plan.lease_limit,
            legacy_dsn_only: !self.plan.routing_enabled,
            routing_enabled: self.plan.routing_enabled,
            endpoint_mode: self.endpoint_mode,
            routing_candidate_count: self.routing_candidate_count,
            selected_authority: self.selected_endpoint.as_ref().map(PgEndpoint::authority),
            startup_failovers: self.startup_failovers,
            selected_read_only: self.selected_read_only,
        }
    }

    pub fn probe_health(&self, conninfo: &str) -> Result<PgEndpointHealthSnapshot, String> {
        if let Some(health) = &self.selected_health {
            return Ok(health.clone());
        }
        let result = postgres_endpoint_probe(conninfo);
        self.health
            .record_probe_result(LEGACY_DSN_AUTHORITY, PgEndpointRole::Unknown, result)
    }

    pub fn health_snapshots(&self) -> Result<Vec<PgEndpointHealthSnapshot>, String> {
        self.health.snapshots()
    }

    pub fn record_connection_failure(
        &self,
        error: &str,
    ) -> Result<PgEndpointHealthSnapshot, String> {
        if let Some(endpoint) = &self.selected_endpoint {
            self.health
                .record_failure(&endpoint.authority(), endpoint.role, error)
        } else {
            self.health
                .record_failure(LEGACY_DSN_AUTHORITY, PgEndpointRole::Unknown, error)
        }
    }
}

fn observability_source(repo: &DbRepo) -> Arc<dyn LaneObservabilitySource + Send + Sync> {
    Arc::new(repo.clone())
}

pub fn mount_with_lanes(
    conninfo: &str,
    runtime: &RuntimeConfig,
    requested_readonly: bool,
    mountpoint: &Path,
) -> Result<(), String> {
    log::debug!("FOD creating opt-in PostgreSQL repository lanes");
    let lanes = DbRepoLanes::from_runtime(conninfo, runtime)
        .map_err(|err| format!("failed to create PostgreSQL repository lanes: {err}"))?;
    let diagnostics = lanes.diagnostics();
    log::info!(
        "FOD PostgreSQL lanes: opt_in_enabled={} dedicated_lanes_active={} mode={} total={} read={} write={} control={} lease={} legacy_dsn_only={} routing_enabled={} endpoint_mode={} routing_candidate_count={} selected_authority={} startup_failovers={} selected_read_only={}",
        diagnostics.opt_in_enabled,
        diagnostics.dedicated_lanes_active,
        diagnostics.mode.as_str(),
        diagnostics.total_limit,
        diagnostics.read_limit,
        diagnostics.write_limit,
        diagnostics.control_limit,
        diagnostics.lease_limit,
        diagnostics.legacy_dsn_only,
        diagnostics.routing_enabled,
        diagnostics.endpoint_mode.as_str(),
        diagnostics.routing_candidate_count,
        diagnostics.selected_authority.as_deref().unwrap_or("none"),
        diagnostics.startup_failovers,
        diagnostics.selected_read_only,
    );

    if diagnostics.routing_enabled {
        for health in lanes.health_snapshots()? {
            let eligible = health
                .eligible_purposes
                .iter()
                .map(|purpose| purpose.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let automatic_routing_enabled = diagnostics
                .selected_authority
                .as_deref()
                .is_some_and(|authority| authority == health.authority);
            log::info!(
                "FOD PostgreSQL endpoint health: authority={} state={} configured_role={} observed_role={:?} successes={} failures={} eligible_purposes={} automatic_routing_enabled={}",
                health.authority,
                health.state.as_str(),
                health.configured_role.as_str(),
                health.observed_role.map(|role| role.as_str()),
                health.total_successes,
                health.total_failures,
                eligible,
                automatic_routing_enabled,
            );
        }
    } else {
        match lanes.probe_health(conninfo) {
        Ok(health) => {
            let eligible = health
                .eligible_purposes
                .iter()
                .map(|purpose| purpose.as_str())
                .collect::<Vec<_>>()
                .join(",");
            log::info!(
                "FOD PostgreSQL lane health: authority={} state={} configured_role={} observed_role={:?} successes={} failures={} eligible_purposes={} automatic_routing_enabled={}",
                health.authority,
                health.state.as_str(),
                health.configured_role.as_str(),
                health.observed_role.map(|role| role.as_str()),
                health.total_successes,
                health.total_failures,
                eligible,
                health.automatic_routing_enabled,
            );
        }
            Err(err) => log::warn!(
                "FOD PostgreSQL lane health probe unavailable; continuing with normal startup checks: {}",
                err
            ),
        }
    }

    let observability_repositories = lanes.observability_repositories();
    let process_rss_peak = Arc::new(AtomicU64::new(0));
    let control_repo = lanes.repo_for(PgConnectionPurpose::Control);
    log_postgres_diagnostics(control_repo);
    if let Err(err) =
        validate_and_log_postgres_requirements(control_repo, diagnostics.total_limit as u64)
    {
        let _ = lanes.record_connection_failure(&err);
        log_lane_observability(
            "startup-failed",
            &observability_repositories,
            &process_rss_peak,
        );
        return Err(format!(
            "PostgreSQL runtime requirements validation failed: {err}"
        ));
    }
    log::debug!("FOD reading startup snapshot through control lane");
    let snapshot = match control_repo.startup_snapshot() {
        Ok(snapshot) => snapshot,
        Err(err) => {
            let _ = lanes.record_connection_failure(&err);
            log_lane_observability(
                "startup-failed",
                &observability_repositories,
                &process_rss_peak,
            );
            return Err(format!("failed to read startup snapshot: {err}"));
        }
    };
    log::debug!("FOD startup snapshot={:?}", snapshot);
    log_lane_observability(
        "post-startup",
        &observability_repositories,
        &process_rss_peak,
    );
    let observability_interval = postgres_observability_interval()?;
    log::info!(
        "FOD PostgreSQL lane observability sampler: interval_ms={}",
        observability_interval.as_millis()
    );
    let mut observability_sampler = LaneObservabilitySampler::spawn(
        observability_repositories.clone(),
        Arc::clone(&process_rss_peak),
        observability_interval,
    )?;
    let settings =
        crate::startup::FodFuseSettings::from_runtime(runtime, &snapshot, requested_readonly);
    let (mount_repo, keepalive) = lanes.into_mount_repo();
    log::debug!(
        "FOD PostgreSQL non-write lane keepalive count={}",
        keepalive.active_lane_count()
    );
    let result = crate::startup::mount_fuse(mount_repo, runtime, settings, mountpoint, &snapshot);
    observability_sampler.stop();
    log_lane_observability("post-mount", &observability_repositories, &process_rss_peak);
    drop(keepalive);
    result
}

fn postgres_observability_interval() -> Result<Duration, String> {
    let interval_ms = match std::env::var(PG_OBSERVABILITY_INTERVAL_MS_ENV) {
        Ok(value) => value.parse::<u64>().map_err(|err| {
            format!(
                "{PG_OBSERVABILITY_INTERVAL_MS_ENV} must be an integer number of milliseconds: {err}"
            )
        })?,
        Err(std::env::VarError::NotPresent) => DEFAULT_PG_OBSERVABILITY_INTERVAL_MS,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(format!(
                "{PG_OBSERVABILITY_INTERVAL_MS_ENV} must contain valid UTF-8"
            ));
        }
    };
    if !(MIN_PG_OBSERVABILITY_INTERVAL_MS..=MAX_PG_OBSERVABILITY_INTERVAL_MS).contains(&interval_ms)
    {
        return Err(format!(
            "{PG_OBSERVABILITY_INTERVAL_MS_ENV} must be between {MIN_PG_OBSERVABILITY_INTERVAL_MS} and {MAX_PG_OBSERVABILITY_INTERVAL_MS} milliseconds"
        ));
    }
    Ok(Duration::from_millis(interval_ms))
}

pub fn validate_and_log_postgres_requirements(
    repo: &DbRepo,
    pool_max_connections: u64,
) -> Result<(), String> {
    let requirements = repo.postgres_runtime_requirements_for_pool_limit(pool_max_connections)?;
    for warning in requirements.server_configuration_warnings()? {
        log::warn!(
            "FOD PostgreSQL instance configuration requires attention: {}",
            warning
        );
    }

    let time_zone = requirements
        .settings
        .get("TimeZone")
        .map(|setting| setting.display_value())
        .unwrap_or_else(|| "unknown".to_string());
    let isolation = requirements
        .settings
        .get("transaction_isolation")
        .map(|setting| setting.display_value())
        .unwrap_or_else(|| "unknown".to_string());
    log::info!(
        "FOD PostgreSQL runtime requirements: server_version_num={} minimum_server_version_num={} pool_max_connections={} max_connections={} required_max_connections={} session_time_zone={} session_transaction_isolation={} session_timeouts=disabled standard_conforming_strings=on",
        requirements.server_version_num,
        requirements.minimum_server_version_num,
        requirements.pool_max_connections,
        requirements.max_connections()?,
        requirements.required_max_connections,
        time_zone,
        isolation,
    );
    Ok(())
}

fn log_postgres_diagnostics(repo: &DbRepo) {
    match repo.postgres_version_diagnostics() {
        Ok(postgres_versions) => log::info!(
            "FOD PostgreSQL diagnostics: libpq={} ({}) server={} ({}) major_relation={} compatibility={}",
            postgres_versions.libpq_version,
            postgres_versions.libpq_version_num,
            postgres_versions.server_version,
            postgres_versions.server_version_num,
            postgres_versions.major_relation,
            postgres_versions.compatibility_label()
        ),
        Err(err) => log::warn!(
            "FOD PostgreSQL diagnostics unavailable; continuing with normal startup checks: {}",
            err
        ),
    }
}

fn build_lane_repo(
    conninfo: &str,
    runtime: &RuntimeConfig,
    plan: &PgPoolPlan,
    purpose: PgConnectionPurpose,
    global_payload_observability: Arc<DbRepoPayloadObservability>,
) -> Result<DbRepo, String> {
    let mut lane_runtime = runtime.clone();
    lane_runtime.pool_max_connections = plan.limit_for(purpose) as u64;
    DbRepo::with_runtime_and_global_payload_observability(
        conninfo,
        &lane_runtime,
        global_payload_observability,
    )
}

fn postgres_endpoint_probe(conninfo: &str) -> Result<PgEndpointProbe, String> {
    let conninfo = CString::new(conninfo)
        .map_err(|_| "PostgreSQL connection string contains NUL byte".to_string())?;
    let sql = CString::new(
        "SELECT pg_is_in_recovery()::text || '|' || current_setting('transaction_read_only')",
    )
    .map_err(|_| "PostgreSQL probe SQL contains NUL byte".to_string())?;

    unsafe {
        let conn = PQconnectdb(conninfo.as_ptr());
        if conn.is_null() {
            return Err("libpq returned a null PostgreSQL connection".to_string());
        }
        if PQstatus(conn) != CONNECTION_OK {
            let error = connection_error(conn);
            PQfinish(conn);
            return Err(error);
        }

        let result = PQexec(conn, sql.as_ptr());
        if result.is_null() {
            let error = connection_error(conn);
            PQfinish(conn);
            return Err(error);
        }
        if PQresultStatus(result) != PGRES_TUPLES_OK
            || PQntuples(result) < 1
            || PQnfields(result) < 1
        {
            PQclear(result);
            let error = connection_error(conn);
            PQfinish(conn);
            return Err(error);
        }

        let value = PQgetvalue(result, 0, 0);
        let parsed = if value.is_null() {
            Err("PostgreSQL endpoint probe returned a null value".to_string())
        } else {
            PgEndpointProbe::parse_row(&CStr::from_ptr(value).to_string_lossy())
        };
        PQclear(result);
        PQfinish(conn);
        parsed
    }
}

fn connection_error(conn: *const PGconn) -> String {
    if conn.is_null() {
        return "PostgreSQL connection is null".to_string();
    }
    let error = unsafe { PQerrorMessage(conn) };
    if error.is_null() {
        "PostgreSQL connection or probe failed".to_string()
    } else {
        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .trim()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn runtime_with_pool_limit(limit: u64) -> RuntimeConfig {
        let mut values = HashMap::new();
        values.insert("pool_max_connections".to_string(), limit.to_string());
        RuntimeConfig::from_runtime_map(&values).unwrap()
    }

    #[test]
    fn disabled_wrapper_preserves_single_repo_contract() {
        let runtime = runtime_with_pool_limit(10);
        let lanes = DbRepoLanes::with_opt_in("host=127.0.0.1", &runtime, false).unwrap();
        let diagnostics = lanes.diagnostics();
        assert!(!diagnostics.opt_in_enabled);
        assert!(!diagnostics.dedicated_lanes_active);
        assert!(diagnostics.legacy_dsn_only);
        assert!(!diagnostics.routing_enabled);
        let (_, keepalive) = lanes.into_mount_repo();
        assert_eq!(keepalive.active_lane_count(), 0);
    }

    #[test]
    fn opt_in_activates_four_dedicated_lane_limits() {
        let runtime = runtime_with_pool_limit(10);
        let lanes = DbRepoLanes::with_opt_in("host=127.0.0.1", &runtime, true).unwrap();
        let diagnostics = lanes.diagnostics();
        assert!(diagnostics.opt_in_enabled);
        assert!(diagnostics.dedicated_lanes_active);
        assert_eq!(diagnostics.read_limit, 2);
        assert_eq!(diagnostics.write_limit, 6);
        assert_eq!(diagnostics.control_limit, 1);
        assert_eq!(diagnostics.lease_limit, 1);
        assert!(!diagnostics.routing_enabled);
        let (_, keepalive) = lanes.into_mount_repo();
        assert_eq!(keepalive.active_lane_count(), 3);
    }

    #[test]
    fn small_limits_keep_shared_fallback_even_when_opted_in() {
        let runtime = runtime_with_pool_limit(3);
        let lanes = DbRepoLanes::with_opt_in("host=127.0.0.1", &runtime, true).unwrap();
        let diagnostics = lanes.diagnostics();
        assert!(diagnostics.opt_in_enabled);
        assert!(!diagnostics.dedicated_lanes_active);
        assert_eq!(diagnostics.mode, PgPoolIsolationMode::SharedFallback);
        let (_, keepalive) = lanes.into_mount_repo();
        assert_eq!(keepalive.active_lane_count(), 0);
    }

    #[test]
    fn all_purposes_have_a_repo_handle() {
        let runtime = runtime_with_pool_limit(10);
        let lanes = DbRepoLanes::with_opt_in("host=127.0.0.1", &runtime, true).unwrap();
        for purpose in PgConnectionPurpose::ALL {
            let _ = lanes.repo_for(purpose);
        }
    }
}

#[cfg(test)]
mod endpoint_routing_tests {
    use super::*;

    fn config(endpoints: Vec<PgEndpoint>) -> PgEndpointConfig {
        PgEndpointConfig {
            mode: PgEndpointMode::ExplicitRoles,
            role_discovery_required: false,
            endpoints,
        }
    }

    #[test]
    fn explicit_fod_role_overrides_runtime_role_for_endpoint_selection() {
        let previous = std::env::var_os("FOD_ROLE");
        std::env::set_var("FOD_ROLE", "primary");
        let runtime = RuntimeConfig::from_runtime_map(&HashMap::new()).unwrap();
        assert_eq!(effective_mount_role(&runtime).unwrap(), MountRole::Primary);
        match previous {
            Some(value) => std::env::set_var("FOD_ROLE", value),
            None => std::env::remove_var("FOD_ROLE"),
        }
    }

    #[test]
    fn auto_prefers_writable_primary_over_healthy_replica() {
        let config = config(vec![
            PgEndpoint {
                host: "replica".to_string(),
                port: 5432,
                role: PgEndpointRole::Replica,
            },
            PgEndpoint {
                host: "primary".to_string(),
                port: 5432,
                role: PgEndpointRole::Primary,
            },
        ]);
        let health = PgEndpointHealthRegistry::default();
        let replica = health
            .record_probe_at(
                "replica:5432",
                PgEndpointRole::Replica,
                PgEndpointProbe::from_flags(true, true),
                1,
            )
            .unwrap();
        let primary = health
            .record_probe_at(
                "primary:5432",
                PgEndpointRole::Primary,
                PgEndpointProbe::from_flags(false, false),
                2,
            )
            .unwrap();

        let selected =
            select_endpoint_for_mount(&config, MountRole::Auto, &[replica, primary]).unwrap();
        assert_eq!(selected.endpoint.authority(), "primary:5432");
        assert!(!selected.read_only);
        assert_eq!(selected.startup_failovers, 0);
    }

    #[test]
    fn primary_selection_counts_unreachable_primary_before_healthy_fallback() {
        let config = config(vec![
            PgEndpoint {
                host: "primary-a".to_string(),
                port: 5432,
                role: PgEndpointRole::Primary,
            },
            PgEndpoint {
                host: "primary-b".to_string(),
                port: 5432,
                role: PgEndpointRole::Primary,
            },
        ]);
        let health = PgEndpointHealthRegistry::default();
        let failed = health
            .record_failure_at(
                "primary-a:5432",
                PgEndpointRole::Primary,
                "connection refused",
                1,
            )
            .unwrap();
        let healthy = health
            .record_probe_at(
                "primary-b:5432",
                PgEndpointRole::Primary,
                PgEndpointProbe::from_flags(false, false),
                2,
            )
            .unwrap();

        let selected =
            select_endpoint_for_mount(&config, MountRole::Primary, &[failed, healthy]).unwrap();
        assert_eq!(selected.endpoint.authority(), "primary-b:5432");
        assert_eq!(selected.startup_failovers, 1);
        assert!(!selected.read_only);
    }

    #[test]
    fn replica_role_selects_only_observed_replica() {
        let config = config(vec![
            PgEndpoint {
                host: "primary".to_string(),
                port: 5432,
                role: PgEndpointRole::Primary,
            },
            PgEndpoint {
                host: "replica".to_string(),
                port: 5432,
                role: PgEndpointRole::Replica,
            },
        ]);
        let health = PgEndpointHealthRegistry::default();
        let primary = health
            .record_probe_at(
                "primary:5432",
                PgEndpointRole::Primary,
                PgEndpointProbe::from_flags(false, false),
                1,
            )
            .unwrap();
        let replica = health
            .record_probe_at(
                "replica:5432",
                PgEndpointRole::Replica,
                PgEndpointProbe::from_flags(true, true),
                2,
            )
            .unwrap();

        let selected =
            select_endpoint_for_mount(&config, MountRole::Replica, &[primary, replica]).unwrap();
        assert_eq!(selected.endpoint.authority(), "replica:5432");
        assert!(selected.read_only);
    }
}
