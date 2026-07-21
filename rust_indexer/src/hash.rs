use crate::db::hex_encode;
use crate::model::{HashSummary, IndexedFile};
use crate::progress::ThrottledProgress;
use crate::scan;
use crate::source;
use crate::source_registry;
use fod_rust_hotpath::pg::{DbRepo, DuplicateSetStageRow, IndexFileHashStageRow};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;

pub(crate) const HASH_ALGORITHM: &str = "sha256";
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

pub(crate) fn compute_partial_hash_from_file(
    file: &mut File,
    file_size: u64,
) -> Result<Vec<u8>, String> {
    let mut hasher = Sha256::new();
    if file_size == 0 {
        return Ok(hasher.finalize().to_vec());
    }
    for (offset, len) in sample_ranges(file_size) {
        let bytes = read_exact_range(file, offset, len)?;
        hasher.update(offset.to_le_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hasher.finalize().to_vec())
}

fn compute_partial_hash(path: &Path, snapshot: &FileSnapshot) -> Result<Vec<u8>, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("unable to open {} for hashing: {err}", path.display()))?;
    compute_partial_hash_from_file(&mut file, snapshot.size)
}

pub(crate) fn compute_full_hash_from_file(file: &mut File) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|err| format!("seek failed before full hashing: {err}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; FULL_READ_BUFFER_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("read failed while hashing: {err}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_vec())
}

