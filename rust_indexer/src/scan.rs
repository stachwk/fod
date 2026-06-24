use crate::db::{sql_nullable_i64, sql_nullable_string, sql_nullable_u64, sql_quote_literal};
use crate::model::{IndexSource, IndexedFile, ScanSummary};
use crate::replay;
use fod_rust_hotpath::pg::DbRepo;
use std::fs;
use std::fs::{File, FileType};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use walkdir::WalkDir;

pub fn register_source(
    repo: &DbRepo,
    name: &str,
    path: &str,
    kind: &str,
) -> Result<IndexSource, String> {
    if kind != "local" {
        return Err(format!("unsupported source kind: {kind}"));
    }

    let root_path = fs::canonicalize(path)
        .map_err(|err| format!("source path {path} is not accessible: {err}"))?;
    let metadata = fs::metadata(&root_path)
        .map_err(|err| format!("source path {} is not readable: {err}", root_path.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "source path {} is not a directory",
            root_path.display()
        ));
    }

    let sql = format!(
        "
        INSERT INTO index_sources (name, kind, root_path, created_at, updated_at)
        VALUES ({name}, {kind}, {root_path}, NOW(), NOW())
        ON CONFLICT (name) DO UPDATE SET
            kind = EXCLUDED.kind,
            root_path = EXCLUDED.root_path,
            updated_at = NOW()
        RETURNING id_index_source, name, kind, root_path
        ",
        name = sql_quote_literal(name),
        kind = sql_quote_literal(kind),
        root_path = sql_quote_literal(&root_path.to_string_lossy()),
    );
    let rows = repo.query_rows_text(&sql)?;
    let row = rows
        .first()
        .ok_or_else(|| "source registration did not return a row".to_string())?;
    IndexSource::from_row(row)
}

pub fn load_source(repo: &DbRepo, name: &str) -> Result<IndexSource, String> {
    let sql = format!(
        "
        SELECT id_index_source, name, kind, root_path
        FROM index_sources
        WHERE name = {}
        LIMIT 1
        ",
        sql_quote_literal(name)
    );
    let rows = repo.query_rows_text(&sql)?;
    let row = rows
        .first()
        .ok_or_else(|| format!("unknown source: {name}"))?;
    IndexSource::from_row(row)
}

fn create_scan_run(repo: &DbRepo, source_id: u64) -> Result<u64, String> {
    let running_status = format!("running:{}", replay::request_token("scan"));
    let sql = format!(
        "
        WITH existing AS (
            SELECT id_scan_run
            FROM index_scan_runs
            WHERE id_index_source = {source_id}
              AND status = {running_status}
            ORDER BY started_at DESC, id_scan_run DESC
            LIMIT 1
        ),
        inserted AS (
            INSERT INTO index_scan_runs (id_index_source, started_at, status)
            SELECT {source_id}, NOW(), {running_status}
            WHERE NOT EXISTS (SELECT 1 FROM existing)
            RETURNING id_scan_run
        )
        SELECT id_scan_run FROM inserted
        UNION ALL
        SELECT id_scan_run FROM existing
        LIMIT 1
        ",
        running_status = sql_quote_literal(&running_status),
    );
    let rows = repo.query_rows_text(&sql)?;
    let row = rows
        .first()
        .ok_or_else(|| "scan run creation did not return a row".to_string())?;
    row.first()
        .ok_or_else(|| "scan run creation returned no id".to_string())?
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("invalid scan run id: {err}"))
}

fn finish_scan_run(repo: &DbRepo, scan_run_id: u64, status: &str, error_message: Option<&str>) {
    let sql = format!(
        "
        UPDATE index_scan_runs
        SET finished_at = NOW(),
            status = {status},
            error_message = {error_message},
            updated_at = NOW()
        WHERE id_scan_run = {scan_run_id}
        ",
        status = sql_quote_literal(status),
        error_message = sql_nullable_string(error_message),
    );
    let _ = repo.exec(&sql);
}

fn file_kind_label(file_type: &FileType) -> &'static str {
    if file_type.is_file() {
        "regular"
    } else if file_type.is_symlink() {
        "symlink"
    } else if file_type.is_dir() {
        "directory"
    } else {
        "other"
    }
}

fn upsert_index_file(
    repo: &DbRepo,
    source_id: u64,
    scan_run_id: u64,
    path: &str,
    size: u64,
    mtime_ns: Option<i64>,
    inode: Option<u64>,
    device: Option<u64>,
    file_kind: &str,
    scan_status: &str,
    source_changed: bool,
) -> Result<(), String> {
    let sql = format!(
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
        VALUES (
            {source_id},
            {scan_run_id},
            {path},
            {size},
            {mtime_ns},
            {inode},
            {device},
            {file_kind},
            {scan_status},
            {source_changed},
            NOW(),
            NOW()
        )
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
        path = sql_quote_literal(path),
        file_kind = sql_quote_literal(file_kind),
        scan_status = sql_quote_literal(scan_status),
        source_changed = if source_changed { "TRUE" } else { "FALSE" },
        mtime_ns = sql_nullable_i64(mtime_ns),
        inode = sql_nullable_u64(inode),
        device = sql_nullable_u64(device),
    );
    repo.exec(&sql)
}

