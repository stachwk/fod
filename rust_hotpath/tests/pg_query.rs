// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_hotpath::pg::{DbRepo, PersistBlockRow, PersistExtentRow};
use fod_rust_runtime::{DataObjectSwapCleanup, RuntimeConfig};
use std::env;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn conninfo_from_env() -> String {
    let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
    let dbname = env::var("POSTGRES_DB").unwrap_or_else(|_| "foddbname".to_string());
    let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "foduser".to_string());
    let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "cichosza".to_string());
    format!(
        "host='{}' port='{}' dbname='{}' user='{}' password='{}'",
        host, port, dbname, user, password
    )
}

fn repo_with_runtime() -> Result<DbRepo, String> {
    repo_with_runtime_config(RuntimeConfig::from_env()?)
}

fn repo_with_swap_cleanup(cleanup: DataObjectSwapCleanup) -> Result<DbRepo, String> {
    let mut runtime = RuntimeConfig::from_env()?;
    runtime.data_object_swap_cleanup = cleanup;
    repo_with_runtime_config(runtime)
}

fn repo_with_runtime_config(mut runtime: RuntimeConfig) -> Result<DbRepo, String> {
    runtime.copy_dedupe_min_blocks = runtime.copy_dedupe_min_blocks.max(1);
    let repo = DbRepo::with_runtime(&conninfo_from_env(), &runtime)?;
    if repo.query_scalar_text(
        "SELECT EXISTS (SELECT 1 FROM information_schema.table_constraints WHERE table_name = 'copy_block_crc' AND constraint_name = 'copy_block_crc_pkey')",
    )? == "t"
    {
        repo.exec("ALTER TABLE copy_block_crc DROP CONSTRAINT copy_block_crc_pkey")?;
    }
    Ok(repo)
}

#[test]
fn append_only_extents_detach_shared_object_and_preserve_hardlink() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_swap_cleanup(DataObjectSwapCleanup::Immediate)?;
    let block_size = repo.startup_snapshot()?.block_size.unwrap_or(4096) as usize;
    let block_size_u64 = block_size as u64;

    let dirname = unique_name("rust_pg_append_shared_dir");
    let dir_id = repo.create_directory(
        None,
        &dirname,
        0o755,
        1000,
        1000,
        &unique_name("append_shared_dir_seed"),
    )?;
    let src_file_id = repo.create_file(
        Some(dir_id),
        &unique_name("rust_pg_append_shared_src"),
        0o644,
        1000,
        1000,
        &unique_name("append_shared_src_seed"),
    )?;
    let dst_file_id = repo.create_file(
        Some(dir_id),
        &unique_name("rust_pg_append_shared_dst"),
        0o644,
        1000,
        1000,
        &unique_name("append_shared_dst_seed"),
    )?;

    let old_payload = repeated_block(b'A', block_size);
    let old_rows = [PersistBlockRow {
        block_index: 0,
        data: &old_payload,
        used_len: block_size_u64,
    }];
    repo.persist_file_blocks(
        src_file_id,
        block_size_u64,
        block_size_u64,
        1,
        false,
        &old_rows,
    )?;
    if !repo.adopt_source_data_object(src_file_id, dst_file_id)? {
        return Err("expected destination to adopt source data object".to_string());
    }
    let shared_data_object_id = repo
        .file_data_object_id(src_file_id)?
        .ok_or_else(|| "missing shared data object".to_string())?;

    let hardlink_id = repo.create_hardlink(
        dst_file_id,
        Some(dir_id),
        &unique_name("rust_pg_append_shared_hardlink"),
        1000,
        1000,
    )?;
    if repo.get_hardlink_file_id(hardlink_id)? != Some(dst_file_id) {
        return Err("hardlink no longer resolves to the destination file".to_string());
    }

    let new_block0 = repeated_block(b'B', block_size);
    let new_block1 = repeated_block(b'C', block_size);
    let new_payload = [new_block0.clone(), new_block1.clone()].concat();
    let extent_rows = [PersistExtentRow {
        start_block: 0,
        block_count: 2,
        used_bytes: new_payload.len() as u64,
        payload: new_payload,
    }];
    let new_data_object_id = repo.persist_new_object_extents(
        dst_file_id,
        2 * block_size_u64,
        block_size_u64,
        2,
        &extent_rows,
        true,
    )?;

    if new_data_object_id == shared_data_object_id
        || repo.file_data_object_id(dst_file_id)? != Some(new_data_object_id)
        || repo.file_data_object_id(src_file_id)? != Some(shared_data_object_id)
    {
        return Err("append-only write did not detach the shared object".to_string());
    }
    assert_block_range_matches(
        &repo,
        src_file_id,
        block_size_u64,
        &[(0, old_payload.as_slice())],
    )?;
    assert_block_range_matches(
        &repo,
        dst_file_id,
        block_size_u64,
        &[(0, new_block0.as_slice()), (1, new_block1.as_slice())],
    )?;

    let shared_reference_count = repo.query_scalar_text(&format!(
        "SELECT reference_count FROM data_objects WHERE id_data_object = {shared_data_object_id}"
    ))?;
    if shared_reference_count.trim() != "1" {
        return Err(format!(
            "expected one remaining shared reference, got {shared_reference_count}"
        ));
    }
    let crc_rows = repo.query_scalar_text(&format!(
        "SELECT COUNT(*) FROM copy_block_crc WHERE data_object_id = {new_data_object_id}"
    ))?;
    if crc_rows.trim() != "2" {
        return Err(format!("expected two append-only CRC rows, got {crc_rows}"));
    }

    repo.delete_hardlink_entry(hardlink_id)?;
    repo.purge_primary_file(dst_file_id)?;
    repo.purge_primary_file(src_file_id)?;
    repo.delete_directory_entry(dir_id)?;
    Ok(())
}

