from pathlib import Path
import re


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, got {count}: {old[:100]!r}")
    target.write_text(text.replace(old, new, 1))


def insert_before(path: str, marker: str, content: str) -> None:
    replace_once(path, marker, content + marker)


# Release and database schema versions.
replace_once("Cargo.toml", 'version = "3.2.20"', 'version = "3.2.21"')
Path("fod_version.txt").write_text("3.2.21\n")
replace_once(
    "rust_indexer/src/output.rs",
    "pub const INDEXER_REQUIRED_DATABASE_SCHEMA_VERSION: u32 = 18;",
    "pub const INDEXER_REQUIRED_DATABASE_SCHEMA_VERSION: u32 = 19;",
)

# Register migration 19 in mkfs.
replace_once("rust_mkfs/src/main.rs", "const SCHEMA_VERSION: u64 = 18;", "const SCHEMA_VERSION: u64 = 19;")
replace_once(
    "rust_mkfs/src/main.rs",
    "const MIGRATION_FILES: [&str; 18] = [",
    "const MIGRATION_FILES: [&str; 19] = [",
)
replace_once(
    "rust_mkfs/src/main.rs",
    '    "0018_payload_capacity_reservations.sql",\n];',
    '    "0018_payload_capacity_reservations.sql",\n    "0019_index_catalog_snapshots.sql",\n];',
)
replace_once(
    "rust_mkfs/src/main.rs",
    "const MIGRATION_DESCRIPTIONS: [&str; 18] = [",
    "const MIGRATION_DESCRIPTIONS: [&str; 19] = [",
)
replace_once(
    "rust_mkfs/src/main.rs",
    '    "Add transactional payload capacity reservations",\n];',
    '    "Add transactional payload capacity reservations",\n    "Add immutable index catalogue snapshots",\n];',
)
replace_once(
    "rust_mkfs/src/main.rs",
    '''        18 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0018_payload_capacity_reservations.sql"
        )),
        _ => "",
''',
    '''        18 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0018_payload_capacity_reservations.sql"
        )),
        19 => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../migrations/0019_index_catalog_snapshots.sql"
        )),
        _ => "",
''',
)
replace_once(
    "rust_mkfs/src/main.rs",
    '''        18 => MIGRATION_DESCRIPTIONS[17],
        _ => "Migration",
''',
    '''        18 => MIGRATION_DESCRIPTIONS[17],
        19 => MIGRATION_DESCRIPTIONS[18],
        _ => "Migration",
''',
)
replace_once(
    "rust_mkfs/src/main.rs",
    '''        18 => MIGRATION_FILES[17],
        _ => "unknown.sql",
''',
    '''        18 => MIGRATION_FILES[17],
        19 => MIGRATION_FILES[18],
        _ => "unknown.sql",
''',
)
replace_once(
    "rust_mkfs/src/main.rs",
    '''                    ('index_import_plan_entries'),
                    ('client_sessions''',
    '''                    ('index_import_plan_entries'),
                    ('index_catalog_snapshots'),
                    ('index_catalog_snapshot_files'),
                    ('client_sessions''',
)

# Fresh installs get the same schema as migration 19.
migration = Path("migrations/0019_index_catalog_snapshots.sql").read_text()
base_append = migration.replace("SET search_path TO fod, public;\n\n", "")
base = Path("migrations/base_schema.sql")
base.write_text(base.read_text().rstrip() + "\n\n" + base_append)

