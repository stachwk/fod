use crate::db::{sql_nullable_u64, sql_quote_literal};
use crate::hash;
use crate::model::{DuplicateSet, ImportPlanSummary, IndexedFile};
use fod_rust_hotpath::pg::DbRepo;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone)]
pub(crate) struct PlannableFile {
    pub(crate) file: IndexedFile,
    pub(crate) hash_algorithm: Option<String>,
    pub(crate) full_hash_hex: Option<String>,
    pub(crate) hash_status: Option<String>,
}

impl PlannableFile {
    pub(crate) fn from_row(row: &[String]) -> Result<Self, String> {
        if row.len() < 21 {
            return Err("plannable file row is too short".to_string());
        }
        let file = IndexedFile::from_row(&row[0..12])?;
        let hash_present = !row[12].trim().is_empty();
        let hash = if hash_present {
            Some(crate::model::FileHash::from_row(&row[12..21])?)
        } else {
            None
        };
        Ok(Self {
            file,
            hash_algorithm: hash.as_ref().map(|value| value.hash_algorithm.clone()),
            full_hash_hex: hash.as_ref().and_then(|value| value.full_hash_hex.clone()),
            hash_status: hash.as_ref().map(|value| value.hash_status.clone()),
        })
    }

    pub(crate) fn needs_revalidation(&self) -> bool {
        self.file.source_changed
            || matches!(self.hash_status.as_deref(), Some("changed_retry_needed"))
    }
}

pub(crate) fn load_duplicate_sets(repo: &DbRepo) -> Result<Vec<DuplicateSet>, String> {
    let rows = repo.query_rows_text(
        "
        SELECT
            id_duplicate_set,
            hash_algorithm,
            COALESCE(encode(full_hash, 'hex'), ''),
            file_size,
            file_count,
            total_bytes
        FROM index_duplicate_sets
        ORDER BY id_duplicate_set
        ",
    )?;
    rows.iter()
        .map(|row| DuplicateSet::from_row(row))
        .collect::<Result<Vec<_>, _>>()
}

pub(crate) fn load_plannable_files(
    repo: &DbRepo,
    source_filter: Option<&str>,
) -> Result<Vec<PlannableFile>, String> {
    let source_clause = source_filter
        .map(|source| format!(" AND s.name = {}", sql_quote_literal(source)))
        .unwrap_or_default();
    let rows = repo.query_rows_text(&format!(
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
            f.source_changed::text,
            COALESCE(h.id_file::text, ''),
            COALESCE(h.hash_algorithm, ''),
            COALESCE(encode(h.partial_hash, 'hex'), ''),
            COALESCE(encode(h.full_hash, 'hex'), ''),
            COALESCE(h.hash_status, ''),
            COALESCE(h.observed_size::text, ''),
            COALESCE(h.observed_mtime_ns::text, ''),
            COALESCE(h.observed_inode::text, ''),
            COALESCE(h.observed_device::text, '')
        FROM index_files f
        JOIN index_sources s ON s.id_index_source = f.id_index_source
        LEFT JOIN index_file_hashes h ON h.id_file = f.id_file
        WHERE f.scan_status = 'ok' AND f.file_kind = 'regular'
        {source_clause}
        ORDER BY f.id_index_source, length(f.path), f.path
        ",
    ))?;

    rows.iter()
        .map(|row| PlannableFile::from_row(row))
        .collect::<Result<Vec<_>, _>>()
}

pub(crate) fn canonical_sort_key(file: &IndexedFile) -> (u64, usize, String) {
    (file.source_id, file.path.len(), file.path.clone())
}

pub(crate) fn import_root_name(source_id: u64, plan_id: u64) -> String {
    format!("index-source-{}-import-{}", source_id, plan_id)
}

pub(crate) fn insert_import_plan(
    repo: &DbRepo,
    status: &str,
    dry_run: bool,
    source_filter: Option<&str>,
) -> Result<u64, String> {
    let sql = format!(
        "
        INSERT INTO index_import_plans (created_at, updated_at, status, dry_run, source_filter)
        VALUES (NOW(), NOW(), {status}, {dry_run}, {source_filter})
        RETURNING id_import_plan
        ",
        status = sql_quote_literal(status),
        dry_run = if dry_run { "TRUE" } else { "FALSE" },
        source_filter = source_filter
            .map(sql_quote_literal)
            .unwrap_or_else(|| "NULL".to_string()),
    );
    let rows = repo.query_rows_text(&sql)?;
    let row = rows
        .first()
        .ok_or_else(|| "import plan creation did not return an id".to_string())?;
    row.first()
        .ok_or_else(|| "import plan creation returned no id".to_string())?
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("invalid import plan id: {err}"))
}