#[test]
fn append_only_extents_follow_immediate_and_deferred_cleanup() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();

    for cleanup in [
        DataObjectSwapCleanup::Immediate,
        DataObjectSwapCleanup::Deferred,
    ] {
        let repo = repo_with_swap_cleanup(cleanup)?;
        let block_size = repo.startup_snapshot()?.block_size.unwrap_or(4096) as usize;
        let block_size_u64 = block_size as u64;
        let dirname = unique_name("rust_pg_append_cleanup_dir");
        let dir_id = repo.create_directory(
            None,
            &dirname,
            0o755,
            1000,
            1000,
            &unique_name("append_cleanup_dir_seed"),
        )?;
        let file_id = repo.create_file(
            Some(dir_id),
            &unique_name("rust_pg_append_cleanup_file"),
            0o644,
            1000,
            1000,
            &unique_name("append_cleanup_file_seed"),
        )?;

        let old_payload = repeated_block(b'D', block_size);
        let old_rows = [PersistBlockRow {
            block_index: 0,
            data: &old_payload,
            used_len: block_size_u64,
        }];
        repo.persist_file_blocks(file_id, block_size_u64, block_size_u64, 1, false, &old_rows)?;
        let old_data_object_id = repo
            .file_data_object_id(file_id)?
            .ok_or_else(|| "missing old data object".to_string())?;

        let new_payload = repeated_block(b'E', block_size);
        let extent_rows = [PersistExtentRow {
            start_block: 0,
            block_count: 1,
            used_bytes: block_size_u64,
            payload: new_payload.clone(),
        }];
        let new_data_object_id = repo.persist_new_object_extents(
            file_id,
            block_size_u64,
            block_size_u64,
            1,
            &extent_rows,
            false,
        )?;
        if new_data_object_id == old_data_object_id {
            return Err("append-only write reused the old data object".to_string());
        }

        let old_object = repo.query_scalar_text(&format!(
            "SELECT COUNT(*) FROM data_objects WHERE id_data_object = {old_data_object_id}"
        ))?;
        match cleanup {
            DataObjectSwapCleanup::Immediate if old_object.trim() != "0" => {
                return Err("immediate cleanup retained the old object".to_string());
            }
            DataObjectSwapCleanup::Deferred if old_object.trim() != "1" => {
                return Err("deferred cleanup removed the old object".to_string());
            }
            DataObjectSwapCleanup::Deferred => {
                let old_reference_count = repo.query_scalar_text(&format!(
                    "SELECT reference_count FROM data_objects WHERE id_data_object = {old_data_object_id}"
                ))?;
                if old_reference_count.trim() != "0" {
                    return Err("deferred old object is still referenced".to_string());
                }
            }
            _ => {}
        }
        assert_block_range_matches(
            &repo,
            file_id,
            block_size_u64,
            &[(0, new_payload.as_slice())],
        )?;

        repo.purge_primary_file(file_id)?;
        if cleanup == DataObjectSwapCleanup::Deferred {
            repo.exec(&format!(
                "DELETE FROM data_blocks WHERE data_object_id = {old_data_object_id}; DELETE FROM data_extents WHERE data_object_id = {old_data_object_id}; DELETE FROM copy_block_crc WHERE data_object_id = {old_data_object_id}; DELETE FROM data_objects WHERE id_data_object = {old_data_object_id}"
            ))?;
        }
        repo.delete_directory_entry(dir_id)?;
    }

    Ok(())
}

#[test]
fn source_adoption_preserves_other_empty_object_references() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let block_size = repo.startup_snapshot()?.block_size.unwrap_or(4096) as usize;
    let block_size_u64 = block_size as u64;
    let dir_id = repo.create_directory(
        None,
        &unique_name("rust_pg_adopt_empty_shared_dir"),
        0o755,
        1000,
        1000,
        &unique_name("adopt_empty_shared_dir_seed"),
    )?;
    let src_file_id = repo.create_file(
        Some(dir_id),
        &unique_name("rust_pg_adopt_empty_shared_src"),
        0o644,
        1000,
        1000,
        &unique_name("adopt_empty_shared_src_seed"),
    )?;
    let survivor_file_id = repo.create_file(
        Some(dir_id),
        &unique_name("rust_pg_adopt_empty_shared_survivor"),
        0o644,
        1000,
        1000,
        &unique_name("adopt_empty_shared_survivor_seed"),
    )?;
    let dst_file_id = repo.create_file(
        Some(dir_id),
        &unique_name("rust_pg_adopt_empty_shared_dst"),
        0o644,
        1000,
        1000,
        &unique_name("adopt_empty_shared_dst_seed"),
    )?;

    let src_payload = repeated_block(b'Z', block_size);
    let src_rows = [PersistBlockRow {
        block_index: 0,
        data: &src_payload,
        used_len: block_size_u64,
    }];
    repo.persist_file_blocks(
        src_file_id,
        block_size_u64,
        block_size_u64,
        1,
        false,
        &src_rows,
    )?;

    let shared_empty_object_id = repo
        .file_data_object_id(survivor_file_id)?
        .ok_or_else(|| "missing shared empty object".to_string())?;
    let replaced_dst_object_id = repo
        .file_data_object_id(dst_file_id)?
        .ok_or_else(|| "missing destination empty object".to_string())?;
    repo.exec(&format!(
        "UPDATE files SET data_object_id = {shared_empty_object_id} WHERE id_file = {dst_file_id}; \
         UPDATE data_objects SET reference_count = 2 WHERE id_data_object = {shared_empty_object_id}; \
         DELETE FROM data_objects WHERE id_data_object = {replaced_dst_object_id}"
    ))?;

    if !repo.adopt_source_data_object(src_file_id, dst_file_id)? {
        return Err("expected whole source object adoption".to_string());
    }
    if repo.file_data_object_id(survivor_file_id)? != Some(shared_empty_object_id) {
        return Err("adoption changed the surviving empty file object".to_string());
    }
    let empty_object_state = repo.query_scalar_text(&format!(
        "SELECT COUNT(*)::text || ':' || COALESCE(MAX(reference_count), 0)::text FROM data_objects WHERE id_data_object = {shared_empty_object_id}"
    ))?;
    if empty_object_state.trim() != "1:1" {
        return Err(format!(
            "expected surviving empty object with one reference, got {empty_object_state}"
        ));
    }

    repo.purge_primary_file(dst_file_id)?;
    repo.purge_primary_file(src_file_id)?;
    repo.purge_primary_file(survivor_file_id)?;
    repo.delete_directory_entry(dir_id)?;
    Ok(())
}

