use crate::db::sql_quote_literal;
use crate::model::CleanupFailedSummary;
use fod_rust_hotpath::pg::DbRepo;
use std::collections::{BTreeMap, BTreeSet};

struct CleanupPlan {
    status: String,
    source_filter: String,
}

fn parse_u64(value: &str, label: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("invalid {label}: {err}"))
}

fn load_cleanup_plan(repo: &DbRepo, plan_id: u64) -> Result<CleanupPlan, String> {
    let rows = repo.query_rows_text(&format!(
        "
        SELECT status, COALESCE(source_filter, '')
        FROM index_import_plans
        WHERE id_import_plan = {plan_id}
        ",
    ))?;
    let row = rows
        .first()
        .ok_or_else(|| format!("unknown import plan: {plan_id}"))?;
    let status = row
        .first()
        .ok_or_else(|| "missing plan status".to_string())?
        .trim()
        .to_string();
    let source_filter = row
        .get(1)
        .ok_or_else(|| "missing plan source filter".to_string())?
        .trim()
        .to_string();
    Ok(CleanupPlan {
        status,
        source_filter,
    })
}

fn find_root_name(repo: &DbRepo, plan_id: u64) -> Result<Option<String>, String> {
    let rows = repo.query_rows_text(&format!(
        "
        SELECT name
        FROM directories
        WHERE id_parent IS NULL AND name LIKE {}
        ORDER BY name
        ",
        sql_quote_literal(&format!("index-source-%-import-{plan_id}")),
    ))?;
    match rows.len() {
        0 => Ok(None),
        1 => rows
            .first()
            .and_then(|row| row.first())
            .cloned()
            .map(Some)
            .ok_or_else(|| "root directory row is malformed".to_string()),
        _ => Err(format!(
            "found multiple materialization roots for plan {plan_id}"
        )),
    }
}

fn collect_directory_rows(repo: &DbRepo, root_name: &str) -> Result<Vec<(u64, u64)>, String> {
    let rows = repo.query_rows_text(&format!(
        "
        WITH RECURSIVE subtree AS (
            SELECT id_directory, 0 AS depth
            FROM directories
            WHERE id_parent IS NULL AND name = {}
            UNION ALL
            SELECT d.id_directory, s.depth + 1
            FROM directories d
            JOIN subtree s ON d.id_parent = s.id_directory
        )
        SELECT id_directory, depth
        FROM subtree
        ORDER BY depth DESC, id_directory DESC
        ",
        sql_quote_literal(root_name),
    ))?;
    rows.iter()
        .map(|row| {
            if row.len() < 2 {
                return Err("directory subtree row is too short".to_string());
            }
            Ok((
                parse_u64(&row[0], "directory id")?,
                parse_u64(&row[1], "directory depth")?,
            ))
        })
        .collect()
}

fn collect_file_rows(repo: &DbRepo, root_name: &str) -> Result<Vec<(u64, u64)>, String> {
    let rows = repo.query_rows_text(&format!(
        "
        WITH RECURSIVE subtree AS (
            SELECT id_directory
            FROM directories
            WHERE id_parent IS NULL AND name = {}
            UNION ALL
            SELECT d.id_directory
            FROM directories d
            JOIN subtree s ON d.id_parent = s.id_directory
        )
        SELECT f.id_file, f.data_object_id
        FROM files f
        WHERE f.id_directory IN (SELECT id_directory FROM subtree)
        ORDER BY f.id_file
        ",
        sql_quote_literal(root_name),
    ))?;
    rows.iter()
        .map(|row| {
            if row.len() < 2 {
                return Err("file subtree row is too short".to_string());
            }
            Ok((
                parse_u64(&row[0], "file id")?,
                parse_u64(&row[1], "data object id")?,
            ))
        })
        .collect()
}

fn collect_data_object_file_ids(repo: &DbRepo, data_object_id: u64) -> Result<Vec<u64>, String> {
    let rows = repo.query_rows_text(&format!(
        "
        SELECT id_file
        FROM files
        WHERE data_object_id = {data_object_id}
        ORDER BY id_file
        ",
    ))?;
    rows.iter()
        .map(|row| {
            row.first()
                .ok_or_else(|| "file row is malformed".to_string())
                .and_then(|value| parse_u64(value, "file id"))
        })
        .collect()
}

fn delete_data_object_rows(repo: &DbRepo, data_object_id: u64) -> Result<(), String> {
    repo.exec(&format!(
        "DELETE FROM data_blocks WHERE data_object_id = {data_object_id}"
    ))?;
    repo.exec(&format!(
        "DELETE FROM data_extents WHERE data_object_id = {data_object_id}"
    ))?;
    repo.exec(&format!(
        "DELETE FROM copy_block_crc WHERE data_object_id = {data_object_id}"
    ))?;
    repo.exec(&format!(
        "DELETE FROM data_objects WHERE id_data_object = {data_object_id}"
    ))?;
    Ok(())
}

