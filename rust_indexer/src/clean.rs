use crate::model::{CleanSourceSummary, IndexSource, IndexedFile};
use crate::{hash, scan, source};
use fod_rust_hotpath::pg::DbRepo;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

struct SourceTreeState {
    visible_paths: BTreeSet<String>,
    blocked_prefixes: Vec<PathBuf>,
    root_missing: bool,
}

fn current_source_tree(root_path: &Path) -> Result<SourceTreeState, String> {
    let metadata = match fs::metadata(root_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SourceTreeState {
                visible_paths: BTreeSet::new(),
                blocked_prefixes: Vec::new(),
                root_missing: true,
            });
        }
        Err(err) => {
            return Err(format!(
                "unable to read source root metadata for {}: {err}",
                root_path.display()
            ));
        }
    };

    if !metadata.is_dir() {
        return Err(format!(
            "source path {} is not a directory",
            root_path.display()
        ));
    }

    let mut visible_paths = BTreeSet::new();
    let mut blocked_prefixes = Vec::new();
    for item in WalkDir::new(root_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !source::is_ignored_source_path(root_path, entry.path()))
    {
        match item {
            Ok(entry) => {
                if entry.depth() == 0 || entry.file_type().is_dir() {
                    continue;
                }
                visible_paths.insert(scan::relative_source_path(root_path, entry.path()));
            }
            Err(err) => {
                if let Some(path) = err.path() {
                    blocked_prefixes.push(path.to_path_buf());
                } else {
                    return Err(format!(
                        "unable to walk source tree {}: {err}",
                        root_path.display()
                    ));
                }
            }
        }
    }
    blocked_prefixes.sort();
    blocked_prefixes.dedup();

    Ok(SourceTreeState {
        visible_paths,
        blocked_prefixes,
        root_missing: false,
    })
}

fn is_under_blocked_prefix(path: &Path, blocked_prefixes: &[PathBuf]) -> bool {
    blocked_prefixes
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

fn collect_stale_files(
    source: &IndexSource,
    indexed_files: &[IndexedFile],
    tree_state: &SourceTreeState,
) -> (Vec<u64>, u64, u64) {
    let mut stale_file_ids = Vec::new();
    let mut present_files = 0u64;
    let mut skipped_files = 0u64;

    for file in indexed_files {
        if tree_state.visible_paths.contains(&file.path) {
            present_files = present_files.saturating_add(1);
            continue;
        }

        let absolute_path = source.root_path.join(&file.path);
        if !tree_state.root_missing
            && is_under_blocked_prefix(&absolute_path, &tree_state.blocked_prefixes)
        {
            skipped_files = skipped_files.saturating_add(1);
            continue;
        }

        stale_file_ids.push(file.id_file);
    }

    (stale_file_ids, present_files, skipped_files)
}

fn ids_clause(ids: &[u64]) -> String {
    ids.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn count_plan_entries_to_delete(repo: &DbRepo, stale_file_ids: &[u64]) -> Result<u64, String> {
    let mut total = 0u64;
    for chunk in stale_file_ids.chunks(512) {
        let ids = ids_clause(chunk);
        let rows = repo.query_rows_text(&format!(
            "
            SELECT COUNT(*)
            FROM index_import_plan_entries
            WHERE id_file IN ({ids})
               OR canonical_file_id IN ({ids})
            "
        ))?;
        let row = rows
            .first()
            .ok_or_else(|| "cleanup plan entry count did not return a row".to_string())?;
        let count = row
            .first()
            .ok_or_else(|| "cleanup plan entry count row is malformed".to_string())?
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid cleanup plan entry count: {err}"))?;
        total = total.saturating_add(count);
    }
    Ok(total)
}

fn delete_stale_rows(repo: &DbRepo, stale_file_ids: &[u64]) -> Result<(), String> {
    for chunk in stale_file_ids.chunks(512) {
        let ids = ids_clause(chunk);
        repo.exec(&format!(
            "
            DELETE FROM index_import_plan_entries
            WHERE id_file IN ({ids})
               OR canonical_file_id IN ({ids})
            "
        ))?;
    }

    for chunk in stale_file_ids.chunks(512) {
        let ids = ids_clause(chunk);
        repo.exec(&format!(
            "
            DELETE FROM index_files
            WHERE id_file IN ({ids})
            "
        ))?;
    }

    Ok(())
}

pub fn clean_source(
    repo: &DbRepo,
    source_name: &str,
    dry_run: bool,
) -> Result<CleanSourceSummary, String> {
    let source = scan::load_source(repo, source_name)?;

    let indexed_files = scan::load_indexed_files(repo, Some(source.name.as_str()))?;
    let tree_state = current_source_tree(&source.root_path)?;
    let (stale_file_ids, present_files, skipped_files) =
        collect_stale_files(&source, &indexed_files, &tree_state);
    let plan_entries_removed = if stale_file_ids.is_empty() {
        0
    } else {
        count_plan_entries_to_delete(repo, &stale_file_ids)?
    };

    let mut summary = CleanSourceSummary {
        source_name: source.name.clone(),
        source_path: source.root_path.display().to_string(),
        dry_run,
        source_root_missing: tree_state.root_missing,
        indexed_files: indexed_files.len() as u64,
        present_files,
        stale_files: stale_file_ids.len() as u64,
        skipped_files,
        plan_entries_removed,
        ..CleanSourceSummary::default()
    };

    if dry_run || stale_file_ids.is_empty() {
        return Ok(summary);
    }

    repo.exec("BEGIN")?;
    let result = (|| -> Result<CleanSourceSummary, String> {
        delete_stale_rows(repo, &stale_file_ids)?;
        let duplicate_sets_refreshed = hash::rebuild_duplicate_sets(repo)?;
        summary.duplicate_sets_refreshed = duplicate_sets_refreshed;
        repo.exec("COMMIT")?;
        Ok(summary)
    })();

    if result.is_err() {
        let _ = repo.exec("ROLLBACK");
    }
    result
}