fn unique_name(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}_{}_{}", std::process::id(), nanos)
}

fn repeated_block(byte: u8, block_size: usize) -> Vec<u8> {
    vec![byte; block_size]
}

fn table_row_count_for_file(repo: &DbRepo, table: &str, file_id: u64) -> Result<u64, String> {
    let data_object_id = repo
        .file_data_object_id(file_id)?
        .ok_or_else(|| format!("missing data object id for file {file_id}"))?;
    let count = repo.query_scalar_text(&format!(
        "SELECT COUNT(*) FROM {table} WHERE data_object_id = {data_object_id}"
    ))?;
    count
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("parse {table} row count: {err}"))
}

fn assert_block_range_matches(
    repo: &DbRepo,
    file_id: u64,
    block_size: u64,
    expected: &[(u64, &[u8])],
) -> Result<(), String> {
    if expected.is_empty() {
        return Err("expected at least one block".to_string());
    }

    let first_block = expected.first().unwrap().0;
    let last_block = expected.last().unwrap().0;
    let actual = repo
        .fetch_block_range(file_id, first_block, last_block, block_size)
        .map_err(|err| format!("fetch_block_range: {err}"))?;

    if actual.len() != expected.len() {
        return Err(format!(
            "expected {} blocks, got {}",
            expected.len(),
            actual.len()
        ));
    }

    for ((actual_index, actual_bytes), (expected_index, expected_bytes)) in
        actual.iter().zip(expected.iter())
    {
        if actual_index != expected_index {
            return Err(format!(
                "expected block index {}, got {}",
                expected_index, actual_index
            ));
        }
        if actual_bytes.as_slice() != *expected_bytes {
            return Err(format!("payload mismatch for block {expected_index}"));
        }
    }

    Ok(())
}

#[test]
fn live_pg_query_helpers_work() {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime().expect("repo");

    let scalar = repo
        .query_scalar_text("SELECT 1")
        .expect("query_scalar_text");
    assert_eq!(scalar.trim(), "1");

    let snapshot = repo.startup_snapshot().expect("startup_snapshot");
    assert!(snapshot.block_size.unwrap_or(0) > 0);
    assert!(!snapshot.is_in_recovery);
    assert!(snapshot.schema_version.unwrap_or(0) > 0);
    assert!(snapshot.schema_is_initialized);

    assert!(repo.is_in_recovery().is_ok());
    assert!(repo.schema_is_initialized().unwrap());
    assert!(repo.schema_version().unwrap().is_some());
    assert!(repo.query_config_value("block_size").unwrap().is_some());

    let root = repo.resolve_path("/").expect("resolve root");
    assert_eq!(root.kind.as_deref(), Some("dir"));
    assert_eq!(root.parent_id, None);
    assert!(root.entry_id.is_some());

    let dirname = unique_name("rust_pg_dir");
    let dir_id = repo
        .create_directory(None, &dirname, 0o755, 1000, 1000, &unique_name("dir_seed"))
        .expect("create directory");
    assert_eq!(
        repo.get_dir_id(&format!("/{dirname}")).unwrap(),
        Some(dir_id)
    );
    assert_eq!(
        repo.resolve_path(&format!("/{dirname}"))
            .unwrap()
            .kind
            .as_deref(),
        Some("dir")
    );

    let symlink_name = unique_name("rust_pg_link");
    let symlink_id = repo
        .create_symlink(
            Some(dir_id),
            &symlink_name,
            "/target",
            1000,
            1000,
            &unique_name("symlink_seed"),
        )
        .expect("create symlink");
    assert_eq!(
        repo.get_symlink_id(&format!("/{dirname}/{symlink_name}"))
            .unwrap(),
        Some(symlink_id)
    );
    assert_eq!(
        repo.load_symlink_target(symlink_id).unwrap().as_deref(),
        Some("/target")
    );

    let file_name = unique_name("rust_pg_file");
    let file_id = repo
        .create_file(
            Some(dir_id),
            &file_name,
            0o644,
            1000,
            1000,
            &unique_name("file_seed"),
        )
        .expect("create file");
    assert_eq!(
        repo.get_file_id(&format!("/{dirname}/{file_name}"))
            .unwrap(),
        Some(file_id)
    );

    let hardlink_name = unique_name("rust_pg_hardlink");
    let hardlink_id = repo
        .create_hardlink(file_id, Some(dir_id), &hardlink_name, 1000, 1000)
        .expect("create hardlink");
    assert_eq!(
        repo.get_hardlink_id(&format!("/{dirname}/{hardlink_name}"))
            .unwrap(),
        Some(hardlink_id)
    );
    assert_eq!(
        repo.get_hardlink_file_id(hardlink_id).unwrap(),
        Some(file_id)
    );
    assert!(repo.count_file_links(file_id).unwrap() >= 2);

    assert!(repo.count_files().unwrap() > 0);
    assert!(repo.count_directories().unwrap() > 0);
    let _ = repo.total_data_size().unwrap();
}

