use crate::db::{ensure_indexer_request_token_schema, sql_quote_literal};
use crate::hash;
use crate::model::{DuplicateSet, ImportPlanSummary, IndexedFile};
use crate::output::{
    DuplicateReportSnapshot, DuplicateSetMemberView, DuplicateSetSnapshot, ImportPlanEntryView,
    ImportPlanSnapshot,
};
use crate::source;
use fod_rust_hotpath::pg::{DbRepo, IndexImportPlanEntryStageRow};
use fod_rust_runtime::request_token;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

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
        WHERE f.scan_status = 'ok' AND f.file_kind = 'regular' AND f.size > 0
        {source_clause}
        ORDER BY f.id_index_source, length(f.path), f.path
        ",
    ))?;

    Ok(rows
        .iter()
        .map(|row| PlannableFile::from_row(row))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|file| !source::is_ignored_indexed_file(&file.file))
        .collect::<Vec<_>>())
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
    let request_token = request_token("plan");
    let sql = format!(
        "
        INSERT INTO index_import_plans (
            created_at,
            updated_at,
            status,
            request_token,
            dry_run,
            source_filter
        )
        VALUES (
            NOW(),
            NOW(),
            {status},
            {request_token},
            {dry_run},
            {source_filter}
        )
        ON CONFLICT (request_token) DO UPDATE SET
            status = EXCLUDED.status,
            dry_run = EXCLUDED.dry_run,
            source_filter = EXCLUDED.source_filter,
            updated_at = NOW()
        RETURNING id_import_plan
        ",
        status = sql_quote_literal(status),
        request_token = sql_quote_literal(&request_token),
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

pub fn report_duplicate_sets(repo: &DbRepo, limit: usize) -> Result<(), String> {
    let snapshot = load_duplicate_report_snapshot(repo, limit)?;
    println!("{}", snapshot.human_readable());
    Ok(())
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "t" | "true" | "1" | "on"
    )
}

fn duplicate_set_member_from_row(row: &[String]) -> Result<DuplicateSetMemberView, String> {
    if row.len() < 15 {
        return Err("duplicate set member row is too short".to_string());
    }
    let file = IndexedFile::from_row(&row[0..12])?;
    let source_name = file.source_name.clone();
    let source_path = file.source_path();
    let source_kind = row[12].clone();
    let hash_algorithm = row[13].clone();
    let full_hash_hex = row[14].clone();
    Ok(DuplicateSetMemberView {
        file_id: file.id_file,
        source_id: file.source_id,
        source_name,
        source_kind,
        source_root_path: file.root_path.display().to_string(),
        logical_path: file.path.clone(),
        source_path,
        size: file.size,
        hash_algorithm,
        full_hash_hex,
        hash_status: "full".to_string(),
        is_canonical: false,
    })
}

pub(crate) fn load_duplicate_set_snapshot(
    repo: &DbRepo,
    duplicate_set_id: u64,
) -> Result<DuplicateSetSnapshot, String> {
    let rows = repo.query_rows_text(&format!(
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
            f.path,
            f.size,
            COALESCE(f.mtime_ns::text, ''),
            COALESCE(f.inode::text, ''),
            COALESCE(f.device::text, ''),
            f.file_kind,
            f.scan_status,
            f.source_changed::text,
            s.kind,
            h.hash_algorithm,
            COALESCE(encode(h.full_hash, 'hex'), '')
        FROM index_duplicate_sets ds
        JOIN index_file_hashes h
            ON h.hash_algorithm = ds.hash_algorithm
           AND h.full_hash = ds.full_hash
           AND h.observed_size = ds.file_size
        JOIN index_files f ON f.id_file = h.id_file
        JOIN index_sources s ON s.id_index_source = f.id_index_source
        WHERE ds.id_duplicate_set = {duplicate_set_id}
        ORDER BY f.id_index_source, length(f.path), f.path
        ",
    ))?;

    let mut duplicate_set: Option<DuplicateSet> = None;
    let mut members = Vec::new();
    for row in rows {
        if row.len() < 21 {
            continue;
        }
        if source::is_ignored_index_path(Path::new(&row[9]), &row[10]) {
            continue;
        }
        let current_set = DuplicateSet::from_row(&row[0..6])?;
        if current_set.file_size == 0 {
            return Err(format!(
                "duplicate set {duplicate_set_id} has zero size and is skipped by the report output"
            ));
        }
        duplicate_set.get_or_insert(current_set);
        let mut member = duplicate_set_member_from_row(&row[6..21])?;
        member.is_canonical = members.is_empty();
        members.push(member);
    }

    let duplicate_set =
        duplicate_set.ok_or_else(|| format!("unknown duplicate set: {duplicate_set_id}"))?;
    members.sort_by(|left, right| {
        let left_file = IndexedFile {
            id_file: left.file_id,
            source_id: left.source_id,
            source_name: left.source_name.clone(),
            root_path: std::path::PathBuf::from(&left.source_root_path),
            path: left.logical_path.clone(),
            size: left.size,
            mtime_ns: None,
            inode: None,
            device: None,
            file_kind: "regular".to_string(),
            scan_status: "ok".to_string(),
            source_changed: false,
        };
        let right_file = IndexedFile {
            id_file: right.file_id,
            source_id: right.source_id,
            source_name: right.source_name.clone(),
            root_path: std::path::PathBuf::from(&right.source_root_path),
            path: right.logical_path.clone(),
            size: right.size,
            mtime_ns: None,
            inode: None,
            device: None,
            file_kind: "regular".to_string(),
            scan_status: "ok".to_string(),
            source_changed: false,
        };
        canonical_sort_key(&left_file).cmp(&canonical_sort_key(&right_file))
    });
    for (idx, member) in members.iter_mut().enumerate() {
        member.is_canonical = idx == 0;
    }
    if members.is_empty() {
        return Err(format!(
            "duplicate set {duplicate_set_id} has no visible members after applying the current filters"
        ));
    }
    Ok(DuplicateSetSnapshot {
        duplicate_set,
        members,
    })
}