fn relative_source_path(root_path: &Path, entry_path: &Path) -> String {
    entry_path
        .strip_prefix(root_path)
        .unwrap_or(entry_path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn scan_source(repo: &DbRepo, name: &str) -> Result<ScanSummary, String> {
    let source = load_source(repo, name)?;
    if source.kind != "local" {
        return Err(format!(
            "source {name} is registered as kind {} and cannot be scanned by the local indexer",
            source.kind
        ));
    }

    let scan_run_id = create_scan_run(repo, source.id_source)?;
    let mut summary = ScanSummary {
        source_name: source.name.clone(),
        source_path: source.root_path.display().to_string(),
        ..ScanSummary::default()
    };

    let walker = WalkDir::new(&source.root_path).follow_links(false);
    for item in walker {
        let entry = match item {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("FOD indexer scan warning: {err}");
                continue;
            }
        };

        if entry.depth() == 0 || entry.file_type().is_dir() {
            continue;
        }

        summary.scanned_files = summary.scanned_files.saturating_add(1);
        let entry_path = entry.path().to_path_buf();
        let relative_path = relative_source_path(&source.root_path, &entry_path);
        let file_kind = file_kind_label(&entry.file_type());

        if file_kind != "regular" {
            summary.unsupported_files = summary.unsupported_files.saturating_add(1);
            upsert_index_file(
                repo,
                source.id_source,
                scan_run_id,
                &relative_path,
                0,
                None,
                None,
                None,
                file_kind,
                "unsupported_type",
                false,
            )?;
            continue;
        }

        let metadata = match fs::metadata(&entry_path) {
            Ok(metadata) => metadata,
            Err(err) => {
                summary.stat_failed_files = summary.stat_failed_files.saturating_add(1);
                upsert_index_file(
                    repo,
                    source.id_source,
                    scan_run_id,
                    &relative_path,
                    0,
                    None,
                    None,
                    None,
                    "regular",
                    "stat_failed",
                    false,
                )?;
                eprintln!(
                    "FOD indexer scan warning: file={} stat failed: {}",
                    entry_path.display(),
                    err
                );
                continue;
            }
        };

        let size = metadata.len();
        let mtime_ns = Some(
            metadata
                .mtime()
                .saturating_mul(1_000_000_000)
                .saturating_add(metadata.mtime_nsec()),
        );
        let inode = Some(metadata.ino());
        let device = Some(metadata.dev());
        let scan_status = match File::open(&entry_path) {
            Ok(_) => {
                summary.ok_files = summary.ok_files.saturating_add(1);
                summary.total_bytes = summary.total_bytes.saturating_add(size);
                "ok"
            }
            Err(err) => {
                summary.unreadable_files = summary.unreadable_files.saturating_add(1);
                eprintln!(
                    "FOD indexer scan warning: file={} open failed: {}",
                    entry_path.display(),
                    err
                );
                "unreadable"
            }
        };

        upsert_index_file(
            repo,
            source.id_source,
            scan_run_id,
            &relative_path,
            size,
            mtime_ns,
            inode,
            device,
            "regular",
            scan_status,
            false,
        )?;
    }

    finish_scan_run(repo, scan_run_id, "completed", None);
    Ok(summary)
}

pub fn load_indexed_files(
    repo: &DbRepo,
    source_name: Option<&str>,
) -> Result<Vec<IndexedFile>, String> {
    let filter = source_name
        .map(|name| format!("WHERE s.name = {}", sql_quote_literal(name)))
        .unwrap_or_default();
    let sql = format!(
        "
        SELECT
            f.id_file,
            f.id_index_source,
            s.name,
            s.root_path,
            f.path,
            f.size,
            COALESCE(f.mtime_ns::text, ''),
            COALESCE(f.inode::text, ''),
            COALESCE(f.device::text, ''),
            f.file_kind,
            f.scan_status,
            f.source_changed::text
        FROM index_files f
        JOIN index_sources s ON s.id_index_source = f.id_index_source
        {}
        ORDER BY f.id_index_source, f.path
        ",
        filter
    );
    let rows = repo.query_rows_text(&sql)?;
    rows.iter()
        .map(|row| IndexedFile::from_row(row))
        .collect::<Result<Vec<_>, _>>()
}