#[test]
fn promote_hardlink_to_primary_preserves_the_remaining_path() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;

    let dirname = unique_name("rust_pg_promote_dir");
    let dir_id = repo
        .create_directory(None, &dirname, 0o755, 1000, 1000, &unique_name("dir_seed"))
        .map_err(|err| format!("create directory: {err}"))?;
    let file_name = unique_name("rust_pg_primary");
    let file_id = repo
        .create_file(
            Some(dir_id),
            &file_name,
            0o644,
            1000,
            1000,
            &unique_name("file_seed"),
        )
        .map_err(|err| format!("create file: {err}"))?;
    let hardlink_name = unique_name("rust_pg_hardlink");
    let hardlink_id = repo
        .create_hardlink(file_id, Some(dir_id), &hardlink_name, 1000, 1000)
        .map_err(|err| format!("create hardlink: {err}"))?;

    if repo
        .count_file_links(file_id)
        .map_err(|err| err.to_string())?
        != 2
    {
        return Err("expected file to have two links before promotion".to_string());
    }
    if !repo
        .promote_hardlink_to_primary(file_id)
        .map_err(|err| format!("promote hardlink: {err}"))?
    {
        return Err("expected hardlink promotion to succeed".to_string());
    }

    let promoted_path = format!("/{dirname}/{hardlink_name}");
    let original_path = format!("/{dirname}/{file_name}");

    if repo
        .get_file_id(&promoted_path)
        .map_err(|err| format!("get promoted file id: {err}"))?
        != Some(file_id)
    {
        return Err("promoted path should resolve to the original file id".to_string());
    }
    if repo
        .get_hardlink_id(&promoted_path)
        .map_err(|err| format!("get promoted hardlink id: {err}"))?
        .is_some()
    {
        return Err("promoted path should no longer be a hardlink".to_string());
    }
    if repo
        .get_file_id(&original_path)
        .map_err(|err| format!("get original file id: {err}"))?
        .is_some()
    {
        return Err("original primary path should be removed after promotion".to_string());
    }
    if repo
        .get_hardlink_file_id(hardlink_id)
        .map_err(|err| format!("get hardlink file id: {err}"))?
        .is_some()
    {
        return Err("promoted hardlink row should be deleted".to_string());
    }
    if repo
        .count_file_links(file_id)
        .map_err(|err| err.to_string())?
        != 1
    {
        return Err("expected file to have one link after promotion".to_string());
    }

    repo.purge_primary_file(file_id)
        .map_err(|err| format!("purge primary file: {err}"))?;
    repo.delete_directory_entry(dir_id)
        .map_err(|err| format!("delete directory: {err}"))?;

    Ok(())
}

#[test]
fn persist_file_blocks_updates_existing_block_data() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let snapshot = repo.startup_snapshot()?;
    let block_size = snapshot.block_size.unwrap_or(4096) as usize;

    let dirname = unique_name("rust_pg_block_update_dir");
    let dir_id = repo
        .create_directory(None, &dirname, 0o755, 1000, 1000, &unique_name("dir_seed"))
        .map_err(|err| format!("create directory: {err}"))?;
    let file_name = unique_name("rust_pg_block_update_file");
    let file_id = repo
        .create_file(
            Some(dir_id),
            &file_name,
            0o644,
            1000,
            1000,
            &unique_name("file_seed"),
        )
        .map_err(|err| format!("create file: {err}"))?;

    let block0 = vec![b'A'; block_size];
    let block1 = vec![b'B'; block_size];
    let block2 = vec![b'C'; block_size];
    let original = [block0.clone(), block1.clone(), block2.clone()].concat();
    let initial_rows = vec![
        PersistBlockRow {
            block_index: 0,
            data: &block0,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 1,
            data: &block1,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 2,
            data: &block2,
            used_len: block_size as u64,
        },
    ];
    repo.persist_file_blocks(
        file_id,
        original.len() as u64,
        block_size as u64,
        3,
        false,
        &initial_rows,
    )
    .map_err(|err| format!("persist initial blocks: {err}"))?;

    let mutated_block1 = vec![b'X'; block_size];
    let updated = [block0.clone(), mutated_block1.clone(), block2.clone()].concat();
    let updated_rows = vec![
        PersistBlockRow {
            block_index: 0,
            data: &block0,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 1,
            data: &mutated_block1,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 2,
            data: &block2,
            used_len: block_size as u64,
        },
    ];
    repo.persist_file_blocks(
        file_id,
        updated.len() as u64,
        block_size as u64,
        3,
        false,
        &updated_rows,
    )
    .map_err(|err| format!("persist updated blocks: {err}"))?;

    if repo
        .file_size(file_id)
        .map_err(|err| format!("file_size: {err}"))?
        != Some(updated.len() as u64)
    {
        return Err("file size changed unexpectedly while updating an existing block".to_string());
    }
    if repo
        .count_file_blocks(file_id)
        .map_err(|err| format!("count_file_blocks: {err}"))?
        != 3
    {
        return Err(
            "block count changed unexpectedly while updating an existing block".to_string(),
        );
    }

    let load_block = |block_index: u64| -> Result<Vec<u8>, String> {
        repo.load_block(file_id, block_index, block_size as u64)?
            .ok_or_else(|| format!("missing block {block_index}"))
    };

    if load_block(0)? != block0 {
        return Err("block 0 changed unexpectedly".to_string());
    }
    if load_block(1)? != mutated_block1 {
        return Err("block 1 was not updated".to_string());
    }
    if load_block(2)? != block2 {
        return Err("block 2 changed unexpectedly".to_string());
    }

    repo.purge_primary_file(file_id)
        .map_err(|err| format!("purge primary file: {err}"))?;
    repo.delete_directory_entry(dir_id)
        .map_err(|err| format!("delete directory: {err}"))?;

    Ok(())
}