pub(crate) fn load_duplicate_report_snapshot(
    repo: &DbRepo,
    limit: usize,
) -> Result<DuplicateReportSnapshot, String> {
    hash::rebuild_duplicate_sets(repo)?;
    let rows = repo.query_rows_text(
        "
        SELECT
            id_duplicate_set,
            file_size
        FROM index_duplicate_sets
        ORDER BY id_duplicate_set
        ",
    )?;

    let mut duplicate_sets = Vec::new();
    for row in rows {
        if row.len() < 2 {
            continue;
        }
        let file_size = row[1]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid duplicate set file size: {err}"))?;
        if file_size == 0 {
            continue;
        }
        let set_id = row
            .first()
            .ok_or_else(|| "duplicate set row is malformed".to_string())?
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid duplicate set id: {err}"))?;
        let snapshot = match load_duplicate_set_snapshot(repo, set_id) {
            Ok(snapshot) => snapshot,
            Err(err) if err.contains("no visible members") => continue,
            Err(err) => return Err(err),
        };
        duplicate_sets.push(snapshot);
    }

    let confirmed_duplicate_sets = duplicate_sets.len() as u64;
    let truncated = duplicate_sets.len() > limit;
    let duplicate_sets = duplicate_sets.into_iter().take(limit).collect::<Vec<_>>();
    Ok(DuplicateReportSnapshot {
        limit: Some(limit),
        confirmed_duplicate_sets,
        truncated,
        duplicate_sets,
    })
}

pub(crate) fn load_import_plan_snapshot(
    repo: &DbRepo,
    plan_id: u64,
) -> Result<ImportPlanSnapshot, String> {
    let plan_rows = repo.query_rows_text(&format!(
        "
        SELECT
            id_import_plan,
            created_at::text,
            updated_at::text,
            status,
            request_token,
            dry_run::text,
            COALESCE(source_filter, ''),
            scanned_file_count,
            candidate_group_count,
            confirmed_group_count,
            unique_payload_count,
            total_source_bytes,
            estimated_import_bytes,
            saved_bytes
        FROM index_import_plans
        WHERE id_import_plan = {plan_id}
        ",
    ))?;
    let plan_row = plan_rows
        .first()
        .ok_or_else(|| format!("unknown import plan: {plan_id}"))?;
    if plan_row.len() < 14 {
        return Err("import plan row is too short".to_string());
    }
    let summary = ImportPlanSummary {
        plan_id: Some(plan_id),
        source_filter: if plan_row[6].trim().is_empty() {
            None
        } else {
            Some(plan_row[6].clone())
        },
        scanned_files: plan_row[7]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid scanned file count: {err}"))?,
        candidate_duplicate_groups: plan_row[8]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid candidate group count: {err}"))?,
        confirmed_duplicate_groups: plan_row[9]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid confirmed group count: {err}"))?,
        unique_payload_count: plan_row[10]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid unique payload count: {err}"))?,
        total_source_bytes: plan_row[11]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid total source bytes: {err}"))?,
        estimated_import_bytes: plan_row[12]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid estimated import bytes: {err}"))?,
        saved_bytes: plan_row[13]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid saved bytes: {err}"))?,
    };

    let entry_rows = repo.query_rows_text(&format!(
        "
        SELECT
            e.id_import_plan_entry,
            e.id_import_plan,
            e.id_file,
            f.id_index_source,
            s.name,
            s.kind,
            s.root_path,
            e.id_duplicate_set,
            e.action,
            e.canonical_file_id,
            e.logical_path,
            e.source_path,
            e.size,
            COALESCE(e.mtime_ns::text, ''),
            e.source_changed::text
        FROM index_import_plan_entries e
        JOIN index_files f ON f.id_file = e.id_file
        JOIN index_sources s ON s.id_index_source = f.id_index_source
        WHERE e.id_import_plan = {plan_id}
        ORDER BY e.id_import_plan_entry
        ",
    ))?;
    let mut entries = Vec::new();
    for row in entry_rows {
        if row.len() < 15 {
            continue;
        }
        entries.push(ImportPlanEntryView {
            id_import_plan_entry: row[0]
                .trim()
                .parse::<u64>()
                .map_err(|err| format!("invalid import plan entry id: {err}"))?,
            id_import_plan: row[1]
                .trim()
                .parse::<u64>()
                .map_err(|err| format!("invalid import plan id: {err}"))?,
            id_file: row[2]
                .trim()
                .parse::<u64>()
                .map_err(|err| format!("invalid file id: {err}"))?,
            source_id: row[3]
                .trim()
                .parse::<u64>()
                .map_err(|err| format!("invalid source id: {err}"))?,
            source_name: row[4].clone(),
            source_kind: row[5].clone(),
            source_root_path: row[6].clone(),
            id_duplicate_set: if row[7].trim().is_empty() {
                None
            } else {
                Some(
                    row[7]
                        .trim()
                        .parse::<u64>()
                        .map_err(|err| format!("invalid duplicate set id: {err}"))?,
                )
            },
            action: row[8].clone(),
            canonical_file_id: if row[9].trim().is_empty() {
                None
            } else {
                Some(
                    row[9]
                        .trim()
                        .parse::<u64>()
                        .map_err(|err| format!("invalid canonical file id: {err}"))?,
                )
            },
            logical_path: row[10].clone(),
            source_path: row[11].clone(),
            size: row[12]
                .trim()
                .parse::<u64>()
                .map_err(|err| format!("invalid plan entry size: {err}"))?,
            mtime_ns: if row[13].trim().is_empty() {
                None
            } else {
                Some(
                    row[13]
                        .trim()
                        .parse::<i64>()
                        .map_err(|err| format!("invalid plan entry mtime: {err}"))?,
                )
            },
            source_changed: parse_bool(&row[14]),
        });
    }

    Ok(ImportPlanSnapshot {
        summary,
        status: plan_row[3].clone(),
        request_token: plan_row[4].clone(),
        dry_run: parse_bool(&plan_row[5]),
        created_at: plan_row[1].clone(),
        updated_at: plan_row[2].clone(),
        entries,
    })
}