# CLI: snapshot commands and --snapshot-id for immutable catalogue reads.
replace_once(
    "rust_indexer/src/cli.rs",
    "  fod-indexer file read --id 42 --offset 0 --length 65536\\n  fod-indexer duplicate-set list --limit 100",
    "  fod-indexer file read --id 42 --offset 0 --length 65536\\n  fod-indexer snapshot create\\n  fod-indexer snapshot list\\n  fod-indexer file list --snapshot-id 12 --limit 100\\n  fod-indexer duplicate-set list --limit 100",
)
replace_once(
    "rust_indexer/src/cli.rs",
    '''    DuplicateSet {
        #[command(subcommand)]
        command: DuplicateSetCommands,
    },
    #[command(
        about = "Manage sources.",''',
    '''    DuplicateSet {
        #[command(subcommand)]
        command: DuplicateSetCommands,
    },
    #[command(
        about = "Manage immutable catalogue snapshots.",
        long_about = "Create, list, inspect, or delete immutable copies of the indexed-file catalogue.\n\nSnapshot creation copies existing index metadata only. It does not scan sources, hash files, rebuild duplicates, or read file contents. file list/search/show can later read a stored snapshot with --snapshot-id."
    )]
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommands,
    },
    #[command(
        about = "Manage sources.",''',
)
insert_before(
    "rust_indexer/src/cli.rs",
    "#[derive(Debug, Clone, Subcommand)]\npub enum DuplicateSetCommands",
    '''#[derive(Debug, Clone, Subcommand)]
pub enum SnapshotCommands {
    #[command(
        about = "Create an immutable catalogue snapshot.",
        long_about = "Atomically copy the current indexed-file catalogue into immutable snapshot tables.\n\nUse --source to capture one registered source only. The command changes snapshot tables but does not modify live file records or source files."
    )]
    Create {
        #[arg(long)]
        source: Option<String>,
    },
    #[command(
        about = "List catalogue snapshots.",
        long_about = "List stored catalogue snapshot headers in descending snapshot-id order. Pass next_cursor as --cursor to continue."
    )]
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        cursor: Option<u64>,
    },
    #[command(about = "Show one catalogue snapshot.")]
    Show {
        #[arg(long)]
        id: u64,
    },
    #[command(
        about = "Delete one catalogue snapshot.",
        long_about = "Delete only the selected stored snapshot and its copied rows. Live catalogue rows and source files are not changed."
    )]
    Delete {
        #[arg(long)]
        id: u64,
    },
}

''',
)
replace_once(
    "rust_indexer/src/cli.rs",
    '''        #[arg(long)]
        cursor: Option<u64>,
        #[arg(long)]
        source: Option<String>,''',
    '''        #[arg(long)]
        cursor: Option<u64>,
        #[arg(long)]
        snapshot_id: Option<u64>,
        #[arg(long)]
        source: Option<String>,''',
)
# This occurrence is the Search cursor, after all search filters.
replace_once(
    "rust_indexer/src/cli.rs",
    '''        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        cursor: Option<u64>,
    },
    #[command(
        about = "Show one indexed file.",''',
    '''        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        cursor: Option<u64>,
        #[arg(long)]
        snapshot_id: Option<u64>,
    },
    #[command(
        about = "Show one indexed file.",''',
)
replace_once(
    "rust_indexer/src/cli.rs",
    '''    Show {
        #[arg(long)]
        id: u64,
    },
    #[command(
        about = "Read revalidated source bytes.",''',
    '''    Show {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        snapshot_id: Option<u64>,
    },
    #[command(
        about = "Read revalidated source bytes.",''',
)
replace_once(
    "rust_indexer/src/cli.rs",
    '''            "capabilities" | "file" | "duplicate-set" | "source" | "report" | "plan"
            | "cleanup-failed" => return None,''',
    '''            "capabilities" | "file" | "duplicate-set" | "snapshot" | "source" | "report"
            | "plan" | "cleanup-failed" => return None,''',
)

