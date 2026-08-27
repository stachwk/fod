// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_monitor::SharedMonitorSessionStats;
use fod_rust_runtime::ini_config::{
    load_config_parser, resolve_pg_endpoint_config, PgEndpointRole,
};
use fod_rust_runtime::FOD_SCHEMA_NAME;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::env;
use std::ffi::{c_char, c_int, CStr, CString};
use std::ptr;

const CONNECTION_OK: c_int = 0;
const PGRES_TUPLES_OK: c_int = 2;
const STALE_SAMPLE_SECONDS_DEFAULT: u64 = 15;
const STALE_SAMPLE_INTERVAL_MULTIPLIER: u64 = 3;
const MONITOR_DSN_ENV: &str = "FOD_MONITOR_DSN";

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
    fn PQfinish(conn: *mut PGconn);
    fn PQexec(conn: *mut PGconn, command: *const c_char) -> *mut PGresult;
    fn PQresultStatus(res: *const PGresult) -> c_int;
    fn PQresultErrorMessage(res: *const PGresult) -> *const c_char;
    fn PQntuples(res: *const PGresult) -> c_int;
    fn PQnfields(res: *const PGresult) -> c_int;
    fn PQgetisnull(res: *const PGresult, row_number: c_int, column_number: c_int) -> c_int;
    fn PQgetvalue(res: *const PGresult, row_number: c_int, column_number: c_int) -> *const c_char;
    fn PQclear(res: *mut PGresult);
}

struct PgConnection {
    raw: *mut PGconn,
    configured_authority: String,
}