pub fn dry_run_import_plan(
    repo: &DbRepo,
    source_filter: Option<&str>,
) -> Result<ImportPlanSummary, String> {
    ensure_indexer_request_token_schema(repo, "fod-indexer plan-import")?;
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
        plan_id: Some(plan_id),
        source_filter: source_filter.map(|value| value.to_string()),
        ..ImportPlanSummary::default()
    };
    summary.scanned_files = files.len() as u64;
    summary.total_source_bytes = files.iter().map(|file| file.file.size).sum();
    let mut staged_rows: Vec<IndexImportPlanEntryStageRow> = Vec::new();

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
                staged_rows.push(IndexImportPlanEntryStageRow {
                    id_import_plan: plan_id,
                    id_file: canonical.file.id_file,
                    id_duplicate_set: Some(duplicate_set.id_duplicate_set),
                    action: "canonical".to_string(),
                    canonical_file_id: Some(canonical.file.id_file),
                    logical_path: canonical.file.path.clone(),
                    source_path: canonical.file.source_path(),
                    size: canonical.file.size,
                    mtime_ns: canonical.file.mtime_ns,
                    source_changed: canonical.file.source_changed,
                });
                for reference in members.iter().skip(1) {
                    staged_rows.push(IndexImportPlanEntryStageRow {
                        id_import_plan: plan_id,
                        id_file: reference.file.id_file,
                        id_duplicate_set: Some(duplicate_set.id_duplicate_set),
                        action: "reference".to_string(),
                        canonical_file_id: Some(canonical.file.id_file),
                        logical_path: reference.file.path.clone(),
                        source_path: reference.file.source_path(),
                        size: reference.file.size,
                        mtime_ns: reference.file.mtime_ns,
                        source_changed: reference.file.source_changed,
                    });
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
            staged_rows.push(IndexImportPlanEntryStageRow {
                id_import_plan: plan_id,
                id_file: file.file.id_file,
                id_duplicate_set: None,
                action: if file.needs_revalidation() {
                    "needs_revalidation".to_string()
                } else {
                    "unique".to_string()
                },
                canonical_file_id: Some(file.file.id_file),
                logical_path: file.file.path.clone(),
                source_path: file.file.source_path(),
                size: file.file.size,
                mtime_ns: file.file.mtime_ns,
                source_changed: file.file.source_changed,
            });
        }
    }

    summary.saved_bytes = summary
        .total_source_bytes
        .saturating_sub(summary.estimated_import_bytes);
    repo.upsert_index_import_plan_entries_staged(&staged_rows)?;
    update_import_plan(repo, plan_id, "dry_run_completed", true, &summary)?;
    Ok(summary)
}