pub(crate) fn update_import_plan(
    repo: &DbRepo,
    plan_id: u64,
    status: &str,
    dry_run: bool,
    summary: &ImportPlanSummary,
) -> Result<(), String> {
    let sql = format!(
        "
        UPDATE index_import_plans
        SET status = {status},
            dry_run = {dry_run},
            scanned_file_count = {scanned_file_count},
            candidate_group_count = {candidate_group_count},
            confirmed_group_count = {confirmed_group_count},
            unique_payload_count = {unique_payload_count},
            total_source_bytes = {total_source_bytes},
            estimated_import_bytes = {estimated_import_bytes},
            saved_bytes = {saved_bytes},
            updated_at = NOW()
        WHERE id_import_plan = {plan_id}
        ",
        status = sql_quote_literal(status),
        dry_run = if dry_run { "TRUE" } else { "FALSE" },
        scanned_file_count = summary.scanned_files,
        candidate_group_count = summary.candidate_duplicate_groups,
        confirmed_group_count = summary.confirmed_duplicate_groups,
        unique_payload_count = summary.unique_payload_count,
        total_source_bytes = summary.total_source_bytes,
        estimated_import_bytes = summary.estimated_import_bytes,
        saved_bytes = summary.saved_bytes,
        plan_id = plan_id,
    );
    repo.exec(&sql)
}

pub(crate) fn insert_import_plan_entry(
    repo: &DbRepo,
    plan_id: u64,
    file: &IndexedFile,
    duplicate_set_id: Option<u64>,
    action: &str,
    canonical_file_id: Option<u64>,
) -> Result<(), String> {
    let sql = format!(
        "
        INSERT INTO index_import_plan_entries (
            id_import_plan,
            id_file,
            id_duplicate_set,
            action,
            canonical_file_id,
            logical_path,
            source_path,
            size,
            mtime_ns,
            source_changed,
            created_at,
            updated_at
        )
        VALUES (
            {plan_id},
            {file_id},
            {duplicate_set_id},
            {action},
            {canonical_file_id},
            {logical_path},
            {source_path},
            {size},
            {mtime_ns},
            {source_changed},
            NOW(),
            NOW()
        )
        ",
        plan_id = plan_id,
        file_id = file.id_file,
        duplicate_set_id = sql_nullable_u64(duplicate_set_id),
        action = sql_quote_literal(action),
        canonical_file_id = sql_nullable_u64(canonical_file_id),
        logical_path = sql_quote_literal(&file.path),
        source_path = sql_quote_literal(&file.source_path()),
        size = file.size,
        mtime_ns = match file.mtime_ns {
            Some(value) => value.to_string(),
            None => "NULL".to_string(),
        },
        source_changed = if file.source_changed { "TRUE" } else { "FALSE" },
    );
    repo.exec(&sql)
}

pub fn report_duplicate_sets(repo: &DbRepo, limit: usize) -> Result<(), String> {
    hash::rebuild_duplicate_sets(repo)?;
    let rows = repo.query_rows_text(
        "
        SELECT
            ds.id_duplicate_set,
            ds.hash_algorithm,
            COALESCE(encode(ds.full_hash, 'hex'), ''),
            ds.file_size,
            ds.file_count,
            ds.total_bytes,
            f.id_file,
            f.id_index_source,
            s.name,
            s.root_path,
            f.path
        FROM index_duplicate_sets ds
        JOIN index_file_hashes h
            ON h.hash_algorithm = ds.hash_algorithm
           AND h.full_hash = ds.full_hash
           AND h.observed_size = ds.file_size
        JOIN index_files f ON f.id_file = h.id_file
        JOIN index_sources s ON s.id_index_source = f.id_index_source
        ORDER BY ds.id_duplicate_set, f.id_index_source, length(f.path), f.path
        ",
    )?;

    let mut sets: BTreeMap<u64, (DuplicateSet, Vec<PlannableFile>)> = BTreeMap::new();
    for row in rows {
        if row.len() < 11 {
            continue;
        }
        let set_id = row[0]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid duplicate set id: {err}"))?;
        let duplicate_set = DuplicateSet::from_row(&row[0..6])?;
        let file = IndexedFile::from_row(&[
            row[6].clone(),
            row[7].clone(),
            row[8].clone(),
            row[9].clone(),
            row[10].clone(),
            duplicate_set.file_size.to_string(),
            String::new(),
            String::new(),
            String::new(),
            "regular".to_string(),
            "ok".to_string(),
            "false".to_string(),
        ])?;
        let plannable = PlannableFile {
            file,
            hash_algorithm: Some(duplicate_set.hash_algorithm.clone()),
            full_hash_hex: Some(duplicate_set.full_hash_hex.clone()),
            hash_status: Some("full".to_string()),
        };
        sets.entry(set_id)
            .and_modify(|(_, members)| members.push(plannable.clone()))
            .or_insert_with(|| (duplicate_set, vec![plannable]));
    }

    println!("FOD indexer duplicate report");
    println!("confirmed duplicate sets: {}", sets.len());
    for (idx, (_set_id, (duplicate_set, mut members))) in sets.into_iter().enumerate() {
        if idx >= limit {
            println!("... truncated after {limit} sets");
            break;
        }
        members.sort_by(|left, right| {
            canonical_sort_key(&left.file).cmp(&canonical_sort_key(&right.file))
        });
        let canonical = members.first().expect("duplicate set must have members");
        println!(
            "set {}: size={} files={} hash={} total_bytes={}",
            duplicate_set.id_duplicate_set,
            duplicate_set.file_size,
            duplicate_set.file_count,
            duplicate_set.full_hash_hex,
            duplicate_set.total_bytes
        );
        println!(
            "  canonical: {}:{}",
            canonical.file.source_name, canonical.file.path
        );
        for member in members.iter().skip(1) {
            println!(
                "  reference: {}:{}",
                member.file.source_name, member.file.path
            );
        }
    }
    Ok(())
}

