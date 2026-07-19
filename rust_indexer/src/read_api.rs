use crate::db::sql_quote_literal;
use crate::output::{IndexerCapabilitiesOutput, INDEXER_MAX_PAGE_LIMIT};
use fod_rust_hotpath::pg::DbRepo;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImportPlanListItem {
    pub plan_id: u64,
    pub status: String,
    pub source_filter: Option<String>,
    pub dry_run: bool,
    pub created_at: String,
    pub updated_at: String,
    pub entry_count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPlanListOutput {
    pub consistency: &'static str,
    pub sort: &'static str,
    pub limit: usize,
    pub cursor: Option<u64>,
    pub status_filter: Option<String>,
    pub items: Vec<ImportPlanListItem>,
    pub next_cursor: Option<u64>,
    pub total: u64,
}

impl ImportPlanListOutput {
    pub fn human_readable(&self) -> String {
        let mut text = format!(
            "FOD indexer import plans\nconsistency: {}\nsort: {}\nlimit: {}\ncursor: {}\nstatus filter: {}\ntotal: {}\nitems: {}",
            self.consistency,
            self.sort,
            self.limit,
            self.cursor
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.status_filter.as_deref().unwrap_or("all"),
            self.total,
            self.items.len(),
        );
        for item in &self.items {
            text.push_str(&format!(
                "\n- plan_id={} status={} source_filter={} dry_run={} created_at={} updated_at={} entry_count={}",
                item.plan_id,
                item.status,
                item.source_filter.as_deref().unwrap_or("all"),
                item.dry_run,
                item.created_at,
                item.updated_at,
                item.entry_count,
            ));
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

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct FileCatalogFilters {
    pub query: Option<String>,
    pub path: Option<String>,
    pub name: Option<String>,
    pub source: Option<String>,
    pub extension: Option<String>,
    pub file_kind: Option<String>,
    pub scan_status: Option<String>,
    pub hash_status: Option<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub mtime_from: Option<i64>,
    pub mtime_to: Option<i64>,
}

impl FileCatalogFilters {
    fn is_empty(&self) -> bool {
        self.query.is_none()
            && self.path.is_none()
            && self.name.is_none()
            && self.source.is_none()
            && self.extension.is_none()
            && self.file_kind.is_none()
            && self.scan_status.is_none()
            && self.hash_status.is_none()
            && self.min_size.is_none()
            && self.max_size.is_none()
            && self.mtime_from.is_none()
            && self.mtime_to.is_none()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileCatalogItem {
    pub file_id: u64,
    pub source_id: u64,
    pub source_name: String,
    pub source_kind: String,
    pub source_root: String,
    pub path: String,
    pub source_path: String,
    pub name: String,
    pub extension: Option<String>,
    pub size: u64,
    pub mtime_ns: Option<i64>,
    pub inode: Option<u64>,
    pub device: Option<u64>,
    pub file_kind: String,
    pub scan_status: String,
    pub source_changed: bool,
    pub hash_algorithm: Option<String>,
    pub full_hash_hex: Option<String>,
    pub hash_status: Option<String>,
    pub scan_run_id: Option<u64>,
    pub created_at: String,
    pub updated_at: String,
}

impl FileCatalogItem {
    fn human_readable(&self) -> String {
        format!(
            "file_id={} source={} kind={} path={} size={} mtime_ns={} file_kind={} scan_status={} hash_status={} hash={} source_changed={} scan_run_id={}",
            self.file_id,
            self.source_name,
            self.source_kind,
            self.path,
            self.size,
            self.mtime_ns
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.file_kind,
            self.scan_status,
            self.hash_status.as_deref().unwrap_or("none"),
            self.full_hash_hex.as_deref().unwrap_or("none"),
            self.source_changed,
            self.scan_run_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FileCatalogOutput {
    pub consistency: &'static str,
    pub sort: &'static str,
    pub limit: usize,
    pub cursor: Option<u64>,
    pub filters: FileCatalogFilters,
    pub items: Vec<FileCatalogItem>,
    pub next_cursor: Option<u64>,
    pub total: u64,
}

impl FileCatalogOutput {
    pub fn human_readable(&self) -> String {
        let mut text = format!(
            "FOD indexer file catalogue\nconsistency: {}\nsort: {}\nlimit: {}\ncursor: {}\ntotal: {}\nitems: {}",
            self.consistency,
            self.sort,
            self.limit,
            self.cursor
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.total,
            self.items.len(),
        );
        if !self.filters.is_empty() {
            text.push_str(&format!("\nfilters: {:?}", self.filters));
        }
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
pub struct FileShowOutput {
    pub consistency: &'static str,
    pub item: FileCatalogItem,
}

impl FileShowOutput {
    pub fn human_readable(&self) -> String {
        format!(
            "FOD indexer file\nconsistency: {}\n{}\nsource_root={}\nsource_path={}\nname={}\nextension={}\nhash_algorithm={}\ncreated_at={}\nupdated_at={}",
            self.consistency,
            self.item.human_readable(),
            self.item.source_root,
            self.item.source_path,
            self.item.name,
            self.item.extension.as_deref().unwrap_or("none"),
            self.item.hash_algorithm.as_deref().unwrap_or("none"),
            self.item.created_at,
            self.item.updated_at,
        )
    }
}

pub fn capabilities_output() -> IndexerCapabilitiesOutput {
    let mut capabilities = IndexerCapabilitiesOutput::current();
    if let Some(command) = capabilities
        .commands
        .iter_mut()
        .find(|command| command.command == "plan list")
    {
        command.status = "available";
        command.notes =
            "Lists stored import plans without creating, refreshing, or modifying them.";
    }
    if let Some(command) = capabilities
        .commands
        .iter_mut()
        .find(|command| command.command == "file list|search|show")
    {
        command.status = "available";
        command.notes = "Lists, searches, or shows existing indexed file records without scanning, hashing, refreshing, or modifying index state. MIME and extracted-text metadata remain outside fod-indexer.";
    }
    capabilities
}

pub fn load_import_plan_list(
    repo: &DbRepo,
    limit: usize,
    cursor: Option<u64>,
    status: Option<&str>,
) -> Result<ImportPlanListOutput, String> {
    validate_plan_list_request(limit, cursor, status)?;

    let normalized_status = status.map(str::trim).filter(|value| !value.is_empty());
    let mut page_conditions = Vec::new();
    if let Some(cursor) = cursor {
        page_conditions.push(format!("p.id_import_plan < {cursor}"));
    }
    if let Some(status) = normalized_status {
        page_conditions.push(format!("p.status = {}", sql_quote_literal(status)));
    }
    let page_where = where_clause(&page_conditions);

    let fetch_limit = limit
        .checked_add(1)
        .ok_or_else(|| "plan list limit is too large".to_string())?;
    let rows = repo.query_rows_text(&format!(
        "
        SELECT
            p.id_import_plan,
            p.status,
            COALESCE(p.source_filter, ''),
            p.dry_run::text,
            p.created_at::text,
            p.updated_at::text,
            COUNT(e.id_import_plan_entry)
        FROM index_import_plans p
        LEFT JOIN index_import_plan_entries e
            ON e.id_import_plan = p.id_import_plan
        {page_where}
        GROUP BY
            p.id_import_plan,
            p.status,
            p.source_filter,
            p.dry_run,
            p.created_at,
            p.updated_at
        ORDER BY p.id_import_plan DESC
        LIMIT {fetch_limit}
        "
    ))?;

    let total_conditions = normalized_status
        .map(|status| vec![format!("status = {}", sql_quote_literal(status))])
        .unwrap_or_default();
    let total_where = where_clause(&total_conditions);
    let total_rows = repo.query_rows_text(&format!(
        "
        SELECT COUNT(*)
        FROM index_import_plans
        {total_where}
        "
    ))?;
    let total = parse_count(&total_rows, "import plan total")?;

    let mut items = rows
        .iter()
        .map(|row| import_plan_list_item_from_row(row))
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor = if has_more {
        items.last().map(|item| item.plan_id)
    } else {
        None
    };

    Ok(ImportPlanListOutput {
        consistency: "live",
        sort: "plan_id DESC",
        limit,
        cursor,
        status_filter: normalized_status.map(str::to_string),
        items,
        next_cursor,
        total,
    })
}

pub fn load_file_list(
    repo: &DbRepo,
    limit: usize,
    cursor: Option<u64>,
    source: Option<&str>,
    file_kind: Option<&str>,
    scan_status: Option<&str>,
    hash_status: Option<&str>,
) -> Result<FileCatalogOutput, String> {
    let filters = normalize_file_filters(FileCatalogFilters {
        source: owned_filter(source),
        file_kind: owned_filter(file_kind),
        scan_status: owned_filter(scan_status),
        hash_status: owned_filter(hash_status),
        ..FileCatalogFilters::default()
    })?;
    load_file_catalog(repo, limit, cursor, filters, false)
}

#[allow(clippy::too_many_arguments)]
pub fn search_files(
    repo: &DbRepo,
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
) -> Result<FileCatalogOutput, String> {
    let filters = normalize_file_filters(FileCatalogFilters {
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
    if filters.is_empty() {
        return Err("file search requires at least one search filter".to_string());
    }
    load_file_catalog(repo, limit, cursor, filters, true)
}

pub fn show_file(repo: &DbRepo, id: u64) -> Result<FileShowOutput, String> {
    if id == 0 {
        return Err("file show --id must be a positive file id".to_string());
    }
    let rows = repo.query_rows_text(&file_catalog_query(
        &format!("WHERE f.id_file = {id}"),
        "LIMIT 1",
    ))?;
    let row = rows
        .first()
        .ok_or_else(|| format!("indexed file {id} does not exist"))?;
    Ok(FileShowOutput {
        consistency: "live",
        item: file_catalog_item_from_row(row)?,
    })
}

fn load_file_catalog(
    repo: &DbRepo,
    limit: usize,
    cursor: Option<u64>,
    filters: FileCatalogFilters,
    search_mode: bool,
) -> Result<FileCatalogOutput, String> {
    validate_file_catalog_request(limit, cursor, &filters, search_mode)?;

    let mut page_conditions = file_filter_conditions(&filters);
    if let Some(cursor) = cursor {
        page_conditions.push(format!("f.id_file > {cursor}"));
    }
    let page_where = where_clause(&page_conditions);
    let fetch_limit = limit
        .checked_add(1)
        .ok_or_else(|| "file catalogue limit is too large".to_string())?;
    let rows = repo.query_rows_text(&file_catalog_query(
        &page_where,
        &format!("LIMIT {fetch_limit}"),
    ))?;

    let total_where = where_clause(&file_filter_conditions(&filters));
    let total_rows = repo.query_rows_text(&format!(
        "
        SELECT COUNT(*)
        FROM index_files f
        JOIN index_sources s ON s.id_index_source = f.id_index_source
        LEFT JOIN index_file_hashes h ON h.id_file = f.id_file
        {total_where}
        "
    ))?;
    let total = parse_count(&total_rows, "indexed file total")?;

    let mut items = rows
        .iter()
        .map(|row| file_catalog_item_from_row(row))
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

    Ok(FileCatalogOutput {
        consistency: "live",
        sort: "file_id ASC",
        limit,
        cursor,
        filters,
        items,
        next_cursor,
        total,
    })
}

fn file_catalog_query(where_clause: &str, limit_clause: &str) -> String {
    format!(
        "
        SELECT
            f.id_file::text,
            s.id_index_source::text,
            s.name,
            s.kind,
            s.root_path,
            f.path,
            f.size::text,
            COALESCE(f.mtime_ns::text, ''),
            COALESCE(f.inode::text, ''),
            COALESCE(f.device::text, ''),
            f.file_kind,
            f.scan_status,
            f.source_changed::text,
            COALESCE(h.hash_algorithm, ''),
            COALESCE(encode(h.full_hash, 'hex'), ''),
            COALESCE(h.hash_status, ''),
            COALESCE(f.id_scan_run::text, ''),
            f.created_at::text,
            f.updated_at::text
        FROM index_files f
        JOIN index_sources s ON s.id_index_source = f.id_index_source
        LEFT JOIN index_file_hashes h ON h.id_file = f.id_file
        {where_clause}
        ORDER BY f.id_file ASC
        {limit_clause}
        "
    )
}

fn file_filter_conditions(filters: &FileCatalogFilters) -> Vec<String> {
    let mut conditions = Vec::new();
    if let Some(query) = filters.query.as_deref() {
        let literal = sql_quote_literal(query);
        conditions.push(format!(
            "(POSITION(lower({literal}) IN lower(f.path)) > 0 OR POSITION(lower({literal}) IN lower(s.name)) > 0)"
        ));
    }
    if let Some(path) = filters.path.as_deref() {
        let literal = sql_quote_literal(path);
        conditions.push(format!("POSITION(lower({literal}) IN lower(f.path)) > 0"));
    }
    if let Some(name) = filters.name.as_deref() {
        let literal = sql_quote_literal(name);
        conditions.push(format!(
            "POSITION(lower({literal}) IN lower(substring(f.path from '[^/]+$'))) > 0"
        ));
    }
    if let Some(source) = filters.source.as_deref() {
        conditions.push(format!("s.name = {}", sql_quote_literal(source)));
    }
    if let Some(extension) = filters.extension.as_deref() {
        conditions.push(format!(
            "lower(COALESCE(substring(f.path from '\\.([^./]+)$'), '')) = lower({})",
            sql_quote_literal(extension)
        ));
    }
    if let Some(file_kind) = filters.file_kind.as_deref() {
        conditions.push(format!("f.file_kind = {}", sql_quote_literal(file_kind)));
    }
    if let Some(scan_status) = filters.scan_status.as_deref() {
        conditions.push(format!(
            "f.scan_status = {}",
            sql_quote_literal(scan_status)
        ));
    }
    if let Some(hash_status) = filters.hash_status.as_deref() {
        conditions.push(format!(
            "h.hash_status = {}",
            sql_quote_literal(hash_status)
        ));
    }
    if let Some(min_size) = filters.min_size {
        conditions.push(format!("f.size >= {min_size}"));
    }
    if let Some(max_size) = filters.max_size {
        conditions.push(format!("f.size <= {max_size}"));
    }
    if let Some(mtime_from) = filters.mtime_from {
        conditions.push(format!("f.mtime_ns >= {mtime_from}"));
    }
    if let Some(mtime_to) = filters.mtime_to {
        conditions.push(format!("f.mtime_ns <= {mtime_to}"));
    }
    conditions
}

fn normalize_file_filters(mut filters: FileCatalogFilters) -> Result<FileCatalogFilters, String> {
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

fn validate_file_catalog_request(
    limit: usize,
    cursor: Option<u64>,
    filters: &FileCatalogFilters,
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
    if let (Some(min_size), Some(max_size)) = (filters.min_size, filters.max_size) {
        if min_size > max_size {
            return Err("file search --min-size must not exceed --max-size".to_string());
        }
    }
    if let (Some(mtime_from), Some(mtime_to)) = (filters.mtime_from, filters.mtime_to) {
        if mtime_from > mtime_to {
            return Err("file search --mtime-from must not exceed --mtime-to".to_string());
        }
    }
    if search_mode && filters.is_empty() {
        return Err("file search requires at least one search filter".to_string());
    }
    Ok(())
}

fn file_catalog_item_from_row(row: &[String]) -> Result<FileCatalogItem, String> {
    if row.len() < 19 {
        return Err("indexed file row is too short".to_string());
    }
    let source_root = row[4].clone();
    let path = row[5].clone();
    let path_view = Path::new(&path);
    let name = path_view
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path.as_str())
        .to_string();
    let extension = path_view
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let source_path = Path::new(&source_root).join(&path).display().to_string();

    Ok(FileCatalogItem {
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
        mtime_ns: parse_optional_i64(&row[7], "file mtime_ns")?,
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

fn validate_plan_list_request(
    limit: usize,
    cursor: Option<u64>,
    status: Option<&str>,
) -> Result<(), String> {
    if !(1..=INDEXER_MAX_PAGE_LIMIT).contains(&limit) {
        return Err(format!(
            "plan list --limit must be between 1 and {INDEXER_MAX_PAGE_LIMIT}, got {limit}"
        ));
    }
    if matches!(cursor, Some(0)) {
        return Err("plan list --cursor must be a positive plan id".to_string());
    }
    if status.is_some_and(|value| value.trim().is_empty()) {
        return Err("plan list --status must not be empty".to_string());
    }
    Ok(())
}

fn import_plan_list_item_from_row(row: &[String]) -> Result<ImportPlanListItem, String> {
    if row.len() < 7 {
        return Err("import plan list row is too short".to_string());
    }
    Ok(ImportPlanListItem {
        plan_id: parse_u64(&row[0], "import plan id")?,
        status: row[1].clone(),
        source_filter: optional_text(&row[2]),
        dry_run: parse_bool(&row[3]),
        created_at: row[4].clone(),
        updated_at: row[5].clone(),
        entry_count: parse_u64(&row[6], "import plan entry count")?,
    })
}

fn where_clause(conditions: &[String]) -> String {
    if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    }
}

fn parse_count(rows: &[Vec<String>], label: &str) -> Result<u64, String> {
    rows.first()
        .and_then(|row| row.first())
        .ok_or_else(|| format!("{label} row is missing"))
        .and_then(|value| parse_u64(value, label))
}

fn owned_filter(value: Option<&str>) -> Option<String> {
    value.map(str::to_string)
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_u64(value: &str, label: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("invalid {label}: {err}"))
}

fn parse_optional_u64(value: &str, label: &str) -> Result<Option<u64>, String> {
    if value.trim().is_empty() {
        Ok(None)
    } else {
        parse_u64(value, label).map(Some)
    }
}

fn parse_optional_i64(value: &str, label: &str) -> Result<Option<i64>, String> {
    if value.trim().is_empty() {
        Ok(None)
    } else {
        value
            .trim()
            .parse::<i64>()
            .map(Some)
            .map_err(|err| format!("invalid {label}: {err}"))
    }
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
    fn parses_import_plan_list_rows() {
        let row = vec![
            "42".to_string(),
            "dry_run_completed".to_string(),
            "lt7300_Documents".to_string(),
            "true".to_string(),
            "2026-07-19 20:00:00+00".to_string(),
            "2026-07-19 20:01:00+00".to_string(),
            "135".to_string(),
        ];
        let item = import_plan_list_item_from_row(&row).expect("row should parse");
        assert_eq!(item.plan_id, 42);
        assert_eq!(item.status, "dry_run_completed");
        assert_eq!(item.source_filter.as_deref(), Some("lt7300_Documents"));
        assert!(item.dry_run);
        assert_eq!(item.entry_count, 135);
    }

    #[test]
    fn parses_empty_source_filter_as_none() {
        let row = vec![
            "7".to_string(),
            "planned".to_string(),
            String::new(),
            "false".to_string(),
            "created".to_string(),
            "updated".to_string(),
            "0".to_string(),
        ];
        let item = import_plan_list_item_from_row(&row).expect("row should parse");
        assert_eq!(item.source_filter, None);
        assert!(!item.dry_run);
    }

    #[test]
    fn validates_plan_list_limits_and_cursor() {
        assert!(
            validate_plan_list_request(crate::output::INDEXER_DEFAULT_PAGE_LIMIT, None, None)
                .is_ok()
        );
        assert!(validate_plan_list_request(0, None, None).is_err());
        assert!(validate_plan_list_request(INDEXER_MAX_PAGE_LIMIT + 1, None, None).is_err());
        assert!(validate_plan_list_request(10, Some(0), None).is_err());
        assert!(validate_plan_list_request(10, None, Some(" ")).is_err());
    }

    #[test]
    fn parses_indexed_file_rows_and_derives_path_fields() {
        let row = vec![
            "17".to_string(),
            "3".to_string(),
            "lt7300_Documents".to_string(),
            "local".to_string(),
            "/home/wojtek/Documents".to_string(),
            "reports/annual.PDF".to_string(),
            "4096".to_string(),
            "123456789".to_string(),
            "55".to_string(),
            "8".to_string(),
            "regular".to_string(),
            "ok".to_string(),
            "false".to_string(),
            "sha256".to_string(),
            "abcdef".to_string(),
            "full".to_string(),
            "9".to_string(),
            "created".to_string(),
            "updated".to_string(),
        ];
        let item = file_catalog_item_from_row(&row).expect("file row should parse");
        assert_eq!(item.file_id, 17);
        assert_eq!(item.name, "annual.PDF");
        assert_eq!(item.extension.as_deref(), Some("PDF"));
        assert_eq!(
            item.source_path,
            "/home/wojtek/Documents/reports/annual.PDF"
        );
        assert_eq!(item.full_hash_hex.as_deref(), Some("abcdef"));
        assert_eq!(item.scan_run_id, Some(9));
    }

    #[test]
    fn validates_file_search_filters_and_ranges() {
        let empty = FileCatalogFilters::default();
        assert!(validate_file_catalog_request(100, None, &empty, false).is_ok());
        assert!(validate_file_catalog_request(100, None, &empty, true).is_err());
        assert!(validate_file_catalog_request(0, None, &empty, false).is_err());
        assert!(validate_file_catalog_request(10, Some(0), &empty, false).is_err());

        let invalid_size = FileCatalogFilters {
            min_size: Some(10),
            max_size: Some(5),
            ..FileCatalogFilters::default()
        };
        assert!(validate_file_catalog_request(10, None, &invalid_size, true).is_err());

        let invalid_mtime = FileCatalogFilters {
            mtime_from: Some(20),
            mtime_to: Some(10),
            ..FileCatalogFilters::default()
        };
        assert!(validate_file_catalog_request(10, None, &invalid_mtime, true).is_err());
    }

    #[test]
    fn normalizes_file_extensions() {
        let filters = normalize_file_filters(FileCatalogFilters {
            extension: Some(" .PDF ".to_string()),
            ..FileCatalogFilters::default()
        })
        .expect("extension should normalize");
        assert_eq!(filters.extension.as_deref(), Some("PDF"));
    }

    #[test]
    fn exposes_read_only_file_catalog_as_available_capability() {
        let capabilities = capabilities_output();
        let file_api = capabilities
            .commands
            .iter()
            .find(|command| command.command == "file list|search|show")
            .expect("file catalogue capability should exist");
        assert_eq!(file_api.status, "available");
        assert!(file_api.read_only);
    }

    #[test]
    fn exposes_plan_list_as_available_capability() {
        let capabilities = capabilities_output();
        let plan_list = capabilities
            .commands
            .iter()
            .find(|command| command.command == "plan list")
            .expect("plan list capability should exist");
        assert_eq!(plan_list.status, "available");
        assert!(plan_list.read_only);
    }
}
