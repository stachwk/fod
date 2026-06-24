use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct IndexSource {
    pub id_source: u64,
    pub name: String,
    pub kind: String,
    pub root_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SourceBrowseEntry {
    pub path: PathBuf,
    pub added_sources: Vec<IndexSource>,
}

#[derive(Debug, Clone)]
pub struct IndexedFile {
    pub id_file: u64,
    pub source_id: u64,
    pub source_name: String,
    pub root_path: PathBuf,
    pub path: String,
    pub size: u64,
    pub mtime_ns: Option<i64>,
    pub inode: Option<u64>,
    pub device: Option<u64>,
    pub file_kind: String,
    pub scan_status: String,
    pub source_changed: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FileHash {
    pub id_file: u64,
    pub hash_algorithm: String,
    pub partial_hash_hex: Option<String>,
    pub full_hash_hex: Option<String>,
    pub hash_status: String,
    pub observed_size: u64,
    pub observed_mtime_ns: Option<i64>,
    pub observed_inode: Option<u64>,
    pub observed_device: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct DuplicateSet {
    pub id_duplicate_set: u64,
    pub hash_algorithm: String,
    pub full_hash_hex: String,
    pub file_size: u64,
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ScanSummary {
    pub source_name: String,
    pub source_path: String,
    pub scanned_files: u64,
    pub ok_files: u64,
    pub unreadable_files: u64,
    pub stat_failed_files: u64,
    pub unsupported_files: u64,
    pub total_bytes: u64,
}

impl ScanSummary {
    pub fn human_readable(&self) -> String {
        format!(
            "FOD indexer scan\nsource: {}\npath: {}\nscanned files: {}\nok files: {}\nunreadable files: {}\nstat failed: {}\nunsupported files: {}\ntotal bytes: {}",
            self.source_name,
            self.source_path,
            self.scanned_files,
            self.ok_files,
            self.unreadable_files,
            self.stat_failed_files,
            self.unsupported_files,
            self.total_bytes
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct HashSummary {
    pub source_name: String,
    pub scanned_files: u64,
    pub candidate_files: u64,
    pub partial_hashed_files: u64,
    pub full_hashed_files: u64,
    pub changed_retry_files: u64,
    pub duplicate_sets: u64,
}

impl HashSummary {
    pub fn human_readable(&self) -> String {
        format!(
            "FOD indexer hash\nsource: {}\nscanned files: {}\ncandidate files: {}\npartial hashed: {}\nfull hashed: {}\nchanged/retry needed: {}\nduplicate sets: {}",
            self.source_name,
            self.scanned_files,
            self.candidate_files,
            self.partial_hashed_files,
            self.full_hashed_files,
            self.changed_retry_files,
            self.duplicate_sets
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct ImportPlanSummary {
    pub source_filter: Option<String>,
    pub scanned_files: u64,
    pub candidate_duplicate_groups: u64,
    pub confirmed_duplicate_groups: u64,
    pub unique_payload_count: u64,
    pub total_source_bytes: u64,
    pub estimated_import_bytes: u64,
    pub saved_bytes: u64,
}

impl ImportPlanSummary {
    pub fn human_readable(&self) -> String {
        let source_line = match self.source_filter.as_deref() {
            Some(source) => format!("source: {}\n", source),
            None => "source: all sources\n".to_string(),
        };
        format!(
            "FOD indexer dry-run import plan\n{}scanned files: {}\ncandidate duplicate groups: {}\nconfirmed duplicate groups: {}\nunique payloads: {}\nsource bytes: {}\nestimated import bytes: {}\nestimated saved bytes: {}",
            source_line,
            self.scanned_files,
            self.candidate_duplicate_groups,
            self.confirmed_duplicate_groups,
            self.unique_payload_count,
            self.total_source_bytes,
            self.estimated_import_bytes,
            self.saved_bytes
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct CleanupFailedSummary {
    pub plan_id: u64,
    pub source_name: String,
    pub import_root: String,
    pub removed_files: u64,
    pub removed_directories: u64,
    pub exclusive_data_objects_removed: u64,
    pub shared_data_objects_preserved: u64,
    pub plan_status_before: String,
    pub plan_status_after: String,
}

impl CleanupFailedSummary {
    pub fn human_readable(&self) -> String {
        format!(
            "FOD indexer cleanup failed materialization\nplan id: {}\nsource: {}\nimport root: {}\nfiles removed: {}\ndirectories removed: {}\nexclusive data objects removed: {}\nshared data objects preserved: {}\nplan status: {} -> {}",
            self.plan_id,
            self.source_name,
            self.import_root,
            self.removed_files,
            self.removed_directories,
            self.exclusive_data_objects_removed,
            self.shared_data_objects_preserved,
            self.plan_status_before,
            self.plan_status_after
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct CleanSourceSummary {
    pub source_name: String,
    pub source_path: String,
    pub dry_run: bool,
    pub source_root_missing: bool,
    pub indexed_files: u64,
    pub present_files: u64,
    pub stale_files: u64,
    pub skipped_files: u64,
    pub plan_entries_removed: u64,
    pub duplicate_sets_refreshed: u64,
}

impl CleanSourceSummary {
    pub fn human_readable(&self) -> String {
        let mode = if self.dry_run { "dry-run" } else { "clean" };
        let root_state = if self.source_root_missing {
            "missing"
        } else {
            "present"
        };
        format!(
            "FOD indexer clean\nmode: {}\nsource: {}\npath: {}\nsource root: {}\nindexed files: {}\npresent files: {}\nstale files: {}\nskipped files: {}\nplan entries removed: {}\nduplicate sets refreshed: {}",
            mode,
            self.source_name,
            self.source_path,
            root_state,
            self.indexed_files,
            self.present_files,
            self.stale_files,
            self.skipped_files,
            self.plan_entries_removed,
            self.duplicate_sets_refreshed
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct MaterializeSummary {
    pub source_name: String,
    pub import_root: String,
    pub dry_run: bool,
    pub scanned_files: u64,
    pub validated_files: u64,
    pub duplicate_groups: u64,
    pub canonical_files: u64,
    pub reference_files: u64,
    pub created_directories: u64,
    pub source_bytes: u64,
    pub imported_bytes: u64,
    pub saved_bytes: u64,
}

impl MaterializeSummary {
    pub fn as_import_plan_summary(&self) -> ImportPlanSummary {
        ImportPlanSummary {
            source_filter: None,
            scanned_files: self.validated_files,
            candidate_duplicate_groups: self.duplicate_groups,
            confirmed_duplicate_groups: self.duplicate_groups,
            unique_payload_count: self.canonical_files,
            total_source_bytes: self.source_bytes,
            estimated_import_bytes: self.imported_bytes,
            saved_bytes: self.saved_bytes,
        }
    }

    pub fn human_readable(&self) -> String {
        let mode = if self.dry_run {
            "dry-run"
        } else {
            "materialize"
        };
        format!(
            "FOD indexer materialize\nmode: {}\nsource: {}\nimport root: {}\nscanned files: {}\nvalidated files: {}\nduplicate groups: {}\ncanonical files: {}\nreference files: {}\ncreated directories: {}\nsource bytes: {}\nimported bytes: {}\nsaved bytes: {}",
            mode,
            self.source_name,
            self.import_root,
            self.scanned_files,
            self.validated_files,
            self.duplicate_groups,
            self.canonical_files,
            self.reference_files,
            self.created_directories,
            self.source_bytes,
            self.imported_bytes,
            self.saved_bytes
        )
    }
}

fn parse_u64(value: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("invalid u64 value `{value}`: {err}"))
}

fn parse_i64(value: &str) -> Result<i64, String> {
    value
        .trim()
        .parse::<i64>()
        .map_err(|err| format!("invalid i64 value `{value}`: {err}"))
}

fn parse_optional_u64(value: &str) -> Result<Option<u64>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    parse_u64(value).map(Some)
}

fn parse_optional_i64(value: &str) -> Result<Option<i64>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    parse_i64(value).map(Some)
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "t" | "true" | "1" | "on"
    )
}

fn row_value<'a>(row: &'a [String], index: usize, label: &str) -> Result<&'a str, String> {
    row.get(index)
        .map(|value| value.as_str())
        .ok_or_else(|| format!("missing column {index} for {label}"))
}

impl IndexSource {
    pub fn from_row(row: &[String]) -> Result<Self, String> {
        Ok(Self {
            id_source: parse_u64(row_value(row, 0, "index source")?)?,
            name: row_value(row, 1, "index source")?.to_string(),
            kind: row_value(row, 2, "index source")?.to_string(),
            root_path: PathBuf::from(row_value(row, 3, "index source")?),
        })
    }
}

impl IndexedFile {
    pub fn from_row(row: &[String]) -> Result<Self, String> {
        Ok(Self {
            id_file: parse_u64(row_value(row, 0, "indexed file")?)?,
            source_id: parse_u64(row_value(row, 1, "indexed file")?)?,
            source_name: row_value(row, 2, "indexed file")?.to_string(),
            root_path: PathBuf::from(row_value(row, 3, "indexed file")?),
            path: row_value(row, 4, "indexed file")?.to_string(),
            size: parse_u64(row_value(row, 5, "indexed file")?)?,
            mtime_ns: parse_optional_i64(row_value(row, 6, "indexed file")?)?,
            inode: parse_optional_u64(row_value(row, 7, "indexed file")?)?,
            device: parse_optional_u64(row_value(row, 8, "indexed file")?)?,
            file_kind: row_value(row, 9, "indexed file")?.to_string(),
            scan_status: row_value(row, 10, "indexed file")?.to_string(),
            source_changed: parse_bool(row_value(row, 11, "indexed file")?),
        })
    }

    pub fn source_path(&self) -> String {
        self.root_path.join(&self.path).display().to_string()
    }
}

impl FileHash {
    pub fn from_row(row: &[String]) -> Result<Self, String> {
        Ok(Self {
            id_file: parse_u64(row_value(row, 0, "index file hash")?)?,
            hash_algorithm: row_value(row, 1, "index file hash")?.to_string(),
            partial_hash_hex: {
                let value = row_value(row, 2, "index file hash")?;
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value.to_string())
                }
            },
            full_hash_hex: {
                let value = row_value(row, 3, "index file hash")?;
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value.to_string())
                }
            },
            hash_status: row_value(row, 4, "index file hash")?.to_string(),
            observed_size: parse_u64(row_value(row, 5, "index file hash")?)?,
            observed_mtime_ns: parse_optional_i64(row_value(row, 6, "index file hash")?)?,
            observed_inode: parse_optional_u64(row_value(row, 7, "index file hash")?)?,
            observed_device: parse_optional_u64(row_value(row, 8, "index file hash")?)?,
        })
    }
}

impl DuplicateSet {
    pub fn from_row(row: &[String]) -> Result<Self, String> {
        Ok(Self {
            id_duplicate_set: parse_u64(row_value(row, 0, "duplicate set")?)?,
            hash_algorithm: row_value(row, 1, "duplicate set")?.to_string(),
            full_hash_hex: row_value(row, 2, "duplicate set")?.to_string(),
            file_size: parse_u64(row_value(row, 3, "duplicate set")?)?,
            file_count: parse_u64(row_value(row, 4, "duplicate set")?)?,
            total_bytes: parse_u64(row_value(row, 5, "duplicate set")?)?,
        })
    }
}
