// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_runtime::ini_config::{
    PgConnectionPurpose, PgEndpointHealthRegistry, PgEndpointHealthSnapshot, PgEndpointProbe,
    PgEndpointRole, PgPoolIsolationMode, PgPoolPlan,
};
use fod_rust_runtime::{env_var_truthy_with_legacy_alias, RuntimeConfig};
use rust_hotpath::pg::DbRepo;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::Path;

pub const PG_POOL_LANES_ENV: &str = "FOD_PG_POOL_LANES_ENABLED";
const LEGACY_DSN_AUTHORITY: &str = "legacy-dsn";
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
        let plan = PgPoolPlan::from_total_limit(runtime.pool_max_connections);
        let dedicated = opt_in_enabled && plan.mode == PgPoolIsolationMode::DedicatedLanes;

        let storage = if dedicated {
            DbRepoLaneStorage::Dedicated {
                read: build_lane_repo(conninfo, runtime, &plan, PgConnectionPurpose::Read)?,
                write: build_lane_repo(conninfo, runtime, &plan, PgConnectionPurpose::Write)?,
                control: build_lane_repo(conninfo, runtime, &plan, PgConnectionPurpose::Control)?,
                lease: build_lane_repo(conninfo, runtime, &plan, PgConnectionPurpose::Lease)?,
            }
        } else {
            DbRepoLaneStorage::Shared(DbRepo::with_runtime(conninfo, runtime)?)
        };

        Ok(Self {
            storage,
            plan,
            opt_in_enabled,
            health: PgEndpointHealthRegistry::default(),
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

    fn observability_repositories(&self) -> Vec<(&'static str, DbRepo)> {
        match &self.storage {
            DbRepoLaneStorage::Shared(repo) => vec![("shared", repo.clone())],
            DbRepoLaneStorage::Dedicated {
                read,
                write,
                control,
                lease,
            } => vec![
                (PgConnectionPurpose::Read.as_str(), read.clone()),
                (PgConnectionPurpose::Write.as_str(), write.clone()),
                (PgConnectionPurpose::Control.as_str(), control.clone()),
                (PgConnectionPurpose::Lease.as_str(), lease.clone()),
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
            legacy_dsn_only: true,
            routing_enabled: false,
        }
    }

    pub fn probe_health(&self, conninfo: &str) -> Result<PgEndpointHealthSnapshot, String> {
        let result = postgres_endpoint_probe(conninfo);
        self.health
            .record_probe_result(LEGACY_DSN_AUTHORITY, PgEndpointRole::Unknown, result)
    }

    pub fn record_connection_failure(
        &self,
        error: &str,
    ) -> Result<PgEndpointHealthSnapshot, String> {
        self.health
            .record_failure(LEGACY_DSN_AUTHORITY, PgEndpointRole::Unknown, error)
    }
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
        "FOD PostgreSQL lanes: opt_in_enabled={} dedicated_lanes_active={} mode={} total={} read={} write={} control={} lease={} legacy_dsn_only={} routing_enabled={}",
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
    );

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

    let observability_repositories = lanes.observability_repositories();
    let control_repo = lanes.repo_for(PgConnectionPurpose::Control);
    log_postgres_diagnostics(control_repo);
    if let Err(err) =
        validate_and_log_postgres_requirements(control_repo, diagnostics.total_limit as u64)
    {
        let _ = lanes.record_connection_failure(&err);
        log_lane_observability("startup-failed", &observability_repositories);
        return Err(format!(
            "PostgreSQL runtime requirements validation failed: {err}"
        ));
    }
    log::debug!("FOD reading startup snapshot through control lane");
    let snapshot = match control_repo.startup_snapshot() {
        Ok(snapshot) => snapshot,
        Err(err) => {
            let _ = lanes.record_connection_failure(&err);
            log_lane_observability("startup-failed", &observability_repositories);
            return Err(format!("failed to read startup snapshot: {err}"));
        }
    };
    log::debug!("FOD startup snapshot={:?}", snapshot);
    log_lane_observability("post-startup", &observability_repositories);
    let settings =
        crate::startup::FodFuseSettings::from_runtime(runtime, &snapshot, requested_readonly);
    let (mount_repo, keepalive) = lanes.into_mount_repo();
    log::debug!(
        "FOD PostgreSQL non-write lane keepalive count={}",
        keepalive.active_lane_count()
    );
    let result = crate::startup::mount_fuse(mount_repo, runtime, settings, mountpoint, &snapshot);
    log_lane_observability("post-mount", &observability_repositories);
    drop(keepalive);
    result
}

fn log_lane_observability(stage: &str, repositories: &[(&str, DbRepo)]) {
    match current_process_rss_bytes() {
        Ok(process_rss_bytes) => log::info!(
            "FOD PostgreSQL lane process observability: stage={} process_rss_bytes={}",
            stage,
            process_rss_bytes
        ),
        Err(err) => log::warn!(
            "FOD PostgreSQL lane process observability unavailable: stage={} error={}",
            stage,
            err
        ),
    }

    for (lane, repo) in repositories {
        match repo.observability_snapshot() {
            Ok(snapshot) => {
                let pool = snapshot.pool;
                log::info!(
                    "FOD PostgreSQL lane observability: stage={} lane={} connection_limit={} live_connections={} idle_connections={} idle_write_connections={} idle_control_connections={} active_connections={} queued_acquisitions={} peak_active_connections={} peak_queued_acquisitions={} acquisition_count={} acquisition_wait_micros_total={} acquisition_wait_micros_max={} connection_create_count={} connection_create_failures={} connection_create_micros_total={} connection_create_micros_max={} operation_count={} operation_failures={} operation_micros_total={} operation_micros_max={} replay_count={} persist_buffer_chunk_blocks={} persist_copy_send_buffer_bytes={} routing_enabled=false",
                    stage,
                    lane,
                    pool.connection_limit,
                    pool.live_connections,
                    pool.idle_connections(),
                    pool.idle_write_connections,
                    pool.idle_control_connections,
                    pool.active_connections,
                    pool.queued_acquisitions,
                    pool.peak_active_connections,
                    pool.peak_queued_acquisitions,
                    pool.acquisition_count,
                    pool.acquisition_wait_micros_total,
                    pool.acquisition_wait_micros_max,
                    pool.connection_create_count,
                    pool.connection_create_failures,
                    pool.connection_create_micros_total,
                    pool.connection_create_micros_max,
                    pool.operation_count,
                    pool.operation_failures,
                    pool.operation_micros_total,
                    pool.operation_micros_max,
                    pool.replay_count,
                    snapshot.persist_buffer_chunk_blocks,
                    snapshot.persist_copy_send_buffer_bytes,
                );
            }
            Err(err) => log::warn!(
                "FOD PostgreSQL lane observability unavailable: stage={} lane={} error={}",
                stage,
                lane,
                err
            ),
        }
    }
}

fn current_process_rss_bytes() -> Result<u64, String> {
    let status = std::fs::read_to_string("/proc/self/status")
        .map_err(|err| format!("unable to read /proc/self/status: {err}"))?;
    let line = status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .ok_or_else(|| "VmRSS is missing from /proc/self/status".to_string())?;
    let mut fields = line.split_whitespace();
    let _label = fields.next();
    let kib = fields
        .next()
        .ok_or_else(|| "VmRSS value is missing".to_string())?
        .parse::<u64>()
        .map_err(|err| format!("invalid VmRSS value: {err}"))?;
    let unit = fields
        .next()
        .ok_or_else(|| "VmRSS unit is missing".to_string())?;
    if unit != "kB" {
        return Err(format!("unsupported VmRSS unit: {unit}"));
    }
    kib.checked_mul(1024)
        .ok_or_else(|| "VmRSS byte value overflowed".to_string())
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
) -> Result<DbRepo, String> {
    let mut lane_runtime = runtime.clone();
    lane_runtime.pool_max_connections = plan.limit_for(purpose) as u64;
    DbRepo::with_runtime(conninfo, &lane_runtime)
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
