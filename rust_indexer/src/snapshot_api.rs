use crate::db::sql_quote_literal;
use crate::file_read_api;
use crate::output::{
    CommandCapabilityView, IndexerCapabilitiesOutput, INDEXER_DEFAULT_PAGE_LIMIT,
    INDEXER_MAX_PAGE_LIMIT,
};
use fod_rust_hotpath::pg::DbRepo;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CatalogSnapshotView {
    pub snapshot_id: u64,
    pub status: String,
    pub source_filter: Option<String>,
    pub file_count: u64,
    pub total_bytes: u64,
    pub max_file_id: Option<u64>,
    pub created_at: String,
}

impl CatalogSnapshotView {
    fn human_readable(&self) -> String {
        format!(
            "snapshot_id={} status={} source_filter={} file_count={} total_bytes={} max_file_id={} created_at={}",
            self.snapshot_id,
            self.status,
            self.source_filter.as_deref().unwrap_or("all"),
            self.file_count,
            self.total_bytes,
            self.max_file_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.created_at,
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogSnapshotCreateOutput {
    pub consistency: &'static str,
    pub snapshot: CatalogSnapshotView,
}

impl CatalogSnapshotCreateOutput {
    pub fn human_readable(&self) -> String {
        format!(
            "FOD indexer catalogue snapshot created\nconsistency: {}\n{}",
            self.consistency,
            self.snapshot.human_readable()
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogSnapshotListOutput {
    pub consistency: &'static str,
    pub sort: &'static str,
    pub limit: usize,
    pub cursor: Option<u64>,
    pub items: Vec<CatalogSnapshotView>,
    pub next_cursor: Option<u64>,
    pub total: u64,
}

impl CatalogSnapshotListOutput {
    pub fn human_readable(&self) -> String {
        let mut text = format!(
            "FOD indexer catalogue snapshots\nconsistency: {}\nsort: {}\nlimit: {}\ncursor: {}\ntotal: {}\nitems: {}",
            self.consistency,
            self.sort,
            self.limit,
            self.cursor
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.total,
            self.items.len(),
        );
        for item in &self.items {
            text.push_str("\n- ");
            text.push_str(&item.human_readable());
        }
        text.push_str(&format!(
            "\nnext_cursor: {}",
            self.next_cursor
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string())
        ));
        text
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogSnapshotShowOutput {
    pub consistency: &'static str,
    pub snapshot: CatalogSnapshotView,
}

impl CatalogSnapshotShowOutput {
    pub fn human_readable(&self) -> String {
        format!(
            "FOD indexer catalogue snapshot\nconsistency: {}\n{}",
            self.consistency,
            self.snapshot.human_readable()
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogSnapshotDeleteOutput {
    pub deleted: bool,
    pub snapshot_id: u64,
    pub deleted_file_count: u64,
}

impl CatalogSnapshotDeleteOutput {
    pub fn human_readable(&self) -> String {
        format!(
            "FOD indexer catalogue snapshot deleted\nsnapshot_id: {}\ndeleted: {}\ndeleted_file_count: {}",
            self.snapshot_id, self.deleted, self.deleted_file_count
        )
    }
}

pub fn capabilities_output() -> IndexerCapabilitiesOutput {
    let mut capabilities = file_read_api::capabilities_output();
    capabilities.commands.extend([
        CommandCapabilityView {
            command: "snapshot create",
            status: "available",
            read_only: false,
            filters: &["source"],
            sort: &["files: file_id ASC"],
            pagination: None,
            default_limit: None,
            max_limit: None,
            consistency: "stored-snapshot",
            notes: "Atomically copies the current catalogue rows into immutable snapshot tables. It does not scan sources, hash files, or modify source/index file rows.",
        },
        CommandCapabilityView {
            command: "snapshot list",
            status: "available",
            read_only: true,
            filters: &["cursor", "limit"],
            sort: &["snapshot_id DESC"],
            pagination: Some("keyset-cursor"),
            default_limit: Some(INDEXER_DEFAULT_PAGE_LIMIT),
            max_limit: Some(INDEXER_MAX_PAGE_LIMIT),
            consistency: "live-snapshot-metadata",
            notes: "Lists stored catalogue snapshot headers without changing them.",
        },
        CommandCapabilityView {
            command: "snapshot show --id",
            status: "available",
            read_only: true,
            filters: &["id"],
            sort: &[],
            pagination: None,
            default_limit: None,
            max_limit: None,
            consistency: "stored-snapshot",
            notes: "Reads one immutable catalogue snapshot header.",
        },
        CommandCapabilityView {
            command: "snapshot delete --id",
            status: "available",
            read_only: false,
            filters: &["id"],
            sort: &[],
            pagination: None,
            default_limit: None,
            max_limit: None,
            consistency: "stored-snapshot",
            notes: "Deletes only the selected stored snapshot and its copied rows; live catalogue and source files are untouched.",
        },
    ]);
    if let Some(command) = capabilities
        .commands
        .iter_mut()
        .find(|command| command.command == "file list|search|show")
    {
        command.filters = &[
            "file_id",
            "snapshot_id",
            "source",
            "path",
            "name",
            "extension",
            "file_kind",
            "scan_status",
            "hash_status",
            "min_size",
            "max_size",
            "mtime_from",
            "mtime_to",
            "cursor",
            "limit",
        ];
        command.consistency = "live-or-stored-snapshot";
        command.notes = "Lists, searches, or shows live indexed records by default, or immutable copied records when --snapshot-id is supplied.";
    }
    capabilities
}

pub fn create_catalog_snapshot(
    repo: &DbRepo,
    source: Option<&str>,
) -> Result<CatalogSnapshotCreateOutput, String> {
    ensure_snapshot_schema(repo, "snapshot create")?;
    let source = normalize_source_filter(source)?;
    if let Some(source_name) = source.as_deref() {
        let rows = repo.query_rows_text(&format!(
            "SELECT 1 FROM index_sources WHERE name = {} LIMIT 1",
            sql_quote_literal(source_name)
        ))?;
        if rows.is_empty() {
            return Err(format!(
                "catalog_snapshot_source_not_found: registered source {source_name:?} does not exist"
            ));
        }
    }

    let source_condition = source
        .as_deref()
        .map(|value| format!("WHERE s.name = {}", sql_quote_literal(value)))
        .unwrap_or_default();
    let source_literal = source
        .as_deref()
        .map(sql_quote_literal)
        .unwrap_or_else(|| "NULL".to_string());

    let rows = repo.query_rows_text(&format!(
        "
        WITH source_rows AS MATERIALIZED (
            SELECT
                f.id_file,
                s.id_index_source,
                s.name AS source_name,
                s.kind AS source_kind,
                s.root_path AS source_root,
                f.path,
                f.size,
                f.mtime_ns,
                f.inode,
                f.device,
                f.file_kind,
                f.scan_status,
                f.source_changed,
                h.hash_algorithm,
                h.full_hash,
                h.hash_status,
                f.id_scan_run,
                f.created_at AS file_created_at,
                f.updated_at AS file_updated_at
            FROM index_files f
            JOIN index_sources s ON s.id_index_source = f.id_index_source
            LEFT JOIN index_file_hashes h ON h.id_file = f.id_file
            {source_condition}
        ),
        created AS (
            INSERT INTO index_catalog_snapshots (
                status,
                source_filter,
                file_count,
                total_bytes,
                max_file_id
            )
            SELECT
                'complete',
                {source_literal},
                COUNT(*)::bigint,
                COALESCE(SUM(size), 0)::bigint,
                MAX(id_file)::bigint
            FROM source_rows
            RETURNING
                id_catalog_snapshot,
                status,
                source_filter,
                file_count,
                total_bytes,
                max_file_id,
                created_at
        ),
        copied AS (
            INSERT INTO index_catalog_snapshot_files (
                id_catalog_snapshot,
                id_file,
                id_index_source,
                source_name,
                source_kind,
                source_root,
                path,
                size,
                mtime_ns,
                inode,
                device,
                file_kind,
                scan_status,
                source_changed,
                hash_algorithm,
                full_hash,
                hash_status,
                id_scan_run,
                file_created_at,
                file_updated_at
            )
            SELECT
                created.id_catalog_snapshot,
                source_rows.id_file,
                source_rows.id_index_source,
                source_rows.source_name,
                source_rows.source_kind,
                source_rows.source_root,
                source_rows.path,
                source_rows.size,
                source_rows.mtime_ns,
                source_rows.inode,
                source_rows.device,
                source_rows.file_kind,
                source_rows.scan_status,
                source_rows.source_changed,
                source_rows.hash_algorithm,
                source_rows.full_hash,
                source_rows.hash_status,
                source_rows.id_scan_run,
                source_rows.file_created_at,
                source_rows.file_updated_at
            FROM source_rows
            CROSS JOIN created
            RETURNING id_file
        )
        SELECT
            created.id_catalog_snapshot::text,
            created.status,
            COALESCE(created.source_filter, ''),
            created.file_count::text,
            created.total_bytes::text,
            COALESCE(created.max_file_id::text, ''),
            created.created_at::text,
            (SELECT COUNT(*) FROM copied)::text
        FROM created
        "
    ))?;
    let row = rows
        .first()
        .ok_or_else(|| "catalog snapshot create did not return a snapshot".to_string())?;
    if row.len() < 8 {
        return Err("catalog snapshot create row is too short".to_string());
    }
    let snapshot = snapshot_from_row(&row[..7])?;
    let copied_count = parse_u64(&row[7], "copied snapshot file count")?;
    if copied_count != snapshot.file_count {
        return Err(format!(
            "catalog_snapshot_incomplete: expected {} copied files, got {copied_count}",
            snapshot.file_count
        ));
    }
    Ok(CatalogSnapshotCreateOutput {
        consistency: "stored-snapshot",
        snapshot,
    })
}

pub fn list_catalog_snapshots(
    repo: &DbRepo,
    limit: usize,
    cursor: Option<u64>,
) -> Result<CatalogSnapshotListOutput, String> {
    ensure_snapshot_schema(repo, "snapshot list")?;
    validate_list_request(limit, cursor)?;
    let fetch_limit = limit
        .checked_add(1)
        .ok_or_else(|| "snapshot list limit is too large".to_string())?;
    let where_clause = cursor
        .map(|value| format!("WHERE id_catalog_snapshot < {value}"))
        .unwrap_or_default();
    let rows = repo.query_rows_text(&format!(
        "
        SELECT
            id_catalog_snapshot::text,
            status,
            COALESCE(source_filter, ''),
            file_count::text,
            total_bytes::text,
            COALESCE(max_file_id::text, ''),
            created_at::text
        FROM index_catalog_snapshots
        {where_clause}
        ORDER BY id_catalog_snapshot DESC
        LIMIT {fetch_limit}
        "
    ))?;
    let total_rows = repo.query_rows_text("SELECT COUNT(*) FROM index_catalog_snapshots")?;
    let total = total_rows
        .first()
        .and_then(|row| row.first())
        .ok_or_else(|| "catalog snapshot total row is missing".to_string())
        .and_then(|value| parse_u64(value, "catalog snapshot total"))?;
    let mut items = rows
        .iter()
        .map(|row| snapshot_from_row(row))
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor = if has_more {
        items.last().map(|item| item.snapshot_id)
    } else {
        None
    };
    Ok(CatalogSnapshotListOutput {
        consistency: "live-snapshot-metadata",
        sort: "snapshot_id DESC",
        limit,
        cursor,
        items,
        next_cursor,
        total,
    })
}

pub fn show_catalog_snapshot(
    repo: &DbRepo,
    snapshot_id: u64,
) -> Result<CatalogSnapshotShowOutput, String> {
    ensure_snapshot_schema(repo, "snapshot show")?;
    validate_snapshot_id(snapshot_id, "snapshot show")?;
    let snapshot = load_snapshot(repo, snapshot_id)?;
    Ok(CatalogSnapshotShowOutput {
        consistency: "stored-snapshot",
        snapshot,
    })
}

pub fn delete_catalog_snapshot(
    repo: &DbRepo,
    snapshot_id: u64,
) -> Result<CatalogSnapshotDeleteOutput, String> {
    ensure_snapshot_schema(repo, "snapshot delete")?;
    validate_snapshot_id(snapshot_id, "snapshot delete")?;
    let rows = repo.query_rows_text(&format!(
        "
        DELETE FROM index_catalog_snapshots
        WHERE id_catalog_snapshot = {snapshot_id}
        RETURNING file_count::text
        "
    ))?;
    let row = rows.first().ok_or_else(|| {
        format!("catalog_snapshot_not_found: snapshot {snapshot_id} does not exist")
    })?;
    let deleted_file_count = row
        .first()
        .ok_or_else(|| "catalog snapshot delete row is too short".to_string())
        .and_then(|value| parse_u64(value, "deleted snapshot file count"))?;
    Ok(CatalogSnapshotDeleteOutput {
        deleted: true,
        snapshot_id,
        deleted_file_count,
    })
}

pub fn ensure_complete_snapshot(repo: &DbRepo, snapshot_id: u64) -> Result<(), String> {
    ensure_snapshot_schema(repo, "file catalogue --snapshot-id")?;
    validate_snapshot_id(snapshot_id, "file catalogue --snapshot-id")?;
    let snapshot = load_snapshot(repo, snapshot_id)?;
    if snapshot.status != "complete" {
        return Err(format!(
            "catalog_snapshot_incomplete: snapshot {snapshot_id} has status {}",
            snapshot.status
        ));
    }
    Ok(())
}

fn load_snapshot(repo: &DbRepo, snapshot_id: u64) -> Result<CatalogSnapshotView, String> {
    let rows = repo.query_rows_text(&format!(
        "
        SELECT
            id_catalog_snapshot::text,
            status,
            COALESCE(source_filter, ''),
            file_count::text,
            total_bytes::text,
            COALESCE(max_file_id::text, ''),
            created_at::text
        FROM index_catalog_snapshots
        WHERE id_catalog_snapshot = {snapshot_id}
        LIMIT 1
        "
    ))?;
    rows.first()
        .ok_or_else(|| format!("catalog_snapshot_not_found: snapshot {snapshot_id} does not exist"))
        .and_then(|row| snapshot_from_row(row))
}

fn ensure_snapshot_schema(repo: &DbRepo, operation: &str) -> Result<(), String> {
    let rows = repo.query_rows_text(
        "SELECT
            to_regclass('fod.index_catalog_snapshots') IS NOT NULL,
            to_regclass('fod.index_catalog_snapshot_files') IS NOT NULL",
    )?;
    let ready = rows
        .first()
        .is_some_and(|row| row.len() >= 2 && parse_bool(&row[0]) && parse_bool(&row[1]));
    if ready {
        Ok(())
    } else {
        Err(format!(
            "{operation} requires FOD database schema 19. Run `mkfs.fod upgrade` so migration 0019_index_catalog_snapshots.sql is applied, then retry."
        ))
    }
}

fn validate_list_request(limit: usize, cursor: Option<u64>) -> Result<(), String> {
    if !(1..=INDEXER_MAX_PAGE_LIMIT).contains(&limit) {
        return Err(format!(
            "snapshot list --limit must be between 1 and {INDEXER_MAX_PAGE_LIMIT}, got {limit}"
        ));
    }
    if matches!(cursor, Some(0)) {
        return Err("snapshot list --cursor must be a positive snapshot id".to_string());
    }
    Ok(())
}

fn validate_snapshot_id(snapshot_id: u64, operation: &str) -> Result<(), String> {
    if snapshot_id == 0 {
        Err(format!("{operation} --id must be a positive snapshot id"))
    } else {
        Ok(())
    }
}

fn normalize_source_filter(source: Option<&str>) -> Result<Option<String>, String> {
    match source {
        None => Ok(None),
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                Err("snapshot create --source must not be empty".to_string())
            } else {
                Ok(Some(value.to_string()))
            }
        }
    }
}

fn snapshot_from_row(row: &[String]) -> Result<CatalogSnapshotView, String> {
    if row.len() < 7 {
        return Err("catalog snapshot row is too short".to_string());
    }
    Ok(CatalogSnapshotView {
        snapshot_id: parse_u64(&row[0], "catalog snapshot id")?,
        status: row[1].clone(),
        source_filter: optional_text(&row[2]),
        file_count: parse_u64(&row[3], "catalog snapshot file count")?,
        total_bytes: parse_u64(&row[4], "catalog snapshot total bytes")?,
        max_file_id: parse_optional_u64(&row[5], "catalog snapshot max file id")?,
        created_at: row[6].clone(),
    })
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn parse_optional_u64(value: &str, label: &str) -> Result<Option<u64>, String> {
    if value.trim().is_empty() {
        Ok(None)
    } else {
        parse_u64(value, label).map(Some)
    }
}

fn parse_u64(value: &str, label: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("invalid {label}: {err}"))
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "t" | "true" | "1" | "on"
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotFileCatalogOutput {
    pub consistency: &'static str,
    pub snapshot_id: Option<u64>,
    pub sort: &'static str,
    pub limit: usize,
    pub cursor: Option<u64>,
    pub filters: crate::read_api::FileCatalogFilters,
    pub items: Vec<crate::read_api::FileCatalogItem>,
    pub next_cursor: Option<u64>,
    pub total: u64,
}

impl SnapshotFileCatalogOutput {
    pub fn from_live(output: crate::read_api::FileCatalogOutput) -> Self {
        Self {
            consistency: output.consistency,
            snapshot_id: None,
            sort: output.sort,
            limit: output.limit,
            cursor: output.cursor,
            filters: output.filters,
            items: output.items,
            next_cursor: output.next_cursor,
            total: output.total,
        }
    }

    pub fn human_readable(&self) -> String {
        let mut text = format!(
            "FOD indexer file catalogue\nconsistency: {}\nsnapshot_id: {}\nsort: {}\nlimit: {}\ncursor: {}\ntotal: {}\nitems: {}",
            self.consistency,
            self.snapshot_id.map(|v| v.to_string()).unwrap_or_else(|| "none".to_string()),
            self.sort,
            self.limit,
            self.cursor.map(|v| v.to_string()).unwrap_or_else(|| "none".to_string()),
            self.total,
            self.items.len(),
        );
        for item in &self.items {
            text.push_str(&format!(
                "\n- file_id={} source={} kind={} path={} size={} scan_status={} hash_status={}",
                item.file_id,
                item.source_name,
                item.source_kind,
                item.path,
                item.size,
                item.scan_status,
                item.hash_status.as_deref().unwrap_or("none"),
            ));
        }
        text.push_str(&format!(
            "\nnext_cursor: {}",
            self.next_cursor
                .map(|v| v.to_string())
                .unwrap_or_else(|| "none".to_string())
        ));
        text
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotFileShowOutput {
    pub consistency: &'static str,
    pub snapshot_id: Option<u64>,
    pub item: crate::read_api::FileCatalogItem,
}

impl SnapshotFileShowOutput {
    pub fn from_live(output: crate::read_api::FileShowOutput) -> Self {
        Self {
            consistency: output.consistency,
            snapshot_id: None,
            item: output.item,
        }
    }

    pub fn human_readable(&self) -> String {
        format!(
            "FOD indexer file\nconsistency: {}\nsnapshot_id: {}\nfile_id={} source={} kind={} source_root={} path={} source_path={} size={} mtime_ns={} inode={} device={} file_kind={} scan_status={} source_changed={} hash_algorithm={} full_hash={} hash_status={} scan_run_id={} created_at={} updated_at={}",
            self.consistency,
            self.snapshot_id.map(|v| v.to_string()).unwrap_or_else(|| "none".to_string()),
            self.item.file_id,
            self.item.source_name,
            self.item.source_kind,
            self.item.source_root,
            self.item.path,
            self.item.source_path,
            self.item.size,
            self.item.mtime_ns.map(|v| v.to_string()).unwrap_or_else(|| "none".to_string()),
            self.item.inode.map(|v| v.to_string()).unwrap_or_else(|| "none".to_string()),
            self.item.device.map(|v| v.to_string()).unwrap_or_else(|| "none".to_string()),
            self.item.file_kind,
            self.item.scan_status,
            self.item.source_changed,
            self.item.hash_algorithm.as_deref().unwrap_or("none"),
            self.item.full_hash_hex.as_deref().unwrap_or("none"),
            self.item.hash_status.as_deref().unwrap_or("none"),
            self.item.scan_run_id.map(|v| v.to_string()).unwrap_or_else(|| "none".to_string()),
            self.item.created_at,
            self.item.updated_at,
        )
    }
}

pub fn load_snapshot_file_list(
    repo: &DbRepo,
    snapshot_id: u64,
    limit: usize,
    cursor: Option<u64>,
    source: Option<&str>,
    file_kind: Option<&str>,
    scan_status: Option<&str>,
    hash_status: Option<&str>,
) -> Result<SnapshotFileCatalogOutput, String> {
    let filters = normalize_snapshot_filters(crate::read_api::FileCatalogFilters {
        source: owned_filter(source),
        file_kind: owned_filter(file_kind),
        scan_status: owned_filter(scan_status),
        hash_status: owned_filter(hash_status),
        ..crate::read_api::FileCatalogFilters::default()
    })?;
    load_snapshot_file_catalog(repo, snapshot_id, limit, cursor, filters, false)
}

#[allow(clippy::too_many_arguments)]
pub fn search_snapshot_files(
    repo: &DbRepo,
    snapshot_id: u64,
    limit: usize,
    cursor: Option<u64>,
    query: Option<&str>,
    path: Option<&str>,
    name: Option<&str>,
    source: Option<&str>,
    extension: Option<&str>,
    file_kind: Option<&str>,
    scan_status: Option<&str>,
    hash_status: Option<&str>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    mtime_from: Option<i64>,
    mtime_to: Option<i64>,
) -> Result<SnapshotFileCatalogOutput, String> {
    let filters = normalize_snapshot_filters(crate::read_api::FileCatalogFilters {
        query: owned_filter(query),
        path: owned_filter(path),
        name: owned_filter(name),
        source: owned_filter(source),
        extension: owned_filter(extension),
        file_kind: owned_filter(file_kind),
        scan_status: owned_filter(scan_status),
        hash_status: owned_filter(hash_status),
        min_size,
        max_size,
        mtime_from,
        mtime_to,
    })?;
    if snapshot_filters_empty(&filters) {
        return Err("file search requires at least one search filter".to_string());
    }
    load_snapshot_file_catalog(repo, snapshot_id, limit, cursor, filters, true)
}

pub fn show_snapshot_file(
    repo: &DbRepo,
    snapshot_id: u64,
    file_id: u64,
) -> Result<SnapshotFileShowOutput, String> {
    ensure_complete_snapshot(repo, snapshot_id)?;
    if file_id == 0 {
        return Err("file show --id must be a positive file id".to_string());
    }
    let rows = repo.query_rows_text(&format!(
        "{} WHERE f.id_catalog_snapshot = {} AND f.id_file = {} LIMIT 1",
        snapshot_file_select(),
        snapshot_id,
        file_id
    ))?;
    let row = rows.first().ok_or_else(|| {
        format!("indexed file {file_id} does not exist in catalogue snapshot {snapshot_id}")
    })?;
    Ok(SnapshotFileShowOutput {
        consistency: "stored-snapshot",
        snapshot_id: Some(snapshot_id),
        item: snapshot_file_item_from_row(row)?,
    })
}

fn load_snapshot_file_catalog(
    repo: &DbRepo,
    snapshot_id: u64,
    limit: usize,
    cursor: Option<u64>,
    filters: crate::read_api::FileCatalogFilters,
    search_mode: bool,
) -> Result<SnapshotFileCatalogOutput, String> {
    ensure_complete_snapshot(repo, snapshot_id)?;
    validate_snapshot_catalog_request(limit, cursor, &filters, search_mode)?;
    let mut conditions = snapshot_filter_conditions(&filters);
    conditions.insert(0, format!("f.id_catalog_snapshot = {snapshot_id}"));
    if let Some(cursor) = cursor {
        conditions.push(format!("f.id_file > {cursor}"));
    }
    let where_clause = format!("WHERE {}", conditions.join(" AND "));
    let fetch_limit = limit
        .checked_add(1)
        .ok_or_else(|| "file catalogue limit is too large".to_string())?;
    let rows = repo.query_rows_text(&format!(
        "{} {} ORDER BY f.id_file ASC LIMIT {}",
        snapshot_file_select(),
        where_clause,
        fetch_limit
    ))?;

    let mut total_conditions = snapshot_filter_conditions(&filters);
    total_conditions.insert(0, format!("f.id_catalog_snapshot = {snapshot_id}"));
    let total_rows = repo.query_rows_text(&format!(
        "SELECT COUNT(*) FROM index_catalog_snapshot_files f WHERE {}",
        total_conditions.join(" AND ")
    ))?;
    let total = total_rows
        .first()
        .and_then(|r| r.first())
        .ok_or_else(|| "snapshot file total row is missing".to_string())
        .and_then(|v| parse_u64(v, "snapshot file total"))?;

    let mut items = rows
        .iter()
        .map(|row| snapshot_file_item_from_row(row))
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor = if has_more {
        items.last().map(|item| item.file_id)
    } else {
        None
    };
    Ok(SnapshotFileCatalogOutput {
        consistency: "stored-snapshot",
        snapshot_id: Some(snapshot_id),
        sort: "file_id ASC",
        limit,
        cursor,
        filters,
        items,
        next_cursor,
        total,
    })
}

fn snapshot_file_select() -> &'static str {
    "SELECT f.id_file::text, f.id_index_source::text, f.source_name, f.source_kind, f.source_root, f.path, f.size::text, COALESCE(f.mtime_ns::text, ''), COALESCE(f.inode::text, ''), COALESCE(f.device::text, ''), f.file_kind, f.scan_status, f.source_changed::text, COALESCE(f.hash_algorithm, ''), COALESCE(encode(f.full_hash, 'hex'), ''), COALESCE(f.hash_status, ''), COALESCE(f.id_scan_run::text, ''), f.file_created_at::text, f.file_updated_at::text FROM index_catalog_snapshot_files f"
}

fn snapshot_filter_conditions(filters: &crate::read_api::FileCatalogFilters) -> Vec<String> {
    let mut conditions = Vec::new();
    if let Some(query) = filters.query.as_deref() {
        let literal = sql_quote_literal(query);
        conditions.push(format!("(POSITION(lower({literal}) IN lower(f.path)) > 0 OR POSITION(lower({literal}) IN lower(f.source_name)) > 0)"));
    }
    if let Some(path) = filters.path.as_deref() {
        conditions.push(format!(
            "POSITION(lower({}) IN lower(f.path)) > 0",
            sql_quote_literal(path)
        ));
    }
    if let Some(name) = filters.name.as_deref() {
        conditions.push(format!(
            "POSITION(lower({}) IN lower(substring(f.path from '[^/]+$'))) > 0",
            sql_quote_literal(name)
        ));
    }
    if let Some(source) = filters.source.as_deref() {
        conditions.push(format!("f.source_name = {}", sql_quote_literal(source)));
    }
    if let Some(extension) = filters.extension.as_deref() {
        conditions.push(format!(
            "lower(COALESCE(substring(f.path from '\\.([^./]+)$'), '')) = lower({})",
            sql_quote_literal(extension)
        ));
    }
    if let Some(value) = filters.file_kind.as_deref() {
        conditions.push(format!("f.file_kind = {}", sql_quote_literal(value)));
    }
    if let Some(value) = filters.scan_status.as_deref() {
        conditions.push(format!("f.scan_status = {}", sql_quote_literal(value)));
    }
    if let Some(value) = filters.hash_status.as_deref() {
        conditions.push(format!("f.hash_status = {}", sql_quote_literal(value)));
    }
    if let Some(value) = filters.min_size {
        conditions.push(format!("f.size >= {value}"));
    }
    if let Some(value) = filters.max_size {
        conditions.push(format!("f.size <= {value}"));
    }
    if let Some(value) = filters.mtime_from {
        conditions.push(format!("f.mtime_ns >= {value}"));
    }
    if let Some(value) = filters.mtime_to {
        conditions.push(format!("f.mtime_ns <= {value}"));
    }
    conditions
}

fn normalize_snapshot_filters(
    mut filters: crate::read_api::FileCatalogFilters,
) -> Result<crate::read_api::FileCatalogFilters, String> {
    for (label, value) in [
        ("query", &mut filters.query),
        ("path", &mut filters.path),
        ("name", &mut filters.name),
        ("source", &mut filters.source),
        ("file-kind", &mut filters.file_kind),
        ("scan-status", &mut filters.scan_status),
        ("hash-status", &mut filters.hash_status),
    ] {
        if let Some(text) = value.as_mut() {
            *text = text.trim().to_string();
            if text.is_empty() {
                return Err(format!("file filter --{label} must not be empty"));
            }
        }
    }
    if let Some(extension) = filters.extension.as_mut() {
        *extension = extension.trim().trim_start_matches('.').to_string();
        if extension.is_empty() {
            return Err("file filter --extension must not be empty".to_string());
        }
    }
    Ok(filters)
}

fn validate_snapshot_catalog_request(
    limit: usize,
    cursor: Option<u64>,
    filters: &crate::read_api::FileCatalogFilters,
    search_mode: bool,
) -> Result<(), String> {
    if !(1..=INDEXER_MAX_PAGE_LIMIT).contains(&limit) {
        return Err(format!(
            "file catalogue --limit must be between 1 and {INDEXER_MAX_PAGE_LIMIT}, got {limit}"
        ));
    }
    if matches!(cursor, Some(0)) {
        return Err("file catalogue --cursor must be a positive file id".to_string());
    }
    if let (Some(min), Some(max)) = (filters.min_size, filters.max_size) {
        if min > max {
            return Err("file search --min-size must not exceed --max-size".to_string());
        }
    }
    if let (Some(from), Some(to)) = (filters.mtime_from, filters.mtime_to) {
        if from > to {
            return Err("file search --mtime-from must not exceed --mtime-to".to_string());
        }
    }
    if search_mode && snapshot_filters_empty(filters) {
        return Err("file search requires at least one search filter".to_string());
    }
    Ok(())
}

fn snapshot_filters_empty(filters: &crate::read_api::FileCatalogFilters) -> bool {
    filters.query.is_none()
        && filters.path.is_none()
        && filters.name.is_none()
        && filters.source.is_none()
        && filters.extension.is_none()
        && filters.file_kind.is_none()
        && filters.scan_status.is_none()
        && filters.hash_status.is_none()
        && filters.min_size.is_none()
        && filters.max_size.is_none()
        && filters.mtime_from.is_none()
        && filters.mtime_to.is_none()
}

fn snapshot_file_item_from_row(row: &[String]) -> Result<crate::read_api::FileCatalogItem, String> {
    if row.len() < 19 {
        return Err("snapshot indexed file row is too short".to_string());
    }
    let source_root = row[4].clone();
    let path = row[5].clone();
    let path_view = std::path::Path::new(&path);
    let name = path_view
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or(path.as_str())
        .to_string();
    let extension = path_view
        .extension()
        .and_then(|v| v.to_str())
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    let source_path = std::path::Path::new(&source_root)
        .join(&path)
        .display()
        .to_string();
    Ok(crate::read_api::FileCatalogItem {
        file_id: parse_u64(&row[0], "file id")?,
        source_id: parse_u64(&row[1], "source id")?,
        source_name: row[2].clone(),
        source_kind: row[3].clone(),
        source_root,
        path,
        source_path,
        name,
        extension,
        size: parse_u64(&row[6], "file size")?,
        mtime_ns: parse_optional_i64_snapshot(&row[7], "file mtime_ns")?,
        inode: parse_optional_u64(&row[8], "file inode")?,
        device: parse_optional_u64(&row[9], "file device")?,
        file_kind: row[10].clone(),
        scan_status: row[11].clone(),
        source_changed: parse_bool(&row[12]),
        hash_algorithm: optional_text(&row[13]),
        full_hash_hex: optional_text(&row[14]),
        hash_status: optional_text(&row[15]),
        scan_run_id: parse_optional_u64(&row[16], "scan run id")?,
        created_at: row[17].clone(),
        updated_at: row[18].clone(),
    })
}

fn parse_optional_i64_snapshot(value: &str, label: &str) -> Result<Option<i64>, String> {
    if value.trim().is_empty() {
        Ok(None)
    } else {
        value
            .trim()
            .parse::<i64>()
            .map(Some)
            .map_err(|e| format!("invalid {label}: {e}"))
    }
}

fn owned_filter(value: Option<&str>) -> Option<String> {
    value.map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_snapshot_rows() {
        let row = vec![
            "12".to_string(),
            "complete".to_string(),
            "documents".to_string(),
            "25".to_string(),
            "4096".to_string(),
            "88".to_string(),
            "created".to_string(),
        ];
        let snapshot = snapshot_from_row(&row).expect("snapshot row should parse");
        assert_eq!(snapshot.snapshot_id, 12);
        assert_eq!(snapshot.source_filter.as_deref(), Some("documents"));
        assert_eq!(snapshot.file_count, 25);
        assert_eq!(snapshot.max_file_id, Some(88));
    }

    #[test]
    fn validates_snapshot_pagination_and_ids() {
        assert!(validate_list_request(100, None).is_ok());
        assert!(validate_list_request(0, None).is_err());
        assert!(validate_list_request(INDEXER_MAX_PAGE_LIMIT + 1, None).is_err());
        assert!(validate_list_request(10, Some(0)).is_err());
        assert!(validate_snapshot_id(1, "snapshot show").is_ok());
        assert!(validate_snapshot_id(0, "snapshot show").is_err());
    }

    #[test]
    fn exposes_snapshot_capabilities() {
        let capabilities = capabilities_output();
        let create = capabilities
            .commands
            .iter()
            .find(|command| command.command == "snapshot create")
            .expect("snapshot create capability should exist");
        assert!(!create.read_only);
        let show = capabilities
            .commands
            .iter()
            .find(|command| command.command == "snapshot show --id")
            .expect("snapshot show capability should exist");
        assert!(show.read_only);
    }
}