fn reassign_shared_data_object(
    repo: &DbRepo,
    data_object_id: u64,
    survivor_file_id: u64,
    reference_count: u64,
) -> Result<(), String> {
    repo.exec(&format!(
        "UPDATE data_blocks SET id_file = {survivor_file_id} WHERE data_object_id = {data_object_id}"
    ))?;
    repo.exec(&format!(
        "UPDATE data_extents SET id_file = {survivor_file_id} WHERE data_object_id = {data_object_id}"
    ))?;
    repo.exec(&format!(
        "UPDATE copy_block_crc SET id_file = {survivor_file_id} WHERE data_object_id = {data_object_id}"
    ))?;
    repo.exec(&format!(
        "
        UPDATE data_objects
        SET reference_count = {reference_count},
            modification_date = NOW()
        WHERE id_data_object = {data_object_id}
        "
    ))?;
    Ok(())
}

pub fn cleanup_failed_materialization(
    repo: &DbRepo,
    plan_id: u64,
) -> Result<CleanupFailedSummary, String> {
    let plan = load_cleanup_plan(repo, plan_id)?;
    match plan.status.as_str() {
        "materialize_running" => {}
        "materialize_failed" | "materialize_cleaned" => {}
        "materialize_completed" => {
            return Err(format!(
                "plan {plan_id} is already completed and cannot be cleaned"
            ))
        }
        other => {
            return Err(format!(
                "plan {plan_id} has status {other} and is not a materialize failure"
            ))
        }
    }
    if plan.source_filter.trim().is_empty() {
        return Err(format!("plan {plan_id} does not have a source filter"));
    }

    let root_name = find_root_name(repo, plan_id)?;
    let import_root = root_name
        .as_ref()
        .map(|name| format!("/{}", name))
        .unwrap_or_else(|| format!("<missing root for plan {plan_id}>"));

    repo.exec("BEGIN")?;
    let result = (|| -> Result<CleanupFailedSummary, String> {
        let mut removed_files = 0u64;
        let mut removed_directories = 0u64;
        let mut exclusive_data_objects_removed = 0u64;
        let mut shared_data_objects_preserved = 0u64;

        if let Some(root_name) = root_name.as_deref() {
            let directory_rows = collect_directory_rows(repo, root_name)?;
            let file_rows = collect_file_rows(repo, root_name)?;
            let subtree_file_ids: BTreeSet<u64> =
                file_rows.iter().map(|(file_id, _)| *file_id).collect();
            let mut file_ids_by_data_object: BTreeMap<u64, Vec<u64>> = BTreeMap::new();

            for (file_id, data_object_id) in &file_rows {
                file_ids_by_data_object
                    .entry(*data_object_id)
                    .or_default()
                    .push(*file_id);
            }

            for (data_object_id, _) in file_ids_by_data_object {
                let all_file_ids = collect_data_object_file_ids(repo, data_object_id)?;
                let outside_file_ids: Vec<u64> = all_file_ids
                    .into_iter()
                    .filter(|file_id| !subtree_file_ids.contains(file_id))
                    .collect();

                if outside_file_ids.is_empty() {
                    delete_data_object_rows(repo, data_object_id)?;
                    exclusive_data_objects_removed =
                        exclusive_data_objects_removed.saturating_add(1);
                    continue;
                }

                let survivor_file_id = *outside_file_ids
                    .first()
                    .ok_or_else(|| "shared data object is missing a survivor file".to_string())?;
                eprintln!(
                    "skipping shared data object during failed import cleanup: data_object_id={} survivor_file_id={} outside_refs={}",
                    data_object_id,
                    survivor_file_id,
                    outside_file_ids.len()
                );
                reassign_shared_data_object(
                    repo,
                    data_object_id,
                    survivor_file_id,
                    outside_file_ids.len() as u64,
                )?;
                shared_data_objects_preserved = shared_data_objects_preserved.saturating_add(1);
            }

            for (file_id, _) in &file_rows {
                repo.exec(&format!("DELETE FROM files WHERE id_file = {file_id}"))?;
            }

            for (directory_id, _) in &directory_rows {
                repo.exec(&format!(
                    "DELETE FROM directories WHERE id_directory = {directory_id}"
                ))?;
            }

            removed_files = file_rows.len() as u64;
            removed_directories = directory_rows.len() as u64;
        }

        repo.exec(&format!(
            "
            UPDATE index_import_plans
            SET status = 'materialize_cleaned',
                updated_at = NOW()
            WHERE id_import_plan = {plan_id}
            "
        ))?;

        Ok(CleanupFailedSummary {
            plan_id,
            source_name: plan.source_filter,
            import_root,
            removed_files,
            removed_directories,
            exclusive_data_objects_removed,
            shared_data_objects_preserved,
            plan_status_before: plan.status,
            plan_status_after: "materialize_cleaned".to_string(),
        })
    })();

    match result {
        Ok(summary) => {
            repo.exec("COMMIT")?;
            Ok(summary)
        }
        Err(err) => {
            let _ = repo.exec("ROLLBACK");
            Err(err)
        }
    }
}
