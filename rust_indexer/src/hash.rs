use crate::db::{hex_encode, sql_bytea_hex, sql_nullable_i64, sql_nullable_u64, sql_quote_literal};
use crate::model::{HashSummary, IndexedFile};
use crate::progress::ThrottledProgress;
use crate::scan;
use crate::source;
use crate::source_registry;
use fod_rust_hotpath::pg::DbRepo;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;

const HASH_ALGORITHM: &str = "sha256";
const PARTIAL_SAMPLE_BYTES: usize = 64 * 1024;
const FULL_READ_BUFFER_BYTES: usize = 128 * 1024;
const HASH_PROGRESS_FILE_STEP: u64 = 50;
const HASH_PROGRESS_TIME_STEP: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileSnapshot {
    size: u64,
    mtime_ns: Option<i64>,
    inode: Option<u64>,
    device: Option<u64>,
}

impl FileSnapshot {
    fn from_path(path: &Path) -> Result<Self, String> {
        let metadata = fs::metadata(path)
            .map_err(|err| format!("unable to read file metadata for {}: {err}", path.display()))?;
        let mtime_ns = Some(
            metadata
                .mtime()
                .saturating_mul(1_000_000_000)
                .saturating_add(metadata.mtime_nsec()),
        );
        Ok(Self {
            size: metadata.len(),
            mtime_ns,
            inode: Some(metadata.ino()),
            device: Some(metadata.dev()),
        })
    }
}

#[derive(Debug, Clone)]
struct PartialHashResult {
    file: IndexedFile,
    snapshot: FileSnapshot,
    partial_hash: Vec<u8>,
}

struct HashProgressReporter {
    source_name: String,
    source_path: String,
    mode: &'static str,
    progress: ThrottledProgress,
}

impl HashProgressReporter {
    fn new(
        source_name: &str,
        source_path: &Path,
        candidates_only: bool,
        summary: &HashSummary,
    ) -> Self {
        let reporter = Self {
            source_name: source_name.to_string(),
            source_path: source_path.display().to_string(),
            mode: if candidates_only {
                "candidates-only"
            } else {
                "all"
            },
            progress: ThrottledProgress::new(),
        };
        reporter.emit("started", summary, None);
        reporter
    }

    fn maybe_report(
        &mut self,
        summary: &HashSummary,
        processed_files: u64,
        phase: &str,
        current: Option<(&Path, &str)>,
    ) {
        if self.progress.should_report(
            processed_files,
            HASH_PROGRESS_FILE_STEP,
            HASH_PROGRESS_TIME_STEP,
        ) {
            self.emit(phase, summary, current);
            self.progress.mark_reported();
        }
    }

    fn emit(&self, phase: &str, summary: &HashSummary, current: Option<(&Path, &str)>) {
        self.emit_with_error(phase, summary, "", current);
    }

    fn finish(&self, summary: &HashSummary) {
        self.emit("done", summary, None);
    }

    fn fail(&self, summary: &HashSummary, error: &str) {
        self.emit_with_error("failed", summary, error, None);
    }

    fn emit_with_error(
        &self,
        phase: &str,
        summary: &HashSummary,
        error: &str,
        current: Option<(&Path, &str)>,
    ) {
        let elapsed = self.progress.elapsed_secs();
        let mut line = format!(
            "FOD indexer hash progress: phase={} mode={} source={} path={} scanned={} candidate={} partial={} full={} changed_retry={} duplicate_sets={} elapsed={:.1}s",
            phase,
            self.mode,
            self.source_name,
            self.source_path,
            summary.scanned_files,
            summary.candidate_files,
            summary.partial_hashed_files,
            summary.full_hashed_files,
            summary.changed_retry_files,
            summary.duplicate_sets,
            elapsed,
        );
        if let Some((path, status)) = current {
            line.push_str(&format!(" current={} status={}", path.display(), status));
        }
        if !error.is_empty() {
            line.push_str(&format!(" error={error}"));
        }

        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "{line}");
        let _ = stderr.flush();
    }
}

fn mark_file_changed(repo: &DbRepo, file_id: u64) {
    let sql = format!(
        "
        UPDATE index_files
        SET source_changed = TRUE,
            updated_at = NOW()
        WHERE id_file = {file_id}
        "
    );
    let _ = repo.exec(&sql);
}