# Main dispatch.
replace_once(
    "rust_indexer/src/main.rs",
    "mod scan;\nmod source;",
    "mod scan;\nmod snapshot_api;\nmod source;",
)
replace_once(
    "rust_indexer/src/main.rs",
    '''    Cli, Commands, DuplicateSetCommands, FileCommands, PlanCommands, ReportCommands,
    SourceCommands, SourceKind,
''',
    '''    Cli, Commands, DuplicateSetCommands, FileCommands, PlanCommands, ReportCommands,
    SnapshotCommands, SourceCommands, SourceKind,
''',
)
replace_once(
    "rust_indexer/src/main.rs",
    "let capabilities = file_read_api::capabilities_output();",
    "let capabilities = snapshot_api::capabilities_output();",
)
replace_once(
    "rust_indexer/src/main.rs",
    '''        Commands::DuplicateSet { command } => match command {
            DuplicateSetCommands::List { limit, cursor } => {
                let sets = duplicate_set_api::load_duplicate_set_list(&repo, limit, cursor)?;
                if output.is_json() {
                    print_json(&sets)?;
                } else {
                    println!("{}", sets.human_readable());
                }
                Ok(())
            }
        },
        Commands::File { command } => match command {''',
    '''        Commands::DuplicateSet { command } => match command {
            DuplicateSetCommands::List { limit, cursor } => {
                let sets = duplicate_set_api::load_duplicate_set_list(&repo, limit, cursor)?;
                if output.is_json() {
                    print_json(&sets)?;
                } else {
                    println!("{}", sets.human_readable());
                }
                Ok(())
            }
        },
        Commands::Snapshot { command } => match command {
            SnapshotCommands::Create { source } => {
                let snapshot = snapshot_api::create_catalog_snapshot(&repo, source.as_deref())?;
                if output.is_json() {
                    print_json(&snapshot)?;
                } else {
                    println!("{}", snapshot.human_readable());
                }
                Ok(())
            }
            SnapshotCommands::List { limit, cursor } => {
                let snapshots = snapshot_api::list_catalog_snapshots(&repo, limit, cursor)?;
                if output.is_json() {
                    print_json(&snapshots)?;
                } else {
                    println!("{}", snapshots.human_readable());
                }
                Ok(())
            }
            SnapshotCommands::Show { id } => {
                let snapshot = snapshot_api::show_catalog_snapshot(&repo, id)?;
                if output.is_json() {
                    print_json(&snapshot)?;
                } else {
                    println!("{}", snapshot.human_readable());
                }
                Ok(())
            }
            SnapshotCommands::Delete { id } => {
                let deleted = snapshot_api::delete_catalog_snapshot(&repo, id)?;
                if output.is_json() {
                    print_json(&deleted)?;
                } else {
                    println!("{}", deleted.human_readable());
                }
                Ok(())
            }
        },
        Commands::File { command } => match command {''',
)
replace_once(
    "rust_indexer/src/main.rs",
    '''                cursor,
                source,
                file_kind,''',
    '''                cursor,
                snapshot_id,
                source,
                file_kind,''',
)
replace_once(
    "rust_indexer/src/main.rs",
    '''                let files = read_api::load_file_list(
                    &repo,
                    limit,
                    cursor,
                    source.as_deref(),
                    file_kind.as_deref(),
                    scan_status.as_deref(),
                    hash_status.as_deref(),
                )?;''',
    '''                let files = if let Some(snapshot_id) = snapshot_id {
                    snapshot_api::load_snapshot_file_list(
                        &repo,
                        snapshot_id,
                        limit,
                        cursor,
                        source.as_deref(),
                        file_kind.as_deref(),
                        scan_status.as_deref(),
                        hash_status.as_deref(),
                    )?
                } else {
                    snapshot_api::SnapshotFileCatalogOutput::from_live(
                        read_api::load_file_list(
                            &repo,
                            limit,
                            cursor,
                            source.as_deref(),
                            file_kind.as_deref(),
                            scan_status.as_deref(),
                            hash_status.as_deref(),
                        )?,
                    )
                };''',
)
# Add snapshot_id in Search destructuring.
replace_once(
    "rust_indexer/src/main.rs",
    '''                limit,
                cursor,
            } => {
                let files = read_api::search_files(''',
    '''                limit,
                cursor,
                snapshot_id,
            } => {
                let files = if let Some(snapshot_id) = snapshot_id {
                    snapshot_api::search_snapshot_files(
                        &repo,
                        snapshot_id,
                        limit,
                        cursor,
                        query.as_deref(),
                        path.as_deref(),
                        name.as_deref(),
                        source.as_deref(),
                        extension.as_deref(),
                        file_kind.as_deref(),
                        scan_status.as_deref(),
                        hash_status.as_deref(),
                        min_size,
                        max_size,
                        mtime_from,
                        mtime_to,
                    )?
                } else {
                    snapshot_api::SnapshotFileCatalogOutput::from_live(read_api::search_files(''',
)
replace_once(
    "rust_indexer/src/main.rs",
    '''                    mtime_from,
                    mtime_to,
                )?;
                if output.is_json() {''',
    '''                    mtime_from,
                    mtime_to,
                )?)
                };
                if output.is_json() {''',
)
replace_once(
    "rust_indexer/src/main.rs",
    '''            FileCommands::Show { id } => {
                let file = read_api::show_file(&repo, id)?;''',
    '''            FileCommands::Show { id, snapshot_id } => {
                let file = if let Some(snapshot_id) = snapshot_id {
                    snapshot_api::show_snapshot_file(&repo, snapshot_id, id)?
                } else {
                    snapshot_api::SnapshotFileShowOutput::from_live(read_api::show_file(&repo, id)?)
                };''',
)