pub fn dry_run_import_plan(
    repo: &DbRepo,
    source_filter: Option<&str>,
) -> Result<ImportPlanSummary, String> {
    hash::rebuild_duplicate_sets(repo)?;
    let duplicate_sets = load_duplicate_sets(repo)?;
    let duplicate_set_map: HashMap<(String, String, u64), DuplicateSet> = duplicate_sets
        .iter()
        .cloned()
        .map(|duplicate_set| {
            (
                (
                    duplicate_set.hash_algorithm.clone(),
                    duplicate_set.full_hash_hex.clone(),
                    duplicate_set.file_size,
                ),
                duplicate_set,
            )
        })
        .collect();

    let files = load_plannable_files(repo, source_filter)?;
    let plan_id = insert_import_plan(repo, "dry_run_running", true, source_filter)?;
    let mut summary = ImportPlanSummary {
        source_filter: source_filter.map(|value| value.to_string()),
        ..ImportPlanSummary::default()
    };
    summary.scanned_files = files.len() as u64;
    summary.total_source_bytes = files.iter().map(|file| file.file.size).sum();

    let mut size_groups: BTreeMap<u64, Vec<PlannableFile>> = BTreeMap::new();
    for file in files {
        size_groups.entry(file.file.size).or_default().push(file);
    }

    for (_, mut group) in size_groups {
        if group.len() > 1 {
            summary.candidate_duplicate_groups =
                summary.candidate_duplicate_groups.saturating_add(1);
        }
        let mut duplicate_grouped: HashMap<(String, String, u64), Vec<PlannableFile>> =
            HashMap::new();
        let mut leftover = Vec::new();

        for file in group.drain(..) {
            if file.needs_revalidation() {
                leftover.push(file);
                continue;
            }
            match (
                file.hash_algorithm.clone(),
                file.full_hash_hex.clone(),
                file.hash_status.as_deref(),
            ) {
                (Some(algorithm), Some(full_hash_hex), Some("full")) => {
                    duplicate_grouped
                        .entry((algorithm, full_hash_hex, file.file.size))
                        .or_default()
                        .push(file);
                }
                _ => leftover.push(file),
            }
        }

        for (key, mut members) in duplicate_grouped {
            if members.len() > 1 {
                summary.confirmed_duplicate_groups =
                    summary.confirmed_duplicate_groups.saturating_add(1);
                let duplicate_set = duplicate_set_map
                    .get(&key)
                    .ok_or_else(|| "duplicate set missing after rebuild".to_string())?;
                members.sort_by(|left, right| {
                    canonical_sort_key(&left.file).cmp(&canonical_sort_key(&right.file))
                });
                let canonical = members.first().expect("duplicate set must have members");
                summary.unique_payload_count = summary.unique_payload_count.saturating_add(1);
                summary.estimated_import_bytes = summary
                    .estimated_import_bytes
                    .saturating_add(duplicate_set.file_size);
                insert_import_plan_entry(
                    repo,
                    plan_id,
                    &canonical.file,
                    Some(duplicate_set.id_duplicate_set),
                    "canonical",
                    Some(canonical.file.id_file),
                )?;
                for reference in members.iter().skip(1) {
                    insert_import_plan_entry(
                        repo,
                        plan_id,
                        &reference.file,
                        Some(duplicate_set.id_duplicate_set),
                        "reference",
                        Some(canonical.file.id_file),
                    )?;
                }
            } else {
                leftover.extend(members);
            }
        }

        for file in leftover {
            summary.unique_payload_count = summary.unique_payload_count.saturating_add(1);
            summary.estimated_import_bytes = summary
                .estimated_import_bytes
                .saturating_add(file.file.size);
            insert_import_plan_entry(
                repo,
                plan_id,
                &file.file,
                None,
                if file.needs_revalidation() {
                    "needs_revalidation"
                } else {
                    "unique"
                },
                Some(file.file.id_file),
            )?;
        }
    }

    summary.saved_bytes = summary
        .total_source_bytes
        .saturating_sub(summary.estimated_import_bytes);
    update_import_plan(repo, plan_id, "dry_run_completed", true, &summary)?;
    Ok(summary)
}
