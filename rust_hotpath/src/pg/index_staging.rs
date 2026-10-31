// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! Index catalog staging through COPY TEXT and transactional merges. Each batch
//! keeps staging, COPY and merge on the same authority connection and transaction.

use super::{
    exec_command, hex_encode_bytes, query_rows_text_on_conn, transactional_replayable,
    CopyInSession, DbRepo, PGconn,
};
use std::ffi::CString;

const INDEX_SOURCES_STAGE_TABLE: &str = "index_sources_stage";
const INDEX_SCAN_RUNS_STAGE_TABLE: &str = "index_scan_runs_stage";
const INDEX_FILES_STAGE_TABLE: &str = "index_files_stage";
const INDEX_FILE_HASHES_STAGE_TABLE: &str = "index_file_hashes_stage";
const INDEX_IMPORT_PLANS_STAGE_TABLE: &str = "index_import_plans_stage";
const INDEX_IMPORT_PLAN_ENTRIES_STAGE_TABLE: &str = "index_import_plan_entries_stage";

fn decode_single_positive_id(rows: Vec<Vec<String>>, description: &str) -> Result<u64, String> {
    if rows.len() != 1 {
        return Err(format!(
            "{description} returned an invalid result: expected 1 row, got {}",
            rows.len()
        ));
    }
    let row = &rows[0];
    if row.len() != 1 {
        return Err(format!(
            "{description} returned an invalid result: expected 1 column, got {}",
            row.len()
        ));
    }
    let id = row[0]
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("{description} returned an invalid ID: {err}"))?;
    if id == 0 {
        return Err(format!("{description} returned a non-positive ID"));
    }
    Ok(id)
}

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

