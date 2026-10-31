use crate::db::sql_quote_literal;
use crate::model::{
    parse_positive_u64, parse_postgres_bool, require_exact_row_columns, require_single_exact_row,
    CleanupFailedSummary,
};
use fod_rust_hotpath::pg::{DbRepo, DbRepoTransaction};
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

// Blokada wiersza planu serializuje cleanup na wszystkich hostach przed odczytem stanu.
fn load_cleanup_plan(repo: &DbRepoTransaction<'_>, plan_id: u64) -> Result<CleanupPlan, String> {
    let rows = repo.query_rows_text(&format!(
        "
        SELECT status, COALESCE(source_filter, '')
        FROM index_import_plans
        WHERE id_import_plan = {plan_id}
        FOR UPDATE
        ",
    ))?;
    let row = match rows.as_slice() {
        [] => return Err(format!("unknown import plan: {plan_id}")),
        [_] => require_single_exact_row(&rows, 2, "cleanup import plan")?,
        _ => return Err(format!("cleanup plan lookup returned {} rows", rows.len())),
    };
    let status = row[0].trim().to_string();
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

fn find_root_name(repo: &DbRepoTransaction<'_>, plan_id: u64) -> Result<Option<String>, String> {
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
        1 => Ok(Some(
            require_single_exact_row(&rows, 1, "materialization root")?[0].clone(),
        )),
        _ => Err(format!(
            "found multiple materialization roots for plan {plan_id}"
        )),
    }
}

fn collect_directory_rows(
    repo: &DbRepoTransaction<'_>,
    root_name: &str,
) -> Result<Vec<(u64, u64)>, String> {
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
            require_exact_row_columns(row, 2, "directory subtree")?;
            Ok((
                parse_positive_u64(&row[0], "directory ID")?,
                parse_u64(&row[1], "directory depth")?,
            ))
        })
        .collect()
}

fn collect_file_rows(
    repo: &DbRepoTransaction<'_>,
    root_name: &str,
) -> Result<Vec<(u64, u64, u64, String)>, String> {
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
        SELECT f.id_file, f.data_object_id, f.id_directory, f.name
        FROM files f
        WHERE f.id_directory IN (SELECT id_directory FROM subtree)
        ORDER BY f.id_file
        ",
        sql_quote_literal(root_name),
    ))?;
    rows.iter()
        .map(|row| {
            require_exact_row_columns(row, 4, "file subtree")?;
            Ok((
                parse_positive_u64(&row[0], "file ID")?,
                parse_positive_u64(&row[1], "data-object ID")?,
                parse_positive_u64(&row[2], "file parent directory ID")?,
                row[3].clone(),
            ))
        })
        .collect()
}

/// Before deleting imported files, exclude active open handles and write owners on any
/// matching file or destination. The advisory key scheme/order must match
/// DbRepo::acquire_write_ownership and fenced persistence on every host.
///
/// This is a cooperative protocol guard, NOT a substitute for C10 role
/// separation and not yet a full namespace/open-handle cleanup protocol.
fn reject_cleanup_during_active_write_lease(
    repo: &DbRepoTransaction<'_>,
    file_rows: &[(u64, u64, u64, String)],
) -> Result<(), String> {
    if file_rows.is_empty() {
        return Ok(());
    }
    let mut resource_keys = Vec::with_capacity(file_rows.len().saturating_mul(2));
    for &(file_id, _, parent_id, ref name) in file_rows {
        resource_keys.push(format!("fod:open:file:{file_id}"));
        resource_keys.push(format!("fod:write:file:{file_id}"));
        resource_keys.push(format!(
            "fod:write:destination:{parent_id}:{}:{name}",
            name.len()
        ));
    }
    resource_keys.sort_unstable();
    resource_keys.dedup();
    for resource_key in resource_keys {
        let locked = repo.query_scalar_text(&format!(
            "SELECT pg_try_advisory_xact_lock(hashtextextended({}, 0))",
            DbRepo::quote_literal(&resource_key)
        ))?;
        if !parse_postgres_bool(&locked, "index cleanup advisory-lock result")? {
            return Err(format!(
                "FOD_INDEXER_CLEANUP_ACTIVE_WRITER advisory_busy resource={resource_key}"
            ));
        }
    }

    let ids = file_rows
        .iter()
        .map(|row| row.0.to_string())
        .collect::<Vec<_>>()
        .join(",");

    // Nie wolno zwolnic danych pliku otwartego na innym hoscie.
    // Odmowa jest bezpieczniejsza od fizycznego usuniecia aktywnego obiektu.
    let open_handles = repo.query_scalar_text(&format!(
        "SELECT EXISTS (
            SELECT 1
            FROM file_open_leases l
            JOIN client_sessions s ON s.session_id = l.session_id
            WHERE l.file_id IN ({ids})
              AND l.lease_expires_at > clock_timestamp()
              AND s.lease_expires_at > clock_timestamp()
        )::text"
    ))?;
    if parse_postgres_bool(&open_handles, "index cleanup active open-handle state")? {
        return Err("FOD_INDEXER_CLEANUP_ACTIVE_OPEN_HANDLE active_open_lease".to_string());
    }

    let active = repo.query_scalar_text(&format!(
        "SELECT EXISTS (
            SELECT 1 FROM file_write_leases
            WHERE file_id IN ({ids})
              AND lease_expires_at > clock_timestamp()
            UNION ALL
            SELECT 1
            FROM destination_write_leases dwl
            JOIN files f ON dwl.parent_key = f.id_directory AND dwl.name = f.name
            WHERE f.id_file IN ({ids})
              AND dwl.lease_expires_at > clock_timestamp()
        )::text"
    ))?;
    if parse_postgres_bool(&active, "index cleanup active write-lease state")? {
        return Err("FOD_INDEXER_CLEANUP_ACTIVE_WRITER active_write_lease".to_string());
    }
    Ok(())
}

