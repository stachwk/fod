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
    let row = rows
        .first()
        .ok_or_else(|| format!("catalog_snapshot_not_found: snapshot {snapshot_id} does not exist"))?;
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
    let ready = rows.first().is_some_and(|row| {
        row.len() >= 2 && parse_bool(&row[0]) && parse_bool(&row[1])
    });
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
