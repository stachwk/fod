// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use crate::crc32_bytes;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use fod_rust_monitor::{
    DbRepoConnectionRoutingSnapshot, DbRepoObservabilitySnapshot, DbRepoPayloadBudgetPermit,
    DbRepoPayloadObservability, DbRepoPoolObservabilitySnapshot,
    DbRepoTransactionAdmissionSnapshot, LaneObservabilitySource, PostgresPressureSnapshot,
};
use fod_rust_runtime::{
    duration_to_micros, env_var_truthy_with_legacy_alias, postgres_session_setup_sql,
    request_token as generate_request_token, DataObjectSwapCleanup, PersistBlockTransport,
    PostgresRuntimeRequirements, PostgresVersionDiagnostics, RuntimeConfig, RuntimePayloadSettings,
    RuntimeStorageSettings, RuntimeTransactionSettings, FOD_SCHEMA_NAME, FOD_SEARCH_PATH,
    MIN_POSTGRES_SERVER_VERSION_NUM, POSTGRES_REQUIREMENT_SETTINGS_SQL,
};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::convert::TryFrom;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::Read;
use std::os::raw::{c_char, c_int, c_uint};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[repr(C)]
struct PGconn {
    _private: [u8; 0],
}

#[repr(C)]
struct PGresult {
    _private: [u8; 0],
}

const CONNECTION_OK: c_int = 0;
const PGRES_TUPLES_OK: c_int = 2;
const PGRES_COMMAND_OK: c_int = 1;
const PGRES_COPY_IN: c_int = 4;
const PG_DIAG_SQLSTATE: c_int = 67;
const COPY_BINARY_SIGNATURE: &[u8] = b"PGCOPY\n\xff\r\n\0";
const DEFAULT_PERSIST_COPY_SEND_BUFFER_BYTES: usize = 1024 * 1024;
const PERSIST_BLOCK_STAGE_TABLE: &str = "fod_persist_block_stage";
const INDEX_SOURCES_STAGE_TABLE: &str = "index_sources_stage";
const INDEX_SCAN_RUNS_STAGE_TABLE: &str = "index_scan_runs_stage";
const INDEX_FILES_STAGE_TABLE: &str = "index_files_stage";
const INDEX_FILE_HASHES_STAGE_TABLE: &str = "index_file_hashes_stage";
const INDEX_IMPORT_PLANS_STAGE_TABLE: &str = "index_import_plans_stage";
const INDEX_IMPORT_PLAN_ENTRIES_STAGE_TABLE: &str = "index_import_plan_entries_stage";
const REPLAYABLE_SQL_ERROR_PREFIX: &str = "__FOD_REPLAYABLE_SQL_ERROR__: ";
const UNIQUE_VIOLATION_ERROR_PREFIX: &str = "SQLSTATE 23505:";
const REPLICA_SCORE_UNKNOWN_LATENCY_MICROS: u64 = 5_000;
const REPLICA_SCORE_LATENCY_DIVISOR: u64 = 100;
const REPLICA_SCORE_LAG_DIVISOR_BYTES: u64 = 4_096;
const REPLICA_SCORE_FAILURE_PENALTY: u64 = 10_000;
const REPLICA_SCORE_HYSTERESIS_MARGIN: u64 = 25;
const REPLICA_CIRCUIT_BREAKER_FAILURE_THRESHOLD: u64 = 2;
const REPLICA_CIRCUIT_BREAKER_COOLDOWN_MS: u64 = 5_000;
const REPLICA_POOL_PRESSURE_MARGIN_MILLI: u64 = 250;

static NEXT_LOCK_SESSION_ID: AtomicI64 = AtomicI64::new(-1);

fn fod_profile_io_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| env_var_truthy_with_legacy_alias("FOD_PROFILE_IO", false))
}

fn fod_sql_label(sql: &CString) -> String {
    // Skraca SQL do jednej linii, aby log byl czytelny
    let raw = sql.to_string_lossy();
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(160).collect()
}

fn persist_copy_send_buffer_bytes() -> usize {
    static VALUE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        std::env::var("FOD_PERSIST_COPY_SEND_BUFFER_BYTES")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_PERSIST_COPY_SEND_BUFFER_BYTES)
    })
}

#[derive(Debug, Default)]
struct DbfsIoProfileAggregate {
    count: u64,
    seconds_sum: f64,
    seconds_max: f64,
    blocks_sum: usize,
    bytes_sum: usize,
}

impl DbfsIoProfileAggregate {
    fn observe(&mut self, seconds: f64, blocks: usize, bytes: usize) {
        self.count = self.count.saturating_add(1);
        self.seconds_sum += seconds;
        if seconds > self.seconds_max {
            self.seconds_max = seconds;
        }
        self.blocks_sum = self.blocks_sum.saturating_add(blocks);
        self.bytes_sum = self.bytes_sum.saturating_add(bytes);
    }
}

#[derive(Debug, Default)]
struct StatementProfileAggregate {
    count: u64,
    failures: u64,
    micros_sum: u64,
    micros_max: u64,
    params_sum: u64,
    param_bytes_sum: u64,
    result_rows_sum: u64,
    result_bytes_sum: u64,
}

impl StatementProfileAggregate {
    fn observe(
        &mut self,
        elapsed: Duration,
        params: usize,
        param_bytes: usize,
        result_rows: u64,
        result_bytes: u64,
        failed: bool,
    ) {
        let elapsed_micros = duration_to_micros(elapsed);
        self.count = self.count.saturating_add(1);
        self.micros_sum = self.micros_sum.saturating_add(elapsed_micros);
        self.micros_max = self.micros_max.max(elapsed_micros);
        self.params_sum = self.params_sum.saturating_add(params as u64);
        self.param_bytes_sum = self.param_bytes_sum.saturating_add(param_bytes as u64);
        self.result_rows_sum = self.result_rows_sum.saturating_add(result_rows);
        self.result_bytes_sum = self.result_bytes_sum.saturating_add(result_bytes);
        if failed {
            self.failures = self.failures.saturating_add(1);
        }
    }
}

#[derive(Debug)]
struct StatementProfileSnapshot {
    label: String,
    op: &'static str,
    count: u64,
    failures: u64,
    micros_sum: u64,
    micros_max: u64,
    params_sum: u64,
    param_bytes_sum: u64,
    result_rows_sum: u64,
    result_bytes_sum: u64,
}

static FOD_PREPARED_STATEMENT_PROFILE_AGGREGATE: OnceLock<
    Mutex<BTreeMap<&'static str, StatementProfileAggregate>>,
> = OnceLock::new();

static FOD_SQL_STATEMENT_PROFILE_AGGREGATE: OnceLock<
    Mutex<BTreeMap<(String, &'static str), StatementProfileAggregate>>,
> = OnceLock::new();

fn fod_profile_prepared_statement_observe(
    statement: PreparedStatement,
    elapsed: Duration,
    params: usize,
    param_bytes: usize,
    result_rows: u64,
    result_bytes: u64,
    failed: bool,
) {
    if !fod_profile_io_enabled() {
        return;
    }
    let aggregate =
        FOD_PREPARED_STATEMENT_PROFILE_AGGREGATE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(mut guard) = aggregate.lock() {
        guard.entry(statement.name()).or_default().observe(
            elapsed,
            params,
            param_bytes,
            result_rows,
            result_bytes,
            failed,
        );
    }
}

fn fod_profile_sql_statement_observe(
    op: &'static str,
    sql_label: String,
    elapsed: Duration,
    params: usize,
    param_bytes: usize,
    result_rows: u64,
    result_bytes: u64,
    failed: bool,
) {
    if !fod_profile_io_enabled() {
        return;
    }
    let aggregate = FOD_SQL_STATEMENT_PROFILE_AGGREGATE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(mut guard) = aggregate.lock() {
        guard.entry((sql_label, op)).or_default().observe(
            elapsed,
            params,
            param_bytes,
            result_rows,
            result_bytes,
            failed,
        );
    }
}

pub fn prepared_statement_profile_snapshot_lines() -> Vec<String> {
    if !fod_profile_io_enabled() {
        return Vec::new();
    }
    let Some(aggregate) = FOD_PREPARED_STATEMENT_PROFILE_AGGREGATE.get() else {
        return Vec::new();
    };
    let Ok(guard) = aggregate.lock() else {
        return Vec::new();
    };
    let mut snapshots = guard
        .iter()
        .map(|(name, aggregate)| StatementProfileSnapshot {
            label: (*name).to_string(),
            op: "prepared",
            count: aggregate.count,
            failures: aggregate.failures,
            micros_sum: aggregate.micros_sum,
            micros_max: aggregate.micros_max,
            params_sum: aggregate.params_sum,
            param_bytes_sum: aggregate.param_bytes_sum,
            result_rows_sum: aggregate.result_rows_sum,
            result_bytes_sum: aggregate.result_bytes_sum,
        })
        .collect::<Vec<_>>();
    snapshots.sort_unstable_by(|left, right| {
        right
            .micros_sum
            .cmp(&left.micros_sum)
            .then_with(|| left.label.cmp(&right.label))
    });
    snapshots
        .into_iter()
        .map(|snapshot| {
            let avg_us = if snapshot.count == 0 {
                0
            } else {
                snapshot.micros_sum / snapshot.count
            };
            format!(
                "pg_prepared_statement name={} count={} total_us={} max_us={} avg_us={} params={} param_bytes={} result_rows={} result_bytes={} failures={}",
                snapshot.label,
                snapshot.count,
                snapshot.micros_sum,
                snapshot.micros_max,
                avg_us,
                snapshot.params_sum,
                snapshot.param_bytes_sum,
                snapshot.result_rows_sum,
                snapshot.result_bytes_sum,
                snapshot.failures
            )
        })
        .collect()
}

pub fn sql_statement_profile_snapshot_lines() -> Vec<String> {
    if !fod_profile_io_enabled() {
        return Vec::new();
    }
    let Some(aggregate) = FOD_SQL_STATEMENT_PROFILE_AGGREGATE.get() else {
        return Vec::new();
    };
    let Ok(guard) = aggregate.lock() else {
        return Vec::new();
    };
    let mut snapshots = guard
        .iter()
        .map(|((label, op), aggregate)| StatementProfileSnapshot {
            label: label.clone(),
            op,
            count: aggregate.count,
            failures: aggregate.failures,
            micros_sum: aggregate.micros_sum,
            micros_max: aggregate.micros_max,
            params_sum: aggregate.params_sum,
            param_bytes_sum: aggregate.param_bytes_sum,
            result_rows_sum: aggregate.result_rows_sum,
            result_bytes_sum: aggregate.result_bytes_sum,
        })
        .collect::<Vec<_>>();
    snapshots.sort_unstable_by(|left, right| {
        right
            .micros_sum
            .cmp(&left.micros_sum)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.op.cmp(right.op))
    });
    snapshots
        .into_iter()
        .map(|snapshot| {
            let avg_us = if snapshot.count == 0 {
                0
            } else {
                snapshot.micros_sum / snapshot.count
            };
            let sql = snapshot.label.replace('"', "'");
            format!(
                "pg_sql_statement op={} count={} total_us={} max_us={} avg_us={} params={} param_bytes={} result_rows={} result_bytes={} failures={} sql=\"{}\"",
                snapshot.op,
                snapshot.count,
                snapshot.micros_sum,
                snapshot.micros_max,
                avg_us,
                snapshot.params_sum,
                snapshot.param_bytes_sum,
                snapshot.result_rows_sum,
                snapshot.result_bytes_sum,
                snapshot.failures,
                sql
            )
        })
        .collect()
}

static FOD_COPY_PUT_DATA_PROFILE_AGGREGATE: std::sync::OnceLock<
    std::sync::Mutex<DbfsIoProfileAggregate>,
> = std::sync::OnceLock::new();

fn fod_profile_io_verbose_enabled() -> bool {
    env_var_truthy_with_legacy_alias("FOD_PROFILE_IO_VERBOSE", false)
}

fn fod_profile_copy_put_data_observe(elapsed: std::time::Duration, blocks: usize, bytes: usize) {
    let aggregate = FOD_COPY_PUT_DATA_PROFILE_AGGREGATE
        .get_or_init(|| std::sync::Mutex::new(DbfsIoProfileAggregate::default()));
    if let Ok(mut guard) = aggregate.lock() {
        guard.observe(elapsed.as_secs_f64(), blocks, bytes);
    }
}

fn fod_profile_copy_put_data_flush() {
    let Some(aggregate) = FOD_COPY_PUT_DATA_PROFILE_AGGREGATE.get() else {
        return;
    };
    let Ok(mut guard) = aggregate.lock() else {
        return;
    };
    if guard.count == 0 {
        return;
    }

    fod_log_io_profile(
        "pg.copy_put_data.aggregate",
        std::time::Duration::from_secs_f64(guard.seconds_sum),
        guard.blocks_sum,
        guard.bytes_sum,
        format!(
            "count={} max={:.6} avg={:.6}",
            guard.count,
            guard.seconds_max,
            guard.seconds_sum / guard.count as f64
        ),
    );

    *guard = DbfsIoProfileAggregate::default();
}

fn fod_log_io_profile(
    op: &str,
    elapsed: Duration,
    blocks: usize,
    bytes: usize,
    detail: impl AsRef<str>,
) {
    // Format musi zaczynac sie od "FOD I/O profile:", bo testlib szuka tego grepem
    if fod_profile_io_enabled() {
        eprintln!(
            "FOD I/O profile: op={} seconds={:.6} blocks={} bytes={} {}",
            op,
            elapsed.as_secs_f64(),
            blocks,
            bytes,
            detail.as_ref()
        );
    }
}

unsafe fn fod_profiled_pq_put_copy_data(
    conn: *mut PGconn,
    buffer: *const c_char,
    len: c_int,
) -> c_int {
    // Profiluje wysylanie danych COPY do PostgreSQL
    let started = Instant::now();
    let rc = PQputCopyData(conn, buffer, len);
    let bytes = if len > 0 { len as usize } else { 0 };
    let fod_copy_put_data_elapsed = started.elapsed();
    fod_profile_copy_put_data_observe(fod_copy_put_data_elapsed, 0, bytes);
    if fod_profile_io_verbose_enabled() {
        fod_log_io_profile(
            "pg.copy_put_data",
            fod_copy_put_data_elapsed,
            0,
            bytes,
            format!("rc={}", rc),
        );
    }

    rc
}

unsafe fn fod_profiled_pq_put_copy_end(conn: *mut PGconn, errormsg: *const c_char) -> c_int {
    // Profiluje zakonczenie COPY
    let started = Instant::now();
    let rc = PQputCopyEnd(conn, errormsg);
    fod_profile_copy_put_data_flush();
    fod_log_io_profile(
        "pg.copy_put_end",
        started.elapsed(),
        0,
        0,
        format!("rc={}", rc),
    );
    rc
}

unsafe fn fod_profiled_pq_get_result(conn: *mut PGconn) -> *mut PGresult {
    // Profiluje odbior wyniku po COPY
    let started = Instant::now();
    let res = PQgetResult(conn);
    let status = if res.is_null() {
        -1
    } else {
        PQresultStatus(res)
    };
    fod_log_io_profile(
        "pg.copy_get_result",
        started.elapsed(),
        0,
        0,
        format!("status={}", status),
    );
    res
}

#[link(name = "pq")]
unsafe extern "C" {
    fn PQconnectdb(conninfo: *const c_char) -> *mut PGconn;
    fn PQstatus(conn: *const PGconn) -> c_int;
    fn PQerrorMessage(conn: *const PGconn) -> *const c_char;
    fn PQlibVersion() -> c_int;
    fn PQserverVersion(conn: *const PGconn) -> c_int;
    fn PQexec(conn: *mut PGconn, command: *const c_char) -> *mut PGresult;
    fn PQprepare(
        conn: *mut PGconn,
        stmtName: *const c_char,
        query: *const c_char,
        nParams: c_int,
        paramTypes: *const c_uint,
    ) -> *mut PGresult;
    fn PQexecPrepared(
        conn: *mut PGconn,
        stmtName: *const c_char,
        nParams: c_int,
        paramValues: *const *const c_char,
        paramLengths: *const c_int,
        paramFormats: *const c_int,
        resultFormat: c_int,
    ) -> *mut PGresult;
    fn PQexecParams(
        conn: *mut PGconn,
        command: *const c_char,
        nParams: c_int,
        paramTypes: *const c_uint,
        paramValues: *const *const c_char,
        paramLengths: *const c_int,
        paramFormats: *const c_int,
        resultFormat: c_int,
    ) -> *mut PGresult;
    fn PQputCopyData(conn: *mut PGconn, buffer: *const c_char, len: c_int) -> c_int;
    fn PQputCopyEnd(conn: *mut PGconn, errormsg: *const c_char) -> c_int;
    fn PQgetResult(conn: *mut PGconn) -> *mut PGresult;
    fn PQresultStatus(res: *const PGresult) -> c_int;
    fn PQresultErrorMessage(res: *const PGresult) -> *const c_char;
    fn PQresultErrorField(res: *const PGresult, fieldcode: c_int) -> *const c_char;
    fn PQntuples(res: *const PGresult) -> c_int;
    fn PQnfields(res: *const PGresult) -> c_int;
    fn PQgetisnull(res: *const PGresult, row_number: c_int, field_number: c_int) -> c_int;
    fn PQgetlength(res: *const PGresult, row_number: c_int, field_number: c_int) -> c_int;
    fn PQgetvalue(res: *const PGresult, row_number: c_int, field_number: c_int) -> *const c_char;
    fn PQclear(res: *mut PGresult);
    fn PQfinish(conn: *mut PGconn);
}

fn conn_error(conn: *const PGconn) -> String {
    if conn.is_null() {
        return "libpq returned a null connection".to_string();
    }
    unsafe {
        let error = PQerrorMessage(conn);
        if error.is_null() {
            return "postgres connection error".to_string();
        }
        CStr::from_ptr(error).to_string_lossy().trim().to_string()
    }
}

fn result_error(res: *const PGresult) -> String {
    if res.is_null() {
        return "postgres result error".to_string();
    }
    unsafe {
        let error = PQresultErrorMessage(res);
        if error.is_null() {
            return "postgres result error".to_string();
        }
        let message = CStr::from_ptr(error).to_string_lossy().trim().to_string();
        let sqlstate = PQresultErrorField(res, PG_DIAG_SQLSTATE);
        if sqlstate.is_null() {
            message
        } else {
            let sqlstate = CStr::from_ptr(sqlstate)
                .to_string_lossy()
                .trim()
                .to_string();
            if sqlstate.is_empty() {
                message
            } else {
                format!("SQLSTATE {}: {}", sqlstate, message)
            }
        }
    }
}

unsafe fn postgres_version_diagnostics_on_conn(
    conn: *mut PGconn,
) -> Result<PostgresVersionDiagnostics, String> {
    let libpq_version_num = PQlibVersion();
    if libpq_version_num <= 0 {
        return Err("libpq runtime version is unavailable".to_string());
    }

    let server_version_num = PQserverVersion(conn);
    if server_version_num <= 0 {
        return Err("PostgreSQL server runtime version is unavailable".to_string());
    }

    let sql =
        CString::new("SHOW server_version").map_err(|_| "SQL contains NUL byte".to_string())?;
    let server_version = query_scalar_text_on_conn(conn, &sql)?;
    if server_version.trim().is_empty() {
        return Err("PostgreSQL server version string is empty".to_string());
    }

    Ok(PostgresVersionDiagnostics::new(
        libpq_version_num,
        server_version_num,
        server_version,
    ))
}

fn error_is_unique_violation(err: &str) -> bool {
    err.starts_with(UNIQUE_VIOLATION_ERROR_PREFIX)
}

fn replayable_sql_error(err: String) -> String {
    format!("{REPLAYABLE_SQL_ERROR_PREFIX}{err}")
}

fn replayable_sql_error_once(err: String) -> String {
    if err.starts_with(REPLAYABLE_SQL_ERROR_PREFIX) {
        err
    } else {
        replayable_sql_error(err)
    }
}

fn strip_replayable_sql_error(err: String) -> String {
    err.strip_prefix(REPLAYABLE_SQL_ERROR_PREFIX)
        .unwrap_or(&err)
        .to_string()
}

unsafe fn is_retryable_connection_error(conn: *const PGconn, err: &str) -> bool {
    if PQstatus(conn) != CONNECTION_OK {
        return true;
    }

    let lower = err.to_ascii_lowercase();
    lower.contains("server closed the connection unexpectedly")
        || lower.contains("connection to server was lost")
        || lower.contains("could not receive data from server")
        || lower.contains("could not send data to server")
        || lower.contains("broken pipe")
        || lower.contains("connection reset by peer")
        || lower.contains("ssl connection has been closed unexpectedly")
        || lower.contains("terminating connection due to administrator command")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbRepoConnectionTargetRequirement {
    Any,
    WritablePrimary,
    ReadOnlyReplica,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbRepoConnectionTarget {
    pub authority: String,
    pub conninfo: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DbRepoConnectionTargetProbe {
    recovery: bool,
    read_only: bool,
    system_identifier: String,
    server_fingerprint: String,
}

impl DbRepoConnectionTargetProbe {
    fn accepts(&self, requirement: DbRepoConnectionTargetRequirement) -> bool {
        match requirement {
            DbRepoConnectionTargetRequirement::Any => true,
            DbRepoConnectionTargetRequirement::WritablePrimary => !self.recovery && !self.read_only,
            DbRepoConnectionTargetRequirement::ReadOnlyReplica => self.recovery && self.read_only,
        }
    }

    fn writable_primary(&self) -> bool {
        self.accepts(DbRepoConnectionTargetRequirement::WritablePrimary)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrimarySafetyCandidate {
    target_index: usize,
    authority: String,
    system_identifier: String,
    server_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrimarySafetyErrorKind {
    NoWritablePrimary,
    ClusterIdentityMismatch,
    SplitBrain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrimarySafetyError {
    kind: PrimarySafetyErrorKind,
    message: String,
}

fn choose_safe_primary_candidate(
    candidates: &[PrimarySafetyCandidate],
    expected_system_identifier: Option<&str>,
    avoid_target_index: Option<usize>,
) -> Result<usize, PrimarySafetyError> {
    if candidates.is_empty() {
        return Err(PrimarySafetyError {
            kind: PrimarySafetyErrorKind::NoWritablePrimary,
            message: "PostgreSQL promotion guard found no writable primary".to_string(),
        });
    }

    if let Some(expected) = expected_system_identifier {
        if let Some(foreign) = candidates
            .iter()
            .find(|candidate| candidate.system_identifier.as_str() != expected)
        {
            return Err(PrimarySafetyError {
                kind: PrimarySafetyErrorKind::ClusterIdentityMismatch,
                message: format!(
                    "PostgreSQL promotion guard rejected writable target {} from foreign cluster system_identifier={} expected={}",
                    foreign.authority, foreign.system_identifier, expected
                ),
            });
        }
    } else {
        let first = &candidates[0].system_identifier;
        if let Some(other) = candidates
            .iter()
            .find(|candidate| candidate.system_identifier != *first)
        {
            return Err(PrimarySafetyError {
                kind: PrimarySafetyErrorKind::ClusterIdentityMismatch,
                message: format!(
                    "PostgreSQL promotion guard found writable targets from multiple cluster identities: {} and {}",
                    first, other.system_identifier
                ),
            });
        }
    }

    let mut fingerprints: Vec<&str> = Vec::new();
    for candidate in candidates {
        if !fingerprints
            .iter()
            .any(|fingerprint| *fingerprint == candidate.server_fingerprint.as_str())
        {
            fingerprints.push(candidate.server_fingerprint.as_str());
        }
    }
    if fingerprints.len() > 1 {
        return Err(PrimarySafetyError {
            kind: PrimarySafetyErrorKind::SplitBrain,
            message: format!(
                "PostgreSQL promotion guard detected {} simultaneously writable server identities for one cluster",
                fingerprints.len()
            ),
        });
    }

    if let Some(avoid) = avoid_target_index {
        if let Some((index, _)) = candidates
            .iter()
            .enumerate()
            .find(|(_, candidate)| candidate.target_index != avoid)
        {
            return Ok(index);
        }
    }
    Ok(0)
}

#[derive(Debug, Clone)]
struct DbRepoConnectionTargetMetric {
    connection_latency_micros_ewma: Option<u64>,
    operation_latency_micros_ewma: Option<u64>,
    replay_lag_bytes: Option<u64>,
    consecutive_failures: u64,
    circuit_open_until: Option<Instant>,
}

impl Default for DbRepoConnectionTargetMetric {
    fn default() -> Self {
        Self {
            connection_latency_micros_ewma: None,
            operation_latency_micros_ewma: None,
            replay_lag_bytes: None,
            consecutive_failures: 0,
            circuit_open_until: None,
        }
    }
}

impl DbRepoConnectionTargetMetric {
    fn observe_ewma(slot: &mut Option<u64>, sample: u64) {
        *slot = Some(match *slot {
            Some(previous) => previous.saturating_mul(3).saturating_add(sample) / 4,
            None => sample,
        });
    }

    fn score(&self, now: Instant) -> Option<u64> {
        if self
            .circuit_open_until
            .as_ref()
            .is_some_and(|until| *until > now)
        {
            return None;
        }
        let connection_latency = self
            .connection_latency_micros_ewma
            .unwrap_or(REPLICA_SCORE_UNKNOWN_LATENCY_MICROS)
            / REPLICA_SCORE_LATENCY_DIVISOR;
        let operation_latency = self
            .operation_latency_micros_ewma
            .unwrap_or(REPLICA_SCORE_UNKNOWN_LATENCY_MICROS)
            / REPLICA_SCORE_LATENCY_DIVISOR;
        let replay_lag = self.replay_lag_bytes.unwrap_or(0) / REPLICA_SCORE_LAG_DIVISOR_BYTES;
        let failure_penalty = self
            .consecutive_failures
            .saturating_mul(REPLICA_SCORE_FAILURE_PENALTY);
        Some(
            connection_latency
                .saturating_add(operation_latency)
                .saturating_add(replay_lag)
                .saturating_add(failure_penalty),
        )
    }

    fn record_connection_success(&mut self, elapsed: Duration) {
        Self::observe_ewma(
            &mut self.connection_latency_micros_ewma,
            elapsed.as_micros().min(u128::from(u64::MAX)) as u64,
        );
        self.consecutive_failures = 0;
        self.circuit_open_until = None;
    }

    fn record_operation_success(&mut self, elapsed: Duration) {
        Self::observe_ewma(
            &mut self.operation_latency_micros_ewma,
            elapsed.as_micros().min(u128::from(u64::MAX)) as u64,
        );
        self.consecutive_failures = 0;
        self.circuit_open_until = None;
    }

    fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= REPLICA_CIRCUIT_BREAKER_FAILURE_THRESHOLD {
            self.circuit_open_until =
                Some(Instant::now() + Duration::from_millis(REPLICA_CIRCUIT_BREAKER_COOLDOWN_MS));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DbRepoConnectionTargetScoringSnapshot {
    enabled: bool,
    active_score: Option<u64>,
    active_replay_lag_bytes: Option<u64>,
    active_connection_latency_micros: Option<u64>,
    active_operation_latency_micros: Option<u64>,
    score_selections: u64,
    score_switches: u64,
    hysteresis_keeps: u64,
    circuit_breaker_skips: u64,
    circuit_open_targets: usize,
}

#[derive(Debug)]
struct DbRepoConnectionTargetState {
    active_index: usize,
    generation: u64,
    failover_count: u64,
    connection_failures: u64,
    role_rejections: u64,
    last_failed_authority: Option<String>,
    target_metrics: Vec<DbRepoConnectionTargetMetric>,
    score_selections: u64,
    score_switches: u64,
    hysteresis_keeps: u64,
    circuit_breaker_skips: u64,
    primary_system_identifier: Option<String>,
    primary_server_fingerprint: Option<String>,
    primary_revalidation_required: bool,
    primary_last_failed_index: Option<usize>,
    primary_guard_scans: u64,
    primary_guard_unreachable_candidates: u64,
    primary_guard_role_rejections: u64,
    primary_guard_cluster_identity_rejections: u64,
    primary_guard_split_brain_rejections: u64,
    primary_guard_no_primary_rejections: u64,
    primary_guard_last_error: Option<String>,
    primary_operations_in_flight: u64,
    primary_transition_pending: bool,
}

impl Default for DbRepoConnectionTargetState {
    fn default() -> Self {
        Self {
            active_index: 0,
            generation: 1,
            failover_count: 0,
            connection_failures: 0,
            role_rejections: 0,
            last_failed_authority: None,
            target_metrics: Vec::new(),
            score_selections: 0,
            score_switches: 0,
            hysteresis_keeps: 0,
            circuit_breaker_skips: 0,
            primary_system_identifier: None,
            primary_server_fingerprint: None,
            primary_revalidation_required: false,
            primary_last_failed_index: None,
            primary_guard_scans: 0,
            primary_guard_unreachable_candidates: 0,
            primary_guard_role_rejections: 0,
            primary_guard_cluster_identity_rejections: 0,
            primary_guard_split_brain_rejections: 0,
            primary_guard_no_primary_rejections: 0,
            primary_guard_last_error: None,
            primary_operations_in_flight: 0,
            primary_transition_pending: false,
        }
    }
}

#[derive(Debug)]
pub struct DbRepoConnectionTargets {
    targets: Vec<DbRepoConnectionTarget>,
    requirement: DbRepoConnectionTargetRequirement,
    endpoint_routing_enabled: bool,
    runtime_failover_enabled: bool,
    state: Mutex<DbRepoConnectionTargetState>,
    transition: Mutex<()>,
    primary_fence: Condvar,
}

struct PrimaryOperationGuard<'a> {
    targets: &'a DbRepoConnectionTargets,
}

impl Drop for PrimaryOperationGuard<'_> {
    fn drop(&mut self) {
        self.targets.finish_primary_operation();
    }
}

impl DbRepoConnectionTargets {
    pub fn single(conninfo: &str) -> Result<Self, String> {
        Self::new(
            vec![DbRepoConnectionTarget {
                authority: "legacy-dsn".to_string(),
                conninfo: conninfo.to_string(),
            }],
            DbRepoConnectionTargetRequirement::Any,
            false,
            false,
        )
    }

    pub fn new(
        targets: Vec<DbRepoConnectionTarget>,
        requirement: DbRepoConnectionTargetRequirement,
        endpoint_routing_enabled: bool,
        runtime_failover_enabled: bool,
    ) -> Result<Self, String> {
        if targets.is_empty() {
            return Err("PostgreSQL connection target list must not be empty".to_string());
        }
        for target in &targets {
            if target.authority.trim().is_empty() {
                return Err("PostgreSQL connection target authority must not be empty".to_string());
            }
            if target.conninfo.trim().is_empty() {
                return Err(format!(
                    "PostgreSQL connection target {} has empty conninfo",
                    target.authority
                ));
            }
        }
        let target_count = targets.len();
        Ok(Self {
            runtime_failover_enabled: runtime_failover_enabled && target_count > 1,
            targets,
            requirement,
            endpoint_routing_enabled,
            state: Mutex::new(DbRepoConnectionTargetState {
                target_metrics: vec![DbRepoConnectionTargetMetric::default(); target_count],
                ..DbRepoConnectionTargetState::default()
            }),
            transition: Mutex::new(()),
            primary_fence: Condvar::new(),
        })
    }

    fn state(&self) -> Result<std::sync::MutexGuard<'_, DbRepoConnectionTargetState>, String> {
        self.state
            .lock()
            .map_err(|_| "PostgreSQL connection target state is poisoned".to_string())
    }

    fn current_target(&self) -> Result<(DbRepoConnectionTarget, u64), String> {
        let state = self.state()?;
        Ok((self.targets[state.active_index].clone(), state.generation))
    }

    pub fn generation(&self) -> Result<u64, String> {
        Ok(self.state()?.generation)
    }

    fn target_count(&self) -> usize {
        self.targets.len()
    }

    fn replica_scoring_enabled(&self) -> bool {
        self.requirement == DbRepoConnectionTargetRequirement::ReadOnlyReplica
    }

    fn primary_promotion_guard_enabled(&self) -> bool {
        self.requirement == DbRepoConnectionTargetRequirement::WritablePrimary
            && self.runtime_failover_enabled
    }

    fn begin_primary_operation(
        &self,
        generation: u64,
    ) -> Result<Option<PrimaryOperationGuard<'_>>, String> {
        if !self.primary_promotion_guard_enabled() {
            return Ok(None);
        }

        let mut state = self.state()?;
        while state.primary_transition_pending {
            state = self
                .primary_fence
                .wait(state)
                .map_err(|_| "PostgreSQL primary generation fence is poisoned".to_string())?;
        }

        if state.generation != generation {
            return Ok(None);
        }

        state.primary_operations_in_flight = state
            .primary_operations_in_flight
            .checked_add(1)
            .ok_or_else(|| "PostgreSQL primary in-flight operation counter overflow".to_string())?;

        Ok(Some(PrimaryOperationGuard { targets: self }))
    }

    fn finish_primary_operation(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.primary_operations_in_flight == 0 {
            debug_assert!(
                false,
                "PostgreSQL primary in-flight operation counter underflow"
            );
            return;
        }
        state.primary_operations_in_flight -= 1;
        if state.primary_operations_in_flight == 0 {
            self.primary_fence.notify_all();
        }
    }

    fn wait_for_primary_operations<'a>(
        &self,
        mut state: std::sync::MutexGuard<'a, DbRepoConnectionTargetState>,
        expected_generation: u64,
    ) -> Result<(std::sync::MutexGuard<'a, DbRepoConnectionTargetState>, bool), String> {
        if !self.primary_promotion_guard_enabled() || state.generation != expected_generation {
            return Ok((state, false));
        }

        state.primary_transition_pending = true;
        while state.primary_operations_in_flight > 0 {
            state = self
                .primary_fence
                .wait(state)
                .map_err(|_| "PostgreSQL primary generation fence is poisoned".to_string())?;
        }

        Ok((state, true))
    }

    fn finish_primary_transition_locked(
        &self,
        state: &mut DbRepoConnectionTargetState,
        primary_transition: bool,
    ) {
        if primary_transition {
            state.primary_transition_pending = false;
            self.primary_fence.notify_all();
        }
    }

    fn record_primary_guard_error_locked(
        state: &mut DbRepoConnectionTargetState,
        error: &PrimarySafetyError,
    ) {
        match error.kind {
            PrimarySafetyErrorKind::NoWritablePrimary => {
                state.primary_guard_no_primary_rejections =
                    state.primary_guard_no_primary_rejections.saturating_add(1);
            }
            PrimarySafetyErrorKind::ClusterIdentityMismatch => {
                state.primary_guard_cluster_identity_rejections = state
                    .primary_guard_cluster_identity_rejections
                    .saturating_add(1);
            }
            PrimarySafetyErrorKind::SplitBrain => {
                state.primary_guard_split_brain_rejections =
                    state.primary_guard_split_brain_rejections.saturating_add(1);
            }
        }
        state.primary_guard_last_error = Some(error.message.clone());
    }

    fn connect_primary_guarded(
        &self,
        tuning: &ConnectionTuning,
        avoid_target_index: Option<usize>,
    ) -> Result<(*mut PGconn, u64), String> {
        let expected_system_identifier = {
            let mut state = self.state()?;
            state.primary_guard_scans = state.primary_guard_scans.saturating_add(1);
            state.primary_system_identifier.clone()
        };

        let mut live: Vec<(PrimarySafetyCandidate, *mut PGconn)> = Vec::new();
        let mut unreachable = 0u64;
        let mut role_rejections = 0u64;

        for (target_index, target) in self.targets.iter().enumerate() {
            match connect(&target.conninfo, tuning) {
                Ok(conn) => match unsafe { probe_connection_target(conn) } {
                    Ok(probe) if probe.writable_primary() => {
                        live.push((
                            PrimarySafetyCandidate {
                                target_index,
                                authority: target.authority.clone(),
                                system_identifier: probe.system_identifier,
                                server_fingerprint: probe.server_fingerprint,
                            },
                            conn,
                        ));
                    }
                    Ok(_) => {
                        role_rejections = role_rejections.saturating_add(1);
                        unsafe { PQfinish(conn) };
                    }
                    Err(_) => {
                        role_rejections = role_rejections.saturating_add(1);
                        unsafe { PQfinish(conn) };
                    }
                },
                Err(_) => {
                    unreachable = unreachable.saturating_add(1);
                }
            }
        }

        let candidates = live
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect::<Vec<_>>();
        let decision = choose_safe_primary_candidate(
            &candidates,
            expected_system_identifier.as_deref(),
            avoid_target_index,
        );

        let selected_position = match decision {
            Ok(index) => index,
            Err(error) => {
                for (_, conn) in live {
                    unsafe { PQfinish(conn) };
                }
                let mut state = self.state()?;
                state.primary_guard_unreachable_candidates = state
                    .primary_guard_unreachable_candidates
                    .saturating_add(unreachable);
                state.primary_guard_role_rejections = state
                    .primary_guard_role_rejections
                    .saturating_add(role_rejections);
                Self::record_primary_guard_error_locked(&mut state, &error);
                state.primary_revalidation_required = true;
                return Err(error.message);
            }
        };

        let selected_candidate = live[selected_position].0.clone();
        let selected_conn = live[selected_position].1;
        for (index, (_, conn)) in live.into_iter().enumerate() {
            if index != selected_position {
                unsafe { PQfinish(conn) };
            }
        }

        let mut state = self.state()?;
        state.primary_guard_unreachable_candidates = state
            .primary_guard_unreachable_candidates
            .saturating_add(unreachable);
        state.primary_guard_role_rejections = state
            .primary_guard_role_rejections
            .saturating_add(role_rejections);
        let was_revalidation = state.primary_revalidation_required;
        let previous_index = state.active_index;
        let needs_generation_transition =
            previous_index != selected_candidate.target_index && !was_revalidation;
        let primary_transition = if needs_generation_transition {
            let expected_generation = state.generation;
            let (next_state, primary_transition) =
                self.wait_for_primary_operations(state, expected_generation)?;
            state = next_state;
            primary_transition
        } else {
            false
        };

        if previous_index != selected_candidate.target_index {
            state.active_index = selected_candidate.target_index;
            state.failover_count = state.failover_count.saturating_add(1);
            if !was_revalidation {
                state.generation = state.generation.saturating_add(1);
            }
        }
        state.primary_system_identifier = Some(selected_candidate.system_identifier);
        state.primary_server_fingerprint = Some(selected_candidate.server_fingerprint);
        state.primary_revalidation_required = false;
        state.primary_last_failed_index = None;
        state.primary_guard_last_error = None;
        let generation = state.generation;
        self.finish_primary_transition_locked(&mut state, primary_transition);
        Ok((selected_conn, generation))
    }

    fn best_replica_candidate_locked(
        &self,
        state: &mut DbRepoConnectionTargetState,
        exclude_current: bool,
    ) -> Option<(usize, u64)> {
        if !self.replica_scoring_enabled() {
            return Some((state.active_index, 0));
        }
        let now = Instant::now();
        let current = state.active_index;
        let mut best: Option<(usize, u64)> = None;
        let mut circuit_skips = 0u64;
        for index in 0..state.target_metrics.len() {
            if exclude_current && index == current {
                continue;
            }
            match state.target_metrics[index].score(now) {
                Some(score) => {
                    if best
                        .as_ref()
                        .map_or(true, |(_, best_score)| score < *best_score)
                    {
                        best = Some((index, score));
                    }
                }
                None => circuit_skips = circuit_skips.saturating_add(1),
            }
        }
        state.circuit_breaker_skips = state.circuit_breaker_skips.saturating_add(circuit_skips);
        best
    }

    fn select_scored_replica_locked(
        &self,
        state: &mut DbRepoConnectionTargetState,
        exclude_current: bool,
        force_switch: bool,
    ) -> bool {
        if !self.replica_scoring_enabled() {
            return false;
        }
        state.score_selections = state.score_selections.saturating_add(1);
        let current = state.active_index;
        let current_score = state.target_metrics[current].score(Instant::now());
        let Some((best_index, best_score)) =
            self.best_replica_candidate_locked(state, exclude_current)
        else {
            return false;
        };
        if best_index == current {
            return false;
        }
        let should_switch = force_switch
            || current_score.is_none()
            || best_score.saturating_add(REPLICA_SCORE_HYSTERESIS_MARGIN)
                < current_score.unwrap_or(u64::MAX);
        if should_switch {
            state.active_index = best_index;
            state.score_switches = state.score_switches.saturating_add(1);
            true
        } else {
            state.hysteresis_keeps = state.hysteresis_keeps.saturating_add(1);
            false
        }
    }

    fn prepare_scored_replica(&self) -> Result<Option<u64>, String> {
        if !self.replica_scoring_enabled() {
            return Ok(Some(self.generation()?));
        }
        let _transition = self
            .transition
            .lock()
            .map_err(|_| "PostgreSQL connection target transition is poisoned".to_string())?;
        let mut state = self.state()?;
        if self.select_scored_replica_locked(&mut state, false, false) {
            state.generation = state.generation.saturating_add(1);
        }
        if state.target_metrics[state.active_index]
            .score(Instant::now())
            .is_some()
        {
            Ok(Some(state.generation))
        } else {
            Ok(None)
        }
    }

    fn record_target_connection_success(
        &self,
        generation: u64,
        elapsed: Duration,
    ) -> Result<(), String> {
        if !self.replica_scoring_enabled() {
            return Ok(());
        }
        let mut state = self.state()?;
        if state.generation == generation {
            let active_index = state.active_index;
            state.target_metrics[active_index].record_connection_success(elapsed);
        }
        Ok(())
    }

    fn record_replica_replay(
        &self,
        generation: u64,
        replay_lag_bytes: u64,
    ) -> Result<bool, String> {
        if !self.replica_scoring_enabled() {
            return Ok(false);
        }
        let _transition = self
            .transition
            .lock()
            .map_err(|_| "PostgreSQL connection target transition is poisoned".to_string())?;
        let mut state = self.state()?;
        if state.generation != generation {
            return Ok(false);
        }
        let active_index = state.active_index;
        state.target_metrics[active_index].replay_lag_bytes = Some(replay_lag_bytes);
        if replay_lag_bytes == 0 {
            return Ok(false);
        }
        let switched = self.select_scored_replica_locked(&mut state, true, true);
        if switched {
            state.generation = state.generation.saturating_add(1);
        }
        Ok(switched)
    }

    fn record_replica_operation_success(
        &self,
        generation: u64,
        elapsed: Duration,
    ) -> Result<(), String> {
        if !self.replica_scoring_enabled() {
            return Ok(());
        }
        let mut state = self.state()?;
        if state.generation == generation {
            let active_index = state.active_index;
            state.target_metrics[active_index].record_operation_success(elapsed);
        }
        Ok(())
    }

    fn scoring_snapshot(&self) -> Result<DbRepoConnectionTargetScoringSnapshot, String> {
        let state = self.state()?;
        let enabled = self.replica_scoring_enabled();
        let now = Instant::now();
        let active_metric = &state.target_metrics[state.active_index];
        let circuit_open_targets = state
            .target_metrics
            .iter()
            .filter(|metric| metric.score(now).is_none())
            .count();
        Ok(DbRepoConnectionTargetScoringSnapshot {
            enabled,
            active_score: if enabled {
                active_metric.score(now)
            } else {
                None
            },
            active_replay_lag_bytes: if enabled {
                active_metric.replay_lag_bytes
            } else {
                None
            },
            active_connection_latency_micros: if enabled {
                active_metric.connection_latency_micros_ewma
            } else {
                None
            },
            active_operation_latency_micros: if enabled {
                active_metric.operation_latency_micros_ewma
            } else {
                None
            },
            score_selections: state.score_selections,
            score_switches: state.score_switches,
            hysteresis_keeps: state.hysteresis_keeps,
            circuit_breaker_skips: state.circuit_breaker_skips,
            circuit_open_targets,
        })
    }

    pub fn snapshot(&self) -> Result<DbRepoConnectionRoutingSnapshot, String> {
        let state = self.state()?;
        Ok(DbRepoConnectionRoutingSnapshot {
            endpoint_routing_enabled: self.endpoint_routing_enabled,
            runtime_failover_enabled: self.runtime_failover_enabled,
            target_count: self.targets.len(),
            active_authority: self.targets[state.active_index].authority.clone(),
            generation: state.generation,
            failover_count: state.failover_count,
            connection_failures: state.connection_failures,
            role_rejections: state.role_rejections,
            last_failed_authority: state.last_failed_authority.clone(),
            replica_read_routing_enabled: false,
            replica_target_count: 0,
            replica_active_authority: None,
            required_primary_wal_lsn: None,
            primary_wal_lsn_updates: 0,
            primary_wal_lsn_capture_failures: 0,
            replica_consistency_checks: 0,
            replica_consistency_passes: 0,
            replica_reads: 0,
            replica_lag_fallbacks: 0,
            replica_read_failures: 0,
            primary_read_fallbacks: 0,
            replica_scoring_enabled: false,
            replica_active_score: None,
            replica_active_replay_lag_bytes: None,
            replica_active_connection_latency_micros: None,
            replica_active_operation_latency_micros: None,
            replica_score_selections: 0,
            replica_score_switches: 0,
            replica_hysteresis_keeps: 0,
            replica_circuit_breaker_skips: 0,
            replica_circuit_open_targets: 0,
            replica_pool_pressure_fallbacks: 0,
            primary_promotion_guard_enabled: self.primary_promotion_guard_enabled(),
            primary_system_identifier: state.primary_system_identifier.clone(),
            primary_server_fingerprint: state.primary_server_fingerprint.clone(),
            primary_guard_scans: state.primary_guard_scans,
            primary_guard_unreachable_candidates: state.primary_guard_unreachable_candidates,
            primary_guard_role_rejections: state.primary_guard_role_rejections,
            primary_guard_cluster_identity_rejections: state
                .primary_guard_cluster_identity_rejections,
            primary_guard_split_brain_rejections: state.primary_guard_split_brain_rejections,
            primary_guard_no_primary_rejections: state.primary_guard_no_primary_rejections,
            primary_guard_last_error: state.primary_guard_last_error.clone(),
        })
    }

    fn advance_after_failure(
        &self,
        expected_generation: u64,
        authority: &str,
        role_rejection: bool,
    ) -> Result<u64, String> {
        let mut state = self.state()?;
        state.connection_failures = state.connection_failures.saturating_add(1);
        if role_rejection {
            state.role_rejections = state.role_rejections.saturating_add(1);
        }
        if state.generation != expected_generation {
            return Ok(state.generation);
        }

        let primary_transition = if self.primary_promotion_guard_enabled() {
            let (next_state, primary_transition) =
                self.wait_for_primary_operations(state, expected_generation)?;
            state = next_state;
            primary_transition
        } else {
            false
        };

        state.last_failed_authority = Some(authority.to_string());
        let failed_index = state.active_index;
        if self.replica_scoring_enabled() {
            state.target_metrics[failed_index].record_failure();
        }
        state.generation = state.generation.saturating_add(1);
        if self.primary_promotion_guard_enabled() {
            state.primary_revalidation_required = true;
            state.primary_last_failed_index = Some(failed_index);
        } else if self.runtime_failover_enabled {
            let previous_index = state.active_index;
            if self.replica_scoring_enabled() {
                let _ = self.select_scored_replica_locked(&mut state, true, true);
            } else {
                state.active_index = (state.active_index + 1) % self.targets.len();
            }
            if state.active_index != previous_index {
                state.failover_count = state.failover_count.saturating_add(1);
            }
        }

        let generation = state.generation;
        self.finish_primary_transition_locked(&mut state, primary_transition);
        Ok(generation)
    }

    fn mark_primary_revalidation(
        &self,
        expected_generation: u64,
        active_index: usize,
        last_error: Option<String>,
    ) -> Result<u64, String> {
        let state = self.state()?;
        if state.generation != expected_generation {
            return Ok(state.generation);
        }

        let (mut state, primary_transition) =
            self.wait_for_primary_operations(state, expected_generation)?;
        if state.generation == expected_generation {
            state.primary_revalidation_required = true;
            state.primary_last_failed_index = Some(active_index);
            if let Some(last_error) = last_error {
                state.primary_guard_last_error = Some(last_error);
            }
            state.generation = state.generation.saturating_add(1);
        }

        let generation = state.generation;
        self.finish_primary_transition_locked(&mut state, primary_transition);
        Ok(generation)
    }

    fn mark_operation_connection_failure(&self, generation: u64) -> Result<u64, String> {
        let _transition = self
            .transition
            .lock()
            .map_err(|_| "PostgreSQL connection target transition is poisoned".to_string())?;
        let (target, _) = self.current_target()?;
        self.advance_after_failure(generation, &target.authority, false)
    }

    fn connect_new(&self, tuning: &ConnectionTuning) -> Result<(*mut PGconn, u64), String> {
        let _transition = self
            .transition
            .lock()
            .map_err(|_| "PostgreSQL connection target transition is poisoned".to_string())?;

        if self.primary_promotion_guard_enabled() {
            let (needs_scan, avoid_target_index, expected_system_identifier, expected_fingerprint) = {
                let state = self.state()?;
                (
                    state.primary_revalidation_required
                        || state.primary_system_identifier.is_none()
                        || state.primary_server_fingerprint.is_none(),
                    state.primary_last_failed_index,
                    state.primary_system_identifier.clone(),
                    state.primary_server_fingerprint.clone(),
                )
            };
            if needs_scan {
                return self.connect_primary_guarded(tuning, avoid_target_index);
            }

            let (target, generation) = self.current_target()?;
            let active_index = self.state()?.active_index;
            match connect(&target.conninfo, tuning) {
                Ok(conn) => {
                    let probe = match unsafe { probe_connection_target(conn) } {
                        Ok(probe) => probe,
                        Err(err) => {
                            unsafe { PQfinish(conn) };
                            self.advance_after_failure(generation, &target.authority, true)?;
                            return self
                                .connect_primary_guarded(tuning, Some(active_index))
                                .map_err(|guard_err| {
                                    format!(
                                        "{}: {}; promotion guard: {}",
                                        target.authority, err, guard_err
                                    )
                                });
                        }
                    };
                    if !probe.writable_primary() {
                        unsafe { PQfinish(conn) };
                        let err = format!(
                            "PostgreSQL target role rejected: recovery={} transaction_read_only={}",
                            probe.recovery, probe.read_only
                        );
                        self.advance_after_failure(generation, &target.authority, true)?;
                        return self
                            .connect_primary_guarded(tuning, Some(active_index))
                            .map_err(|guard_err| {
                                format!(
                                    "{}: {}; promotion guard: {}",
                                    target.authority, err, guard_err
                                )
                            });
                    }

                    if expected_system_identifier.as_deref()
                        != Some(probe.system_identifier.as_str())
                    {
                        unsafe { PQfinish(conn) };
                        self.mark_primary_revalidation(
                            generation,
                            active_index,
                            Some(format!(
                                "PostgreSQL promotion guard observed changed cluster identity on {}: observed={} expected={}",
                                target.authority,
                                probe.system_identifier,
                                expected_system_identifier.as_deref().unwrap_or("none")
                            )),
                        )?;
                        return self.connect_primary_guarded(tuning, Some(active_index));
                    }

                    if expected_fingerprint.as_deref() != Some(probe.server_fingerprint.as_str()) {
                        unsafe { PQfinish(conn) };
                        self.mark_primary_revalidation(generation, active_index, None)?;
                        return self.connect_primary_guarded(tuning, Some(active_index));
                    }

                    return Ok((conn, generation));
                }
                Err(err) => {
                    self.advance_after_failure(generation, &target.authority, false)?;
                    return self
                        .connect_primary_guarded(tuning, Some(active_index))
                        .map_err(|guard_err| {
                            format!(
                                "{}: {}; promotion guard: {}",
                                target.authority, err, guard_err
                            )
                        });
                }
            }
        }

        let attempts = if self.runtime_failover_enabled {
            self.targets.len()
        } else {
            1
        };
        let mut errors = Vec::new();
        for _ in 0..attempts {
            if self.replica_scoring_enabled() {
                let mut state = self.state()?;
                if self.select_scored_replica_locked(&mut state, false, false) {
                    state.generation = state.generation.saturating_add(1);
                }
                let active_index = state.active_index;
                if state.target_metrics[active_index]
                    .score(Instant::now())
                    .is_none()
                {
                    errors.push("all replica circuits are cooling down".to_string());
                    break;
                }
            }
            let (target, generation) = self.current_target()?;
            let connect_started = Instant::now();
            match connect(&target.conninfo, tuning) {
                Ok(conn) => {
                    match unsafe { validate_connection_target_role(conn, self.requirement) } {
                        Ok(()) => {
                            if let Err(err) = self.record_target_connection_success(
                                generation,
                                connect_started.elapsed(),
                            ) {
                                unsafe {
                                    PQfinish(conn);
                                }
                                return Err(err);
                            }
                            return Ok((conn, generation));
                        }
                        Err(err) => {
                            unsafe {
                                PQfinish(conn);
                            }
                            errors.push(format!("{}: {}", target.authority, err));
                            self.advance_after_failure(generation, &target.authority, true)?;
                        }
                    }
                }
                Err(err) => {
                    errors.push(format!("{}: {}", target.authority, err));
                    self.advance_after_failure(generation, &target.authority, false)?;
                }
            }
        }
        Err(format!(
            "PostgreSQL connection targets exhausted: {}",
            errors.join(" | ")
        ))
    }
}

#[derive(Debug, Default)]
struct ReplicaReadConsistencyState {
    required_primary_wal_lsn: Option<u64>,
    required_primary_wal_lsn_text: Option<String>,
    force_primary: bool,
    primary_wal_lsn_updates: u64,
    primary_wal_lsn_capture_failures: u64,
    replica_consistency_checks: u64,
    replica_consistency_passes: u64,
    replica_reads: u64,
    replica_lag_fallbacks: u64,
    replica_read_failures: u64,
    primary_read_fallbacks: u64,
    replica_pool_pressure_fallbacks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplicaReadConsistencySnapshot {
    required_primary_wal_lsn: Option<String>,
    force_primary: bool,
    primary_wal_lsn_updates: u64,
    primary_wal_lsn_capture_failures: u64,
    replica_consistency_checks: u64,
    replica_consistency_passes: u64,
    replica_reads: u64,
    replica_lag_fallbacks: u64,
    replica_read_failures: u64,
    primary_read_fallbacks: u64,
    replica_pool_pressure_fallbacks: u64,
}

#[derive(Debug, Default)]
struct ReplicaReadConsistency {
    state: Mutex<ReplicaReadConsistencyState>,
}

fn parse_pg_lsn(value: &str) -> Result<u64, String> {
    let value = value.trim();
    let (high, low) = value
        .split_once('/')
        .ok_or_else(|| format!("invalid PostgreSQL LSN `{value}`"))?;
    let high =
        u64::from_str_radix(high, 16).map_err(|_| format!("invalid PostgreSQL LSN `{value}`"))?;
    let low =
        u64::from_str_radix(low, 16).map_err(|_| format!("invalid PostgreSQL LSN `{value}`"))?;
    if high > u64::from(u32::MAX) || low > u64::from(u32::MAX) {
        return Err(format!("PostgreSQL LSN component overflow `{value}`"));
    }
    Ok((high << 32) | low)
}

impl ReplicaReadConsistency {
    fn record_primary_wal_lsn(&self, value: &str) -> Result<(), String> {
        let parsed = parse_pg_lsn(value)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "replica consistency state is poisoned".to_string())?;
        state.required_primary_wal_lsn = Some(parsed);
        state.required_primary_wal_lsn_text = Some(value.trim().to_ascii_uppercase());
        state.force_primary = false;
        state.primary_wal_lsn_updates = state.primary_wal_lsn_updates.saturating_add(1);
        Ok(())
    }

    fn record_primary_wal_lsn_capture_failure(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.force_primary = true;
            state.primary_wal_lsn_capture_failures =
                state.primary_wal_lsn_capture_failures.saturating_add(1);
        }
    }

    fn required_lsn(&self) -> Result<Option<u64>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "replica consistency state is poisoned".to_string())?;
        if state.force_primary {
            Ok(None)
        } else {
            Ok(state.required_primary_wal_lsn)
        }
    }

    fn force_primary(&self) -> Result<bool, String> {
        self.state
            .lock()
            .map(|state| state.force_primary)
            .map_err(|_| "replica consistency state is poisoned".to_string())
    }

    fn record_consistency_check(&self, passed: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.replica_consistency_checks = state.replica_consistency_checks.saturating_add(1);
            if passed {
                state.replica_consistency_passes =
                    state.replica_consistency_passes.saturating_add(1);
            } else {
                state.replica_lag_fallbacks = state.replica_lag_fallbacks.saturating_add(1);
            }
        }
    }

    fn record_replica_read(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.replica_reads = state.replica_reads.saturating_add(1);
        }
    }

    fn record_replica_failure(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.replica_read_failures = state.replica_read_failures.saturating_add(1);
        }
    }

    fn record_primary_fallback(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.primary_read_fallbacks = state.primary_read_fallbacks.saturating_add(1);
        }
    }

    fn record_forced_primary_fallback(&self) {
        self.record_primary_fallback();
    }

    fn record_pool_pressure_fallback(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.replica_pool_pressure_fallbacks =
                state.replica_pool_pressure_fallbacks.saturating_add(1);
            state.primary_read_fallbacks = state.primary_read_fallbacks.saturating_add(1);
        }
    }

    fn snapshot(&self) -> Result<ReplicaReadConsistencySnapshot, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "replica consistency state is poisoned".to_string())?;
        Ok(ReplicaReadConsistencySnapshot {
            required_primary_wal_lsn: state.required_primary_wal_lsn_text.clone(),
            force_primary: state.force_primary,
            primary_wal_lsn_updates: state.primary_wal_lsn_updates,
            primary_wal_lsn_capture_failures: state.primary_wal_lsn_capture_failures,
            replica_consistency_checks: state.replica_consistency_checks,
            replica_consistency_passes: state.replica_consistency_passes,
            replica_reads: state.replica_reads,
            replica_lag_fallbacks: state.replica_lag_fallbacks,
            replica_read_failures: state.replica_read_failures,
            primary_read_fallbacks: state.primary_read_fallbacks,
            replica_pool_pressure_fallbacks: state.replica_pool_pressure_fallbacks,
        })
    }
}

unsafe fn probe_connection_target(
    conn: *mut PGconn,
) -> Result<DbRepoConnectionTargetProbe, String> {
    let sql = CString::new(
        "SELECT pg_is_in_recovery()::text || '|' || \
         current_setting('transaction_read_only') || '|' || \
         (SELECT system_identifier::text FROM pg_control_system()) || '|' || \
         COALESCE(inet_server_addr()::text, 'local') || ':' || \
         COALESCE(inet_server_port()::text, 'local') || '|' || \
         EXTRACT(EPOCH FROM pg_postmaster_start_time())::text",
    )
    .map_err(|_| "PostgreSQL target validation SQL contains NUL byte".to_string())?;
    let res = PQexec(conn, sql.as_ptr());
    if res.is_null() {
        return Err(conn_error(conn));
    }
    if PQresultStatus(res) != PGRES_TUPLES_OK {
        let err = result_error(res);
        PQclear(res);
        return Err(err);
    }
    let value = fetch_single_text(res)?;
    let fields = value.trim().split('|').collect::<Vec<_>>();
    if fields.len() != 5 {
        return Err(format!(
            "invalid PostgreSQL target identity probe `{value}`"
        ));
    }
    let parse_bool = |value: &str| -> Result<bool, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "t" | "true" | "1" | "on" => Ok(true),
            "f" | "false" | "0" | "off" => Ok(false),
            other => Err(format!("invalid PostgreSQL boolean `{other}`")),
        }
    };
    let recovery = parse_bool(fields[0])?;
    let read_only = parse_bool(fields[1])?;
    let system_identifier = fields[2].trim().to_string();
    let server_endpoint = fields[3].trim();
    let postmaster_start = fields[4].trim();
    if system_identifier.is_empty() || server_endpoint.is_empty() || postmaster_start.is_empty() {
        return Err(format!(
            "incomplete PostgreSQL target identity probe `{value}`"
        ));
    }
    Ok(DbRepoConnectionTargetProbe {
        recovery,
        read_only,
        system_identifier,
        server_fingerprint: format!("{server_endpoint}|{postmaster_start}"),
    })
}

unsafe fn validate_connection_target_role(
    conn: *mut PGconn,
    requirement: DbRepoConnectionTargetRequirement,
) -> Result<(), String> {
    if requirement == DbRepoConnectionTargetRequirement::Any {
        return Ok(());
    }
    let probe = probe_connection_target(conn)?;
    if probe.accepts(requirement) {
        Ok(())
    } else {
        Err(format!(
            "PostgreSQL target role rejected: recovery={} transaction_read_only={}",
            probe.recovery, probe.read_only
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreparedStatement {
    FileDataObjectId,
    FileDataObjectInfo,
    FileReadMetadata,
    DataObjectReferenceCount,
    GetDirId,
    GetFileIdRoot,
    GetFileIdNested,
    GetFileModeRoot,
    GetFileModeNested,
    GetHardlinkIdRoot,
    GetHardlinkIdNested,
    GetHardlinkFileId,
    ChoosePrimaryHardlink,
    LoadBlock,
    FetchBlockRange,
    ResolvePathRoot,
    ResolvePathNested,
    FetchPathAttrsBlobFile,
    FetchPathAttrsBlobDir,
    FetchPathAttrsBlobSymlink,
    FetchPathAttrsBlobHardlink,
    StatfsSnapshot,
    LoadSymlinkTarget,
    GetSpecialFileMetadata,
    GetSymlinkIdRoot,
    GetSymlinkIdNested,
}

impl PreparedStatement {
    fn all() -> &'static [PreparedStatement] {
        &[
            PreparedStatement::FileDataObjectId,
            PreparedStatement::FileDataObjectInfo,
            PreparedStatement::FileReadMetadata,
            PreparedStatement::DataObjectReferenceCount,
            PreparedStatement::GetDirId,
            PreparedStatement::GetFileIdRoot,
            PreparedStatement::GetFileIdNested,
            PreparedStatement::GetFileModeRoot,
            PreparedStatement::GetFileModeNested,
            PreparedStatement::GetHardlinkIdRoot,
            PreparedStatement::GetHardlinkIdNested,
            PreparedStatement::GetHardlinkFileId,
            PreparedStatement::ChoosePrimaryHardlink,
            PreparedStatement::LoadBlock,
            PreparedStatement::FetchBlockRange,
            PreparedStatement::ResolvePathRoot,
            PreparedStatement::ResolvePathNested,
            PreparedStatement::FetchPathAttrsBlobFile,
            PreparedStatement::FetchPathAttrsBlobDir,
            PreparedStatement::FetchPathAttrsBlobSymlink,
            PreparedStatement::FetchPathAttrsBlobHardlink,
            PreparedStatement::StatfsSnapshot,
            PreparedStatement::LoadSymlinkTarget,
            PreparedStatement::GetSpecialFileMetadata,
            PreparedStatement::GetSymlinkIdRoot,
            PreparedStatement::GetSymlinkIdNested,
        ]
    }

    fn name(self) -> &'static str {
        match self {
            PreparedStatement::FileDataObjectId => "fod_file_data_object_id",
            PreparedStatement::FileDataObjectInfo => "fod_file_data_object_info",
            PreparedStatement::FileReadMetadata => "fod_file_read_metadata",
            PreparedStatement::DataObjectReferenceCount => "fod_data_object_reference_count",
            PreparedStatement::GetDirId => "fod_get_dir_id",
            PreparedStatement::GetFileIdRoot => "fod_get_file_id_root",
            PreparedStatement::GetFileIdNested => "fod_get_file_id_nested",
            PreparedStatement::GetFileModeRoot => "fod_get_file_mode_root",
            PreparedStatement::GetFileModeNested => "fod_get_file_mode_nested",
            PreparedStatement::GetHardlinkIdRoot => "fod_get_hardlink_id_root",
            PreparedStatement::GetHardlinkIdNested => "fod_get_hardlink_id_nested",
            PreparedStatement::GetHardlinkFileId => "fod_get_hardlink_file_id",
            PreparedStatement::ChoosePrimaryHardlink => "fod_choose_primary_hardlink",
            PreparedStatement::LoadBlock => "fod_load_block",
            PreparedStatement::FetchBlockRange => "fod_fetch_block_range",
            PreparedStatement::ResolvePathRoot => "fod_resolve_path_root",
            PreparedStatement::ResolvePathNested => "fod_resolve_path_nested",
            PreparedStatement::FetchPathAttrsBlobFile => "fod_fetch_path_attrs_blob_file",
            PreparedStatement::FetchPathAttrsBlobDir => "fod_fetch_path_attrs_blob_dir",
            PreparedStatement::FetchPathAttrsBlobSymlink => "fod_fetch_path_attrs_blob_symlink",
            PreparedStatement::FetchPathAttrsBlobHardlink => "fod_fetch_path_attrs_blob_hardlink",
            PreparedStatement::StatfsSnapshot => "fod_statfs_snapshot",
            PreparedStatement::LoadSymlinkTarget => "fod_load_symlink_target",
            PreparedStatement::GetSpecialFileMetadata => "fod_get_special_file_metadata",
            PreparedStatement::GetSymlinkIdRoot => "fod_get_symlink_id_root",
            PreparedStatement::GetSymlinkIdNested => "fod_get_symlink_id_nested",
        }
    }

    fn sql(self) -> &'static str {
        match self {
            PreparedStatement::FileDataObjectId => {
                "SELECT data_object_id FROM files WHERE id_file = $1"
            }
            PreparedStatement::FileDataObjectInfo => {
                "SELECT f.data_object_id, d.reference_count FROM files f JOIN data_objects d ON d.id_data_object = f.data_object_id WHERE f.id_file = $1"
            }
            PreparedStatement::FileReadMetadata => {
                "SELECT size, access_date, modification_date, change_date FROM files WHERE id_file = $1"
            }
            PreparedStatement::DataObjectReferenceCount => {
                "SELECT reference_count FROM data_objects WHERE id_data_object = $1"
            }
            PreparedStatement::GetDirId => {
                "
                WITH RECURSIVE parts AS (
                    SELECT part, ord
                    FROM unnest(string_to_array(btrim($1, '/'), '/')) WITH ORDINALITY AS t(part, ord)
                ),
                walk AS (
                    SELECT d.id_directory, p.ord
                    FROM directories d
                    JOIN parts p ON p.ord = 1
                    WHERE d.id_parent IS NULL AND d.name = p.part
                    UNION ALL
                    SELECT d.id_directory, p.ord
                    FROM walk w
                    JOIN parts p ON p.ord = w.ord + 1
                    JOIN directories d ON d.id_parent = w.id_directory AND d.name = p.part
                )
                SELECT id_directory
                FROM walk
                ORDER BY ord DESC
                LIMIT 1
                "
            }
            PreparedStatement::GetFileIdRoot => {
                "
                SELECT id_file FROM (
                    SELECT 1 AS precedence, id_file FROM hardlinks WHERE name = $1 AND id_directory IS NULL
                    UNION ALL
                    SELECT 2 AS precedence, id_file FROM files WHERE name = $1 AND id_directory IS NULL
                ) entries
                ORDER BY precedence
                LIMIT 1
                "
            }
            PreparedStatement::GetFileIdNested => {
                "
                SELECT id_file FROM (
                    SELECT 1 AS precedence, id_file FROM hardlinks WHERE name = $1 AND id_directory = $2
                    UNION ALL
                    SELECT 2 AS precedence, id_file FROM files WHERE name = $1 AND id_directory = $2
                ) entries
                ORDER BY precedence
                LIMIT 1
                "
            }
            PreparedStatement::GetFileModeRoot => {
                "
                SELECT mode FROM (
                    SELECT 1 AS precedence, mode
                    FROM hardlinks JOIN files ON hardlinks.id_file = files.id_file
                    WHERE hardlinks.name = $1 AND hardlinks.id_directory IS NULL
                    UNION ALL
                    SELECT 2 AS precedence, mode
                    FROM files
                    WHERE name = $1 AND id_directory IS NULL
                ) entries
                ORDER BY precedence
                LIMIT 1
                "
            }
            PreparedStatement::GetFileModeNested => {
                "
                SELECT mode FROM (
                    SELECT 1 AS precedence, mode
                    FROM hardlinks JOIN files ON hardlinks.id_file = files.id_file
                    WHERE hardlinks.name = $1 AND hardlinks.id_directory = $2
                    UNION ALL
                    SELECT 2 AS precedence, mode
                    FROM files
                    WHERE name = $1 AND id_directory = $2
                ) entries
                ORDER BY precedence
                LIMIT 1
                "
            }
            PreparedStatement::GetHardlinkIdRoot => {
                "SELECT id_hardlink FROM hardlinks WHERE name = $1 AND id_directory IS NULL"
            }
            PreparedStatement::GetHardlinkIdNested => {
                "SELECT id_hardlink FROM hardlinks WHERE name = $1 AND id_directory = $2"
            }
            PreparedStatement::GetHardlinkFileId => {
                "SELECT id_file FROM hardlinks WHERE id_hardlink = $1"
            }
            PreparedStatement::ChoosePrimaryHardlink => {
                "SELECT id_hardlink, id_directory, name FROM hardlinks WHERE id_file = $1 ORDER BY id_hardlink ASC LIMIT 1"
            }
            PreparedStatement::LoadBlock => {
                "
                SELECT db.data
                FROM files f
                JOIN data_blocks db ON db.data_object_id = f.data_object_id
                WHERE f.id_file = $1 AND db._order = $2
                "
            }
            PreparedStatement::FetchBlockRange => {
                "
                SELECT db._order::bigint, db.data
                FROM files f
                JOIN data_blocks db ON db.data_object_id = f.data_object_id
                WHERE f.id_file = $1 AND db._order BETWEEN $2 AND $3
                ORDER BY db._order ASC
                "
            }
            PreparedStatement::ResolvePathRoot => {
                "
                SELECT kind, entry_id FROM (
                    SELECT 1 AS precedence, 'hardlink' AS kind, h.id_hardlink AS entry_id
                    FROM hardlinks h
                    WHERE h.name = $1 AND h.id_directory IS NULL
                    UNION ALL
                    SELECT 2 AS precedence, 'symlink' AS kind, s.id_symlink AS entry_id
                    FROM symlinks s
                    WHERE s.name = $1 AND s.id_parent IS NULL
                    UNION ALL
                    SELECT 3 AS precedence, 'file' AS kind, f.id_file AS entry_id
                    FROM files f
                    WHERE f.name = $1 AND f.id_directory IS NULL
                    UNION ALL
                    SELECT 4 AS precedence, 'dir' AS kind, d.id_directory AS entry_id
                    FROM directories d
                    WHERE d.name = $1 AND d.id_parent IS NULL
                ) entries
                ORDER BY precedence
                LIMIT 1
                "
            }
            PreparedStatement::ResolvePathNested => {
                "
                SELECT kind, entry_id FROM (
                    SELECT 1 AS precedence, 'hardlink' AS kind, h.id_hardlink AS entry_id
                    FROM hardlinks h
                    WHERE h.name = $1 AND h.id_directory = $2
                    UNION ALL
                    SELECT 2 AS precedence, 'symlink' AS kind, s.id_symlink AS entry_id
                    FROM symlinks s
                    WHERE s.name = $1 AND s.id_parent = $2
                    UNION ALL
                    SELECT 3 AS precedence, 'file' AS kind, f.id_file AS entry_id
                    FROM files f
                    WHERE f.name = $1 AND f.id_directory = $2
                    UNION ALL
                    SELECT 4 AS precedence, 'dir' AS kind, d.id_directory AS entry_id
                    FROM directories d
                    WHERE d.name = $1 AND d.id_parent = $2
                ) entries
                ORDER BY precedence
                LIMIT 1
                "
            }
            PreparedStatement::FetchPathAttrsBlobFile => {
                "
                SELECT
                    f.id_file,
                    f.size,
                    f.mode,
                    f.modification_date,
                    f.access_date,
                    f.change_date,
                    f.uid,
                    f.gid,
                    f.inode_seed,
                    (
                        SELECT COUNT(*) * (SELECT value FROM config WHERE key = 'block_size')
                        FROM data_blocks db
                        WHERE db.data_object_id = f.data_object_id
                    )
                FROM files f
                WHERE f.id_file = $1
                "
            }
            PreparedStatement::FetchPathAttrsBlobDir => {
                "SELECT id_directory, 0, mode, modification_date, access_date, change_date, uid, gid, inode_seed, 0 FROM directories WHERE id_directory = $1"
            }
            PreparedStatement::FetchPathAttrsBlobSymlink => {
                "SELECT id_symlink, target, modification_date, access_date, change_date, uid, gid, inode_seed FROM symlinks WHERE id_symlink = $1"
            }
            PreparedStatement::FetchPathAttrsBlobHardlink => {
                "
                SELECT
                    h.id_hardlink,
                    f.size,
                    f.mode,
                    f.modification_date,
                    f.access_date,
                    f.change_date,
                    f.uid,
                    f.gid,
                    f.inode_seed,
                    (
                        SELECT COUNT(*) * (SELECT value FROM config WHERE key = 'block_size')
                        FROM data_blocks db
                        WHERE db.data_object_id = f.data_object_id
                    )
                FROM hardlinks h
                JOIN files f ON h.id_file = f.id_file
                WHERE h.id_hardlink = $1
                "
            }
            PreparedStatement::StatfsSnapshot => {
                "SELECT (SELECT COUNT(*) FROM files)::text, (SELECT COUNT(*) FROM directories)::text, (SELECT COUNT(*) FROM symlinks)::text, ((SELECT COUNT(*) FROM data_blocks) * (SELECT value FROM config WHERE key = 'block_size'))::text, (SELECT COALESCE(SUM(reserved_bytes), 0) FROM payload_capacity_reservations WHERE expires_at > NOW())::text, COALESCE((SELECT value FROM config WHERE key = 'max_fs_size_bytes'), 0)::text"
            }
            PreparedStatement::LoadSymlinkTarget => {
                "SELECT target FROM symlinks WHERE id_symlink = $1"
            }
            PreparedStatement::GetSpecialFileMetadata => {
                "SELECT file_type, rdev_major, rdev_minor FROM special_files WHERE id_file = $1"
            }
            PreparedStatement::GetSymlinkIdRoot => {
                "SELECT id_symlink FROM symlinks WHERE name = $1 AND id_parent IS NULL"
            }
            PreparedStatement::GetSymlinkIdNested => {
                "SELECT id_symlink FROM symlinks WHERE name = $1 AND id_parent = $2"
            }
        }
    }

    fn is_read_only(self) -> bool {
        // All currently defined prepared statements are lookup queries.
        // Keep this explicit so adding a write statement requires an audit.
        match self {
            PreparedStatement::FileDataObjectId
            | PreparedStatement::FileDataObjectInfo
            | PreparedStatement::FileReadMetadata
            | PreparedStatement::DataObjectReferenceCount
            | PreparedStatement::GetDirId
            | PreparedStatement::GetFileIdRoot
            | PreparedStatement::GetFileIdNested
            | PreparedStatement::GetFileModeRoot
            | PreparedStatement::GetFileModeNested
            | PreparedStatement::GetHardlinkIdRoot
            | PreparedStatement::GetHardlinkIdNested
            | PreparedStatement::GetHardlinkFileId
            | PreparedStatement::ChoosePrimaryHardlink
            | PreparedStatement::LoadBlock
            | PreparedStatement::FetchBlockRange
            | PreparedStatement::ResolvePathRoot
            | PreparedStatement::ResolvePathNested
            | PreparedStatement::FetchPathAttrsBlobFile
            | PreparedStatement::FetchPathAttrsBlobDir
            | PreparedStatement::FetchPathAttrsBlobSymlink
            | PreparedStatement::FetchPathAttrsBlobHardlink
            | PreparedStatement::StatfsSnapshot
            | PreparedStatement::LoadSymlinkTarget
            | PreparedStatement::GetSpecialFileMetadata
            | PreparedStatement::GetSymlinkIdRoot
            | PreparedStatement::GetSymlinkIdNested => true,
        }
    }

    fn param_count(self) -> c_int {
        match self {
            PreparedStatement::FileDataObjectId
            | PreparedStatement::FileDataObjectInfo
            | PreparedStatement::FileReadMetadata
            | PreparedStatement::DataObjectReferenceCount
            | PreparedStatement::GetDirId
            | PreparedStatement::GetFileIdRoot
            | PreparedStatement::GetFileModeRoot
            | PreparedStatement::GetHardlinkIdRoot
            | PreparedStatement::GetHardlinkFileId
            | PreparedStatement::ChoosePrimaryHardlink
            | PreparedStatement::ResolvePathRoot
            | PreparedStatement::FetchPathAttrsBlobFile
            | PreparedStatement::FetchPathAttrsBlobDir
            | PreparedStatement::FetchPathAttrsBlobSymlink
            | PreparedStatement::FetchPathAttrsBlobHardlink
            | PreparedStatement::LoadSymlinkTarget
            | PreparedStatement::GetSpecialFileMetadata
            | PreparedStatement::GetSymlinkIdRoot => 1,
            PreparedStatement::GetFileIdNested
            | PreparedStatement::GetFileModeNested
            | PreparedStatement::GetHardlinkIdNested
            | PreparedStatement::ResolvePathNested
            | PreparedStatement::GetSymlinkIdNested
            | PreparedStatement::LoadBlock => 2,
            PreparedStatement::FetchBlockRange => 3,
            PreparedStatement::StatfsSnapshot => 0,
        }
    }
}

unsafe fn prepare_statement(conn: *mut PGconn, statement: PreparedStatement) -> Result<(), String> {
    let name = CString::new(statement.name())
        .map_err(|_| "prepared statement name contains NUL byte".to_string())?;
    let sql = CString::new(statement.sql())
        .map_err(|_| "prepared statement SQL contains NUL byte".to_string())?;
    let res = PQprepare(
        conn,
        name.as_ptr(),
        sql.as_ptr(),
        statement.param_count(),
        std::ptr::null(),
    );
    if res.is_null() {
        return Err(conn_error(conn));
    }
    let status = PQresultStatus(res);
    if status == PGRES_COMMAND_OK {
        PQclear(res);
        Ok(())
    } else {
        let error = result_error(res);
        PQclear(res);
        Err(error)
    }
}

unsafe fn prepare_connection(conn: *mut PGconn) -> Result<(), String> {
    for statement in PreparedStatement::all() {
        prepare_statement(conn, *statement)?;
    }
    Ok(())
}

unsafe fn exec_prepared_params(
    conn: *mut PGconn,
    statement: PreparedStatement,
    params: &[&CString],
) -> Result<*mut PGresult, String> {
    exec_prepared_params_with_result_format(conn, statement, params, 0)
}

unsafe fn exec_prepared_params_with_result_format(
    conn: *mut PGconn,
    statement: PreparedStatement,
    params: &[&CString],
    result_format: c_int,
) -> Result<*mut PGresult, String> {
    let profile_enabled = fod_profile_io_enabled();
    if params.len() != statement.param_count() as usize {
        return Err(format!(
            "prepared statement {} expected {} parameters, got {}",
            statement.name(),
            statement.param_count(),
            params.len()
        ));
    }
    let name = CString::new(statement.name())
        .map_err(|_| "prepared statement name contains NUL byte".to_string())?;
    let param_values = params
        .iter()
        .map(|value| value.as_ptr())
        .collect::<Vec<_>>();
    let param_lengths = params
        .iter()
        .map(|value| value.as_bytes().len() as c_int)
        .collect::<Vec<_>>();
    let param_bytes = if profile_enabled {
        param_lengths
            .iter()
            .map(|value| if *value > 0 { *value as usize } else { 0 })
            .sum::<usize>()
    } else {
        0
    };
    let param_formats = vec![0 as c_int; params.len()];
    let (param_values_ptr, param_lengths_ptr, param_formats_ptr) = if params.is_empty() {
        (std::ptr::null(), std::ptr::null(), std::ptr::null())
    } else {
        (
            param_values.as_ptr(),
            param_lengths.as_ptr(),
            param_formats.as_ptr(),
        )
    };
    let started = Instant::now();
    let res = PQexecPrepared(
        conn,
        name.as_ptr(),
        params.len() as c_int,
        param_values_ptr,
        param_lengths_ptr,
        param_formats_ptr,
        result_format,
    );
    let elapsed = started.elapsed();
    if res.is_null() {
        if profile_enabled {
            fod_profile_prepared_statement_observe(
                statement,
                elapsed,
                params.len(),
                param_bytes,
                0,
                0,
                true,
            );
        }
        let err = conn_error(conn);
        Err(maybe_replayable_prepared_error(conn, statement, err))
    } else if statement.is_read_only() {
        let status = PQresultStatus(res);
        let failed = status != PGRES_TUPLES_OK;
        if profile_enabled {
            let (result_rows, result_bytes) = pgresult_profile_payload(res, status);
            fod_profile_prepared_statement_observe(
                statement,
                elapsed,
                params.len(),
                param_bytes,
                result_rows,
                result_bytes,
                failed,
            );
        }
        if status != PGRES_TUPLES_OK {
            let error = result_error(res);
            if is_retryable_connection_error(conn, &error) {
                PQclear(res);
                return Err(replayable_sql_error(error));
            }
        }
        Ok(res)
    } else {
        if profile_enabled {
            let status = PQresultStatus(res);
            let failed = !matches!(status, PGRES_TUPLES_OK | PGRES_COMMAND_OK | PGRES_COPY_IN);
            fod_profile_prepared_statement_observe(
                statement,
                elapsed,
                params.len(),
                param_bytes,
                0,
                0,
                failed,
            );
        }
        Ok(res)
    }
}

#[derive(Debug, Clone, Default)]
struct ConnectionTuning {
    synchronous_commit: Option<String>,
}

impl ConnectionTuning {
    fn from_storage(storage: &RuntimeStorageSettings) -> Self {
        Self {
            synchronous_commit: Some(storage.synchronous_commit.clone()),
        }
    }
}

fn apply_connection_tuning(conninfo: &str, tuning: &ConnectionTuning) -> Result<String, String> {
    let mut tuned = conninfo.to_string();
    if let Some(value) = tuning.synchronous_commit.as_ref() {
        let value = value.trim().to_ascii_lowercase();
        if !value.is_empty() {
            match value.as_str() {
                "on" | "off" | "local" | "remote_write" | "remote_apply" | "true" | "false" => {
                    tuned.push_str(" options='-c synchronous_commit=");
                    tuned.push_str(&value);
                    tuned.push('\'');
                }
                other => return Err(format!("invalid synchronous_commit value: {other}")),
            }
        }
    }
    Ok(tuned)
}

unsafe fn exec_connection_setup_command(conn: *mut PGconn, sql: &str) -> Result<(), String> {
    let sql =
        CString::new(sql).map_err(|_| "PostgreSQL session SQL contains NUL byte".to_string())?;
    let res = PQexec(conn, sql.as_ptr());
    if res.is_null() {
        return Err(conn_error(conn));
    }
    let status = PQresultStatus(res);
    if status == PGRES_COMMAND_OK {
        PQclear(res);
        Ok(())
    } else {
        let err = result_error(res);
        PQclear(res);
        Err(err)
    }
}

fn connect(conninfo: &str, tuning: &ConnectionTuning) -> Result<*mut PGconn, String> {
    let conninfo = apply_connection_tuning(conninfo, tuning)?;
    let conninfo =
        CString::new(conninfo).map_err(|_| "connection string contains NUL byte".to_string())?;
    unsafe {
        let conn = PQconnectdb(conninfo.as_ptr());
        if conn.is_null() {
            return Err("failed to create PostgreSQL connection".to_string());
        }
        if PQstatus(conn) != CONNECTION_OK {
            let err = conn_error(conn);
            PQfinish(conn);
            return Err(err);
        }
        let server_version_num = PQserverVersion(conn);
        if server_version_num < MIN_POSTGRES_SERVER_VERSION_NUM {
            let err = format!(
                "PostgreSQL server_version_num={server_version_num} is unsupported; FOD requires {MIN_POSTGRES_SERVER_VERSION_NUM} or newer. Upgrade the PostgreSQL instance before starting FOD."
            );
            PQfinish(conn);
            return Err(err);
        }

        let set_search_path = format!("SET search_path TO {FOD_SEARCH_PATH}");
        let session_setup_sql = postgres_session_setup_sql(server_version_num)
            .iter()
            .copied()
            .chain(std::iter::once(set_search_path.as_str()))
            .collect::<Vec<_>>()
            .join("; ");
        if let Err(err) = exec_connection_setup_command(conn, &session_setup_sql) {
            PQfinish(conn);
            return Err(format!(
                "failed to apply required PostgreSQL session settings: {err}"
            ));
        }
        let schema_is_initialized = schema_is_initialized_on_conn(conn).map_err(|err| {
            PQfinish(conn);
            err
        })?;
        if schema_is_initialized {
            // Empty databases must stay connectable so bootstrap and status paths can
            // report that initialization is still missing.
            prepare_connection(conn).map_err(|err| {
                PQfinish(conn);
                err
            })?;
        }
        Ok(conn)
    }
}

unsafe fn query_scalar_text_on_conn(conn: *mut PGconn, sql: &CString) -> Result<String, String> {
    let profile_enabled = fod_profile_io_enabled();
    let started = Instant::now();
    let res = PQexec(conn, sql.as_ptr());
    let elapsed = started.elapsed();
    if res.is_null() {
        if profile_enabled {
            fod_profile_sql_statement_observe(
                "query_scalar",
                fod_sql_label(sql),
                elapsed,
                0,
                0,
                0,
                0,
                true,
            );
        }
        let err = conn_error(conn);
        return Err(maybe_replayable_sql_error(conn, sql, err));
    }
    let status = PQresultStatus(res);
    let failed = status != PGRES_TUPLES_OK;
    if profile_enabled {
        let (result_rows, result_bytes) = pgresult_profile_payload(res, status);
        fod_profile_sql_statement_observe(
            "query_scalar",
            fod_sql_label(sql),
            elapsed,
            0,
            0,
            result_rows,
            result_bytes,
            failed,
        );
    }
    if status != PGRES_TUPLES_OK {
        let err = result_error(res);
        PQclear(res);
        if sql_is_read_only(sql) && is_retryable_connection_error(conn, &err) {
            return Err(replayable_sql_error(err));
        }
        return Err(err);
    }
    fetch_single_text(res)
}

unsafe fn query_rows_text_on_conn(
    conn: *mut PGconn,
    sql: &CString,
) -> Result<Vec<Vec<String>>, String> {
    let profile_enabled = fod_profile_io_enabled();
    let started = Instant::now();
    let res = PQexec(conn, sql.as_ptr());
    let elapsed = started.elapsed();
    if res.is_null() {
        if profile_enabled {
            fod_profile_sql_statement_observe(
                "query_rows",
                fod_sql_label(sql),
                elapsed,
                0,
                0,
                0,
                0,
                true,
            );
        }
        let err = conn_error(conn);
        return Err(maybe_replayable_sql_error(conn, sql, err));
    }
    let status = PQresultStatus(res);
    let failed = status != PGRES_TUPLES_OK;
    if profile_enabled {
        let (result_rows, result_bytes) = pgresult_profile_payload(res, status);
        fod_profile_sql_statement_observe(
            "query_rows",
            fod_sql_label(sql),
            elapsed,
            0,
            0,
            result_rows,
            result_bytes,
            failed,
        );
    }
    if status != PGRES_TUPLES_OK {
        let err = result_error(res);
        PQclear(res);
        if sql_is_read_only(sql) && is_retryable_connection_error(conn, &err) {
            return Err(replayable_sql_error(err));
        }
        return Err(err);
    }
    fetch_rows_text(res)
}

unsafe fn postgres_runtime_requirements_on_conn(
    conn: *mut PGconn,
    pool_max_connections: u64,
) -> Result<PostgresRuntimeRequirements, String> {
    let sql = CString::new(POSTGRES_REQUIREMENT_SETTINGS_SQL)
        .map_err(|_| "PostgreSQL requirements SQL contains NUL byte".to_string())?;
    let rows = query_rows_text_on_conn(conn, &sql)?;
    let requirements = PostgresRuntimeRequirements::from_pg_settings_rows(
        PQserverVersion(conn),
        pool_max_connections,
        rows,
    )?;
    if let Some(err) = requirements.unsupported_version_error() {
        return Err(err);
    }
    let session_errors = requirements.session_configuration_errors();
    if !session_errors.is_empty() {
        return Err(format!(
            "PostgreSQL session requirements were not applied: {}",
            session_errors.join("; ")
        ));
    }
    Ok(requirements)
}

unsafe fn schema_is_initialized_on_conn(conn: *mut PGconn) -> Result<bool, String> {
    let sql = CString::new(format!(
        "SELECT \
            to_regclass('{schema}.directories') IS NOT NULL AND \
            to_regclass('{schema}.files') IS NOT NULL AND \
            to_regclass('{schema}.schema_version') IS NOT NULL",
        schema = FOD_SCHEMA_NAME
    ))
    .map_err(|_| "SQL contains NUL byte".to_string())?;
    let value = query_scalar_text_on_conn(conn, &sql)?;
    Ok(matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "t" | "true" | "1" | "on"
    ))
}

unsafe fn fetch_single_text(res: *mut PGresult) -> Result<String, String> {
    let value = match PQresultStatus(res) {
        PGRES_TUPLES_OK => {
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            if rows < 1 || cols < 1 {
                Ok(String::new())
            } else {
                let value_ptr = PQgetvalue(res, 0, 0);
                if value_ptr.is_null() {
                    Ok(String::new())
                } else {
                    Ok(CStr::from_ptr(value_ptr).to_string_lossy().to_string())
                }
            }
        }
        _ => Err("unexpected PostgreSQL result status".to_string()),
    };
    PQclear(res);
    value
}

unsafe fn fetch_single_text_option(res: *mut PGresult) -> Result<Option<String>, String> {
    let value = fetch_single_text(res)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

unsafe fn fetch_first_row_texts(res: *mut PGresult) -> Result<Vec<String>, String> {
    let result = match PQresultStatus(res) {
        PGRES_TUPLES_OK => {
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            if rows < 1 || cols < 1 {
                Ok(Vec::new())
            } else {
                let mut values = Vec::with_capacity(cols as usize);
                for col in 0..cols {
                    let value_ptr = PQgetvalue(res, 0, col);
                    if value_ptr.is_null() {
                        values.push(String::new());
                    } else {
                        values.push(CStr::from_ptr(value_ptr).to_string_lossy().to_string());
                    }
                }
                Ok(values)
            }
        }
        _ => Err("unexpected PostgreSQL result status".to_string()),
    };
    PQclear(res);
    result
}

unsafe fn fetch_rows_text(res: *mut PGresult) -> Result<Vec<Vec<String>>, String> {
    let result = match PQresultStatus(res) {
        PGRES_TUPLES_OK => {
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            if rows < 1 || cols < 1 {
                Ok(Vec::new())
            } else {
                let mut values = Vec::with_capacity(rows as usize);
                for row in 0..rows {
                    let mut current = Vec::with_capacity(cols as usize);
                    for col in 0..cols {
                        let value_ptr = PQgetvalue(res, row, col);
                        if value_ptr.is_null() {
                            current.push(String::new());
                        } else {
                            current.push(CStr::from_ptr(value_ptr).to_string_lossy().to_string());
                        }
                    }
                    values.push(current);
                }
                Ok(values)
            }
        }
        _ => Err("unexpected PostgreSQL result status".to_string()),
    };
    PQclear(res);
    result
}

unsafe fn fetch_first_column_texts(res: *mut PGresult) -> Result<Vec<String>, String> {
    let result = match PQresultStatus(res) {
        PGRES_TUPLES_OK => {
            let rows = PQntuples(res);
            if rows < 1 {
                Ok(Vec::new())
            } else {
                let cols = PQnfields(res);
                if cols < 1 {
                    Ok(Vec::new())
                } else {
                    let mut values = Vec::with_capacity(rows as usize);
                    for row in 0..rows {
                        let value_ptr = PQgetvalue(res, row, 0);
                        if value_ptr.is_null() {
                            values.push(String::new());
                        } else {
                            values.push(CStr::from_ptr(value_ptr).to_string_lossy().to_string());
                        }
                    }
                    Ok(values)
                }
            }
        }
        _ => Err("unexpected PostgreSQL result status".to_string()),
    };
    PQclear(res);
    result
}

unsafe fn read_result_binary_field<T, F>(
    res: *const PGresult,
    row: c_int,
    col: c_int,
    decode: F,
) -> Result<Option<T>, String>
where
    F: FnOnce(&[u8]) -> Result<T, String>,
{
    if PQgetisnull(res, row, col) != 0 {
        return Ok(None);
    }
    let len = PQgetlength(res, row, col);
    if len < 0 {
        return Err("invalid binary field length".to_string());
    }
    let value_ptr = PQgetvalue(res, row, col);
    if value_ptr.is_null() {
        return Err("binary field pointer is null".to_string());
    }
    let bytes = std::slice::from_raw_parts(value_ptr.cast::<u8>(), len as usize);
    decode(bytes).map(Some)
}

unsafe fn result_binary_i64(res: *const PGresult, row: c_int, col: c_int) -> Result<i64, String> {
    read_result_binary_field(res, row, col, |bytes| match bytes.len() {
        8 => Ok(i64::from_be_bytes(
            bytes
                .try_into()
                .map_err(|_| "invalid BIGINT binary field".to_string())?,
        )),
        4 => Ok(i32::from_be_bytes(
            bytes
                .try_into()
                .map_err(|_| "invalid INTEGER binary field".to_string())?,
        ) as i64),
        other => Err(format!("unexpected integer binary field length: {other}")),
    })?
    .ok_or_else(|| "integer field is null".to_string())
}

unsafe fn fetch_single_block_binary(
    res: *mut PGresult,
    block_size: usize,
) -> Result<Option<Vec<u8>>, String> {
    let result = match PQresultStatus(res) {
        PGRES_TUPLES_OK => {
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            if rows < 1 || cols < 1 {
                Ok(None)
            } else {
                read_result_binary_field(res, 0, 0, |bytes| {
                    Ok(normalize_block_bytes(bytes, block_size))
                })
            }
        }
        _ => Err("unexpected PostgreSQL result status".to_string()),
    };
    PQclear(res);
    result
}

unsafe fn fetch_block_range_rows_shared(
    res: *mut PGresult,
    block_size: usize,
) -> Result<Vec<(u64, Arc<[u8]>)>, String> {
    let result = match PQresultStatus(res) {
        PGRES_TUPLES_OK => {
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            if rows < 1 || cols < 2 {
                Ok(Vec::new())
            } else {
                let mut blocks = Vec::with_capacity(rows as usize);
                for row in 0..rows {
                    let index = result_binary_i64(res, row, 0)?;
                    let index = u64::try_from(index)
                        .map_err(|_| "negative block index value".to_string())?;
                    let bytes = match read_result_binary_field(res, row, 1, |bytes| {
                        Ok(normalize_block_bytes(bytes, block_size))
                    })? {
                        Some(bytes) => bytes,
                        None => continue,
                    };
                    blocks.push((index, Arc::from(bytes)));
                }
                blocks.sort_unstable_by_key(|(index, _)| *index);
                Ok(blocks)
            }
        }
        _ => Err("unexpected PostgreSQL result status".to_string()),
    };
    PQclear(res);
    result
}

fn join_nul_text(values: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            out.push(0);
        }
        out.extend_from_slice(value.as_bytes());
    }
    out
}

fn sql_is_read_only(sql: &CString) -> bool {
    let raw = sql.to_string_lossy();
    let token = raw
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(
        token.as_str(),
        "SELECT" | "WITH" | "SHOW" | "VALUES" | "EXPLAIN"
    )
}

fn sql_is_replayable_data_blocks_upsert(sql: &str) -> bool {
    sql.starts_with("INSERT INTO data_blocks ")
        && sql.contains("ON CONFLICT (data_object_id, _order) DO UPDATE SET data = EXCLUDED.data")
}

fn sql_is_replayable_copy_block_crc_upsert(sql: &str) -> bool {
    sql.starts_with("INSERT INTO copy_block_crc ")
        && sql.contains("ON CONFLICT (data_object_id, _order) DO UPDATE SET crc32 = EXCLUDED.crc32, updated_at = NOW()")
}

fn sql_is_replayable_schema_ddl(sql: &str) -> bool {
    sql.starts_with("CREATE TABLE IF NOT EXISTS ")
        || sql.starts_with("CREATE TEMP TABLE IF NOT EXISTS ")
        || sql.starts_with("CREATE INDEX IF NOT EXISTS ")
        || sql.starts_with("CREATE UNIQUE INDEX IF NOT EXISTS ")
        || sql.starts_with("CREATE OR REPLACE FUNCTION ")
        || sql.starts_with("DROP TRIGGER IF EXISTS ")
        || sql.starts_with("CREATE TRIGGER fod_client_sessions_prune_lock_leases ")
        || (sql.starts_with("ALTER TABLE IF EXISTS ")
            && (sql.contains(" ADD COLUMN IF NOT EXISTS ")
                || sql.contains(" ALTER COLUMN session_id SET DEFAULT 0")
                || sql.contains(" ALTER COLUMN session_id SET NOT NULL")
                || sql.contains(
                    " DROP CONSTRAINT IF EXISTS lock_leases_resource_kind_resource_id_owner_key_lease_kind_key",
                )
                || sql.contains(
                    " ALTER COLUMN owner_key TYPE NUMERIC(20,0) USING owner_key::numeric",
                )
                || sql.contains(" ALTER COLUMN session_id DROP DEFAULT")))
}

fn sql_compact(sql: &CString) -> String {
    sql.to_string_lossy()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn sql_is_replayable_command(sql: &CString) -> bool {
    let sql = sql_compact(sql);

    sql.starts_with("DELETE FROM ")
        || sql.starts_with("INSERT INTO data_objects ")
            && sql.contains("RETURNING id_data_object")
        || sql.starts_with("INSERT INTO hardlinks ")
            && sql.contains("RETURNING id_hardlink")
        || sql.starts_with("INSERT INTO symlinks ")
            && sql.contains("RETURNING id_symlink")
        || sql.starts_with("INSERT INTO directories ")
            && sql.contains("RETURNING id_directory")
        || sql.starts_with("INSERT INTO files ")
            && sql.contains("RETURNING id_file")
        || sql.starts_with("INSERT INTO special_files ")
        || sql.starts_with("INSERT INTO data_object_request_tokens ")
            && sql.contains(
                "ON CONFLICT (request_token) DO UPDATE SET updated_at = NOW() RETURNING id_data_object",
            )
        || sql.starts_with("INSERT INTO hardlink_promotion_request_tokens ")
            && sql.contains(
                "ON CONFLICT (request_token) DO UPDATE SET updated_at = NOW() RETURNING did_promote",
            )
        || sql.starts_with("INSERT INTO lock_lease_request_tokens ")
            && sql.contains(
                "ON CONFLICT (request_token) DO UPDATE SET did_grant = EXCLUDED.did_grant, updated_at = NOW() RETURNING did_grant",
            )
        || sql.starts_with("INSERT INTO payload_capacity_reservations ")
            && sql.contains(
                "ON CONFLICT (request_token) DO UPDATE SET reserved_bytes = EXCLUDED.reserved_bytes, expires_at = EXCLUDED.expires_at",
            )
        || sql.starts_with("INSERT INTO index_sources ")
            && sql.contains("ON CONFLICT (name) DO UPDATE SET kind = EXCLUDED.kind, root_path = EXCLUDED.root_path, updated_at = NOW()")
        || sql.starts_with("INSERT INTO index_files ")
            && sql.contains("ON CONFLICT (id_index_source, path) DO UPDATE SET id_scan_run = EXCLUDED.id_scan_run, size = EXCLUDED.size, mtime_ns = EXCLUDED.mtime_ns, inode = EXCLUDED.inode, device = EXCLUDED.device, file_kind = EXCLUDED.file_kind, scan_status = EXCLUDED.scan_status, source_changed = EXCLUDED.source_changed, updated_at = NOW()")
        || sql.starts_with("INSERT INTO index_scan_runs ")
            && sql.contains("ON CONFLICT (request_token) DO UPDATE SET id_index_source = EXCLUDED.id_index_source, status = EXCLUDED.status, updated_at = NOW()")
        || sql.starts_with("INSERT INTO index_file_hashes ")
            && sql.contains("ON CONFLICT (id_file) DO UPDATE SET hash_algorithm = EXCLUDED.hash_algorithm, partial_hash = EXCLUDED.partial_hash, full_hash = EXCLUDED.full_hash, hash_status = EXCLUDED.hash_status, observed_size = EXCLUDED.observed_size, observed_mtime_ns = EXCLUDED.observed_mtime_ns, observed_inode = EXCLUDED.observed_inode, observed_device = EXCLUDED.observed_device, updated_at = NOW()")
        || sql.starts_with("INSERT INTO index_duplicate_sets ")
            && sql.contains("ON CONFLICT (hash_algorithm, full_hash, file_size) DO UPDATE SET file_count = EXCLUDED.file_count, total_bytes = EXCLUDED.total_bytes, updated_at = NOW()")
        || sql.starts_with("INSERT INTO index_import_plans ")
            && sql.contains("ON CONFLICT (request_token) DO UPDATE SET status = EXCLUDED.status, dry_run = EXCLUDED.dry_run, source_filter = EXCLUDED.source_filter, updated_at = NOW()")
        || sql.starts_with("INSERT INTO client_sessions ")
            && sql.contains("ON CONFLICT (request_token) DO UPDATE SET updated_at = NOW()")
        || sql.starts_with("UPDATE index_scan_runs SET finished_at = NOW(), status = ")
        || sql.starts_with("UPDATE index_import_plans SET status = ")
        || sql.starts_with("UPDATE index_files SET source_changed = TRUE, updated_at = NOW() WHERE id_file = ")
        || sql.starts_with(
            "UPDATE payload_capacity_reservations SET expires_at = NOW() + INTERVAL '1 hour' WHERE request_token = $1",
        )
        || sql_is_replayable_data_blocks_upsert(&sql)
        || sql_is_replayable_copy_block_crc_upsert(&sql)
        || sql_is_replayable_schema_ddl(&sql)
        || sql.starts_with("UPDATE client_sessions")
        || sql.starts_with("UPDATE lock_leases")
        || sql.starts_with("UPDATE lock_range_leases")
        || sql.starts_with("INSERT INTO client_session_owner_keys ")
        || sql.starts_with("INSERT INTO lock_leases ")
            && sql.contains("ON CONFLICT (resource_kind, resource_id, session_id, owner_key, lease_kind) DO UPDATE SET lock_type = EXCLUDED.lock_type, lease_expires_at = EXCLUDED.lease_expires_at, heartbeat_at = EXCLUDED.heartbeat_at, updated_at = NOW()")
        || sql.starts_with("INSERT INTO lock_range_leases ")
        || sql.starts_with("UPDATE data_objects SET file_size = ")
        || sql.starts_with("UPDATE data_objects SET modification_date = NOW() WHERE id_data_object = $1")
        || sql.starts_with("UPDATE data_objects SET reference_count = reference_count + 1, modification_date = NOW() WHERE id_data_object = $1")
        || sql.starts_with("UPDATE data_objects SET reference_count = GREATEST(reference_count - 1, 0), modification_date = NOW() WHERE id_data_object = $1")
        || sql.starts_with(
            "UPDATE data_objects SET reference_count = 0, modification_date = NOW() WHERE id_data_object = $1",
        )
        || sql.starts_with("UPDATE files SET size = ")
        || sql.starts_with("UPDATE files SET data_object_id = $1, size = $2, change_date = NOW(), modification_date = NOW() WHERE id_file = $3")
        || sql.starts_with("UPDATE files SET modification_date = NOW(), change_date = NOW() WHERE id_file = $1")
        || sql.starts_with("UPDATE directories SET modification_date = NOW(), change_date = NOW() WHERE id_directory = $1")
        || sql.starts_with("UPDATE symlinks SET modification_date = NOW(), change_date = NOW() WHERE id_symlink = $1")
        || sql.starts_with("UPDATE files SET name = $1, id_directory = $2, change_date = NOW(), modification_date = NOW() WHERE id_file = $3")
        || sql.starts_with("UPDATE files SET name = $1, id_directory = NULL, change_date = NOW(), modification_date = NOW() WHERE id_file = $2")
        || sql.starts_with("UPDATE hardlinks SET name = $1, id_directory = $2, modification_date = NOW() WHERE id_hardlink = $3")
        || sql.starts_with("UPDATE hardlinks SET name = $1, id_directory = NULL, modification_date = NOW() WHERE id_hardlink = $2")
        || sql.starts_with("UPDATE symlinks SET name = $1, id_parent = $2, modification_date = NOW() WHERE id_symlink = $3")
        || sql.starts_with("UPDATE symlinks SET name = $1, id_parent = NULL, modification_date = NOW() WHERE id_symlink = $2")
        || sql.starts_with("UPDATE directories SET name = $1, id_parent = $2, modification_date = NOW(), change_date = NOW() WHERE id_directory = $3")
        || sql.starts_with("UPDATE directories SET name = $1, id_parent = NULL, modification_date = NOW(), change_date = NOW() WHERE id_directory = $2")
        || sql.starts_with("UPDATE files SET mode = $1, change_date = NOW(), modification_date = NOW() WHERE id_file = $2")
        || sql.starts_with("UPDATE directories SET mode = $1, change_date = NOW(), modification_date = NOW() WHERE id_directory = $2")
        || sql.starts_with("UPDATE files SET uid = $1, gid = $2, mode = $3, change_date = NOW(), modification_date = NOW() WHERE id_file = $4")
        || sql.starts_with("UPDATE directories SET uid = $1, gid = $2, mode = $3, change_date = NOW(), modification_date = NOW() WHERE id_directory = $4")
        || sql.starts_with("UPDATE symlinks SET uid = $1, gid = $2, change_date = NOW(), modification_date = NOW() WHERE id_symlink = $3")
        || sql.starts_with("UPDATE symlinks SET access_date = $1 WHERE id_symlink = $2")
        || sql.starts_with("UPDATE files SET access_date = $1, modification_date = $2, change_date = NOW() WHERE id_file = $3")
        || sql.starts_with("UPDATE directories SET access_date = $1, modification_date = $2, change_date = NOW() WHERE id_directory = $3")
        || sql.starts_with("UPDATE files SET access_date = $1 WHERE id_file = $2")
        || sql.starts_with("UPDATE directories SET access_date = $1 WHERE id_directory = $2")
        || sql.starts_with("INSERT INTO xattrs ")
}

fn sql_is_replayable_after_disconnect(sql: &CString) -> bool {
    sql_is_read_only(sql) || sql_is_replayable_command(sql)
}

unsafe fn maybe_replayable_sql_error(conn: *const PGconn, sql: &CString, err: String) -> String {
    if sql_is_replayable_after_disconnect(sql) && is_retryable_connection_error(conn, &err) {
        replayable_sql_error(err)
    } else {
        err
    }
}

unsafe fn maybe_replayable_prepared_error(
    conn: *const PGconn,
    statement: PreparedStatement,
    err: String,
) -> String {
    if statement.is_read_only() && is_retryable_connection_error(conn, &err) {
        replayable_sql_error(err)
    } else {
        err
    }
}

unsafe fn pgresult_profile_payload(res: *const PGresult, status: c_int) -> (u64, u64) {
    if res.is_null() || status != PGRES_TUPLES_OK {
        return (0, 0);
    }
    let rows = PQntuples(res).max(0);
    let cols = PQnfields(res).max(0);
    let mut bytes = 0u64;
    for row in 0..rows {
        for col in 0..cols {
            if PQgetisnull(res, row, col) != 0 {
                continue;
            }
            let len = PQgetlength(res, row, col);
            if len > 0 {
                bytes = bytes.saturating_add(len as u64);
            }
        }
    }
    (rows as u64, bytes)
}

unsafe fn exec_command(conn: *mut PGconn, sql: &CString) -> Result<(), String> {
    let started = Instant::now();
    let res = PQexec(conn, sql.as_ptr());
    let elapsed = started.elapsed();
    let sql_label = fod_sql_label(sql);

    if res.is_null() {
        fod_profile_sql_statement_observe(
            "exec_command",
            sql_label.clone(),
            elapsed,
            0,
            0,
            0,
            0,
            true,
        );
        fod_log_io_profile(
            "pg.exec_command.error",
            elapsed,
            0,
            0,
            format!("sql=\"{}\"", sql_label),
        );
        let err = conn_error(conn);
        if sql_is_replayable_command(sql) && is_retryable_connection_error(conn, &err) {
            return Err(replayable_sql_error_once(err));
        }
        return Err(err);
    }

    let status = PQresultStatus(res);
    let failed = status != PGRES_COMMAND_OK;
    fod_profile_sql_statement_observe(
        "exec_command",
        sql_label.clone(),
        elapsed,
        0,
        0,
        0,
        0,
        failed,
    );
    fod_log_io_profile(
        "pg.exec_command",
        elapsed,
        0,
        0,
        format!("status={} sql=\"{}\"", status, sql_label),
    );

    if status == PGRES_COMMAND_OK {
        PQclear(res);
        Ok(())
    } else {
        let error = result_error(res);
        PQclear(res);
        if sql_is_replayable_command(sql) && is_retryable_connection_error(conn, &error) {
            Err(replayable_sql_error_once(error))
        } else {
            Err(error)
        }
    }
}

unsafe fn exec_params(
    conn: *mut PGconn,
    sql: &CString,
    params: &[&CString],
) -> Result<*mut PGresult, String> {
    let param_values = params
        .iter()
        .map(|value| value.as_ptr())
        .collect::<Vec<_>>();
    let param_lengths = params
        .iter()
        .map(|value| value.as_bytes().len() as c_int)
        .collect::<Vec<_>>();
    let param_formats = vec![0 as c_int; params.len()];
    let param_bytes = param_lengths
        .iter()
        .map(|value| if *value > 0 { *value as usize } else { 0 })
        .sum::<usize>();

    let started = Instant::now();
    let res = PQexecParams(
        conn,
        sql.as_ptr(),
        params.len() as c_int,
        std::ptr::null(),
        param_values.as_ptr(),
        param_lengths.as_ptr(),
        param_formats.as_ptr(),
        0,
    );
    let elapsed = started.elapsed();
    let sql_label = fod_sql_label(sql);
    if fod_profile_io_enabled() {
        let (failed, result_rows, result_bytes) = if res.is_null() {
            (true, 0, 0)
        } else {
            let status = PQresultStatus(res);
            let failed = !matches!(status, PGRES_TUPLES_OK | PGRES_COMMAND_OK | PGRES_COPY_IN);
            let (result_rows, result_bytes) = pgresult_profile_payload(res, status);
            (failed, result_rows, result_bytes)
        };
        fod_profile_sql_statement_observe(
            "exec_params",
            sql_label.clone(),
            elapsed,
            params.len(),
            param_bytes,
            result_rows,
            result_bytes,
            failed,
        );
    }

    fod_log_io_profile(
        "pg.exec_params",
        elapsed,
        params.len(),
        param_bytes,
        format!("null_result={} sql=\"{}\"", res.is_null(), sql_label),
    );

    if res.is_null() {
        let err = conn_error(conn);
        Err(maybe_replayable_sql_error(conn, sql, err))
    } else if sql_is_read_only(sql) {
        let status = PQresultStatus(res);
        if status != PGRES_TUPLES_OK {
            let error = result_error(res);
            if is_retryable_connection_error(conn, &error) {
                PQclear(res);
                return Err(replayable_sql_error(error));
            }
        }
        Ok(res)
    } else {
        Ok(res)
    }
}

unsafe fn exec_command_params(
    conn: *mut PGconn,
    sql: &CString,
    params: &[&CString],
) -> Result<(), String> {
    let started = Instant::now();
    let res = match exec_params(conn, sql, params) {
        Ok(res) => res,
        Err(err) => {
            if sql_is_replayable_command(sql) && is_retryable_connection_error(conn, &err) {
                return Err(replayable_sql_error_once(err));
            }
            return Err(err);
        }
    };
    let elapsed = started.elapsed();
    let status = PQresultStatus(res);
    let sql_label = fod_sql_label(sql);

    fod_log_io_profile(
        "pg.exec_command_params",
        elapsed,
        params.len(),
        0,
        format!("status={} sql=\"{}\"", status, sql_label),
    );

    if status == PGRES_COMMAND_OK {
        PQclear(res);
        Ok(())
    } else {
        let error = result_error(res);
        PQclear(res);
        if sql_is_replayable_command(sql) && is_retryable_connection_error(conn, &error) {
            Err(replayable_sql_error_once(error))
        } else {
            Err(error)
        }
    }
}

enum SqlParam<'a> {
    Text(&'a CString),
    Binary(&'a [u8]),
}

impl<'a> SqlParam<'a> {
    fn ptr(&self) -> *const c_char {
        match self {
            Self::Text(value) => value.as_ptr(),
            Self::Binary(bytes) => bytes.as_ptr() as *const c_char,
        }
    }

    fn len(&self) -> c_int {
        match self {
            Self::Text(value) => value.as_bytes().len() as c_int,
            Self::Binary(bytes) => bytes.len() as c_int,
        }
    }

    fn format(&self) -> c_int {
        match self {
            Self::Text(_) => 0,
            Self::Binary(_) => 1,
        }
    }
}

unsafe fn exec_params_with_formats(
    conn: *mut PGconn,
    sql: &CString,
    params: &[SqlParam<'_>],
) -> Result<*mut PGresult, String> {
    let param_values = params.iter().map(SqlParam::ptr).collect::<Vec<_>>();
    let param_lengths = params.iter().map(SqlParam::len).collect::<Vec<_>>();
    let param_formats = params.iter().map(SqlParam::format).collect::<Vec<_>>();
    let param_bytes = param_lengths
        .iter()
        .map(|value| if *value > 0 { *value as usize } else { 0 })
        .sum::<usize>();

    let started = Instant::now();
    let res = PQexecParams(
        conn,
        sql.as_ptr(),
        params.len() as c_int,
        std::ptr::null(),
        param_values.as_ptr(),
        param_lengths.as_ptr(),
        param_formats.as_ptr(),
        0,
    );
    let elapsed = started.elapsed();
    let sql_label = fod_sql_label(sql);
    if fod_profile_io_enabled() {
        let (failed, result_rows, result_bytes) = if res.is_null() {
            (true, 0, 0)
        } else {
            let status = PQresultStatus(res);
            let failed = !matches!(status, PGRES_TUPLES_OK | PGRES_COMMAND_OK | PGRES_COPY_IN);
            let (result_rows, result_bytes) = pgresult_profile_payload(res, status);
            (failed, result_rows, result_bytes)
        };
        fod_profile_sql_statement_observe(
            "exec_params_with_formats",
            sql_label.clone(),
            elapsed,
            params.len(),
            param_bytes,
            result_rows,
            result_bytes,
            failed,
        );
    }

    fod_log_io_profile(
        "pg.exec_params_with_formats",
        elapsed,
        params.len(),
        param_bytes,
        format!("null_result={} sql=\"{}\"", res.is_null(), sql_label),
    );

    if res.is_null() {
        let err = conn_error(conn);
        Err(maybe_replayable_sql_error(conn, sql, err))
    } else {
        Ok(res)
    }
}

unsafe fn exec_command_params_with_formats(
    conn: *mut PGconn,
    sql: &CString,
    params: &[SqlParam<'_>],
) -> Result<(), String> {
    let started = Instant::now();
    let res = exec_params_with_formats(conn, sql, params)?;
    let elapsed = started.elapsed();
    let status = PQresultStatus(res);
    let sql_label = fod_sql_label(sql);

    fod_log_io_profile(
        "pg.exec_command_params_with_formats",
        elapsed,
        params.len(),
        0,
        format!("status={} sql=\"{}\"", status, sql_label),
    );

    if status == PGRES_COMMAND_OK {
        PQclear(res);
        Ok(())
    } else {
        let error = result_error(res);
        PQclear(res);
        if sql_is_replayable_command(sql) && is_retryable_connection_error(conn, &error) {
            Err(replayable_sql_error_once(error))
        } else {
            Err(error)
        }
    }
}

fn hex_encode_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn escape_copy_text_field(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            other => out.push(other),
        }
    }
    out
}

fn append_copy_text_null_field(out: &mut String) {
    out.push_str("\\N");
}

fn append_copy_text_string_field(out: &mut String, value: &str) {
    out.push_str(&escape_copy_text_field(value));
}

fn append_copy_text_i64_field(out: &mut String, value: i64) {
    out.push_str(&value.to_string());
}

fn append_copy_text_u64_field(out: &mut String, value: u64) {
    out.push_str(&value.to_string());
}

fn append_copy_text_bool_field(out: &mut String, value: bool) {
    out.push_str(if value { "t" } else { "f" });
}

fn append_copy_text_optional_i64_field(out: &mut String, value: Option<i64>) {
    match value {
        Some(value) => append_copy_text_i64_field(out, value),
        None => append_copy_text_null_field(out),
    }
}

fn append_copy_text_optional_u64_field(out: &mut String, value: Option<u64>) {
    match value {
        Some(value) => append_copy_text_u64_field(out, value),
        None => append_copy_text_null_field(out),
    }
}

fn append_copy_text_bytea_field(out: &mut String, value: &[u8]) {
    out.push_str("\\\\x");
    out.push_str(&hex_encode_bytes(value));
}

fn build_copy_text_payload<T, F>(rows: &[T], mut write_row: F) -> String
where
    F: FnMut(&T, &mut String),
{
    let mut out = String::new();
    for row in rows {
        write_row(row, &mut out);
        out.push('\n');
    }
    out
}

fn normalize_block_bytes(bytes: &[u8], target_len: usize) -> Vec<u8> {
    let payload_len = bytes.len().min(target_len);
    let mut out = Vec::with_capacity(target_len);
    out.extend_from_slice(&bytes[..payload_len]);
    if payload_len < target_len {
        out.resize(target_len, 0);
    }
    out
}

fn normalized_block_bytes_cow(bytes: &[u8], target_len: usize) -> Cow<'_, [u8]> {
    let payload_len = bytes.len().min(target_len);
    if payload_len == target_len {
        Cow::Borrowed(&bytes[..target_len])
    } else {
        Cow::Owned(normalize_block_bytes(bytes, target_len))
    }
}

fn persist_blocks_cover_full_file(
    file_size: u64,
    block_size: u64,
    total_blocks: u64,
    blocks: &[PersistBlockRow<'_>],
) -> bool {
    if file_size == 0 {
        return false;
    }

    let block_size = block_size.max(1);
    let expected_total_blocks = 1 + (file_size - 1) / block_size;
    if total_blocks != expected_total_blocks {
        return false;
    }
    if u64::try_from(blocks.len()).ok() != Some(expected_total_blocks) {
        return false;
    }

    for (expected_block, block) in blocks.iter().enumerate() {
        let expected_block = expected_block as u64;
        if block.block_index != expected_block {
            return false;
        }

        let block_start = expected_block.saturating_mul(block_size);
        let expected_used_len = file_size
            .min(block_start.saturating_add(block_size))
            .saturating_sub(block_start);
        if block.used_len != expected_used_len {
            return false;
        }
    }

    true
}

fn append_copy_binary_i16(out: &mut Vec<u8>, value: i16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn append_copy_binary_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn append_copy_binary_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn append_copy_binary_i64_field(out: &mut Vec<u8>, value: i64) {
    append_copy_binary_i32(out, 8);
    append_copy_binary_i64(out, value);
}

fn append_copy_binary_null_field(out: &mut Vec<u8>) {
    append_copy_binary_i32(out, -1);
}

fn append_copy_binary_padded_bytes_field(
    out: &mut Vec<u8>,
    bytes: &[u8],
    target_len: usize,
) -> Result<(), String> {
    let payload_len = bytes.len().min(target_len);
    let len = i32::try_from(target_len).map_err(|_| "copy field too large".to_string())?;
    append_copy_binary_i32(out, len);
    out.extend_from_slice(&bytes[..payload_len]);
    if payload_len < target_len {
        out.resize(out.len() + (target_len - payload_len), 0);
    }
    Ok(())
}

fn append_copy_binary_header(out: &mut Vec<u8>) {
    out.extend_from_slice(COPY_BINARY_SIGNATURE);
    append_copy_binary_i32(out, 0);
    append_copy_binary_i32(out, 0);
}

fn append_persist_block_copy_binary_row(
    out: &mut Vec<u8>,
    data_object_id: i64,
    block: &PersistBlockRow<'_>,
    block_size: u64,
) -> Result<(), String> {
    let block_size = block_size.max(1);
    let block_index = i64::try_from(block.block_index)
        .map_err(|_| "block index out of range for copy staging".to_string())?;
    let normalized = normalized_block_bytes_cow(block.data, block_size as usize);
    let normalized = normalized.as_ref();
    let sparse = is_all_zero_block(normalized);
    let crc32 = if !sparse && block.used_len >= block_size {
        Some(i64::from(crc32_bytes(normalized)))
    } else {
        None
    };

    append_copy_binary_i16(out, 4);
    append_copy_binary_i64_field(out, data_object_id);
    append_copy_binary_i64_field(out, block_index);
    if sparse {
        append_copy_binary_null_field(out);
    } else {
        append_copy_binary_padded_bytes_field(out, normalized, block_size as usize)?;
    }
    if let Some(crc32) = crc32 {
        append_copy_binary_i64_field(out, crc32);
    } else {
        append_copy_binary_null_field(out);
    }

    Ok(())
}

unsafe fn create_persist_block_stage_table(conn: *mut PGconn) -> Result<(), String> {
    let sql = CString::new(format!(
        "CREATE TEMP TABLE IF NOT EXISTS {} (data_object_id BIGINT NOT NULL, _order BIGINT NOT NULL, data BYTEA, crc32 BIGINT) ON COMMIT DROP; TRUNCATE {}",
        PERSIST_BLOCK_STAGE_TABLE, PERSIST_BLOCK_STAGE_TABLE
    ))
    .map_err(|_| "SQL contains NUL byte".to_string())?;
    exec_command(conn, &sql)
}

unsafe fn merge_persist_block_stage_table(
    conn: *mut PGconn,
    maintain_copy_crc_table: bool,
    target_data_object_is_empty: bool,
) -> Result<(), String> {
    let sql_insert_data = if target_data_object_is_empty {
        CString::new(format!(
            "
        INSERT INTO data_blocks (data_object_id, _order, data)
        SELECT data_object_id, _order, data
        FROM {}
        WHERE data IS NOT NULL
        ",
            PERSIST_BLOCK_STAGE_TABLE
        ))
    } else {
        CString::new(format!(
            "
        INSERT INTO data_blocks (data_object_id, _order, data)
        SELECT data_object_id, _order, data
        FROM {}
        WHERE data IS NOT NULL
        ON CONFLICT (data_object_id, _order)
        DO UPDATE SET data = EXCLUDED.data
        WHERE data_blocks.data IS DISTINCT FROM EXCLUDED.data
        ",
            PERSIST_BLOCK_STAGE_TABLE
        ))
    }
    .map_err(|_| "SQL contains NUL byte".to_string())?;
    let sql_delete_data = CString::new(format!(
        "
        DELETE FROM data_blocks
        USING {}
        WHERE data_blocks.data_object_id = {}.data_object_id
          AND data_blocks._order = {}._order
          AND {}.data IS NULL
        ",
        PERSIST_BLOCK_STAGE_TABLE,
        PERSIST_BLOCK_STAGE_TABLE,
        PERSIST_BLOCK_STAGE_TABLE,
        PERSIST_BLOCK_STAGE_TABLE
    ))
    .map_err(|_| "SQL contains NUL byte".to_string())?;
    let sql_insert_crc = if target_data_object_is_empty {
        CString::new(format!(
            "
        INSERT INTO copy_block_crc (data_object_id, _order, crc32)
        SELECT data_object_id, _order, crc32
        FROM {}
        WHERE crc32 IS NOT NULL
        ",
            PERSIST_BLOCK_STAGE_TABLE
        ))
    } else {
        CString::new(format!(
            "
        INSERT INTO copy_block_crc (data_object_id, _order, crc32)
        SELECT data_object_id, _order, crc32
        FROM {}
        WHERE crc32 IS NOT NULL
        ON CONFLICT (data_object_id, _order)
        DO UPDATE SET crc32 = EXCLUDED.crc32, updated_at = NOW()
        ",
            PERSIST_BLOCK_STAGE_TABLE
        ))
    }
    .map_err(|_| "SQL contains NUL byte".to_string())?;
    let sql_delete_crc = CString::new(format!(
        "
        DELETE FROM copy_block_crc
        USING {}
        WHERE copy_block_crc.data_object_id = {}.data_object_id
          AND copy_block_crc._order = {}._order
          AND {}.crc32 IS NULL
        ",
        PERSIST_BLOCK_STAGE_TABLE,
        PERSIST_BLOCK_STAGE_TABLE,
        PERSIST_BLOCK_STAGE_TABLE,
        PERSIST_BLOCK_STAGE_TABLE
    ))
    .map_err(|_| "SQL contains NUL byte".to_string())?;

    exec_command(conn, &sql_insert_data)?;
    if !target_data_object_is_empty {
        exec_command(conn, &sql_delete_data)?;
    }
    if maintain_copy_crc_table {
        exec_command(conn, &sql_insert_crc)?;
        if !target_data_object_is_empty {
            exec_command(conn, &sql_delete_crc)?;
        }
    }
    Ok(())
}

struct CopyInSession {
    conn: *mut PGconn,
    finished: bool,
}

impl CopyInSession {
    unsafe fn start(conn: *mut PGconn, sql: &CString) -> Result<Self, String> {
        let res = PQexec(conn, sql.as_ptr());
        if res.is_null() {
            return Err(conn_error(conn));
        }
        let status = PQresultStatus(res);
        if status != PGRES_COPY_IN {
            let error = result_error(res);
            PQclear(res);
            return Err(error);
        }
        PQclear(res);
        Ok(Self {
            conn,
            finished: false,
        })
    }

    unsafe fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        let res = fod_profiled_pq_put_copy_data(
            self.conn,
            bytes.as_ptr() as *const c_char,
            bytes.len() as c_int,
        );
        if res == 1 {
            Ok(())
        } else {
            Err(conn_error(self.conn))
        }
    }

    unsafe fn finish(mut self) -> Result<(), String> {
        let res = fod_profiled_pq_put_copy_end(self.conn, std::ptr::null());
        if res != 1 {
            return Err(conn_error(self.conn));
        }
        self.finished = true;
        loop {
            let res = fod_profiled_pq_get_result(self.conn);
            if res.is_null() {
                break Ok(());
            }
            let status = PQresultStatus(res);
            if status != PGRES_COMMAND_OK {
                let error = result_error(res);
                PQclear(res);
                break Err(error);
            }
            PQclear(res);
        }
    }

    unsafe fn abort(&mut self) {
        if let Ok(message) = CString::new("copy staging aborted") {
            let _ = fod_profiled_pq_put_copy_end(self.conn, message.as_ptr());
        }
        loop {
            let res = fod_profiled_pq_get_result(self.conn);
            if res.is_null() {
                break;
            }
            PQclear(res);
        }
    }
}

unsafe fn flush_copy_send_buffer_if_full(
    copy: &mut CopyInSession,
    buffer: &mut Vec<u8>,
) -> Result<(), String> {
    if buffer.len() >= persist_copy_send_buffer_bytes() {
        copy.send(buffer)?;
        buffer.clear();
    }
    Ok(())
}

unsafe fn flush_copy_send_buffer(
    copy: &mut CopyInSession,
    buffer: &mut Vec<u8>,
) -> Result<(), String> {
    if !buffer.is_empty() {
        copy.send(buffer)?;
        buffer.clear();
    }
    Ok(())
}

unsafe fn copy_text_payload_on_conn(
    conn: *mut PGconn,
    copy_sql: &CString,
    payload: &str,
) -> Result<(), String> {
    let mut copy = CopyInSession::start(conn, copy_sql)?;
    if !payload.is_empty() {
        copy.send(payload.as_bytes())?;
    }
    copy.finish()
}

unsafe fn create_or_reset_temp_table(conn: *mut PGconn, sql: &CString) -> Result<(), String> {
    exec_command(conn, sql)
}

impl Drop for CopyInSession {
    fn drop(&mut self) {
        if !self.finished {
            unsafe {
                self.abort();
            }
        }
    }
}

unsafe fn transactional_impl<T, F>(
    conn: *mut PGconn,
    mut f: F,
    replay_commit_disconnect: bool,
) -> Result<T, String>
where
    F: FnMut(*mut PGconn) -> Result<T, String>,
{
    let _transaction_admission = acquire_current_transaction_admission()?;
    let begin = CString::new("BEGIN").map_err(|_| "SQL contains NUL byte".to_string())?;
    let commit = CString::new("COMMIT").map_err(|_| "SQL contains NUL byte".to_string())?;
    let rollback = CString::new("ROLLBACK").map_err(|_| "SQL contains NUL byte".to_string())?;
    let started = Instant::now();
    let result = match exec_command(conn, &begin) {
        Ok(()) => match f(conn) {
            Ok(value) => {
                if let Err(err) = exec_command(conn, &commit) {
                    let _ = exec_command(conn, &rollback);
                    if replay_commit_disconnect && is_retryable_connection_error(conn, &err) {
                        Err(replayable_sql_error_once(err))
                    } else {
                        Err(err)
                    }
                } else {
                    Ok(value)
                }
            }
            Err(err) => {
                let _ = exec_command(conn, &rollback);
                if is_retryable_connection_error(conn, &err) {
                    Err(replayable_sql_error(err))
                } else {
                    Err(err)
                }
            }
        },
        Err(err) => {
            if is_retryable_connection_error(conn, &err) {
                Err(replayable_sql_error(err))
            } else {
                Err(err)
            }
        }
    };
    observe_current_transaction(started.elapsed(), result.is_err());
    result
}

unsafe fn transactional_replayable<T, F>(conn: *mut PGconn, f: F) -> Result<T, String>
where
    F: FnMut(*mut PGconn) -> Result<T, String>,
{
    transactional_impl(conn, f, true)
}

/// Probe a durable outcome during the transaction so a committed result can be
/// confirmed after a lost COMMIT acknowledgement without replaying the body.
unsafe fn transactional_replay_confirmed<T, Probe, Body>(
    conn: *mut PGconn,
    mut probe: Probe,
    mut body: Body,
) -> Result<T, String>
where
    Probe: FnMut(*mut PGconn) -> Result<Option<T>, String>,
    Body: FnMut(*mut PGconn) -> Result<T, String>,
{
    transactional_replayable(conn, |conn| {
        if let Some(value) = probe(conn)? {
            return Ok(value);
        }
        body(conn)
    })
}

/// DbRepo keeps separate cached connections for write-heavy work and control-plane
/// lease/session maintenance so long flushes do not starve heartbeats or cleanup.
pub struct DbRepo {
    connection_targets: Arc<DbRepoConnectionTargets>,
    replica_read_targets: Option<Arc<DbRepoConnectionTargets>>,
    replica_read_pool: Option<Arc<SharedConnectionPool>>,
    replica_consistency: Arc<ReplicaReadConsistency>,
    connection_tuning: ConnectionTuning,
    persist_buffer_chunk_blocks: u64,
    persist_block_transport: PersistBlockTransport,
    data_object_swap_cleanup: DataObjectSwapCleanup,
    pool: Arc<SharedConnectionPool>,
    payload_observability: Arc<DbRepoPayloadObservability>,
    global_payload_observability: Arc<DbRepoPayloadObservability>,
    lock_session_id: Mutex<i64>,
    lock_schema_ready: Mutex<bool>,
    owner_session_cache: Mutex<HashMap<u64, i64>>,
}

impl Clone for DbRepo {
    fn clone(&self) -> Self {
        Self {
            connection_targets: Arc::clone(&self.connection_targets),
            replica_read_targets: self.replica_read_targets.as_ref().map(Arc::clone),
            replica_read_pool: self.replica_read_pool.as_ref().map(Arc::clone),
            replica_consistency: Arc::clone(&self.replica_consistency),
            connection_tuning: self.connection_tuning.clone(),
            persist_buffer_chunk_blocks: self.persist_buffer_chunk_blocks,
            persist_block_transport: self.persist_block_transport,
            data_object_swap_cleanup: self.data_object_swap_cleanup,
            pool: Arc::clone(&self.pool),
            payload_observability: Arc::clone(&self.payload_observability),
            global_payload_observability: Arc::clone(&self.global_payload_observability),
            lock_session_id: Mutex::new(
                self.lock_session_id
                    .lock()
                    .map(|guard| *guard)
                    .unwrap_or_else(|_| NEXT_LOCK_SESSION_ID.fetch_sub(1, Ordering::Relaxed)),
            ),
            lock_schema_ready: Mutex::new(
                self.lock_schema_ready
                    .lock()
                    .map(|guard| *guard)
                    .unwrap_or(false),
            ),
            owner_session_cache: Mutex::new(
                self.owner_session_cache
                    .lock()
                    .map(|guard| guard.clone())
                    .unwrap_or_default(),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupSnapshot {
    pub block_size: Option<u32>,
    pub is_in_recovery: bool,
    pub schema_version: Option<u32>,
    pub schema_is_initialized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbRepoSourceSnapshot {
    pub data_source_role: String,
    pub transaction_read_only: bool,
    pub wal_replay_lag_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    pub parent_id: Option<u64>,
    pub kind: Option<String>,
    pub entry_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReadMetadata {
    pub size: u64,
    pub access_date: String,
    pub modification_date: String,
    pub change_date: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistBlockRow<'a> {
    pub block_index: u64,
    pub data: &'a [u8],
    pub used_len: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplacedDataObject {
    old_data_object_id: u64,
    old_reference_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DataObjectWriteTarget {
    data_object_id: u64,
    replaced: Option<ReplacedDataObject>,
    payload_rows_empty: bool,
}

fn persist_block_input_bytes(blocks: &[PersistBlockRow<'_>]) -> u64 {
    blocks.iter().fold(0u64, |total, block| {
        total.saturating_add(block.data.len() as u64)
    })
}

pub const STORAGE_QUOTA_EXCEEDED_PREFIX: &str = "FOD_ENOSPC:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSourceStageRow {
    pub name: String,
    pub kind: String,
    pub root_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexScanRunStageRow {
    pub id_index_source: u64,
    pub status: String,
    pub request_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexImportPlanStageRow {
    pub status: String,
    pub request_token: String,
    pub dry_run: bool,
    pub source_filter: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexFileStageRow {
    pub id_index_source: u64,
    pub id_scan_run: u64,
    pub path: String,
    pub size: u64,
    pub mtime_ns: Option<i64>,
    pub inode: Option<u64>,
    pub device: Option<u64>,
    pub file_kind: String,
    pub scan_status: String,
    pub source_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexFileHashStageRow {
    pub id_file: u64,
    pub hash_algorithm: String,
    pub partial_hash: Option<Vec<u8>>,
    pub full_hash: Option<Vec<u8>>,
    pub hash_status: String,
    pub observed_size: u64,
    pub observed_mtime_ns: Option<i64>,
    pub observed_inode: Option<u64>,
    pub observed_device: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexImportPlanEntryStageRow {
    pub id_import_plan: u64,
    pub id_file: u64,
    pub id_duplicate_set: Option<u64>,
    pub action: String,
    pub canonical_file_id: Option<u64>,
    pub logical_path: String,
    pub source_path: String,
    pub size: u64,
    pub mtime_ns: Option<i64>,
    pub source_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateSetStageRow {
    pub hash_algorithm: String,
    pub full_hash: Vec<u8>,
    pub file_size: u64,
    pub file_count: u64,
    pub total_bytes: u64,
}

fn is_all_zero_block(data: &[u8]) -> bool {
    // Keep fully zero blocks sparse; the read path already materializes
    // missing blocks as zero-filled ranges.
    data.iter().all(|byte| *byte == 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionLane {
    Write,
    Control,
}

impl ConnectionLane {
    fn other(self) -> Self {
        match self {
            Self::Write => Self::Control,
            Self::Control => Self::Write,
        }
    }
}

struct PayloadPersistGuard {
    _budget_permit: DbRepoPayloadBudgetPermit,
    local: Arc<DbRepoPayloadObservability>,
    global: Option<Arc<DbRepoPayloadObservability>>,
    local_started: bool,
    global_started: bool,
    input_bytes: u64,
    input_rows: u64,
    started: Instant,
    failed: bool,
}

impl PayloadPersistGuard {
    fn new(
        local: Arc<DbRepoPayloadObservability>,
        global: Arc<DbRepoPayloadObservability>,
        input_bytes: u64,
        input_rows: u64,
    ) -> Self {
        let budget_permit = global.acquire_payload_budget(input_bytes);
        let local_started = local.begin_persist(input_bytes);
        let global = (!Arc::ptr_eq(&local, &global)).then_some(global);
        let global_started = global
            .as_ref()
            .is_some_and(|tracker| tracker.begin_persist(input_bytes));
        Self {
            _budget_permit: budget_permit,
            local,
            global,
            local_started,
            global_started,
            input_bytes,
            input_rows,
            started: Instant::now(),
            failed: true,
        }
    }

    fn mark_success(&mut self) {
        self.failed = false;
    }
}

impl Drop for PayloadPersistGuard {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        if self.local_started {
            self.local
                .finish_persist(self.input_bytes, self.input_rows, elapsed, self.failed);
        }
        if self.global_started {
            if let Some(global) = &self.global {
                global.finish_persist(self.input_bytes, self.input_rows, elapsed, self.failed);
            }
        }
    }
}

struct PayloadQuotaLockObservation {
    quota_bytes: Option<u64>,
    local: Arc<DbRepoPayloadObservability>,
    global: Option<Arc<DbRepoPayloadObservability>>,
    acquired_at: Instant,
}

impl PayloadQuotaLockObservation {
    fn new(
        local: Arc<DbRepoPayloadObservability>,
        global: Arc<DbRepoPayloadObservability>,
        acquired_at: Instant,
    ) -> Self {
        let global = (!Arc::ptr_eq(&local, &global)).then_some(global);
        Self {
            quota_bytes: None,
            local,
            global,
            acquired_at,
        }
    }

    fn quota_bytes(&self) -> Option<u64> {
        self.quota_bytes
    }

    fn set_quota_bytes(&mut self, quota_bytes: Option<u64>) {
        self.quota_bytes = quota_bytes;
    }
}

impl Drop for PayloadQuotaLockObservation {
    fn drop(&mut self) {
        let elapsed = self.acquired_at.elapsed();
        self.local.record_quota_lock_held(elapsed);
        if let Some(global) = &self.global {
            global.record_quota_lock_held(elapsed);
        }
    }
}

#[derive(Debug)]
struct TransactionAdmissionWaiter {
    ticket: u64,
    ready: Mutex<bool>,
    ready_changed: Condvar,
}

impl TransactionAdmissionWaiter {
    fn new(ticket: u64) -> Self {
        Self {
            ticket,
            ready: Mutex::new(false),
            ready_changed: Condvar::new(),
        }
    }

    fn wait(&self) {
        let mut ready = self.ready.lock().unwrap_or_else(|err| err.into_inner());
        while !*ready {
            ready = self
                .ready_changed
                .wait(ready)
                .unwrap_or_else(|err| err.into_inner());
        }
    }

    fn signal(&self) {
        let mut ready = self.ready.lock().unwrap_or_else(|err| err.into_inner());
        *ready = true;
        drop(ready);
        self.ready_changed.notify_one();
    }
}

#[derive(Debug, Default)]
struct TransactionAdmissionState {
    active: u64,
    queued: u64,
    peak_active: u64,
    peak_queued: u64,
    next_ticket: u64,
    serving_ticket: u64,
    waiters: VecDeque<Arc<TransactionAdmissionWaiter>>,
    admission_count: u64,
    backpressure_events: u64,
    fairness_yields: u64,
    accounting_errors: u64,
}

#[derive(Debug)]
struct TransactionAdmissionGate {
    limit: u64,
    state: Mutex<TransactionAdmissionState>,
}

#[derive(Debug)]
struct TransactionAdmissionPermit {
    gate: Arc<TransactionAdmissionGate>,
    acquired: bool,
}

impl TransactionAdmissionGate {
    fn new(limit: u64) -> Self {
        Self {
            limit,
            state: Mutex::new(TransactionAdmissionState::default()),
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, TransactionAdmissionState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(err) => {
                let mut state = err.into_inner();
                state.accounting_errors = state.accounting_errors.saturating_add(1);
                state
            }
        }
    }

    fn reserve_next_waiter(
        &self,
        state: &mut TransactionAdmissionState,
    ) -> Option<Arc<TransactionAdmissionWaiter>> {
        if state.active >= self.limit {
            return None;
        }
        let waiter = state.waiters.pop_front()?;
        if waiter.ticket != state.serving_ticket {
            state.accounting_errors = state.accounting_errors.saturating_add(1);
            state.serving_ticket = waiter.ticket;
        }
        match state.queued.checked_sub(1) {
            Some(queued) => state.queued = queued,
            None => state.accounting_errors = state.accounting_errors.saturating_add(1),
        }
        state.active = state.active.saturating_add(1);
        state.peak_active = state.peak_active.max(state.active);
        state.admission_count = state.admission_count.saturating_add(1);
        state.serving_ticket = state.serving_ticket.wrapping_add(1);
        Some(waiter)
    }

    fn acquire(self: &Arc<Self>) -> TransactionAdmissionPermit {
        if self.limit == 0 {
            return TransactionAdmissionPermit {
                gate: Arc::clone(self),
                acquired: false,
            };
        }

        let mut state = self.lock_state();
        let ticket = state.next_ticket;
        state.next_ticket = state.next_ticket.wrapping_add(1);
        let had_older_waiter = !state.waiters.is_empty();
        let capacity_full = state.active >= self.limit;
        let waiter = Arc::new(TransactionAdmissionWaiter::new(ticket));
        state.waiters.push_back(Arc::clone(&waiter));
        state.queued = state.queued.saturating_add(1);
        state.peak_queued = state.peak_queued.max(state.queued);
        if capacity_full || had_older_waiter {
            state.backpressure_events = state.backpressure_events.saturating_add(1);
        }
        if had_older_waiter {
            state.fairness_yields = state.fairness_yields.saturating_add(1);
        }
        let waiter_to_wake = self.reserve_next_waiter(&mut state);
        drop(state);

        if let Some(waiter_to_wake) = waiter_to_wake {
            waiter_to_wake.signal();
        }
        waiter.wait();

        TransactionAdmissionPermit {
            gate: Arc::clone(self),
            acquired: true,
        }
    }

    fn snapshot(&self) -> DbRepoTransactionAdmissionSnapshot {
        let state = self.lock_state();
        DbRepoTransactionAdmissionSnapshot {
            limit: self.limit,
            active: state.active,
            queued: state.queued,
            peak_active: state.peak_active,
            peak_queued: state.peak_queued,
            admission_count: state.admission_count,
            backpressure_events: state.backpressure_events,
            fairness_yields: state.fairness_yields,
            accounting_errors: state.accounting_errors,
        }
    }
}

impl Drop for TransactionAdmissionPermit {
    fn drop(&mut self) {
        if !self.acquired {
            return;
        }
        let mut state = self.gate.lock_state();
        match state.active.checked_sub(1) {
            Some(active) => state.active = active,
            None => {
                state.accounting_errors = state.accounting_errors.saturating_add(1);
                return;
            }
        }
        let waiter_to_wake = self.gate.reserve_next_waiter(&mut state);
        drop(state);
        if let Some(waiter_to_wake) = waiter_to_wake {
            waiter_to_wake.signal();
        }
    }
}

#[derive(Debug)]
struct CachedConnection {
    conn: usize,
    generation: u64,
}

#[derive(Debug, Default)]
struct SharedConnectionPoolState {
    cached_conn: Vec<CachedConnection>,
    control_cached_conn: Vec<CachedConnection>,
    live_connections: usize,
    active_connections: usize,
    queued_acquisitions: usize,
    peak_active_connections: usize,
    peak_queued_acquisitions: usize,
    acquisition_count: u64,
    acquisition_wait_micros_total: u64,
    acquisition_wait_micros_max: u64,
    connection_create_count: u64,
    connection_create_failures: u64,
    connection_create_micros_total: u64,
    connection_create_micros_max: u64,
    operation_count: u64,
    operation_failures: u64,
    operation_micros_total: u64,
    operation_micros_max: u64,
    replay_count: u64,
    stale_connection_discards: u64,
    transaction_count: u64,
    transaction_failures: u64,
    transaction_micros_total: u64,
    transaction_micros_max: u64,
    heartbeat_count: u64,
    heartbeat_failures: u64,
    heartbeat_schedule_delay_micros_total: u64,
    heartbeat_schedule_delay_micros_max: u64,
    heartbeat_execution_micros_total: u64,
    heartbeat_execution_micros_max: u64,
}

#[derive(Debug)]
struct SharedConnectionPool {
    state: Mutex<SharedConnectionPoolState>,
    available: Condvar,
    limit: usize,
    write_transaction_gate: Arc<TransactionAdmissionGate>,
    control_transaction_gate: Arc<TransactionAdmissionGate>,
}

#[derive(Clone)]
struct CurrentTransactionContext {
    pool: Arc<SharedConnectionPool>,
    lane: ConnectionLane,
}

thread_local! {
    static CURRENT_TRANSACTION_CONTEXT: RefCell<Option<CurrentTransactionContext>> =
        const { RefCell::new(None) };
}

struct CurrentTransactionPoolScope {
    previous: Option<CurrentTransactionContext>,
}

impl CurrentTransactionPoolScope {
    fn enter(pool: Arc<SharedConnectionPool>, lane: ConnectionLane) -> Self {
        let previous = CURRENT_TRANSACTION_CONTEXT
            .with(|current| current.replace(Some(CurrentTransactionContext { pool, lane })));
        Self { previous }
    }
}

impl Drop for CurrentTransactionPoolScope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT_TRANSACTION_CONTEXT.with(|current| {
            current.replace(previous);
        });
    }
}

fn acquire_current_transaction_admission() -> Result<Option<TransactionAdmissionPermit>, String> {
    CURRENT_TRANSACTION_CONTEXT.with(|current| {
        current
            .borrow()
            .as_ref()
            .map(|context| context.pool.acquire_transaction(context.lane))
            .transpose()
    })
}

fn observe_current_transaction(elapsed: Duration, failed: bool) {
    CURRENT_TRANSACTION_CONTEXT.with(|current| {
        if let Some(context) = current.borrow().as_ref() {
            context.pool.record_transaction(elapsed, failed);
        }
    });
}

#[derive(Debug)]
enum ConnectionAcquisition {
    Cached { conn: *mut PGconn, generation: u64 },
    ReservedSlot,
}

impl SharedConnectionPool {
    fn new(limit: usize, transaction_settings: RuntimeTransactionSettings) -> Self {
        Self {
            state: Mutex::new(SharedConnectionPoolState::default()),
            available: Condvar::new(),
            limit: limit.max(1),
            write_transaction_gate: Arc::new(TransactionAdmissionGate::new(
                transaction_settings.write_active_limit,
            )),
            control_transaction_gate: Arc::new(TransactionAdmissionGate::new(
                transaction_settings.control_active_limit,
            )),
        }
    }

    fn transaction_gate(&self, lane: ConnectionLane) -> Arc<TransactionAdmissionGate> {
        match lane {
            ConnectionLane::Write => Arc::clone(&self.write_transaction_gate),
            ConnectionLane::Control => Arc::clone(&self.control_transaction_gate),
        }
    }

    fn acquire_transaction(
        &self,
        lane: ConnectionLane,
    ) -> Result<TransactionAdmissionPermit, String> {
        Ok(self.transaction_gate(lane).acquire())
    }

    fn take_cached(
        state: &mut SharedConnectionPoolState,
        lane: ConnectionLane,
        generation: u64,
    ) -> Result<Option<CachedConnection>, String> {
        loop {
            let cached = match lane {
                ConnectionLane::Write => state.cached_conn.pop(),
                ConnectionLane::Control => state.control_cached_conn.pop(),
            };
            let Some(cached) = cached else {
                return Ok(None);
            };
            if cached.generation == generation {
                return Ok(Some(cached));
            }
            state.live_connections = state
                .live_connections
                .checked_sub(1)
                .ok_or_else(|| "connection pool live connection underflow".to_string())?;
            state.stale_connection_discards = state.stale_connection_discards.saturating_add(1);
            unsafe {
                PQfinish(cached.conn as *mut PGconn);
            }
        }
    }

    fn push_cached(
        state: &mut SharedConnectionPoolState,
        lane: ConnectionLane,
        conn: *mut PGconn,
        generation: u64,
    ) {
        let cached = CachedConnection {
            conn: conn as usize,
            generation,
        };
        match lane {
            ConnectionLane::Write => state.cached_conn.push(cached),
            ConnectionLane::Control => state.control_cached_conn.push(cached),
        }
    }

    fn duration_micros(elapsed: Duration) -> u64 {
        elapsed.as_micros().min(u64::MAX as u128) as u64
    }

    fn finish_acquisition(
        state: &mut SharedConnectionPoolState,
        started: Instant,
        queued: bool,
    ) -> Result<(), String> {
        if queued {
            state.queued_acquisitions = state
                .queued_acquisitions
                .checked_sub(1)
                .ok_or_else(|| "connection pool queued acquisition underflow".to_string())?;
        }
        state.active_connections = state.active_connections.saturating_add(1);
        state.peak_active_connections = state.peak_active_connections.max(state.active_connections);
        state.acquisition_count = state.acquisition_count.saturating_add(1);
        let elapsed_micros = Self::duration_micros(started.elapsed());
        state.acquisition_wait_micros_total = state
            .acquisition_wait_micros_total
            .saturating_add(elapsed_micros);
        state.acquisition_wait_micros_max = state.acquisition_wait_micros_max.max(elapsed_micros);
        Ok(())
    }

    fn finish_operation(state: &mut SharedConnectionPoolState, elapsed: Duration, failed: bool) {
        state.operation_count = state.operation_count.saturating_add(1);
        if failed {
            state.operation_failures = state.operation_failures.saturating_add(1);
        }
        let elapsed_micros = Self::duration_micros(elapsed);
        state.operation_micros_total = state.operation_micros_total.saturating_add(elapsed_micros);
        state.operation_micros_max = state.operation_micros_max.max(elapsed_micros);
    }

    fn release_active(state: &mut SharedConnectionPoolState) -> Result<(), String> {
        state.active_connections = state
            .active_connections
            .checked_sub(1)
            .ok_or_else(|| "connection pool active connection underflow".to_string())?;
        Ok(())
    }

    fn release_live(state: &mut SharedConnectionPoolState) -> Result<(), String> {
        state.live_connections = state
            .live_connections
            .checked_sub(1)
            .ok_or_else(|| "connection pool live connection underflow".to_string())?;
        Ok(())
    }

    fn acquire(
        &self,
        lane: ConnectionLane,
        generation: u64,
    ) -> Result<ConnectionAcquisition, String> {
        let started = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "connection pool is poisoned".to_string())?;
        let mut queued = false;
        loop {
            if let Some(cached) = Self::take_cached(&mut state, lane, generation)? {
                Self::finish_acquisition(&mut state, started, queued)?;
                return Ok(ConnectionAcquisition::Cached {
                    conn: cached.conn as *mut PGconn,
                    generation: cached.generation,
                });
            }
            if let Some(cached) = Self::take_cached(&mut state, lane.other(), generation)? {
                Self::finish_acquisition(&mut state, started, queued)?;
                return Ok(ConnectionAcquisition::Cached {
                    conn: cached.conn as *mut PGconn,
                    generation: cached.generation,
                });
            }
            if state.live_connections < self.limit {
                state.live_connections += 1;
                Self::finish_acquisition(&mut state, started, queued)?;
                return Ok(ConnectionAcquisition::ReservedSlot);
            }
            if !queued {
                queued = true;
                state.queued_acquisitions = state.queued_acquisitions.saturating_add(1);
                state.peak_queued_acquisitions = state
                    .peak_queued_acquisitions
                    .max(state.queued_acquisitions);
            }
            state = self
                .available
                .wait(state)
                .map_err(|_| "connection pool is poisoned".to_string())?;
        }
    }

    fn record_connection_created(&self, elapsed: Duration) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "connection pool is poisoned".to_string())?;
        state.connection_create_count = state.connection_create_count.saturating_add(1);
        let elapsed_micros = Self::duration_micros(elapsed);
        state.connection_create_micros_total = state
            .connection_create_micros_total
            .saturating_add(elapsed_micros);
        state.connection_create_micros_max = state.connection_create_micros_max.max(elapsed_micros);
        Ok(())
    }

    fn return_cached(
        &self,
        lane: ConnectionLane,
        conn: *mut PGconn,
        generation: u64,
        current_generation: u64,
        operation_elapsed: Duration,
    ) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "connection pool is poisoned".to_string())?;
        Self::finish_operation(&mut state, operation_elapsed, false);
        Self::release_active(&mut state)?;
        if generation == current_generation {
            Self::push_cached(&mut state, lane, conn, generation);
            self.available.notify_one();
            Ok(true)
        } else {
            Self::release_live(&mut state)?;
            state.stale_connection_discards = state.stale_connection_discards.saturating_add(1);
            self.available.notify_one();
            Ok(false)
        }
    }

    fn release_failed_operation(&self, operation_elapsed: Duration) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "connection pool is poisoned".to_string())?;
        Self::finish_operation(&mut state, operation_elapsed, true);
        Self::release_active(&mut state)?;
        Self::release_live(&mut state)?;
        self.available.notify_one();
        Ok(())
    }

    fn release_unconnected_slot(&self, connect_elapsed: Duration) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "connection pool is poisoned".to_string())?;
        state.connection_create_failures = state.connection_create_failures.saturating_add(1);
        let elapsed_micros = Self::duration_micros(connect_elapsed);
        state.connection_create_micros_total = state
            .connection_create_micros_total
            .saturating_add(elapsed_micros);
        state.connection_create_micros_max = state.connection_create_micros_max.max(elapsed_micros);
        Self::release_active(&mut state)?;
        Self::release_live(&mut state)?;
        self.available.notify_one();
        Ok(())
    }

    fn record_replay(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "connection pool is poisoned".to_string())?;
        state.replay_count = state.replay_count.saturating_add(1);
        Ok(())
    }

    fn record_transaction(&self, elapsed: Duration, failed: bool) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.transaction_count = state.transaction_count.saturating_add(1);
        if failed {
            state.transaction_failures = state.transaction_failures.saturating_add(1);
        }
        let elapsed_micros = Self::duration_micros(elapsed);
        state.transaction_micros_total = state
            .transaction_micros_total
            .saturating_add(elapsed_micros);
        state.transaction_micros_max = state.transaction_micros_max.max(elapsed_micros);
    }

    fn record_heartbeat(
        &self,
        schedule_delay: Duration,
        execution_elapsed: Duration,
        failed: bool,
    ) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.heartbeat_count = state.heartbeat_count.saturating_add(1);
        if failed {
            state.heartbeat_failures = state.heartbeat_failures.saturating_add(1);
        }
        let schedule_delay_micros = Self::duration_micros(schedule_delay);
        state.heartbeat_schedule_delay_micros_total = state
            .heartbeat_schedule_delay_micros_total
            .saturating_add(schedule_delay_micros);
        state.heartbeat_schedule_delay_micros_max = state
            .heartbeat_schedule_delay_micros_max
            .max(schedule_delay_micros);
        let execution_micros = Self::duration_micros(execution_elapsed);
        state.heartbeat_execution_micros_total = state
            .heartbeat_execution_micros_total
            .saturating_add(execution_micros);
        state.heartbeat_execution_micros_max =
            state.heartbeat_execution_micros_max.max(execution_micros);
    }

    fn snapshot(&self) -> Result<DbRepoPoolObservabilitySnapshot, String> {
        let write_transaction_admission = self.write_transaction_gate.snapshot();
        let control_transaction_admission = self.control_transaction_gate.snapshot();
        let state = self
            .state
            .lock()
            .map_err(|_| "connection pool is poisoned".to_string())?;
        Ok(DbRepoPoolObservabilitySnapshot {
            connection_limit: self.limit,
            live_connections: state.live_connections,
            idle_write_connections: state.cached_conn.len(),
            idle_control_connections: state.control_cached_conn.len(),
            active_connections: state.active_connections,
            queued_acquisitions: state.queued_acquisitions,
            peak_active_connections: state.peak_active_connections,
            peak_queued_acquisitions: state.peak_queued_acquisitions,
            acquisition_count: state.acquisition_count,
            acquisition_wait_micros_total: state.acquisition_wait_micros_total,
            acquisition_wait_micros_max: state.acquisition_wait_micros_max,
            connection_create_count: state.connection_create_count,
            connection_create_failures: state.connection_create_failures,
            connection_create_micros_total: state.connection_create_micros_total,
            connection_create_micros_max: state.connection_create_micros_max,
            operation_count: state.operation_count,
            operation_failures: state.operation_failures,
            operation_micros_total: state.operation_micros_total,
            operation_micros_max: state.operation_micros_max,
            replay_count: state.replay_count,
            stale_connection_discards: state.stale_connection_discards,
            transaction_count: state.transaction_count,
            transaction_failures: state.transaction_failures,
            transaction_micros_total: state.transaction_micros_total,
            transaction_micros_max: state.transaction_micros_max,
            write_transaction_admission,
            control_transaction_admission,
            heartbeat_count: state.heartbeat_count,
            heartbeat_failures: state.heartbeat_failures,
            heartbeat_schedule_delay_micros_total: state.heartbeat_schedule_delay_micros_total,
            heartbeat_schedule_delay_micros_max: state.heartbeat_schedule_delay_micros_max,
            heartbeat_execution_micros_total: state.heartbeat_execution_micros_total,
            heartbeat_execution_micros_max: state.heartbeat_execution_micros_max,
        })
    }
}

fn pool_pressure_milli(
    active_connections: usize,
    connection_limit: usize,
    queued_acquisitions: usize,
) -> u64 {
    let limit = u64::try_from(connection_limit.max(1)).unwrap_or(u64::MAX);
    let active = u64::try_from(active_connections).unwrap_or(u64::MAX);
    let queued = u64::try_from(queued_acquisitions).unwrap_or(u64::MAX);
    active
        .saturating_mul(1_000)
        .checked_div(limit)
        .unwrap_or(u64::MAX)
        .saturating_add(queued.saturating_mul(1_000))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplicaReplayStatus {
    caught_up: bool,
    lag_bytes: u64,
}

impl DbRepo {
    pub fn new(conninfo: &str) -> Result<Self, String> {
        let runtime = RuntimeConfig::from_env()?;
        Self::with_runtime(conninfo, &runtime)
    }

    pub fn with_runtime(conninfo: &str, runtime: &RuntimeConfig) -> Result<Self, String> {
        let payload_settings = RuntimePayloadSettings::from_env()?;
        let global_payload_observability =
            Arc::new(DbRepoPayloadObservability::with_in_flight_limit_bytes(
                payload_settings.in_flight_limit_bytes,
            ));
        Self::with_runtime_observability(
            conninfo,
            runtime,
            Arc::new(DbRepoPayloadObservability::default()),
            global_payload_observability,
        )
    }

    pub fn with_runtime_and_global_payload_observability(
        conninfo: &str,
        runtime: &RuntimeConfig,
        global_payload_observability: Arc<DbRepoPayloadObservability>,
    ) -> Result<Self, String> {
        Self::with_runtime_observability(
            conninfo,
            runtime,
            Arc::new(DbRepoPayloadObservability::default()),
            global_payload_observability,
        )
    }

    fn with_runtime_observability(
        conninfo: &str,
        runtime: &RuntimeConfig,
        payload_observability: Arc<DbRepoPayloadObservability>,
        global_payload_observability: Arc<DbRepoPayloadObservability>,
    ) -> Result<Self, String> {
        let connection_targets = Arc::new(DbRepoConnectionTargets::single(conninfo)?);
        Self::with_runtime_observability_targets(
            connection_targets,
            runtime,
            payload_observability,
            global_payload_observability,
        )
    }

    pub fn with_runtime_connection_targets(
        connection_targets: Arc<DbRepoConnectionTargets>,
        runtime: &RuntimeConfig,
    ) -> Result<Self, String> {
        let payload_settings = RuntimePayloadSettings::from_env()?;
        let global_payload_observability =
            Arc::new(DbRepoPayloadObservability::with_in_flight_limit_bytes(
                payload_settings.in_flight_limit_bytes,
            ));
        Self::with_runtime_observability_targets(
            connection_targets,
            runtime,
            Arc::new(DbRepoPayloadObservability::default()),
            global_payload_observability,
        )
    }

    pub fn with_runtime_connection_targets_and_replica_read_targets(
        connection_targets: Arc<DbRepoConnectionTargets>,
        replica_read_targets: Option<Arc<DbRepoConnectionTargets>>,
        replica_read_pool_limit: usize,
        runtime: &RuntimeConfig,
    ) -> Result<Self, String> {
        let payload_settings = RuntimePayloadSettings::from_env()?;
        let global_payload_observability =
            Arc::new(DbRepoPayloadObservability::with_in_flight_limit_bytes(
                payload_settings.in_flight_limit_bytes,
            ));
        Self::with_runtime_observability_targets_and_replica(
            connection_targets,
            replica_read_targets,
            replica_read_pool_limit,
            runtime,
            Arc::new(DbRepoPayloadObservability::default()),
            global_payload_observability,
        )
    }

    pub fn with_runtime_connection_targets_and_replica_read_targets_and_global_payload_observability(
        connection_targets: Arc<DbRepoConnectionTargets>,
        replica_read_targets: Option<Arc<DbRepoConnectionTargets>>,
        replica_read_pool_limit: usize,
        runtime: &RuntimeConfig,
        global_payload_observability: Arc<DbRepoPayloadObservability>,
    ) -> Result<Self, String> {
        Self::with_runtime_observability_targets_and_replica(
            connection_targets,
            replica_read_targets,
            replica_read_pool_limit,
            runtime,
            Arc::new(DbRepoPayloadObservability::default()),
            global_payload_observability,
        )
    }

    pub fn with_runtime_connection_targets_and_global_payload_observability(
        connection_targets: Arc<DbRepoConnectionTargets>,
        runtime: &RuntimeConfig,
        global_payload_observability: Arc<DbRepoPayloadObservability>,
    ) -> Result<Self, String> {
        Self::with_runtime_observability_targets(
            connection_targets,
            runtime,
            Arc::new(DbRepoPayloadObservability::default()),
            global_payload_observability,
        )
    }

    fn with_runtime_observability_targets(
        connection_targets: Arc<DbRepoConnectionTargets>,
        runtime: &RuntimeConfig,
        payload_observability: Arc<DbRepoPayloadObservability>,
        global_payload_observability: Arc<DbRepoPayloadObservability>,
    ) -> Result<Self, String> {
        Self::with_runtime_observability_targets_and_replica(
            connection_targets,
            None,
            1,
            runtime,
            payload_observability,
            global_payload_observability,
        )
    }

    fn with_runtime_observability_targets_and_replica(
        connection_targets: Arc<DbRepoConnectionTargets>,
        replica_read_targets: Option<Arc<DbRepoConnectionTargets>>,
        replica_read_pool_limit: usize,
        runtime: &RuntimeConfig,
        payload_observability: Arc<DbRepoPayloadObservability>,
        global_payload_observability: Arc<DbRepoPayloadObservability>,
    ) -> Result<Self, String> {
        let core = runtime.core_settings();
        let storage = runtime.storage_settings();
        let transaction_settings = RuntimeTransactionSettings::from_env()?;
        Self::new_with_tuning_targets_and_replica(
            connection_targets,
            replica_read_targets,
            replica_read_pool_limit,
            ConnectionTuning::from_storage(&storage),
            storage.persist_buffer_chunk_blocks,
            storage.persist_block_transport,
            storage.data_object_swap_cleanup,
            core.pool_max_connections.max(1) as usize,
            transaction_settings,
            payload_observability,
            global_payload_observability,
        )
    }

    #[cfg(test)]
    fn new_with_tuning(
        conninfo: &str,
        connection_tuning: ConnectionTuning,
        persist_buffer_chunk_blocks: u64,
        persist_block_transport: PersistBlockTransport,
        data_object_swap_cleanup: DataObjectSwapCleanup,
        connection_limit: usize,
        transaction_settings: RuntimeTransactionSettings,
        payload_observability: Arc<DbRepoPayloadObservability>,
        global_payload_observability: Arc<DbRepoPayloadObservability>,
    ) -> Result<Self, String> {
        let connection_targets = Arc::new(DbRepoConnectionTargets::single(conninfo)?);
        Self::new_with_tuning_targets(
            connection_targets,
            connection_tuning,
            persist_buffer_chunk_blocks,
            persist_block_transport,
            data_object_swap_cleanup,
            connection_limit,
            transaction_settings,
            payload_observability,
            global_payload_observability,
        )
    }

    #[cfg(test)]
    fn new_with_tuning_targets(
        connection_targets: Arc<DbRepoConnectionTargets>,
        connection_tuning: ConnectionTuning,
        persist_buffer_chunk_blocks: u64,
        persist_block_transport: PersistBlockTransport,
        data_object_swap_cleanup: DataObjectSwapCleanup,
        connection_limit: usize,
        transaction_settings: RuntimeTransactionSettings,
        payload_observability: Arc<DbRepoPayloadObservability>,
        global_payload_observability: Arc<DbRepoPayloadObservability>,
    ) -> Result<Self, String> {
        Self::new_with_tuning_targets_and_replica(
            connection_targets,
            None,
            1,
            connection_tuning,
            persist_buffer_chunk_blocks,
            persist_block_transport,
            data_object_swap_cleanup,
            connection_limit,
            transaction_settings,
            payload_observability,
            global_payload_observability,
        )
    }

    fn new_with_tuning_targets_and_replica(
        connection_targets: Arc<DbRepoConnectionTargets>,
        replica_read_targets: Option<Arc<DbRepoConnectionTargets>>,
        replica_read_pool_limit: usize,
        connection_tuning: ConnectionTuning,
        persist_buffer_chunk_blocks: u64,
        persist_block_transport: PersistBlockTransport,
        data_object_swap_cleanup: DataObjectSwapCleanup,
        connection_limit: usize,
        transaction_settings: RuntimeTransactionSettings,
        payload_observability: Arc<DbRepoPayloadObservability>,
        global_payload_observability: Arc<DbRepoPayloadObservability>,
    ) -> Result<Self, String> {
        let replica_read_pool = replica_read_targets.as_ref().map(|_| {
            Arc::new(SharedConnectionPool::new(
                replica_read_pool_limit.max(1),
                transaction_settings,
            ))
        });
        Ok(Self {
            connection_targets,
            replica_read_targets,
            replica_read_pool,
            replica_consistency: Arc::new(ReplicaReadConsistency::default()),
            connection_tuning,
            persist_buffer_chunk_blocks: persist_buffer_chunk_blocks.max(1),
            persist_block_transport,
            data_object_swap_cleanup,
            pool: Arc::new(SharedConnectionPool::new(
                connection_limit.max(1),
                transaction_settings,
            )),
            payload_observability,
            global_payload_observability,
            lock_session_id: Mutex::new(NEXT_LOCK_SESSION_ID.fetch_sub(1, Ordering::Relaxed)),
            lock_schema_ready: Mutex::new(false),
            owner_session_cache: Mutex::new(HashMap::new()),
        })
    }

    fn refresh_primary_wal_lsn(&self, conn: *mut PGconn) {
        if self.replica_read_targets.is_none() {
            return;
        }
        let Ok(sql) = CString::new("SELECT pg_current_wal_lsn()::text") else {
            self.replica_consistency
                .record_primary_wal_lsn_capture_failure();
            return;
        };
        match unsafe { query_scalar_text_on_conn(conn, &sql) } {
            Ok(value) => {
                if self
                    .replica_consistency
                    .record_primary_wal_lsn(&value)
                    .is_err()
                {
                    self.replica_consistency
                        .record_primary_wal_lsn_capture_failure();
                }
            }
            Err(_) => self
                .replica_consistency
                .record_primary_wal_lsn_capture_failure(),
        }
    }

    fn replica_is_caught_up(&self, conn: *mut PGconn) -> Result<ReplicaReplayStatus, String> {
        if self.replica_consistency.force_primary()? {
            return Ok(ReplicaReplayStatus {
                caught_up: false,
                lag_bytes: u64::MAX,
            });
        }
        let Some(required_lsn) = self.replica_consistency.required_lsn()? else {
            return Ok(ReplicaReplayStatus {
                caught_up: true,
                lag_bytes: 0,
            });
        };
        let sql = CString::new("SELECT COALESCE(pg_last_wal_replay_lsn()::text, '')")
            .map_err(|_| "replica replay LSN SQL contains NUL byte".to_string())?;
        let replay_lsn = unsafe { query_scalar_text_on_conn(conn, &sql) }?;
        let replay_lsn = replay_lsn.trim();
        let (caught_up, lag_bytes) = if replay_lsn.is_empty() {
            (false, u64::MAX)
        } else {
            let replay_lsn = parse_pg_lsn(replay_lsn)?;
            (
                replay_lsn >= required_lsn,
                required_lsn.saturating_sub(replay_lsn),
            )
        };
        self.replica_consistency.record_consistency_check(caught_up);
        Ok(ReplicaReplayStatus {
            caught_up,
            lag_bytes,
        })
    }

    fn replica_pool_pressure_prefers_primary(
        &self,
        replica_pool: &SharedConnectionPool,
    ) -> Result<bool, String> {
        let replica = replica_pool.snapshot()?;
        let primary = self.pool.snapshot()?;
        let replica_pressure = pool_pressure_milli(
            replica.active_connections,
            replica.connection_limit,
            replica.queued_acquisitions,
        );
        let primary_pressure = pool_pressure_milli(
            primary.active_connections,
            primary.connection_limit,
            primary.queued_acquisitions,
        );
        Ok(replica_pressure > primary_pressure.saturating_add(REPLICA_POOL_PRESSURE_MARGIN_MILLI))
    }

    fn with_read_connection<T, F>(&self, mut f: F) -> Result<T, String>
    where
        F: FnMut(*mut PGconn) -> Result<T, String>,
    {
        let (Some(targets), Some(pool)) = (
            self.replica_read_targets.as_ref(),
            self.replica_read_pool.as_ref(),
        ) else {
            return self.with_connection(ConnectionLane::Write, f);
        };

        if self.replica_consistency.force_primary()? {
            self.replica_consistency.record_forced_primary_fallback();
            return self.with_connection(ConnectionLane::Write, f);
        }

        if self.replica_pool_pressure_prefers_primary(pool)? {
            self.replica_consistency.record_pool_pressure_fallback();
            return self.with_connection(ConnectionLane::Write, f);
        }

        let max_attempts = targets.target_count().max(1);
        let mut attempts = 0usize;

        while attempts < max_attempts {
            let Some(generation) = targets.prepare_scored_replica()? else {
                break;
            };
            attempts = attempts.saturating_add(1);

            let acquisition = match pool.acquire(ConnectionLane::Write, generation) {
                Ok(value) => value,
                Err(_) => {
                    self.replica_consistency.record_replica_failure();
                    break;
                }
            };
            let (conn, generation) = match acquisition {
                ConnectionAcquisition::Cached { conn, generation } => (conn, generation),
                ConnectionAcquisition::ReservedSlot => {
                    let started = Instant::now();
                    match targets.connect_new(&self.connection_tuning) {
                        Ok((conn, generation)) => {
                            if let Err(err) = pool.record_connection_created(started.elapsed()) {
                                unsafe { PQfinish(conn) };
                                return Err(err);
                            }
                            (conn, generation)
                        }
                        Err(_) => {
                            let _ = pool.release_unconnected_slot(started.elapsed());
                            self.replica_consistency.record_replica_failure();
                            break;
                        }
                    }
                }
            };

            let started = Instant::now();
            match self.replica_is_caught_up(conn) {
                Ok(status) if status.caught_up => {
                    targets.record_replica_replay(generation, status.lag_bytes)?;
                }
                Ok(status) => {
                    let switched = targets.record_replica_replay(generation, status.lag_bytes)?;
                    let current_generation = targets.generation()?;
                    match pool.return_cached(
                        ConnectionLane::Write,
                        conn,
                        generation,
                        current_generation,
                        started.elapsed(),
                    ) {
                        Ok(true) => {}
                        Ok(false) | Err(_) => unsafe { PQfinish(conn) },
                    }
                    if switched {
                        continue;
                    }
                    break;
                }
                Err(_) => {
                    let _ = pool.release_failed_operation(started.elapsed());
                    unsafe { PQfinish(conn) };
                    let _ = targets.mark_operation_connection_failure(generation);
                    self.replica_consistency.record_replica_failure();
                    continue;
                }
            }

            match f(conn) {
                Ok(value) => {
                    let elapsed = started.elapsed();
                    let current_generation = targets.generation()?;
                    match pool.return_cached(
                        ConnectionLane::Write,
                        conn,
                        generation,
                        current_generation,
                        elapsed,
                    ) {
                        Ok(true) => {}
                        Ok(false) => unsafe { PQfinish(conn) },
                        Err(err) => {
                            unsafe { PQfinish(conn) };
                            return Err(err);
                        }
                    }
                    targets.record_replica_operation_success(generation, elapsed)?;
                    self.replica_consistency.record_replica_read();
                    return Ok(value);
                }
                Err(_) => {
                    let _ = pool.release_failed_operation(started.elapsed());
                    unsafe { PQfinish(conn) };
                    let _ = targets.mark_operation_connection_failure(generation);
                    self.replica_consistency.record_replica_failure();
                }
            }
        }

        self.replica_consistency.record_primary_fallback();
        self.with_connection(ConnectionLane::Write, f)
    }

    fn with_connection<T, F>(&self, lane: ConnectionLane, mut f: F) -> Result<T, String>
    where
        F: FnMut(*mut PGconn) -> Result<T, String>,
    {
        let mut replayed = false;

        loop {
            let current_generation = self.connection_targets.generation()?;
            let acquisition = self.pool.acquire(lane, current_generation)?;
            let (conn, generation) = match acquisition {
                ConnectionAcquisition::Cached { conn, generation } => (conn, generation),
                ConnectionAcquisition::ReservedSlot => {
                    let connect_started = Instant::now();
                    match self.connection_targets.connect_new(&self.connection_tuning) {
                        Ok((conn, generation)) => {
                            if let Err(err) = self
                                .pool
                                .record_connection_created(connect_started.elapsed())
                            {
                                unsafe {
                                    PQfinish(conn);
                                }
                                return Err(err);
                            }
                            (conn, generation)
                        }
                        Err(err) => {
                            if let Err(pool_err) = self
                                .pool
                                .release_unconnected_slot(connect_started.elapsed())
                            {
                                return Err(format!(
                                    "{err}; connection pool cleanup failed: {pool_err}"
                                ));
                            }
                            return Err(err);
                        }
                    }
                }
            };

            let primary_operation_guard =
                if self.connection_targets.primary_promotion_guard_enabled() {
                    match self
                        .connection_targets
                        .begin_primary_operation(generation)?
                    {
                        Some(guard) => Some(guard),
                        None => {
                            let current_generation = self.connection_targets.generation()?;
                            match self.pool.return_cached(
                                lane,
                                conn,
                                generation,
                                current_generation,
                                Duration::ZERO,
                            ) {
                                Ok(true) => {}
                                Ok(false) => unsafe {
                                    PQfinish(conn);
                                },
                                Err(err) => {
                                    unsafe {
                                        PQfinish(conn);
                                    }
                                    return Err(err);
                                }
                            }
                            continue;
                        }
                    }
                } else {
                    None
                };

            let operation_started = Instant::now();
            let _transaction_pool_scope =
                CurrentTransactionPoolScope::enter(Arc::clone(&self.pool), lane);
            let result = f(conn);
            let operation_elapsed = operation_started.elapsed();
            match result {
                Ok(value) => {
                    if lane == ConnectionLane::Write {
                        self.refresh_primary_wal_lsn(conn);
                    }
                    let current_generation = self.connection_targets.generation()?;
                    match self.pool.return_cached(
                        lane,
                        conn,
                        generation,
                        current_generation,
                        operation_elapsed,
                    ) {
                        Ok(true) => {}
                        Ok(false) => unsafe {
                            PQfinish(conn);
                        },
                        Err(err) => {
                            unsafe {
                                PQfinish(conn);
                            }
                            return Err(err);
                        }
                    }
                    return Ok(value);
                }
                Err(err) => {
                    let replayable = err.starts_with(REPLAYABLE_SQL_ERROR_PREFIX);
                    let err = strip_replayable_sql_error(err);
                    let pool_result = self.pool.release_failed_operation(operation_elapsed);
                    unsafe {
                        PQfinish(conn);
                    }

                    // Operacja, ktora sama wykryla disconnect, musi zejsc z
                    // in-flight fence przed mark_operation_connection_failure().
                    // Transition czeka na zero aktywnych operacji tej generacji,
                    // wiec trzymanie guarda tutaj spowodowaloby self-deadlock.
                    drop(primary_operation_guard);

                    if let Err(pool_err) = pool_result {
                        return Err(format!("{err}; connection pool cleanup failed: {pool_err}"));
                    }
                    if replayable && !replayed {
                        self.connection_targets
                            .mark_operation_connection_failure(generation)?;
                        self.pool.record_replay()?;
                        replayed = true;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }

    fn with_cached_connection<T, F>(&self, f: F) -> Result<T, String>
    where
        F: FnMut(*mut PGconn) -> Result<T, String>,
    {
        self.with_connection(ConnectionLane::Write, f)
    }

    fn with_control_connection<T, F>(&self, f: F) -> Result<T, String>
    where
        F: FnMut(*mut PGconn) -> Result<T, String>,
    {
        self.with_connection(ConnectionLane::Control, f)
    }

    pub fn observability_snapshot(&self) -> Result<DbRepoObservabilitySnapshot, String> {
        let mut routing = self.connection_targets.snapshot()?;
        let consistency = self.replica_consistency.snapshot()?;
        routing.replica_read_routing_enabled = self.replica_read_targets.is_some();
        if let Some(replica_targets) = &self.replica_read_targets {
            let replica = replica_targets.snapshot()?;
            let scoring = replica_targets.scoring_snapshot()?;
            routing.replica_target_count = replica.target_count;
            routing.replica_active_authority = Some(replica.active_authority);
            routing.replica_scoring_enabled = scoring.enabled;
            routing.replica_active_score = scoring.active_score;
            routing.replica_active_replay_lag_bytes = scoring.active_replay_lag_bytes;
            routing.replica_active_connection_latency_micros =
                scoring.active_connection_latency_micros;
            routing.replica_active_operation_latency_micros =
                scoring.active_operation_latency_micros;
            routing.replica_score_selections = scoring.score_selections;
            routing.replica_score_switches = scoring.score_switches;
            routing.replica_hysteresis_keeps = scoring.hysteresis_keeps;
            routing.replica_circuit_breaker_skips = scoring.circuit_breaker_skips;
            routing.replica_circuit_open_targets = scoring.circuit_open_targets;
        }
        routing.required_primary_wal_lsn = consistency.required_primary_wal_lsn;
        routing.primary_wal_lsn_updates = consistency.primary_wal_lsn_updates;
        routing.primary_wal_lsn_capture_failures = consistency.primary_wal_lsn_capture_failures;
        routing.replica_consistency_checks = consistency.replica_consistency_checks;
        routing.replica_consistency_passes = consistency.replica_consistency_passes;
        routing.replica_reads = consistency.replica_reads;
        routing.replica_lag_fallbacks = consistency.replica_lag_fallbacks;
        routing.replica_read_failures = consistency.replica_read_failures;
        routing.primary_read_fallbacks = consistency.primary_read_fallbacks;
        routing.replica_pool_pressure_fallbacks = consistency.replica_pool_pressure_fallbacks;
        Ok(DbRepoObservabilitySnapshot {
            pool: self.pool.snapshot()?,
            routing,
            payload: self.payload_observability.snapshot()?,
            global_payload: self.global_payload_observability.snapshot()?,
            persist_buffer_chunk_blocks: self.persist_buffer_chunk_blocks,
            persist_copy_send_buffer_bytes: persist_copy_send_buffer_bytes(),
        })
    }

    pub fn record_heartbeat_observability(
        &self,
        schedule_delay: Duration,
        execution_elapsed: Duration,
        failed: bool,
    ) {
        self.pool
            .record_heartbeat(schedule_delay, execution_elapsed, failed);
    }

    pub fn postgres_pressure_snapshot(&self) -> Result<PostgresPressureSnapshot, String> {
        self.with_control_connection(|conn| unsafe {
            let backend_memory_sql = if PQserverVersion(conn) >= 130_000 {
                "COALESCE((SELECT SUM(total_bytes)::bigint::text FROM pg_backend_memory_contexts), '')"
            } else {
                "''"
            };
            let sql = CString::new(format!(
                "SELECT \
                    COALESCE((SELECT numbackends::text FROM pg_stat_database WHERE datname = current_database()), '0'), \
                    (SELECT COUNT(*)::text FROM pg_stat_activity WHERE datname = current_database()), \
                    (SELECT COUNT(*) FILTER (WHERE state = 'active')::text FROM pg_stat_activity WHERE datname = current_database()), \
                    (SELECT COUNT(*) FILTER (WHERE state = 'idle')::text FROM pg_stat_activity WHERE datname = current_database()), \
                    (SELECT COUNT(*) FILTER (WHERE state = 'idle in transaction')::text FROM pg_stat_activity WHERE datname = current_database()), \
                    COALESCE((SELECT temp_files::text FROM pg_stat_database WHERE datname = current_database()), '0'), \
                    COALESCE((SELECT temp_bytes::text FROM pg_stat_database WHERE datname = current_database()), '0'), \
                    COALESCE((SELECT deadlocks::text FROM pg_stat_database WHERE datname = current_database()), '0'), \
                    current_setting('shared_buffers'), \
                    current_setting('work_mem'), \
                    current_setting('maintenance_work_mem'), \
                    current_setting('temp_buffers'), \
                    {backend_memory_sql}"
            ))
            .map_err(|_| "PostgreSQL pressure SQL contains NUL byte".to_string())?;
            let rows = query_rows_text_on_conn(conn, &sql)?;
            let row = rows
                .into_iter()
                .next()
                .ok_or_else(|| "PostgreSQL pressure query returned no rows".to_string())?;
            if row.len() != 13 {
                return Err(format!(
                    "PostgreSQL pressure query returned {} columns instead of 13",
                    row.len()
                ));
            }
            let parse_u64 = |index: usize, name: &str| {
                row[index]
                    .trim()
                    .parse::<u64>()
                    .map_err(|err| format!("invalid PostgreSQL {name} value: {err}"))
            };
            Ok(PostgresPressureSnapshot {
                database_connections: parse_u64(0, "database connection count")?,
                activity_connections: parse_u64(1, "activity connection count")?,
                activity_active: parse_u64(2, "active connection count")?,
                activity_idle: parse_u64(3, "idle connection count")?,
                activity_idle_in_transaction: parse_u64(
                    4,
                    "idle-in-transaction connection count",
                )?,
                temp_files: parse_u64(5, "temp file count")?,
                temp_bytes: parse_u64(6, "temp byte count")?,
                deadlocks: parse_u64(7, "deadlock count")?,
                shared_buffers: row[8].clone(),
                work_mem: row[9].clone(),
                maintenance_work_mem: row[10].clone(),
                temp_buffers: row[11].clone(),
                current_backend_memory_bytes: if row[12].trim().is_empty() {
                    None
                } else {
                    Some(parse_u64(12, "current backend memory byte count")?)
                },
            })
        })
    }

    fn payload_persist_guard(&self, input_bytes: u64, input_rows: u64) -> PayloadPersistGuard {
        PayloadPersistGuard::new(
            Arc::clone(&self.payload_observability),
            Arc::clone(&self.global_payload_observability),
            input_bytes,
            input_rows,
        )
    }

    fn record_persist_transaction_elapsed(&self, elapsed: Duration, failed: bool) {
        self.payload_observability
            .record_persist_transaction(elapsed, failed);
        if !Arc::ptr_eq(
            &self.payload_observability,
            &self.global_payload_observability,
        ) {
            self.global_payload_observability
                .record_persist_transaction(elapsed, failed);
        }
    }

    fn record_persist_copy_stage_elapsed(&self, elapsed: Duration) {
        self.payload_observability
            .record_persist_copy_stage(elapsed);
        if !Arc::ptr_eq(
            &self.payload_observability,
            &self.global_payload_observability,
        ) {
            self.global_payload_observability
                .record_persist_copy_stage(elapsed);
        }
    }

    fn record_persist_data_blocks_merge_elapsed(&self, elapsed: Duration) {
        self.payload_observability
            .record_persist_data_blocks_merge(elapsed);
        if !Arc::ptr_eq(
            &self.payload_observability,
            &self.global_payload_observability,
        ) {
            self.global_payload_observability
                .record_persist_data_blocks_merge(elapsed);
        }
    }

    fn record_quota_lock_wait_elapsed(&self, elapsed: Duration) {
        self.payload_observability.record_quota_lock_wait(elapsed);
        if !Arc::ptr_eq(
            &self.payload_observability,
            &self.global_payload_observability,
        ) {
            self.global_payload_observability
                .record_quota_lock_wait(elapsed);
        }
    }

    fn record_quota_final_check_elapsed(&self, elapsed: Duration) {
        self.payload_observability.record_quota_final_check(elapsed);
        if !Arc::ptr_eq(
            &self.payload_observability,
            &self.global_payload_observability,
        ) {
            self.global_payload_observability
                .record_quota_final_check(elapsed);
        }
    }

    pub fn postgres_runtime_requirements(&self) -> Result<PostgresRuntimeRequirements, String> {
        self.postgres_runtime_requirements_for_pool_limit(self.pool.limit as u64)
    }

    pub fn postgres_runtime_requirements_for_pool_limit(
        &self,
        pool_max_connections: u64,
    ) -> Result<PostgresRuntimeRequirements, String> {
        self.with_control_connection(|conn| unsafe {
            postgres_runtime_requirements_on_conn(conn, pool_max_connections)
        })
    }

    pub fn postgres_version_diagnostics(&self) -> Result<PostgresVersionDiagnostics, String> {
        self.with_control_connection(|conn| unsafe { postgres_version_diagnostics_on_conn(conn) })
    }

    fn confirm_unique_violation<T, F>(&self, err: String, confirm: F) -> Result<T, String>
    where
        F: FnOnce(&Self) -> Result<Option<T>, String>,
    {
        if !error_is_unique_violation(&err) {
            return Err(err);
        }

        match confirm(self)? {
            Some(value) => Ok(value),
            None => Err(err),
        }
    }

    fn confirm_created_hardlink(
        &self,
        target_parent_id: Option<u64>,
        target_name: &str,
        source_file_id: u64,
        uid: u32,
        gid: u32,
    ) -> Result<Option<u64>, String> {
        let parent_clause = match target_parent_id {
            Some(parent_id) => format!("id_directory = {parent_id}"),
            None => "id_directory IS NULL".to_string(),
        };
        let sql = format!(
            "SELECT id_hardlink, id_file, uid, gid FROM hardlinks WHERE {parent_clause} AND name = {} LIMIT 1",
            Self::quote_literal(target_name)
        );
        let rows = self.query_rows_text(&sql)?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        if row.len() < 4 {
            return Ok(None);
        }
        let hardlink_id = row[0].trim().parse::<u64>().ok();
        let id_file = row[1].trim().parse::<u64>().ok();
        let matches = hardlink_id.is_some()
            && id_file == Some(source_file_id)
            && row[2].trim() == uid.to_string()
            && row[3].trim() == gid.to_string();
        if matches {
            Ok(hardlink_id)
        } else {
            Ok(None)
        }
    }

    fn confirm_created_symlink(
        &self,
        target_parent_id: Option<u64>,
        target_name: &str,
        target: &str,
        uid: u32,
        gid: u32,
        inode_seed: &str,
    ) -> Result<Option<u64>, String> {
        let parent_clause = match target_parent_id {
            Some(parent_id) => format!("id_parent = {parent_id}"),
            None => "id_parent IS NULL".to_string(),
        };
        let sql = format!(
            "SELECT id_symlink, target, uid, gid, inode_seed FROM symlinks WHERE {parent_clause} AND name = {} LIMIT 1",
            Self::quote_literal(target_name)
        );
        let rows = self.query_rows_text(&sql)?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        if row.len() < 5 {
            return Ok(None);
        }
        let symlink_id = row[0].trim().parse::<u64>().ok();
        let matches = symlink_id.is_some()
            && row[1].trim() == target
            && row[2].trim() == uid.to_string()
            && row[3].trim() == gid.to_string()
            && row[4].trim() == inode_seed;
        if matches {
            Ok(symlink_id)
        } else {
            Ok(None)
        }
    }

    fn confirm_created_directory(
        &self,
        target_parent_id: Option<u64>,
        target_name: &str,
        mode: &str,
        uid: u32,
        gid: u32,
        inode_seed: &str,
    ) -> Result<Option<u64>, String> {
        let parent_clause = match target_parent_id {
            Some(parent_id) => format!("id_parent = {parent_id}"),
            None => "id_parent IS NULL".to_string(),
        };
        let sql = format!(
            "SELECT id_directory, mode, uid, gid, inode_seed FROM directories WHERE {parent_clause} AND name = {} LIMIT 1",
            Self::quote_literal(target_name)
        );
        let rows = self.query_rows_text(&sql)?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        if row.len() < 5 {
            return Ok(None);
        }
        let directory_id = row[0].trim().parse::<u64>().ok();
        let matches = directory_id.is_some()
            && row[1].trim() == mode
            && row[2].trim() == uid.to_string()
            && row[3].trim() == gid.to_string()
            && row[4].trim() == inode_seed;
        if matches {
            Ok(directory_id)
        } else {
            Ok(None)
        }
    }

    fn confirm_created_file(
        &self,
        target_parent_id: Option<u64>,
        target_name: &str,
        mode: &str,
        uid: u32,
        gid: u32,
        inode_seed: &str,
    ) -> Result<Option<u64>, String> {
        let parent_clause = match target_parent_id {
            Some(parent_id) => format!("id_directory = {parent_id}"),
            None => "id_directory IS NULL".to_string(),
        };
        let sql = format!(
            "SELECT id_file, size, mode, uid, gid, inode_seed FROM files WHERE {parent_clause} AND name = {} LIMIT 1",
            Self::quote_literal(target_name)
        );
        let rows = self.query_rows_text(&sql)?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        if row.len() < 6 {
            return Ok(None);
        }
        let file_id = row[0].trim().parse::<u64>().ok();
        let matches = file_id.is_some()
            && row[1].trim() == "0"
            && row[2].trim() == mode
            && row[3].trim() == uid.to_string()
            && row[4].trim() == gid.to_string()
            && row[5].trim() == inode_seed;
        if matches {
            Ok(file_id)
        } else {
            Ok(None)
        }
    }

    fn confirm_created_special_file(
        &self,
        target_parent_id: Option<u64>,
        target_name: &str,
        mode: &str,
        uid: u32,
        gid: u32,
        inode_seed: &str,
        file_kind: &str,
        rdev_major: u32,
        rdev_minor: u32,
    ) -> Result<Option<u64>, String> {
        let parent_clause = match target_parent_id {
            Some(parent_id) => format!("files.id_directory = {parent_id}"),
            None => "files.id_directory IS NULL".to_string(),
        };
        let sql = format!(
            "SELECT files.id_file, files.size, files.mode, files.uid, files.gid, files.inode_seed, special_files.file_type, special_files.rdev_major, special_files.rdev_minor FROM files JOIN special_files ON special_files.id_file = files.id_file WHERE {parent_clause} AND files.name = {} LIMIT 1",
            Self::quote_literal(target_name)
        );
        let rows = self.query_rows_text(&sql)?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        if row.len() < 9 {
            return Ok(None);
        }
        let file_id = row[0].trim().parse::<u64>().ok();
        let matches = file_id.is_some()
            && row[1].trim() == "0"
            && row[2].trim() == mode
            && row[3].trim() == uid.to_string()
            && row[4].trim() == gid.to_string()
            && row[5].trim() == inode_seed
            && row[6].trim() == file_kind
            && row[7].trim() == rdev_major.to_string()
            && row[8].trim() == rdev_minor.to_string();
        if matches {
            Ok(file_id)
        } else {
            Ok(None)
        }
    }

    fn current_lock_session_id(&self) -> Result<i64, String> {
        self.lock_session_id
            .lock()
            .map(|guard| *guard)
            .map_err(|_| "lock session id is poisoned".to_string())
    }

    fn set_lock_session_id(&self, session_id: i64) -> Result<(), String> {
        let mut guard = self
            .lock_session_id
            .lock()
            .map_err(|_| "lock session id is poisoned".to_string())?;
        *guard = session_id;
        Ok(())
    }

    fn current_lock_session_id_text(&self) -> Result<CString, String> {
        CString::new(self.current_lock_session_id()?.to_string())
            .map_err(|_| "session id contains NUL byte".to_string())
    }

    fn session_id_for_owner_key_text(&self, owner_key: u64) -> Result<CString, String> {
        if let Ok(guard) = self.owner_session_cache.lock() {
            if let Some(session_id) = guard.get(&owner_key) {
                return CString::new(session_id.to_string())
                    .map_err(|_| "session id contains NUL byte".to_string());
            }
        }
        self.current_lock_session_id_text()
    }

    fn ensure_lock_schema_ready(&self) -> Result<(), String> {
        let ready = self
            .lock_schema_ready
            .lock()
            .map_err(|_| "lock schema state is poisoned".to_string())?;
        if *ready {
            return Ok(());
        }
        drop(ready);
        self.ensure_lock_schema()?;
        let mut ready = self
            .lock_schema_ready
            .lock()
            .map_err(|_| "lock schema state is poisoned".to_string())?;
        *ready = true;
        Ok(())
    }

    fn file_data_object_id_on_conn(
        &self,
        conn: *mut PGconn,
        file_id: u64,
    ) -> Result<Option<u64>, String> {
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        unsafe {
            let params = [&file_id];
            let res = exec_prepared_params(conn, PreparedStatement::FileDataObjectId, &params)?;
            let text = fetch_single_text(res)?;
            if text.is_empty() {
                return Ok(None);
            }
            let value = text
                .parse::<u64>()
                .map_err(|_| "invalid data_object_id value".to_string())?;
            Ok(Some(value))
        }
    }

    fn file_data_object_info_on_conn(
        &self,
        conn: *mut PGconn,
        file_id: u64,
    ) -> Result<Option<(u64, u64)>, String> {
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        unsafe {
            let params = [&file_id];
            let res = exec_prepared_params(conn, PreparedStatement::FileDataObjectInfo, &params)?;
            let values = fetch_first_row_texts(res)?;
            if values.len() < 2 {
                return Ok(None);
            }
            let data_object_id = values[0]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid data_object_id value".to_string())?;
            let reference_count = values[1]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid reference_count value".to_string())?;
            Ok(Some((data_object_id, reference_count)))
        }
    }

    fn data_object_reference_count_on_conn(
        &self,
        conn: *mut PGconn,
        data_object_id: u64,
    ) -> Result<Option<u64>, String> {
        let data_object_id = CString::new(data_object_id.to_string())
            .map_err(|_| "data object id contains NUL byte".to_string())?;

        unsafe {
            let params = [&data_object_id];
            let res =
                exec_prepared_params(conn, PreparedStatement::DataObjectReferenceCount, &params)?;
            let text = fetch_single_text(res)?;
            if text.is_empty() {
                Ok(None)
            } else {
                let value = text
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "invalid reference_count value".to_string())?;
                Ok(Some(value))
            }
        }
    }

    unsafe fn persist_copy_block_crc_rows_on_conn<'a>(
        &self,
        conn: *mut PGconn,
        file_id: u64,
        block_size: u64,
        blocks: &[PersistBlockRow<'a>],
    ) -> Result<(), String> {
        let data_object_id = match self.file_data_object_id_on_conn(conn, file_id)? {
            Some(value) => value,
            None => return Ok(()),
        };
        self.persist_copy_block_crc_rows_for_data_object_on_conn(
            conn,
            data_object_id,
            block_size,
            blocks,
        )
    }

    unsafe fn persist_copy_block_crc_rows_for_data_object_on_conn<'a>(
        &self,
        conn: *mut PGconn,
        data_object_id: u64,
        block_size: u64,
        blocks: &[PersistBlockRow<'a>],
    ) -> Result<(), String> {
        if blocks.is_empty() {
            return Ok(());
        }
        let block_size = block_size.max(1);
        let sql_upsert = CString::new(
            "
            INSERT INTO copy_block_crc (data_object_id, _order, crc32)
            VALUES ($1, $2, $3)
            ON CONFLICT (data_object_id, _order)
            DO UPDATE SET crc32 = EXCLUDED.crc32, updated_at = NOW()
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_delete = CString::new(
            "
            DELETE FROM copy_block_crc
            WHERE data_object_id = $1 AND _order = $2
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        let data_object_id = CString::new(data_object_id.to_string())
            .map_err(|_| "data object id contains NUL byte".to_string())?;
        for block in blocks {
            let block_index = CString::new(block.block_index.to_string())
                .map_err(|_| "block index contains NUL byte".to_string())?;
            if block.used_len >= block_size {
                let normalized = normalize_block_bytes(block.data, block_size as usize);
                if is_all_zero_block(&normalized) {
                    let params = [&data_object_id, &block_index];
                    exec_command_params(conn, &sql_delete, &params)?;
                } else {
                    let crc32 = CString::new(crc32_bytes(&normalized).to_string())
                        .map_err(|_| "crc32 contains NUL byte".to_string())?;
                    let params = [&data_object_id, &block_index, &crc32];
                    exec_command_params(conn, &sql_upsert, &params)?;
                }
            } else {
                let params = [&data_object_id, &block_index];
                exec_command_params(conn, &sql_delete, &params)?;
            }
        }
        Ok(())
    }

    unsafe fn update_file_sizes_on_conn(
        &self,
        conn: *mut PGconn,
        file_id: u64,
        data_object_id: u64,
        file_size: u64,
    ) -> Result<(), String> {
        let sql = CString::new(format!(
            "
            UPDATE data_objects
            SET file_size = {file_size}, modification_date = NOW()
            WHERE id_data_object = {data_object_id};
            UPDATE files
            SET size = {file_size}, modification_date = NOW(), change_date = NOW()
            WHERE id_file = {file_id}
            "
        ))
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        exec_command(conn, &sql)
    }

    unsafe fn create_data_object_on_conn(
        &self,
        conn: *mut PGconn,
        file_size: u64,
    ) -> Result<u64, String> {
        let sql = CString::new(
            "INSERT INTO data_objects (file_size, content_hash, reference_count, creation_date, modification_date) \
             VALUES ($1, NULL, 1, NOW(), NOW()) RETURNING id_data_object",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_size = CString::new(file_size.to_string())
            .map_err(|_| "file size contains NUL byte".to_string())?;
        let params = [&file_size];
        let res = exec_params(conn, &sql, &params)?;
        fetch_single_text(res)?
            .trim()
            .parse::<u64>()
            .map_err(|_| "invalid id_data_object value".to_string())
    }

    unsafe fn data_object_payload_rows_empty_on_conn(
        &self,
        conn: *mut PGconn,
        data_object_id: u64,
        maintain_copy_crc_table: bool,
    ) -> Result<bool, String> {
        let sql = if maintain_copy_crc_table {
            CString::new(
                "SELECT (
                    NOT EXISTS (SELECT 1 FROM data_blocks WHERE data_object_id = $1 LIMIT 1)
                    AND NOT EXISTS (SELECT 1 FROM copy_block_crc WHERE data_object_id = $1 LIMIT 1)
                )::text",
            )
        } else {
            CString::new(
                "SELECT (NOT EXISTS (
                    SELECT 1 FROM data_blocks WHERE data_object_id = $1 LIMIT 1
                ))::text",
            )
        }
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let data_object_id = CString::new(data_object_id.to_string())
            .map_err(|_| "data object id contains NUL byte".to_string())?;
        let params = [&data_object_id];
        let res = exec_params(conn, &sql, &params)?;
        let value = fetch_single_text(res)?;
        Ok(matches!(value.trim(), "t" | "true" | "1"))
    }

    unsafe fn data_object_write_target_on_conn(
        &self,
        conn: *mut PGconn,
        file_id: u64,
        file_size: u64,
        prefer_replacement: bool,
        maintain_copy_crc_table: bool,
    ) -> Result<Option<DataObjectWriteTarget>, String> {
        if prefer_replacement {
            let Some((old_data_object_id, old_reference_count)) =
                self.file_data_object_info_on_conn(conn, file_id)?
            else {
                return Ok(None);
            };
            let new_data_object_id = self.create_data_object_on_conn(conn, file_size)?;
            return Ok(Some(DataObjectWriteTarget {
                data_object_id: new_data_object_id,
                replaced: Some(ReplacedDataObject {
                    old_data_object_id,
                    old_reference_count,
                }),
                payload_rows_empty: true,
            }));
        }

        self.detach_shared_data_object_on_conn(conn, file_id, file_size, maintain_copy_crc_table)
            .map(|value| {
                value.map(
                    |(data_object_id, payload_rows_empty)| DataObjectWriteTarget {
                        data_object_id,
                        replaced: None,
                        payload_rows_empty,
                    },
                )
            })
    }

    unsafe fn finish_data_object_write_on_conn(
        &self,
        conn: *mut PGconn,
        file_id: u64,
        file_size: u64,
        target: DataObjectWriteTarget,
    ) -> Result<(), String> {
        let Some(replaced) = target.replaced else {
            return self.update_file_sizes_on_conn(conn, file_id, target.data_object_id, file_size);
        };

        let sql_update_file = CString::new(
            "UPDATE files SET data_object_id = $1, size = $2, change_date = NOW(), modification_date = NOW() WHERE id_file = $3",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_delete_data_object =
            CString::new("DELETE FROM data_objects WHERE id_data_object = $1")
                .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_mark_old_object_unreferenced = CString::new(
            "UPDATE data_objects SET reference_count = 0, modification_date = NOW() WHERE id_data_object = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_touch_old_object = CString::new(
            "UPDATE data_objects SET reference_count = GREATEST(reference_count - 1, 0), modification_date = NOW() WHERE id_data_object = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        let new_data_object_id = CString::new(target.data_object_id.to_string())
            .map_err(|_| "data object id contains NUL byte".to_string())?;
        let old_data_object_id = CString::new(replaced.old_data_object_id.to_string())
            .map_err(|_| "data object id contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let file_size = CString::new(file_size.to_string())
            .map_err(|_| "file size contains NUL byte".to_string())?;

        let params = [&new_data_object_id, &file_size, &file_id];
        exec_command_params(conn, &sql_update_file, &params)?;

        if replaced.old_reference_count <= 1 && self.data_object_swap_cleanup.is_deferred() {
            let params = [&old_data_object_id];
            exec_command_params(conn, &sql_mark_old_object_unreferenced, &params)?;
        } else if replaced.old_reference_count <= 1 {
            let params = [&old_data_object_id];
            exec_command_params(conn, &sql_delete_data_object, &params)?;
        } else {
            let params = [&old_data_object_id];
            exec_command_params(conn, &sql_touch_old_object, &params)?;
        }

        Ok(())
    }

    fn detach_shared_data_object_on_conn(
        &self,
        conn: *mut PGconn,
        file_id: u64,
        file_size: u64,
        maintain_copy_crc_table: bool,
    ) -> Result<Option<(u64, bool)>, String> {
        let sql_copy_data_object = CString::new(
            "INSERT INTO data_objects (file_size, content_hash, reference_count, creation_date, modification_date) \
             VALUES ($1, NULL, 1, NOW(), NOW()) RETURNING id_data_object",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_copy_blocks = CString::new(
            "
            INSERT INTO data_blocks (data_object_id, _order, data)
            SELECT $2, _order, data
            FROM data_blocks
            WHERE data_object_id = $1
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_copy_crc = CString::new(
            "
            INSERT INTO copy_block_crc (data_object_id, _order, crc32)
            SELECT $2, _order, crc32
            FROM copy_block_crc
            WHERE data_object_id = $1
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_update_file = CString::new(
            "UPDATE files SET data_object_id = $1, size = $2, change_date = NOW(), modification_date = NOW() WHERE id_file = $3",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_touch_old_object = CString::new(
            "UPDATE data_objects SET reference_count = GREATEST(reference_count - 1, 0), modification_date = NOW() WHERE id_data_object = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        unsafe {
            let Some((data_object_id, reference_count)) =
                self.file_data_object_info_on_conn(conn, file_id)?
            else {
                return Ok(None);
            };
            let payload_rows_empty = self.data_object_payload_rows_empty_on_conn(
                conn,
                data_object_id,
                maintain_copy_crc_table,
            )?;
            if reference_count <= 1 {
                return Ok(Some((data_object_id, payload_rows_empty)));
            }

            let file_size = CString::new(file_size.to_string())
                .map_err(|_| "file size contains NUL byte".to_string())?;
            let file_id = CString::new(file_id.to_string())
                .map_err(|_| "file id contains NUL byte".to_string())?;
            let old_data_object_id = CString::new(data_object_id.to_string())
                .map_err(|_| "data object id contains NUL byte".to_string())?;

            let params = [&file_size];
            let res = exec_params(conn, &sql_copy_data_object, &params)?;
            let new_data_object_id = fetch_single_text(res)?
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid id_data_object value".to_string())?;
            let new_data_object_id_text = CString::new(new_data_object_id.to_string())
                .map_err(|_| "data object id contains NUL byte".to_string())?;

            let params = [&old_data_object_id, &new_data_object_id_text];
            exec_command_params(conn, &sql_copy_blocks, &params)?;
            if maintain_copy_crc_table {
                exec_command_params(conn, &sql_copy_crc, &params)?;
            }

            let params = [&new_data_object_id_text, &file_size, &file_id];
            exec_command_params(conn, &sql_update_file, &params)?;

            let params = [&old_data_object_id];
            exec_command_params(conn, &sql_touch_old_object, &params)?;

            Ok(Some((new_data_object_id, payload_rows_empty)))
        }
    }

    pub fn file_data_object_id(&self, file_id: u64) -> Result<Option<u64>, String> {
        self.with_read_connection(|conn| self.file_data_object_id_on_conn(conn, file_id))
    }

    pub fn file_size(&self, file_id: u64) -> Result<Option<u64>, String> {
        let sql = CString::new("SELECT size FROM files WHERE id_file = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id];
            let res = exec_params(conn, &sql, &params)?;
            let text = fetch_single_text(res)?;
            if text.is_empty() {
                Ok(None)
            } else {
                let value = text
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "invalid file size value".to_string())?;
                Ok(Some(value))
            }
        })
    }

    pub fn file_read_metadata(&self, file_id: u64) -> Result<Option<FileReadMetadata>, String> {
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id];
            let res = exec_prepared_params(conn, PreparedStatement::FileReadMetadata, &params)?;
            let row = fetch_first_row_texts(res)?;
            if row.is_empty() {
                return Ok(None);
            }
            if row.len() < 4 {
                return Err("file read metadata row is missing fields".to_string());
            }
            let size = row[0]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid file size value".to_string())?;
            Ok(Some(FileReadMetadata {
                size,
                access_date: row[1].clone(),
                modification_date: row[2].clone(),
                change_date: row[3].clone(),
            }))
        })
    }

    pub fn load_block(
        &self,
        file_id: u64,
        block_index: u64,
        block_size: u64,
    ) -> Result<Option<Vec<u8>>, String> {
        let file_id_text = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let block_index_text = CString::new(block_index.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;
        let block_size = block_size.max(1) as usize;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id_text, &block_index_text];
            let res = exec_prepared_params_with_result_format(
                conn,
                PreparedStatement::LoadBlock,
                &params,
                1,
            )?;
            fetch_single_block_binary(res, block_size)
        })
    }

    pub fn fetch_block_range_shared(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        block_size: u64,
    ) -> Result<Vec<(u64, Arc<[u8]>)>, String> {
        if last_block < first_block {
            return Ok(Vec::new());
        }
        let file_id_text = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let first_block_text = CString::new(first_block.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;
        let last_block_text = CString::new(last_block.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;
        let block_size = block_size.max(1) as usize;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id_text, &first_block_text, &last_block_text];
            let res = exec_prepared_params_with_result_format(
                conn,
                PreparedStatement::FetchBlockRange,
                &params,
                1,
            )?;
            fetch_block_range_rows_shared(res, block_size)
        })
    }

    pub fn fetch_block_range(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        block_size: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, String> {
        let blocks = self.fetch_block_range_shared(file_id, first_block, last_block, block_size)?;
        Ok(blocks
            .into_iter()
            .map(|(index, data)| (index, data.as_ref().to_vec()))
            .collect())
    }

    pub fn assemble_file_slice(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        offset: u64,
        end_offset: u64,
        block_size: u64,
    ) -> Result<Vec<u8>, String> {
        let blocks = self.fetch_block_range(file_id, first_block, last_block, block_size)?;
        Ok(crate::assemble_read_slice(
            first_block,
            last_block,
            offset,
            end_offset,
            block_size,
            &blocks,
        ))
    }

    pub fn create_data_object(
        &self,
        file_size: u64,
        content_hash: Option<&str>,
    ) -> Result<u64, String> {
        let request_token_value = generate_request_token("data-object");
        let request_token = CString::new(request_token_value)
            .map_err(|_| "request token contains NUL byte".to_string())?;
        let file_size = CString::new(file_size.to_string())
            .map_err(|_| "file size contains NUL byte".to_string())?;
        let content_hash = match content_hash {
            Some(value) => Some(
                CString::new(value).map_err(|_| "content hash contains NUL byte".to_string())?,
            ),
            None => None,
        };
        let hash_dedupe_enabled = content_hash.is_some()
            && self
                .schema_version()
                .ok()
                .flatten()
                .map(|version| version >= 6)
                .unwrap_or(false);
        let sql_insert_hash = CString::new(
            "INSERT INTO data_objects (file_size, content_hash, reference_count, creation_date, modification_date) \
             VALUES ($1, $2, 1, NOW(), NOW()) RETURNING id_data_object",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_insert_null = CString::new(
            "INSERT INTO data_objects (file_size, content_hash, reference_count, creation_date, modification_date) \
             VALUES ($1, NULL, 1, NOW(), NOW()) RETURNING id_data_object",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_lookup_hash = CString::new(
            "SELECT id_data_object FROM data_objects WHERE file_size = $1 AND content_hash = $2 LIMIT 1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_touch_existing = CString::new(
            "UPDATE data_objects SET reference_count = reference_count + 1, modification_date = NOW() WHERE id_data_object = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_lookup_request_token = CString::new(
            "SELECT COALESCE((SELECT id_data_object::text FROM data_object_request_tokens WHERE request_token = $1 LIMIT 1), '')",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_store_request_token = CString::new(
            "INSERT INTO data_object_request_tokens (request_token, id_data_object, created_at, updated_at) \
             VALUES ($1, $2, NOW(), NOW()) \
             ON CONFLICT (request_token) DO UPDATE SET updated_at = NOW() \
             RETURNING id_data_object",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_upsert_hash = CString::new(
            "INSERT INTO data_objects AS existing (file_size, content_hash, reference_count, creation_date, modification_date) \
             VALUES ($1, $2, 1, NOW(), NOW()) \
             ON CONFLICT (file_size, content_hash) \
             DO UPDATE SET reference_count = existing.reference_count + 1, modification_date = NOW() \
             RETURNING id_data_object",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            transactional_replay_confirmed(
                conn,
                |conn| {
                    let request_token_params = [&request_token];
                    let existing_token_res =
                        exec_params(conn, &sql_lookup_request_token, &request_token_params)?;
                    let existing_token = fetch_single_text_option(existing_token_res)?;
                    match existing_token {
                        Some(existing) => existing
                            .parse::<u64>()
                            .map(Some)
                            .map_err(|_| "invalid data object request token mapping".to_string()),
                        None => Ok(None),
                    }
                },
                |conn| {
                    let value = match content_hash.as_ref() {
                        Some(content_hash) if hash_dedupe_enabled => {
                            let params = [&file_size, content_hash];
                            let res = exec_params(conn, &sql_upsert_hash, &params)?;
                            let text = fetch_single_text(res)?;
                            text.trim()
                                .parse::<u64>()
                                .map_err(|_| "invalid id_data_object value".to_string())?
                        }
                        Some(content_hash) => {
                            let params = [&file_size, content_hash];
                            let res = exec_params(conn, &sql_lookup_hash, &params)?;
                            let existing = fetch_single_text(res)?;
                            if let Ok(existing_id) = existing.trim().parse::<u64>() {
                                let existing_id_text = CString::new(existing_id.to_string())
                                    .map_err(|_| "data object id contains NUL byte".to_string())?;
                                let params = [&existing_id_text];
                                exec_command_params(conn, &sql_touch_existing, &params)?;
                                existing_id
                            } else {
                                let params = [&file_size, content_hash];
                                let res = exec_params(conn, &sql_insert_hash, &params)?;
                                let text = fetch_single_text(res)?;
                                text.trim()
                                    .parse::<u64>()
                                    .map_err(|_| "invalid id_data_object value".to_string())?
                            }
                        }
                        None => {
                            let params = [&file_size];
                            let res = exec_params(conn, &sql_insert_null, &params)?;
                            let text = fetch_single_text(res)?;
                            text.trim()
                                .parse::<u64>()
                                .map_err(|_| "invalid id_data_object value".to_string())?
                        }
                    };

                    let value_text = CString::new(value.to_string())
                        .map_err(|_| "data object id contains NUL byte".to_string())?;
                    let params = [&request_token, &value_text];
                    let res = exec_params(conn, &sql_store_request_token, &params)?;
                    let stored = fetch_single_text(res)?;
                    let stored_id = stored
                        .trim()
                        .parse::<u64>()
                        .map_err(|_| "invalid data object request token mapping".to_string())?;
                    if stored_id != value {
                        return Err("data object request token mapped to unexpected id".to_string());
                    }
                    Ok(value)
                },
            )
        })
    }

    pub fn touch_data_object(
        &self,
        data_object_id: u64,
        file_size: Option<u64>,
    ) -> Result<bool, String> {
        let sql = match file_size {
            Some(_) => CString::new(
                "UPDATE data_objects SET file_size = $1, modification_date = NOW() WHERE id_data_object = $2",
            )
            .map_err(|_| "SQL contains NUL byte".to_string())?,
            None => CString::new(
                "UPDATE data_objects SET modification_date = NOW() WHERE id_data_object = $1",
            )
            .map_err(|_| "SQL contains NUL byte".to_string())?,
        };

        let data_object_id = CString::new(data_object_id.to_string())
            .map_err(|_| "data object id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let status = match file_size {
                Some(value) => {
                    let file_size = CString::new(value.to_string())
                        .map_err(|_| "file size contains NUL byte".to_string())?;
                    let params = [&file_size, &data_object_id];
                    exec_command_params(conn, &sql, &params)
                }
                None => {
                    let params = [&data_object_id];
                    exec_command_params(conn, &sql, &params)
                }
            };
            match status {
                Ok(()) => Ok(true),
                Err(_) => Err("failed to update data object".to_string()),
            }
        })
    }

    pub fn upsert_index_source_staged(&self, row: &IndexSourceStageRow) -> Result<u64, String> {
        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let stage_sql = CString::new(format!(
                    "
                    CREATE TEMP TABLE IF NOT EXISTS {stage} (
                        name TEXT NOT NULL,
                        kind TEXT NOT NULL,
                        root_path TEXT NOT NULL
                    ) ON COMMIT PRESERVE ROWS;
                    TRUNCATE {stage}
                    ",
                    stage = INDEX_SOURCES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                create_or_reset_temp_table(conn, &stage_sql)?;

                let payload = build_copy_text_payload(std::slice::from_ref(row), |row, out| {
                    append_copy_text_string_field(out, &row.name);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.kind);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.root_path);
                });
                let copy_sql = CString::new(format!(
                    "COPY {stage} (name, kind, root_path) FROM STDIN",
                    stage = INDEX_SOURCES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                copy_text_payload_on_conn(conn, &copy_sql, &payload)?;

                let merge_sql = CString::new(format!(
                    "
                    INSERT INTO index_sources (
                        name,
                        kind,
                        root_path,
                        created_at,
                        updated_at
                    )
                    SELECT
                        name,
                        kind,
                        root_path,
                        NOW(),
                        NOW()
                    FROM {stage}
                    ON CONFLICT (name) DO UPDATE SET
                        kind = EXCLUDED.kind,
                        root_path = EXCLUDED.root_path,
                        updated_at = NOW()
                    RETURNING id_index_source
                    ",
                    stage = INDEX_SOURCES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let rows = query_rows_text_on_conn(conn, &merge_sql)?;
                let row = rows
                    .first()
                    .ok_or_else(|| "source registration did not return a row".to_string())?;
                row.first()
                    .ok_or_else(|| "source registration returned no id".to_string())?
                    .trim()
                    .parse::<u64>()
                    .map_err(|err| format!("invalid source id: {err}"))
            })
        })
    }

    pub fn upsert_index_scan_run_staged(&self, row: &IndexScanRunStageRow) -> Result<u64, String> {
        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let stage_sql = CString::new(format!(
                    "
                    CREATE TEMP TABLE IF NOT EXISTS {stage} (
                        id_index_source BIGINT NOT NULL,
                        status TEXT NOT NULL,
                        request_token TEXT NOT NULL
                    ) ON COMMIT PRESERVE ROWS;
                    TRUNCATE {stage}
                    ",
                    stage = INDEX_SCAN_RUNS_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                create_or_reset_temp_table(conn, &stage_sql)?;

                let payload = build_copy_text_payload(std::slice::from_ref(row), |row, out| {
                    append_copy_text_u64_field(out, row.id_index_source);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.status);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.request_token);
                });
                let copy_sql = CString::new(format!(
                    "COPY {stage} (id_index_source, status, request_token) FROM STDIN",
                    stage = INDEX_SCAN_RUNS_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                copy_text_payload_on_conn(conn, &copy_sql, &payload)?;

                let merge_sql = CString::new(format!(
                    "
                    INSERT INTO index_scan_runs (
                        id_index_source,
                        started_at,
                        status,
                        updated_at,
                        request_token
                    )
                    SELECT
                        id_index_source,
                        NOW(),
                        status,
                        NOW(),
                        request_token
                    FROM {stage}
                    ON CONFLICT (request_token) DO UPDATE SET
                        id_index_source = EXCLUDED.id_index_source,
                        status = EXCLUDED.status,
                        updated_at = NOW()
                    RETURNING id_scan_run
                    ",
                    stage = INDEX_SCAN_RUNS_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let rows = query_rows_text_on_conn(conn, &merge_sql)?;
                let row = rows
                    .first()
                    .ok_or_else(|| "scan run creation did not return a row".to_string())?;
                row.first()
                    .ok_or_else(|| "scan run creation returned no id".to_string())?
                    .trim()
                    .parse::<u64>()
                    .map_err(|err| format!("invalid scan run id: {err}"))
            })
        })
    }

    pub fn upsert_index_import_plan_staged(
        &self,
        row: &IndexImportPlanStageRow,
    ) -> Result<u64, String> {
        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let stage_sql = CString::new(format!(
                    "
                    CREATE TEMP TABLE IF NOT EXISTS {stage} (
                        status TEXT NOT NULL,
                        request_token TEXT NOT NULL,
                        dry_run BOOLEAN NOT NULL,
                        source_filter TEXT
                    ) ON COMMIT PRESERVE ROWS;
                    TRUNCATE {stage}
                    ",
                    stage = INDEX_IMPORT_PLANS_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                create_or_reset_temp_table(conn, &stage_sql)?;

                let payload = build_copy_text_payload(std::slice::from_ref(row), |row, out| {
                    append_copy_text_string_field(out, &row.status);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.request_token);
                    out.push('\t');
                    append_copy_text_bool_field(out, row.dry_run);
                    out.push('\t');
                    match &row.source_filter {
                        Some(value) => append_copy_text_string_field(out, value),
                        None => append_copy_text_null_field(out),
                    }
                });
                let copy_sql = CString::new(format!(
                    "COPY {stage} (status, request_token, dry_run, source_filter) FROM STDIN",
                    stage = INDEX_IMPORT_PLANS_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                copy_text_payload_on_conn(conn, &copy_sql, &payload)?;

                let merge_sql = CString::new(format!(
                    "
                    INSERT INTO index_import_plans (
                        created_at,
                        updated_at,
                        status,
                        request_token,
                        dry_run,
                        source_filter
                    )
                    SELECT
                        NOW(),
                        NOW(),
                        status,
                        request_token,
                        dry_run,
                        source_filter
                    FROM {stage}
                    ON CONFLICT (request_token) DO UPDATE SET
                        status = EXCLUDED.status,
                        dry_run = EXCLUDED.dry_run,
                        source_filter = EXCLUDED.source_filter,
                        updated_at = NOW()
                    RETURNING id_import_plan
                    ",
                    stage = INDEX_IMPORT_PLANS_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let rows = query_rows_text_on_conn(conn, &merge_sql)?;
                let row = rows
                    .first()
                    .ok_or_else(|| "import plan creation did not return a row".to_string())?;
                row.first()
                    .ok_or_else(|| "import plan creation returned no id".to_string())?
                    .trim()
                    .parse::<u64>()
                    .map_err(|err| format!("invalid import plan id: {err}"))
            })
        })
    }

    pub fn upsert_index_files_staged(&self, rows: &[IndexFileStageRow]) -> Result<(), String> {
        if rows.is_empty() {
            return Ok(());
        }

        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let stage_sql = CString::new(format!(
                    "
                    CREATE TEMP TABLE IF NOT EXISTS {stage} (
                        id_index_source BIGINT NOT NULL,
                        id_scan_run BIGINT NOT NULL,
                        path TEXT NOT NULL,
                        size BIGINT NOT NULL,
                        mtime_ns BIGINT,
                        inode BIGINT,
                        device BIGINT,
                        file_kind TEXT NOT NULL,
                        scan_status TEXT NOT NULL,
                        source_changed BOOLEAN NOT NULL
                    ) ON COMMIT PRESERVE ROWS;
                    TRUNCATE {stage}
                    ",
                    stage = INDEX_FILES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                create_or_reset_temp_table(conn, &stage_sql)?;

                let payload = build_copy_text_payload(rows, |row, out| {
                    append_copy_text_u64_field(out, row.id_index_source);
                    out.push('\t');
                    append_copy_text_u64_field(out, row.id_scan_run);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.path);
                    out.push('\t');
                    append_copy_text_u64_field(out, row.size);
                    out.push('\t');
                    append_copy_text_optional_i64_field(out, row.mtime_ns);
                    out.push('\t');
                    append_copy_text_optional_u64_field(out, row.inode);
                    out.push('\t');
                    append_copy_text_optional_u64_field(out, row.device);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.file_kind);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.scan_status);
                    out.push('\t');
                    append_copy_text_bool_field(out, row.source_changed);
                });
                let copy_sql = CString::new(format!(
                    "COPY {stage} (id_index_source, id_scan_run, path, size, mtime_ns, inode, device, file_kind, scan_status, source_changed) FROM STDIN",
                    stage = INDEX_FILES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                copy_text_payload_on_conn(conn, &copy_sql, &payload)?;

                let merge_sql = CString::new(format!(
                    "
                    INSERT INTO index_files (
                        id_index_source,
                        id_scan_run,
                        path,
                        size,
                        mtime_ns,
                        inode,
                        device,
                        file_kind,
                        scan_status,
                        source_changed,
                        created_at,
                        updated_at
                    )
                    SELECT
                        id_index_source,
                        id_scan_run,
                        path,
                        size,
                        mtime_ns,
                        inode,
                        device,
                        file_kind,
                        scan_status,
                        source_changed,
                        NOW(),
                        NOW()
                    FROM {stage}
                    ON CONFLICT (id_index_source, path) DO UPDATE SET
                        id_scan_run = EXCLUDED.id_scan_run,
                        size = EXCLUDED.size,
                        mtime_ns = EXCLUDED.mtime_ns,
                        inode = EXCLUDED.inode,
                        device = EXCLUDED.device,
                        file_kind = EXCLUDED.file_kind,
                        scan_status = EXCLUDED.scan_status,
                        source_changed = EXCLUDED.source_changed,
                        updated_at = NOW()
                    ",
                    stage = INDEX_FILES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                exec_command(conn, &merge_sql)?;
                Ok(())
            })
        })
    }

    pub fn upsert_index_file_hashes_staged(
        &self,
        rows: &[IndexFileHashStageRow],
    ) -> Result<(), String> {
        if rows.is_empty() {
            return Ok(());
        }

        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let stage_sql = CString::new(format!(
                    "
                    CREATE TEMP TABLE IF NOT EXISTS {stage} (
                        id_file BIGINT NOT NULL,
                        hash_algorithm TEXT NOT NULL,
                        partial_hash BYTEA,
                        full_hash BYTEA,
                        hash_status TEXT NOT NULL,
                        observed_size BIGINT NOT NULL,
                        observed_mtime_ns BIGINT,
                        observed_inode BIGINT,
                        observed_device BIGINT
                    ) ON COMMIT PRESERVE ROWS;
                    TRUNCATE {stage}
                    ",
                    stage = INDEX_FILE_HASHES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                create_or_reset_temp_table(conn, &stage_sql)?;

                let payload = build_copy_text_payload(rows, |row, out| {
                    append_copy_text_u64_field(out, row.id_file);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.hash_algorithm);
                    out.push('\t');
                    match &row.partial_hash {
                        Some(bytes) => append_copy_text_bytea_field(out, bytes),
                        None => append_copy_text_null_field(out),
                    }
                    out.push('\t');
                    match &row.full_hash {
                        Some(bytes) => append_copy_text_bytea_field(out, bytes),
                        None => append_copy_text_null_field(out),
                    }
                    out.push('\t');
                    append_copy_text_string_field(out, &row.hash_status);
                    out.push('\t');
                    append_copy_text_u64_field(out, row.observed_size);
                    out.push('\t');
                    append_copy_text_optional_i64_field(out, row.observed_mtime_ns);
                    out.push('\t');
                    append_copy_text_optional_u64_field(out, row.observed_inode);
                    out.push('\t');
                    append_copy_text_optional_u64_field(out, row.observed_device);
                });
                let copy_sql = CString::new(format!(
                    "COPY {stage} (id_file, hash_algorithm, partial_hash, full_hash, hash_status, observed_size, observed_mtime_ns, observed_inode, observed_device) FROM STDIN",
                    stage = INDEX_FILE_HASHES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                copy_text_payload_on_conn(conn, &copy_sql, &payload)?;

                let merge_sql = CString::new(format!(
                    "
                    INSERT INTO index_file_hashes (
                        id_file,
                        hash_algorithm,
                        partial_hash,
                        full_hash,
                        hash_status,
                        observed_size,
                        observed_mtime_ns,
                        observed_inode,
                        observed_device,
                        created_at,
                        updated_at
                    )
                    SELECT
                        id_file,
                        hash_algorithm,
                        partial_hash,
                        full_hash,
                        hash_status,
                        observed_size,
                        observed_mtime_ns,
                        observed_inode,
                        observed_device,
                        NOW(),
                        NOW()
                    FROM {stage}
                    ON CONFLICT (id_file) DO UPDATE SET
                        hash_algorithm = EXCLUDED.hash_algorithm,
                        partial_hash = EXCLUDED.partial_hash,
                        full_hash = EXCLUDED.full_hash,
                        hash_status = EXCLUDED.hash_status,
                        observed_size = EXCLUDED.observed_size,
                        observed_mtime_ns = EXCLUDED.observed_mtime_ns,
                        observed_inode = EXCLUDED.observed_inode,
                        observed_device = EXCLUDED.observed_device,
                        updated_at = NOW()
                    ",
                    stage = INDEX_FILE_HASHES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                exec_command(conn, &merge_sql)?;
                Ok(())
            })
        })
    }

    pub fn upsert_index_import_plan_entries_staged(
        &self,
        rows: &[IndexImportPlanEntryStageRow],
    ) -> Result<(), String> {
        if rows.is_empty() {
            return Ok(());
        }

        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let stage_sql = CString::new(format!(
                    "
                    CREATE TEMP TABLE IF NOT EXISTS {stage} (
                        id_import_plan BIGINT NOT NULL,
                        id_file BIGINT NOT NULL,
                        id_duplicate_set BIGINT,
                        action TEXT NOT NULL,
                        canonical_file_id BIGINT,
                        logical_path TEXT NOT NULL,
                        source_path TEXT NOT NULL,
                        size BIGINT NOT NULL,
                        mtime_ns BIGINT,
                        source_changed BOOLEAN NOT NULL
                    ) ON COMMIT PRESERVE ROWS;
                    TRUNCATE {stage}
                    ",
                    stage = INDEX_IMPORT_PLAN_ENTRIES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                create_or_reset_temp_table(conn, &stage_sql)?;

                let payload = build_copy_text_payload(rows, |row, out| {
                    append_copy_text_u64_field(out, row.id_import_plan);
                    out.push('\t');
                    append_copy_text_u64_field(out, row.id_file);
                    out.push('\t');
                    append_copy_text_optional_u64_field(out, row.id_duplicate_set);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.action);
                    out.push('\t');
                    append_copy_text_optional_u64_field(out, row.canonical_file_id);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.logical_path);
                    out.push('\t');
                    append_copy_text_string_field(out, &row.source_path);
                    out.push('\t');
                    append_copy_text_u64_field(out, row.size);
                    out.push('\t');
                    append_copy_text_optional_i64_field(out, row.mtime_ns);
                    out.push('\t');
                    append_copy_text_bool_field(out, row.source_changed);
                });
                let copy_sql = CString::new(format!(
                    "COPY {stage} (id_import_plan, id_file, id_duplicate_set, action, canonical_file_id, logical_path, source_path, size, mtime_ns, source_changed) FROM STDIN",
                    stage = INDEX_IMPORT_PLAN_ENTRIES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                copy_text_payload_on_conn(conn, &copy_sql, &payload)?;

                let merge_sql = CString::new(format!(
                    "
                    DELETE FROM index_import_plan_entries
                    WHERE id_import_plan IN (
                        SELECT DISTINCT id_import_plan FROM {stage}
                    );
                    INSERT INTO index_import_plan_entries (
                        id_import_plan,
                        id_file,
                        id_duplicate_set,
                        action,
                        canonical_file_id,
                        logical_path,
                        source_path,
                        size,
                        mtime_ns,
                        source_changed,
                        created_at,
                        updated_at
                    )
                    SELECT
                        id_import_plan,
                        id_file,
                        id_duplicate_set,
                        action,
                        canonical_file_id,
                        logical_path,
                        source_path,
                        size,
                        mtime_ns,
                        source_changed,
                        NOW(),
                        NOW()
                    FROM {stage}
                    ",
                    stage = INDEX_IMPORT_PLAN_ENTRIES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                exec_command(conn, &merge_sql)?;
                Ok(())
            })
        })
    }

    pub fn upsert_index_duplicate_sets_staged(
        &self,
        rows: &[DuplicateSetStageRow],
    ) -> Result<u64, String> {
        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let delete_sql = CString::new("DELETE FROM index_duplicate_sets")
                    .map_err(|_| "SQL contains NUL byte".to_string())?;
                exec_command(conn, &delete_sql)?;

                if rows.is_empty() {
                    return Ok(0);
                }

                let stage_sql = CString::new(format!(
                    "
                    CREATE TEMP TABLE IF NOT EXISTS index_duplicate_sets_stage (
                        hash_algorithm TEXT NOT NULL,
                        full_hash BYTEA NOT NULL,
                        file_size BIGINT NOT NULL,
                        file_count INTEGER NOT NULL,
                        total_bytes BIGINT NOT NULL
                    ) ON COMMIT PRESERVE ROWS;
                    TRUNCATE index_duplicate_sets_stage
                    "
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                create_or_reset_temp_table(conn, &stage_sql)?;

                let payload = build_copy_text_payload(rows, |row, out| {
                    append_copy_text_string_field(out, &row.hash_algorithm);
                    out.push('\t');
                    append_copy_text_bytea_field(out, &row.full_hash);
                    out.push('\t');
                    append_copy_text_u64_field(out, row.file_size);
                    out.push('\t');
                    append_copy_text_u64_field(out, row.file_count);
                    out.push('\t');
                    append_copy_text_u64_field(out, row.total_bytes);
                });
                let copy_sql = CString::new(
                    "COPY index_duplicate_sets_stage (hash_algorithm, full_hash, file_size, file_count, total_bytes) FROM STDIN",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                copy_text_payload_on_conn(conn, &copy_sql, &payload)?;

                let merge_sql = CString::new(
                    "
                    INSERT INTO index_duplicate_sets (
                        hash_algorithm,
                        full_hash,
                        file_size,
                        file_count,
                        total_bytes,
                        created_at,
                        updated_at
                    )
                    SELECT
                        hash_algorithm,
                        full_hash,
                        file_size,
                        file_count,
                        total_bytes,
                        NOW(),
                        NOW()
                    FROM index_duplicate_sets_stage
                    ON CONFLICT (hash_algorithm, full_hash, file_size) DO UPDATE SET
                        file_count = EXCLUDED.file_count,
                        total_bytes = EXCLUDED.total_bytes,
                        updated_at = NOW()
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                exec_command(conn, &merge_sql)?;
                Ok(rows.len() as u64)
            })
        })
    }

    pub fn query_scalar_text_stale_sensitive(&self, sql: &str) -> Result<String, String> {
        let sql = CString::new(sql).map_err(|_| "SQL contains NUL byte".to_string())?;
        if !sql_is_read_only(&sql) {
            return Err("stale-sensitive replica routing accepts read-only SQL only".to_string());
        }
        self.with_read_connection(|conn| unsafe { query_scalar_text_on_conn(conn, &sql) })
    }

    pub fn query_scalar_text(&self, sql: &str) -> Result<String, String> {
        let sql = CString::new(sql).map_err(|_| "SQL contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe { query_scalar_text_on_conn(conn, &sql) })
    }

    pub fn query_rows_text(&self, sql: &str) -> Result<Vec<Vec<String>>, String> {
        let sql = CString::new(sql).map_err(|_| "SQL contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe { query_rows_text_on_conn(conn, &sql) })
    }

    pub fn source_snapshot(&self) -> Result<DbRepoSourceSnapshot, String> {
        let value = self.query_scalar_text_stale_sensitive(
            "
            SELECT
                CASE
                    WHEN pg_is_in_recovery() THEN 'replica'
                    WHEN current_setting('transaction_read_only')::boolean THEN 'primary-read-only'
                    ELSE 'primary-writable'
                END
                || '|' || current_setting('transaction_read_only')
                || '|' || COALESCE(
                    CASE
                        WHEN pg_is_in_recovery() THEN
                            GREATEST(
                                COALESCE(
                                    pg_wal_lsn_diff(
                                        COALESCE(pg_last_wal_receive_lsn(), pg_last_wal_replay_lsn()),
                                        pg_last_wal_replay_lsn()
                                    ),
                                    0
                                ),
                                0
                            )::bigint::text
                        ELSE NULL
                    END,
                    ''
                )
            ",
        )?;
        let fields = value.trim().split('|').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(format!(
                "invalid PostgreSQL source snapshot `{}`: expected role|transaction_read_only|wal_lag",
                value.trim()
            ));
        }
        let transaction_read_only = match fields[1].trim().to_ascii_lowercase().as_str() {
            "t" | "true" | "on" | "1" => true,
            "f" | "false" | "off" | "0" => false,
            other => {
                return Err(format!(
                    "invalid PostgreSQL source transaction_read_only value `{other}`"
                ));
            }
        };
        let wal_replay_lag_bytes = if fields[2].trim().is_empty() {
            None
        } else {
            Some(fields[2].trim().parse::<u64>().map_err(|err| {
                format!(
                    "invalid PostgreSQL source WAL lag value `{}`: {err}",
                    fields[2]
                )
            })?)
        };
        Ok(DbRepoSourceSnapshot {
            data_source_role: fields[0].trim().to_string(),
            transaction_read_only,
            wal_replay_lag_bytes,
        })
    }

    pub fn exec(&self, sql: &str) -> Result<(), String> {
        let sql = CString::new(sql).map_err(|_| "SQL contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let res = PQexec(conn, sql.as_ptr());
            if res.is_null() {
                let err = conn_error(conn);
                return Err(maybe_replayable_sql_error(conn, &sql, err));
            }
            let status = PQresultStatus(res);
            if status == PGRES_COMMAND_OK {
                PQclear(res);
                Ok(())
            } else {
                let error = result_error(res);
                PQclear(res);
                Err(maybe_replayable_sql_error(conn, &sql, error))
            }
        })
    }

    pub fn quote_identifier(ident: &str) -> String {
        format!("\"{}\"", ident.replace('\"', "\"\""))
    }

    pub fn quote_literal(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    unsafe fn query_config_value_on_conn(
        conn: *mut PGconn,
        key: &str,
    ) -> Result<Option<String>, String> {
        let sql = CString::new("SELECT value FROM config WHERE key = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let key = CString::new(key).map_err(|_| "config key contains NUL byte".to_string())?;
        let params = [&key];
        let res = exec_params(conn, &sql, &params)?;
        let rows = PQntuples(res);
        let cols = PQnfields(res);
        let value = if rows < 1 || cols < 1 {
            None
        } else {
            let value_ptr = PQgetvalue(res, 0, 0);
            if value_ptr.is_null() {
                None
            } else {
                Some(CStr::from_ptr(value_ptr).to_string_lossy().to_string())
            }
        };
        PQclear(res);
        Ok(value)
    }

    pub fn query_config_value(&self, key: &str) -> Result<Option<String>, String> {
        self.with_cached_connection(|conn| unsafe { Self::query_config_value_on_conn(conn, key) })
    }

    unsafe fn is_in_recovery_on_conn(conn: *mut PGconn) -> Result<bool, String> {
        let sql = CString::new("SELECT pg_is_in_recovery()")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let value = query_scalar_text_on_conn(conn, &sql)?;
        Ok(matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        ))
    }

    pub fn is_in_recovery(&self) -> Result<bool, String> {
        self.with_cached_connection(|conn| unsafe { Self::is_in_recovery_on_conn(conn) })
    }

    unsafe fn schema_version_on_conn(conn: *mut PGconn) -> Result<Option<u32>, String> {
        let sql =
            CString::new("SELECT version FROM schema_version ORDER BY applied_at DESC LIMIT 1")
                .map_err(|_| "SQL contains NUL byte".to_string())?;
        let value = query_scalar_text_on_conn(conn, &sql)?;
        if value.trim().is_empty() {
            return Ok(None);
        }
        value
            .trim()
            .parse::<u32>()
            .map(Some)
            .map_err(|err| format!("invalid schema version returned by PostgreSQL: {err}"))
    }

    pub fn schema_version(&self) -> Result<Option<u32>, String> {
        self.with_cached_connection(|conn| unsafe { Self::schema_version_on_conn(conn) })
    }

    unsafe fn schema_is_initialized_on_conn(conn: *mut PGconn) -> Result<bool, String> {
        let sql = CString::new(format!(
            "SELECT \
                to_regclass('{schema}.directories') IS NOT NULL AND \
                to_regclass('{schema}.files') IS NOT NULL AND \
                to_regclass('{schema}.schema_version') IS NOT NULL",
            schema = FOD_SCHEMA_NAME
        ))
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let value = query_scalar_text_on_conn(conn, &sql)?;
        Ok(matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        ))
    }

    pub fn schema_is_initialized(&self) -> Result<bool, String> {
        self.with_cached_connection(|conn| unsafe { Self::schema_is_initialized_on_conn(conn) })
    }

    unsafe fn startup_probe_on_conn(conn: *mut PGconn) -> Result<(bool, bool), String> {
        // Pierwszy round-trip jest bezpieczny takze przed inicjalizacja FOD:
        // odwoluje sie tylko do pg_catalog/to_regclass i pg_is_in_recovery().
        let sql = CString::new(format!(
            "SELECT \
                (\
                    to_regclass('{schema}.directories') IS NOT NULL AND \
                    to_regclass('{schema}.files') IS NOT NULL AND \
                    to_regclass('{schema}.schema_version') IS NOT NULL\
                )::text, \
                pg_is_in_recovery()::text",
            schema = FOD_SCHEMA_NAME
        ))
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let rows = query_rows_text_on_conn(conn, &sql)?;
        let row = rows
            .first()
            .ok_or_else(|| "startup probe returned no rows".to_string())?;
        if row.len() < 2 {
            return Err(format!(
                "startup probe returned {} columns, expected at least 2",
                row.len()
            ));
        }

        let schema_is_initialized = matches!(
            row[0].trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        );
        let is_in_recovery = matches!(
            row[1].trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        );
        Ok((schema_is_initialized, is_in_recovery))
    }

    unsafe fn startup_initialized_values_on_conn(conn: *mut PGconn) -> Result<(u32, u32), String> {
        // Drugie zapytanie jest wykonywane dopiero po potwierdzeniu schematu.
        // EXISTS rozroznia brak rekordu config od obecnego, ale pustego rekordu,
        // aby zachowac komunikaty i walidacje z poprzedniej implementacji.
        let sql = CString::new(
            "SELECT \
                (EXISTS (SELECT 1 FROM config WHERE key = 'block_size'))::text, \
                COALESCE((SELECT value::text FROM config WHERE key = 'block_size' LIMIT 1), ''), \
                (EXISTS (SELECT 1 FROM config WHERE key = 'max_fs_size_bytes'))::text, \
                COALESCE((SELECT value::text FROM config WHERE key = 'max_fs_size_bytes' LIMIT 1), ''), \
                COALESCE((\
                    SELECT version::text \
                    FROM schema_version \
                    ORDER BY applied_at DESC \
                    LIMIT 1\
                ), '')",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let rows = query_rows_text_on_conn(conn, &sql)?;
        let row = rows
            .first()
            .ok_or_else(|| "initialized startup values query returned no rows".to_string())?;
        if row.len() < 5 {
            return Err(format!(
                "initialized startup values query returned {} columns, expected at least 5",
                row.len()
            ));
        }

        let block_size_present = matches!(
            row[0].trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        );
        if !block_size_present {
            return Err("initialized FOD schema is missing config.block_size".to_string());
        }
        let block_size = row[1]
            .trim()
            .parse::<u32>()
            .map_err(|err| format!("invalid config.block_size value: {err}"))?;
        if block_size == 0 {
            return Err("config.block_size must be greater than zero".to_string());
        }

        let max_fs_size_bytes_present = matches!(
            row[2].trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        );
        if !max_fs_size_bytes_present {
            return Err("initialized FOD schema is missing config.max_fs_size_bytes".to_string());
        }
        row[3]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid config.max_fs_size_bytes value: {err}"))?;

        let schema_version_value = row[4].trim();
        if schema_version_value.is_empty() {
            return Err("initialized FOD schema is missing schema version".to_string());
        }
        let schema_version = schema_version_value
            .parse::<u32>()
            .map_err(|err| format!("invalid schema version returned by PostgreSQL: {err}"))?;

        Ok((block_size, schema_version))
    }

    unsafe fn startup_snapshot_on_conn(
        &self,
        conn: *mut PGconn,
    ) -> Result<StartupSnapshot, String> {
        // Caly logiczny snapshot nadal pozostaje na jednym PGconn.
        // Niezainicjalizowany schemat konczy sie po jednym zapytaniu.
        // Zainicjalizowany schemat wymaga maksymalnie dwoch zapytan.
        let (schema_is_initialized, is_in_recovery) = Self::startup_probe_on_conn(conn)?;

        if !schema_is_initialized {
            return Ok(StartupSnapshot {
                block_size: None,
                is_in_recovery,
                schema_version: None,
                schema_is_initialized: false,
            });
        }

        let (block_size, schema_version) = Self::startup_initialized_values_on_conn(conn)?;

        Ok(StartupSnapshot {
            block_size: Some(block_size),
            is_in_recovery,
            schema_version: Some(schema_version),
            schema_is_initialized: true,
        })
    }

    pub fn startup_snapshot(&self) -> Result<StartupSnapshot, String> {
        self.with_cached_connection(|conn| unsafe { self.startup_snapshot_on_conn(conn) })
    }

    fn query_schema_ready_bool(&self, sql: &str) -> Result<bool, String> {
        let value = self.query_scalar_text(sql)?;
        Ok(matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        ))
    }

    fn runtime_lock_schema_is_ready(&self) -> Result<bool, String> {
        let sql = format!(
            "
            SELECT CASE WHEN
                to_regclass('{schema}.lock_leases') IS NOT NULL
                AND to_regclass('{schema}.lock_lease_request_tokens') IS NOT NULL
                AND to_regclass('{schema}.lock_range_leases') IS NOT NULL
                AND EXISTS (
                    SELECT 1
                    FROM information_schema.columns
                    WHERE table_schema = '{schema}'
                      AND table_name = 'lock_leases'
                      AND column_name = 'session_id'
                      AND is_nullable = 'NO'
                )
                AND EXISTS (
                    SELECT 1
                    FROM information_schema.columns
                    WHERE table_schema = '{schema}'
                      AND table_name = 'lock_leases'
                      AND column_name = 'request_token'
                )
                AND EXISTS (
                    SELECT 1
                    FROM information_schema.columns
                    WHERE table_schema = '{schema}'
                      AND table_name = 'lock_lease_request_tokens'
                      AND column_name = 'request_token'
                )
                AND EXISTS (
                    SELECT 1
                    FROM information_schema.columns
                    WHERE table_schema = '{schema}'
                      AND table_name = 'lock_lease_request_tokens'
                      AND column_name = 'did_grant'
                )
                AND EXISTS (
                    SELECT 1
                    FROM information_schema.columns
                    WHERE table_schema = '{schema}'
                      AND table_name = 'lock_range_leases'
                      AND column_name = 'session_id'
                      AND is_nullable = 'NO'
                )
                AND EXISTS (
                    SELECT 1
                    FROM pg_indexes
                    WHERE schemaname = '{schema}'
                      AND indexname = 'idx_lock_leases_identity'
                )
                AND EXISTS (
                    SELECT 1
                    FROM pg_indexes
                    WHERE schemaname = '{schema}'
                      AND indexname = 'idx_lock_leases_request_token'
                )
                AND EXISTS (
                    SELECT 1
                    FROM pg_indexes
                    WHERE schemaname = '{schema}'
                      AND indexname = 'idx_lock_range_leases_session'
                )
            THEN '1' ELSE '0' END
            ",
            schema = FOD_SCHEMA_NAME
        );

        self.query_schema_ready_bool(&sql)
    }

    fn runtime_client_session_schema_is_ready(&self) -> Result<bool, String> {
        let sql = format!(
            "
            SELECT CASE WHEN
                to_regclass('{schema}.client_sessions') IS NOT NULL
                AND to_regclass('{schema}.client_session_owner_keys') IS NOT NULL
                AND to_regclass('{schema}.monitor_session_stats') IS NOT NULL
                AND EXISTS (
                    SELECT 1
                    FROM information_schema.columns
                    WHERE table_schema = '{schema}'
                      AND table_name = 'client_sessions'
                      AND column_name = 'session_id'
                )
                AND EXISTS (
                    SELECT 1
                    FROM information_schema.columns
                    WHERE table_schema = '{schema}'
                      AND table_name = 'client_sessions'
                      AND column_name = 'request_token'
                )
                AND EXISTS (
                    SELECT 1
                    FROM information_schema.columns
                    WHERE table_schema = '{schema}'
                      AND table_name = 'client_session_owner_keys'
                      AND column_name = 'owner_key'
                )
                AND EXISTS (
                    SELECT 1
                    FROM pg_indexes
                    WHERE schemaname = '{schema}'
                      AND indexname = 'idx_client_sessions_expires'
                )
                AND EXISTS (
                    SELECT 1
                    FROM pg_indexes
                    WHERE schemaname = '{schema}'
                      AND indexname = 'idx_client_sessions_request_token'
                )
                AND EXISTS (
                    SELECT 1
                    FROM pg_indexes
                    WHERE schemaname = '{schema}'
                      AND indexname = 'idx_client_session_owner_keys_owner'
                )
                AND EXISTS (
                    SELECT 1
                    FROM pg_indexes
                    WHERE schemaname = '{schema}'
                      AND indexname = 'idx_monitor_session_stats_sampled'
                )
                AND EXISTS (
                    SELECT 1
                    FROM pg_proc p
                    JOIN pg_namespace n ON n.oid = p.pronamespace
                    WHERE n.nspname = '{schema}'
                      AND p.proname = 'fod_prune_client_session_lock_leases'
                )
                AND EXISTS (
                    SELECT 1
                    FROM pg_trigger t
                    JOIN pg_class c ON c.oid = t.tgrelid
                    JOIN pg_namespace n ON n.oid = c.relnamespace
                    WHERE n.nspname = '{schema}'
                      AND c.relname = 'client_sessions'
                      AND t.tgname = 'fod_client_sessions_prune_lock_leases'
                      AND NOT t.tgisinternal
                )
            THEN '1' ELSE '0' END
            ",
            schema = FOD_SCHEMA_NAME
        );

        self.query_schema_ready_bool(&sql)
    }

    pub fn ensure_lock_schema(&self) -> Result<(), String> {
        if self.runtime_lock_schema_is_ready()? {
            if let Ok(mut guard) = self.lock_schema_ready.lock() {
                *guard = true;
            }
            return Ok(());
        }
        const FOD_RUNTIME_SCHEMA_DDL_LOCK_SQL: &str = "SELECT pg_advisory_lock(4466778912201122)";
        const FOD_RUNTIME_SCHEMA_DDL_UNLOCK_SQL: &str =
            "SELECT pg_advisory_unlock(4466778912201122)";

        // Serializuje runtime DDL miedzy rownolegle startowanymi mountami/testami.
        // Uzywamy locka sesyjnego, bo lokalny kod nie zawsze ma jawne BEGIN przy markerze DDL.
        let _ = self.query_scalar_text(FOD_RUNTIME_SCHEMA_DDL_LOCK_SQL)?;
        let fod_runtime_schema_ddl_result = (|| -> Result<(), String> {
            if self.runtime_lock_schema_is_ready()? {
                if let Ok(mut guard) = self.lock_schema_ready.lock() {
                    *guard = true;
                }
                return Ok(());
            }

            self.with_control_connection(|conn| unsafe {
                transactional_replayable(conn, |conn| {
                    let statements = [
                        CString::new(
                            "
                            CREATE TABLE IF NOT EXISTS lock_leases (
                                id_lock SERIAL PRIMARY KEY,
                                resource_kind VARCHAR(20) NOT NULL,
                                resource_id BIGINT NOT NULL,
                                session_id BIGINT NOT NULL DEFAULT 0,
                                owner_key NUMERIC(20,0) NOT NULL,
                                lease_kind VARCHAR(20) NOT NULL,
                                lock_type INTEGER NOT NULL,
                                lease_expires_at TIMESTAMP NOT NULL,
                                heartbeat_at TIMESTAMP NOT NULL,
                                created_at TIMESTAMP NOT NULL DEFAULT NOW(),
                                updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
                                UNIQUE(resource_kind, resource_id, session_id, owner_key, lease_kind)
                            )
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_leases
                            ADD COLUMN IF NOT EXISTS session_id BIGINT DEFAULT 0
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_leases
                            ADD COLUMN IF NOT EXISTS request_token TEXT
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            UPDATE lock_leases
                            SET session_id = 0
                            WHERE session_id IS NULL
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_leases
                            ALTER COLUMN session_id SET DEFAULT 0
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_leases
                            ALTER COLUMN session_id SET NOT NULL
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_leases
                            DROP CONSTRAINT IF EXISTS lock_leases_resource_kind_resource_id_owner_key_lease_kind_key
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_leases
                            ALTER COLUMN owner_key TYPE NUMERIC(20,0)
                            USING owner_key::numeric
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_lock_leases_resource
                            ON lock_leases (resource_kind, resource_id, lease_kind)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_lock_leases_expires
                            ON lock_leases (lease_expires_at)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_lock_leases_owner
                            ON lock_leases (owner_key)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_lock_leases_session
                            ON lock_leases (session_id)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE UNIQUE INDEX IF NOT EXISTS idx_lock_leases_identity
                            ON lock_leases (resource_kind, resource_id, session_id, owner_key, lease_kind)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE UNIQUE INDEX IF NOT EXISTS idx_lock_leases_request_token
                            ON lock_leases (request_token)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE TABLE IF NOT EXISTS lock_lease_request_tokens (
                                request_token TEXT PRIMARY KEY,
                                did_grant BOOLEAN NOT NULL,
                                created_at TIMESTAMP NOT NULL DEFAULT NOW(),
                                updated_at TIMESTAMP NOT NULL DEFAULT NOW()
                            )
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE TABLE IF NOT EXISTS lock_range_leases (
                                id_lock SERIAL PRIMARY KEY,
                                resource_kind VARCHAR(20) NOT NULL,
                                resource_id BIGINT NOT NULL,
                                session_id BIGINT NOT NULL DEFAULT 0,
                                owner_key NUMERIC(20,0) NOT NULL,
                                lock_type INTEGER NOT NULL,
                                range_start BIGINT NOT NULL,
                                range_end BIGINT NULL,
                                lease_expires_at TIMESTAMP NOT NULL,
                                heartbeat_at TIMESTAMP NOT NULL,
                                created_at TIMESTAMP NOT NULL DEFAULT NOW(),
                                updated_at TIMESTAMP NOT NULL DEFAULT NOW()
                            )
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_range_leases
                            ADD COLUMN IF NOT EXISTS session_id BIGINT DEFAULT 0
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            UPDATE lock_range_leases
                            SET session_id = 0
                            WHERE session_id IS NULL
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_range_leases
                            ALTER COLUMN session_id SET DEFAULT 0
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_range_leases
                            ALTER COLUMN session_id SET NOT NULL
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_range_leases
                            ALTER COLUMN owner_key TYPE NUMERIC(20,0)
                            USING owner_key::numeric
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_lock_range_leases_resource
                            ON lock_range_leases (resource_kind, resource_id)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_lock_range_leases_expires
                            ON lock_range_leases (lease_expires_at)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_lock_range_leases_owner
                            ON lock_range_leases (owner_key)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_lock_range_leases_session
                            ON lock_range_leases (session_id)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                    ];
                    for statement in statements.iter() {
                        exec_command(conn, statement)?;
                    }
                    Ok(())
                })
            })?;
            if let Ok(mut guard) = self.lock_schema_ready.lock() {
                *guard = true;
            }
            Ok(())
        })();

        let fod_runtime_schema_ddl_unlock_result =
            self.query_scalar_text(FOD_RUNTIME_SCHEMA_DDL_UNLOCK_SQL);
        match (
            fod_runtime_schema_ddl_result,
            fod_runtime_schema_ddl_unlock_result,
        ) {
            (Ok(()), Ok(_)) => Ok(()),
            (Err(err), _) => Err(err),
            (Ok(()), Err(err)) => Err(err),
        }
    }

    pub fn ensure_client_session_schema(&self) -> Result<(), String> {
        if self.runtime_lock_schema_is_ready()? && self.runtime_client_session_schema_is_ready()? {
            if let Ok(mut guard) = self.lock_schema_ready.lock() {
                *guard = true;
            }
            return Ok(());
        }
        const FOD_RUNTIME_SCHEMA_DDL_LOCK_SQL: &str = "SELECT pg_advisory_lock(4466778912201122)";
        const FOD_RUNTIME_SCHEMA_DDL_UNLOCK_SQL: &str =
            "SELECT pg_advisory_unlock(4466778912201122)";

        // Serializuje runtime DDL miedzy rownolegle startowanymi mountami/testami.
        // Uzywamy locka sesyjnego, bo lokalny kod nie zawsze ma jawne BEGIN przy markerze DDL.
        let _ = self.query_scalar_text(FOD_RUNTIME_SCHEMA_DDL_LOCK_SQL)?;
        let fod_runtime_schema_ddl_result = (|| -> Result<(), String> {
            if self.runtime_lock_schema_is_ready()?
                && self.runtime_client_session_schema_is_ready()?
            {
                if let Ok(mut guard) = self.lock_schema_ready.lock() {
                    *guard = true;
                }
                return Ok(());
            }

            self.ensure_lock_schema_ready()?;
            self.with_control_connection(|conn| unsafe {
                transactional_replayable(conn, |conn| {
                    let statements = [
                        CString::new(
                            "
                            CREATE TABLE IF NOT EXISTS client_sessions (
                                session_id BIGSERIAL PRIMARY KEY,
                                host_name VARCHAR(255) NOT NULL,
                                mountpoint TEXT NOT NULL,
                                mount_mode VARCHAR(20) NOT NULL,
                                lock_backend VARCHAR(20) NOT NULL,
                                pid BIGINT NOT NULL,
                                request_token TEXT,
                                lease_expires_at TIMESTAMP NOT NULL,
                                heartbeat_at TIMESTAMP NOT NULL,
                                last_lock_at TIMESTAMP NULL,
                                last_write_at TIMESTAMP NULL,
                                started_at TIMESTAMP NOT NULL DEFAULT NOW(),
                                updated_at TIMESTAMP NOT NULL DEFAULT NOW()
                            )
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_client_sessions_expires
                            ON client_sessions (lease_expires_at)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS client_sessions
                            ADD COLUMN IF NOT EXISTS request_token TEXT
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE UNIQUE INDEX IF NOT EXISTS idx_client_sessions_request_token
                            ON client_sessions (request_token)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE TABLE IF NOT EXISTS client_session_owner_keys (
                                session_id BIGINT NOT NULL REFERENCES client_sessions(session_id) ON DELETE CASCADE,
                                owner_key NUMERIC(20,0) NOT NULL,
                                first_seen_at TIMESTAMP NOT NULL DEFAULT NOW(),
                                last_seen_at TIMESTAMP NOT NULL DEFAULT NOW(),
                                updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
                                PRIMARY KEY(session_id, owner_key)
                            )
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS client_session_owner_keys
                            ALTER COLUMN owner_key TYPE NUMERIC(20,0)
                            USING owner_key::numeric
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_client_session_owner_keys_owner
                            ON client_session_owner_keys (owner_key)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_client_session_owner_keys_last_seen
                            ON client_session_owner_keys (last_seen_at)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE TABLE IF NOT EXISTS monitor_session_stats (
                                session_id BIGINT PRIMARY KEY REFERENCES client_sessions(session_id) ON DELETE CASCADE,
                                fod_version VARCHAR(32) NOT NULL,
                                sample_seq BIGINT NOT NULL DEFAULT 0,
                                sampled_at TIMESTAMP NOT NULL DEFAULT NOW(),
                                payload_json JSONB NOT NULL DEFAULT '{}'::jsonb,
                                updated_at TIMESTAMP NOT NULL DEFAULT NOW()
                            )
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE INDEX IF NOT EXISTS idx_monitor_session_stats_sampled
                            ON monitor_session_stats (sampled_at)
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE OR REPLACE FUNCTION fod_prune_client_session_lock_leases()
                            RETURNS trigger AS $$
                            BEGIN
                                DELETE FROM lock_leases
                                WHERE session_id = OLD.session_id;
                                DELETE FROM lock_range_leases
                                WHERE session_id = OLD.session_id;
                                RETURN OLD;
                            END;
                            $$ LANGUAGE plpgsql
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            DROP TRIGGER IF EXISTS fod_client_sessions_prune_lock_leases
                            ON client_sessions
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            CREATE TRIGGER fod_client_sessions_prune_lock_leases
                            BEFORE DELETE ON client_sessions
                            FOR EACH ROW
                            EXECUTE FUNCTION fod_prune_client_session_lock_leases()
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                    ];
                    for statement in statements.iter() {
                        exec_command(conn, statement)?;
                    }
                    let backfill_statements = [
                        CString::new(
                            "
                            UPDATE lock_leases
                            SET session_id = mapping.session_id
                            FROM (
                                SELECT DISTINCT ON (owner_key)
                                    owner_key,
                                    session_id
                                FROM client_session_owner_keys
                                ORDER BY owner_key, last_seen_at DESC, session_id DESC
                            ) AS mapping
                            WHERE lock_leases.session_id = 0
                              AND lock_leases.owner_key = mapping.owner_key
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            UPDATE lock_range_leases
                            SET session_id = mapping.session_id
                            FROM (
                                SELECT DISTINCT ON (owner_key)
                                    owner_key,
                                    session_id
                                FROM client_session_owner_keys
                                ORDER BY owner_key, last_seen_at DESC, session_id DESC
                            ) AS mapping
                            WHERE lock_range_leases.session_id = 0
                              AND lock_range_leases.owner_key = mapping.owner_key
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            DELETE FROM lock_leases
                            WHERE session_id = 0
                              AND lease_expires_at <= NOW()
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            DELETE FROM lock_range_leases
                            WHERE session_id = 0
                              AND lease_expires_at <= NOW()
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_leases
                            ALTER COLUMN session_id DROP DEFAULT
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                        CString::new(
                            "
                            ALTER TABLE IF EXISTS lock_range_leases
                            ALTER COLUMN session_id DROP DEFAULT
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?,
                    ];
                    for statement in backfill_statements.iter() {
                        exec_command(conn, statement)?;
                    }
                    Ok(())
                })
            })
        })();

        let fod_runtime_schema_ddl_unlock_result =
            self.query_scalar_text(FOD_RUNTIME_SCHEMA_DDL_UNLOCK_SQL);
        match (
            fod_runtime_schema_ddl_result,
            fod_runtime_schema_ddl_unlock_result,
        ) {
            (Ok(()), Ok(_)) => Ok(()),
            (Err(err), _) => Err(err),
            (Ok(()), Err(err)) => Err(err),
        }
    }

    pub fn prune_lock_leases(
        &self,
        resource_kind: Option<&str>,
        resource_id: Option<u64>,
    ) -> Result<(), String> {
        self.ensure_lock_schema_ready()?;
        self.with_control_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                match (resource_kind, resource_id) {
                    (None, None) => {
                        let sql =
                            CString::new("DELETE FROM lock_leases WHERE lease_expires_at <= NOW()")
                                .map_err(|_| "SQL contains NUL byte".to_string())?;
                        exec_command(conn, &sql)?;
                    }
                    (Some(resource_kind), Some(resource_id)) => {
                        let sql = CString::new(
                            "
                            DELETE FROM lock_leases
                            WHERE resource_kind = $1
                              AND resource_id = $2
                              AND lease_kind = $3
                              AND lease_expires_at <= NOW()
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?;
                        let resource_kind = CString::new(resource_kind)
                            .map_err(|_| "resource kind contains NUL byte".to_string())?;
                        let resource_id = CString::new(resource_id.to_string())
                            .map_err(|_| "resource id contains NUL byte".to_string())?;
                        let lease_kind = CString::new("flock")
                            .map_err(|_| "lease kind contains NUL byte".to_string())?;
                        let params = [&resource_kind, &resource_id, &lease_kind];
                        exec_command_params(conn, &sql, &params)?;
                    }
                    _ => {
                        return Err(
                            "resource kind and resource id must be provided together".to_string()
                        )
                    }
                }
                Ok(())
            })
        })
    }

    pub fn delete_lock_lease(
        &self,
        resource_kind: &str,
        resource_id: u64,
        owner_key: u64,
    ) -> Result<(), String> {
        self.ensure_lock_schema_ready()?;
        let session_id = self.session_id_for_owner_key_text(owner_key)?;
        let sql = CString::new(
            "
            DELETE FROM lock_leases
            WHERE resource_kind = $1
              AND resource_id = $2
              AND session_id = $3
              AND owner_key = $4
              AND lease_kind = $5
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let resource_kind = CString::new(resource_kind)
            .map_err(|_| "resource kind contains NUL byte".to_string())?;
        let resource_id = CString::new(resource_id.to_string())
            .map_err(|_| "resource id contains NUL byte".to_string())?;
        let session_id_text = session_id;
        let owner_key = CString::new(owner_key.to_string())
            .map_err(|_| "owner key contains NUL byte".to_string())?;
        let lease_kind =
            CString::new("flock").map_err(|_| "lease kind contains NUL byte".to_string())?;

        self.with_control_connection(|conn| unsafe {
            let params = [
                &resource_kind,
                &resource_id,
                &session_id_text,
                &owner_key,
                &lease_kind,
            ];
            exec_command_params(conn, &sql, &params).map(|_| ())
        })
    }

    pub fn prune_lock_range_leases(
        &self,
        resource_kind: Option<&str>,
        resource_id: Option<u64>,
    ) -> Result<(), String> {
        self.ensure_lock_schema_ready()?;
        self.with_control_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                match (resource_kind, resource_id) {
                    (None, None) => {
                        let sql = CString::new(
                            "DELETE FROM lock_range_leases WHERE lease_expires_at <= NOW()",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?;
                        exec_command(conn, &sql)?;
                    }
                    (Some(resource_kind), Some(resource_id)) => {
                        let sql = CString::new(
                            "
                            DELETE FROM lock_range_leases
                            WHERE resource_kind = $1
                              AND resource_id = $2
                              AND lease_expires_at <= NOW()
                            ",
                        )
                        .map_err(|_| "SQL contains NUL byte".to_string())?;
                        let resource_kind = CString::new(resource_kind)
                            .map_err(|_| "resource kind contains NUL byte".to_string())?;
                        let resource_id = CString::new(resource_id.to_string())
                            .map_err(|_| "resource id contains NUL byte".to_string())?;
                        let params = [&resource_kind, &resource_id];
                        exec_command_params(conn, &sql, &params)?;
                    }
                    _ => {
                        return Err(
                            "resource kind and resource id must be provided together".to_string()
                        )
                    }
                }
                Ok(())
            })
        })
    }

    pub fn delete_range_leases(
        &self,
        resource_kind: &str,
        resource_id: u64,
        owner_key: Option<u64>,
    ) -> Result<(), String> {
        self.ensure_lock_schema_ready()?;
        let (sql, params): (CString, Vec<CString>) = if let Some(owner_key) = owner_key {
            let session_id = self.session_id_for_owner_key_text(owner_key)?;
            (
                CString::new(
                    "
                    DELETE FROM lock_range_leases
                    WHERE resource_kind = $1
                      AND resource_id = $2
                      AND session_id = $3
                      AND owner_key = $4
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![
                    CString::new(resource_kind)
                        .map_err(|_| "resource kind contains NUL byte".to_string())?,
                    CString::new(resource_id.to_string())
                        .map_err(|_| "resource id contains NUL byte".to_string())?,
                    session_id,
                    CString::new(owner_key.to_string())
                        .map_err(|_| "owner key contains NUL byte".to_string())?,
                ],
            )
        } else {
            (
                CString::new(
                    "
                    DELETE FROM lock_range_leases
                    WHERE resource_kind = $1
                      AND resource_id = $2
                      AND session_id = $3
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![
                    CString::new(resource_kind)
                        .map_err(|_| "resource kind contains NUL byte".to_string())?,
                    CString::new(resource_id.to_string())
                        .map_err(|_| "resource id contains NUL byte".to_string())?,
                    self.current_lock_session_id_text()?,
                ],
            )
        };

        self.with_control_connection(|conn| unsafe {
            let params_ref = params.iter().collect::<Vec<_>>();
            exec_command_params(conn, &sql, &params_ref).map(|_| ())
        })
    }

    pub fn delete_lock_leases_for_owner(&self, owner_key: u64) -> Result<(), String> {
        self.ensure_lock_schema_ready()?;
        let session_id = self.session_id_for_owner_key_text(owner_key)?;
        let sql = CString::new(
            "
            DELETE FROM lock_leases
            WHERE session_id = $1
              AND owner_key = $2
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let session_id = session_id;
        let owner_key = CString::new(owner_key.to_string())
            .map_err(|_| "owner key contains NUL byte".to_string())?;

        self.with_control_connection(|conn| unsafe {
            let params = [&session_id, &owner_key];
            exec_command_params(conn, &sql, &params).map(|_| ())
        })
    }

    pub fn delete_lock_range_leases_for_owner(&self, owner_key: u64) -> Result<(), String> {
        self.ensure_lock_schema_ready()?;
        let session_id = self.session_id_for_owner_key_text(owner_key)?;
        let sql = CString::new(
            "
            DELETE FROM lock_range_leases
            WHERE session_id = $1
              AND owner_key = $2
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let session_id = session_id;
        let owner_key = CString::new(owner_key.to_string())
            .map_err(|_| "owner key contains NUL byte".to_string())?;

        self.with_control_connection(|conn| unsafe {
            let params = [&session_id, &owner_key];
            exec_command_params(conn, &sql, &params).map(|_| ())
        })
    }

    pub fn register_client_session(
        &self,
        host_name: &str,
        mountpoint: &str,
        mount_mode: &str,
        lock_backend: &str,
        pid: u64,
        lease_ttl_seconds: u64,
    ) -> Result<u64, String> {
        let request_token_value = generate_request_token("client-session");
        let request_token = CString::new(request_token_value)
            .map_err(|_| "request token contains NUL byte".to_string())?;
        let sql_lookup_request_token = CString::new(
            "
            SELECT COALESCE((SELECT session_id::text FROM client_sessions WHERE request_token = $1 LIMIT 1), '')
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql = CString::new(
            "
            INSERT INTO client_sessions (
                host_name,
                mountpoint,
                mount_mode,
                lock_backend,
                pid,
                request_token,
                lease_expires_at,
                heartbeat_at,
                started_at,
                updated_at
            ) VALUES (
                $1,
                $2,
                $3,
                $4,
                $5,
                $6,
                NOW() + ($7 || ' seconds')::interval,
                NOW(),
                NOW(),
                NOW()
            )
            ON CONFLICT (request_token) DO UPDATE SET
                updated_at = NOW()
            RETURNING session_id
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let host_name =
            CString::new(host_name).map_err(|_| "host name contains NUL byte".to_string())?;
        let mountpoint =
            CString::new(mountpoint).map_err(|_| "mountpoint contains NUL byte".to_string())?;
        let mount_mode =
            CString::new(mount_mode).map_err(|_| "mount mode contains NUL byte".to_string())?;
        let lock_backend =
            CString::new(lock_backend).map_err(|_| "lock backend contains NUL byte".to_string())?;
        let pid = CString::new(pid.to_string()).map_err(|_| "pid contains NUL byte".to_string())?;
        let lease_ttl_seconds = CString::new(lease_ttl_seconds.to_string())
            .map_err(|_| "lease ttl contains NUL byte".to_string())?;

        self.with_control_connection(|conn| unsafe {
            transactional_replay_confirmed(
                conn,
                |conn| {
                    let request_token_params = [&request_token];
                    let existing_token_res =
                        exec_params(conn, &sql_lookup_request_token, &request_token_params)?;
                    let existing_token = fetch_single_text_option(existing_token_res)?;
                    match existing_token {
                        Some(session_id_text) => {
                            let session_id = session_id_text
                                .parse::<u64>()
                                .map_err(|_| "invalid client session id value".to_string())?;
                            if session_id > i64::MAX as u64 {
                                return Err("invalid client session id value".to_string());
                            }
                            self.set_lock_session_id(session_id as i64)?;
                            Ok(Some(session_id))
                        }
                        None => Ok(None),
                    }
                },
                |conn| {
                    let params = [
                        &host_name,
                        &mountpoint,
                        &mount_mode,
                        &lock_backend,
                        &pid,
                        &request_token,
                        &lease_ttl_seconds,
                    ];
                    let res = exec_params(conn, &sql, &params)?;
                    let session_id = fetch_single_text(res)?;
                    if session_id.trim().is_empty() {
                        return Err("failed to create client session".to_string());
                    }
                    let session_id = session_id
                        .trim()
                        .parse::<u64>()
                        .map_err(|_| "invalid client session id value".to_string())?;
                    if session_id > i64::MAX as u64 {
                        return Err("invalid client session id value".to_string());
                    }
                    self.set_lock_session_id(session_id as i64)?;
                    Ok(session_id)
                },
            )
        })
    }

    pub fn heartbeat_client_session(
        &self,
        session_id: u64,
        lease_ttl_seconds: u64,
    ) -> Result<(), String> {
        let sql = CString::new(
            "
            UPDATE client_sessions
            SET lease_expires_at = NOW() + ($2 || ' seconds')::interval,
                heartbeat_at = NOW(),
                updated_at = NOW()
            WHERE session_id = $1
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let session_id = CString::new(session_id.to_string())
            .map_err(|_| "session id contains NUL byte".to_string())?;
        let lease_ttl_seconds = CString::new(lease_ttl_seconds.to_string())
            .map_err(|_| "lease ttl contains NUL byte".to_string())?;

        self.with_control_connection(|conn| unsafe {
            let params = [&session_id, &lease_ttl_seconds];
            exec_command_params(conn, &sql, &params).map(|_| ())
        })
    }

    pub fn touch_client_session_owner_key(
        &self,
        session_id: u64,
        owner_key: u64,
    ) -> Result<(), String> {
        let session_id_value = i64::try_from(session_id).unwrap_or(0);
        let owner_key_value = owner_key;
        let probe_sql = CString::new(
            "
            SELECT COALESCE((SELECT owner_key::text FROM client_session_owner_keys WHERE session_id = $1 AND owner_key = $2 LIMIT 1), '')
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        self.with_control_connection(|conn| unsafe {
            transactional_replay_confirmed(
                conn,
                |conn| {
                    let session_id = CString::new(session_id.to_string())
                        .map_err(|_| "session id contains NUL byte".to_string())?;
                    let owner_key = CString::new(owner_key_value.to_string())
                        .map_err(|_| "owner key contains NUL byte".to_string())?;
                    let params = [&session_id, &owner_key];
                    let existing_res = exec_params(conn, &probe_sql, &params)?;
                    match fetch_single_text_option(existing_res)? {
                        Some(_) => {
                            if let Ok(mut guard) = self.owner_session_cache.lock() {
                                guard.insert(owner_key_value, session_id_value);
                            }
                            Ok(Some(()))
                        }
                        None => Ok(None),
                    }
                },
                |conn| {
                    let insert_sql = CString::new(
                        "
                        INSERT INTO client_session_owner_keys (
                            session_id,
                            owner_key,
                            first_seen_at,
                            last_seen_at,
                            updated_at
                        ) VALUES (
                            $1,
                            $2,
                            NOW(),
                            NOW(),
                            NOW()
                        )
                        ON CONFLICT (session_id, owner_key)
                        DO UPDATE SET
                            last_seen_at = NOW(),
                            updated_at = NOW()
                        ",
                    )
                    .map_err(|_| "SQL contains NUL byte".to_string())?;
                    let touch_sql = CString::new(
                        "
                        UPDATE client_sessions
                        SET last_lock_at = NOW(),
                            updated_at = NOW()
                        WHERE session_id = $1
                        ",
                    )
                    .map_err(|_| "SQL contains NUL byte".to_string())?;
                    let session_id = CString::new(session_id.to_string())
                        .map_err(|_| "session id contains NUL byte".to_string())?;
                    let owner_key = CString::new(owner_key_value.to_string())
                        .map_err(|_| "owner key contains NUL byte".to_string())?;
                    let params = [&session_id, &owner_key];
                    exec_command_params(conn, &insert_sql, &params)?;
                    let touch_params = [&session_id];
                    exec_command_params(conn, &touch_sql, &touch_params)?;
                    if let Ok(mut guard) = self.owner_session_cache.lock() {
                        guard.insert(owner_key_value, session_id_value);
                    }
                    Ok(())
                },
            )
        })
    }

    pub fn publish_monitor_session_stats(
        &self,
        session_id: u64,
        fod_version: &str,
        sample_seq: u64,
        payload_json: &str,
    ) -> Result<(), String> {
        let sql = CString::new(
            "
            INSERT INTO monitor_session_stats (
                session_id, fod_version, sample_seq, sampled_at, payload_json, updated_at
            )
            SELECT session_id, $2, $3, NOW(), $4::jsonb, NOW()
            FROM client_sessions
            WHERE session_id = $1
            ON CONFLICT (session_id) DO UPDATE SET
                fod_version = EXCLUDED.fod_version,
                sample_seq = EXCLUDED.sample_seq,
                sampled_at = EXCLUDED.sampled_at,
                payload_json = EXCLUDED.payload_json,
                updated_at = NOW()
            WHERE monitor_session_stats.sample_seq <= EXCLUDED.sample_seq
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let session_id = CString::new(session_id.to_string())
            .map_err(|_| "session id contains NUL byte".to_string())?;
        let fod_version =
            CString::new(fod_version).map_err(|_| "FOD version contains NUL byte".to_string())?;
        let sample_seq = CString::new(sample_seq.to_string())
            .map_err(|_| "sample sequence contains NUL byte".to_string())?;
        let payload_json = CString::new(payload_json)
            .map_err(|_| "monitor payload contains NUL byte".to_string())?;
        self.with_control_connection(|conn| unsafe {
            let params = [&session_id, &fod_version, &sample_seq, &payload_json];
            exec_command_params(conn, &sql, &params).map(|_| ())
        })
    }

    pub fn touch_client_session_write(&self, session_id: u64) -> Result<(), String> {
        let sql = CString::new(
            "
            UPDATE client_sessions
            SET last_write_at = NOW(),
                updated_at = NOW()
            WHERE session_id = $1
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let session_id = CString::new(session_id.to_string())
            .map_err(|_| "session id contains NUL byte".to_string())?;

        self.with_control_connection(|conn| unsafe {
            let params = [&session_id];
            exec_command_params(conn, &sql, &params).map(|_| ())
        })
    }

    pub fn prune_expired_client_sessions(&self) -> Result<bool, String> {
        let delete_sql = CString::new(
            "
            DELETE FROM client_sessions
            WHERE lease_expires_at <= NOW()
            RETURNING session_id
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let cleanup_zero_session_sql = CString::new(
            "
            DELETE FROM lock_leases
            WHERE session_id = 0
              AND lease_expires_at <= NOW();
            DELETE FROM lock_range_leases
            WHERE session_id = 0
              AND lease_expires_at <= NOW()
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_control_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let empty_params: [&CString; 0] = [];
                let res = exec_params(conn, &delete_sql, &empty_params)?;
                let expired = match PQresultStatus(res) {
                    PGRES_TUPLES_OK => PQntuples(res) > 0,
                    _ => {
                        let error = result_error(res);
                        PQclear(res);
                        return Err(error);
                    }
                };
                PQclear(res);
                exec_command(conn, &cleanup_zero_session_sql)?;
                Ok(expired)
            })
        })
    }

    unsafe fn try_advisory_xact_lock_on_conn(
        conn: *mut PGconn,
        resource_lock_id: i64,
    ) -> Result<bool, String> {
        let sql = format!("SELECT pg_try_advisory_xact_lock({resource_lock_id})");
        let sql = CString::new(sql).map_err(|_| "SQL contains NUL byte".to_string())?;
        let empty_params: [&CString; 0] = [];
        let res = exec_params(conn, &sql, &empty_params)?;
        let value = fetch_single_text(res)?;
        Ok(matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        ))
    }

    pub fn try_advisory_xact_lock(&self, resource_lock_id: i64) -> Result<bool, String> {
        self.with_control_connection(|conn| unsafe {
            Self::try_advisory_xact_lock_on_conn(conn, resource_lock_id)
        })
    }

    pub fn acquire_flock_lease(
        &self,
        resource_kind: &str,
        resource_id: u64,
        owner_key: u64,
        requested_type: i32,
        lease_ttl_seconds: u64,
        resource_lock_id: i64,
    ) -> Result<bool, String> {
        self.ensure_lock_schema_ready()?;
        let session_id = self.session_id_for_owner_key_text(owner_key)?;
        let request_token_value = generate_request_token("flock-lease");
        let request_token = CString::new(request_token_value)
            .map_err(|_| "request token contains NUL byte".to_string())?;
        let sql_lookup_request_token = CString::new(
            "SELECT COALESCE((SELECT did_grant::text FROM lock_lease_request_tokens WHERE request_token = $1 LIMIT 1), '')",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_store_request_token = CString::new(
            "
            INSERT INTO lock_lease_request_tokens (
                request_token,
                did_grant,
                created_at,
                updated_at
            ) VALUES (
                $1,
                $2,
                NOW(),
                NOW()
            )
            ON CONFLICT (request_token) DO UPDATE SET
                did_grant = EXCLUDED.did_grant,
                updated_at = NOW()
            RETURNING did_grant
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let resource_kind = CString::new(resource_kind)
            .map_err(|_| "resource kind contains NUL byte".to_string())?;
        let resource_id = CString::new(resource_id.to_string())
            .map_err(|_| "resource id contains NUL byte".to_string())?;
        let lease_kind =
            CString::new("flock").map_err(|_| "lease kind contains NUL byte".to_string())?;
        let requested_type_value = requested_type;
        let requested_type = CString::new(requested_type_value.to_string())
            .map_err(|_| "lock type contains NUL byte".to_string())?;
        let lease_ttl_seconds = CString::new(lease_ttl_seconds.to_string())
            .map_err(|_| "lease ttl contains NUL byte".to_string())?;
        let owner_key = CString::new(owner_key.to_string())
            .map_err(|_| "owner key contains NUL byte".to_string())?;

        self.with_control_connection(|conn| unsafe {
            transactional_replay_confirmed(
                conn,
                |conn| {
                    let request_token_params = [&request_token];
                    let existing_token_res =
                        exec_params(conn, &sql_lookup_request_token, &request_token_params)?;
                    let existing_token = fetch_single_text_option(existing_token_res)?;
                    match existing_token {
                        Some(existing) => Ok(Some(matches!(
                            existing.trim().to_ascii_lowercase().as_str(),
                            "t" | "true" | "1" | "on"
                        ))),
                        None => Ok(None),
                    }
                },
                |conn| {
                    let lock_granted =
                        Self::try_advisory_xact_lock_on_conn(conn, resource_lock_id)?;
                    if !lock_granted {
                        let did_grant = CString::new("false")
                            .map_err(|_| "lock lease outcome contains NUL byte".to_string())?;
                        let params = [&request_token, &did_grant];
                        let res = exec_params(conn, &sql_store_request_token, &params)?;
                        let stored = fetch_single_text(res)?;
                        if !matches!(
                            stored.trim().to_ascii_lowercase().as_str(),
                            "f" | "false" | "0" | "off"
                        ) {
                            return Err(
                                "lock lease request token stored unexpected result".to_string()
                            );
                        }
                        return Ok(false);
                    }

                    let prune_sql = CString::new(
                        "
                        DELETE FROM lock_leases
                        WHERE resource_kind = $1
                          AND resource_id = $2
                          AND lease_kind = $3
                          AND lease_expires_at <= NOW()
                        ",
                    )
                    .map_err(|_| "SQL contains NUL byte".to_string())?;
                    let prune_params = [&resource_kind, &resource_id, &lease_kind];
                    exec_command_params(conn, &prune_sql, &prune_params)?;

                    let conflict_sql = CString::new(
                        "
                        SELECT lock_type
                        FROM lock_leases
                        WHERE resource_kind = $1
                          AND resource_id = $2
                          AND lease_kind = $3
                          AND lease_expires_at > NOW()
                          AND NOT (session_id = $4 AND owner_key = $5)
                        ORDER BY session_id, owner_key
                        LIMIT 1
                        ",
                    )
                    .map_err(|_| "SQL contains NUL byte".to_string())?;
                    let conflict_params = [
                        &resource_kind,
                        &resource_id,
                        &lease_kind,
                        &session_id,
                        &owner_key,
                    ];
                    let res = exec_params(conn, &conflict_sql, &conflict_params)?;
                    let conflict = fetch_single_text(res)?;
                    if !conflict.trim().is_empty() {
                        let other_type = conflict.trim().parse::<i32>().unwrap_or(0);
                        let blocked = match requested_type_value {
                            1 => other_type == 2,
                            2 => other_type == 1 || other_type == 2,
                            _ => false,
                        };
                        if blocked {
                            let did_grant = CString::new("false")
                                .map_err(|_| "lock lease outcome contains NUL byte".to_string())?;
                            let params = [&request_token, &did_grant];
                            let res = exec_params(conn, &sql_store_request_token, &params)?;
                            let stored = fetch_single_text(res)?;
                            if !matches!(
                                stored.trim().to_ascii_lowercase().as_str(),
                                "f" | "false" | "0" | "off"
                            ) {
                                return Err(
                                    "lock lease request token stored unexpected result".to_string()
                                );
                            }
                            return Ok(false);
                        }
                    }

                    let upsert_sql = CString::new(
                        "
                        INSERT INTO lock_leases (
                            resource_kind,
                            resource_id,
                            session_id,
                            owner_key,
                            lease_kind,
                            lock_type,
                            request_token,
                            lease_expires_at,
                            heartbeat_at,
                            created_at,
                            updated_at
                        ) VALUES (
                            $1,
                            $2,
                            $3,
                            $4,
                            $5,
                            $6,
                            $7,
                            NOW() + ($8 || ' seconds')::interval,
                            NOW(),
                            NOW(),
                            NOW()
                        )
                        ON CONFLICT (resource_kind, resource_id, session_id, owner_key, lease_kind)
                        DO UPDATE SET
                            lock_type = EXCLUDED.lock_type,
                            request_token = EXCLUDED.request_token,
                            lease_expires_at = EXCLUDED.lease_expires_at,
                            heartbeat_at = EXCLUDED.heartbeat_at,
                            updated_at = NOW()
                        ",
                    )
                    .map_err(|_| "SQL contains NUL byte".to_string())?;
                    let upsert_params = [
                        &resource_kind,
                        &resource_id,
                        &session_id,
                        &owner_key,
                        &lease_kind,
                        &requested_type,
                        &request_token,
                        &lease_ttl_seconds,
                    ];
                    exec_command_params(conn, &upsert_sql, &upsert_params)?;
                    let did_grant = CString::new("true")
                        .map_err(|_| "lock lease outcome contains NUL byte".to_string())?;
                    let params = [&request_token, &did_grant];
                    let res = exec_params(conn, &sql_store_request_token, &params)?;
                    let stored = fetch_single_text(res)?;
                    if !matches!(
                        stored.trim().to_ascii_lowercase().as_str(),
                        "t" | "true" | "1" | "on"
                    ) {
                        return Err("lock lease request token stored unexpected result".to_string());
                    }
                    Ok(true)
                },
            )
        })
    }

    pub fn release_flock_lease(
        &self,
        resource_kind: &str,
        resource_id: u64,
        owner_key: u64,
    ) -> Result<(), String> {
        self.ensure_lock_schema_ready()?;
        self.delete_lock_lease(resource_kind, resource_id, owner_key)?;
        self.delete_range_leases(resource_kind, resource_id, Some(owner_key))?;
        Ok(())
    }

    pub fn heartbeat_lock_lease(
        &self,
        resource_kind: &str,
        resource_id: u64,
        owner_key: u64,
        lease_ttl_seconds: u64,
    ) -> Result<(), String> {
        self.ensure_lock_schema_ready()?;
        let session_id = self.session_id_for_owner_key_text(owner_key)?;
        let sql = CString::new(
            "
            UPDATE lock_leases
            SET lease_expires_at = NOW() + ($5 || ' seconds')::interval,
                heartbeat_at = NOW(),
                updated_at = NOW()
            WHERE resource_kind = $1
              AND resource_id = $2
              AND session_id = $3
              AND owner_key = $4
              AND lease_kind = $6
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let resource_kind = CString::new(resource_kind)
            .map_err(|_| "resource kind contains NUL byte".to_string())?;
        let resource_id = CString::new(resource_id.to_string())
            .map_err(|_| "resource id contains NUL byte".to_string())?;
        let owner_key = CString::new(owner_key.to_string())
            .map_err(|_| "owner key contains NUL byte".to_string())?;
        let lease_ttl_seconds = CString::new(lease_ttl_seconds.to_string())
            .map_err(|_| "lease ttl contains NUL byte".to_string())?;
        let lease_kind =
            CString::new("flock").map_err(|_| "lease kind contains NUL byte".to_string())?;

        self.with_control_connection(|conn| unsafe {
            let params = [
                &resource_kind,
                &resource_id,
                &session_id,
                &owner_key,
                &lease_ttl_seconds,
                &lease_kind,
            ];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn heartbeat_lock_range_lease(
        &self,
        resource_kind: &str,
        resource_id: u64,
        owner_key: u64,
        range_start: u64,
        range_end: Option<u64>,
        lease_ttl_seconds: u64,
    ) -> Result<(), String> {
        self.ensure_lock_schema_ready()?;
        let session_id = self.session_id_for_owner_key_text(owner_key)?;
        let sql = CString::new(if range_end.is_some() {
            "
                UPDATE lock_range_leases
                SET lease_expires_at = NOW() + ($7 || ' seconds')::interval,
                    heartbeat_at = NOW(),
                    updated_at = NOW()
                WHERE resource_kind = $1
                  AND resource_id = $2
                  AND session_id = $3
                  AND owner_key = $4
                  AND range_start = $5
                  AND range_end = $6
                "
        } else {
            "
                UPDATE lock_range_leases
                SET lease_expires_at = NOW() + ($6 || ' seconds')::interval,
                    heartbeat_at = NOW(),
                    updated_at = NOW()
                WHERE resource_kind = $1
                  AND resource_id = $2
                  AND session_id = $3
                  AND owner_key = $4
                  AND range_start = $5
                  AND range_end IS NULL
                "
        })
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let resource_kind = CString::new(resource_kind)
            .map_err(|_| "resource kind contains NUL byte".to_string())?;
        let resource_id = CString::new(resource_id.to_string())
            .map_err(|_| "resource id contains NUL byte".to_string())?;
        let owner_key = CString::new(owner_key.to_string())
            .map_err(|_| "owner key contains NUL byte".to_string())?;
        let range_start = CString::new(range_start.to_string())
            .map_err(|_| "range start contains NUL byte".to_string())?;
        let lease_ttl_seconds = CString::new(lease_ttl_seconds.to_string())
            .map_err(|_| "lease ttl contains NUL byte".to_string())?;

        self.with_control_connection(|conn| unsafe {
            if let Some(range_end) = range_end {
                let range_end = CString::new(range_end.to_string())
                    .map_err(|_| "range end contains NUL byte".to_string())?;
                let params = [
                    &resource_kind,
                    &resource_id,
                    &session_id,
                    &owner_key,
                    &range_start,
                    &range_end,
                    &lease_ttl_seconds,
                ];
                exec_command_params(conn, &sql, &params)
            } else {
                let params = [
                    &resource_kind,
                    &resource_id,
                    &session_id,
                    &owner_key,
                    &range_start,
                    &lease_ttl_seconds,
                ];
                exec_command_params(conn, &sql, &params)
            }
        })
    }

    pub fn load_lock_range_state_blob(
        &self,
        resource_kind: &str,
        resource_id: u64,
    ) -> Result<Vec<u8>, String> {
        self.ensure_lock_schema_ready()?;
        let sql = CString::new(
            "
            SELECT owner_key, lock_type, range_start, range_end
            FROM lock_range_leases
            WHERE resource_kind = $1
              AND resource_id = $2
              AND lease_expires_at > NOW()
            ORDER BY owner_key, range_start, COALESCE(range_end, 9223372036854775807)
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let resource_kind = CString::new(resource_kind)
            .map_err(|_| "resource kind contains NUL byte".to_string())?;
        let resource_id = CString::new(resource_id.to_string())
            .map_err(|_| "resource id contains NUL byte".to_string())?;
        self.with_control_connection(|conn| unsafe {
            let params = [&resource_kind, &resource_id];
            let res = exec_params(conn, &sql, &params)?;
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            let mut output = Vec::new();
            if rows >= 1 && cols >= 4 {
                for row in 0..rows {
                    if row > 0 {
                        output.push(b'\n');
                    }
                    let owner_ptr = PQgetvalue(res, row, 0);
                    let type_ptr = PQgetvalue(res, row, 1);
                    let start_ptr = PQgetvalue(res, row, 2);
                    let end_ptr = PQgetvalue(res, row, 3);
                    let owner = if owner_ptr.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr(owner_ptr).to_string_lossy().to_string()
                    };
                    let lock_type = if type_ptr.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr(type_ptr).to_string_lossy().to_string()
                    };
                    let start = if start_ptr.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr(start_ptr).to_string_lossy().to_string()
                    };
                    let end = if end_ptr.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr(end_ptr).to_string_lossy().to_string()
                    };
                    output.extend_from_slice(owner.as_bytes());
                    output.push(b'\t');
                    output.extend_from_slice(lock_type.as_bytes());
                    output.push(b'\t');
                    output.extend_from_slice(start.as_bytes());
                    output.push(b'\t');
                    output.extend_from_slice(end.as_bytes());
                }
            }
            PQclear(res);
            Ok(output)
        })
    }

    pub fn persist_lock_range_state_blob(
        &self,
        resource_kind: &str,
        resource_id: u64,
        lease_ttl_seconds: u64,
        payload: &str,
    ) -> Result<(), String> {
        self.ensure_lock_schema_ready()?;
        let session_id = self.current_lock_session_id_text()?;
        self.with_control_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let delete_sql = CString::new(
                    "
                    DELETE FROM lock_range_leases
                    WHERE resource_kind = $1
                      AND resource_id = $2
                      AND session_id = $3
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let resource_kind_sql = CString::new(resource_kind)
                    .map_err(|_| "resource kind contains NUL byte".to_string())?;
                let resource_id_sql = CString::new(resource_id.to_string())
                    .map_err(|_| "resource id contains NUL byte".to_string())?;
                let delete_params = [&resource_kind_sql, &resource_id_sql, &session_id];
                exec_command_params(conn, &delete_sql, &delete_params)?;
                if payload.trim().is_empty() {
                    return Ok(());
                }
                let insert_sql = CString::new(
                    "
                    INSERT INTO lock_range_leases (
                        resource_kind,
                        resource_id,
                        session_id,
                        owner_key,
                        lock_type,
                        range_start,
                        range_end,
                        lease_expires_at,
                        heartbeat_at,
                        created_at,
                        updated_at
                    ) VALUES (
                        $1,
                        $2,
                        $3,
                        $4,
                        $5,
                        $6,
                        $7,
                        NOW() + ($8 || ' seconds')::interval,
                        NOW(),
                        NOW(),
                        NOW()
                    )
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let insert_sql_null_end = CString::new(
                    "
                    INSERT INTO lock_range_leases (
                        resource_kind,
                        resource_id,
                        session_id,
                        owner_key,
                        lock_type,
                        range_start,
                        range_end,
                        lease_expires_at,
                        heartbeat_at,
                        created_at,
                        updated_at
                    ) VALUES (
                        $1,
                        $2,
                        $3,
                        $4,
                        $5,
                        $6,
                        NULL,
                        NOW() + ($7 || ' seconds')::interval,
                        NOW(),
                        NOW(),
                        NOW()
                    )
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let resource_kind_sql = CString::new(resource_kind)
                    .map_err(|_| "resource kind contains NUL byte".to_string())?;
                let resource_id_sql = CString::new(resource_id.to_string())
                    .map_err(|_| "resource id contains NUL byte".to_string())?;
                let ttl_sql = CString::new(lease_ttl_seconds.to_string())
                    .map_err(|_| "lease ttl contains NUL byte".to_string())?;

                for line in payload.lines() {
                    let mut parts = line.split('\t');
                    let owner_key_text = parts
                        .next()
                        .ok_or_else(|| "invalid range state line".to_string())?;
                    let lock_type = parts
                        .next()
                        .ok_or_else(|| "invalid range state line".to_string())?;
                    let range_start = parts
                        .next()
                        .ok_or_else(|| "invalid range state line".to_string())?;
                    let range_end = parts
                        .next()
                        .ok_or_else(|| "invalid range state line".to_string())?;
                    let owner_key_value = owner_key_text
                        .parse::<u64>()
                        .map_err(|_| "owner key contains invalid value".to_string())?;
                    let owner_key = CString::new(owner_key_text)
                        .map_err(|_| "owner key contains NUL byte".to_string())?;
                    let lock_type = CString::new(lock_type)
                        .map_err(|_| "lock type contains NUL byte".to_string())?;
                    let range_start = CString::new(range_start)
                        .map_err(|_| "range start contains NUL byte".to_string())?;
                    let line_session_id = self.session_id_for_owner_key_text(owner_key_value)?;
                    let range_end = if range_end.is_empty() {
                        None
                    } else {
                        Some(
                            CString::new(range_end)
                                .map_err(|_| "range end contains NUL byte".to_string())?,
                        )
                    };
                    if parts.next().is_some() {
                        return Err("invalid range state line".to_string());
                    }
                    if let Some(range_end) = range_end.as_ref() {
                        let params = [
                            &resource_kind_sql,
                            &resource_id_sql,
                            &line_session_id,
                            &owner_key,
                            &lock_type,
                            &range_start,
                            range_end,
                            &ttl_sql,
                        ];
                        exec_command_params(conn, &insert_sql, &params)?;
                    } else {
                        let params = [
                            &resource_kind_sql,
                            &resource_id_sql,
                            &line_session_id,
                            &owner_key,
                            &lock_type,
                            &range_start,
                            &ttl_sql,
                        ];
                        exec_command_params(conn, &insert_sql_null_end, &params)?;
                    }
                }
                Ok(())
            })
        })
    }

    pub fn replace_lock_range_state_blob_for_owner(
        &self,
        resource_kind: &str,
        resource_id: u64,
        owner_key: u64,
        lease_ttl_seconds: u64,
        payload: &str,
    ) -> Result<(), String> {
        self.ensure_lock_schema_ready()?;
        self.with_control_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let delete_sql = CString::new(
                    "
                    DELETE FROM lock_range_leases
                    WHERE resource_kind = $1
                      AND resource_id = $2
                      AND session_id = $3
                      AND owner_key = $4
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let resource_kind_sql = CString::new(resource_kind)
                    .map_err(|_| "resource kind contains NUL byte".to_string())?;
                let resource_id_sql = CString::new(resource_id.to_string())
                    .map_err(|_| "resource id contains NUL byte".to_string())?;
                let owner_key_text = owner_key.to_string();
                let owner_key_sql = CString::new(owner_key_text.clone())
                    .map_err(|_| "owner key contains NUL byte".to_string())?;
                let line_session_id = self.session_id_for_owner_key_text(owner_key)?;
                let delete_params = [
                    &resource_kind_sql,
                    &resource_id_sql,
                    &line_session_id,
                    &owner_key_sql,
                ];
                exec_command_params(conn, &delete_sql, &delete_params)?;
                if payload.trim().is_empty() {
                    return Ok(());
                }
                let insert_sql = CString::new(
                    "
                    INSERT INTO lock_range_leases (
                        resource_kind,
                        resource_id,
                        session_id,
                        owner_key,
                        lock_type,
                        range_start,
                        range_end,
                        lease_expires_at,
                        heartbeat_at,
                        created_at,
                        updated_at
                    ) VALUES (
                        $1,
                        $2,
                        $3,
                        $4,
                        $5,
                        $6,
                        $7,
                        NOW() + ($8 || ' seconds')::interval,
                        NOW(),
                        NOW(),
                        NOW()
                    )
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let insert_sql_null_end = CString::new(
                    "
                    INSERT INTO lock_range_leases (
                        resource_kind,
                        resource_id,
                        session_id,
                        owner_key,
                        lock_type,
                        range_start,
                        range_end,
                        lease_expires_at,
                        heartbeat_at,
                        created_at,
                        updated_at
                    ) VALUES (
                        $1,
                        $2,
                        $3,
                        $4,
                        $5,
                        $6,
                        NULL,
                        NOW() + ($7 || ' seconds')::interval,
                        NOW(),
                        NOW(),
                        NOW()
                    )
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let resource_kind_sql = CString::new(resource_kind)
                    .map_err(|_| "resource kind contains NUL byte".to_string())?;
                let resource_id_sql = CString::new(resource_id.to_string())
                    .map_err(|_| "resource id contains NUL byte".to_string())?;
                let owner_key_text = owner_key.to_string();
                let owner_key_sql = CString::new(owner_key_text.clone())
                    .map_err(|_| "owner key contains NUL byte".to_string())?;
                let ttl_sql = CString::new(lease_ttl_seconds.to_string())
                    .map_err(|_| "lease ttl contains NUL byte".to_string())?;

                for line in payload.lines() {
                    let mut parts = line.split('\t');
                    let payload_owner = parts
                        .next()
                        .ok_or_else(|| "invalid range state line".to_string())?;
                    let lock_type = parts
                        .next()
                        .ok_or_else(|| "invalid range state line".to_string())?;
                    let range_start = parts
                        .next()
                        .ok_or_else(|| "invalid range state line".to_string())?;
                    let range_end = parts
                        .next()
                        .ok_or_else(|| "invalid range state line".to_string())?;
                    if parts.next().is_some() {
                        return Err("invalid range state line".to_string());
                    }
                    if payload_owner != owner_key_text {
                        return Err("owner mismatch in range state payload".to_string());
                    }
                    let lock_type = CString::new(lock_type)
                        .map_err(|_| "lock type contains NUL byte".to_string())?;
                    let range_start = CString::new(range_start)
                        .map_err(|_| "range start contains NUL byte".to_string())?;
                    let line_session_id = self.session_id_for_owner_key_text(owner_key)?;
                    if range_end.is_empty() {
                        let params = [
                            &resource_kind_sql,
                            &resource_id_sql,
                            &line_session_id,
                            &owner_key_sql,
                            &lock_type,
                            &range_start,
                            &ttl_sql,
                        ];
                        exec_command_params(conn, &insert_sql_null_end, &params)?;
                    } else {
                        let range_end = CString::new(range_end)
                            .map_err(|_| "range end contains NUL byte".to_string())?;
                        let params = [
                            &resource_kind_sql,
                            &resource_id_sql,
                            &line_session_id,
                            &owner_key_sql,
                            &lock_type,
                            &range_start,
                            &range_end,
                            &ttl_sql,
                        ];
                        exec_command_params(conn, &insert_sql, &params)?;
                    }
                }
                Ok(())
            })
        })
    }

    pub fn get_dir_id(&self, path: &str) -> Result<Option<u64>, String> {
        let path = CString::new(path).map_err(|_| "path contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let params = [&path];
                let res = exec_prepared_params(conn, PreparedStatement::GetDirId, &params)?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 1 {
                                None
                            } else {
                                let value_ptr = PQgetvalue(res, 0, 0);
                                if value_ptr.is_null() {
                                    None
                                } else {
                                    let value =
                                        CStr::from_ptr(value_ptr).to_string_lossy().to_string();
                                    value.trim().parse::<u64>().ok()
                                }
                            };
                            PQclear(res);
                            Ok(value)
                        }
                        _ => {
                            PQclear(res);
                            Err(conn_error(conn))
                        }
                    }
                }
            };
            result
        })
    }

    pub fn get_file_id(&self, path: &str) -> Result<Option<u64>, String> {
        let normalized = path.trim();
        let (parent_path, file_name) = match normalized.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => {
                (if parent.is_empty() { "/" } else { parent }, name)
            }
            _ => ("/", normalized),
        };
        let parent_id = self.get_dir_id(parent_path)?;
        let file_name =
            CString::new(file_name).map_err(|_| "path contains NUL byte".to_string())?;
        let parent_id_text = parent_id
            .map(|value| {
                CString::new(value.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())
            })
            .transpose()?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let res = if let Some(ref parent_id_text) = parent_id_text {
                    let params = [&file_name, parent_id_text];
                    exec_prepared_params(conn, PreparedStatement::GetFileIdNested, &params)
                } else {
                    let params = [&file_name];
                    exec_prepared_params(conn, PreparedStatement::GetFileIdRoot, &params)
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 1 {
                                None
                            } else {
                                let value_ptr = PQgetvalue(res, 0, 0);
                                if value_ptr.is_null() {
                                    None
                                } else {
                                    let value =
                                        CStr::from_ptr(value_ptr).to_string_lossy().to_string();
                                    value.trim().parse::<u64>().ok()
                                }
                            };
                            PQclear(res);
                            Ok(value)
                        }
                        _ => {
                            PQclear(res);
                            Err(conn_error(conn))
                        }
                    }
                }
            };
            result
        })
    }

    pub fn get_file_mode_value(&self, path: &str) -> Result<Option<String>, String> {
        let normalized = path.trim();
        let (parent_path, file_name) = match normalized.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => {
                (if parent.is_empty() { "/" } else { parent }, name)
            }
            _ => ("/", normalized),
        };
        let parent_id = self.get_dir_id(parent_path)?;
        let file_name =
            CString::new(file_name).map_err(|_| "path contains NUL byte".to_string())?;
        let parent_id_text = parent_id
            .map(|value| {
                CString::new(value.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())
            })
            .transpose()?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let res = if let Some(ref parent_id_text) = parent_id_text {
                    let params = [&file_name, parent_id_text];
                    exec_prepared_params(conn, PreparedStatement::GetFileModeNested, &params)
                } else {
                    let params = [&file_name];
                    exec_prepared_params(conn, PreparedStatement::GetFileModeRoot, &params)
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 1 {
                                None
                            } else {
                                let value_ptr = PQgetvalue(res, 0, 0);
                                if value_ptr.is_null() {
                                    None
                                } else {
                                    Some(CStr::from_ptr(value_ptr).to_string_lossy().to_string())
                                }
                            };
                            PQclear(res);
                            Ok(value)
                        }
                        _ => {
                            PQclear(res);
                            Err(conn_error(conn))
                        }
                    }
                }
            };
            result
        })
    }

    pub fn get_hardlink_id(&self, path: &str) -> Result<Option<u64>, String> {
        let normalized = path.trim();
        let (parent_path, link_name) = match normalized.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => {
                (if parent.is_empty() { "/" } else { parent }, name)
            }
            _ => ("/", normalized),
        };
        let parent_id = self.get_dir_id(parent_path)?;
        let link_name =
            CString::new(link_name).map_err(|_| "path contains NUL byte".to_string())?;
        let parent_id_text = parent_id
            .map(|value| {
                CString::new(value.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())
            })
            .transpose()?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let res = if let Some(ref parent_id_text) = parent_id_text {
                    let params = [&link_name, parent_id_text];
                    exec_prepared_params(conn, PreparedStatement::GetHardlinkIdNested, &params)
                } else {
                    let params = [&link_name];
                    exec_prepared_params(conn, PreparedStatement::GetHardlinkIdRoot, &params)
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 1 {
                                None
                            } else {
                                let value_ptr = PQgetvalue(res, 0, 0);
                                if value_ptr.is_null() {
                                    None
                                } else {
                                    let value =
                                        CStr::from_ptr(value_ptr).to_string_lossy().to_string();
                                    value.trim().parse::<u64>().ok()
                                }
                            };
                            PQclear(res);
                            Ok(value)
                        }
                        _ => {
                            PQclear(res);
                            Err(conn_error(conn))
                        }
                    }
                }
            };
            result
        })
    }

    pub fn get_hardlink_file_id(&self, hardlink_id: u64) -> Result<Option<u64>, String> {
        let hardlink_id = CString::new(hardlink_id.to_string())
            .map_err(|_| "hardlink id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let params = [&hardlink_id];
                let res =
                    exec_prepared_params(conn, PreparedStatement::GetHardlinkFileId, &params)?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 1 {
                                None
                            } else {
                                let value_ptr = PQgetvalue(res, 0, 0);
                                if value_ptr.is_null() {
                                    None
                                } else {
                                    let value =
                                        CStr::from_ptr(value_ptr).to_string_lossy().to_string();
                                    value.trim().parse::<u64>().ok()
                                }
                            };
                            PQclear(res);
                            Ok(value)
                        }
                        _ => {
                            PQclear(res);
                            Err(conn_error(conn))
                        }
                    }
                }
            };
            result
        })
    }

    pub fn choose_primary_hardlink(
        &self,
        file_id: u64,
    ) -> Result<Option<(u64, Option<u64>, String)>, String> {
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let params = [&file_id];
                let res =
                    exec_prepared_params(conn, PreparedStatement::ChoosePrimaryHardlink, &params)?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 3 {
                                None
                            } else {
                                let hardlink_ptr = PQgetvalue(res, 0, 0);
                                let parent_ptr = PQgetvalue(res, 0, 1);
                                let name_ptr = PQgetvalue(res, 0, 2);
                                if hardlink_ptr.is_null()
                                    || parent_ptr.is_null()
                                    || name_ptr.is_null()
                                {
                                    None
                                } else {
                                    let hardlink_id = CStr::from_ptr(hardlink_ptr)
                                        .to_string_lossy()
                                        .trim()
                                        .parse::<u64>()
                                        .ok();
                                    let parent_id = CStr::from_ptr(parent_ptr)
                                        .to_string_lossy()
                                        .trim()
                                        .parse::<u64>()
                                        .ok();
                                    let name =
                                        CStr::from_ptr(name_ptr).to_string_lossy().to_string();
                                    hardlink_id.map(|hardlink_id| (hardlink_id, parent_id, name))
                                }
                            };
                            PQclear(res);
                            Ok(value)
                        }
                        _ => {
                            PQclear(res);
                            Err(conn_error(conn))
                        }
                    }
                }
            };
            result
        })
    }

    pub fn promote_hardlink_to_primary(&self, file_id: u64) -> Result<bool, String> {
        let request_token_value = generate_request_token("promote-hardlink");
        let request_token = CString::new(request_token_value)
            .map_err(|_| "request token contains NUL byte".to_string())?;
        let sql_choose = CString::new(
            "
            SELECT id_hardlink, id_directory, name
            FROM hardlinks
            WHERE id_file = $1
            ORDER BY id_hardlink ASC
            LIMIT 1
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_update_null_parent = CString::new(
            "
            UPDATE files
            SET id_directory = NULL, name = $1
            WHERE id_file = $2
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_update_parent = CString::new(
            "
            UPDATE files
            SET id_directory = $1, name = $2
            WHERE id_file = $3
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_delete = CString::new("DELETE FROM hardlinks WHERE id_hardlink = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_lookup_request_token = CString::new(
            "SELECT COALESCE((SELECT did_promote::text FROM hardlink_promotion_request_tokens WHERE request_token = $1 LIMIT 1), '')",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_store_request_token = CString::new(
            "INSERT INTO hardlink_promotion_request_tokens (request_token, id_file, did_promote, created_at, updated_at) \
             VALUES ($1, $2, $3, NOW(), NOW()) \
             ON CONFLICT (request_token) DO UPDATE SET updated_at = NOW() \
             RETURNING did_promote",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let parse_bool = |value: &str| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "t" | "true" | "1" | "on"
            )
        };

        self.with_cached_connection(|conn| unsafe {
            transactional_replay_confirmed(
                conn,
                |conn| {
                    let request_token_params = [&request_token];
                    let existing_token_res =
                        exec_params(conn, &sql_lookup_request_token, &request_token_params)?;
                    let existing_token = fetch_single_text_option(existing_token_res)?;
                    match existing_token {
                        Some(existing) => Ok(Some(parse_bool(&existing))),
                        None => Ok(None),
                    }
                },
                |conn| {
                    let params = [&file_id];
                    let res = exec_params(conn, &sql_choose, &params)?;
                    let chosen = match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 3 {
                                None
                            } else {
                                let hardlink_ptr = PQgetvalue(res, 0, 0);
                                let parent_ptr = PQgetvalue(res, 0, 1);
                                let name_ptr = PQgetvalue(res, 0, 2);
                                if hardlink_ptr.is_null()
                                    || parent_ptr.is_null()
                                    || name_ptr.is_null()
                                {
                                    None
                                } else {
                                    let hardlink_id = CStr::from_ptr(hardlink_ptr)
                                        .to_string_lossy()
                                        .trim()
                                        .parse::<u64>()
                                        .ok();
                                    let parent_id = CStr::from_ptr(parent_ptr)
                                        .to_string_lossy()
                                        .trim()
                                        .parse::<u64>()
                                        .ok();
                                    let name =
                                        CStr::from_ptr(name_ptr).to_string_lossy().to_string();
                                    hardlink_id.map(|hardlink_id| (hardlink_id, parent_id, name))
                                }
                            };
                            PQclear(res);
                            value
                        }
                        _ => {
                            PQclear(res);
                            return Err(conn_error(conn));
                        }
                    };

                    let Some((hardlink_id, parent_id, name)) = chosen else {
                        let did_promote = CString::new("false")
                            .map_err(|_| "promotion flag contains NUL byte".to_string())?;
                        let params = [&request_token, &file_id, &did_promote];
                        let res = exec_params(conn, &sql_store_request_token, &params)?;
                        let stored = fetch_single_text(res)?;
                        if parse_bool(&stored) {
                            return Err(
                                "hardlink promotion request token stored unexpected result"
                                    .to_string(),
                            );
                        }
                        return Ok(false);
                    };

                    let file_name = CString::new(name)
                        .map_err(|_| "hardlink name contains NUL byte".to_string())?;
                    let hardlink_id = CString::new(hardlink_id.to_string())
                        .map_err(|_| "hardlink id contains NUL byte".to_string())?;
                    if let Some(parent_id) = parent_id {
                        let parent_id = CString::new(parent_id.to_string())
                            .map_err(|_| "parent id contains NUL byte".to_string())?;
                        let params = [&parent_id, &file_name, &file_id];
                        let res = exec_params(conn, &sql_update_parent, &params)?;
                        PQclear(res);
                    } else {
                        let params = [&file_name, &file_id];
                        let res = exec_params(conn, &sql_update_null_parent, &params)?;
                        PQclear(res);
                    }
                    let params = [&hardlink_id];
                    let res = exec_params(conn, &sql_delete, &params)?;
                    PQclear(res);
                    let did_promote = CString::new("true")
                        .map_err(|_| "promotion flag contains NUL byte".to_string())?;
                    let params = [&request_token, &file_id, &did_promote];
                    let res = exec_params(conn, &sql_store_request_token, &params)?;
                    let stored = fetch_single_text(res)?;
                    if !parse_bool(&stored) {
                        return Err(
                            "hardlink promotion request token stored unexpected result".to_string()
                        );
                    }
                    Ok(true)
                },
            )
        })
    }

    pub fn touch_file_entry(&self, file_id: u64) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE files SET modification_date = NOW(), change_date = NOW() WHERE id_file = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&file_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn touch_directory_entry(&self, directory_id: u64) -> Result<(), String> {
        let sql = CString::new("UPDATE directories SET modification_date = NOW(), change_date = NOW() WHERE id_directory = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&directory_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn touch_symlink_entry(&self, symlink_id: u64) -> Result<(), String> {
        let sql = CString::new("UPDATE symlinks SET modification_date = NOW(), change_date = NOW() WHERE id_symlink = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let symlink_id = CString::new(symlink_id.to_string())
            .map_err(|_| "symlink id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&symlink_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn rename_file_entry(
        &self,
        file_id: u64,
        new_parent_id: Option<u64>,
        new_name: &str,
    ) -> Result<(), String> {
        let (sql, params) = if let Some(parent_id) = new_parent_id {
            (
                CString::new("UPDATE files SET name = $1, id_directory = $2, change_date = NOW(), modification_date = NOW() WHERE id_file = $3")
                    .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![
                    CString::new(new_name).map_err(|_| "name contains NUL byte".to_string())?,
                    CString::new(parent_id.to_string()).map_err(|_| "parent id contains NUL byte".to_string())?,
                    CString::new(file_id.to_string()).map_err(|_| "file id contains NUL byte".to_string())?,
                ],
            )
        } else {
            (
                CString::new("UPDATE files SET name = $1, id_directory = NULL, change_date = NOW(), modification_date = NOW() WHERE id_file = $2")
                    .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![
                    CString::new(new_name).map_err(|_| "name contains NUL byte".to_string())?,
                    CString::new(file_id.to_string()).map_err(|_| "file id contains NUL byte".to_string())?,
                ],
            )
        };
        self.with_cached_connection(|conn| unsafe {
            let param_refs: Vec<&CString> = params.iter().collect();
            exec_command_params(conn, &sql, &param_refs)
        })
    }

    pub fn rename_hardlink_entry(
        &self,
        hardlink_id: u64,
        new_parent_id: Option<u64>,
        new_name: &str,
    ) -> Result<(), String> {
        let (sql, params) = if let Some(parent_id) = new_parent_id {
            (
                CString::new("UPDATE hardlinks SET name = $1, id_directory = $2, modification_date = NOW() WHERE id_hardlink = $3")
                    .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![
                    CString::new(new_name).map_err(|_| "name contains NUL byte".to_string())?,
                    CString::new(parent_id.to_string()).map_err(|_| "parent id contains NUL byte".to_string())?,
                    CString::new(hardlink_id.to_string()).map_err(|_| "hardlink id contains NUL byte".to_string())?,
                ],
            )
        } else {
            (
                CString::new("UPDATE hardlinks SET name = $1, id_directory = NULL, modification_date = NOW() WHERE id_hardlink = $2")
                    .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![
                    CString::new(new_name).map_err(|_| "name contains NUL byte".to_string())?,
                    CString::new(hardlink_id.to_string()).map_err(|_| "hardlink id contains NUL byte".to_string())?,
                ],
            )
        };
        self.with_cached_connection(|conn| unsafe {
            let param_refs: Vec<&CString> = params.iter().collect();
            exec_command_params(conn, &sql, &param_refs)
        })
    }

    pub fn rename_symlink_entry(
        &self,
        symlink_id: u64,
        new_parent_id: Option<u64>,
        new_name: &str,
    ) -> Result<(), String> {
        let (sql, params) = if let Some(parent_id) = new_parent_id {
            (
                CString::new("UPDATE symlinks SET name = $1, id_parent = $2, modification_date = NOW() WHERE id_symlink = $3")
                    .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![
                    CString::new(new_name).map_err(|_| "name contains NUL byte".to_string())?,
                    CString::new(parent_id.to_string()).map_err(|_| "parent id contains NUL byte".to_string())?,
                    CString::new(symlink_id.to_string()).map_err(|_| "symlink id contains NUL byte".to_string())?,
                ],
            )
        } else {
            (
                CString::new("UPDATE symlinks SET name = $1, id_parent = NULL, modification_date = NOW() WHERE id_symlink = $2")
                    .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![
                    CString::new(new_name).map_err(|_| "name contains NUL byte".to_string())?,
                    CString::new(symlink_id.to_string()).map_err(|_| "symlink id contains NUL byte".to_string())?,
                ],
            )
        };
        self.with_cached_connection(|conn| unsafe {
            let param_refs: Vec<&CString> = params.iter().collect();
            exec_command_params(conn, &sql, &param_refs)
        })
    }

    pub fn rename_directory_entry(
        &self,
        directory_id: u64,
        new_parent_id: Option<u64>,
        new_name: &str,
    ) -> Result<(), String> {
        let (sql, params) = if let Some(parent_id) = new_parent_id {
            (
                CString::new("UPDATE directories SET name = $1, id_parent = $2, modification_date = NOW(), change_date = NOW() WHERE id_directory = $3")
                    .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![
                    CString::new(new_name).map_err(|_| "name contains NUL byte".to_string())?,
                    CString::new(parent_id.to_string()).map_err(|_| "parent id contains NUL byte".to_string())?,
                    CString::new(directory_id.to_string()).map_err(|_| "directory id contains NUL byte".to_string())?,
                ],
            )
        } else {
            (
                CString::new("UPDATE directories SET name = $1, id_parent = NULL, modification_date = NOW(), change_date = NOW() WHERE id_directory = $2")
                    .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![
                    CString::new(new_name).map_err(|_| "name contains NUL byte".to_string())?,
                    CString::new(directory_id.to_string()).map_err(|_| "directory id contains NUL byte".to_string())?,
                ],
            )
        };
        self.with_cached_connection(|conn| unsafe {
            let param_refs: Vec<&CString> = params.iter().collect();
            exec_command_params(conn, &sql, &param_refs)
        })
    }

    pub fn delete_hardlink_entry(&self, hardlink_id: u64) -> Result<(), String> {
        let sql = CString::new("DELETE FROM hardlinks WHERE id_hardlink = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let hardlink_id = CString::new(hardlink_id.to_string())
            .map_err(|_| "hardlink id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&hardlink_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn delete_symlink_entry(&self, symlink_id: u64) -> Result<(), String> {
        let sql = CString::new("DELETE FROM symlinks WHERE id_symlink = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let symlink_id = CString::new(symlink_id.to_string())
            .map_err(|_| "symlink id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&symlink_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn delete_directory_entry(&self, directory_id: u64) -> Result<(), String> {
        let sql = CString::new("DELETE FROM directories WHERE id_directory = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&directory_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn create_hardlink(
        &self,
        source_file_id: u64,
        target_parent_id: Option<u64>,
        target_name: &str,
        uid: u32,
        gid: u32,
    ) -> Result<u64, String> {
        let target_name_text = target_name.to_string();
        let source_file_id_value = source_file_id;
        let uid_value = uid;
        let gid_value = gid;
        let target_name =
            CString::new(target_name).map_err(|_| "target name contains NUL byte".to_string())?;
        let source_file_id = CString::new(source_file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let uid = CString::new(uid.to_string()).map_err(|_| "uid contains NUL byte".to_string())?;
        let gid = CString::new(gid.to_string()).map_err(|_| "gid contains NUL byte".to_string())?;
        let sql_touch_parent = CString::new(
            "UPDATE directories SET modification_date = NOW(), change_date = NOW() WHERE id_directory = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_null_parent = CString::new(
            "
            INSERT INTO hardlinks (id_file, id_directory, name, uid, gid, change_date, creation_date, modification_date, access_date)
            VALUES ($1, NULL, $2, $3, $4, NOW(), NOW(), NOW(), NOW())
            RETURNING id_hardlink
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_parent = CString::new(
            "
            INSERT INTO hardlinks (id_file, id_directory, name, uid, gid, change_date, creation_date, modification_date, access_date)
            VALUES ($1, $5, $2, $3, $4, NOW(), NOW(), NOW(), NOW())
            RETURNING id_hardlink
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        let result = self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let res = if let Some(parent_id) = target_parent_id {
                    let parent_id = CString::new(parent_id.to_string())
                        .map_err(|_| "parent id contains NUL byte".to_string())?;
                    let params = [&source_file_id, &target_name, &uid, &gid, &parent_id];
                    exec_params(conn, &sql_parent, &params)?
                } else {
                    let params = [&source_file_id, &target_name, &uid, &gid];
                    exec_params(conn, &sql_null_parent, &params)?
                };
                let value = match PQresultStatus(res) {
                    PGRES_TUPLES_OK => {
                        let rows = PQntuples(res);
                        let cols = PQnfields(res);
                        let value = if rows < 1 || cols < 1 {
                            None
                        } else {
                            let value_ptr = PQgetvalue(res, 0, 0);
                            if value_ptr.is_null() {
                                None
                            } else {
                                CStr::from_ptr(value_ptr)
                                    .to_string_lossy()
                                    .trim()
                                    .parse::<u64>()
                                    .ok()
                            }
                        };
                        PQclear(res);
                        value.ok_or_else(|| "failed to create hardlink".to_string())
                    }
                    _ => {
                        PQclear(res);
                        Err(conn_error(conn))
                    }
                }?;
                if let Some(parent_id) = target_parent_id {
                    let parent_id = CString::new(parent_id.to_string())
                        .map_err(|_| "parent id contains NUL byte".to_string())?;
                    let params = [&parent_id];
                    let res = exec_params(conn, &sql_touch_parent, &params)?;
                    PQclear(res);
                }
                Ok(value)
            })
        });
        match result {
            Ok(value) => Ok(value),
            Err(err) => self.confirm_unique_violation(err, |repo| {
                repo.confirm_created_hardlink(
                    target_parent_id,
                    &target_name_text,
                    source_file_id_value,
                    uid_value,
                    gid_value,
                )
            }),
        }
    }

    pub fn create_symlink(
        &self,
        target_parent_id: Option<u64>,
        target_name: &str,
        target: &str,
        uid: u32,
        gid: u32,
        inode_seed: &str,
    ) -> Result<u64, String> {
        let target_name_text = target_name.to_string();
        let target_text = target.to_string();
        let inode_seed_text = inode_seed.to_string();
        let uid_value = uid;
        let gid_value = gid;
        let target_name =
            CString::new(target_name).map_err(|_| "target name contains NUL byte".to_string())?;
        let target =
            CString::new(target).map_err(|_| "symlink target contains NUL byte".to_string())?;
        let uid = CString::new(uid.to_string()).map_err(|_| "uid contains NUL byte".to_string())?;
        let gid = CString::new(gid.to_string()).map_err(|_| "gid contains NUL byte".to_string())?;
        let inode_seed =
            CString::new(inode_seed).map_err(|_| "inode seed contains NUL byte".to_string())?;
        let sql_touch_parent = CString::new(
            "UPDATE directories SET modification_date = NOW(), change_date = NOW() WHERE id_directory = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_null_parent = CString::new(
            "
            INSERT INTO symlinks (id_parent, name, target, uid, gid, inode_seed, change_date, creation_date, modification_date, access_date)
            VALUES (NULL, $1, $2, $3, $4, $5, NOW(), NOW(), NOW(), NOW())
            RETURNING id_symlink
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_parent = CString::new(
            "
            INSERT INTO symlinks (id_parent, name, target, uid, gid, inode_seed, change_date, creation_date, modification_date, access_date)
            VALUES ($6, $1, $2, $3, $4, $5, NOW(), NOW(), NOW(), NOW())
            RETURNING id_symlink
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        let result = self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let res = if let Some(parent_id) = target_parent_id {
                    let parent_id = CString::new(parent_id.to_string())
                        .map_err(|_| "parent id contains NUL byte".to_string())?;
                    let params = [&target_name, &target, &uid, &gid, &inode_seed, &parent_id];
                    exec_params(conn, &sql_parent, &params)?
                } else {
                    let params = [&target_name, &target, &uid, &gid, &inode_seed];
                    exec_params(conn, &sql_null_parent, &params)?
                };
                let value = match PQresultStatus(res) {
                    PGRES_TUPLES_OK => {
                        let rows = PQntuples(res);
                        let cols = PQnfields(res);
                        let value = if rows < 1 || cols < 1 {
                            None
                        } else {
                            let value_ptr = PQgetvalue(res, 0, 0);
                            if value_ptr.is_null() {
                                None
                            } else {
                                CStr::from_ptr(value_ptr)
                                    .to_string_lossy()
                                    .trim()
                                    .parse::<u64>()
                                    .ok()
                            }
                        };
                        PQclear(res);
                        value.ok_or_else(|| "failed to create symlink".to_string())
                    }
                    _ => {
                        PQclear(res);
                        Err(conn_error(conn))
                    }
                }?;
                if let Some(parent_id) = target_parent_id {
                    let parent_id = CString::new(parent_id.to_string())
                        .map_err(|_| "parent id contains NUL byte".to_string())?;
                    let params = [&parent_id];
                    let res = exec_params(conn, &sql_touch_parent, &params)?;
                    PQclear(res);
                }
                Ok(value)
            })
        });
        match result {
            Ok(value) => Ok(value),
            Err(err) => self.confirm_unique_violation(err, |repo| {
                repo.confirm_created_symlink(
                    target_parent_id,
                    &target_name_text,
                    &target_text,
                    uid_value,
                    gid_value,
                    &inode_seed_text,
                )
            }),
        }
    }

    pub fn create_directory(
        &self,
        target_parent_id: Option<u64>,
        target_name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
        inode_seed: &str,
    ) -> Result<u64, String> {
        let target_name_text = target_name.to_string();
        let inode_seed_text = inode_seed.to_string();
        let mode_text = format!("{:o}", mode);
        let uid_value = uid;
        let gid_value = gid;
        let target_name =
            CString::new(target_name).map_err(|_| "target name contains NUL byte".to_string())?;
        let uid = CString::new(uid.to_string()).map_err(|_| "uid contains NUL byte".to_string())?;
        let gid = CString::new(gid.to_string()).map_err(|_| "gid contains NUL byte".to_string())?;
        let inode_seed =
            CString::new(inode_seed).map_err(|_| "inode seed contains NUL byte".to_string())?;
        let mode = CString::new(format!("{:o}", mode))
            .map_err(|_| "mode contains NUL byte".to_string())?;
        let sql_touch_parent = CString::new(
            "UPDATE directories SET modification_date = NOW(), change_date = NOW() WHERE id_directory = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_null_parent = CString::new(
            "
            INSERT INTO directories (id_parent, name, mode, uid, gid, inode_seed, change_date, creation_date, modification_date, access_date)
            VALUES (NULL, $1, $2, $3, $4, $5, NOW(), NOW(), NOW(), NOW())
            RETURNING id_directory
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_parent = CString::new(
            "
            INSERT INTO directories (id_parent, name, mode, uid, gid, inode_seed, change_date, creation_date, modification_date, access_date)
            VALUES ($6, $1, $2, $3, $4, $5, NOW(), NOW(), NOW(), NOW())
            RETURNING id_directory
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        let result = self.with_cached_connection(|conn| unsafe {
            let result = transactional_replayable(conn, |conn| {
                let res = if let Some(parent_id) = target_parent_id {
                    let parent_id = CString::new(parent_id.to_string())
                        .map_err(|_| "parent id contains NUL byte".to_string())?;
                    let params = [&target_name, &mode, &uid, &gid, &inode_seed, &parent_id];
                    exec_params(conn, &sql_parent, &params)?
                } else {
                    let params = [&target_name, &mode, &uid, &gid, &inode_seed];
                    exec_params(conn, &sql_null_parent, &params)?
                };
                let value = match PQresultStatus(res) {
                    PGRES_TUPLES_OK => {
                        let rows = PQntuples(res);
                        let cols = PQnfields(res);
                        let value = if rows < 1 || cols < 1 {
                            None
                        } else {
                            let value_ptr = PQgetvalue(res, 0, 0);
                            if value_ptr.is_null() {
                                None
                            } else {
                                CStr::from_ptr(value_ptr)
                                    .to_string_lossy()
                                    .trim()
                                    .parse::<u64>()
                                    .ok()
                            }
                        };
                        PQclear(res);
                        value.ok_or_else(|| "failed to create directory".to_string())
                    }
                    _ => {
                        PQclear(res);
                        Err(conn_error(conn))
                    }
                }?;
                if let Some(parent_id) = target_parent_id {
                    let parent_id = CString::new(parent_id.to_string())
                        .map_err(|_| "parent id contains NUL byte".to_string())?;
                    let params = [&parent_id];
                    let res = exec_params(conn, &sql_touch_parent, &params)?;
                    PQclear(res);
                }
                Ok(value)
            });
            result
        });
        match result {
            Ok(value) => Ok(value),
            Err(err) => self.confirm_unique_violation(err, |repo| {
                repo.confirm_created_directory(
                    target_parent_id,
                    &target_name_text,
                    &mode_text,
                    uid_value,
                    gid_value,
                    &inode_seed_text,
                )
            }),
        }
    }

    pub fn create_file(
        &self,
        target_parent_id: Option<u64>,
        target_name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
        inode_seed: &str,
    ) -> Result<u64, String> {
        let target_name_text = target_name.to_string();
        let inode_seed_text = inode_seed.to_string();
        let mode_text = format!("{:o}", mode);
        let uid_value = uid;
        let gid_value = gid;
        let target_name =
            CString::new(target_name).map_err(|_| "target name contains NUL byte".to_string())?;
        let uid = CString::new(uid.to_string()).map_err(|_| "uid contains NUL byte".to_string())?;
        let gid = CString::new(gid.to_string()).map_err(|_| "gid contains NUL byte".to_string())?;
        let inode_seed =
            CString::new(inode_seed).map_err(|_| "inode seed contains NUL byte".to_string())?;
        let mode = CString::new(format!("{:o}", mode))
            .map_err(|_| "mode contains NUL byte".to_string())?;
        let sql_touch_parent = CString::new(
            "UPDATE directories SET modification_date = NOW(), change_date = NOW() WHERE id_directory = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_data_object = CString::new(
            "INSERT INTO data_objects (file_size, content_hash, reference_count, creation_date, modification_date) \
             VALUES (0, NULL, 1, NOW(), NOW()) RETURNING id_data_object",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_null_parent = CString::new(
            "
            INSERT INTO files (id_directory, name, size, mode, uid, gid, inode_seed, data_object_id, change_date, modification_date, access_date, creation_date)
            VALUES (NULL, $1, 0, $2, $3, $4, $5, $6, NOW(), NOW(), NOW(), NOW())
            RETURNING id_file
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_parent = CString::new(
            "
            INSERT INTO files (id_directory, name, size, mode, uid, gid, inode_seed, data_object_id, change_date, modification_date, access_date, creation_date)
            VALUES ($7, $1, 0, $2, $3, $4, $5, $6, NOW(), NOW(), NOW(), NOW())
            RETURNING id_file
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        let result = self.with_cached_connection(|conn| unsafe {
            let result = transactional_replayable(conn, |conn| {
                let data_object_res = exec_params(conn, &sql_data_object, &[])?;
                let data_object_id = fetch_single_text(data_object_res)?
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "failed to create data object".to_string())?;
                let data_object_id = CString::new(data_object_id.to_string())
                    .map_err(|_| "data object id contains NUL byte".to_string())?;
                let res = if let Some(parent_id) = target_parent_id {
                    let parent_id = CString::new(parent_id.to_string())
                        .map_err(|_| "parent id contains NUL byte".to_string())?;
                    let params = [
                        &target_name,
                        &mode,
                        &uid,
                        &gid,
                        &inode_seed,
                        &data_object_id,
                        &parent_id,
                    ];
                    exec_params(conn, &sql_parent, &params)?
                } else {
                    let params = [
                        &target_name,
                        &mode,
                        &uid,
                        &gid,
                        &inode_seed,
                        &data_object_id,
                    ];
                    exec_params(conn, &sql_null_parent, &params)?
                };
                let value = match PQresultStatus(res) {
                    PGRES_TUPLES_OK => {
                        let rows = PQntuples(res);
                        let cols = PQnfields(res);
                        let value = if rows < 1 || cols < 1 {
                            None
                        } else {
                            let value_ptr = PQgetvalue(res, 0, 0);
                            if value_ptr.is_null() {
                                None
                            } else {
                                CStr::from_ptr(value_ptr)
                                    .to_string_lossy()
                                    .trim()
                                    .parse::<u64>()
                                    .ok()
                            }
                        };
                        PQclear(res);
                        value.ok_or_else(|| "failed to create file".to_string())
                    }
                    _ => {
                        PQclear(res);
                        Err(conn_error(conn))
                    }
                }?;
                if let Some(parent_id) = target_parent_id {
                    let parent_id = CString::new(parent_id.to_string())
                        .map_err(|_| "parent id contains NUL byte".to_string())?;
                    let params = [&parent_id];
                    let res = exec_params(conn, &sql_touch_parent, &params)?;
                    PQclear(res);
                }
                Ok(value)
            });
            result
        });
        match result {
            Ok(value) => Ok(value),
            Err(err) => self.confirm_unique_violation(err, |repo| {
                repo.confirm_created_file(
                    target_parent_id,
                    &target_name_text,
                    &mode_text,
                    uid_value,
                    gid_value,
                    &inode_seed_text,
                )
            }),
        }
    }

    pub fn create_special_file(
        &self,
        target_parent_id: Option<u64>,
        target_name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
        inode_seed: &str,
        file_kind: &str,
        rdev_major: u32,
        rdev_minor: u32,
    ) -> Result<u64, String> {
        let target_name_text = target_name.to_string();
        let inode_seed_text = inode_seed.to_string();
        let file_kind_text = file_kind.to_string();
        let mode_text = format!("{:o}", mode);
        let uid_value = uid;
        let gid_value = gid;
        let rdev_major_value = rdev_major;
        let rdev_minor_value = rdev_minor;
        let target_name =
            CString::new(target_name).map_err(|_| "target name contains NUL byte".to_string())?;
        let uid = CString::new(uid.to_string()).map_err(|_| "uid contains NUL byte".to_string())?;
        let gid = CString::new(gid.to_string()).map_err(|_| "gid contains NUL byte".to_string())?;
        let inode_seed =
            CString::new(inode_seed).map_err(|_| "inode seed contains NUL byte".to_string())?;
        let mode = CString::new(format!("{:o}", mode))
            .map_err(|_| "mode contains NUL byte".to_string())?;
        let file_kind =
            CString::new(file_kind).map_err(|_| "file kind contains NUL byte".to_string())?;
        let rdev_major = CString::new(rdev_major.to_string())
            .map_err(|_| "rdev major contains NUL byte".to_string())?;
        let rdev_minor = CString::new(rdev_minor.to_string())
            .map_err(|_| "rdev minor contains NUL byte".to_string())?;
        let sql_touch_parent = CString::new(
            "UPDATE directories SET modification_date = NOW(), change_date = NOW() WHERE id_directory = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_data_object = CString::new(
            "INSERT INTO data_objects (file_size, content_hash, reference_count, creation_date, modification_date) \
             VALUES (0, NULL, 1, NOW(), NOW()) RETURNING id_data_object",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_null_parent = CString::new(
            "
            INSERT INTO files (id_directory, name, size, mode, uid, gid, inode_seed, data_object_id, change_date, modification_date, access_date, creation_date)
            VALUES (NULL, $1, 0, $2, $3, $4, $5, $6, NOW(), NOW(), NOW(), NOW())
            RETURNING id_file
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_parent = CString::new(
            "
            INSERT INTO files (id_directory, name, size, mode, uid, gid, inode_seed, data_object_id, change_date, modification_date, access_date, creation_date)
            VALUES ($7, $1, 0, $2, $3, $4, $5, $6, NOW(), NOW(), NOW(), NOW())
            RETURNING id_file
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_special = CString::new(
            "
            INSERT INTO special_files (id_file, file_type, rdev_major, rdev_minor)
            VALUES ($1, $2, $3, $4)
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        let result = self.with_cached_connection(|conn| unsafe {
            let result = transactional_replayable(conn, |conn| {
                let data_object_res = exec_params(conn, &sql_data_object, &[])?;
                let data_object_id = fetch_single_text(data_object_res)?
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "failed to create data object".to_string())?;
                let data_object_id = CString::new(data_object_id.to_string())
                    .map_err(|_| "data object id contains NUL byte".to_string())?;
                let res = if let Some(parent_id) = target_parent_id {
                    let parent_id = CString::new(parent_id.to_string())
                        .map_err(|_| "parent id contains NUL byte".to_string())?;
                    let params = [
                        &target_name,
                        &mode,
                        &uid,
                        &gid,
                        &inode_seed,
                        &data_object_id,
                        &parent_id,
                    ];
                    exec_params(conn, &sql_parent, &params)?
                } else {
                    let params = [
                        &target_name,
                        &mode,
                        &uid,
                        &gid,
                        &inode_seed,
                        &data_object_id,
                    ];
                    exec_params(conn, &sql_null_parent, &params)?
                };
                let id_file = match PQresultStatus(res) {
                    PGRES_TUPLES_OK => {
                        let rows = PQntuples(res);
                        let cols = PQnfields(res);
                        let value = if rows < 1 || cols < 1 {
                            None
                        } else {
                            let value_ptr = PQgetvalue(res, 0, 0);
                            if value_ptr.is_null() {
                                None
                            } else {
                                CStr::from_ptr(value_ptr)
                                    .to_string_lossy()
                                    .trim()
                                    .parse::<u64>()
                                    .ok()
                            }
                        };
                        PQclear(res);
                        value.ok_or_else(|| "failed to create special file".to_string())?
                    }
                    _ => {
                        PQclear(res);
                        return Err(conn_error(conn));
                    }
                };

                let id_file_text = CString::new(id_file.to_string())
                    .map_err(|_| "file id contains NUL byte".to_string())?;
                let special_params = [&id_file_text, &file_kind, &rdev_major, &rdev_minor];
                let res = exec_params(conn, &sql_special, &special_params)?;
                match PQresultStatus(res) {
                    PGRES_COMMAND_OK => {
                        PQclear(res);
                    }
                    _ => {
                        PQclear(res);
                        return Err(conn_error(conn));
                    }
                }

                if let Some(parent_id) = target_parent_id {
                    let parent_id = CString::new(parent_id.to_string())
                        .map_err(|_| "parent id contains NUL byte".to_string())?;
                    let params = [&parent_id];
                    let res = exec_params(conn, &sql_touch_parent, &params)?;
                    PQclear(res);
                }
                Ok(id_file)
            });
            result
        });
        match result {
            Ok(value) => Ok(value),
            Err(err) => self.confirm_unique_violation(err, |repo| {
                repo.confirm_created_special_file(
                    target_parent_id,
                    &target_name_text,
                    &mode_text,
                    uid_value,
                    gid_value,
                    &inode_seed_text,
                    &file_kind_text,
                    rdev_major_value,
                    rdev_minor_value,
                )
            }),
        }
    }

    pub fn persist_copy_block_crc_rows<'a>(
        &self,
        file_id: u64,
        block_size: u64,
        blocks: &[PersistBlockRow<'a>],
    ) -> Result<(), String> {
        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                self.persist_copy_block_crc_rows_on_conn(conn, file_id, block_size, blocks)
            })
        })
    }

    unsafe fn persist_file_blocks_copy_binary_staging_on_conn<'a>(
        &self,
        conn: *mut PGconn,
        file_id: u64,
        file_size: u64,
        block_size: u64,
        total_blocks: u64,
        truncate_pending: bool,
        blocks: &[PersistBlockRow<'a>],
        maintain_copy_crc_table: bool,
    ) -> Result<(), String> {
        let block_size = block_size.max(1);
        let total_blocks_text = CString::new(total_blocks.to_string())
            .map_err(|_| "total blocks contains NUL byte".to_string())?;
        let sql_delete_tail = CString::new(
            "
            DELETE FROM data_blocks
            WHERE data_object_id = $1 AND _order >= $2
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_delete_crc_tail = CString::new(
            "
            DELETE FROM copy_block_crc
            WHERE data_object_id = $1 AND _order >= $2
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let prefer_replacement =
            persist_blocks_cover_full_file(file_size, block_size, total_blocks, blocks);
        let target = match self.data_object_write_target_on_conn(
            conn,
            file_id,
            file_size,
            prefer_replacement,
            maintain_copy_crc_table,
        )? {
            Some(value) => value,
            None => return Ok(()),
        };
        let data_object_id = target.data_object_id;
        let target_data_object_is_empty = target.payload_rows_empty;
        let data_object_id_text = CString::new(data_object_id.to_string())
            .map_err(|_| "data object id contains NUL byte".to_string())?;
        if truncate_pending && !target_data_object_is_empty {
            let params = [&data_object_id_text, &total_blocks_text];
            exec_command_params(conn, &sql_delete_tail, &params)?;
            if maintain_copy_crc_table {
                exec_command_params(conn, &sql_delete_crc_tail, &params)?;
            }
        }

        if !blocks.is_empty() {
            create_persist_block_stage_table(conn)?;
            let copy_sql = CString::new(format!(
                "COPY {} (data_object_id, _order, data, crc32) FROM STDIN BINARY",
                PERSIST_BLOCK_STAGE_TABLE
            ))
            .map_err(|_| "SQL contains NUL byte".to_string())?;
            let copy_started = Instant::now();
            let mut copy = CopyInSession::start(conn, &copy_sql)?;
            let mut copy_buffer = Vec::with_capacity(persist_copy_send_buffer_bytes());
            append_copy_binary_header(&mut copy_buffer);
            let data_object_id_i64 = i64::try_from(data_object_id)
                .map_err(|_| "data object id out of range for copy staging".to_string())?;
            let block_size = block_size.max(1);
            for block in blocks {
                append_persist_block_copy_binary_row(
                    &mut copy_buffer,
                    data_object_id_i64,
                    block,
                    block_size,
                )?;
                flush_copy_send_buffer_if_full(&mut copy, &mut copy_buffer)?;
            }
            flush_copy_send_buffer(&mut copy, &mut copy_buffer)?;
            copy.finish()?;
            self.record_persist_copy_stage_elapsed(copy_started.elapsed());
            let merge_started = Instant::now();
            merge_persist_block_stage_table(
                conn,
                maintain_copy_crc_table,
                target_data_object_is_empty,
            )?;
            self.record_persist_data_blocks_merge_elapsed(merge_started.elapsed());
        }

        self.finish_data_object_write_on_conn(conn, file_id, file_size, target)?;
        Ok(())
    }

    unsafe fn persist_file_blocks_direct_on_conn<'a>(
        &self,
        conn: *mut PGconn,
        file_id: u64,
        file_size: u64,
        block_size: u64,
        total_blocks: u64,
        truncate_pending: bool,
        blocks: &[PersistBlockRow<'a>],
        maintain_copy_crc_table: bool,
        transport: PersistBlockTransport,
    ) -> Result<(), String> {
        let block_size = block_size.max(1);
        let total_blocks_text = CString::new(total_blocks.to_string())
            .map_err(|_| "total blocks contains NUL byte".to_string())?;
        let sql_delete_tail = CString::new(
            "
            DELETE FROM data_blocks
            WHERE data_object_id = $1 AND _order >= $2
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_delete_crc_tail = CString::new(
            "
            DELETE FROM copy_block_crc
            WHERE data_object_id = $1 AND _order >= $2
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_upsert_data_binary = CString::new(
            "
            INSERT INTO data_blocks (data_object_id, _order, data)
            VALUES ($1, $2, $3)
            ON CONFLICT (data_object_id, _order)
            DO UPDATE SET data = EXCLUDED.data
            WHERE data_blocks.data IS DISTINCT FROM EXCLUDED.data
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_upsert_data_hex = CString::new(
            "
            INSERT INTO data_blocks (data_object_id, _order, data)
            VALUES ($1, $2, decode($3, 'hex'))
            ON CONFLICT (data_object_id, _order)
            DO UPDATE SET data = EXCLUDED.data
            WHERE data_blocks.data IS DISTINCT FROM EXCLUDED.data
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_delete_data = CString::new(
            "
            DELETE FROM data_blocks
            WHERE data_object_id = $1 AND _order = $2
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        let prefer_replacement =
            persist_blocks_cover_full_file(file_size, block_size, total_blocks, blocks);
        let target = match self.data_object_write_target_on_conn(
            conn,
            file_id,
            file_size,
            prefer_replacement,
            maintain_copy_crc_table,
        )? {
            Some(value) => value,
            None => return Ok(()),
        };
        let data_object_id = target.data_object_id;
        let target_data_object_is_empty = target.payload_rows_empty;
        let data_object_id_text = CString::new(data_object_id.to_string())
            .map_err(|_| "data object id contains NUL byte".to_string())?;
        if truncate_pending && !target_data_object_is_empty {
            let params = [&data_object_id_text, &total_blocks_text];
            exec_command_params(conn, &sql_delete_tail, &params)?;
            if maintain_copy_crc_table {
                exec_command_params(conn, &sql_delete_crc_tail, &params)?;
            }
        }

        let chunk_size = self.persist_buffer_chunk_blocks.max(1) as usize;
        for block_chunk in blocks.chunks(chunk_size) {
            for block in block_chunk {
                let block_index_text = CString::new(block.block_index.to_string())
                    .map_err(|_| "block index contains NUL byte".to_string())?;
                if block.used_len >= block_size {
                    let normalized = normalize_block_bytes(block.data, block_size as usize);
                    match transport {
                        PersistBlockTransport::BinaryBytea => {
                            let params = [
                                SqlParam::Text(&data_object_id_text),
                                SqlParam::Text(&block_index_text),
                                SqlParam::Binary(&normalized),
                            ];
                            exec_command_params_with_formats(
                                conn,
                                &sql_upsert_data_binary,
                                &params,
                            )?;
                        }
                        PersistBlockTransport::LegacyHex => {
                            let hex = CString::new(hex_encode_bytes(&normalized))
                                .map_err(|_| "hex payload contains NUL byte".to_string())?;
                            let params = [
                                SqlParam::Text(&data_object_id_text),
                                SqlParam::Text(&block_index_text),
                                SqlParam::Text(&hex),
                            ];
                            exec_command_params_with_formats(conn, &sql_upsert_data_hex, &params)?;
                        }
                        PersistBlockTransport::CopyBinaryStaging => {
                            return Err("copy_binary_staging is handled by the staging write path"
                                .to_string());
                        }
                    }
                } else {
                    let params = [&data_object_id_text, &block_index_text];
                    exec_command_params(conn, &sql_delete_data, &params)?;
                }
            }
        }

        if maintain_copy_crc_table {
            self.persist_copy_block_crc_rows_for_data_object_on_conn(
                conn,
                data_object_id,
                block_size,
                blocks,
            )?;
        }

        self.finish_data_object_write_on_conn(conn, file_id, file_size, target)?;
        Ok(())
    }

    unsafe fn persist_file_blocks_streaming_on_conn(
        &self,
        conn: *mut PGconn,
        file_id: u64,
        file_size: u64,
        block_size: u64,
        total_blocks: u64,
        truncate_pending: bool,
        path: &Path,
        expected_hash: &str,
        maintain_copy_crc_table: bool,
    ) -> Result<(), String> {
        let block_size = block_size.max(1);
        let block_size_usize = block_size as usize;
        let total_blocks_text = CString::new(total_blocks.to_string())
            .map_err(|_| "total blocks contains NUL byte".to_string())?;
        let sql_delete_tail = CString::new(
            "
            DELETE FROM data_blocks
            WHERE data_object_id = $1 AND _order >= $2
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_delete_crc_tail = CString::new(
            "
            DELETE FROM copy_block_crc
            WHERE data_object_id = $1 AND _order >= $2
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let prefer_replacement = file_size > 0;
        let target = match self.data_object_write_target_on_conn(
            conn,
            file_id,
            file_size,
            prefer_replacement,
            maintain_copy_crc_table,
        )? {
            Some(value) => value,
            None => return Ok(()),
        };
        let data_object_id = target.data_object_id;
        let target_data_object_is_empty = target.payload_rows_empty;
        let data_object_id_text = CString::new(data_object_id.to_string())
            .map_err(|_| "data object id contains NUL byte".to_string())?;
        if truncate_pending && !target_data_object_is_empty {
            let params = [&data_object_id_text, &total_blocks_text];
            exec_command_params(conn, &sql_delete_tail, &params)?;
            if maintain_copy_crc_table {
                exec_command_params(conn, &sql_delete_crc_tail, &params)?;
            }
        }

        let chunk_limit = self.persist_buffer_chunk_blocks.max(1) as usize;
        let mut file = File::open(path)
            .map_err(|err| format!("unable to open {} for import: {err}", path.display()))?;
        let mut hasher = Sha256::new();
        let mut read_total = 0u64;
        let mut next_block_index = 0u64;
        let mut chunk_start_block = 0u64;
        let mut chunk_data: Vec<Vec<u8>> = Vec::with_capacity(chunk_limit);
        let mut buffer = vec![0u8; block_size_usize];

        match self.persist_block_transport {
            PersistBlockTransport::CopyBinaryStaging => {
                create_persist_block_stage_table(conn)?;
                let copy_sql = CString::new(format!(
                    "COPY {} (data_object_id, _order, data, crc32) FROM STDIN BINARY",
                    PERSIST_BLOCK_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let copy_started = Instant::now();
                let mut copy = CopyInSession::start(conn, &copy_sql)?;
                let mut copy_buffer = Vec::with_capacity(persist_copy_send_buffer_bytes());
                append_copy_binary_header(&mut copy_buffer);
                let data_object_id_i64 = i64::try_from(data_object_id)
                    .map_err(|_| "data object id out of range for copy staging".to_string())?;

                loop {
                    let read = file.read(&mut buffer).map_err(|err| {
                        format!("read failed while importing {}: {err}", path.display())
                    })?;
                    if read == 0 {
                        break;
                    }
                    hasher.update(&buffer[..read]);
                    read_total = read_total.saturating_add(read as u64);
                    if chunk_data.is_empty() {
                        chunk_start_block = next_block_index;
                    }
                    chunk_data.push(buffer[..read].to_vec());
                    next_block_index = next_block_index.saturating_add(1);
                    if chunk_data.len() >= chunk_limit {
                        let chunk_rows = chunk_data
                            .iter()
                            .enumerate()
                            .map(|(offset, data)| PersistBlockRow {
                                block_index: chunk_start_block + offset as u64,
                                data: data.as_slice(),
                                used_len: data.len() as u64,
                            })
                            .collect::<Vec<_>>();
                        for row in &chunk_rows {
                            append_persist_block_copy_binary_row(
                                &mut copy_buffer,
                                data_object_id_i64,
                                row,
                                block_size,
                            )?;
                            flush_copy_send_buffer_if_full(&mut copy, &mut copy_buffer)?;
                        }
                        chunk_data.clear();
                    }
                }

                if !chunk_data.is_empty() {
                    let chunk_rows = chunk_data
                        .iter()
                        .enumerate()
                        .map(|(offset, data)| PersistBlockRow {
                            block_index: chunk_start_block + offset as u64,
                            data: data.as_slice(),
                            used_len: data.len() as u64,
                        })
                        .collect::<Vec<_>>();
                    for row in &chunk_rows {
                        append_persist_block_copy_binary_row(
                            &mut copy_buffer,
                            data_object_id_i64,
                            row,
                            block_size,
                        )?;
                        flush_copy_send_buffer_if_full(&mut copy, &mut copy_buffer)?;
                    }
                    chunk_data.clear();
                }

                if read_total != file_size {
                    return Err(format!(
                        "source file changed while importing {}",
                        path.display()
                    ));
                }
                let actual_hash = hex_encode_bytes(&hasher.finalize());
                if actual_hash != expected_hash {
                    return Err(format!(
                        "full hash changed while importing {}",
                        path.display()
                    ));
                }

                flush_copy_send_buffer(&mut copy, &mut copy_buffer)?;
                copy.finish()?;
                self.record_persist_copy_stage_elapsed(copy_started.elapsed());
                let merge_started = Instant::now();
                merge_persist_block_stage_table(
                    conn,
                    maintain_copy_crc_table,
                    target_data_object_is_empty,
                )?;
                self.record_persist_data_blocks_merge_elapsed(merge_started.elapsed());
            }
            PersistBlockTransport::BinaryBytea | PersistBlockTransport::LegacyHex => {
                let sql_upsert_data_binary = CString::new(
                    "
                    INSERT INTO data_blocks (data_object_id, _order, data)
                    VALUES ($1, $2, $3)
                    ON CONFLICT (data_object_id, _order)
                    DO UPDATE SET data = EXCLUDED.data
                    WHERE data_blocks.data IS DISTINCT FROM EXCLUDED.data
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let sql_upsert_data_hex = CString::new(
                    "
                    INSERT INTO data_blocks (data_object_id, _order, data)
                    VALUES ($1, $2, decode($3, 'hex'))
                    ON CONFLICT (data_object_id, _order)
                    DO UPDATE SET data = EXCLUDED.data
                    WHERE data_blocks.data IS DISTINCT FROM EXCLUDED.data
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let sql_delete_data = CString::new(
                    "
                    DELETE FROM data_blocks
                    WHERE data_object_id = $1 AND _order = $2
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?;

                loop {
                    let read = file.read(&mut buffer).map_err(|err| {
                        format!("read failed while importing {}: {err}", path.display())
                    })?;
                    if read == 0 {
                        break;
                    }
                    hasher.update(&buffer[..read]);
                    read_total = read_total.saturating_add(read as u64);
                    if chunk_data.is_empty() {
                        chunk_start_block = next_block_index;
                    }
                    chunk_data.push(buffer[..read].to_vec());
                    next_block_index = next_block_index.saturating_add(1);
                    if chunk_data.len() >= chunk_limit {
                        let chunk_rows = chunk_data
                            .iter()
                            .enumerate()
                            .map(|(offset, data)| PersistBlockRow {
                                block_index: chunk_start_block + offset as u64,
                                data: data.as_slice(),
                                used_len: data.len() as u64,
                            })
                            .collect::<Vec<_>>();
                        for row in &chunk_rows {
                            let block_index_text = CString::new(row.block_index.to_string())
                                .map_err(|_| "block index contains NUL byte".to_string())?;
                            if row.used_len >= block_size {
                                let normalized = normalize_block_bytes(row.data, block_size_usize);
                                match self.persist_block_transport {
                                    PersistBlockTransport::BinaryBytea => {
                                        let params = [
                                            SqlParam::Text(&data_object_id_text),
                                            SqlParam::Text(&block_index_text),
                                            SqlParam::Binary(&normalized),
                                        ];
                                        exec_command_params_with_formats(
                                            conn,
                                            &sql_upsert_data_binary,
                                            &params,
                                        )?;
                                    }
                                    PersistBlockTransport::LegacyHex => {
                                        let hex = CString::new(hex_encode_bytes(&normalized))
                                            .map_err(|_| {
                                                "hex payload contains NUL byte".to_string()
                                            })?;
                                        let params = [
                                            SqlParam::Text(&data_object_id_text),
                                            SqlParam::Text(&block_index_text),
                                            SqlParam::Text(&hex),
                                        ];
                                        exec_command_params_with_formats(
                                            conn,
                                            &sql_upsert_data_hex,
                                            &params,
                                        )?;
                                    }
                                    PersistBlockTransport::CopyBinaryStaging => {
                                        return Err(
                                            "copy_binary_staging is handled by the staging write path"
                                                .to_string(),
                                        );
                                    }
                                }
                            } else {
                                let params = [&data_object_id_text, &block_index_text];
                                exec_command_params(conn, &sql_delete_data, &params)?;
                            }
                        }
                        if maintain_copy_crc_table {
                            self.persist_copy_block_crc_rows_for_data_object_on_conn(
                                conn,
                                data_object_id,
                                block_size,
                                &chunk_rows,
                            )?;
                        }
                        chunk_data.clear();
                    }
                }

                if !chunk_data.is_empty() {
                    let chunk_rows = chunk_data
                        .iter()
                        .enumerate()
                        .map(|(offset, data)| PersistBlockRow {
                            block_index: chunk_start_block + offset as u64,
                            data: data.as_slice(),
                            used_len: data.len() as u64,
                        })
                        .collect::<Vec<_>>();
                    for row in &chunk_rows {
                        let block_index_text = CString::new(row.block_index.to_string())
                            .map_err(|_| "block index contains NUL byte".to_string())?;
                        if row.used_len >= block_size {
                            let normalized = normalize_block_bytes(row.data, block_size_usize);
                            match self.persist_block_transport {
                                PersistBlockTransport::BinaryBytea => {
                                    let params = [
                                        SqlParam::Text(&data_object_id_text),
                                        SqlParam::Text(&block_index_text),
                                        SqlParam::Binary(&normalized),
                                    ];
                                    exec_command_params_with_formats(
                                        conn,
                                        &sql_upsert_data_binary,
                                        &params,
                                    )?;
                                }
                                PersistBlockTransport::LegacyHex => {
                                    let hex = CString::new(hex_encode_bytes(&normalized))
                                        .map_err(|_| "hex payload contains NUL byte".to_string())?;
                                    let params = [
                                        SqlParam::Text(&data_object_id_text),
                                        SqlParam::Text(&block_index_text),
                                        SqlParam::Text(&hex),
                                    ];
                                    exec_command_params_with_formats(
                                        conn,
                                        &sql_upsert_data_hex,
                                        &params,
                                    )?;
                                }
                                PersistBlockTransport::CopyBinaryStaging => {
                                    return Err(
                                        "copy_binary_staging is handled by the staging write path"
                                            .to_string(),
                                    );
                                }
                            }
                        } else {
                            let params = [&data_object_id_text, &block_index_text];
                            exec_command_params(conn, &sql_delete_data, &params)?;
                        }
                    }
                    if maintain_copy_crc_table {
                        self.persist_copy_block_crc_rows_for_data_object_on_conn(
                            conn,
                            data_object_id,
                            block_size,
                            &chunk_rows,
                        )?;
                    }
                    chunk_data.clear();
                }

                if read_total != file_size {
                    return Err(format!(
                        "source file changed while importing {}",
                        path.display()
                    ));
                }
                let actual_hash = hex_encode_bytes(&hasher.finalize());
                if actual_hash != expected_hash {
                    return Err(format!(
                        "full hash changed while importing {}",
                        path.display()
                    ));
                }
            }
        }

        self.finish_data_object_write_on_conn(conn, file_id, file_size, target)?;
        Ok(())
    }

    pub fn persist_file_blocks_from_path(
        &self,
        file_id: u64,
        file_size: u64,
        block_size: u64,
        total_blocks: u64,
        truncate_pending: bool,
        path: &Path,
        expected_hash: &str,
        maintain_copy_crc_table: bool,
    ) -> Result<(), String> {
        let mut payload_guard = self.payload_persist_guard(file_size, total_blocks);
        let result = self.with_cached_connection(|conn| unsafe {
            let transaction_started = Instant::now();
            let result = transactional_replayable(conn, |conn| {
                self.persist_file_blocks_streaming_on_conn(
                    conn,
                    file_id,
                    file_size,
                    block_size,
                    total_blocks,
                    truncate_pending,
                    path,
                    expected_hash,
                    maintain_copy_crc_table,
                )?;
                // FOD connection setup pins READ COMMITTED; the final quota
                // query relies on seeing writers committed while this
                // transaction waited for the advisory lock.
                let quota_lock = self.lock_payload_quota_on_conn(conn)?;
                self.enforce_payload_quota_on_conn(conn, &quota_lock, None)
            });
            self.record_persist_transaction_elapsed(transaction_started.elapsed(), result.is_err());
            result
        });
        if result.is_ok() {
            payload_guard.mark_success();
        }
        result
    }

    pub fn persist_file_blocks(
        &self,
        file_id: u64,
        file_size: u64,
        block_size: u64,
        total_blocks: u64,
        truncate_pending: bool,
        blocks: &[PersistBlockRow],
    ) -> Result<(), String> {
        self.persist_file_blocks_with_crc_flag(
            file_id,
            file_size,
            block_size,
            total_blocks,
            truncate_pending,
            blocks,
            true,
        )
    }

    pub fn persist_file_blocks_with_crc_flag(
        &self,
        file_id: u64,
        file_size: u64,
        block_size: u64,
        total_blocks: u64,
        truncate_pending: bool,
        blocks: &[PersistBlockRow],
        maintain_copy_crc_table: bool,
    ) -> Result<(), String> {
        self.persist_file_blocks_with_crc_flag_and_reservation(
            file_id,
            file_size,
            block_size,
            total_blocks,
            truncate_pending,
            blocks,
            maintain_copy_crc_table,
            None,
        )
    }

    pub fn persist_file_blocks_with_crc_flag_and_reservation(
        &self,
        file_id: u64,
        file_size: u64,
        block_size: u64,
        total_blocks: u64,
        truncate_pending: bool,
        blocks: &[PersistBlockRow],
        maintain_copy_crc_table: bool,
        capacity_reservation_token: Option<&str>,
    ) -> Result<(), String> {
        let mut payload_guard =
            self.payload_persist_guard(persist_block_input_bytes(blocks), blocks.len() as u64);
        let result = self.with_cached_connection(|conn| unsafe {
            let transaction_started = Instant::now();
            let result = transactional_replayable(conn, |conn| {
                if capacity_reservation_token.is_some() {
                    let quota_lock = self.lock_payload_quota_on_conn(conn)?;
                    self.refresh_payload_reservation_on_conn(
                        conn,
                        quota_lock.quota_bytes(),
                        capacity_reservation_token,
                    )?;
                    self.persist_file_blocks_rows_on_conn(
                        conn,
                        file_id,
                        file_size,
                        block_size,
                        total_blocks,
                        truncate_pending,
                        blocks,
                        maintain_copy_crc_table,
                    )?;
                    return self.enforce_payload_quota_on_conn(
                        conn,
                        &quota_lock,
                        capacity_reservation_token,
                    );
                }

                self.persist_file_blocks_rows_on_conn(
                    conn,
                    file_id,
                    file_size,
                    block_size,
                    total_blocks,
                    truncate_pending,
                    blocks,
                    maintain_copy_crc_table,
                )?;
                // FOD connection setup pins READ COMMITTED; the final quota
                // query relies on seeing writers committed while this
                // transaction waited for the advisory lock.
                let quota_lock = self.lock_payload_quota_on_conn(conn)?;
                self.enforce_payload_quota_on_conn(conn, &quota_lock, None)
            });
            self.record_persist_transaction_elapsed(transaction_started.elapsed(), result.is_err());
            result
        });
        if result.is_ok() {
            payload_guard.mark_success();
        }
        result
    }

    unsafe fn persist_file_blocks_rows_on_conn<'a>(
        &self,
        conn: *mut PGconn,
        file_id: u64,
        file_size: u64,
        block_size: u64,
        total_blocks: u64,
        truncate_pending: bool,
        blocks: &[PersistBlockRow<'a>],
        maintain_copy_crc_table: bool,
    ) -> Result<(), String> {
        match self.persist_block_transport {
            PersistBlockTransport::CopyBinaryStaging => self
                .persist_file_blocks_copy_binary_staging_on_conn(
                    conn,
                    file_id,
                    file_size,
                    block_size,
                    total_blocks,
                    truncate_pending,
                    blocks,
                    maintain_copy_crc_table,
                ),
            PersistBlockTransport::BinaryBytea | PersistBlockTransport::LegacyHex => self
                .persist_file_blocks_direct_on_conn(
                    conn,
                    file_id,
                    file_size,
                    block_size,
                    total_blocks,
                    truncate_pending,
                    blocks,
                    maintain_copy_crc_table,
                    self.persist_block_transport,
                ),
        }
    }

    unsafe fn lock_payload_quota_on_conn(
        &self,
        conn: *mut PGconn,
    ) -> Result<PayloadQuotaLockObservation, String> {
        let sql_lock = CString::new("SELECT pg_advisory_xact_lock(4607812, 1)")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let lock_started = Instant::now();
        query_scalar_text_on_conn(conn, &sql_lock)?;
        let lock_acquired_at = Instant::now();
        self.record_quota_lock_wait_elapsed(lock_acquired_at.duration_since(lock_started));
        let mut observation = PayloadQuotaLockObservation::new(
            Arc::clone(&self.payload_observability),
            Arc::clone(&self.global_payload_observability),
            lock_acquired_at,
        );

        let sql_limit =
            CString::new("SELECT value::text FROM config WHERE key = 'max_fs_size_bytes'")
                .map_err(|_| "SQL contains NUL byte".to_string())?;
        let value = query_scalar_text_on_conn(conn, &sql_limit)?;
        let limit = value
            .trim()
            .parse::<u64>()
            .map_err(|_| "invalid config.max_fs_size_bytes value".to_string())?;
        observation.set_quota_bytes((limit > 0).then_some(limit));
        Ok(observation)
    }

    unsafe fn persisted_and_reserved_payload_bytes_on_conn(
        &self,
        conn: *mut PGconn,
        excluded_reservation_token: Option<&str>,
    ) -> Result<u64, String> {
        let excluded_reservation_token = CString::new(excluded_reservation_token.unwrap_or(""))
            .map_err(|_| "capacity reservation token contains NUL byte".to_string())?;
        let sql_used = CString::new(
            "SELECT (\
                COALESCE((SELECT COUNT(*)::numeric FROM data_blocks), 0) \
                    * (SELECT value::numeric FROM config WHERE key = 'block_size') \
                + COALESCE((SELECT SUM(reserved_bytes)::numeric \
                            FROM payload_capacity_reservations \
                            WHERE expires_at > NOW() \
                              AND ($1 = '' OR request_token <> $1)), 0)\
             )::text",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let res = exec_params(conn, &sql_used, &[&excluded_reservation_token])?;
        fetch_single_text(res)?
            .trim()
            .parse::<u64>()
            .map_err(|_| "invalid persisted and reserved payload byte count".to_string())
    }

    unsafe fn refresh_payload_reservation_on_conn(
        &self,
        conn: *mut PGconn,
        quota_bytes: Option<u64>,
        request_token: Option<&str>,
    ) -> Result<(), String> {
        let Some(request_token) = request_token else {
            return Ok(());
        };
        let request_token_text = request_token;
        let request_token = CString::new(request_token)
            .map_err(|_| "capacity reservation token contains NUL byte".to_string())?;
        let sql = CString::new(
            "SELECT reserved_bytes::text FROM payload_capacity_reservations \
             WHERE request_token = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let res = exec_params(conn, &sql, &[&request_token])?;
        let reserved_bytes = fetch_single_text_option(res)?
            .ok_or_else(|| "payload capacity reservation is missing".to_string())?
            .trim()
            .parse::<u64>()
            .map_err(|_| "invalid payload capacity reservation".to_string())?;
        if let Some(quota_bytes) = quota_bytes {
            let committed_and_reserved =
                self.persisted_and_reserved_payload_bytes_on_conn(conn, Some(request_token_text))?;
            if committed_and_reserved.saturating_add(reserved_bytes) > quota_bytes {
                return Err(format!(
                    "{} copy reservation refresh requires {} reserved bytes, {} bytes are already committed or reserved, limit is {} bytes",
                    STORAGE_QUOTA_EXCEEDED_PREFIX,
                    reserved_bytes,
                    committed_and_reserved,
                    quota_bytes
                ));
            }
        }
        let sql_refresh = CString::new(
            "UPDATE payload_capacity_reservations \
             SET expires_at = NOW() + INTERVAL '1 hour' \
             WHERE request_token = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        exec_command_params(conn, &sql_refresh, &[&request_token])
    }

    unsafe fn enforce_payload_quota_on_conn(
        &self,
        conn: *mut PGconn,
        quota_lock: &PayloadQuotaLockObservation,
        excluded_reservation_token: Option<&str>,
    ) -> Result<(), String> {
        let quota_bytes = quota_lock.quota_bytes();
        let Some(quota_bytes) = quota_bytes else {
            return Ok(());
        };
        let check_started = Instant::now();
        let used_result =
            self.persisted_and_reserved_payload_bytes_on_conn(conn, excluded_reservation_token);
        self.record_quota_final_check_elapsed(check_started.elapsed());
        let used_bytes = used_result?;
        if used_bytes > quota_bytes {
            return Err(format!(
                "{} persisted payload would use {} bytes, limit is {} bytes",
                STORAGE_QUOTA_EXCEEDED_PREFIX, used_bytes, quota_bytes
            ));
        }
        Ok(())
    }

    pub fn reserve_payload_capacity(&self, reserved_bytes: u64) -> Result<Option<String>, String> {
        if reserved_bytes == 0 {
            return Ok(None);
        }
        let request_token = generate_request_token("payload-capacity");
        let request_token_param = CString::new(request_token.as_str())
            .map_err(|_| "capacity reservation token contains NUL byte".to_string())?;
        let reserved_bytes_param = CString::new(reserved_bytes.to_string())
            .map_err(|_| "reserved byte count contains NUL byte".to_string())?;
        let sql_delete_expired =
            CString::new("DELETE FROM payload_capacity_reservations WHERE expires_at <= NOW()")
                .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_insert = CString::new(
            "INSERT INTO payload_capacity_reservations \
                (request_token, reserved_bytes, created_at, expires_at) \
             VALUES ($1, $2, NOW(), NOW() + INTERVAL '1 hour') \
             ON CONFLICT (request_token) DO UPDATE SET \
                reserved_bytes = EXCLUDED.reserved_bytes, \
                expires_at = EXCLUDED.expires_at",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let quota_lock = self.lock_payload_quota_on_conn(conn)?;
                if quota_lock.quota_bytes().is_none() {
                    return Ok(None);
                }
                exec_command(conn, &sql_delete_expired)?;
                let committed_and_reserved =
                    self.persisted_and_reserved_payload_bytes_on_conn(conn, None)?;
                let quota_bytes = quota_lock.quota_bytes().unwrap_or(0);
                if committed_and_reserved.saturating_add(reserved_bytes) > quota_bytes {
                    return Err(format!(
                        "{} copy requires {} reserved bytes, {} bytes are already committed or reserved, limit is {} bytes",
                        STORAGE_QUOTA_EXCEEDED_PREFIX,
                        reserved_bytes,
                        committed_and_reserved,
                        quota_bytes
                    ));
                }
                exec_command_params(
                    conn,
                    &sql_insert,
                    &[&request_token_param, &reserved_bytes_param],
                )?;
                Ok(Some(request_token.clone()))
            })
        })
    }

    pub fn release_payload_capacity_reservation(&self, request_token: &str) -> Result<(), String> {
        let request_token = CString::new(request_token)
            .map_err(|_| "capacity reservation token contains NUL byte".to_string())?;
        let sql =
            CString::new("DELETE FROM payload_capacity_reservations WHERE request_token = $1")
                .map_err(|_| "SQL contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                exec_command_params(conn, &sql, &[&request_token])
            })
        })
    }

    pub fn adopt_source_data_object(
        &self,
        src_file_id: u64,
        dst_file_id: u64,
    ) -> Result<bool, String> {
        let sql_file_info =
            CString::new("SELECT size, data_object_id FROM files WHERE id_file = $1")
                .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_touch_src_object = CString::new(
            "UPDATE data_objects SET reference_count = reference_count + 1, modification_date = NOW() WHERE id_data_object = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_update_dst_file = CString::new(
            "UPDATE files SET data_object_id = $1, size = $2, change_date = NOW(), modification_date = NOW() WHERE id_file = $3",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_count_dst_references =
            CString::new("SELECT COUNT(*) FROM files WHERE data_object_id = $1")
                .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_delete_data_object =
            CString::new("DELETE FROM data_objects WHERE id_data_object = $1")
                .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_touch_dst_object = CString::new(
            "UPDATE data_objects SET reference_count = GREATEST(reference_count - 1, 0), modification_date = NOW() WHERE id_data_object = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let fetch_file_info =
            |conn: *mut PGconn, file_id: &CString| -> Result<Option<(u64, u64)>, String> {
                unsafe {
                    let params = [file_id];
                    let res = exec_params(conn, &sql_file_info, &params)?;
                    let info = match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 2 {
                                None
                            } else {
                                let size_ptr = PQgetvalue(res, 0, 0);
                                let data_object_ptr = PQgetvalue(res, 0, 1);
                                if size_ptr.is_null() || data_object_ptr.is_null() {
                                    None
                                } else {
                                    let size = CStr::from_ptr(size_ptr)
                                        .to_string_lossy()
                                        .trim()
                                        .parse::<u64>()
                                        .ok();
                                    let data_object_id = CStr::from_ptr(data_object_ptr)
                                        .to_string_lossy()
                                        .trim()
                                        .parse::<u64>()
                                        .ok();
                                    match (size, data_object_id) {
                                        (Some(size), Some(data_object_id)) => {
                                            Some((size, data_object_id))
                                        }
                                        _ => None,
                                    }
                                }
                            };
                            PQclear(res);
                            value
                        }
                        _ => {
                            PQclear(res);
                            return Err(conn_error(conn));
                        }
                    };
                    Ok(info)
                }
            };
        let src_file_id = CString::new(src_file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let dst_file_id = CString::new(dst_file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            transactional_replay_confirmed(
                conn,
                |conn| {
                    let src_info = fetch_file_info(conn, &src_file_id)?;
                    let (src_size, src_data_object_id) = match src_info {
                        Some(value) => value,
                        None => return Ok(None),
                    };
                    if src_size == 0 {
                        return Ok(None);
                    }

                    let dst_info = fetch_file_info(conn, &dst_file_id)?;
                    let (dst_size, dst_data_object_id) = match dst_info {
                        Some(value) => value,
                        None => return Ok(None),
                    };
                    if dst_size == src_size && src_data_object_id == dst_data_object_id {
                        return Ok(Some(true));
                    }
                    Ok(None)
                },
                |conn| {
                    let src_info = fetch_file_info(conn, &src_file_id)?;
                    let (src_size, src_data_object_id) = match src_info {
                        Some(value) => value,
                        None => return Ok(false),
                    };
                    if src_size == 0 {
                        return Ok(false);
                    }

                    let dst_info = fetch_file_info(conn, &dst_file_id)?;
                    let (dst_size, dst_data_object_id) = match dst_info {
                        Some(value) => value,
                        None => return Ok(false),
                    };
                    if dst_size == src_size && src_data_object_id == dst_data_object_id {
                        return Ok(true);
                    }
                    if dst_size != 0 || src_data_object_id == dst_data_object_id {
                        return Ok(false);
                    }

                    let src_data_object_id = CString::new(src_data_object_id.to_string())
                        .map_err(|_| "data object id contains NUL byte".to_string())?;
                    let dst_data_object_id = CString::new(dst_data_object_id.to_string())
                        .map_err(|_| "data object id contains NUL byte".to_string())?;
                    let src_size_text = CString::new(src_size.to_string())
                        .map_err(|_| "file size contains NUL byte".to_string())?;

                    let params = [&src_data_object_id];
                    exec_command_params(conn, &sql_touch_src_object, &params)?;

                    let params = [&src_data_object_id, &src_size_text, &dst_file_id];
                    exec_command_params(conn, &sql_update_dst_file, &params)?;

                    let params = [&dst_data_object_id];
                    let res = exec_params(conn, &sql_count_dst_references, &params)?;
                    let remaining_dst_references =
                        fetch_single_text(res)?.trim().parse::<u64>().unwrap_or(0);

                    if remaining_dst_references == 0 {
                        let params = [&dst_data_object_id];
                        exec_command_params(conn, &sql_delete_data_object, &params)?;
                    } else {
                        let params = [&dst_data_object_id];
                        exec_command_params(conn, &sql_touch_dst_object, &params)?;
                    }

                    Ok(true)
                },
            )
        })
    }

    pub fn set_file_size(&self, file_id: u64, file_size: u64) -> Result<(), String> {
        let desired_file_size = file_size;
        let sql_update_file = CString::new(
            "UPDATE files SET size = $1, modification_date = NOW(), change_date = NOW() WHERE id_file = $2",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_lookup_size = CString::new("SELECT size FROM files WHERE id_file = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let file_size = CString::new(desired_file_size.to_string())
            .map_err(|_| "file size contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            transactional_replay_confirmed(
                conn,
                |conn| {
                    let params = [&file_id];
                    let res = exec_params(conn, &sql_lookup_size, &params)?;
                    match fetch_single_text_option(res)? {
                        Some(current_size) => {
                            let current_size = current_size
                                .trim()
                                .parse::<u64>()
                                .map_err(|_| "invalid file size value".to_string())?;
                            if current_size == desired_file_size {
                                Ok(Some(()))
                            } else {
                                Ok(None)
                            }
                        }
                        None => Ok(None),
                    }
                },
                |conn| {
                    let params = [&file_size, &file_id];
                    exec_command_params(conn, &sql_update_file, &params)?;
                    Ok(())
                },
            )
        })
    }

    pub fn purge_primary_file(&self, file_id: u64) -> Result<(), String> {
        let sql_lookup = CString::new("SELECT data_object_id FROM files WHERE id_file = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_delete_file = CString::new("DELETE FROM files WHERE id_file = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_delete_data_object =
            CString::new("DELETE FROM data_objects WHERE id_data_object = $1")
                .map_err(|_| "SQL contains NUL byte".to_string())?;
        let sql_touch_data_object = CString::new(
            "UPDATE data_objects SET reference_count = GREATEST(reference_count - 1, 0), modification_date = NOW() WHERE id_data_object = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            // A committed purge is observable because the file row disappears,
            // so a lost COMMIT can be confirmed by the empty lookup before the
            // body runs again.
            transactional_replay_confirmed(
                conn,
                |conn| {
                    let params = [&file_id];
                    let res = exec_params(conn, &sql_lookup, &params)?;
                    match fetch_single_text_option(res)? {
                        Some(_) => Ok(None),
                        None => Ok(Some(())),
                    }
                },
                |conn| {
                    let data_object_id = match {
                        let params = [&file_id];
                        let res = exec_params(conn, &sql_lookup, &params)?;
                        let text = fetch_single_text(res)?;
                        if text.is_empty() {
                            None
                        } else {
                            Some(
                                text.trim()
                                    .parse::<u64>()
                                    .map_err(|_| "invalid data_object_id value".to_string())?,
                            )
                        }
                    } {
                        Some(value) => value,
                        None => {
                            let params = [&file_id];
                            exec_command_params(conn, &sql_delete_file, &params)?;
                            return Ok(());
                        }
                    };
                    let reference_count = self
                        .data_object_reference_count_on_conn(conn, data_object_id)?
                        .unwrap_or(1);
                    let data_object_id = CString::new(data_object_id.to_string())
                        .map_err(|_| "data object id contains NUL byte".to_string())?;
                    if reference_count <= 1 {
                        let params = [&file_id];
                        exec_command_params(conn, &sql_delete_file, &params)?;
                        let params = [&data_object_id];
                        exec_command_params(conn, &sql_delete_data_object, &params)?;
                    } else {
                        let params = [&data_object_id];
                        exec_command_params(conn, &sql_touch_data_object, &params)?;
                        let params = [&file_id];
                        exec_command_params(conn, &sql_delete_file, &params)?;
                    }
                    Ok(())
                },
            )
        })
    }

    pub fn count_file_links(&self, file_id: u64) -> Result<u64, String> {
        let sql = CString::new("SELECT 1 + COUNT(*) FROM hardlinks WHERE id_file = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id];
            let res = exec_params(conn, &sql, &params)?;
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            let value = if rows < 1 || cols < 1 {
                0
            } else {
                let value_ptr = PQgetvalue(res, 0, 0);
                if value_ptr.is_null() {
                    0
                } else {
                    CStr::from_ptr(value_ptr)
                        .to_string_lossy()
                        .trim()
                        .parse::<u64>()
                        .unwrap_or(0)
                }
            };
            PQclear(res);
            Ok(value)
        })
    }

    pub fn count_file_blocks(&self, file_id: u64) -> Result<u64, String> {
        let sql_blocks = CString::new(
            "
            SELECT COUNT(*)
            FROM data_blocks db
            JOIN files f ON f.data_object_id = db.data_object_id
            WHERE f.id_file = $1
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id];
            let res = exec_params(conn, &sql_blocks, &params)?;
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            let value = if rows < 1 || cols < 1 {
                0
            } else {
                let value_ptr = PQgetvalue(res, 0, 0);
                if value_ptr.is_null() {
                    0
                } else {
                    CStr::from_ptr(value_ptr)
                        .to_string_lossy()
                        .trim()
                        .parse::<u64>()
                        .unwrap_or(0)
                }
            };
            PQclear(res);
            Ok(value)
        })
    }

    pub fn path_has_children(&self, directory_id: u64) -> Result<bool, String> {
        let sql = CString::new(
            "
            SELECT 1
            FROM files
            WHERE id_directory = $1
            UNION ALL
            SELECT 1
            FROM directories
            WHERE id_parent = $1
            UNION ALL
            SELECT 1
            FROM hardlinks
            WHERE id_directory = $1
            UNION ALL
            SELECT 1
            FROM symlinks
            WHERE id_parent = $1
            LIMIT 1
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&directory_id];
            let res = exec_params(conn, &sql, &params)?;
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            let value = rows >= 1 && cols >= 1;
            PQclear(res);
            Ok(value)
        })
    }

    pub fn count_directory_children(&self, directory_id: u64) -> Result<u64, String> {
        let sql = CString::new(
            "
            SELECT
                (SELECT COUNT(*) FROM directories WHERE id_parent = $1)
              + (SELECT COUNT(*) FROM files WHERE id_directory = $1)
              + (SELECT COUNT(*) FROM hardlinks WHERE id_directory = $1)
              + (SELECT COUNT(*) FROM symlinks WHERE id_parent = $1)
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&directory_id];
            let res = exec_params(conn, &sql, &params)?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid directory children count".to_string())?;
            Ok(value)
        })
    }

    pub fn count_directory_subdirs(&self, directory_id: u64) -> Result<u64, String> {
        let sql = CString::new("SELECT COUNT(*) FROM directories WHERE id_parent = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&directory_id];
            let res = exec_params(conn, &sql, &params)?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid directory subdir count".to_string())?;
            Ok(value)
        })
    }

    pub fn count_root_directory_children(&self) -> Result<u64, String> {
        let sql = CString::new(
            "
            SELECT COUNT(*)
            FROM directories
            WHERE id_parent IS NULL AND name != '/'
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = exec_params(conn, &sql, &[])?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid root directory children count".to_string())?;
            Ok(value)
        })
    }

    pub fn count_symlinks(&self) -> Result<u64, String> {
        let sql = CString::new("SELECT COUNT(*) FROM symlinks")
            .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = exec_params(conn, &sql, &[])?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid symlink count".to_string())?;
            Ok(value)
        })
    }

    pub fn count_files(&self) -> Result<u64, String> {
        let sql = CString::new("SELECT COUNT(*) FROM files")
            .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = exec_params(conn, &sql, &[])?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid file count".to_string())?;
            Ok(value)
        })
    }

    pub fn count_directories(&self) -> Result<u64, String> {
        let sql = CString::new("SELECT COUNT(*) FROM directories")
            .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = exec_params(conn, &sql, &[])?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid directory count".to_string())?;
            Ok(value)
        })
    }

    pub fn total_data_size(&self) -> Result<u64, String> {
        let sql = CString::new("SELECT COALESCE(SUM(LENGTH(data)), 0) FROM data_blocks")
            .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = exec_params(conn, &sql, &[])?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid total data size".to_string())?;
            Ok(value)
        })
    }

    pub fn statfs_snapshot(&self) -> Result<(u64, u64, u64, u64, u64, Option<u64>), String> {
        self.with_read_connection(|conn| unsafe {
            let res = exec_prepared_params(conn, PreparedStatement::StatfsSnapshot, &[])?;
            let values = fetch_first_row_texts(res)?;
            if values.len() < 6 {
                return Err("invalid statfs snapshot".to_string());
            }
            let files = values[0]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid file count".to_string())?;
            let dirs = values[1]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid directory count".to_string())?;
            let symlinks = values[2]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid symlink count".to_string())?;
            let total_data_size = values[3]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid total data size".to_string())?;
            let reserved_data_size = values[4]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid reserved data size".to_string())?;
            let max_fs_size_bytes = values[5]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid max filesystem size".to_string())?;
            Ok((
                files,
                dirs,
                symlinks,
                total_data_size,
                reserved_data_size,
                (max_fs_size_bytes > 0).then_some(max_fs_size_bytes),
            ))
        })
    }

    pub fn load_symlink_target(&self, symlink_id: u64) -> Result<Option<String>, String> {
        let symlink_id = CString::new(symlink_id.to_string())
            .map_err(|_| "symlink id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&symlink_id];
            let res = exec_prepared_params(conn, PreparedStatement::LoadSymlinkTarget, &params)?;
            let text = fetch_single_text(res)?;
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        })
    }

    pub fn get_special_file_metadata(
        &self,
        file_id: u64,
    ) -> Result<Option<(String, u32, u32)>, String> {
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id];
            let res =
                exec_prepared_params(conn, PreparedStatement::GetSpecialFileMetadata, &params)?;
            let result = {
                if PQresultStatus(res) != PGRES_TUPLES_OK {
                    PQclear(res);
                    return Err(conn_error(conn));
                }
                let rows = PQntuples(res);
                let cols = PQnfields(res);
                if rows < 1 || cols < 3 {
                    PQclear(res);
                    return Ok(None);
                }
                let file_type_ptr = PQgetvalue(res, 0, 0);
                let major_ptr = PQgetvalue(res, 0, 1);
                let minor_ptr = PQgetvalue(res, 0, 2);
                if file_type_ptr.is_null() || major_ptr.is_null() || minor_ptr.is_null() {
                    PQclear(res);
                    return Ok(None);
                }
                let file_type = CStr::from_ptr(file_type_ptr).to_string_lossy().to_string();
                let rdev_major = CStr::from_ptr(major_ptr)
                    .to_string_lossy()
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| "invalid special file major".to_string())?;
                let rdev_minor = CStr::from_ptr(minor_ptr)
                    .to_string_lossy()
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| "invalid special file minor".to_string())?;
                PQclear(res);
                Ok(Some((file_type, rdev_major, rdev_minor)))
            };
            result
        })
    }

    pub fn get_symlink_id(&self, path: &str) -> Result<Option<u64>, String> {
        let normalized = path.trim();
        let (parent_path, link_name) = match normalized.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => {
                (if parent.is_empty() { "/" } else { parent }, name)
            }
            _ => ("/", normalized),
        };
        let parent_id = self.get_dir_id(parent_path)?;
        let link_name =
            CString::new(link_name).map_err(|_| "path contains NUL byte".to_string())?;
        let parent_id_text = parent_id
            .map(|value| {
                CString::new(value.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())
            })
            .transpose()?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let res = if let Some(ref parent_id_text) = parent_id_text {
                    let params = [&link_name, parent_id_text];
                    exec_prepared_params(conn, PreparedStatement::GetSymlinkIdNested, &params)
                } else {
                    let params = [&link_name];
                    exec_prepared_params(conn, PreparedStatement::GetSymlinkIdRoot, &params)
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 1 {
                                None
                            } else {
                                let value_ptr = PQgetvalue(res, 0, 0);
                                if value_ptr.is_null() {
                                    None
                                } else {
                                    let value =
                                        CStr::from_ptr(value_ptr).to_string_lossy().to_string();
                                    value.trim().parse::<u64>().ok()
                                }
                            };
                            PQclear(res);
                            Ok(value)
                        }
                        _ => {
                            PQclear(res);
                            Err(conn_error(conn))
                        }
                    }
                }
            };
            result
        })
    }

    pub fn resolve_path(&self, path: &str) -> Result<ResolvedPath, String> {
        let normalized = path.trim();
        if normalized.is_empty() {
            return Ok(ResolvedPath {
                parent_id: None,
                kind: None,
                entry_id: None,
            });
        }

        let (parent_path, name) = match normalized.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => {
                (if parent.is_empty() { "/" } else { parent }, name)
            }
            _ => ("/", normalized),
        };
        let parent_id = self.get_dir_id(parent_path)?;

        let name = CString::new(name).map_err(|_| "path contains NUL byte".to_string())?;
        let parent_id_text = parent_id
            .map(|value| {
                CString::new(value.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())
            })
            .transpose()?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let res = if let Some(ref parent_id_text) = parent_id_text {
                    let params = [&name, parent_id_text];
                    exec_prepared_params(conn, PreparedStatement::ResolvePathNested, &params)
                } else {
                    let params = [&name];
                    exec_prepared_params(conn, PreparedStatement::ResolvePathRoot, &params)
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 2 {
                                None
                            } else {
                                let kind_ptr = PQgetvalue(res, 0, 0);
                                let entry_ptr = PQgetvalue(res, 0, 1);
                                if kind_ptr.is_null() || entry_ptr.is_null() {
                                    None
                                } else {
                                    let kind =
                                        CStr::from_ptr(kind_ptr).to_string_lossy().to_string();
                                    let entry_id = CStr::from_ptr(entry_ptr)
                                        .to_string_lossy()
                                        .parse::<u64>()
                                        .ok();
                                    entry_id.map(|entry_id| (kind, entry_id))
                                }
                            };
                            PQclear(res);
                            Ok(value)
                        }
                        _ => {
                            PQclear(res);
                            Err(conn_error(conn))
                        }
                    }
                }
            };
            result.map(|entry| ResolvedPath {
                parent_id,
                kind: entry.as_ref().map(|(kind, _)| kind.clone()),
                entry_id: entry.map(|(_, entry_id)| entry_id),
            })
        })
    }

    pub fn fetch_xattr_value(&self, path: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
        let resolved = self.resolve_path(path)?;
        let (owner_kind, owner_id) = match resolved.kind.as_deref() {
            Some("hardlink") => {
                let file_id = self.get_file_id(path)?;
                match file_id {
                    Some(file_id) => ("file".to_string(), file_id),
                    None => return Ok(None),
                }
            }
            Some("file") => match resolved.entry_id {
                Some(entry_id) => ("file".to_string(), entry_id),
                None => return Ok(None),
            },
            Some("dir") => ("dir".to_string(), resolved.entry_id.unwrap_or(0)),
            Some("symlink") => match resolved.entry_id {
                Some(entry_id) => ("symlink".to_string(), entry_id),
                None => return Ok(None),
            },
            _ => return Ok(None),
        };

        let sql = CString::new(
            "SELECT encode(value, 'base64') FROM xattrs WHERE owner_kind = $1 AND owner_id = $2 AND name = $3",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let owner_kind =
            CString::new(owner_kind).map_err(|_| "owner kind contains NUL byte".to_string())?;
        let owner_id = CString::new(owner_id.to_string())
            .map_err(|_| "owner id contains NUL byte".to_string())?;
        let name = CString::new(name).map_err(|_| "xattr name contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&owner_kind, &owner_id, &name];
            let res = exec_params(conn, &sql, &params)?;
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            let encoded = if rows < 1 || cols < 1 {
                None
            } else {
                let value_ptr = PQgetvalue(res, 0, 0);
                if value_ptr.is_null() {
                    None
                } else {
                    Some(CStr::from_ptr(value_ptr).to_string_lossy().to_string())
                }
            };
            PQclear(res);
            let value = match encoded {
                None => None,
                Some(encoded) => {
                    let decoded = BASE64_STANDARD.decode(encoded.trim()).map_err(|err| {
                        format!("invalid xattr payload returned by PostgreSQL: {err}")
                    })?;
                    Some(decoded)
                }
            };
            Ok(value)
        })
    }

    pub fn list_xattr_names(&self, path: &str) -> Result<Option<Vec<String>>, String> {
        let resolved = self.resolve_path(path)?;
        let (owner_kind, owner_id) = match resolved.kind.as_deref() {
            Some("hardlink") => {
                let file_id = self.get_file_id(path)?;
                match file_id {
                    Some(file_id) => ("file".to_string(), file_id),
                    None => return Ok(None),
                }
            }
            Some("file") => match resolved.entry_id {
                Some(entry_id) => ("file".to_string(), entry_id),
                None => return Ok(None),
            },
            Some("dir") => ("dir".to_string(), resolved.entry_id.unwrap_or(0)),
            Some("symlink") => match resolved.entry_id {
                Some(entry_id) => ("symlink".to_string(), entry_id),
                None => return Ok(None),
            },
            _ => return Ok(None),
        };

        let sql = CString::new(
            "SELECT name FROM xattrs WHERE owner_kind = $1 AND owner_id = $2 ORDER BY name",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let owner_kind =
            CString::new(owner_kind).map_err(|_| "owner kind contains NUL byte".to_string())?;
        let owner_id = CString::new(owner_id.to_string())
            .map_err(|_| "owner id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&owner_kind, &owner_id];
            let res = exec_params(conn, &sql, &params)?;
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            let mut names = Vec::with_capacity(rows.max(0) as usize);
            if rows >= 1 && cols >= 1 {
                for row in 0..rows {
                    let value_ptr = PQgetvalue(res, row, 0);
                    if !value_ptr.is_null() {
                        names.push(CStr::from_ptr(value_ptr).to_string_lossy().to_string());
                    }
                }
            }
            PQclear(res);
            Ok(Some(names))
        })
    }

    pub fn store_xattr_value_for_owner(
        &self,
        owner_kind: &str,
        owner_id: u64,
        name: &str,
        value: &[u8],
    ) -> Result<(), String> {
        let sql = CString::new(
            "
            INSERT INTO xattrs (owner_kind, owner_id, name, value)
            VALUES ($1, $2, $3, decode($4, 'base64'))
            ON CONFLICT (owner_kind, owner_id, name) DO UPDATE
            SET value = EXCLUDED.value
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let owner_kind =
            CString::new(owner_kind).map_err(|_| "owner kind contains NUL byte".to_string())?;
        let owner_id = CString::new(owner_id.to_string())
            .map_err(|_| "owner id contains NUL byte".to_string())?;
        let name = CString::new(name).map_err(|_| "xattr name contains NUL byte".to_string())?;
        let value_b64 = CString::new(BASE64_STANDARD.encode(value))
            .map_err(|_| "xattr value contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            let params = [&owner_kind, &owner_id, &name, &value_b64];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn delete_owner_xattrs(&self, owner_kind: &str, owner_id: u64) -> Result<(), String> {
        let sql = CString::new("DELETE FROM xattrs WHERE owner_kind = $1 AND owner_id = $2")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let owner_kind =
            CString::new(owner_kind).map_err(|_| "owner kind contains NUL byte".to_string())?;
        let owner_id = CString::new(owner_id.to_string())
            .map_err(|_| "owner id contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            let params = [&owner_kind, &owner_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn remove_xattr_for_owner(
        &self,
        owner_kind: &str,
        owner_id: u64,
        name: &str,
    ) -> Result<u64, String> {
        let sql = CString::new(
            "DELETE FROM xattrs WHERE owner_kind = $1 AND owner_id = $2 AND name = $3 RETURNING 1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let owner_kind =
            CString::new(owner_kind).map_err(|_| "owner kind contains NUL byte".to_string())?;
        let owner_id = CString::new(owner_id.to_string())
            .map_err(|_| "owner id contains NUL byte".to_string())?;
        let name = CString::new(name).map_err(|_| "xattr name contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            let params = [&owner_kind, &owner_id, &name];
            let res = exec_params(conn, &sql, &params)?;
            if PQresultStatus(res) != PGRES_TUPLES_OK {
                let error = result_error(res);
                PQclear(res);
                if sql_is_replayable_command(&sql) && is_retryable_connection_error(conn, &error) {
                    return Err(replayable_sql_error(error));
                }
                return Err(error);
            }
            let rows = PQntuples(res);
            PQclear(res);
            Ok(rows as u64)
        })
    }

    pub fn list_directory_entries_blob(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let normalized = path.trim();
        let parent_id = self.get_dir_id(normalized)?;
        let (sql, params) = if let Some(parent_id) = parent_id {
            (
                CString::new(
                    "
                    SELECT name FROM files WHERE id_directory = $1
                    UNION ALL
                    SELECT name FROM hardlinks WHERE id_directory = $1
                    UNION ALL
                    SELECT name FROM directories WHERE id_parent = $1
                    UNION ALL
                    SELECT name FROM symlinks WHERE id_parent = $1
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![CString::new(parent_id.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())?],
            )
        } else {
            (
                CString::new(
                    "
                    SELECT name FROM directories WHERE id_parent IS NULL AND name != '/'
                    UNION ALL
                    SELECT name FROM files WHERE id_directory IS NULL
                    UNION ALL
                    SELECT name FROM hardlinks WHERE id_directory IS NULL
                    UNION ALL
                    SELECT name FROM symlinks WHERE id_parent IS NULL
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?,
                Vec::new(),
            )
        };

        self.with_read_connection(|conn| unsafe {
            let param_refs = params.iter().collect::<Vec<_>>();
            let res = exec_params(conn, &sql, &param_refs)?;
            let names = fetch_first_column_texts(res)?;
            Ok(Some(join_nul_text(&names)))
        })
    }

    pub fn fetch_path_attrs_blob(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let resolved = self.resolve_path(path)?;
        let kind = match resolved.kind.as_deref() {
            Some(kind) => kind,
            None => return Ok(None),
        };
        let entry_id = match resolved.entry_id {
            Some(entry_id) => entry_id,
            None => return Ok(None),
        };
        let entry_id = CString::new(entry_id.to_string())
            .map_err(|_| "entry id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = {
                let res = match kind {
                    "file" => {
                        let params = [&entry_id];
                        exec_prepared_params(
                            conn,
                            PreparedStatement::FetchPathAttrsBlobFile,
                            &params,
                        )
                    }
                    "dir" => {
                        let params = [&entry_id];
                        exec_prepared_params(
                            conn,
                            PreparedStatement::FetchPathAttrsBlobDir,
                            &params,
                        )
                    }
                    "symlink" => {
                        let params = [&entry_id];
                        exec_prepared_params(
                            conn,
                            PreparedStatement::FetchPathAttrsBlobSymlink,
                            &params,
                        )
                    }
                    "hardlink" => {
                        let params = [&entry_id];
                        exec_prepared_params(
                            conn,
                            PreparedStatement::FetchPathAttrsBlobHardlink,
                            &params,
                        )
                    }
                    _ => return Ok(None),
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let row = fetch_first_row_texts(res)?;
                            if row.is_empty() {
                                Ok(None)
                            } else {
                                let mut output = Vec::new();
                                output.extend_from_slice(kind.as_bytes());
                                output.push(0);
                                output.extend_from_slice(&join_nul_text(&row));
                                Ok(Some(output))
                            }
                        }
                        _ => {
                            PQclear(res);
                            Err(conn_error(conn))
                        }
                    }
                }
            };
            res
        })
    }

    pub fn update_file_mode(&self, file_id: u64, mode: &str) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE files SET mode = $1, change_date = NOW(), modification_date = NOW() WHERE id_file = $2",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let mode = CString::new(mode).map_err(|_| "mode contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&mode, &file_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_directory_mode(&self, directory_id: u64, mode: &str) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE directories SET mode = $1, change_date = NOW(), modification_date = NOW() WHERE id_directory = $2",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let mode = CString::new(mode).map_err(|_| "mode contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&mode, &directory_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_file_owner(
        &self,
        file_id: u64,
        uid: u32,
        gid: u32,
        mode: &str,
    ) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE files SET uid = $1, gid = $2, mode = $3, change_date = NOW(), modification_date = NOW() WHERE id_file = $4",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let uid = CString::new(uid.to_string()).map_err(|_| "uid contains NUL byte".to_string())?;
        let gid = CString::new(gid.to_string()).map_err(|_| "gid contains NUL byte".to_string())?;
        let mode = CString::new(mode).map_err(|_| "mode contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&uid, &gid, &mode, &file_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_directory_owner(
        &self,
        directory_id: u64,
        uid: u32,
        gid: u32,
        mode: &str,
    ) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE directories SET uid = $1, gid = $2, mode = $3, change_date = NOW(), modification_date = NOW() WHERE id_directory = $4",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let uid = CString::new(uid.to_string()).map_err(|_| "uid contains NUL byte".to_string())?;
        let gid = CString::new(gid.to_string()).map_err(|_| "gid contains NUL byte".to_string())?;
        let mode = CString::new(mode).map_err(|_| "mode contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&uid, &gid, &mode, &directory_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_symlink_owner(&self, symlink_id: u64, uid: u32, gid: u32) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE symlinks SET uid = $1, gid = $2, change_date = NOW(), modification_date = NOW() WHERE id_symlink = $3",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let uid = CString::new(uid.to_string()).map_err(|_| "uid contains NUL byte".to_string())?;
        let gid = CString::new(gid.to_string()).map_err(|_| "gid contains NUL byte".to_string())?;
        let symlink_id = CString::new(symlink_id.to_string())
            .map_err(|_| "symlink id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&uid, &gid, &symlink_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_symlink_access_date(&self, symlink_id: u64, atime: &str) -> Result<(), String> {
        let sql = CString::new("UPDATE symlinks SET access_date = $1 WHERE id_symlink = $2")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let atime = CString::new(atime).map_err(|_| "atime contains NUL byte".to_string())?;
        let symlink_id = CString::new(symlink_id.to_string())
            .map_err(|_| "symlink id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&atime, &symlink_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn touch_file_times(&self, file_id: u64, atime: &str, mtime: &str) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE files SET access_date = $1, modification_date = $2, change_date = NOW() WHERE id_file = $3",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let atime = CString::new(atime).map_err(|_| "atime contains NUL byte".to_string())?;
        let mtime = CString::new(mtime).map_err(|_| "mtime contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&atime, &mtime, &file_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn touch_directory_times(
        &self,
        directory_id: u64,
        atime: &str,
        mtime: &str,
    ) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE directories SET access_date = $1, modification_date = $2, change_date = NOW() WHERE id_directory = $3",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let atime = CString::new(atime).map_err(|_| "atime contains NUL byte".to_string())?;
        let mtime = CString::new(mtime).map_err(|_| "mtime contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&atime, &mtime, &directory_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_file_access_date(&self, file_id: u64, atime: &str) -> Result<(), String> {
        let sql = CString::new("UPDATE files SET access_date = $1 WHERE id_file = $2")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let atime = CString::new(atime).map_err(|_| "atime contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&atime, &file_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_directory_access_date(
        &self,
        directory_id: u64,
        atime: &str,
    ) -> Result<(), String> {
        let sql = CString::new("UPDATE directories SET access_date = $1 WHERE id_directory = $2")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let atime = CString::new(atime).map_err(|_| "atime contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&atime, &directory_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn append_journal_event(
        &self,
        id_user: u32,
        directory_id: Option<u64>,
        file_id: Option<u64>,
        action: &str,
    ) -> Result<(), String> {
        let sql = CString::new(
            "INSERT INTO journal (id_user, id_directory, id_file, action, date_time) VALUES ($1, $2, $3, $4, NOW())",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let id_user = CString::new(id_user.to_string())
            .map_err(|_| "user id contains NUL byte".to_string())?;
        let action =
            CString::new(action).map_err(|_| "journal action contains NUL byte".to_string())?;
        let directory_id = directory_id
            .map(|value| {
                CString::new(value.to_string())
                    .map_err(|_| "directory id contains NUL byte".to_string())
            })
            .transpose()?;
        let file_id = file_id
            .map(|value| {
                CString::new(value.to_string()).map_err(|_| "file id contains NUL byte".to_string())
            })
            .transpose()?;

        self.with_cached_connection(|conn| unsafe {
            let (directory_ptr, directory_len) = match directory_id.as_ref() {
                Some(value) => (value.as_ptr(), value.as_bytes().len() as c_int),
                None => (std::ptr::null(), 0),
            };
            let (file_ptr, file_len) = match file_id.as_ref() {
                Some(value) => (value.as_ptr(), value.as_bytes().len() as c_int),
                None => (std::ptr::null(), 0),
            };
            let param_values = [id_user.as_ptr(), directory_ptr, file_ptr, action.as_ptr()];
            let param_lengths = [
                id_user.as_bytes().len() as c_int,
                directory_len,
                file_len,
                action.as_bytes().len() as c_int,
            ];
            let param_formats = [0 as c_int; 4];
            let res = PQexecParams(
                conn,
                sql.as_ptr(),
                4,
                std::ptr::null(),
                param_values.as_ptr(),
                param_lengths.as_ptr(),
                param_formats.as_ptr(),
                0,
            );
            if res.is_null() {
                return Err(conn_error(conn));
            }
            let status = PQresultStatus(res);
            PQclear(res);
            if status == PGRES_COMMAND_OK {
                Ok(())
            } else {
                Err(conn_error(conn))
            }
        })
    }

    pub fn list_xattr_names_for_owner(
        &self,
        owner_kind: &str,
        owner_id: u64,
    ) -> Result<Vec<String>, String> {
        let sql = CString::new(
            "SELECT name FROM xattrs WHERE owner_kind = $1 AND owner_id = $2 ORDER BY name",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let owner_kind =
            CString::new(owner_kind).map_err(|_| "owner kind contains NUL byte".to_string())?;
        let owner_id = CString::new(owner_id.to_string())
            .map_err(|_| "owner id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&owner_kind, &owner_id];
            let res = exec_params(conn, &sql, &params)?;
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            let mut names = Vec::with_capacity(rows.max(0) as usize);
            if rows >= 1 && cols >= 1 {
                for row in 0..rows {
                    let value_ptr = PQgetvalue(res, row, 0);
                    if !value_ptr.is_null() {
                        names.push(CStr::from_ptr(value_ptr).to_string_lossy().to_string());
                    }
                }
            }
            PQclear(res);
            Ok(names)
        })
    }
}

impl LaneObservabilitySource for DbRepo {
    fn observability_snapshot(&self) -> Result<DbRepoObservabilitySnapshot, String> {
        DbRepo::observability_snapshot(self)
    }

    fn postgres_pressure_snapshot(&self) -> Result<PostgresPressureSnapshot, String> {
        DbRepo::postgres_pressure_snapshot(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fod_rust_monitor::DbRepoPayloadObservabilitySnapshot;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;

    const TEST_BLOCK_SIZE: u64 = 4096;

    fn conninfo() -> String {
        let dbname = std::env::var("POSTGRES_DB").unwrap_or_else(|_| "foddbname".to_string());
        let user = std::env::var("POSTGRES_USER").unwrap_or_else(|_| "foduser".to_string());
        let password =
            std::env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "cichosza".to_string());
        let host = std::env::var("POSTGRES_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = std::env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
        format!(
            "host={host} port={port} dbname={dbname} user={user} password={password} connect_timeout=5"
        )
    }

    fn repo_with_global_payload(global: Arc<DbRepoPayloadObservability>) -> DbRepo {
        let runtime = RuntimeConfig::from_runtime_map(&HashMap::new()).unwrap();
        let storage = runtime.storage_settings();
        DbRepo::new_with_tuning(
            &conninfo(),
            ConnectionTuning::from_storage(&storage),
            storage.persist_buffer_chunk_blocks,
            PersistBlockTransport::CopyBinaryStaging,
            storage.data_object_swap_cleanup,
            4,
            RuntimeTransactionSettings {
                write_active_limit: 2,
                control_active_limit: 1,
            },
            Arc::new(DbRepoPayloadObservability::default()),
            global,
        )
        .unwrap()
    }

    fn quoted_list(values: &[String]) -> String {
        values
            .iter()
            .map(|value| DbRepo::quote_literal(value))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn cleanup_files_by_name(repo: &DbRepo, names: &[String]) {
        if names.is_empty() {
            return;
        }
        let names_sql = quoted_list(names);
        let ids = repo
            .query_rows_text(&format!(
                "SELECT data_object_id::text FROM files WHERE name IN ({names_sql})"
            ))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|row| row.into_iter().next())
            .filter(|value| value.chars().all(|ch| ch.is_ascii_digit()))
            .collect::<Vec<_>>();
        let id_list = if ids.is_empty() {
            "NULL".to_string()
        } else {
            ids.join(", ")
        };
        let _ = repo.exec(&format!(
            "
            DELETE FROM copy_block_crc WHERE data_object_id IN ({id_list});
            DELETE FROM data_blocks WHERE data_object_id IN ({id_list});
            DELETE FROM files WHERE name IN ({names_sql});
            DELETE FROM data_objects WHERE id_data_object IN ({id_list});
            "
        ));
    }

    struct TestFilesCleanup {
        repo: DbRepo,
        names: Vec<String>,
    }

    impl Drop for TestFilesCleanup {
        fn drop(&mut self) {
            cleanup_files_by_name(&self.repo, &self.names);
        }
    }

    struct ConfigValueGuard {
        repo: DbRepo,
        key: String,
        original_value: String,
    }

    impl ConfigValueGuard {
        fn set(repo: &DbRepo, key: &str, value: &str) -> Self {
            let key_sql = DbRepo::quote_literal(key);
            let original_value = repo
                .query_scalar_text(&format!("SELECT value FROM config WHERE key = {key_sql}"))
                .expect("failed to read original config value");
            repo.exec(&format!(
                "UPDATE config SET value = {} WHERE key = {key_sql}",
                DbRepo::quote_literal(value)
            ))
            .expect("failed to update config value");
            Self {
                repo: repo.clone(),
                key: key.to_string(),
                original_value,
            }
        }
    }

    impl Drop for ConfigValueGuard {
        fn drop(&mut self) {
            let _ = self.repo.exec(&format!(
                "UPDATE config SET value = {} WHERE key = {}",
                DbRepo::quote_literal(&self.original_value),
                DbRepo::quote_literal(&self.key)
            ));
        }
    }

    fn current_payload_bytes(repo: &DbRepo) -> u64 {
        repo.query_scalar_text(
            "
            SELECT (
                (SELECT COUNT(*)::bigint FROM data_blocks)
                * (SELECT value::bigint FROM config WHERE key = 'block_size')
            )::text
            ",
        )
        .unwrap()
        .trim()
        .parse()
        .unwrap()
    }

    fn current_quota_accounted_bytes(repo: &DbRepo) -> u64 {
        repo.query_scalar_text(
            "
            SELECT (
                (SELECT COUNT(*)::bigint FROM data_blocks)
                * (SELECT value::bigint FROM config WHERE key = 'block_size')
                + COALESCE((
                    SELECT SUM(reserved_bytes)::bigint
                    FROM payload_capacity_reservations
                    WHERE expires_at > NOW()
                ), 0)
            )::text
            ",
        )
        .unwrap()
        .trim()
        .parse()
        .unwrap()
    }

    fn wait_for_advisory_waiters(repo: &DbRepo, expected: u64) -> Result<u64, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let sql = "
            SELECT COUNT(*)::text
            FROM pg_stat_activity
            WHERE datname = current_database()
              AND wait_event_type = 'Lock'
              AND wait_event = 'advisory'
        ";
        loop {
            let waiting = repo
                .query_scalar_text(sql)?
                .trim()
                .parse::<u64>()
                .unwrap_or(0);
            if waiting >= expected {
                return Ok(waiting);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "expected {expected} advisory-lock waiters, observed {waiting}"
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_for_copy_merge_metrics(
        global: &DbRepoPayloadObservability,
        expected: u64,
    ) -> Result<DbRepoPayloadObservabilitySnapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let snapshot = global.snapshot()?;
            if snapshot.persist_copy_stage_count >= expected
                && snapshot.persist_data_blocks_merge_count >= expected
            {
                return Ok(snapshot);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "expected {expected} COPY/merge observations, got copy={} merge={}",
                    snapshot.persist_copy_stage_count, snapshot.persist_data_blocks_merge_count
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn file_storage_state(repo: &DbRepo, names: &[String]) -> HashMap<String, (u64, u64)> {
        let names_sql = quoted_list(names);
        repo.query_rows_text(&format!(
            "
            SELECT
                name,
                size::text,
                (SELECT COUNT(*)::text
                 FROM data_blocks
                 WHERE data_object_id = files.data_object_id)
            FROM files
            WHERE name IN ({names_sql})
            "
        ))
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row[0].clone(),
                (
                    row[1].trim().parse().unwrap(),
                    row[2].trim().parse().unwrap(),
                ),
            )
        })
        .collect()
    }

    fn run_single_block_persist(
        repo: DbRepo,
        file_id: u64,
        fill: u8,
        barrier: Arc<std::sync::Barrier>,
    ) -> Result<(), String> {
        let block = vec![fill; TEST_BLOCK_SIZE as usize];
        let rows = [PersistBlockRow {
            block_index: 0,
            data: &block,
            used_len: TEST_BLOCK_SIZE,
        }];
        barrier.wait();
        repo.persist_file_blocks_with_crc_flag(
            file_id,
            TEST_BLOCK_SIZE,
            TEST_BLOCK_SIZE,
            1,
            false,
            &rows,
            false,
        )
    }

    #[test]
    fn tunes_connection_string_from_runtime_config() {
        let tuning = ConnectionTuning {
            synchronous_commit: Some("remote_write".to_string()),
        };
        let tuned = apply_connection_tuning("host=localhost dbname=fod", &tuning).unwrap();
        assert_eq!(
            tuned,
            "host=localhost dbname=fod options='-c synchronous_commit=remote_write'"
        );
    }

    #[test]
    fn builds_repo_tuning_from_runtime_config() {
        let mut runtime = HashMap::new();
        runtime.insert("synchronous_commit".to_string(), "off".to_string());
        let runtime = RuntimeConfig::from_runtime_map(&runtime).unwrap();
        let repo = DbRepo::with_runtime("host=localhost dbname=fod", &runtime).unwrap();
        assert_eq!(
            repo.connection_tuning.synchronous_commit.as_deref(),
            Some("off")
        );
    }

    #[test]
    fn leaves_connection_string_unchanged_without_tuning() {
        let tuning = ConnectionTuning::default();
        let tuned = apply_connection_tuning("host=localhost dbname=fod", &tuning).unwrap();
        assert_eq!(tuned, "host=localhost dbname=fod");
    }

    #[test]
    fn recognizes_read_only_sql_for_replay() {
        let select_sql = std::ffi::CString::new("SELECT 1").unwrap();
        let recursive_sql =
            std::ffi::CString::new("WITH RECURSIVE parts AS (SELECT 1) SELECT 1").unwrap();
        let insert_sql = std::ffi::CString::new("INSERT INTO foo VALUES (1)").unwrap();

        assert!(sql_is_read_only(&select_sql));
        assert!(sql_is_read_only(&recursive_sql));
        assert!(!sql_is_read_only(&insert_sql));
    }

    #[test]
    fn recognizes_replayable_command_sql_for_disconnect_retry() {
        let heartbeat_sql = std::ffi::CString::new(
            "UPDATE lock_leases SET lease_expires_at = NOW() WHERE resource_kind = $1",
        )
        .unwrap();
        let delete_sql =
            std::ffi::CString::new("DELETE FROM xattrs WHERE owner_kind = $1").unwrap();
        let owner_key_insert_sql = std::ffi::CString::new(
            "INSERT INTO client_session_owner_keys (session_id, owner_key, first_seen_at, last_seen_at, updated_at) VALUES ($1, $2, NOW(), NOW(), NOW()) ON CONFLICT (session_id, owner_key) DO UPDATE SET last_seen_at = NOW(), updated_at = NOW()",
        )
        .unwrap();
        let data_blocks_upsert_sql = std::ffi::CString::new(
            "INSERT INTO data_blocks (data_object_id, _order, data) VALUES ($1, $2, $3) ON CONFLICT (data_object_id, _order) DO UPDATE SET data = EXCLUDED.data",
        )
        .unwrap();
        let scan_run_insert_sql = std::ffi::CString::new(
            "INSERT INTO index_scan_runs (id_index_source, started_at, status, request_token) VALUES (1, NOW(), 'running', 'scan:1:2:3') ON CONFLICT (request_token) DO UPDATE SET id_index_source = EXCLUDED.id_index_source, status = EXCLUDED.status, updated_at = NOW() RETURNING id_scan_run",
        )
        .unwrap();
        let import_plan_insert_sql = std::ffi::CString::new(
            "INSERT INTO index_import_plans (created_at, updated_at, status, request_token, dry_run, source_filter) VALUES (NOW(), NOW(), 'dry_run_running', 'plan:1:2:3', TRUE, NULL) ON CONFLICT (request_token) DO UPDATE SET status = EXCLUDED.status, dry_run = EXCLUDED.dry_run, source_filter = EXCLUDED.source_filter, updated_at = NOW() RETURNING id_import_plan",
        )
        .unwrap();
        let capacity_reservation_insert_sql = std::ffi::CString::new(
            "INSERT INTO payload_capacity_reservations (request_token, reserved_bytes, created_at, expires_at) VALUES ($1, $2, NOW(), NOW() + INTERVAL '1 hour') ON CONFLICT (request_token) DO UPDATE SET reserved_bytes = EXCLUDED.reserved_bytes, expires_at = EXCLUDED.expires_at",
        )
        .unwrap();
        let data_blocks_copy_sql = std::ffi::CString::new(
            "INSERT INTO data_blocks (data_object_id, _order, data) SELECT $2, _order, data FROM data_blocks WHERE data_object_id = $1",
        )
        .unwrap();
        let copy_block_crc_upsert_sql = std::ffi::CString::new(
            "INSERT INTO copy_block_crc (data_object_id, _order, crc32) SELECT data_object_id, _order, crc32 FROM staging_blocks ON CONFLICT (data_object_id, _order) DO UPDATE SET crc32 = EXCLUDED.crc32, updated_at = NOW()",
        )
        .unwrap();
        let copy_block_crc_copy_sql = std::ffi::CString::new(
            "INSERT INTO copy_block_crc (data_object_id, _order, crc32) SELECT $2, _order, crc32 FROM copy_block_crc WHERE data_object_id = $1",
        )
        .unwrap();
        let range_state_insert_sql = std::ffi::CString::new(
            "INSERT INTO lock_range_leases (resource_kind, resource_id, session_id, owner_key, lock_type, range_start, range_end, lease_expires_at, heartbeat_at, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), NOW(), NOW(), NOW())",
        )
        .unwrap();
        let file_size_update_sql = std::ffi::CString::new(
            "UPDATE files SET size = $1, modification_date = NOW(), change_date = NOW() WHERE id_file = $2",
        )
        .unwrap();
        let data_object_size_update_sql = std::ffi::CString::new(
            "UPDATE data_objects SET file_size = $1, modification_date = NOW() WHERE id_data_object = $2",
        )
        .unwrap();
        let xattr_insert_sql = std::ffi::CString::new(
            "INSERT INTO xattrs (owner_kind, owner_id, name, value) VALUES ($1, $2, $3, $4)",
        )
        .unwrap();
        let journal_sql = std::ffi::CString::new(
            "INSERT INTO journal (id_user, id_directory, id_file, action, date_time) VALUES ($1, $2, $3, $4, NOW())",
        )
        .unwrap();

        assert!(sql_is_replayable_command(&heartbeat_sql));
        assert!(sql_is_replayable_command(&delete_sql));
        assert!(sql_is_replayable_command(&owner_key_insert_sql));
        assert!(sql_is_replayable_command(&data_blocks_upsert_sql));
        assert!(sql_is_replayable_command(&scan_run_insert_sql));
        assert!(sql_is_replayable_command(&import_plan_insert_sql));
        assert!(sql_is_replayable_command(&capacity_reservation_insert_sql));
        assert!(!sql_is_replayable_command(&data_blocks_copy_sql));
        assert!(sql_is_replayable_command(&copy_block_crc_upsert_sql));
        assert!(!sql_is_replayable_command(&copy_block_crc_copy_sql));
        assert!(sql_is_replayable_command(&range_state_insert_sql));
        assert!(sql_is_replayable_command(&file_size_update_sql));
        assert!(sql_is_replayable_command(&data_object_size_update_sql));
        assert!(sql_is_replayable_command(&xattr_insert_sql));
        assert!(!sql_is_replayable_command(&journal_sql));
    }

    #[test]
    fn recognizes_replayable_schema_ddl_sql_for_disconnect_retry() {
        let lock_table_sql = std::ffi::CString::new(
            "
            CREATE TABLE IF NOT EXISTS lock_leases (
                id_lock SERIAL PRIMARY KEY
            )
            ",
        )
        .unwrap();
        let lock_index_sql = std::ffi::CString::new(
            "
            CREATE INDEX IF NOT EXISTS idx_lock_leases_expires
            ON lock_leases (lease_expires_at)
            ",
        )
        .unwrap();
        let lock_alter_sql = std::ffi::CString::new(
            "
            ALTER TABLE IF EXISTS lock_leases
            ALTER COLUMN session_id SET NOT NULL
            ",
        )
        .unwrap();
        let client_table_sql = std::ffi::CString::new(
            "
            CREATE TABLE IF NOT EXISTS client_sessions (
                session_id BIGSERIAL PRIMARY KEY
            )
            ",
        )
        .unwrap();
        let function_sql = std::ffi::CString::new(
            "
            CREATE OR REPLACE FUNCTION fod_prune_client_session_lock_leases()
            RETURNS trigger AS $$
            BEGIN
                RETURN OLD;
            END;
            $$ LANGUAGE plpgsql
            ",
        )
        .unwrap();
        let trigger_drop_sql = std::ffi::CString::new(
            "
            DROP TRIGGER IF EXISTS fod_client_sessions_prune_lock_leases
            ON client_sessions
            ",
        )
        .unwrap();
        let trigger_create_sql = std::ffi::CString::new(
            "
            CREATE TRIGGER fod_client_sessions_prune_lock_leases
            BEFORE DELETE ON client_sessions
            FOR EACH ROW
            EXECUTE FUNCTION fod_prune_client_session_lock_leases()
            ",
        )
        .unwrap();
        let negative_sql = std::ffi::CString::new(
            "
            ALTER TABLE lock_leases
            DROP COLUMN session_id
            ",
        )
        .unwrap();

        assert!(sql_is_replayable_command(&lock_table_sql));
        assert!(sql_is_replayable_command(&lock_index_sql));
        assert!(sql_is_replayable_command(&lock_alter_sql));
        assert!(sql_is_replayable_command(&client_table_sql));
        assert!(sql_is_replayable_command(&function_sql));
        assert!(sql_is_replayable_command(&trigger_drop_sql));
        assert!(sql_is_replayable_command(&trigger_create_sql));
        assert!(!sql_is_replayable_command(&negative_sql));
    }

    #[test]
    fn payload_persist_guard_uses_global_byte_budget() {
        let global = Arc::new(DbRepoPayloadObservability::with_in_flight_limit_bytes(10));
        let mut first = PayloadPersistGuard::new(
            Arc::new(DbRepoPayloadObservability::default()),
            Arc::clone(&global),
            7,
            1,
        );

        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker_global = Arc::clone(&global);
        let worker = std::thread::spawn(move || {
            let mut guard = PayloadPersistGuard::new(
                Arc::new(DbRepoPayloadObservability::default()),
                worker_global,
                6,
                1,
            );
            guard.mark_success();
            acquired_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = global.snapshot().unwrap();
            if snapshot.queued_requests >= 1 && snapshot.queued_bytes >= 6 {
                assert_eq!(snapshot.reserved_bytes, 7);
                assert_eq!(snapshot.in_flight_bytes, 7);
                assert!(snapshot.backpressure_events >= 1);
                break;
            }
            assert!(Instant::now() < deadline, "payload guard did not queue");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(acquired_rx.try_recv().is_err());

        first.mark_success();
        drop(first);
        acquired_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("queued payload guard was not admitted");
        let active = global.snapshot().unwrap();
        assert_eq!(active.reserved_bytes, 6);
        assert_eq!(active.in_flight_bytes, 6);

        release_tx.send(()).unwrap();
        worker.join().unwrap();

        let final_snapshot = global.snapshot().unwrap();
        assert_eq!(final_snapshot.reserved_bytes, 0);
        assert_eq!(final_snapshot.in_flight_bytes, 0);
        assert_eq!(final_snapshot.queued_requests, 0);
        assert_eq!(final_snapshot.budget_accounting_errors, 0);
        assert_eq!(final_snapshot.accounting_errors, 0);
    }

    #[test]
    fn ordinary_persist_copy_merge_completes_before_final_quota_gate() {
        let global = Arc::new(DbRepoPayloadObservability::default());
        let observer = repo_with_global_payload(Arc::clone(&global));
        assert!(
            observer.schema_is_initialized().unwrap(),
            "FOD schema must be initialized before running the quota concurrency test"
        );

        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let names = vec![
            format!("late-quota-a-{suffix}.bin"),
            format!("late-quota-b-{suffix}.bin"),
        ];
        cleanup_files_by_name(&observer, &names);
        let _cleanup = TestFilesCleanup {
            repo: observer.clone(),
            names: names.clone(),
        };

        let repo_a = repo_with_global_payload(Arc::clone(&global));
        let repo_b = repo_with_global_payload(Arc::clone(&global));
        let file_ids = [
            repo_a
                .create_file(None, &names[0], 0o644, 1000, 1000, &format!("{suffix}-a"))
                .unwrap(),
            repo_b
                .create_file(None, &names[1], 0o644, 1000, 1000, &format!("{suffix}-b"))
                .unwrap(),
        ];

        let baseline_payload = current_payload_bytes(&observer);
        let quota_accounted = current_quota_accounted_bytes(&observer);
        let _limit_guard = ConfigValueGuard::set(
            &observer,
            "max_fs_size_bytes",
            &quota_accounted.saturating_add(TEST_BLOCK_SIZE).to_string(),
        );

        let blocker = repo_with_global_payload(Arc::clone(&global));
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut handles = vec![
            {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    run_single_block_persist(repo_a, file_ids[0], b'A', barrier)
                })
            },
            {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    run_single_block_persist(repo_b, file_ids[1], b'B', barrier)
                })
            },
        ];

        let before_release = blocker
            .with_cached_connection(|conn| unsafe {
                let begin = CString::new("BEGIN").unwrap();
                let rollback = CString::new("ROLLBACK").unwrap();
                let commit = CString::new("COMMIT").unwrap();
                let lock = CString::new("SELECT pg_advisory_xact_lock(4607812, 1)").unwrap();
                exec_command(conn, &begin)?;
                query_scalar_text_on_conn(conn, &lock)?;
                barrier.wait();

                let result = (|| {
                    let waiting = wait_for_advisory_waiters(&observer, 2)?;
                    let snapshot = wait_for_copy_merge_metrics(&global, 2)?;
                    if snapshot.quota_final_check_count != 0 {
                        return Err(format!(
                            "final quota check ran before the external advisory lock was released: {}",
                            snapshot.quota_final_check_count
                        ));
                    }
                    Ok((waiting, snapshot))
                })();

                let release = if result.is_ok() {
                    exec_command(conn, &commit)
                } else {
                    exec_command(conn, &rollback)
                };
                release?;
                result
            })
            .unwrap();

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles.drain(..) {
            results.push(
                handle
                    .join()
                    .unwrap_or_else(|_| Err("quota writer panicked".to_string())),
            );
        }

        let winners = results
            .iter()
            .enumerate()
            .filter_map(|(index, result)| result.as_ref().ok().map(|_| index))
            .collect::<Vec<_>>();
        let rejected = results
            .iter()
            .enumerate()
            .filter_map(|(index, result)| match result {
                Err(err) if err.starts_with(STORAGE_QUOTA_EXCEEDED_PREFIX) => Some(index),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            winners.len(),
            1,
            "expected one successful writer, got {results:?}"
        );
        assert_eq!(
            rejected.len(),
            1,
            "expected one quota rejection, got {results:?}"
        );

        let after_payload = current_payload_bytes(&observer);
        assert_eq!(after_payload, baseline_payload + TEST_BLOCK_SIZE);
        let states = file_storage_state(&observer, &names);
        let expected_states = HashMap::from([
            (names[winners[0]].clone(), (TEST_BLOCK_SIZE, 1)),
            (names[rejected[0]].clone(), (0, 0)),
        ]);
        assert_eq!(states, expected_states);

        let final_snapshot = global.snapshot().unwrap();
        assert!(before_release.0 >= 2);
        assert!(before_release.1.persist_copy_stage_count >= 2);
        assert!(before_release.1.persist_data_blocks_merge_count >= 2);
        assert_eq!(before_release.1.quota_final_check_count, 0);
        assert!(final_snapshot.quota_lock_wait_count >= 2);
        assert!(final_snapshot.quota_lock_held_count >= 2);
        assert!(final_snapshot.quota_final_check_count >= 2);
        assert_eq!(final_snapshot.persist_transaction_count, 2);
        assert_eq!(final_snapshot.persist_transaction_failures, 1);
    }

    #[test]
    fn transaction_admission_limits_real_postgres_transactions_by_lane() {
        fn run_phase(repo: &DbRepo, lane: ConnectionLane, workers: usize, sleep_seconds: f64) {
            let barrier = Arc::new(std::sync::Barrier::new(workers));
            let mut handles = Vec::with_capacity(workers);
            for _ in 0..workers {
                let repo = repo.clone();
                let barrier = Arc::clone(&barrier);
                handles.push(std::thread::spawn(move || {
                    repo.with_connection(lane, |conn| {
                        barrier.wait();
                        unsafe {
                            transactional_replayable(conn, |conn| {
                                let sql = CString::new(format!("SELECT pg_sleep({sleep_seconds})"))
                                    .map_err(|_| "SQL contains NUL byte".to_string())?;
                                let _ = query_scalar_text_on_conn(conn, &sql)?;
                                Ok(())
                            })
                        }
                    })
                    .expect("transaction admission phase failed");
                }));
            }
            for handle in handles {
                handle
                    .join()
                    .expect("transaction admission worker panicked");
            }
        }

        let runtime = RuntimeConfig::from_runtime_map(&HashMap::new()).unwrap();
        let storage = runtime.storage_settings();
        let payload = Arc::new(DbRepoPayloadObservability::default());
        let repo = DbRepo::new_with_tuning(
            &conninfo(),
            ConnectionTuning::from_storage(&storage),
            storage.persist_buffer_chunk_blocks,
            storage.persist_block_transport,
            storage.data_object_swap_cleanup,
            8,
            RuntimeTransactionSettings {
                write_active_limit: 2,
                control_active_limit: 1,
            },
            Arc::clone(&payload),
            payload,
        )
        .unwrap();

        run_phase(&repo, ConnectionLane::Write, 6, 0.08);
        let after_write = repo.observability_snapshot().unwrap().pool;
        assert_eq!(after_write.write_transaction_admission.limit, 2);
        assert_eq!(after_write.write_transaction_admission.active, 0);
        assert_eq!(after_write.write_transaction_admission.queued, 0);
        assert_eq!(after_write.write_transaction_admission.peak_active, 2);
        assert!(after_write.write_transaction_admission.peak_queued >= 2);
        assert!(after_write.write_transaction_admission.backpressure_events >= 1);
        assert_eq!(after_write.write_transaction_admission.accounting_errors, 0);
        assert_eq!(after_write.control_transaction_admission.peak_active, 0);

        run_phase(&repo, ConnectionLane::Control, 4, 0.08);
        let after_control = repo.observability_snapshot().unwrap().pool;
        assert_eq!(after_control.control_transaction_admission.limit, 1);
        assert_eq!(after_control.control_transaction_admission.active, 0);
        assert_eq!(after_control.control_transaction_admission.queued, 0);
        assert_eq!(after_control.control_transaction_admission.peak_active, 1);
        assert!(after_control.control_transaction_admission.peak_queued >= 2);
        assert!(
            after_control
                .control_transaction_admission
                .backpressure_events
                >= 1
        );
        assert_eq!(
            after_control
                .control_transaction_admission
                .accounting_errors,
            0
        );
        assert!(after_control.transaction_count >= 10);
    }

    #[test]
    fn replayable_sql_error_prefix_is_stripped() {
        let err = replayable_sql_error("synthetic failure".to_string());
        assert!(err.starts_with(REPLAYABLE_SQL_ERROR_PREFIX));
        assert_eq!(strip_replayable_sql_error(err), "synthetic failure");
    }

    #[test]
    fn replayable_connection_error_retries_once() {
        let repo = DbRepo::new(&conninfo()).expect("failed to connect to PostgreSQL");
        let attempts = AtomicUsize::new(0);

        let value = repo
            .with_cached_connection(|_conn| {
                let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if attempt == 0 {
                    Err(replayable_sql_error("synthetic replay".to_string()))
                } else {
                    Ok("replayed")
                }
            })
            .expect("replayable closure should succeed after retry");

        assert_eq!(value, "replayed");
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn non_replayable_error_is_not_retried() {
        let repo = DbRepo::new(&conninfo()).expect("failed to connect to PostgreSQL");
        let attempts = AtomicUsize::new(0);

        let err: String = repo
            .with_cached_connection(|_conn| {
                attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err::<&str, String>("plain failure".to_string())
            })
            .expect_err("plain failure should be returned directly");

        assert_eq!(err, "plain failure");
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn encodes_copy_binary_stage_rows_with_padding_and_sparse_nulls() {
        let mut out = Vec::new();
        append_copy_binary_header(&mut out);
        let block = PersistBlockRow {
            block_index: 3,
            data: b"abc",
            used_len: 2,
        };
        append_persist_block_copy_binary_row(&mut out, 11, &block, 4).unwrap();

        let signature = &out[..COPY_BINARY_SIGNATURE.len()];
        assert_eq!(signature, COPY_BINARY_SIGNATURE);

        let mut cursor = COPY_BINARY_SIGNATURE.len();
        let flags = i32::from_be_bytes(out[cursor..cursor + 4].try_into().unwrap());
        cursor += 4;
        let header_len = i32::from_be_bytes(out[cursor..cursor + 4].try_into().unwrap());
        cursor += 4;
        assert_eq!(flags, 0);
        assert_eq!(header_len, 0);

        let field_count = i16::from_be_bytes(out[cursor..cursor + 2].try_into().unwrap());
        cursor += 2;
        assert_eq!(field_count, 4);

        let data_object_len = i32::from_be_bytes(out[cursor..cursor + 4].try_into().unwrap());
        cursor += 4;
        assert_eq!(data_object_len, 8);
        let data_object_id = i64::from_be_bytes(out[cursor..cursor + 8].try_into().unwrap());
        cursor += 8;
        assert_eq!(data_object_id, 11);

        let block_index_len = i32::from_be_bytes(out[cursor..cursor + 4].try_into().unwrap());
        cursor += 4;
        assert_eq!(block_index_len, 8);
        let block_index = i64::from_be_bytes(out[cursor..cursor + 8].try_into().unwrap());
        cursor += 8;
        assert_eq!(block_index, 3);

        let data_len = i32::from_be_bytes(out[cursor..cursor + 4].try_into().unwrap());
        cursor += 4;
        assert_eq!(data_len, 4);
        assert_eq!(&out[cursor..cursor + 4], b"abc\0");
        cursor += 4;

        let crc_len = i32::from_be_bytes(out[cursor..cursor + 4].try_into().unwrap());
        assert_eq!(crc_len, -1);
    }
}

#[cfg(test)]
mod replica_read_consistency_tests {
    use super::*;

    #[test]
    fn parses_and_orders_postgresql_lsn_values() {
        assert_eq!(parse_pg_lsn("0/0").unwrap(), 0);
        assert_eq!(parse_pg_lsn("0/10").unwrap(), 16);
        assert!(parse_pg_lsn("1/0").unwrap() > parse_pg_lsn("0/FFFFFFFF").unwrap());
        assert!(parse_pg_lsn("invalid").is_err());
    }

    #[test]
    fn primary_lsn_capture_sets_replica_barrier() {
        let tracker = ReplicaReadConsistency::default();
        assert_eq!(tracker.required_lsn().unwrap(), None);
        tracker.record_primary_wal_lsn("0/16B6C50").unwrap();
        assert_eq!(
            tracker.required_lsn().unwrap(),
            Some(parse_pg_lsn("0/16B6C50").unwrap())
        );
        let snapshot = tracker.snapshot().unwrap();
        assert_eq!(
            snapshot.required_primary_wal_lsn.as_deref(),
            Some("0/16B6C50")
        );
        assert_eq!(snapshot.primary_wal_lsn_updates, 1);
        assert!(!snapshot.force_primary);
    }

    #[test]
    fn lsn_capture_failure_forces_primary_reads() {
        let tracker = ReplicaReadConsistency::default();
        tracker.record_primary_wal_lsn_capture_failure();
        assert!(tracker.force_primary().unwrap());
        assert_eq!(
            tracker.snapshot().unwrap().primary_wal_lsn_capture_failures,
            1
        );
    }
}

#[cfg(test)]
mod replica_scoring_tests {
    use super::*;

    fn target(authority: &str) -> DbRepoConnectionTarget {
        DbRepoConnectionTarget {
            authority: authority.to_string(),
            conninfo: format!("host={authority}"),
        }
    }

    fn replica_targets() -> DbRepoConnectionTargets {
        DbRepoConnectionTargets::new(
            vec![target("replica-a"), target("replica-b")],
            DbRepoConnectionTargetRequirement::ReadOnlyReplica,
            true,
            true,
        )
        .unwrap()
    }

    #[test]
    fn lagged_replica_forces_switch_to_another_candidate() {
        let targets = replica_targets();
        let generation = targets.generation().unwrap();
        assert_eq!(targets.snapshot().unwrap().active_authority, "replica-a");
        let switched = targets
            .record_replica_replay(generation, 1024 * 1024)
            .unwrap();
        assert!(switched);
        assert_eq!(targets.snapshot().unwrap().active_authority, "replica-b");
        assert_eq!(targets.scoring_snapshot().unwrap().score_switches, 1);
    }

    #[test]
    fn hysteresis_keeps_current_replica_for_small_score_difference() {
        let targets = replica_targets();
        {
            let mut state = targets.state().unwrap();
            state.target_metrics[0].connection_latency_micros_ewma = Some(1_000);
            state.target_metrics[0].operation_latency_micros_ewma = Some(1_000);
            state.target_metrics[0].replay_lag_bytes = Some(0);
            state.target_metrics[1].connection_latency_micros_ewma = Some(900);
            state.target_metrics[1].operation_latency_micros_ewma = Some(900);
            state.target_metrics[1].replay_lag_bytes = Some(0);
        }
        let generation = targets.generation().unwrap();
        assert_eq!(targets.prepare_scored_replica().unwrap(), Some(generation));
        assert_eq!(targets.snapshot().unwrap().active_authority, "replica-a");
        assert!(targets.scoring_snapshot().unwrap().hysteresis_keeps >= 1);
    }

    #[test]
    fn large_score_improvement_switches_replica() {
        let targets = replica_targets();
        let generation = targets.generation().unwrap();
        {
            let mut state = targets.state().unwrap();
            state.target_metrics[0].connection_latency_micros_ewma = Some(20_000);
            state.target_metrics[0].operation_latency_micros_ewma = Some(20_000);
            state.target_metrics[0].replay_lag_bytes = Some(0);
            state.target_metrics[1].connection_latency_micros_ewma = Some(100);
            state.target_metrics[1].operation_latency_micros_ewma = Some(100);
            state.target_metrics[1].replay_lag_bytes = Some(0);
        }
        let selected = targets.prepare_scored_replica().unwrap().unwrap();
        assert!(selected > generation);
        assert_eq!(targets.snapshot().unwrap().active_authority, "replica-b");
    }

    #[test]
    fn open_circuit_is_skipped() {
        let targets = replica_targets();
        {
            let mut state = targets.state().unwrap();
            state.target_metrics[0].circuit_open_until =
                Some(Instant::now() + Duration::from_secs(30));
        }
        let _ = targets.prepare_scored_replica().unwrap().unwrap();
        assert_eq!(targets.snapshot().unwrap().active_authority, "replica-b");
        let scoring = targets.scoring_snapshot().unwrap();
        assert_eq!(scoring.circuit_open_targets, 1);
        assert!(scoring.circuit_breaker_skips >= 1);
    }

    #[test]
    fn stale_failure_generation_does_not_misattribute_current_authority() {
        let targets = DbRepoConnectionTargets::new(
            vec![target("primary-a"), target("primary-b")],
            DbRepoConnectionTargetRequirement::WritablePrimary,
            true,
            true,
        )
        .unwrap();

        let old_generation = targets.generation().unwrap();
        targets
            .mark_operation_connection_failure(old_generation)
            .unwrap();
        let after_first = targets.snapshot().unwrap();
        assert_eq!(after_first.active_authority, "primary-a");
        assert_eq!(
            after_first.last_failed_authority.as_deref(),
            Some("primary-a")
        );
        assert_eq!(after_first.connection_failures, 1);
        {
            let mut state = targets.state().unwrap();
            assert!(state.primary_revalidation_required);
            state.active_index = 1;
            state.primary_revalidation_required = false;
        }

        targets
            .mark_operation_connection_failure(old_generation)
            .unwrap();
        let after_stale = targets.snapshot().unwrap();
        assert_eq!(after_stale.active_authority, "primary-b");
        assert_eq!(
            after_stale.last_failed_authority.as_deref(),
            Some("primary-a")
        );
        assert_eq!(after_stale.connection_failures, 2);
    }

    #[test]
    fn pool_pressure_score_counts_utilization_and_queue() {
        assert_eq!(pool_pressure_milli(0, 4, 0), 0);
        assert_eq!(pool_pressure_milli(2, 4, 0), 500);
        assert_eq!(pool_pressure_milli(2, 4, 1), 1_500);
    }
}

#[cfg(test)]
mod primary_promotion_guard_tests {
    use super::*;

    fn candidate(
        target_index: usize,
        authority: &str,
        system_identifier: &str,
        server_fingerprint: &str,
    ) -> PrimarySafetyCandidate {
        PrimarySafetyCandidate {
            target_index,
            authority: authority.to_string(),
            system_identifier: system_identifier.to_string(),
            server_fingerprint: server_fingerprint.to_string(),
        }
    }

    #[test]
    fn aliases_to_same_writable_primary_are_not_split_brain() {
        let candidates = vec![
            candidate(0, "entry-a", "cluster-1", "10.0.0.1:5432|start-a"),
            candidate(1, "entry-b", "cluster-1", "10.0.0.1:5432|start-a"),
        ];
        assert_eq!(
            choose_safe_primary_candidate(&candidates, None, None).unwrap(),
            0
        );
    }

    #[test]
    fn failed_alias_prefers_another_alias_to_same_primary() {
        let candidates = vec![
            candidate(0, "entry-a", "cluster-1", "10.0.0.1:5432|start-a"),
            candidate(1, "entry-b", "cluster-1", "10.0.0.1:5432|start-a"),
        ];
        assert_eq!(
            choose_safe_primary_candidate(&candidates, Some("cluster-1"), Some(0)).unwrap(),
            1
        );
    }

    #[test]
    fn two_distinct_writable_servers_are_rejected_as_split_brain() {
        let candidates = vec![
            candidate(0, "primary-a", "cluster-1", "10.0.0.1:5432|start-a"),
            candidate(1, "primary-b", "cluster-1", "10.0.0.2:5432|start-b"),
        ];
        let err = choose_safe_primary_candidate(&candidates, Some("cluster-1"), None).unwrap_err();
        assert_eq!(err.kind, PrimarySafetyErrorKind::SplitBrain);
    }

    #[test]
    fn foreign_writable_cluster_is_rejected() {
        let candidates = vec![
            candidate(0, "primary-a", "cluster-1", "10.0.0.1:5432|start-a"),
            candidate(1, "foreign", "cluster-2", "10.0.0.2:5432|start-b"),
        ];
        let err = choose_safe_primary_candidate(&candidates, Some("cluster-1"), None).unwrap_err();
        assert_eq!(err.kind, PrimarySafetyErrorKind::ClusterIdentityMismatch);
    }

    #[test]
    fn no_writable_candidate_is_rejected() {
        let err = choose_safe_primary_candidate(&[], Some("cluster-1"), None).unwrap_err();
        assert_eq!(err.kind, PrimarySafetyErrorKind::NoWritablePrimary);
    }

    #[test]
    fn primary_generation_transition_waits_for_in_flight_operation() {
        let targets = Arc::new(
            DbRepoConnectionTargets::new(
                vec![
                    DbRepoConnectionTarget {
                        authority: "primary-a".to_string(),
                        conninfo: "host=primary-a".to_string(),
                    },
                    DbRepoConnectionTarget {
                        authority: "primary-b".to_string(),
                        conninfo: "host=primary-b".to_string(),
                    },
                ],
                DbRepoConnectionTargetRequirement::WritablePrimary,
                true,
                true,
            )
            .unwrap(),
        );

        let generation = targets.generation().unwrap();
        let operation = targets
            .begin_primary_operation(generation)
            .unwrap()
            .expect("primary operation should enter current generation");

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let transition_targets = Arc::clone(&targets);
        let transition = std::thread::spawn(move || {
            let result = transition_targets.mark_operation_connection_failure(generation);
            done_tx.send(result).unwrap();
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let pending = {
                let state = targets.state().unwrap();
                state.primary_transition_pending
            };
            if pending {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "primary transition did not enter pending state"
            );
            std::thread::sleep(Duration::from_millis(1));
        }

        assert!(
            done_rx.try_recv().is_err(),
            "generation transition completed while old operation was still active"
        );
        assert_eq!(targets.generation().unwrap(), generation);
        {
            let state = targets.state().unwrap();
            assert_eq!(state.primary_operations_in_flight, 1);
            assert!(state.primary_transition_pending);
        }

        let (waiter_started_tx, waiter_started_rx) = std::sync::mpsc::channel();
        let (waiter_done_tx, waiter_done_rx) = std::sync::mpsc::channel();
        let waiter_targets = Arc::clone(&targets);
        let waiter = std::thread::spawn(move || {
            waiter_started_tx.send(()).unwrap();
            let admitted = waiter_targets
                .begin_primary_operation(generation)
                .unwrap()
                .is_some();
            waiter_done_tx.send(admitted).unwrap();
        });

        waiter_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("new-operation waiter did not start");
        assert!(
            waiter_done_rx
                .recv_timeout(Duration::from_millis(25))
                .is_err(),
            "new primary operation completed admission while transition was pending"
        );

        drop(operation);

        let next_generation = done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("generation transition did not complete after operation drain")
            .expect("generation transition failed");
        transition
            .join()
            .expect("generation transition thread panicked");

        let stale_admitted = waiter_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("new-operation waiter did not finish after transition");
        waiter.join().expect("new-operation waiter panicked");
        assert!(
            !stale_admitted,
            "operation waiting across transition entered the stale generation"
        );

        assert!(next_generation > generation);
        assert_eq!(targets.generation().unwrap(), next_generation);
        {
            let state = targets.state().unwrap();
            assert_eq!(state.primary_operations_in_flight, 0);
            assert!(!state.primary_transition_pending);
            assert!(state.primary_revalidation_required);
        }

        assert!(
            targets
                .begin_primary_operation(generation)
                .unwrap()
                .is_none(),
            "stale generation unexpectedly admitted a new primary operation"
        );
        let current_operation = targets
            .begin_primary_operation(next_generation)
            .unwrap()
            .expect("current generation should admit a primary operation");
        drop(current_operation);
    }
}