fn upsert_file_hash(
    repo: &DbRepo,
    file: &IndexedFile,
    snapshot: &FileSnapshot,
    partial_hash: Option<&[u8]>,
    full_hash: Option<&[u8]>,
    hash_status: &str,
) -> Result<(), String> {
    let partial_sql = partial_hash
        .map(sql_bytea_hex)
        .unwrap_or_else(|| "NULL".to_string());
    let full_sql = full_hash
        .map(sql_bytea_hex)
        .unwrap_or_else(|| "NULL".to_string());
    let sql = format!(
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
        VALUES (
            {file_id},
            {hash_algorithm},
            {partial_hash},
            {full_hash},
            {hash_status},
            {observed_size},
            {observed_mtime_ns},
            {observed_inode},
            {observed_device},
            NOW(),
            NOW()
        )
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
        file_id = file.id_file,
        hash_algorithm = sql_quote_literal(HASH_ALGORITHM),
        partial_hash = partial_sql,
        full_hash = full_sql,
        hash_status = sql_quote_literal(hash_status),
        observed_size = snapshot.size,
        observed_mtime_ns = sql_nullable_i64(snapshot.mtime_ns),
        observed_inode = sql_nullable_u64(snapshot.inode),
        observed_device = sql_nullable_u64(snapshot.device),
    );
    repo.exec(&sql)
}

fn sample_ranges(file_size: u64) -> Vec<(u64, usize)> {
    if file_size == 0 {
        return Vec::new();
    }

    let chunk = PARTIAL_SAMPLE_BYTES.min(file_size as usize).max(1);
    if file_size as usize <= chunk {
        return vec![(0, file_size as usize)];
    }

    let file_size_usize = file_size as usize;
    let middle_start = file_size_usize.saturating_sub(chunk) / 2;
    let last_start = file_size_usize.saturating_sub(chunk);
    let mut ranges = vec![
        (0u64, chunk),
        (middle_start as u64, chunk),
        (last_start as u64, chunk),
    ];
    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

fn read_exact_range(file: &mut File, offset: u64, len: usize) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|err| format!("seek failed at {offset}: {err}"))?;
    let mut buffer = vec![0u8; len];
    let mut read_total = 0usize;
    while read_total < len {
        let read = file
            .read(&mut buffer[read_total..])
            .map_err(|err| format!("read failed at {offset}: {err}"))?;
        if read == 0 {
            break;
        }
        read_total = read_total.saturating_add(read);
    }
    buffer.truncate(read_total);
    Ok(buffer)
}

fn compute_partial_hash(path: &Path, snapshot: &FileSnapshot) -> Result<Vec<u8>, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("unable to open {} for hashing: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    if snapshot.size == 0 {
        return Ok(hasher.finalize().to_vec());
    }

    for (offset, len) in sample_ranges(snapshot.size) {
        let bytes = read_exact_range(&mut file, offset, len)?;
        hasher.update(offset.to_le_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hasher.finalize().to_vec())
}

fn compute_full_hash(path: &Path) -> Result<Vec<u8>, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("unable to open {} for hashing: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; FULL_READ_BUFFER_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("read failed while hashing {}: {err}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_vec())
}