#[test]
fn switching_between_block_and_extent_storage_keeps_reads_and_cleanup_consistent(
) -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let snapshot = repo.startup_snapshot()?;
    let block_size = snapshot.block_size.unwrap_or(4096) as usize;
    let block_size_u64 = block_size as u64;

    let dirname = unique_name("rust_pg_switch_dir");
    let dir_id = repo
        .create_directory(None, &dirname, 0o755, 1000, 1000, &unique_name("dir_seed"))
        .map_err(|err| format!("create directory: {err}"))?;

    let extent_file_name = unique_name("rust_pg_extent_to_block");
    let extent_file_id = repo
        .create_file(
            Some(dir_id),
            &extent_file_name,
            0o644,
            1000,
            1000,
            &unique_name("extent_seed"),
        )
        .map_err(|err| format!("create extent-backed file: {err}"))?;

    let extent_block0 = repeated_block(b'A', block_size);
    let extent_block1 = repeated_block(b'B', block_size);
    let extent_tail_len = block_size / 2 + 1;
    let mut extent_block2 = vec![0; block_size];
    extent_block2[..extent_tail_len].fill(b'C');
    let extent_payload = [
        extent_block0.clone(),
        extent_block1.clone(),
        extent_block2[..extent_tail_len].to_vec(),
    ]
    .concat();
    let extent_rows = vec![PersistExtentRow {
        start_block: 0,
        block_count: 3,
        used_bytes: extent_payload.len() as u64,
        payload: extent_payload.clone(),
    }];
    repo.persist_file_extents_native(
        extent_file_id,
        extent_payload.len() as u64,
        block_size_u64,
        3,
        false,
        &extent_rows,
        false,
    )
    .map_err(|err| format!("persist initial extents: {err}"))?;

    assert_block_range_matches(
        &repo,
        extent_file_id,
        block_size_u64,
        &[
            (0, extent_block0.as_slice()),
            (1, extent_block1.as_slice()),
            (2, extent_block2.as_slice()),
        ],
    )?;

    let extent_object_id = repo
        .file_data_object_id(extent_file_id)
        .map_err(|err| format!("extent file data object id: {err}"))?
        .ok_or_else(|| "missing extent file data object id".to_string())?;
    if table_row_count_for_file(&repo, "data_extents", extent_file_id)? != 1 {
        return Err("extent-backed file should have exactly one extent row".to_string());
    }
    if table_row_count_for_file(&repo, "data_blocks", extent_file_id)? != 0 {
        return Err("extent-backed file should not have block rows".to_string());
    }

    let partial_block1 = repeated_block(b'P', block_size);
    let partial_rows = [PersistBlockRow {
        block_index: 1,
        data: &partial_block1,
        used_len: block_size_u64,
    }];
    repo.persist_file_blocks(
        extent_file_id,
        extent_payload.len() as u64,
        block_size_u64,
        3,
        false,
        &partial_rows,
    )
    .map_err(|err| format!("partially update extent-backed file: {err}"))?;

    if repo.file_data_object_id(extent_file_id)? != Some(extent_object_id) {
        return Err("partial block update should keep the unshared data object".to_string());
    }
    assert_block_range_matches(
        &repo,
        extent_file_id,
        block_size_u64,
        &[
            (0, extent_block0.as_slice()),
            (1, partial_block1.as_slice()),
            (2, extent_block2.as_slice()),
        ],
    )?;
    if table_row_count_for_file(&repo, "data_extents", extent_file_id)? != 0 {
        return Err("partial block update should remove converted extent rows".to_string());
    }
    if table_row_count_for_file(&repo, "data_blocks", extent_file_id)? != 3 {
        return Err("partial block update should preserve all three blocks".to_string());
    }

    let block_block0 = repeated_block(b'X', block_size);
    let block_block1 = repeated_block(b'Y', block_size);
    let block_block2 = repeated_block(b'Z', block_size);
    let block_payload = [
        block_block0.clone(),
        block_block1.clone(),
        block_block2.clone(),
    ]
    .concat();
    let block_rows = vec![
        PersistBlockRow {
            block_index: 0,
            data: &block_block0,
            used_len: block_size_u64,
        },
        PersistBlockRow {
            block_index: 1,
            data: &block_block1,
            used_len: block_size_u64,
        },
        PersistBlockRow {
            block_index: 2,
            data: &block_block2,
            used_len: block_size_u64,
        },
    ];
    repo.persist_file_blocks(
        extent_file_id,
        block_payload.len() as u64,
        block_size_u64,
        3,
        false,
        &block_rows,
    )
    .map_err(|err| format!("switch extent-backed file to blocks: {err}"))?;

    let block_replacement_object_id = repo
        .file_data_object_id(extent_file_id)
        .map_err(|err| format!("extent file data object id after block write: {err}"))?
        .ok_or_else(|| "missing replacement data object after block write".to_string())?;
    assert_ne!(block_replacement_object_id, extent_object_id);
    assert_block_range_matches(
        &repo,
        extent_file_id,
        block_size_u64,
        &[
            (0, block_block0.as_slice()),
            (1, block_block1.as_slice()),
            (2, block_block2.as_slice()),
        ],
    )?;
    if table_row_count_for_file(&repo, "data_extents", extent_file_id)? != 0 {
        return Err("block-backed rewrite should remove stale extent rows".to_string());
    }
    if table_row_count_for_file(&repo, "data_blocks", extent_file_id)? != 3 {
        return Err("block-backed rewrite should leave three block rows".to_string());
    }

    let block_file_name = unique_name("rust_pg_block_to_extent");
    let block_file_id = repo
        .create_file(
            Some(dir_id),
            &block_file_name,
            0o644,
            1000,
            1000,
            &unique_name("block_seed"),
        )
        .map_err(|err| format!("create block-backed file: {err}"))?;

    let block_initial0 = repeated_block(b'D', block_size);
    let block_initial1 = repeated_block(b'E', block_size);
    let block_initial2 = repeated_block(b'F', block_size);
    let block_initial_payload = [
        block_initial0.clone(),
        block_initial1.clone(),
        block_initial2.clone(),
    ]
    .concat();
    let block_initial_rows = vec![
        PersistBlockRow {
            block_index: 0,
            data: &block_initial0,
            used_len: block_size_u64,
        },
        PersistBlockRow {
            block_index: 1,
            data: &block_initial1,
            used_len: block_size_u64,
        },
        PersistBlockRow {
            block_index: 2,
            data: &block_initial2,
            used_len: block_size_u64,
        },
    ];
    repo.persist_file_blocks(
        block_file_id,
        block_initial_payload.len() as u64,
        block_size_u64,
        3,
        false,
        &block_initial_rows,
    )
    .map_err(|err| format!("persist initial blocks: {err}"))?;

    assert_block_range_matches(
        &repo,
        block_file_id,
        block_size_u64,
        &[
            (0, block_initial0.as_slice()),
            (1, block_initial1.as_slice()),
            (2, block_initial2.as_slice()),
        ],
    )?;

    let block_object_id = repo
        .file_data_object_id(block_file_id)
        .map_err(|err| format!("block file data object id: {err}"))?
        .ok_or_else(|| "missing block file data object id".to_string())?;
    if table_row_count_for_file(&repo, "data_blocks", block_file_id)? != 3 {
        return Err("block-backed file should have three block rows".to_string());
    }
    if table_row_count_for_file(&repo, "data_extents", block_file_id)? != 0 {
        return Err("block-backed file should not have extent rows".to_string());
    }

    let extent_swap_block0 = repeated_block(b'G', block_size);
    let extent_swap_block1 = repeated_block(b'H', block_size);
    let extent_swap_block2 = repeated_block(b'I', block_size);
    let extent_swap_payload = [
        extent_swap_block0.clone(),
        extent_swap_block1.clone(),
        extent_swap_block2.clone(),
    ]
    .concat();
    let extent_swap_rows = vec![PersistExtentRow {
        start_block: 0,
        block_count: 3,
        used_bytes: extent_swap_payload.len() as u64,
        payload: extent_swap_payload.clone(),
    }];
    repo.persist_file_extents_native(
        block_file_id,
        extent_swap_payload.len() as u64,
        block_size_u64,
        3,
        false,
        &extent_swap_rows,
        false,
    )
    .map_err(|err| format!("switch block-backed file to extents: {err}"))?;

    assert_eq!(
        repo.file_data_object_id(block_file_id)
            .map_err(|err| format!("block file data object id after extent write: {err}"))?,
        Some(block_object_id)
    );
    assert_block_range_matches(
        &repo,
        block_file_id,
        block_size_u64,
        &[
            (0, extent_swap_block0.as_slice()),
            (1, extent_swap_block1.as_slice()),
            (2, extent_swap_block2.as_slice()),
        ],
    )?;
    if table_row_count_for_file(&repo, "data_blocks", block_file_id)? != 0 {
        return Err("extent-backed rewrite should remove stale block rows".to_string());
    }
    if table_row_count_for_file(&repo, "data_extents", block_file_id)? != 1 {
        return Err("extent-backed rewrite should leave one extent row".to_string());
    }

    repo.purge_primary_file(extent_file_id)
        .map_err(|err| format!("purge extent-switch file: {err}"))?;
    repo.purge_primary_file(block_file_id)
        .map_err(|err| format!("purge block-switch file: {err}"))?;
    repo.delete_directory_entry(dir_id)
        .map_err(|err| format!("delete directory: {err}"))?;

    Ok(())
}

