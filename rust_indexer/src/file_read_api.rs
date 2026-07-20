use crate::duplicate_set_api;
use crate::hash;
use crate::output::IndexerCapabilitiesOutput;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use fod_rust_hotpath::pg::DbRepo;
use serde::Serialize;
use std::fs::{self, File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
struct FileReadRecord {
    file_id: u64,
    source_id: u64,
    source_name: String,
    source_kind: String,
    source_root: PathBuf,
    path: String,
    size: u64,
    mtime_ns: Option<i64>,
    inode: Option<u64>,
    device: Option<u64>,
    file_kind: String,
    scan_status: String,
    source_changed: bool,
    hash_algorithm: Option<String>,
    partial_hash_hex: Option<String>,
    full_hash_hex: Option<String>,
    hash_status: Option<String>,
    hash_observed_size: Option<u64>,
    hash_observed_mtime_ns: Option<i64>,
    hash_observed_inode: Option<u64>,
    hash_observed_device: Option<u64>,
    scan_run_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileSnapshot {
    size: u64,
    mtime_ns: i64,
    inode: u64,
    device: u64,
}

impl FileSnapshot {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            size: metadata.len(),
            mtime_ns: metadata
                .mtime()
                .saturating_mul(1_000_000_000)
                .saturating_add(metadata.mtime_nsec()),
            inode: metadata.ino(),
            device: metadata.dev(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FileReadProvenance {
    pub file_id: u64,
    pub source_id: u64,
    pub source_name: String,
    pub source_kind: String,
    pub source_root: String,
    pub path: String,
    pub source_path: String,
    pub resolved_source_path: String,
    pub scan_run_id: Option<u64>,
    pub indexed_size: u64,
    pub indexed_mtime_ns: Option<i64>,
    pub indexed_inode: Option<u64>,
    pub indexed_device: Option<u64>,
    pub hash_algorithm: Option<String>,
    pub partial_hash_hex: Option<String>,
    pub full_hash_hex: Option<String>,
    pub hash_status: Option<String>,
    pub hash_observed_size: Option<u64>,
    pub hash_observed_mtime_ns: Option<i64>,
    pub hash_observed_inode: Option<u64>,
    pub hash_observed_device: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileReadValidation {
    pub status: &'static str,
    pub basis: &'static str,
    pub metadata_match: bool,
    pub indexed_hash_match: Option<bool>,
    pub observed_hash_algorithm: &'static str,
    pub observed_full_hash_hex: String,
    pub observed_size: u64,
    pub observed_mtime_ns: i64,
    pub observed_inode: u64,
    pub observed_device: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileReadRange {
    pub offset: u64,
    pub requested_length: Option<u64>,
    pub returned_length: u64,
    pub end_offset: u64,
    pub eof: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileReadOutput {
    pub consistency: &'static str,
    pub provenance: FileReadProvenance,
    pub validation: FileReadValidation,
    pub range: FileReadRange,
    pub encoding: &'static str,
    pub data_base64: String,
    #[serde(skip)]
    bytes: Vec<u8>,
}

impl FileReadOutput {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn provenance_human_readable(&self) -> String {
        format!(
            "FOD indexer file read\nfile_id: {}\nsource: {} (id={}, kind={})\npath: {}\nresolved path: {}\nscan_run_id: {}\nindexed size: {}\nindexed mtime_ns: {}\nhash status: {}\nindexed full hash: {}\nobserved sha256: {}\nvalidation: {} ({})\nrange: offset={} returned={} end={} eof={}\nraw bytes are written to stdout",
            self.provenance.file_id,
            self.provenance.source_name,
            self.provenance.source_id,
            self.provenance.source_kind,
            self.provenance.source_path,
            self.provenance.resolved_source_path,
            self.provenance
                .scan_run_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.provenance.indexed_size,
            self.provenance
                .indexed_mtime_ns
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.provenance.hash_status.as_deref().unwrap_or("none"),
            self.provenance.full_hash_hex.as_deref().unwrap_or("none"),
            self.validation.observed_full_hash_hex,
            self.validation.status,
            self.validation.basis,
            self.range.offset,
            self.range.returned_length,
            self.range.end_offset,
            self.range.eof,
        )
    }
}

pub fn capabilities_output() -> IndexerCapabilitiesOutput {
    let mut capabilities = duplicate_set_api::capabilities_output();
    if let Some(command) = capabilities
        .commands
        .iter_mut()
        .find(|command| command.command == "file read --id")
    {
        command.status = "available";
        command.notes = "Revalidates indexed metadata and stored hashes before returning source bytes. Text output writes raw bytes to stdout and provenance to stderr; JSON returns Base64 data and provenance.";
    }
    capabilities
}

pub fn read_file(
    repo: &DbRepo,
    file_id: u64,
    offset: u64,
    length: Option<u64>,
) -> Result<FileReadOutput, String> {
    if file_id == 0 {
        return Err("file_read_invalid_id: --id must be a positive file id".to_string());
    }
    let rows = repo.query_rows_text(&format!(
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
            COALESCE(encode(h.partial_hash, 'hex'), ''),
            COALESCE(encode(h.full_hash, 'hex'), ''),
            COALESCE(h.hash_status, ''),
            COALESCE(h.observed_size::text, ''),
            COALESCE(h.observed_mtime_ns::text, ''),
            COALESCE(h.observed_inode::text, ''),
            COALESCE(h.observed_device::text, ''),
            COALESCE(f.id_scan_run::text, '')
        FROM index_files f
        JOIN index_sources s ON s.id_index_source = f.id_index_source
        LEFT JOIN index_file_hashes h ON h.id_file = f.id_file
        WHERE f.id_file = {file_id}
        LIMIT 1
        "
    ))?;
    let row = rows
        .first()
        .ok_or_else(|| format!("file_read_not_found: indexed file {file_id} does not exist"))?;
    let record = file_read_record_from_row(row)?;
    read_revalidated_record(record, offset, length)
}

fn read_revalidated_record(
    record: FileReadRecord,
    offset: u64,
    length: Option<u64>,
) -> Result<FileReadOutput, String> {
    validate_index_record(&record)?;
    let relative_path = validate_relative_path(&record.path)?;
    let source_path = record.source_root.join(&relative_path);
    let canonical_root = fs::canonicalize(&record.source_root)
        .map_err(|err| classify_io_error("source root", &record.source_root, err))?;
    let canonical_path = fs::canonicalize(&source_path)
        .map_err(|err| classify_io_error("source file", &source_path, err))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(format!(
            "file_read_invalid_source_path: indexed path {} resolves outside source root {}",
            source_path.display(),
            canonical_root.display()
        ));
    }

    let mut file = File::open(&canonical_path)
        .map_err(|err| classify_io_error("source file", &canonical_path, err))?;
    let metadata_before = file
        .metadata()
        .map_err(|err| classify_io_error("open source file metadata", &canonical_path, err))?;
    if !metadata_before.is_file() {
        return Err(format!(
            "file_read_source_changed: {} is no longer a regular file",
            canonical_path.display()
        ));
    }
    let snapshot_before = FileSnapshot::from_metadata(&metadata_before);
    validate_snapshot(&record, snapshot_before)?;

    if let Some(algorithm) = record.hash_algorithm.as_deref() {
        if algorithm != hash::HASH_ALGORITHM {
            return Err(format!(
                "file_read_unsupported_hash: indexed hash algorithm {algorithm} is not supported"
            ));
        }
    }

    let observed_partial_hash_hex = if record.partial_hash_hex.is_some() {
        Some(crate::db::hex_encode(&hash::compute_partial_hash_from_file(
            &mut file,
            snapshot_before.size,
        )?))
    } else {
        None
    };
    let observed_full_hash_hex = crate::db::hex_encode(&hash::compute_full_hash_from_file(&mut file)?);

    let (validation_basis, indexed_hash_match) = if let Some(expected) = record.full_hash_hex.as_deref() {
        if observed_full_hash_hex != expected {
            return Err(format!(
                "file_read_source_changed: full hash differs for indexed file {} at {}",
                record.file_id,
                canonical_path.display()
            ));
        }
        ("metadata+full-hash", Some(true))
    } else if let Some(expected) = record.partial_hash_hex.as_deref() {
        if observed_partial_hash_hex.as_deref() != Some(expected) {
            return Err(format!(
                "file_read_source_changed: partial hash differs for indexed file {} at {}",
                record.file_id,
                canonical_path.display()
            ));
        }
        ("metadata+partial-hash", Some(true))
    } else {
        ("metadata-only", None)
    };

    let (bytes, range) = read_range(&mut file, snapshot_before.size, offset, length)?;

    let metadata_after = file
        .metadata()
        .map_err(|err| classify_io_error("source file metadata after read", &canonical_path, err))?;
    let snapshot_after = FileSnapshot::from_metadata(&metadata_after);
    if snapshot_after != snapshot_before {
        return Err(format!(
            "file_read_source_changed: {} changed while it was being validated or read",
            canonical_path.display()
        ));
    }
    let path_metadata_after = fs::metadata(&canonical_path)
        .map_err(|err| classify_io_error("source file after read", &canonical_path, err))?;
    let path_snapshot_after = FileSnapshot::from_metadata(&path_metadata_after);
    if path_snapshot_after.inode != snapshot_before.inode
        || path_snapshot_after.device != snapshot_before.device
    {
        return Err(format!(
            "file_read_source_changed: {} was replaced while it was being read",
            canonical_path.display()
        ));
    }

    let provenance = FileReadProvenance {
        file_id: record.file_id,
        source_id: record.source_id,
        source_name: record.source_name,
        source_kind: record.source_kind,
        source_root: record.source_root.display().to_string(),
        path: record.path,
        source_path: source_path.display().to_string(),
        resolved_source_path: canonical_path.display().to_string(),
        scan_run_id: record.scan_run_id,
        indexed_size: record.size,
        indexed_mtime_ns: record.mtime_ns,
        indexed_inode: record.inode,
        indexed_device: record.device,
        hash_algorithm: record.hash_algorithm,
        partial_hash_hex: record.partial_hash_hex,
        full_hash_hex: record.full_hash_hex,
        hash_status: record.hash_status,
        hash_observed_size: record.hash_observed_size,
        hash_observed_mtime_ns: record.hash_observed_mtime_ns,
        hash_observed_inode: record.hash_observed_inode,
        hash_observed_device: record.hash_observed_device,
    };
    Ok(FileReadOutput {
        consistency: "revalidated-source-read",
        provenance,
        validation: FileReadValidation {
            status: "ok",
            basis: validation_basis,
            metadata_match: true,
            indexed_hash_match,
            observed_hash_algorithm: hash::HASH_ALGORITHM,
            observed_full_hash_hex,
            observed_size: snapshot_before.size,
            observed_mtime_ns: snapshot_before.mtime_ns,
            observed_inode: snapshot_before.inode,
            observed_device: snapshot_before.device,
        },
        range,
        encoding: "base64",
        data_base64: BASE64_STANDARD.encode(&bytes),
        bytes,
    })
}

fn validate_index_record(record: &FileReadRecord) -> Result<(), String> {
    if record.file_kind != "regular" || record.scan_status != "ok" {
        return Err(format!(
            "file_read_unavailable: indexed file {} has file_kind={} scan_status={}",
            record.file_id, record.file_kind, record.scan_status
        ));
    }
    if record.source_changed
        || matches!(record.hash_status.as_deref(), Some("changed_retry_needed"))
    {
        return Err(format!(
            "file_read_source_changed: indexed file {} is already marked as changed",
            record.file_id
        ));
    }
    Ok(())
}

fn validate_snapshot(record: &FileReadRecord, observed: FileSnapshot) -> Result<(), String> {
    let indexed_match = observed.size == record.size
        && record.mtime_ns.is_none_or(|value| value == observed.mtime_ns)
        && record.inode.is_none_or(|value| value == observed.inode)
        && record.device.is_none_or(|value| value == observed.device);
    let hash_observation_match = record
        .hash_observed_size
        .is_none_or(|value| value == observed.size)
        && record
            .hash_observed_mtime_ns
            .is_none_or(|value| value == observed.mtime_ns)
        && record
            .hash_observed_inode
            .is_none_or(|value| value == observed.inode)
        && record
            .hash_observed_device
            .is_none_or(|value| value == observed.device);
    if !indexed_match || !hash_observation_match {
        return Err(format!(
            "file_read_source_changed: metadata differs for indexed file {}",
            record.file_id
        ));
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(path);
    if candidate.as_os_str().is_empty() {
        return Err("file_read_invalid_source_path: indexed path is empty".to_string());
    }
    for component in candidate.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "file_read_invalid_source_path: indexed path {path:?} is not a safe relative path"
                ))
            }
        }
    }
    Ok(candidate.to_path_buf())
}

fn read_range(
    file: &mut File,
    file_size: u64,
    offset: u64,
    requested_length: Option<u64>,
) -> Result<(Vec<u8>, FileReadRange), String> {
    if offset > file_size {
        return Err(format!(
            "file_read_invalid_range: offset {offset} exceeds file size {file_size}"
        ));
    }
    let available = file_size.saturating_sub(offset);
    let returned_length = requested_length.unwrap_or(available).min(available);
    let buffer_length = usize::try_from(returned_length).map_err(|_| {
        format!(
            "file_read_invalid_range: requested range length {returned_length} cannot fit in memory on this platform"
        )
    })?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|err| format!("file_read_io_error: seek to offset {offset} failed: {err}"))?;
    let mut bytes = vec![0u8; buffer_length];
    let mut read_total = 0usize;
    while read_total < bytes.len() {
        let read = file
            .read(&mut bytes[read_total..])
            .map_err(|err| format!("file_read_io_error: range read failed: {err}"))?;
        if read == 0 {
            break;
        }
        read_total = read_total.saturating_add(read);
    }
    if read_total != buffer_length {
        return Err(format!(
            "file_read_source_changed: source ended after {read_total} bytes, expected {buffer_length}"
        ));
    }
    let end_offset = offset.saturating_add(returned_length);
    Ok((
        bytes,
        FileReadRange {
            offset,
            requested_length,
            returned_length,
            end_offset,
            eof: end_offset >= file_size,
        },
    ))
}

fn classify_io_error(action: &str, path: &Path, err: std::io::Error) -> String {
    let code = match err.kind() {
        std::io::ErrorKind::NotFound => "file_read_source_missing",
        std::io::ErrorKind::PermissionDenied => "file_read_access_denied",
        _ => "file_read_io_error",
    };
    format!("{code}: unable to access {action} {}: {err}", path.display())
}

fn file_read_record_from_row(row: &[String]) -> Result<FileReadRecord, String> {
    if row.len() < 22 {
        return Err("file read row is too short".to_string());
    }
    Ok(FileReadRecord {
        file_id: parse_u64(&row[0], "file id")?,
        source_id: parse_u64(&row[1], "source id")?,
        source_name: row[2].clone(),
        source_kind: row[3].clone(),
        source_root: PathBuf::from(&row[4]),
        path: row[5].clone(),
        size: parse_u64(&row[6], "file size")?,
        mtime_ns: parse_optional_i64(&row[7], "file mtime_ns")?,
        inode: parse_optional_u64(&row[8], "file inode")?,
        device: parse_optional_u64(&row[9], "file device")?,
        file_kind: row[10].clone(),
        scan_status: row[11].clone(),
        source_changed: parse_bool(&row[12]),
        hash_algorithm: optional_text(&row[13]),
        partial_hash_hex: optional_text(&row[14]),
        full_hash_hex: optional_text(&row[15]),
        hash_status: optional_text(&row[16]),
        hash_observed_size: parse_optional_u64(&row[17], "hash observed size")?,
        hash_observed_mtime_ns: parse_optional_i64(&row[18], "hash observed mtime_ns")?,
        hash_observed_inode: parse_optional_u64(&row[19], "hash observed inode")?,
        hash_observed_device: parse_optional_u64(&row[20], "hash observed device")?,
        scan_run_id: parse_optional_u64(&row[21], "scan run id")?,
    })
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
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
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_ID: AtomicU64 = AtomicU64::new(1);

    fn fixture_record(contents: &[u8]) -> (PathBuf, FileReadRecord) {
        let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("fod-file-read-{}-{id}", std::process::id()));
        fs::create_dir_all(&root).expect("fixture root should be created");
        let path = root.join("fixture.bin");
        let mut output = File::create(&path).expect("fixture file should be created");
        output.write_all(contents).expect("fixture should be written");
        output.sync_all().expect("fixture should be synced");
        drop(output);
        let metadata = fs::metadata(&path).expect("fixture metadata should be readable");
        let snapshot = FileSnapshot::from_metadata(&metadata);
        let mut input = File::open(&path).expect("fixture should open");
        let partial_hash_hex = crate::db::hex_encode(
            &hash::compute_partial_hash_from_file(&mut input, snapshot.size)
                .expect("partial hash should compute"),
        );
        let full_hash_hex = crate::db::hex_encode(
            &hash::compute_full_hash_from_file(&mut input).expect("full hash should compute"),
        );
        (
            root.clone(),
            FileReadRecord {
                file_id: 17,
                source_id: 3,
                source_name: "fixture".to_string(),
                source_kind: "local".to_string(),
                source_root: root,
                path: "fixture.bin".to_string(),
                size: snapshot.size,
                mtime_ns: Some(snapshot.mtime_ns),
                inode: Some(snapshot.inode),
                device: Some(snapshot.device),
                file_kind: "regular".to_string(),
                scan_status: "ok".to_string(),
                source_changed: false,
                hash_algorithm: Some(hash::HASH_ALGORITHM.to_string()),
                partial_hash_hex: Some(partial_hash_hex),
                full_hash_hex: Some(full_hash_hex),
                hash_status: Some("full".to_string()),
                hash_observed_size: Some(snapshot.size),
                hash_observed_mtime_ns: Some(snapshot.mtime_ns),
                hash_observed_inode: Some(snapshot.inode),
                hash_observed_device: Some(snapshot.device),
                scan_run_id: Some(9),
            },
        )
    }

    #[test]
    fn reads_revalidated_range_and_encodes_base64() {
        let (root, record) = fixture_record(b"abcdefgh");
        let output = read_revalidated_record(record, 2, Some(3)).expect("range should read");
        assert_eq!(output.bytes(), b"cde");
        assert_eq!(output.data_base64, "Y2Rl");
        assert_eq!(output.range.offset, 2);
        assert_eq!(output.range.returned_length, 3);
        assert!(!output.range.eof);
        assert_eq!(output.validation.basis, "metadata+full-hash");
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn detects_changed_source_before_returning_bytes() {
        let (root, record) = fixture_record(b"abcdefgh");
        let path = root.join("fixture.bin");
        let mut output = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("fixture should reopen");
        output.write_all(b"changed").expect("fixture should change");
        drop(output);
        let error = read_revalidated_record(record, 0, None).expect_err("change should fail");
        assert!(error.starts_with("file_read_source_changed:"));
        fs::remove_dir_all(root).expect("fixture should be removed");
    }

    #[test]
    fn rejects_parent_directory_paths() {
        assert!(validate_relative_path("../secret").is_err());
        assert!(validate_relative_path("/absolute/path").is_err());
        assert!(validate_relative_path("safe/path").is_ok());
    }

    #[test]
    fn validates_range_bounds() {
        let (root, record) = fixture_record(b"abc");
        let error = read_revalidated_record(record, 4, None).expect_err("offset should fail");
        assert!(error.starts_with("file_read_invalid_range:"));
        fs::remove_dir_all(root).expect("fixture should be removed");
    }
}