fn compute_full_hash(path: &Path) -> Result<Vec<u8>, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("unable to open {} for hashing: {err}", path.display()))?;
    compute_full_hash_from_file(&mut file).map_err(|err| format!("{err} for {}", path.display()))
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

    let mut duplicate_rows = Vec::new();
    for ((algorithm, full_hash_hex, file_size), file_ids) in groups {
        if file_ids.len() <= 1 {
            continue;
        }
        let full_hash_bytes = decode_hex(&full_hash_hex)?;
        let file_count = file_ids.len() as u64;
        let total_bytes = file_size.saturating_mul(file_count);
        duplicate_rows.push(DuplicateSetStageRow {
            hash_algorithm: algorithm,
            full_hash: full_hash_bytes,
            file_size,
            file_count,
            total_bytes,
        });
    }

    repo.upsert_index_duplicate_sets_staged(&duplicate_rows)
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
    staged_rows: &mut HashMap<u64, IndexFileHashStageRow>,
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
            staged_rows.insert(
                file.id_file,
                IndexFileHashStageRow {
                    id_file: file.id_file,
                    hash_algorithm: HASH_ALGORITHM.to_string(),
                    partial_hash: None,
                    full_hash: None,
                    hash_status: "changed_retry_needed".to_string(),
                    observed_size: snapshot.size,
                    observed_mtime_ns: snapshot.mtime_ns,
                    observed_inode: snapshot.inode,
                    observed_device: snapshot.device,
                },
            );
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
            staged_rows.insert(
                file.id_file,
                IndexFileHashStageRow {
                    id_file: file.id_file,
                    hash_algorithm: HASH_ALGORITHM.to_string(),
                    partial_hash: None,
                    full_hash: None,
                    hash_status: "changed_retry_needed".to_string(),
                    observed_size: snapshot.size,
                    observed_mtime_ns: snapshot.mtime_ns,
                    observed_inode: snapshot.inode,
                    observed_device: snapshot.device,
                },
            );
            progress.maybe_report(
                summary,
                *processed_files,
                "partial",
                Some((path.as_path(), "changed_retry_needed")),
            );
            continue;
        }
        staged_rows.insert(
            file.id_file,
            IndexFileHashStageRow {
                id_file: file.id_file,
                hash_algorithm: HASH_ALGORITHM.to_string(),
                partial_hash: Some(partial_hash.clone()),
                full_hash: None,
                hash_status: "partial".to_string(),
                observed_size: snapshot.size,
                observed_mtime_ns: snapshot.mtime_ns,
                observed_inode: snapshot.inode,
                observed_device: snapshot.device,
            },
        );
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
    staged_rows: &mut HashMap<u64, IndexFileHashStageRow>,
) -> Result<Vec<(IndexedFile, FileSnapshot, Vec<u8>)>, String> {
    let mut results = Vec::with_capacity(group.len());
    for item in group {
        *processed_files = processed_files.saturating_add(1);
        let path = item.file.root_path.join(&item.file.path);
        let before = FileSnapshot::from_path(&path)?;
        if before != item.snapshot {
            summary.changed_retry_files = summary.changed_retry_files.saturating_add(1);
            mark_file_changed(repo, item.file.id_file);
            staged_rows.insert(
                item.file.id_file,
                IndexFileHashStageRow {
                    id_file: item.file.id_file,
                    hash_algorithm: HASH_ALGORITHM.to_string(),
                    partial_hash: None,
                    full_hash: None,
                    hash_status: "changed_retry_needed".to_string(),
                    observed_size: before.size,
                    observed_mtime_ns: before.mtime_ns,
                    observed_inode: before.inode,
                    observed_device: before.device,
                },
            );
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
            staged_rows.insert(
                item.file.id_file,
                IndexFileHashStageRow {
                    id_file: item.file.id_file,
                    hash_algorithm: HASH_ALGORITHM.to_string(),
                    partial_hash: None,
                    full_hash: None,
                    hash_status: "changed_retry_needed".to_string(),
                    observed_size: before.size,
                    observed_mtime_ns: before.mtime_ns,
                    observed_inode: before.inode,
                    observed_device: before.device,
                },
            );
            progress.maybe_report(
                summary,
                *processed_files,
                "full",
                Some((path.as_path(), "changed_retry_needed")),
            );
            continue;
        }
        staged_rows.insert(
            item.file.id_file,
            IndexFileHashStageRow {
                id_file: item.file.id_file,
                hash_algorithm: HASH_ALGORITHM.to_string(),
                partial_hash: Some(item.partial_hash.clone()),
                full_hash: Some(full_hash.clone()),
                hash_status: "full".to_string(),
                observed_size: before.size,
                observed_mtime_ns: before.mtime_ns,
                observed_inode: before.inode,
                observed_device: before.device,
            },
        );
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

fn eligible_hash_files(files: Vec<IndexedFile>) -> Vec<IndexedFile> {
    files
        .into_iter()
        .filter(|file| file.scan_status == "ok" && file.file_kind == "regular")
        .filter(|file| !source::is_ignored_indexed_file(file))
        .collect()
}

fn load_global_candidate_sizes(repo: &DbRepo) -> Result<HashSet<u64>, String> {
    let rows = repo.query_rows_text(
        "
        SELECT f.size::text
        FROM index_files f
        WHERE f.scan_status = 'ok'
          AND f.file_kind = 'regular'
          AND f.size > 0
        GROUP BY f.size
        HAVING COUNT(*) > 1
        ORDER BY f.size
        ",
    )?;

    rows.iter()
        .map(|row| {
            row.first()
                .ok_or_else(|| "global candidate-size row is empty".to_string())?
                .trim()
                .parse::<u64>()
                .map_err(|err| format!("invalid global candidate size: {err}"))
        })
        .collect()
}

fn select_hash_scope(
    source_files: &[IndexedFile],
    global_candidate_sizes: Option<&HashSet<u64>>,
    candidates_only: bool,
) -> Vec<IndexedFile> {
    if !candidates_only {
        return source_files.to_vec();
    }

    let Some(global_candidate_sizes) = global_candidate_sizes else {
        return Vec::new();
    };

    source_files
        .iter()
        .filter(|file| global_candidate_sizes.contains(&file.size))
        .cloned()
        .collect()
}

pub fn hash_source(
    repo: &DbRepo,
    source_name: &str,
    candidates_only: bool,
) -> Result<HashSummary, String> {
    let source = source_registry::load_source(repo, source_name)?;
    let source_files = eligible_hash_files(scan::load_indexed_files(repo, Some(source_name))?);
    let global_candidate_sizes = if candidates_only {
        Some(load_global_candidate_sizes(repo)?)
    } else {
        None
    };
    let files = select_hash_scope(
        &source_files,
        global_candidate_sizes.as_ref(),
        candidates_only,
    );
    let mut summary = HashSummary {
        source_name: source.name.clone(),
        source_path: source.root_path.display().to_string(),
        scanned_files: source_files.len() as u64,
        ..HashSummary::default()
    };
    let mut progress =
        HashProgressReporter::new(&source.name, &source.root_path, candidates_only, &summary);
    let mut processed_files = 0u64;
    let mut staged_rows: HashMap<u64, IndexFileHashStageRow> = HashMap::new();

    let hash_result = (|| -> Result<(), String> {
        let mut groups: BTreeMap<u64, Vec<IndexedFile>> = BTreeMap::new();
        for file in files {
            groups.entry(file.size).or_default().push(file);
        }

        let mut partial_groups: HashMap<(u64, String), Vec<PartialHashResult>> = HashMap::new();
        for (size, group) in groups {
            summary.candidate_files = summary.candidate_files.saturating_add(group.len() as u64);
            let partials = partial_results_for_group(
                repo,
                &group,
                &mut summary,
                &mut progress,
                &mut processed_files,
                &mut staged_rows,
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
            if !candidates_only && group.len() <= 1 {
                continue;
            }
            let _full_results = full_hash_group(
                repo,
                &group,
                &mut summary,
                &mut progress,
                &mut processed_files,
                &mut staged_rows,
            )?;
        }

        let staged_rows = staged_rows.into_values().collect::<Vec<_>>();
        repo.upsert_index_file_hashes_staged(&staged_rows)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn indexed_file(
        id_file: u64,
        source_id: u64,
        source_name: &str,
        path: &str,
        size: u64,
    ) -> IndexedFile {
        IndexedFile {
            id_file,
            source_id,
            source_name: source_name.to_string(),
            root_path: PathBuf::from(format!("/tmp/{source_name}")),
            path: path.to_string(),
            size,
            mtime_ns: None,
            inode: None,
            device: None,
            file_kind: "regular".to_string(),
            scan_status: "ok".to_string(),
            source_changed: false,
        }
    }

    #[test]
    fn candidates_only_uses_global_sizes_but_keeps_reads_source_scoped() {
        let source_files = vec![
            indexed_file(1, 10, "source-a", "a.bin", 100),
            indexed_file(2, 10, "source-a", "unique.bin", 200),
        ];
        let global_candidate_sizes = HashSet::from([100]);

        let selected = select_hash_scope(&source_files, Some(&global_candidate_sizes), true);
        let ids = selected.iter().map(|file| file.id_file).collect::<Vec<_>>();

        assert_eq!(ids, vec![1]);
    }

    #[test]
    fn full_hash_mode_remains_source_scoped() {
        let source_files = vec![
            indexed_file(1, 10, "source-a", "a.bin", 100),
            indexed_file(2, 10, "source-a", "b.bin", 200),
        ];

        let selected = select_hash_scope(&source_files, None, false);
        let ids = selected.iter().map(|file| file.id_file).collect::<Vec<_>>();

        assert_eq!(ids, vec![1, 2]);
    }
}