#[test]
fn create_data_object_reuses_matching_hash_and_size() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;

    let file_size = 4096u64;
    let content_hash = format!("hash_{}", unique_name("rust_pg_hash"));

    let first_id = repo
        .create_data_object(file_size, Some(content_hash.as_str()))
        .map_err(|err| format!("create first data object: {err}"))?;
    let second_id = repo
        .create_data_object(file_size, Some(content_hash.as_str()))
        .map_err(|err| format!("create second data object: {err}"))?;

    if first_id != second_id {
        return Err("expected identical hash+size to reuse the same data object id".to_string());
    }

    let reference_count = repo
        .query_scalar_text(&format!(
            "SELECT reference_count FROM data_objects WHERE id_data_object = {}",
            first_id
        ))
        .map_err(|err| format!("query reference_count: {err}"))?
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("parse reference_count: {err}"))?;
    if reference_count != 2 {
        return Err(format!(
            "expected reused data object reference_count to be 2, got {reference_count}"
        ));
    }

    repo.query_scalar_text(&format!(
        "DELETE FROM data_objects WHERE id_data_object = {} RETURNING id_data_object::text",
        first_id
    ))
    .map_err(|err| format!("cleanup data object: {err}"))?;

    Ok(())
}

#[test]
fn shared_data_object_survives_purge_of_adopted_copy() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let snapshot = repo.startup_snapshot()?;
    let block_size = snapshot.block_size.unwrap_or(4096) as usize;

    let dirname = unique_name("rust_pg_shared_purge_dir");
    let dir_id = repo
        .create_directory(None, &dirname, 0o755, 1000, 1000, &unique_name("dir_seed"))
        .map_err(|err| format!("create directory: {err}"))?;

    let src_name = unique_name("rust_pg_shared_src");
    let src_file_id = repo
        .create_file(
            Some(dir_id),
            &src_name,
            0o644,
            1000,
            1000,
            &unique_name("src_seed"),
        )
        .map_err(|err| format!("create source file: {err}"))?;
    let dst_name = unique_name("rust_pg_shared_dst");
    let dst_file_id = repo
        .create_file(
            Some(dir_id),
            &dst_name,
            0o644,
            1000,
            1000,
            &unique_name("dst_seed"),
        )
        .map_err(|err| format!("create destination file: {err}"))?;

    let block0 = vec![b'A'; block_size];
    let block1 = vec![b'B'; block_size];
    let block2 = vec![b'C'; block_size];
    let payload = [block0.clone(), block1.clone(), block2.clone()].concat();
    let rows = vec![
        PersistBlockRow {
            block_index: 0,
            data: &block0,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 1,
            data: &block1,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 2,
            data: &block2,
            used_len: block_size as u64,
        },
    ];
    repo.persist_file_blocks(
        src_file_id,
        payload.len() as u64,
        block_size as u64,
        3,
        false,
        &rows,
    )
    .map_err(|err| format!("persist source blocks: {err}"))?;

    if !repo
        .adopt_source_data_object(src_file_id, dst_file_id)
        .map_err(|err| format!("adopt source data object: {err}"))?
    {
        return Err("expected source adoption to succeed for empty destination".to_string());
    }

    if repo
        .file_data_object_id(src_file_id)
        .map_err(|err| format!("source data object id: {err}"))?
        != repo
            .file_data_object_id(dst_file_id)
            .map_err(|err| format!("destination data object id: {err}"))?
    {
        return Err(
            "expected source and destination to share the same data object before purge"
                .to_string(),
        );
    }

    repo.purge_primary_file(dst_file_id)
        .map_err(|err| format!("purge shared destination: {err}"))?;

    if repo
        .load_block(src_file_id, 0, block_size as u64)
        .map_err(|err| format!("load source block 0: {err}"))?
        != Some(block0.clone())
    {
        return Err("source block 0 changed after purging shared destination".to_string());
    }
    if repo
        .load_block(src_file_id, 1, block_size as u64)
        .map_err(|err| format!("load source block 1: {err}"))?
        != Some(block1.clone())
    {
        return Err("source block 1 changed after purging shared destination".to_string());
    }
    if repo
        .load_block(src_file_id, 2, block_size as u64)
        .map_err(|err| format!("load source block 2: {err}"))?
        != Some(block2.clone())
    {
        return Err("source block 2 changed after purging shared destination".to_string());
    }
    if repo
        .file_data_object_id(dst_file_id)
        .map_err(|err| format!("destination data object id after purge: {err}"))?
        .is_some()
    {
        return Err("purged destination should no longer have a data object".to_string());
    }

    repo.purge_primary_file(src_file_id)
        .map_err(|err| format!("purge source file: {err}"))?;
    repo.delete_directory_entry(dir_id)
        .map_err(|err| format!("delete directory: {err}"))?;

    Ok(())
}