# Append immutable snapshot-file query implementation.
snapshot_file_code = r'''
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
            text.push_str(&format!("\n- file_id={} source={} kind={} path={} size={} scan_status={} hash_status={}",
                item.file_id,
                item.source_name,
                item.source_kind,
                item.path,
                item.size,
                item.scan_status,
                item.hash_status.as_deref().unwrap_or("none"),
            ));
        }
        text.push_str(&format!("\nnext_cursor: {}", self.next_cursor.map(|v| v.to_string()).unwrap_or_else(|| "none".to_string())));
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
        snapshot_file_select(), snapshot_id, file_id
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
    let fetch_limit = limit.checked_add(1).ok_or_else(|| "file catalogue limit is too large".to_string())?;
    let rows = repo.query_rows_text(&format!(
        "{} {} ORDER BY f.id_file ASC LIMIT {}",
        snapshot_file_select(), where_clause, fetch_limit
    ))?;

    let mut total_conditions = snapshot_filter_conditions(&filters);
    total_conditions.insert(0, format!("f.id_catalog_snapshot = {snapshot_id}"));
    let total_rows = repo.query_rows_text(&format!(
        "SELECT COUNT(*) FROM index_catalog_snapshot_files f WHERE {}",
        total_conditions.join(" AND ")
    ))?;
    let total = total_rows.first().and_then(|r| r.first())
        .ok_or_else(|| "snapshot file total row is missing".to_string())
        .and_then(|v| parse_u64(v, "snapshot file total"))?;

    let mut items = rows.iter().map(|row| snapshot_file_item_from_row(row)).collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > limit;
    if has_more { items.truncate(limit); }
    let next_cursor = if has_more { items.last().map(|item| item.file_id) } else { None };
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
        conditions.push(format!("POSITION(lower({}) IN lower(f.path)) > 0", sql_quote_literal(path)));
    }
    if let Some(name) = filters.name.as_deref() {
        conditions.push(format!("POSITION(lower({}) IN lower(substring(f.path from '[^/]+$'))) > 0", sql_quote_literal(name)));
    }
    if let Some(source) = filters.source.as_deref() {
        conditions.push(format!("f.source_name = {}", sql_quote_literal(source)));
    }
    if let Some(extension) = filters.extension.as_deref() {
        conditions.push(format!("lower(COALESCE(substring(f.path from '\\.([^./]+)$'), '')) = lower({})", sql_quote_literal(extension)));
    }
    if let Some(value) = filters.file_kind.as_deref() { conditions.push(format!("f.file_kind = {}", sql_quote_literal(value))); }
    if let Some(value) = filters.scan_status.as_deref() { conditions.push(format!("f.scan_status = {}", sql_quote_literal(value))); }
    if let Some(value) = filters.hash_status.as_deref() { conditions.push(format!("f.hash_status = {}", sql_quote_literal(value))); }
    if let Some(value) = filters.min_size { conditions.push(format!("f.size >= {value}")); }
    if let Some(value) = filters.max_size { conditions.push(format!("f.size <= {value}")); }
    if let Some(value) = filters.mtime_from { conditions.push(format!("f.mtime_ns >= {value}")); }
    if let Some(value) = filters.mtime_to { conditions.push(format!("f.mtime_ns <= {value}")); }
    conditions
}

fn normalize_snapshot_filters(mut filters: crate::read_api::FileCatalogFilters) -> Result<crate::read_api::FileCatalogFilters, String> {
    for (label, value) in [
        ("query", &mut filters.query), ("path", &mut filters.path), ("name", &mut filters.name),
        ("source", &mut filters.source), ("file-kind", &mut filters.file_kind),
        ("scan-status", &mut filters.scan_status), ("hash-status", &mut filters.hash_status),
    ] {
        if let Some(text) = value.as_mut() {
            *text = text.trim().to_string();
            if text.is_empty() { return Err(format!("file filter --{label} must not be empty")); }
        }
    }
    if let Some(extension) = filters.extension.as_mut() {
        *extension = extension.trim().trim_start_matches('.').to_string();
        if extension.is_empty() { return Err("file filter --extension must not be empty".to_string()); }
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
        return Err(format!("file catalogue --limit must be between 1 and {INDEXER_MAX_PAGE_LIMIT}, got {limit}"));
    }
    if matches!(cursor, Some(0)) { return Err("file catalogue --cursor must be a positive file id".to_string()); }
    if let (Some(min), Some(max)) = (filters.min_size, filters.max_size) {
        if min > max { return Err("file search --min-size must not exceed --max-size".to_string()); }
    }
    if let (Some(from), Some(to)) = (filters.mtime_from, filters.mtime_to) {
        if from > to { return Err("file search --mtime-from must not exceed --mtime-to".to_string()); }
    }
    if search_mode && snapshot_filters_empty(filters) { return Err("file search requires at least one search filter".to_string()); }
    Ok(())
}

fn snapshot_filters_empty(filters: &crate::read_api::FileCatalogFilters) -> bool {
    filters.query.is_none() && filters.path.is_none() && filters.name.is_none() && filters.source.is_none()
        && filters.extension.is_none() && filters.file_kind.is_none() && filters.scan_status.is_none()
        && filters.hash_status.is_none() && filters.min_size.is_none() && filters.max_size.is_none()
        && filters.mtime_from.is_none() && filters.mtime_to.is_none()
}

fn snapshot_file_item_from_row(row: &[String]) -> Result<crate::read_api::FileCatalogItem, String> {
    if row.len() < 19 { return Err("snapshot indexed file row is too short".to_string()); }
    let source_root = row[4].clone();
    let path = row[5].clone();
    let path_view = std::path::Path::new(&path);
    let name = path_view.file_name().and_then(|v| v.to_str()).unwrap_or(path.as_str()).to_string();
    let extension = path_view.extension().and_then(|v| v.to_str()).filter(|v| !v.is_empty()).map(str::to_string);
    let source_path = std::path::Path::new(&source_root).join(&path).display().to_string();
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
    if value.trim().is_empty() { Ok(None) } else {
        value.trim().parse::<i64>().map(Some).map_err(|e| format!("invalid {label}: {e}"))
    }
}

fn owned_filter(value: Option<&str>) -> Option<String> { value.map(str::to_string) }
'''
insert_before("rust_indexer/src/snapshot_api.rs", "#[cfg(test)]\nmod tests", snapshot_file_code + "\n")