fn reject_cleanup_with_external_hardlink(
    repo: &DbRepoTransaction<'_>,
    root_name: &str,
) -> Result<(), String> {
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
        SELECT h.id_hardlink, h.id_file
        FROM files f
        JOIN subtree file_parent ON file_parent.id_directory = f.id_directory
        JOIN hardlinks h ON h.id_file = f.id_file
        WHERE h.id_directory IS NULL
           OR NOT EXISTS (
                SELECT 1 FROM subtree alias_parent
                WHERE alias_parent.id_directory = h.id_directory
           )
        ORDER BY h.id_hardlink
        LIMIT 1
        ",
        sql_quote_literal(root_name),
    ))?;
    match rows.as_slice() {
        [] => Ok(()),
        [_] => {
            let row = require_single_exact_row(&rows, 2, "external import hardlink")?;
            let hardlink_id = parse_positive_u64(&row[0], "hardlink ID")?;
            let file_id = parse_positive_u64(&row[1], "hardlink file ID")?;
            Err(format!(
                "FOD_INDEXER_CLEANUP_EXTERNAL_HARDLINK hardlink_id={hardlink_id} file_id={file_id}"
            ))
        }
        _ => Err("external hardlink lookup returned too many rows".to_string()),
    }
}

fn collect_data_object_file_ids(
    repo: &DbRepoTransaction<'_>,
    data_object_id: u64,
) -> Result<Vec<u64>, String> {
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
            require_exact_row_columns(row, 1, "data-object file reference")?;
            parse_positive_u64(&row[0], "file ID")
        })
        .collect()
}

fn delete_data_object_rows(
    repo: &DbRepoTransaction<'_>,
    data_object_id: u64,
    expected_reference_count: u64,
) -> Result<(), String> {
    let rows = repo.query_rows_text(&format!(
        "DELETE FROM data_objects
         WHERE id_data_object = {data_object_id}
           AND reference_count = {expected_reference_count}
         RETURNING id_data_object"
    ))?;
    match rows.as_slice() {
        [] => Err(format!(
            "FOD_INDEXER_CLEANUP_DATA_OBJECT_CHANGED data_object_id={data_object_id} expected_reference_count={expected_reference_count}"
        )),
        [_] => {
            let row = require_single_exact_row(&rows, 1, "deleted cleanup data object")?;
            let deleted_id = parse_positive_u64(&row[0], "deleted data-object ID")?;
            if deleted_id != data_object_id {
                return Err(format!(
                    "cleanup deleted unexpected data object: expected={data_object_id} actual={deleted_id}"
                ));
            }
            Ok(())
        }
        _ => Err(format!(
            "cleanup deleted multiple rows for data_object_id={data_object_id}"
        )),
    }
}

fn update_shared_data_object_reference_count(
    repo: &DbRepoTransaction<'_>,
    data_object_id: u64,
    removed_reference_count: u64,
) -> Result<(), String> {
    let rows = repo.query_rows_text(&format!(
        "
        UPDATE data_objects
        SET reference_count = reference_count - {removed_reference_count},
            modification_date = NOW()
        WHERE id_data_object = {data_object_id}
          AND reference_count >= {removed_reference_count}
        RETURNING reference_count
        "
    ))?;
    match rows.as_slice() {
        [] => Err(format!(
            "FOD_INDEXER_CLEANUP_REFCOUNT_UNDERFLOW data_object_id={data_object_id} removed_reference_count={removed_reference_count}"
        )),
        [_] => {
            let row = require_single_exact_row(&rows, 1, "updated shared data-object count")?;
            row[0]
                .trim()
                .parse::<u64>()
                .map(|_| ())
                .map_err(|err| format!("invalid updated data-object reference count: {err}"))
        }
        _ => Err(format!(
            "cleanup updated multiple rows for data_object_id={data_object_id}"
        )),
    }
}