#[test]
fn shared_data_object_survives_purge_of_adopted_source() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let snapshot = repo.startup_snapshot()?;
    let block_size = snapshot.block_size.unwrap_or(4096) as usize;

    let dirname = unique_name("rust_pg_shared_source_dir");
    let dir_id = repo
        .create_directory(None, &dirname, 0o755, 1000, 1000, &unique_name("dir_seed"))
        .map_err(|err| format!("create directory: {err}"))?;

    let src_name = unique_name("rust_pg_shared_source_src");
    let src_file_id = repo
        .create_file(
            Some(dir_id),
            &src_name,
            0o644,
            1000,
            1000,
            &unique_name("src_seed"),
        )
        .map_err(|err| format!("create source file: {err}"))?;
    let dst_name = unique_name("rust_pg_shared_source_dst");
    let dst_file_id = repo
        .create_file(
            Some(dir_id),
            &dst_name,
            0o644,
            1000,
            1000,
            &unique_name("dst_seed"),
        )
        .map_err(|err| format!("create destination file: {err}"))?;

    let block0 = vec![b'A'; block_size];
    let block1 = vec![b'B'; block_size];
    let block2 = vec![b'C'; block_size];
    let payload = [block0.clone(), block1.clone(), block2.clone()].concat();
    let rows = vec![
        PersistBlockRow {
            block_index: 0,
            data: &block0,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 1,
            data: &block1,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 2,
            data: &block2,
            used_len: block_size as u64,
        },
    ];
    repo.persist_file_blocks(
        src_file_id,
        payload.len() as u64,
        block_size as u64,
        3,
        false,
        &rows,
    )
    .map_err(|err| format!("persist source blocks: {err}"))?;

    if !repo
        .adopt_source_data_object(src_file_id, dst_file_id)
        .map_err(|err| format!("adopt source data object: {err}"))?
    {
        return Err("expected source adoption to succeed for empty destination".to_string());
    }

    repo.purge_primary_file(src_file_id)
        .map_err(|err| format!("purge shared source: {err}"))?;

    if repo
        .load_block(dst_file_id, 0, block_size as u64)
        .map_err(|err| format!("load destination block 0: {err}"))?
        != Some(block0.clone())
    {
        return Err("destination block 0 changed after purging shared source".to_string());
    }
    if repo
        .load_block(dst_file_id, 1, block_size as u64)
        .map_err(|err| format!("load destination block 1: {err}"))?
        != Some(block1.clone())
    {
        return Err("destination block 1 changed after purging shared source".to_string());
    }
    if repo
        .load_block(dst_file_id, 2, block_size as u64)
        .map_err(|err| format!("load destination block 2: {err}"))?
        != Some(block2.clone())
    {
        return Err("destination block 2 changed after purging shared source".to_string());
    }
    if repo
        .file_data_object_id(src_file_id)
        .map_err(|err| format!("source data object id after purge: {err}"))?
        .is_some()
    {
        return Err("purged source should no longer have a data object".to_string());
    }

    repo.purge_primary_file(dst_file_id)
        .map_err(|err| format!("purge destination file: {err}"))?;
    repo.delete_directory_entry(dir_id)
        .map_err(|err| format!("delete directory: {err}"))?;

    Ok(())
}

#[test]
fn shared_data_object_detaches_on_write() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let snapshot = repo.startup_snapshot()?;
    let block_size = snapshot.block_size.unwrap_or(4096) as usize;

    let dirname = unique_name("rust_pg_shared_detach_dir");
    let dir_id = repo
        .create_directory(None, &dirname, 0o755, 1000, 1000, &unique_name("dir_seed"))
        .map_err(|err| format!("create directory: {err}"))?;

    let src_name = unique_name("rust_pg_detach_src");
    let src_file_id = repo
        .create_file(
            Some(dir_id),
            &src_name,
            0o644,
            1000,
            1000,
            &unique_name("src_seed"),
        )
        .map_err(|err| format!("create source file: {err}"))?;
    let dst_name = unique_name("rust_pg_detach_dst");
    let dst_file_id = repo
        .create_file(
            Some(dir_id),
            &dst_name,
            0o644,
            1000,
            1000,
            &unique_name("dst_seed"),
        )
        .map_err(|err| format!("create destination file: {err}"))?;

    let block0 = vec![b'A'; block_size];
    let block1 = vec![b'B'; block_size];
    let block2 = vec![b'C'; block_size];
    let payload = [block0.clone(), block1.clone(), block2.clone()].concat();
    let rows = vec![
        PersistBlockRow {
            block_index: 0,
            data: &block0,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 1,
            data: &block1,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 2,
            data: &block2,
            used_len: block_size as u64,
        },
    ];
    repo.persist_file_blocks(
        src_file_id,
        payload.len() as u64,
        block_size as u64,
        3,
        false,
        &rows,
    )
    .map_err(|err| format!("persist source blocks: {err}"))?;

    if !repo
        .adopt_source_data_object(src_file_id, dst_file_id)
        .map_err(|err| format!("adopt source data object: {err}"))?
    {
        return Err("expected source adoption to succeed for empty destination".to_string());
    }

    let mutated_block1 = vec![b'X'; block_size];
    let updated_payload = [block0.clone(), mutated_block1.clone(), block2.clone()].concat();
    let updated_rows = vec![
        PersistBlockRow {
            block_index: 0,
            data: &block0,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 1,
            data: &mutated_block1,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 2,
            data: &block2,
            used_len: block_size as u64,
        },
    ];
    repo.persist_file_blocks(
        dst_file_id,
        updated_payload.len() as u64,
        block_size as u64,
        3,
        false,
        &updated_rows,
    )
    .map_err(|err| format!("persist destination blocks after adopt: {err}"))?;

    if repo
        .load_block(src_file_id, 0, block_size as u64)
        .map_err(|err| format!("load source block 0: {err}"))?
        != Some(block0.clone())
    {
        return Err("source block 0 changed after destination write".to_string());
    }
    if repo
        .load_block(src_file_id, 1, block_size as u64)
        .map_err(|err| format!("load source block 1: {err}"))?
        != Some(block1.clone())
    {
        return Err("source block 1 changed after destination write".to_string());
    }
    if repo
        .load_block(src_file_id, 2, block_size as u64)
        .map_err(|err| format!("load source block 2: {err}"))?
        != Some(block2.clone())
    {
        return Err("source block 2 changed after destination write".to_string());
    }

    if repo
        .load_block(dst_file_id, 1, block_size as u64)
        .map_err(|err| format!("load destination block 1: {err}"))?
        != Some(mutated_block1.clone())
    {
        return Err("destination block 1 was not updated after detach".to_string());
    }

    let src_object_id = repo
        .file_data_object_id(src_file_id)
        .map_err(|err| format!("source data object id: {err}"))?
        .ok_or_else(|| "missing source data object id".to_string())?;
    let dst_object_id = repo
        .file_data_object_id(dst_file_id)
        .map_err(|err| format!("destination data object id: {err}"))?
        .ok_or_else(|| "missing destination data object id".to_string())?;
    if src_object_id == dst_object_id {
        return Err("expected destination to detach to a new data object".to_string());
    }

    repo.purge_primary_file(dst_file_id)
        .map_err(|err| format!("purge destination file: {err}"))?;
    repo.purge_primary_file(src_file_id)
        .map_err(|err| format!("purge source file: {err}"))?;
    repo.delete_directory_entry(dir_id)
        .map_err(|err| format!("delete directory: {err}"))?;

    Ok(())
}