impl DbRepo {
    pub fn insert_materialize_import_plan(
        &self,
        request_token: &str,
        source_filter: &str,
    ) -> Result<super::MaterializationFence, String> {
        if request_token.is_empty() || source_filter.is_empty() {
            return Err("materialization plan requires a request token and source".to_string());
        }
        let sql = CString::new(format!(
            "INSERT INTO index_import_plans (
                created_at, updated_at, status, request_token, dry_run, source_filter,
                materialize_fencing_token, materialize_lease_expires_at
            ) VALUES (
                clock_timestamp(), clock_timestamp(), 'materialize_running', {}, FALSE, {},
                1, clock_timestamp() + INTERVAL '60 seconds'
            )
            ON CONFLICT (request_token) DO NOTHING
            RETURNING id_import_plan, materialize_fencing_token",
            DbRepo::quote_literal(request_token),
            DbRepo::quote_literal(source_filter),
        ))
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                let mut rows = query_rows_text_on_conn(conn, &sql)?;
                if rows.is_empty() {
                    let retry_sql = CString::new(format!(
                        "SELECT id_import_plan, materialize_fencing_token
                         FROM index_import_plans
                         WHERE request_token = {}
                           AND status = 'materialize_running'
                           AND materialize_fencing_token = 1
                           AND materialize_lease_expires_at > clock_timestamp()",
                        DbRepo::quote_literal(request_token),
                    ))
                    .map_err(|_| "SQL contains NUL byte".to_string())?;
                    rows = query_rows_text_on_conn(conn, &retry_sql)?;
                }
                if rows.len() != 1 || rows[0].len() != 2 {
                    return Err("materialization plan insert returned an invalid row".to_string());
                }
                let plan_id = rows[0][0]
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "materialization plan id is invalid".to_string())?;
                let fencing_token = rows[0][1]
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "materialization fencing token is invalid".to_string())?;
                if plan_id == 0 || fencing_token == 0 {
                    return Err("materialization plan returned a non-positive identifier".into());
                }
                Ok(super::MaterializationFence {
                    plan_id,
                    fencing_token,
                })
            })
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

                let lock_sql = CString::new(format!(
                    "DO $fod$ DECLARE lock_key BIGINT; BEGIN \
                     FOR lock_key IN SELECT DISTINCT hashtextextended('fod:index-source:' || md5(name), 0) \
                     FROM {stage} ORDER BY 1 LOOP \
                     PERFORM pg_advisory_xact_lock(lock_key); END LOOP; END $fod$",
                    stage = INDEX_SOURCES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                exec_command(conn, &lock_sql)?;

                let merge_sql = CString::new(format!(
                    "
                    UPDATE index_sources AS target
                    SET kind = staged.kind,
                        root_path = staged.root_path,
                        updated_at = NOW()
                    FROM {stage} AS staged
                    WHERE md5(target.name) = md5(staged.name)
                      AND target.name COLLATE \"C\" = staged.name COLLATE \"C\";

                    INSERT INTO index_sources (
                        name, kind, root_path, created_at, updated_at
                    )
                    SELECT staged.name, staged.kind, staged.root_path, NOW(), NOW()
                    FROM {stage} AS staged
                    WHERE NOT EXISTS (
                        SELECT 1 FROM index_sources AS target
                        WHERE md5(target.name) = md5(staged.name)
                          AND target.name COLLATE \"C\" = staged.name COLLATE \"C\"
                    )
                    ",
                    stage = INDEX_SOURCES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                exec_command(conn, &merge_sql)?;
                let lookup_sql = CString::new(format!(
                    "SELECT target.id_index_source::text FROM index_sources AS target \
                     JOIN {stage} AS staged ON md5(target.name) = md5(staged.name) \
                     AND target.name COLLATE \"C\" = staged.name COLLATE \"C\" LIMIT 1",
                    stage = INDEX_SOURCES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                let rows = query_rows_text_on_conn(conn, &lookup_sql)?;
                decode_single_positive_id(rows, "source registration")
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
                decode_single_positive_id(rows, "scan run creation")
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
                decode_single_positive_id(rows, "import plan creation")
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

                let duplicate_sql = CString::new(format!(
                    "SELECT 1 FROM {stage} GROUP BY id_index_source, path COLLATE \"C\" \
                     HAVING COUNT(*) > 1 LIMIT 1",
                    stage = INDEX_FILES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                if !query_rows_text_on_conn(conn, &duplicate_sql)?.is_empty() {
                    return Err("index scan produced duplicate normalized source paths".to_string());
                }

                let lock_sql = CString::new(format!(
                    "DO $fod$ DECLARE lock_key BIGINT; BEGIN \
                     FOR lock_key IN SELECT DISTINCT hashtextextended( \
                         'fod:index-file:' || id_index_source::text || ':' || md5(path), 0) \
                     FROM {stage} ORDER BY 1 LOOP \
                     PERFORM pg_advisory_xact_lock(lock_key); END LOOP; END $fod$",
                    stage = INDEX_FILES_STAGE_TABLE
                ))
                .map_err(|_| "SQL contains NUL byte".to_string())?;
                exec_command(conn, &lock_sql)?;

                let merge_sql = CString::new(format!(
                    "
                    UPDATE index_files AS target
                    SET id_scan_run = staged.id_scan_run,
                        size = staged.size,
                        mtime_ns = staged.mtime_ns,
                        inode = staged.inode,
                        device = staged.device,
                        file_kind = staged.file_kind,
                        scan_status = staged.scan_status,
                        source_changed = staged.source_changed,
                        updated_at = NOW()
                    FROM {stage} AS staged
                    WHERE target.id_index_source = staged.id_index_source
                      AND md5(target.path) = md5(staged.path)
                      AND target.path COLLATE \"C\" = staged.path COLLATE \"C\";

                    INSERT INTO index_files (
                        id_index_source, id_scan_run, path, size, mtime_ns,
                        inode, device, file_kind, scan_status, source_changed,
                        created_at, updated_at
                    )
                    SELECT staged.id_index_source, staged.id_scan_run, staged.path,
                           staged.size, staged.mtime_ns, staged.inode, staged.device,
                           staged.file_kind, staged.scan_status, staged.source_changed,
                           NOW(), NOW()
                    FROM {stage} AS staged
                    WHERE NOT EXISTS (
                        SELECT 1 FROM index_files AS target
                        WHERE target.id_index_source = staged.id_index_source
                          AND md5(target.path) = md5(staged.path)
                          AND target.path COLLATE \"C\" = staged.path COLLATE \"C\"
                    )
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
        if let Some(fence) = self.materialization_fence() {
            if rows.iter().any(|row| row.id_import_plan != fence.plan_id) {
                return Err("FOD_INDEXER_MATERIALIZE_FENCE_PLAN_MISMATCH".to_string());
            }
        }
        if rows.is_empty() {
            return Ok(());
        }

        self.with_cached_connection(|conn| unsafe {
            transactional_replayable(conn, |conn| {
                self.validate_materialization_fence_on_conn(conn)?;
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
                self.validate_materialization_fence_on_conn(conn)?;
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

                let stage_sql = CString::new(
                    "
                    CREATE TEMP TABLE IF NOT EXISTS index_duplicate_sets_stage (
                        hash_algorithm TEXT NOT NULL,
                        full_hash BYTEA NOT NULL,
                        file_size BIGINT NOT NULL,
                        file_count BIGINT NOT NULL,
                        total_bytes BIGINT NOT NULL
                    ) ON COMMIT PRESERVE ROWS;
                    TRUNCATE index_duplicate_sets_stage
                    ",
                )
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
}

#[cfg(test)]
mod tests {
    use super::decode_single_positive_id;

    #[test]
    fn staged_id_decoder_requires_one_positive_scalar() {
        assert_eq!(
            decode_single_positive_id(vec![vec!["42".to_string()]], "staged ID"),
            Ok(42)
        );
        for rows in [
            Vec::new(),
            vec![vec!["1".to_string()], vec!["2".to_string()]],
            vec![vec!["1".to_string(), "extra".to_string()]],
            vec![vec!["0".to_string()]],
            vec![vec!["invalid".to_string()]],
        ] {
            assert!(
                decode_single_positive_id(rows, "staged ID").is_err(),
                "staged ID decoder accepted malformed rows"
            );
        }
    }
}