# UML and documentation.
Path("uml/fod-catalog-snapshot-flow.puml").write_text(r'''@startuml
title FOD 3.2.21 — immutable catalogue snapshot flow
skinparam shadowing false
actor User
participant "fod-indexer" as CLI
database PostgreSQL as DB
User -> CLI: snapshot create [--source NAME]
CLI -> DB: one atomic WITH statement
DB -> DB: materialize live catalogue rows
DB -> DB: INSERT index_catalog_snapshots
DB -> DB: INSERT index_catalog_snapshot_files
DB --> CLI: snapshot_id + counts
CLI --> User: stored-snapshot metadata
User -> CLI: file list/search/show --snapshot-id ID
CLI -> DB: validate complete snapshot
CLI -> DB: SELECT immutable copied rows
DB --> CLI: deterministic file_id page
CLI --> User: consistency=stored-snapshot\nsnapshot_id=ID
User -> CLI: snapshot delete --id ID
CLI -> DB: DELETE header\nON DELETE CASCADE copied rows
DB --> CLI: deleted file count
note over CLI,DB
Snapshot creation changes only snapshot tables.
Live index rows and source files remain untouched.
end note
@enduml
''')

# Add a concise docs section; detailed API contract remains in the read API doc.
for path in ["docs/fod-indexer.md", "docs/fod-indexer-read-api.md"]:
    p = Path(path)
    text = p.read_text()
    addition = '''

## Immutable catalogue snapshots (FOD 3.2.21)

```bash
fod-indexer snapshot create [--source NAME]
fod-indexer snapshot list [--limit N] [--cursor ID]
fod-indexer snapshot show --id ID
fod-indexer snapshot delete --id ID
fod-indexer file list --snapshot-id ID
fod-indexer file search QUERY --snapshot-id ID
fod-indexer file show --id FILE_ID --snapshot-id ID
```

`snapshot create` atomically copies the current catalogue metadata into `index_catalog_snapshots` and `index_catalog_snapshot_files`. Snapshot-backed file queries return `consistency: stored-snapshot` and the selected `snapshot_id`; later scans, hash updates, source removal, and live catalogue cleanup do not alter copied rows. Snapshot creation and deletion write only snapshot tables. They do not scan, hash, materialize, read source bytes, or modify live index rows.
'''
    if "## Immutable catalogue snapshots (FOD 3.2.21)" not in text:
        p.write_text(text.rstrip() + addition + "\n")

# Update UML index.
uml = Path("uml/README.md")
uml.write_text(uml.read_text().rstrip() + "\n\n## FOD 3.2.21 — catalogue snapshots\n\n- `fod-catalog-snapshot-flow.puml` — creation, immutable reads, and deletion of catalogue snapshots.\n")
