use crate::db::{
    ensure_indexer_request_token_schema, sql_nullable_i64, sql_nullable_string, sql_nullable_u64,
    sql_quote_literal,
};
use crate::model::{IndexedFile, ScanSummary};
use crate::progress::ThrottledProgress;
use crate::replay;
use crate::source;
use crate::source_registry;
use fod_rust_hotpath::pg::DbRepo;
use std::fs;
use std::fs::{File, FileType};
use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;
use walkdir::WalkDir;

const SCAN_PROGRESS_FILE_STEP: u64 = 50;
const SCAN_PROGRESS_TIME_STEP: Duration = Duration::from_secs(1);

fn create_scan_run(repo: &DbRepo, source_id: u64) -> Result<u64, String> {
    let request_token = replay::request_token("scan");
    let sql = format!(
        "
        INSERT INTO index_scan_runs (
            id_index_source,
            started_at,
            status,
            request_token
        )
        VALUES (
            {source_id},
            NOW(),
            'running',
            {request_token}
        )
        ON CONFLICT (request_token) DO UPDATE SET
            id_index_source = EXCLUDED.id_index_source,
            status = EXCLUDED.status,
            updated_at = NOW()
        RETURNING id_scan_run
        ",
        request_token = sql_quote_literal(&request_token),
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

pub(crate) fn relative_source_path(root_path: &Path, entry_path: &Path) -> String {
    entry_path
        .strip_prefix(root_path)
        .unwrap_or(entry_path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn scan_source(repo: &DbRepo, name: &str) -> Result<ScanSummary, String> {
    ensure_indexer_request_token_schema(repo, "fod-indexer scan")?;
    let source = source_registry::load_source(repo, name)?;

    let scan_run_id = create_scan_run(repo, source.id_source)?;
    let mut summary = ScanSummary {
        source_name: source.name.clone(),
        source_path: source.root_path.display().to_string(),
        ..ScanSummary::default()
    };
    let mut progress = ScanProgressReporter::new(&source.name, &source.root_path);

    let scan_result = (|| -> Result<(), String> {
        let walker = WalkDir::new(&source.root_path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !source::is_ignored_source_path(&source.root_path, entry.path()));
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
                progress.maybe_report(&summary, &entry_path, "unsupported_type");
                continue;
            }

            if source::is_ignored_index_path(&source.root_path, &relative_path) {
                summary.filtered_files = summary.filtered_files.saturating_add(1);
                progress.maybe_report(&summary, &entry_path, "filtered");
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
                    progress.maybe_report(&summary, &entry_path, "stat_failed");
                    continue;
                }
            };

            let size = metadata.len();
            if source::is_zero_length_file(size) {
                summary.filtered_files = summary.filtered_files.saturating_add(1);
                progress.maybe_report(&summary, &entry_path, "filtered");
                continue;
            }
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
            progress.maybe_report(&summary, &entry_path, scan_status);
        }
        Ok(())
    })();

    match scan_result {
        Ok(()) => {
            progress.finish(&summary);
            finish_scan_run(repo, scan_run_id, "completed", None);
            Ok(summary)
        }
        Err(err) => {
            progress.fail(&summary, &err);
            finish_scan_run(repo, scan_run_id, "failed", Some(&err));
            Err(err)
        }
    }
}

struct ScanProgressReporter {
    source_name: String,
    source_path: String,
    progress: ThrottledProgress,
}

impl ScanProgressReporter {
    fn new(source_name: &str, source_path: &Path) -> Self {
        let reporter = Self {
            source_name: source_name.to_string(),
            source_path: source_path.display().to_string(),
            progress: ThrottledProgress::new(),
        };
        reporter.emit("started", &ScanSummary::default(), None);
        reporter
    }

    fn maybe_report(&mut self, summary: &ScanSummary, current_path: &Path, status: &str) {
        if self.progress.should_report(
            summary.scanned_files,
            SCAN_PROGRESS_FILE_STEP,
            SCAN_PROGRESS_TIME_STEP,
        ) {
            self.emit("running", summary, Some((current_path, status)));
            self.progress.mark_reported();
        }
    }

    fn finish(&mut self, summary: &ScanSummary) {
        self.emit("done", summary, None);
    }

    fn fail(&mut self, summary: &ScanSummary, error: &str) {
        self.emit_with_error("failed", summary, error, None);
    }

    fn emit(&self, phase: &str, summary: &ScanSummary, current: Option<(&Path, &str)>) {
        self.emit_with_error(phase, summary, "", current);
    }

    fn emit_with_error(
        &self,
        phase: &str,
        summary: &ScanSummary,
        error: &str,
        current: Option<(&Path, &str)>,
    ) {
        let elapsed = self.progress.elapsed_secs();
        let mut line = format!(
            "FOD indexer scan progress: phase={} source={} path={} scanned={} ok={} unreadable={} stat_failed={} unsupported={} filtered={} bytes={} elapsed={:.1}s",
            phase,
            self.source_name,
            self.source_path,
            summary.scanned_files,
            summary.ok_files,
            summary.unreadable_files,
            summary.stat_failed_files,
            summary.unsupported_files,
            summary.filtered_files,
            summary.total_bytes,
            elapsed,
        );
        if let Some((path, status)) = current {
            line.push_str(&format!(" current={} status={}", path.display(), status));
        }
        if !error.is_empty() {
            line.push_str(&format!(" error={error}"));
        }

        let mut stderr = io::stderr().lock();
        let _ = writeln!(stderr, "{line}");
        let _ = stderr.flush();
    }
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