impl Drop for PgConnection {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { PQfinish(self.raw) };
            self.raw = ptr::null_mut();
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterSessionSnapshot {
    pub session_id: u64,
    pub host_name: String,
    pub mountpoint: String,
    pub mount_mode: String,
    pub lock_backend: String,
    pub pid: u64,
    pub heartbeat_age_seconds: u64,
    pub started_age_seconds: u64,
    pub last_write_age_seconds: Option<u64>,
    pub fod_version: Option<String>,
    pub sample_seq: Option<u64>,
    pub sample_age_seconds: Option<u64>,
    pub sample_epoch_micros: Option<u64>,
    pub stats: Option<SharedMonitorSessionStats>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterSnapshot {
    pub source_authority: String,
    pub source_database: String,
    pub source_role: String,
    pub sessions: Vec<ClusterSessionSnapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ClusterSummary {
    pub active_sessions: u64,
    pub active_hosts: u64,
    pub telemetry_sessions: u64,
    pub stale_telemetry_sessions: u64,
    pub read_completed_tasks: u64,
    pub read_completed_bytes: u64,
    pub read_average_callback_bytes: u64,
    pub write_completed_tasks: u64,
    pub write_completed_bytes: u64,
    pub write_average_callback_bytes: u64,
    pub copy_completed_tasks: u64,
    pub copy_completed_bytes: u64,
    pub copy_average_callback_bytes: u64,
    pub task_failures: u64,
    pub database_operations: u64,
    pub database_operation_failures: u64,
    pub database_operations_per_completed_task_milli_proxy: u64,
    pub database_operations_per_read_task_milli_proxy: u64,
    pub database_operations_per_write_task_milli_proxy: u64,
    pub persist_operations: u64,
    pub persist_input_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterJsonSource<'a> {
    pub authority: &'a str,
    pub database: &'a str,
    pub role: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterJsonSnapshot<'a> {
    pub source: ClusterJsonSource<'a>,
    pub summary: ClusterSummary,
    pub sessions: &'a [ClusterSessionSnapshot],
}

fn c_text(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .trim()
        .to_string()
}

fn connection_error(conn: *const PGconn) -> String {
    if conn.is_null() {
        return "libpq returned a null connection".to_string();
    }
    unsafe { c_text(PQerrorMessage(conn)) }
}

fn result_error(result: *const PGresult) -> String {
    if result.is_null() {
        return "PostgreSQL returned a null result".to_string();
    }
    unsafe { c_text(PQresultErrorMessage(result)) }
}

fn quote_conninfo_value(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

fn build_conninfo(params: &HashMap<String, String>) -> String {
    let mut items = params.iter().collect::<Vec<_>>();
    items.sort_by(|left, right| left.0.cmp(right.0));
    items
        .into_iter()
        .map(|(key, value)| format!("{key}={}", quote_conninfo_value(value)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn connection_candidates() -> Result<Vec<(String, String)>, String> {
    if let Ok(dsn) = env::var(MONITOR_DSN_ENV) {
        if !dsn.trim().is_empty() {
            return Ok(vec![(MONITOR_DSN_ENV.to_string(), dsn)]);
        }
    }
    let (config, path) = load_config_parser(None)?;
    let database = config
        .section("database")
        .ok_or_else(|| format!("missing [database] section in {}", path.display()))?;
    let endpoint_config = resolve_pg_endpoint_config(database)?;
    let mut endpoints = endpoint_config.endpoints;
    endpoints.sort_by_key(|endpoint| match endpoint.role {
        PgEndpointRole::Primary => 0_u8,
        PgEndpointRole::Unknown => 1_u8,
        PgEndpointRole::Replica => 2_u8,
    });
    let mut base = HashMap::new();
    for key in [
        "dbname",
        "user",
        "password",
        "sslmode",
        "sslrootcert",
        "sslcert",
        "sslkey",
        "connect_timeout",
    ] {
        if let Some(value) = database.get(key) {
            if !value.trim().is_empty() {
                base.insert(key.to_string(), value.clone());
            }
        }
    }
    base.insert("application_name".to_string(), "fod-monitor".to_string());
    Ok(endpoints
        .into_iter()
        .map(|endpoint| {
            let mut params = base.clone();
            params.insert("host".to_string(), endpoint.host.clone());
            params.insert("port".to_string(), endpoint.port.to_string());
            (endpoint.authority(), build_conninfo(&params))
        })
        .collect())
}

fn connect_cluster() -> Result<PgConnection, String> {
    let candidates = connection_candidates()?;
    if candidates.is_empty() {
        return Err("no PostgreSQL endpoint configured for fod-monitor".to_string());
    }
    let mut errors = Vec::new();
    for (authority, conninfo) in candidates {
        let conninfo = CString::new(conninfo)
            .map_err(|_| "PostgreSQL connection string contains NUL byte".to_string())?;
        let conn = unsafe { PQconnectdb(conninfo.as_ptr()) };
        if conn.is_null() {
            errors.push(format!("{authority}: null connection"));
            continue;
        }
        if unsafe { PQstatus(conn) } == CONNECTION_OK {
            return Ok(PgConnection {
                raw: conn,
                configured_authority: authority,
            });
        }
        errors.push(format!("{authority}: {}", connection_error(conn)));
        unsafe { PQfinish(conn) };
    }
    Err(format!(
        "unable to connect fod-monitor to PostgreSQL: {}",
        errors.join("; ")
    ))
}

impl PgConnection {
    fn query_scalar(&mut self, sql: &str) -> Result<String, String> {
        let sql = CString::new(sql).map_err(|_| "SQL contains NUL byte".to_string())?;
        let result = unsafe { PQexec(self.raw, sql.as_ptr()) };
        if result.is_null() {
            return Err(connection_error(self.raw));
        }
        if unsafe { PQresultStatus(result) } != PGRES_TUPLES_OK {
            let err = result_error(result);
            unsafe { PQclear(result) };
            return Err(err);
        }
        if unsafe { PQntuples(result) } < 1 || unsafe { PQnfields(result) } < 1 {
            unsafe { PQclear(result) };
            return Err("PostgreSQL scalar query returned no value".to_string());
        }
        let value = unsafe { c_text(PQgetvalue(result, 0, 0)) };
        unsafe { PQclear(result) };
        Ok(value)
    }

    fn actual_authority(&mut self) -> String {
        self.query_scalar("SELECT COALESCE(host(inet_server_addr()), 'local') || ':' || COALESCE(inet_server_port()::text, '0')")
            .unwrap_or_else(|_| self.configured_authority.clone())
    }

    fn source_role(&mut self) -> String {
        self.query_scalar("SELECT CASE WHEN pg_is_in_recovery() THEN 'replica' WHEN current_setting('transaction_read_only')::boolean THEN 'primary-read-only' ELSE 'primary-writable' END")
            .unwrap_or_else(|_| "unknown".to_string())
    }

    fn query_sessions(&mut self) -> Result<Vec<ClusterSessionSnapshot>, String> {
        let sql = format!(
            r#"
            SELECT
                cs.session_id::text,
                cs.host_name,
                cs.mountpoint,
                cs.mount_mode,
                cs.lock_backend,
                cs.pid::text,
                GREATEST(0, EXTRACT(EPOCH FROM ((CURRENT_TIMESTAMP AT TIME ZONE 'UTC') - cs.heartbeat_at))::bigint)::text,
                GREATEST(0, EXTRACT(EPOCH FROM ((CURRENT_TIMESTAMP AT TIME ZONE 'UTC') - cs.started_at))::bigint)::text,
                CASE WHEN cs.last_write_at IS NULL THEN NULL ELSE GREATEST(0, EXTRACT(EPOCH FROM ((CURRENT_TIMESTAMP AT TIME ZONE 'UTC') - cs.last_write_at))::bigint)::text END,
                ms.fod_version,
                ms.sample_seq::text,
                CASE WHEN ms.sampled_at IS NULL THEN NULL ELSE GREATEST(0, EXTRACT(EPOCH FROM ((CURRENT_TIMESTAMP AT TIME ZONE 'UTC') - ms.sampled_at))::bigint)::text END,
                (EXTRACT(EPOCH FROM ms.sampled_at) * 1000000)::bigint::text,
                ms.payload_json::text
            FROM {schema}.client_sessions AS cs
            LEFT JOIN {schema}.monitor_session_stats AS ms ON ms.session_id = cs.session_id
            WHERE cs.lease_expires_at > (CURRENT_TIMESTAMP AT TIME ZONE 'UTC')
            ORDER BY cs.host_name, cs.mountpoint, cs.session_id
        "#,
            schema = FOD_SCHEMA_NAME
        );
        let sql = CString::new(sql).map_err(|_| "SQL contains NUL byte".to_string())?;
        let result = unsafe { PQexec(self.raw, sql.as_ptr()) };
        if result.is_null() {
            return Err(connection_error(self.raw));
        }
        if unsafe { PQresultStatus(result) } != PGRES_TUPLES_OK {
            let err = result_error(result);
            unsafe { PQclear(result) };
            return Err(err);
        }
        let rows = unsafe { PQntuples(result) };
        let fields = unsafe { PQnfields(result) };
        if fields != 14 {
            unsafe { PQclear(result) };
            return Err(format!("unexpected cluster query field count: {fields}"));
        }
        let mut sessions = Vec::with_capacity(rows.max(0) as usize);
        for row in 0..rows {
            let field = |column: c_int| -> Option<String> {
                if unsafe { PQgetisnull(result, row, column) } != 0 {
                    None
                } else {
                    Some(unsafe { c_text(PQgetvalue(result, row, column)) })
                }
            };
            let parse_req = |column: c_int, name: &str| -> Result<u64, String> {
                field(column)
                    .ok_or_else(|| format!("missing {name}"))?
                    .parse::<u64>()
                    .map_err(|err| format!("invalid {name}: {err}"))
            };
            let parse_opt = |column: c_int, name: &str| -> Result<Option<u64>, String> {
                field(column)
                    .map(|value| {
                        value
                            .parse::<u64>()
                            .map_err(|err| format!("invalid {name}: {err}"))
                    })
                    .transpose()
            };
            let stats = match field(13) {
                Some(payload) if !payload.trim().is_empty() => {
                    Some(SharedMonitorSessionStats::from_json(&payload)?)
                }
                _ => None,
            };
            sessions.push(ClusterSessionSnapshot {
                session_id: parse_req(0, "session_id")?,
                host_name: field(1).unwrap_or_default(),
                mountpoint: field(2).unwrap_or_default(),
                mount_mode: field(3).unwrap_or_default(),
                lock_backend: field(4).unwrap_or_default(),
                pid: parse_req(5, "pid")?,
                heartbeat_age_seconds: parse_req(6, "heartbeat_age_seconds")?,
                started_age_seconds: parse_req(7, "started_age_seconds")?,
                last_write_age_seconds: parse_opt(8, "last_write_age_seconds")?,
                fod_version: field(9),
                sample_seq: parse_opt(10, "sample_seq")?,
                sample_age_seconds: parse_opt(11, "sample_age_seconds")?,
                sample_epoch_micros: parse_opt(12, "sample_epoch_micros")?,
                stats,
            });
        }
        unsafe { PQclear(result) };
        Ok(sessions)
    }
}

pub fn load_cluster_snapshot() -> Result<ClusterSnapshot, String> {
    let mut connection = connect_cluster()?;
    let source_authority = connection.actual_authority();
    let source_database = connection
        .query_scalar("SELECT current_database()")
        .unwrap_or_else(|_| "unknown".to_string());
    let source_role = connection.source_role();
    let sessions = connection.query_sessions()?;
    Ok(ClusterSnapshot {
        source_authority,
        source_database,
        source_role,
        sessions,
    })
}

fn stale_sample_threshold_seconds(publish_interval_millis: Option<u64>) -> u64 {
    let Some(publish_interval_millis) = publish_interval_millis.filter(|value| *value > 0) else {
        return STALE_SAMPLE_SECONDS_DEFAULT;
    };
    let publish_interval_seconds = publish_interval_millis.saturating_add(999) / 1_000;
    STALE_SAMPLE_SECONDS_DEFAULT
        .max(publish_interval_seconds.saturating_mul(STALE_SAMPLE_INTERVAL_MULTIPLIER))
}

fn previous_session(
    previous: Option<&ClusterSnapshot>,
    session_id: u64,
) -> Option<&ClusterSessionSnapshot> {
    previous?
        .sessions
        .iter()
        .find(|session| session.session_id == session_id)
}

fn rate(
    current: &ClusterSessionSnapshot,
    previous: Option<&ClusterSessionSnapshot>,
    selector: impl Fn(&SharedMonitorSessionStats) -> u64,
) -> Option<u64> {
    let previous = previous?;
    let current_epoch = current.sample_epoch_micros?;
    let previous_epoch = previous.sample_epoch_micros?;
    if current_epoch <= previous_epoch {
        return None;
    }
    let delta_us = current_epoch - previous_epoch;
    let current_value = selector(current.stats.as_ref()?);
    let previous_value = selector(previous.stats.as_ref()?);
    Some(
        current_value
            .saturating_sub(previous_value)
            .saturating_mul(1_000_000)
            / delta_us,
    )
}

fn opt(value: Option<u64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn avg_bytes(bytes: u64, tasks: u64) -> u64 {
    if tasks == 0 {
        0
    } else {
        bytes.checked_div(tasks).unwrap_or(0)
    }
}

fn per_task_milli(numerator: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        0
    } else {
        numerator
            .saturating_mul(1_000)
            .checked_div(denominator)
            .unwrap_or(0)
    }
}

pub fn cluster_summary(snapshot: &ClusterSnapshot) -> ClusterSummary {
    let mut hosts = BTreeSet::new();
    let mut summary = ClusterSummary {
        active_sessions: snapshot.sessions.len() as u64,
        ..ClusterSummary::default()
    };
    for session in &snapshot.sessions {
        hosts.insert(session.host_name.as_str());
        if session.stats.is_some() {
            summary.telemetry_sessions = summary.telemetry_sessions.saturating_add(1);
        }
        let stale_threshold_seconds = stale_sample_threshold_seconds(
            session
                .stats
                .as_ref()
                .map(|stats| stats.publish_interval_millis),
        );
        if session
            .sample_age_seconds
            .is_some_and(|age| age > stale_threshold_seconds)
        {
            summary.stale_telemetry_sessions = summary.stale_telemetry_sessions.saturating_add(1);
        }
        if let Some(stats) = &session.stats {
            summary.read_completed_tasks = summary
                .read_completed_tasks
                .saturating_add(stats.read.completed_tasks);
            summary.read_completed_bytes = summary
                .read_completed_bytes
                .saturating_add(stats.read.completed_bytes);
            summary.write_completed_tasks = summary
                .write_completed_tasks
                .saturating_add(stats.write.completed_tasks);
            summary.write_completed_bytes = summary
                .write_completed_bytes
                .saturating_add(stats.write.completed_bytes);
            summary.copy_completed_tasks = summary
                .copy_completed_tasks
                .saturating_add(stats.copy.completed_tasks);
            summary.copy_completed_bytes = summary
                .copy_completed_bytes
                .saturating_add(stats.copy.completed_bytes);
            summary.task_failures = summary
                .task_failures
                .saturating_add(stats.read.failed_tasks)
                .saturating_add(stats.write.failed_tasks)
                .saturating_add(stats.copy.failed_tasks);
            summary.database_operations = summary
                .database_operations
                .saturating_add(stats.database.operation_count);
            summary.database_operation_failures = summary
                .database_operation_failures
                .saturating_add(stats.database.operation_failures);
            summary.persist_operations = summary
                .persist_operations
                .saturating_add(stats.persistence.persist_operation_count);
            summary.persist_input_bytes = summary
                .persist_input_bytes
                .saturating_add(stats.persistence.persist_input_bytes_total);
        }
    }
    summary.active_hosts = hosts.len() as u64;
    summary.read_average_callback_bytes =
        avg_bytes(summary.read_completed_bytes, summary.read_completed_tasks);
    summary.write_average_callback_bytes =
        avg_bytes(summary.write_completed_bytes, summary.write_completed_tasks);
    summary.copy_average_callback_bytes =
        avg_bytes(summary.copy_completed_bytes, summary.copy_completed_tasks);
    let completed_tasks = summary
        .read_completed_tasks
        .saturating_add(summary.write_completed_tasks)
        .saturating_add(summary.copy_completed_tasks);
    summary.database_operations_per_completed_task_milli_proxy =
        per_task_milli(summary.database_operations, completed_tasks);
    summary.database_operations_per_read_task_milli_proxy =
        per_task_milli(summary.database_operations, summary.read_completed_tasks);
    summary.database_operations_per_write_task_milli_proxy =
        per_task_milli(summary.database_operations, summary.write_completed_tasks);
    summary
}

pub fn cluster_json_snapshot(snapshot: &ClusterSnapshot) -> ClusterJsonSnapshot<'_> {
    ClusterJsonSnapshot {
        source: ClusterJsonSource {
            authority: &snapshot.source_authority,
            database: &snapshot.source_database,
            role: &snapshot.source_role,
        },
        summary: cluster_summary(snapshot),
        sessions: &snapshot.sessions,
    }
}
fn text(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}
fn compact(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    format!(
        "{}...",
        value
            .chars()
            .take(max.saturating_sub(3))
            .collect::<String>()
    )
}

pub fn print_cluster_snapshot(snapshot: &ClusterSnapshot, previous: Option<&ClusterSnapshot>) {
    let summary = cluster_summary(snapshot);
    println!("Cluster:");
    println!("  source_authority={}", snapshot.source_authority);
    println!("  source_database={}", snapshot.source_database);
    println!("  source_role={}", snapshot.source_role);
    println!("  active_sessions={}", summary.active_sessions);
    println!("  active_hosts={}", summary.active_hosts);
    println!("  telemetry_sessions={}", summary.telemetry_sessions);
    println!(
        "  stale_telemetry_sessions={}",
        summary.stale_telemetry_sessions
    );
    println!("  read_completed_tasks={}", summary.read_completed_tasks);
    println!("  read_completed_bytes={}", summary.read_completed_bytes);
    println!(
        "  read_average_callback_bytes={}",
        summary.read_average_callback_bytes
    );
    println!("  write_completed_tasks={}", summary.write_completed_tasks);
    println!("  write_completed_bytes={}", summary.write_completed_bytes);
    println!(
        "  write_average_callback_bytes={}",
        summary.write_average_callback_bytes
    );
    println!("  copy_completed_tasks={}", summary.copy_completed_tasks);
    println!("  copy_completed_bytes={}", summary.copy_completed_bytes);
    println!(
        "  copy_average_callback_bytes={}",
        summary.copy_average_callback_bytes
    );
    println!("  task_failures={}", summary.task_failures);
    println!("  database_operations={}", summary.database_operations);
    println!(
        "  database_operation_failures={}",
        summary.database_operation_failures
    );
    println!(
        "  database_operations_per_completed_task_milli_proxy={}",
        summary.database_operations_per_completed_task_milli_proxy
    );
    println!(
        "  database_operations_per_read_task_milli_proxy={}",
        summary.database_operations_per_read_task_milli_proxy
    );
    println!(
        "  database_operations_per_write_task_milli_proxy={}",
        summary.database_operations_per_write_task_milli_proxy
    );
    println!("  persist_operations={}", summary.persist_operations);
    println!("  persist_input_bytes={}", summary.persist_input_bytes);
    if snapshot.sessions.is_empty() {
        println!("No active shared FOD sessions detected.");
        return;
    }
    println!("HOST\tMOUNT\tMODE\tSRC_ROLE\tWAL_LAG_B\tLOCK\tPID\tVER\tHB_S\tSAMPLE_S\tREAD_N\tREAD_B\tREAD_BPS\tREAD_AVG_B\tWRITE_N\tWRITE_B\tWRITE_BPS\tWRITE_AVG_B\tCOPY_N\tCOPY_B\tCOPY_BPS\tCOPY_AVG_B\tDB_OPS\tDB_ERR\tDB_READ_M\tDB_WRITE_M\tPERSIST_N");
    for session in &snapshot.sessions {
        let stats = session.stats.as_ref();
        let prev = previous_session(previous, session.session_id);
        let source_role = stats
            .map(|v| v.source.data_source_role.as_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("-");
        let wal_lag = stats.and_then(|v| v.source.wal_replay_lag_bytes);
        let db_ops = stats.map(|v| v.database.operation_count).unwrap_or(0);
        let read_tasks = stats.map(|v| v.read.completed_tasks).unwrap_or(0);
        let write_tasks = stats.map(|v| v.write.completed_tasks).unwrap_or(0);
        let copy_tasks = stats.map(|v| v.copy.completed_tasks).unwrap_or(0);
        let read_bytes = stats.map(|v| v.read.completed_bytes).unwrap_or(0);
        let write_bytes = stats.map(|v| v.write.completed_bytes).unwrap_or(0);
        let copy_bytes = stats.map(|v| v.copy.completed_bytes).unwrap_or(0);
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            compact(&session.host_name, 24),
            compact(&session.mountpoint, 36),
            session.mount_mode,
            source_role,
            opt(wal_lag),
            session.lock_backend,
            session.pid,
            text(session.fod_version.as_deref()),
            session.heartbeat_age_seconds,
            opt(session.sample_age_seconds),
            read_tasks,
            read_bytes,
            opt(rate(session, prev, |v| v.read.completed_bytes)),
            avg_bytes(read_bytes, read_tasks),
            write_tasks,
            write_bytes,
            opt(rate(session, prev, |v| v.write.completed_bytes)),
            avg_bytes(write_bytes, write_tasks),
            copy_tasks,
            copy_bytes,
            opt(rate(session, prev, |v| v.copy.completed_bytes)),
            avg_bytes(copy_bytes, copy_tasks),
            db_ops,
            stats.map(|v| v.database.operation_failures).unwrap_or(0),
            per_task_milli(db_ops, read_tasks),
            per_task_milli(db_ops, write_tasks),
            stats
                .map(|v| v.persistence.persist_operation_count)
                .unwrap_or(0)
        );
    }
}

pub fn print_cluster_details(snapshot: &ClusterSnapshot) {
    println!("Cluster session details:");
    for session in &snapshot.sessions {
        println!("  session_id={} host={} mount={} pid={} uptime_s={} last_write_age_s={} sample_seq={} sample_age_s={} publish_interval_ms={}",
            session.session_id, session.host_name, session.mountpoint, session.pid, session.started_age_seconds,
            opt(session.last_write_age_seconds), opt(session.sample_seq), opt(session.sample_age_seconds),
            session.stats.as_ref().map(|stats| stats.publish_interval_millis).unwrap_or(0));
        if let Some(stats) = &session.stats {
            println!(
                "    source role={} transaction_read_only={} wal_replay_lag_bytes={}",
                text(
                    (!stats.source.data_source_role.is_empty())
                        .then_some(stats.source.data_source_role.as_str())
                ),
                stats.source.data_source_transaction_read_only,
                opt(stats.source.wal_replay_lag_bytes)
            );
            println!("    db authority={} live={} active={} queued={} failovers={} connection_failures={} replica_reads={} primary_fallbacks={}",
                stats.database.active_authority, stats.database.live_connections, stats.database.active_connections,
                stats.database.queued_acquisitions, stats.database.failover_count, stats.database.connection_failures,
                stats.database.replica_reads, stats.database.primary_read_fallbacks);
            println!("    persistence in_flight={} queued={} backpressure={} ops={} failures={} bytes={} rows={} micros={}",
                stats.persistence.in_flight_bytes, stats.persistence.queued_bytes, stats.persistence.backpressure_events,
                stats.persistence.persist_operation_count, stats.persistence.persist_operation_failures,
                stats.persistence.persist_input_bytes_total, stats.persistence.persist_input_rows_total, stats.persistence.persist_micros_total);
            println!("    timings fuse_read_us={} fuse_write_us={} fetch_us={} persist_us={} update_write_buffer_us={} flush_write_state_us={}",
                stats.timings.fuse_read_total_us, stats.timings.fuse_write_total_us, stats.timings.repo_fetch_block_range_us,
                stats.timings.repo_persist_blocks_us, stats.timings.update_write_buffer_us, stats.timings.flush_write_state_us);
        } else {
            println!("    telemetry=not-yet-published");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{cluster_json_snapshot, cluster_summary, stale_sample_threshold_seconds};
    use crate::cluster::{ClusterSessionSnapshot, ClusterSnapshot};
    use fod_rust_monitor::SharedMonitorSessionStats;

    #[test]
    fn stale_threshold_defaults_to_fifteen_seconds() {
        assert_eq!(stale_sample_threshold_seconds(None), 15);
        assert_eq!(stale_sample_threshold_seconds(Some(0)), 15);
        assert_eq!(stale_sample_threshold_seconds(Some(500)), 15);
        assert_eq!(stale_sample_threshold_seconds(Some(5_000)), 15);
    }

    #[test]
    fn stale_threshold_scales_with_slow_publish_interval() {
        assert_eq!(stale_sample_threshold_seconds(Some(20_000)), 60);
        assert_eq!(stale_sample_threshold_seconds(Some(60_000)), 180);
    }

    #[test]
    fn cluster_summary_reports_efficiency_indicators() {
        let mut stats = SharedMonitorSessionStats {
            publish_interval_millis: 5_000,
            ..SharedMonitorSessionStats::default()
        };
        stats.read.completed_tasks = 4;
        stats.read.completed_bytes = 16_384;
        stats.write.completed_tasks = 2;
        stats.write.completed_bytes = 8_192;
        stats.database.operation_count = 18;
        stats.persistence.persist_operation_count = 2;
        stats.persistence.persist_input_bytes_total = 8_192;
        let snapshot = ClusterSnapshot {
            source_authority: "127.0.0.1:5432".to_string(),
            source_database: "foddbname".to_string(),
            source_role: "primary-writable".to_string(),
            sessions: vec![ClusterSessionSnapshot {
                session_id: 1,
                host_name: "host-a".to_string(),
                mountpoint: "/mnt/fod".to_string(),
                mount_mode: "primary".to_string(),
                lock_backend: "postgres_lease".to_string(),
                pid: 123,
                heartbeat_age_seconds: 1,
                started_age_seconds: 2,
                last_write_age_seconds: Some(1),
                fod_version: Some("3.3.5-test".to_string()),
                sample_seq: Some(7),
                sample_age_seconds: Some(1),
                sample_epoch_micros: Some(10),
                stats: Some(stats),
            }],
        };

        let summary = cluster_summary(&snapshot);
        assert_eq!(summary.active_sessions, 1);
        assert_eq!(summary.active_hosts, 1);
        assert_eq!(summary.read_average_callback_bytes, 4096);
        assert_eq!(summary.write_average_callback_bytes, 4096);
        assert_eq!(summary.database_operations_per_read_task_milli_proxy, 4500);
        assert_eq!(summary.database_operations_per_write_task_milli_proxy, 9000);
        assert_eq!(summary.persist_input_bytes, 8192);

        let json = cluster_json_snapshot(&snapshot);
        assert_eq!(json.source.authority, "127.0.0.1:5432");
        assert_eq!(json.summary.read_completed_tasks, 4);
    }
}
