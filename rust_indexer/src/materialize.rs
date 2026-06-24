use crate::db::{ensure_indexer_request_token_schema, hex_encode};
use crate::model::{IndexSource, MaterializeSummary};
use crate::plan::{self, canonical_sort_key, PlannableFile};
use crate::{hash, scan};
use fod_rust_hotpath::block_count_for_length;
use fod_rust_hotpath::pg::{DbRepo, PersistBlockRow};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const DEFAULT_BLOCK_SIZE: u64 = 4096;
type MaterializeGroups = BTreeMap<(String, String, u64), Vec<ValidatedCandidate>>;

fn mtime_ns(metadata: &fs::Metadata) -> i64 {
    metadata
        .mtime()
        .saturating_mul(1_000_000_000)
        .saturating_add(metadata.mtime_nsec())
}

fn collect_full_hash(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("unable to open {} for hashing: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("read failed while hashing {}: {err}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

fn collect_file_blocks(path: &Path, block_size: usize) -> Result<(Vec<Vec<u8>>, String), String> {
    let mut file = File::open(path)
        .map_err(|err| format!("unable to open {} for import: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut blocks = Vec::new();
    let mut buffer = vec![0u8; block_size.max(1)];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("read failed while importing {}: {err}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        blocks.push(buffer[..read].to_vec());
    }

    Ok((blocks, hex_encode(&hasher.finalize())))
}

fn source_path(candidate: &PlannableFile) -> PathBuf {
    candidate.file.root_path.join(&candidate.file.path)
}

#[derive(Debug, Clone)]
struct ValidatedCandidate {
    file: PlannableFile,
    full_hash_hex: String,
}

fn validate_candidate(candidate: PlannableFile) -> Result<ValidatedCandidate, String> {
    let path = source_path(&candidate);

    let metadata_before = fs::metadata(&path)
        .map_err(|err| {
            format!(
                "unable to read metadata for {}: {err}; validation aborts before any imported data is created",
                path.display()
            )
        })?;
    if metadata_before.len() != candidate.file.size
        || candidate.file.mtime_ns != Some(mtime_ns(&metadata_before))
        || candidate.file.inode != Some(metadata_before.ino())
        || candidate.file.device != Some(metadata_before.dev())
    {
        return Err(format!(
            "source file changed before import: {}; validation aborts before any imported data is created",
            path.display()
        ));
    }

    let actual_hash = collect_full_hash(&path)?;
    let metadata_after = fs::metadata(&path)
        .map_err(|err| {
            format!(
                "unable to reread metadata for {}: {err}; validation aborts before any imported data is created",
                path.display()
            )
        })?;
    if metadata_after.len() != metadata_before.len()
        || mtime_ns(&metadata_after) != mtime_ns(&metadata_before)
        || metadata_after.ino() != metadata_before.ino()
        || metadata_after.dev() != metadata_before.dev()
    {
        return Err(format!(
            "source file changed while validating import candidate: {}; validation aborts before any imported data is created",
            path.display()
        ));
    }

    if let Some(expected_hash) = candidate.full_hash_hex.as_deref() {
        if actual_hash != expected_hash {
            return Err(format!(
                "full hash changed before import for {}; validation aborts before any imported data is created",
                path.display()
            ));
        }
    }

    Ok(ValidatedCandidate {
        file: candidate,
        full_hash_hex: actual_hash,
    })
}

fn group_validated_candidates(candidates: Vec<ValidatedCandidate>) -> MaterializeGroups {
    let mut groups: MaterializeGroups = BTreeMap::new();
    for file in candidates {
        let key = (
            file.file
                .hash_algorithm
                .clone()
                .unwrap_or_else(|| "sha256".to_string()),
            file.full_hash_hex.clone(),
            file.file.file.size,
        );
        groups.entry(key).or_default().push(file);
    }
    groups
}

fn summarize_materialize_preview(
    source_name: &str,
    groups: &MaterializeGroups,
    import_root: &str,
) -> MaterializeSummary {
    let mut summary = MaterializeSummary {
        source_name: source_name.to_string(),
        import_root: import_root.to_string(),
        dry_run: true,
        ..MaterializeSummary::default()
    };

    for members in groups.values() {
        let member_count = members.len() as u64;
        if member_count > 1 {
            summary.duplicate_groups = summary.duplicate_groups.saturating_add(1);
        }
        summary.canonical_files = summary.canonical_files.saturating_add(1);
        summary.reference_files = summary
            .reference_files
            .saturating_add(member_count.saturating_sub(1));
        summary.scanned_files = summary.scanned_files.saturating_add(member_count);
        summary.validated_files = summary.validated_files.saturating_add(member_count);
        let group_source_bytes: u64 = members.iter().map(|member| member.file.file.size).sum();
        summary.source_bytes = summary.source_bytes.saturating_add(group_source_bytes);
        if let Some(canonical) = members.first() {
            summary.imported_bytes = summary
                .imported_bytes
                .saturating_add(canonical.file.file.size);
        }
    }

    summary.saved_bytes = summary.source_bytes.saturating_sub(summary.imported_bytes);
    summary
}

fn load_block_size(repo: &DbRepo) -> Result<u64, String> {
    let block_size = match repo.query_config_value("block_size")? {
        Some(value) => value
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid block_size value: {err}"))?,
        None => DEFAULT_BLOCK_SIZE,
    };
    if block_size == 0 {
        return Err("block_size must be greater than zero".to_string());
    }
    Ok(block_size)
}

fn ensure_root_directory(
    repo: &DbRepo,
    source: &IndexSource,
    root_name: &str,
) -> Result<(u64, u64), String> {
    if let Some(id) = repo.get_dir_id(root_name)? {
        return Ok((id, 0));
    }

    let metadata = fs::metadata(&source.root_path).map_err(|err| {
        format!(
            "unable to read root directory metadata for {}: {err}",
            source.root_path.display()
        )
    })?;
    let inode_seed = format!("indexer:root:{}:{}", source.id_source, root_name);
    let created = repo.create_directory(
        None,
        root_name,
        (metadata.mode() & 0o7777) as u32,
        metadata.uid(),
        metadata.gid(),
        &inode_seed,
    )?;
    Ok((created, 1))
}

fn ensure_directory_chain(
    repo: &DbRepo,
    source_root: &Path,
    root_name: &str,
    root_id: u64,
    relative_parent: &Path,
    source_id: u64,
) -> Result<(u64, u64), String> {
    let mut current_dir_id = root_id;
    let mut current_fod_path = root_name.to_string();
    let mut current_source_path = PathBuf::new();
    let mut created = 0u64;

    for component in relative_parent.components() {
        let component_text = component.as_os_str().to_string_lossy().to_string();
        current_source_path.push(component.as_os_str());
        current_fod_path = format!("{}/{}", current_fod_path, component_text);

        if let Some(existing) = repo.get_dir_id(&current_fod_path)? {
            current_dir_id = existing;
            continue;
        }

        let source_dir = source_root.join(&current_source_path);
        let metadata = fs::metadata(&source_dir).map_err(|err| {
            format!(
                "unable to read source directory metadata for {}: {err}",
                source_dir.display()
            )
        })?;
        let inode_seed = format!(
            "indexer:dir:{}:{}:{}",
            source_id,
            current_fod_path,
            source_dir.display()
        );
        current_dir_id = repo.create_directory(
            Some(current_dir_id),
            &component_text,
            (metadata.mode() & 0o7777) as u32,
            metadata.uid(),
            metadata.gid(),
            &inode_seed,
        )?;
        created = created.saturating_add(1);
    }

    Ok((current_dir_id, created))
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .map(|value| value.to_string_lossy().to_string())
        .ok_or_else(|| format!("path has no file name: {}", path.display()))
}

fn materialize_canonical_file(
    repo: &DbRepo,
    source: &IndexSource,
    root_name: &str,
    root_id: u64,
    candidate: &PlannableFile,
    expected_hash: &str,
    block_size: u64,
) -> Result<(u64, u64), String> {
    let path = source_path(candidate);
    let metadata = fs::metadata(&path)
        .map_err(|err| format!("unable to read metadata for {}: {err}", path.display()))?;
    let relative_path = Path::new(&candidate.file.path);
    let parent_relative = relative_path.parent().unwrap_or_else(|| Path::new(""));
    let (parent_dir_id, created_dirs) = ensure_directory_chain(
        repo,
        &source.root_path,
        root_name,
        root_id,
        parent_relative,
        source.id_source,
    )?;
    let import_name = file_name(relative_path)?;
    let inode_seed = format!(
        "indexer:file:{}:{}:{}",
        candidate.file.id_file, candidate.file.path, candidate.file.source_name
    );
    let file_id = repo.create_file(
        Some(parent_dir_id),
        &import_name,
        (metadata.mode() & 0o7777) as u32,
        metadata.uid(),
        metadata.gid(),
        &inode_seed,
    )?;

    let size = metadata.len();
    if size == 0 {
        return Ok((file_id, created_dirs));
    }

    let (blocks, actual_hash) = collect_file_blocks(
        &path,
        usize::try_from(block_size)
            .map_err(|_| format!("block size is too large for this platform: {block_size}"))?,
    )?;
    if actual_hash != expected_hash {
        return Err(format!(
            "full hash changed while importing {}",
            path.display()
        ));
    }

    let block_rows = blocks
        .iter()
        .enumerate()
        .map(|(block_index, block)| PersistBlockRow {
            block_index: block_index as u64,
            data: block.as_slice(),
            used_len: block.len() as u64,
        })
        .collect::<Vec<_>>();
    let total_blocks = block_count_for_length(size, block_size, false);
    repo.persist_file_blocks(file_id, size, block_size, total_blocks, false, &block_rows)?;
    Ok((file_id, created_dirs))
}

fn materialize_reference_file(
    repo: &DbRepo,
    source: &IndexSource,
    root_name: &str,
    root_id: u64,
    candidate: &PlannableFile,
    canonical_file_id: u64,
) -> Result<u64, String> {
    let path = source_path(candidate);
    let metadata = fs::metadata(&path)
        .map_err(|err| format!("unable to read metadata for {}: {err}", path.display()))?;
    let relative_path = Path::new(&candidate.file.path);
    let parent_relative = relative_path.parent().unwrap_or_else(|| Path::new(""));
    let (parent_dir_id, created_dirs) = ensure_directory_chain(
        repo,
        &source.root_path,
        root_name,
        root_id,
        parent_relative,
        source.id_source,
    )?;
    let import_name = file_name(relative_path)?;
    let inode_seed = format!(
        "indexer:file:{}:{}:{}",
        candidate.file.id_file, candidate.file.path, candidate.file.source_name
    );
    let file_id = repo.create_file(
        Some(parent_dir_id),
        &import_name,
        (metadata.mode() & 0o7777) as u32,
        metadata.uid(),
        metadata.gid(),
        &inode_seed,
    )?;

    if metadata.len() > 0 {
        let adopted = repo.adopt_source_data_object(canonical_file_id, file_id)?;
        if !adopted {
            return Err(format!(
                "failed to reuse canonical payload for {}",
                path.display()
            ));
        }
    }

    Ok(created_dirs)
}

pub fn materialize_source(
    repo: &DbRepo,
    source_name: &str,
    dry_run: bool,
) -> Result<MaterializeSummary, String> {
    let source = scan::load_source(repo, source_name)?;
    if source.kind != "local" {
        return Err(format!(
            "source {source_name} is registered as kind {} and cannot be materialized by the local indexer",
            source.kind
        ));
    }

    if !dry_run {
        ensure_indexer_request_token_schema(repo, "fod-indexer materialize")?;

        // Refresh the source first so the materialization stage works from a fresh scan and hash view.
        scan::scan_source(repo, source_name)?;
        hash::hash_source(repo, source_name, false)?;
    }

    let candidates = plan::load_plannable_files(repo, Some(source.name.as_str()))?
        .into_iter()
        .map(validate_candidate)
        .collect::<Result<Vec<_>, _>>()?;
    let groups = group_validated_candidates(candidates);

    if dry_run {
        return Ok(summarize_materialize_preview(
            &source.name,
            &groups,
            "<dry-run>",
        ));
    }

    let duplicate_sets = plan::load_duplicate_sets(repo)?;
    let duplicate_set_map: HashMap<(String, String, u64), u64> = duplicate_sets
        .into_iter()
        .map(|duplicate_set| {
            (
                (
                    duplicate_set.hash_algorithm,
                    duplicate_set.full_hash_hex,
                    duplicate_set.file_size,
                ),
                duplicate_set.id_duplicate_set,
            )
        })
        .collect();

    let plan_id = plan::insert_import_plan(
        repo,
        "materialize_running",
        false,
        Some(source.name.as_str()),
    )?;
    let root_name = plan::import_root_name(source.id_source, plan_id);
    let (root_id, root_created) = ensure_root_directory(repo, &source, &root_name)?;
    let block_size = load_block_size(repo)?;
    let scanned_files = groups.values().map(|members| members.len() as u64).sum();
    let source_bytes = groups
        .values()
        .map(|members| {
            members
                .iter()
                .map(|member| member.file.file.size)
                .sum::<u64>()
        })
        .sum();
    let mut summary = MaterializeSummary {
        source_name: source.name.clone(),
        import_root: format!("/{}", root_name),
        dry_run: false,
        scanned_files,
        validated_files: scanned_files,
        source_bytes,
        created_directories: root_created,
        ..MaterializeSummary::default()
    };

    let result = (|| -> Result<(), String> {
        for ((algorithm, full_hash_hex, file_size), mut members) in groups {
            members.sort_by(|left, right| {
                canonical_sort_key(&left.file.file).cmp(&canonical_sort_key(&right.file.file))
            });
            let duplicate_set_id = duplicate_set_map
                .get(&(algorithm.clone(), full_hash_hex.clone(), file_size))
                .copied();
            let is_duplicate_group = members.len() > 1;
            if is_duplicate_group {
                summary.duplicate_groups = summary.duplicate_groups.saturating_add(1);
            }

            let canonical = members
                .first()
                .expect("group must contain at least one file");
            let (canonical_file_id, created_dirs) = materialize_canonical_file(
                repo,
                &source,
                &root_name,
                root_id,
                &canonical.file,
                &canonical.full_hash_hex,
                block_size,
            )?;
            summary.created_directories = summary.created_directories.saturating_add(created_dirs);
            summary.canonical_files = summary.canonical_files.saturating_add(1);
            summary.imported_bytes = summary
                .imported_bytes
                .saturating_add(canonical.file.file.size);
            plan::insert_import_plan_entry(
                repo,
                plan_id,
                &canonical.file.file,
                duplicate_set_id,
                if is_duplicate_group {
                    "materialized_canonical"
                } else {
                    "materialized_unique"
                },
                Some(canonical.file.file.id_file),
            )?;

            for reference in members.iter().skip(1) {
                let created_dirs = materialize_reference_file(
                    repo,
                    &source,
                    &root_name,
                    root_id,
                    &reference.file,
                    canonical_file_id,
                )?;
                summary.created_directories =
                    summary.created_directories.saturating_add(created_dirs);
                summary.reference_files = summary.reference_files.saturating_add(1);
                plan::insert_import_plan_entry(
                    repo,
                    plan_id,
                    &reference.file.file,
                    duplicate_set_id,
                    "materialized_reference",
                    Some(canonical.file.file.id_file),
                )?;
            }
        }

        Ok(())
    })();

    match result {
        Ok(()) => {
            summary.saved_bytes = summary.source_bytes.saturating_sub(summary.imported_bytes);
            let plan_summary = summary.as_import_plan_summary();
            plan::update_import_plan(repo, plan_id, "materialize_completed", false, &plan_summary)?;
            Ok(summary)
        }
        Err(err) => {
            let plan_summary = summary.as_import_plan_summary();
            let _ =
                plan::update_import_plan(repo, plan_id, "materialize_failed", false, &plan_summary);
            Err(err)
        }
    }
}