#[test]
fn shared_data_object_detaches_on_source_write() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let snapshot = repo.startup_snapshot()?;
    let block_size = snapshot.block_size.unwrap_or(4096) as usize;

    let dirname = unique_name("rust_pg_shared_source_write_dir");
    let dir_id = repo
        .create_directory(None, &dirname, 0o755, 1000, 1000, &unique_name("dir_seed"))
        .map_err(|err| format!("create directory: {err}"))?;

    let src_name = unique_name("rust_pg_source_write_src");
    let src_file_id = repo
        .create_file(
            Some(dir_id),
            &src_name,
            0o644,
            1000,
            1000,
            &unique_name("src_seed"),
        )
        .map_err(|err| format!("create source file: {err}"))?;
    let dst_name = unique_name("rust_pg_source_write_dst");
    let dst_file_id = repo
        .create_file(
            Some(dir_id),
            &dst_name,
            0o644,
            1000,
            1000,
            &unique_name("dst_seed"),
        )
        .map_err(|err| format!("create destination file: {err}"))?;

    let block0 = vec![b'A'; block_size];
    let block1 = vec![b'B'; block_size];
    let block2 = vec![b'C'; block_size];
    let payload = [block0.clone(), block1.clone(), block2.clone()].concat();
    let rows = vec![
        PersistBlockRow {
            block_index: 0,
            data: &block0,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 1,
            data: &block1,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 2,
            data: &block2,
            used_len: block_size as u64,
        },
    ];
    repo.persist_file_blocks(
        src_file_id,
        payload.len() as u64,
        block_size as u64,
        3,
        false,
        &rows,
    )
    .map_err(|err| format!("persist source blocks: {err}"))?;

    if !repo
        .adopt_source_data_object(src_file_id, dst_file_id)
        .map_err(|err| format!("adopt source data object: {err}"))?
    {
        return Err("expected source adoption to succeed for empty destination".to_string());
    }

    let mutated_block1 = vec![b'Y'; block_size];
    let updated_payload = [block0.clone(), mutated_block1.clone(), block2.clone()].concat();
    let updated_rows = vec![
        PersistBlockRow {
            block_index: 0,
            data: &block0,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 1,
            data: &mutated_block1,
            used_len: block_size as u64,
        },
        PersistBlockRow {
            block_index: 2,
            data: &block2,
            used_len: block_size as u64,
        },
    ];
    repo.persist_file_blocks(
        src_file_id,
        updated_payload.len() as u64,
        block_size as u64,
        3,
        false,
        &updated_rows,
    )
    .map_err(|err| format!("persist source blocks after adopt: {err}"))?;

    if repo
        .load_block(dst_file_id, 0, block_size as u64)
        .map_err(|err| format!("load destination block 0: {err}"))?
        != Some(block0.clone())
    {
        return Err("destination block 0 changed after source write".to_string());
    }
    if repo
        .load_block(dst_file_id, 1, block_size as u64)
        .map_err(|err| format!("load destination block 1: {err}"))?
        != Some(block1.clone())
    {
        return Err("destination block 1 changed after source write".to_string());
    }
    if repo
        .load_block(dst_file_id, 2, block_size as u64)
        .map_err(|err| format!("load destination block 2: {err}"))?
        != Some(block2.clone())
    {
        return Err("destination block 2 changed after source write".to_string());
    }

    let src_object_id = repo
        .file_data_object_id(src_file_id)
        .map_err(|err| format!("source data object id: {err}"))?
        .ok_or_else(|| "missing source data object id".to_string())?;
    let dst_object_id = repo
        .file_data_object_id(dst_file_id)
        .map_err(|err| format!("destination data object id: {err}"))?
        .ok_or_else(|| "missing destination data object id".to_string())?;
    if src_object_id == dst_object_id {
        return Err("expected source to detach to a new data object".to_string());
    }

    repo.purge_primary_file(src_file_id)
        .map_err(|err| format!("purge source file: {err}"))?;
    repo.purge_primary_file(dst_file_id)
        .map_err(|err| format!("purge destination file: {err}"))?;
    repo.delete_directory_entry(dir_id)
        .map_err(|err| format!("delete directory: {err}"))?;

    Ok(())
}