pub fn cleanup_failed_materialization(
    repo: &DbRepo,
    plan_id: u64,
) -> Result<CleanupFailedSummary, String> {
    repo.with_transaction(|tx| {
        let plan = load_cleanup_plan(tx, plan_id)?;
        match plan.status.as_str() {
            "materialize_failed" | "materialize_cleaned" => {}
            "materialize_running" => {
                return Err(format!(
                    "FOD_INDEXER_CLEANUP_ACTIVE_MATERIALIZER plan {plan_id} is still running; cleanup refused"
                ));
            }
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

        let root_name = find_root_name(tx, plan_id)?;
        let import_root = root_name
            .as_ref()
            .map(|name| format!("/{}", name))
            .unwrap_or_else(|| format!("<missing root for plan {plan_id}>"));

        let mut removed_files = 0u64;
        let mut removed_directories = 0u64;
        let mut exclusive_data_objects_removed = 0u64;
        let mut shared_data_objects_preserved = 0u64;

        if let Some(root_name) = root_name.as_deref() {
            let directory_rows = collect_directory_rows(tx, root_name)?;
            let file_rows = collect_file_rows(tx, root_name)?;
            reject_cleanup_during_active_write_lease(tx, &file_rows)?;
            // Hardlink creation takes the same per-file lifecycle advisory lock.
            // Once held, no new alias can race this check and the following DELETE.
            reject_cleanup_with_external_hardlink(tx, root_name)?;
            let subtree_file_ids: BTreeSet<u64> =
                file_rows.iter().map(|row| row.0).collect();
            let mut file_ids_by_data_object: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
            let mut exclusive_data_objects = Vec::new();
            let mut shared_data_object_decrements = Vec::new();

            for (file_id, data_object_id, _, _) in &file_rows {
                file_ids_by_data_object
                    .entry(*data_object_id)
                    .or_default()
                    .push(*file_id);
            }

            for (data_object_id, imported_file_ids) in file_ids_by_data_object {
                let removed_reference_count = imported_file_ids.len() as u64;
                let all_file_ids = collect_data_object_file_ids(tx, data_object_id)?;
                let outside_file_ids: Vec<u64> = all_file_ids
                    .into_iter()
                    .filter(|file_id| !subtree_file_ids.contains(file_id))
                    .collect();

                if outside_file_ids.is_empty() {
                    exclusive_data_objects.push((data_object_id, removed_reference_count));
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
                shared_data_object_decrements.push((data_object_id, removed_reference_count));
                shared_data_objects_preserved = shared_data_objects_preserved.saturating_add(1);
            }

            for (file_id, _, _, _) in &file_rows {
                tx.exec(&format!("DELETE FROM files WHERE id_file = {file_id}"))?;
            }

            // Match normal unlink/write ordering: mutate file references before
            // locking their data-object counters. PostgreSQL applies each
            // arithmetic UPDATE atomically against the latest committed value.
            for (data_object_id, removed_reference_count) in shared_data_object_decrements {
                update_shared_data_object_reference_count(
                    tx,
                    data_object_id,
                    removed_reference_count,
                )?;
            }

            for (data_object_id, expected_reference_count) in exclusive_data_objects {
                delete_data_object_rows(tx, data_object_id, expected_reference_count)?;
            }

            for (directory_id, _) in &directory_rows {
                tx.exec(&format!(
                    "DELETE FROM directories WHERE id_directory = {directory_id}"
                ))?;
            }

            removed_files = file_rows.len() as u64;
            removed_directories = directory_rows.len() as u64;
        }

        // Punkt awarii TX03D jest dostepny tylko w debug i tylko dla bazy testowej.
        // Po wykonaniu DELETE, ale przed COMMIT, test moze zabic proces indexera.
        #[cfg(debug_assertions)]
        if std::env::var("FOD_TX03D_AFTER_DELETE_PAUSE")
            .ok()
            .as_deref()
            == Some("1")
        {
            let dbname = tx.query_scalar_text("SELECT current_database()")?;
            if !dbname.ends_with("_test") {
                return Err("TX03D pause requires isolated _test database".to_string());
            }
            eprintln!("TX03D_AFTER_DELETE_READY plan_id={plan_id}");
            std::thread::sleep(std::time::Duration::from_secs(30));
        }

        tx.exec(&format!(
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
    })
}