pub fn rebuild_duplicate_sets(repo: &DbRepo) -> Result<u64, String> {
    repo.exec("DELETE FROM index_duplicate_sets")?;
    let rows = repo.query_rows_text(
        "
        SELECT
            h.id_file,
            s.root_path,
            f.path,
            h.hash_algorithm,
            COALESCE(encode(h.full_hash, 'hex'), ''),
            h.observed_size
        FROM index_file_hashes h
        JOIN index_files f ON f.id_file = h.id_file
        JOIN index_sources s ON s.id_index_source = f.id_index_source
        WHERE h.hash_status = 'full' AND h.full_hash IS NOT NULL
        ORDER BY h.hash_algorithm, h.observed_size, h.full_hash, h.id_file
        ",
    )?;

    let mut groups: BTreeMap<(String, String, u64), Vec<u64>> = BTreeMap::new();
    for row in rows {
        if row.len() < 6 {
            continue;
        }
        let file_id = row[0]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid file id in hash rows: {err}"))?;
        if source::is_ignored_index_path(Path::new(&row[1]), &row[2]) {
            continue;
        }
        let algorithm = row[3].clone();
        let full_hash_hex = row[4].clone();
        let observed_size = row[5]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid file size in hash rows: {err}"))?;
        if source::is_zero_length_file(observed_size) {
            continue;
        }
        groups
            .entry((algorithm, full_hash_hex, observed_size))
            .or_default()
            .push(file_id);
    }

    let mut duplicate_sets = 0u64;
    for ((algorithm, full_hash_hex, file_size), file_ids) in groups {
        if file_ids.len() <= 1 {
            continue;
        }
        let full_hash_bytes = decode_hex(&full_hash_hex)?;
        let file_count = file_ids.len() as u64;
        let total_bytes = file_size.saturating_mul(file_count);
        let sql = format!(
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
            VALUES (
                {hash_algorithm},
                {full_hash},
                {file_size},
                {file_count},
                {total_bytes},
                NOW(),
                NOW()
            )
            ON CONFLICT (hash_algorithm, full_hash, file_size) DO UPDATE SET
                file_count = EXCLUDED.file_count,
                total_bytes = EXCLUDED.total_bytes,
                updated_at = NOW()
            ",
            hash_algorithm = sql_quote_literal(&algorithm),
            full_hash = sql_bytea_hex(&full_hash_bytes),
            file_size = file_size,
            file_count = file_count,
            total_bytes = total_bytes,
        );
        repo.exec(&sql)?;
        duplicate_sets = duplicate_sets.saturating_add(1);
    }

    Ok(duplicate_sets)
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    if value.len() % 2 != 0 {
        return Err(format!("invalid hex value: {value}"));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let chars: Vec<char> = value.chars().collect();
    for idx in (0..chars.len()).step_by(2) {
        let hi = chars[idx]
            .to_digit(16)
            .ok_or_else(|| format!("invalid hex value: {value}"))?;
        let lo = chars[idx + 1]
            .to_digit(16)
            .ok_or_else(|| format!("invalid hex value: {value}"))?;
        bytes.push(((hi << 4) | lo) as u8);
    }
    Ok(bytes)
}

fn partial_results_for_group(
    repo: &DbRepo,
    group: &[IndexedFile],
    summary: &mut HashSummary,
    progress: &mut HashProgressReporter,
    processed_files: &mut u64,
) -> Result<Vec<PartialHashResult>, String> {
    let mut results = Vec::with_capacity(group.len());
    for file in group {
        *processed_files = processed_files.saturating_add(1);
        let path = file.root_path.join(&file.path);
        let snapshot = FileSnapshot::from_path(&path)?;
        let expected_snapshot = FileSnapshot {
            size: file.size,
            mtime_ns: file.mtime_ns,
            inode: file.inode,
            device: file.device,
        };
        if snapshot != expected_snapshot {
            summary.changed_retry_files = summary.changed_retry_files.saturating_add(1);
            mark_file_changed(repo, file.id_file);
            upsert_file_hash(repo, file, &snapshot, None, None, "changed_retry_needed")?;
            progress.maybe_report(
                summary,
                *processed_files,
                "partial",
                Some((path.as_path(), "changed_retry_needed")),
            );
            continue;
        }
        let partial_hash = compute_partial_hash(&path, &snapshot)?;
        let after = FileSnapshot::from_path(&path)?;
        if after != snapshot {
            summary.changed_retry_files = summary.changed_retry_files.saturating_add(1);
            mark_file_changed(repo, file.id_file);
            upsert_file_hash(repo, file, &snapshot, None, None, "changed_retry_needed")?;
            progress.maybe_report(
                summary,
                *processed_files,
                "partial",
                Some((path.as_path(), "changed_retry_needed")),
            );
            continue;
        }
        upsert_file_hash(repo, file, &snapshot, Some(&partial_hash), None, "partial")?;
        summary.partial_hashed_files = summary.partial_hashed_files.saturating_add(1);
        progress.maybe_report(
            summary,
            *processed_files,
            "partial",
            Some((path.as_path(), "partial")),
        );
        results.push(PartialHashResult {
            file: file.clone(),
            snapshot,
            partial_hash,
        });
    }
    Ok(results)
}

fn full_hash_group(
    repo: &DbRepo,
    group: &[PartialHashResult],
    summary: &mut HashSummary,
    progress: &mut HashProgressReporter,
    processed_files: &mut u64,
) -> Result<Vec<(IndexedFile, FileSnapshot, Vec<u8>)>, String> {
    let mut results = Vec::with_capacity(group.len());
    for item in group {
        *processed_files = processed_files.saturating_add(1);
        let path = item.file.root_path.join(&item.file.path);
        let before = FileSnapshot::from_path(&path)?;
        if before != item.snapshot {
            summary.changed_retry_files = summary.changed_retry_files.saturating_add(1);
            mark_file_changed(repo, item.file.id_file);
            upsert_file_hash(
                repo,
                &item.file,
                &before,
                None,
                None,
                "changed_retry_needed",
            )?;
            progress.maybe_report(
                summary,
                *processed_files,
                "full",
                Some((path.as_path(), "changed_retry_needed")),
            );
            continue;
        }
        let full_hash = compute_full_hash(&path)?;
        let after = FileSnapshot::from_path(&path)?;
        if after != before {
            summary.changed_retry_files = summary.changed_retry_files.saturating_add(1);
            mark_file_changed(repo, item.file.id_file);
            upsert_file_hash(
                repo,
                &item.file,
                &before,
                None,
                None,
                "changed_retry_needed",
            )?;
            progress.maybe_report(
                summary,
                *processed_files,
                "full",
                Some((path.as_path(), "changed_retry_needed")),
            );
            continue;
        }
        upsert_file_hash(
            repo,
            &item.file,
            &before,
            Some(&item.partial_hash),
            Some(&full_hash),
            "full",
        )?;
        summary.full_hashed_files = summary.full_hashed_files.saturating_add(1);
        progress.maybe_report(
            summary,
            *processed_files,
            "full",
            Some((path.as_path(), "full")),
        );
        results.push((item.file.clone(), before, full_hash));
    }
    Ok(results)
}

pub fn hash_source(
    repo: &DbRepo,
    source_name: &str,
    candidates_only: bool,
) -> Result<HashSummary, String> {
    let source = source_registry::load_source(repo, source_name)?;
    let files = scan::load_indexed_files(repo, Some(source_name))?;
    let files = files
        .into_iter()
        .filter(|file| file.scan_status == "ok" && file.file_kind == "regular")
        .filter(|file| !source::is_ignored_indexed_file(file))
        .collect::<Vec<_>>();
    let mut summary = HashSummary {
        source_name: source.name.clone(),
        scanned_files: files.len() as u64,
        ..HashSummary::default()
    };
    let mut progress =
        HashProgressReporter::new(&source.name, &source.root_path, candidates_only, &summary);
    let mut processed_files = 0u64;

    let hash_result = (|| -> Result<(), String> {
        let mut groups: BTreeMap<u64, Vec<IndexedFile>> = BTreeMap::new();
        for file in files {
            groups.entry(file.size).or_default().push(file);
        }

        let mut partial_groups: HashMap<(u64, String), Vec<PartialHashResult>> = HashMap::new();
        for (size, group) in groups {
            if candidates_only && group.len() <= 1 {
                continue;
            }
            summary.candidate_files = summary.candidate_files.saturating_add(group.len() as u64);
            let partials = partial_results_for_group(
                repo,
                &group,
                &mut summary,
                &mut progress,
                &mut processed_files,
            )?;
            for item in partials {
                partial_groups
                    .entry((size, hex_encode(&item.partial_hash)))
                    .or_default()
                    .push(item);
            }
        }

        progress.emit("rebuilding", &summary, None);
        for group in partial_groups.into_values() {
            if group.len() <= 1 {
                continue;
            }
            let _full_results = full_hash_group(
                repo,
                &group,
                &mut summary,
                &mut progress,
                &mut processed_files,
            )?;
        }

        let duplicate_sets = rebuild_duplicate_sets(repo)?;
        summary.duplicate_sets = duplicate_sets;
        Ok(())
    })();

    match hash_result {
        Ok(()) => {
            progress.finish(&summary);
            Ok(summary)
        }
        Err(err) => {
            progress.fail(&summary, &err);
            Err(err)
        }
    }
}
