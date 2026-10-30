// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_hotpath::pg::{
    DbRepo, DirectoryEntryMetadata, DirectoryUnlinkOutcome, FileUnlinkOutcome, PersistBlockRow,
    ResolvedPath, STORAGE_QUOTA_EXCEEDED_PREFIX,
};
use fod_rust_runtime::{DataObjectSwapCleanup, RuntimeConfig};
use std::env;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn prefer_connection_value(primary: Option<&str>, legacy: Option<&str>, default: &str) -> String {
    primary
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| legacy.map(str::trim).filter(|value| !value.is_empty()))
        .unwrap_or(default)
        .to_string()
}

fn selected_env_value(primary: &str, legacy: &str, default: &str) -> String {
    let primary_value = env::var(primary).ok();
    let legacy_value = env::var(legacy).ok();
    prefer_connection_value(primary_value.as_deref(), legacy_value.as_deref(), default)
}

fn conninfo_from_env() -> String {
    conninfo_for_database(&selected_env_value(
        "FOD_PG_DBNAME",
        "POSTGRES_DB",
        "foddbname",
    ))
}

fn conninfo_for_database(dbname: &str) -> String {
    let host = selected_env_value("FOD_PG_HOST", "POSTGRES_HOST", "127.0.0.1");
    let port = selected_env_value("FOD_PG_PORT", "POSTGRES_PORT", "5432");
    let user = selected_env_value("FOD_PG_USER", "POSTGRES_USER", "foduser");
    let password = selected_env_value("FOD_PG_PASSWORD", "POSTGRES_PASSWORD", "cichosza");
    format!(
        "host='{}' port='{}' dbname='{}' user='{}' password='{}'",
        host, port, dbname, user, password
    )
}

struct TemporaryDatabase {
    admin_repo: DbRepo,
    name: String,
}

impl TemporaryDatabase {
    fn create() -> Result<Self, String> {
        let admin_repo = repo_with_runtime()?;
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| err.to_string())?
            .as_nanos();
        let name = format!("fod_query_fault_{}_{}", std::process::id(), suffix);
        admin_repo.exec(&format!("CREATE DATABASE {name}"))?;
        Ok(Self { admin_repo, name })
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        if let Err(err) = self
            .admin_repo
            .exec(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name))
        {
            eprintln!("temporary PostgreSQL database cleanup failed: {err}");
        }
    }
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
    DbRepo::with_runtime(&conninfo_from_env(), &runtime)
}

#[test]
fn connection_value_prefers_fod_endpoint_over_legacy_postgres_env() {
    assert_eq!(
        prefer_connection_value(Some(" 192.168.1.11 "), Some("127.0.0.1"), "fallback"),
        "192.168.1.11"
    );
    assert_eq!(
        prefer_connection_value(Some("   "), Some(" localhost "), "fallback"),
        "localhost"
    );
    assert_eq!(prefer_connection_value(None, None, "fallback"), "fallback");
}

struct NamespaceFixture {
    repo: DbRepo,
    directory_name: String,
    directory_id: u64,
    file_id: Option<u64>,
    hardlink_id: Option<u64>,
    symlink_id: Option<u64>,
    child_directory_id: Option<u64>,
}

impl Drop for NamespaceFixture {
    fn drop(&mut self) {
        let mut cleanup = Vec::new();
        if let Some(id) = self.hardlink_id {
            cleanup.push(self.repo.delete_hardlink_entry(id));
        }
        if let Some(id) = self.symlink_id {
            cleanup.push(self.repo.delete_owner_xattrs("symlink", id));
            cleanup.push(self.repo.delete_symlink_entry(id));
        }
        if let Some(id) = self.file_id {
            cleanup.push(self.repo.delete_owner_xattrs("file", id));
            cleanup.push(self.repo.purge_primary_file(id));
        }
        cleanup.push(self.repo.delete_owner_xattrs("dir", self.directory_id));
        if let Some(id) = self.child_directory_id {
            cleanup.push(self.repo.delete_directory_entry(id));
        }
        cleanup.push(self.repo.delete_directory_entry(self.directory_id));
        for result in cleanup {
            if let Err(error) = result {
                eprintln!("namespace fixture cleanup failed: {error}");
            }
        }
    }
}

impl NamespaceFixture {
    fn new() -> Result<Self, String> {
        let repo = repo_with_runtime()?;
        let directory_name = unique_name("rust_pg_namespace");
        let directory_id =
            repo.create_directory(None, &directory_name, 0o755, 1000, 1000, &directory_name)?;
        let mut fixture = Self {
            repo,
            directory_name,
            directory_id,
            file_id: None,
            hardlink_id: None,
            symlink_id: None,
            child_directory_id: None,
        };
        let file_id = fixture.repo.create_file(
            Some(directory_id),
            "file",
            0o644,
            1000,
            1000,
            &unique_name("namespace_file_seed"),
        )?;
        fixture.file_id = Some(file_id);
        fixture.hardlink_id = Some(fixture.repo.create_hardlink(
            file_id,
            Some(directory_id),
            "hardlink",
            1000,
            1000,
        )?);
        let symlink_id = fixture.repo.create_symlink(
            Some(directory_id),
            "symlink",
            "file",
            1000,
            1000,
            &unique_name("namespace_symlink_seed"),
        )?;
        fixture.symlink_id = Some(symlink_id);
        Ok(fixture)
    }
}

#[test]
fn metadata_updates_preserve_content_and_hardlink_identity() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let fixture = NamespaceFixture::new()?;
    let repo = &fixture.repo;
    let file_id = fixture.file_id.unwrap();
    let directory_id = fixture.directory_id;
    let symlink_id = fixture.symlink_id.unwrap();
    let bs = u64::from(repo.startup_snapshot()?.block_size.unwrap_or(32768));
    let payload = vec![0x5a; bs as usize];
    let rows = [PersistBlockRow {
        block_index: 0,
        data: &payload,
        used_len: bs,
    }];
    repo.persist_file_blocks_with_crc_flag(file_id, bs, bs, 1, false, &rows, false)?;
    let identity_sql = format!(
        "SELECT data_object_id::text, size::text, content_generation::text FROM files WHERE id_file = {file_id}"
    );
    let identity_before = repo.query_rows_text(&identity_sql)?;
    let alias_path = format!("/{}/hardlink", fixture.directory_name);

    repo.update_file_mode(file_id, "600")?;
    assert_eq!(
        repo.get_file_mode_value(&alias_path)?.as_deref(),
        Some("600")
    );
    repo.update_directory_mode(directory_id, "700")?;
    assert_eq!(
        repo.query_scalar_text(&format!(
            "SELECT mode FROM directories WHERE id_directory = {directory_id}"
        ))?,
        "700"
    );
    repo.update_file_owner(file_id, 2001, 2002, "640")?;
    repo.update_directory_owner(directory_id, 3001, 3002, "750")?;
    repo.update_symlink_owner(symlink_id, 4001, 4002)?;

    repo.touch_file_times(file_id, "2000-01-01 01:02:03", "2000-01-02 01:02:03")?;
    repo.touch_directory_times(directory_id, "2000-01-01 01:02:03", "2000-01-02 01:02:03")?;
    for (table, key, id) in [
        ("files", "id_file", file_id),
        ("directories", "id_directory", directory_id),
    ] {
        assert_eq!(repo.query_scalar_text(&format!(
            "SELECT (access_date = TIMESTAMP '2000-01-01 01:02:03' AND modification_date = TIMESTAMP '2000-01-02 01:02:03')::text FROM {table} WHERE {key} = {id}"
        ))?, "true");
    }
    repo.update_file_access_date(file_id, "2001-02-03 04:05:06")?;
    repo.update_directory_access_date(directory_id, "2001-02-03 04:05:06")?;
    repo.update_symlink_access_date(symlink_id, "2001-02-03 04:05:06")?;
    repo.touch_file_entry(file_id)?;
    repo.touch_directory_entry(directory_id)?;
    repo.touch_symlink_entry(symlink_id)?;

    for (table, key, id, uid, gid) in [
        ("files", "id_file", file_id, 2001, 2002),
        ("directories", "id_directory", directory_id, 3001, 3002),
        ("symlinks", "id_symlink", symlink_id, 4001, 4002),
    ] {
        assert_eq!(repo.query_scalar_text(&format!(
            "SELECT (uid = {uid} AND gid = {gid} AND access_date = TIMESTAMP '2001-02-03 04:05:06' AND modification_date > TIMESTAMP '2000-01-02 01:02:03')::text FROM {table} WHERE {key} = {id}"
        ))?, "true", "table={table}");
    }
    assert_eq!(
        repo.get_file_mode_value(&alias_path)?.as_deref(),
        Some("640")
    );
    assert_eq!(
        repo.query_scalar_text(&format!(
            "SELECT mode FROM directories WHERE id_directory = {directory_id}"
        ))?,
        "750"
    );
    assert_eq!(repo.get_file_id(&alias_path)?, Some(file_id));
    assert_eq!(
        repo.load_symlink_target(symlink_id)?.as_deref(),
        Some("file")
    );
    assert_eq!(repo.query_rows_text(&identity_sql)?, identity_before);
    let (size, blocks) = repo
        .fetch_block_range_with_size_shared(file_id, 0, 0, bs)?
        .expect("persisted payload");
    assert_eq!(size, bs);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, 0);
    assert_eq!(blocks[0].1.as_ref(), payload.as_slice());
    Ok(())
}

#[test]
fn namespace_statistics_count_entries_and_preserve_capacity() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let before = repo.statfs_snapshot()?;
    let roots_before = repo.count_root_directory_children()?;
    let payload_before = repo.total_data_size()?;
    let fixture = NamespaceFixture::new()?;
    let after = repo.statfs_snapshot()?;
    assert_eq!(
        (after.0, after.1, after.2),
        (before.0 + 1, before.1 + 1, before.2 + 1)
    );
    assert_eq!((after.3, after.5), (before.3, before.5));
    assert_eq!(repo.count_files()?, after.0);
    assert_eq!(repo.count_directories()?, after.1);
    assert_eq!(repo.count_symlinks()?, after.2);
    assert_eq!(repo.count_root_directory_children()?, roots_before + 1);
    assert_eq!(repo.total_data_size()?, payload_before);
    assert!(repo.path_has_children(fixture.directory_id)?);
    assert_eq!(repo.count_directory_children(fixture.directory_id)?, 3);
    assert_eq!(repo.count_directory_subdirs(fixture.directory_id)?, 0);
    assert_eq!(repo.count_file_links(fixture.file_id.unwrap())?, 2);
    assert_eq!(repo.count_file_blocks(fixture.file_id.unwrap())?, 0);
    drop(fixture);
    assert_eq!(repo.count_files()?, before.0);
    assert_eq!(repo.count_directories()?, before.1);
    assert_eq!(repo.count_symlinks()?, before.2);
    assert_eq!(repo.count_root_directory_children()?, roots_before);
    Ok(())
}

#[test]
fn directory_unlink_revalidates_binding_and_children() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let mut fixture = NamespaceFixture::new()?;
    let repo = &fixture.repo;
    assert_eq!(
        repo.unlink_directory_entry(fixture.directory_id, None, &fixture.directory_name, 1000)?,
        DirectoryUnlinkOutcome::NotEmpty
    );
    let child = repo.create_directory(
        Some(fixture.directory_id),
        "rmdir-child",
        0o755,
        1000,
        1000,
        &unique_name("rmdir_seed"),
    )?;
    fixture.child_directory_id = Some(child);
    assert_eq!(
        repo.unlink_directory_entry(child, None, "rmdir-child", 1000)?,
        DirectoryUnlinkOutcome::SourceChanged
    );
    repo.exec(&format!(
        "UPDATE directories SET name = 'rmdir-moved' WHERE id_directory = {child}"
    ))?;
    assert_eq!(
        repo.unlink_directory_entry(child, Some(fixture.directory_id), "rmdir-child", 1000)?,
        DirectoryUnlinkOutcome::SourceChanged
    );
    assert_eq!(
        repo.get_dir_id(&format!("/{}/rmdir-moved", fixture.directory_name))?,
        Some(child)
    );
    assert_eq!(
        repo.unlink_directory_entry(child, Some(fixture.directory_id), "rmdir-moved", 1000)?,
        DirectoryUnlinkOutcome::Removed
    );
    assert_eq!(
        repo.unlink_directory_entry(child, Some(fixture.directory_id), "rmdir-moved", 1000)?,
        DirectoryUnlinkOutcome::SourceChanged
    );
    Ok(())
}

#[test]
fn directory_unlink_waits_for_child_transaction_and_sees_its_commit() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let mut fixture = NamespaceFixture::new()?;
    let child = fixture.repo.create_directory(
        Some(fixture.directory_id),
        "rmdir-gate",
        0o755,
        1000,
        1000,
        &unique_name("rmdir_gate_seed"),
    )?;
    fixture.child_directory_id = Some(child);
    let writer = repo_with_runtime()?;
    let remover = repo_with_runtime()?;
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let parent = fixture.directory_id;
    let writer_thread = std::thread::spawn(move || {
        writer.with_transaction(|tx| {
        tx.exec(&format!("INSERT INTO directories (id_parent,name,mode,uid,gid,inode_seed,modification_date,access_date,change_date,creation_date) VALUES ({child},'pending-child','755',1000,1000,'rmdir-pending',NOW(),NOW(),NOW(),NOW())"))?;
        ready_tx.send(()).map_err(|err| err.to_string())?;
        release_rx.recv_timeout(std::time::Duration::from_secs(30)).map_err(|err| err.to_string())?;
        Ok(())
    })
    });
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .map_err(|err| err.to_string())?;
    let remove_thread = std::thread::spawn(move || {
        remover.unlink_directory_entry(child, Some(parent), "rmdir-gate", 1000)
    });
    let observed_wait = (|| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let count = fixture.repo.query_scalar_text("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query LIKE '%FROM directories%FOR UPDATE%'")?;
            if count.trim() != "0" {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err("rmdir did not wait for the child's FK lock".to_string());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    })();
    release_tx.send(()).map_err(|err| err.to_string())?;
    writer_thread
        .join()
        .map_err(|_| "writer thread panicked".to_string())??;
    let outcome = remove_thread
        .join()
        .map_err(|_| "rmdir thread panicked".to_string())??;
    // Clean the committed test child before the fixture removes its parent.
    fixture.repo.exec(&format!(
        "DELETE FROM directories WHERE id_parent = {child}"
    ))?;
    observed_wait?;
    assert_eq!(outcome, DirectoryUnlinkOutcome::NotEmpty);
    Ok(())
}

#[test]
fn namespace_lookups_preserve_entry_identity_and_metadata() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let fixture = NamespaceFixture::new()?;
    let repo = &fixture.repo;
    let directory_path = format!("/{}", fixture.directory_name);
    let file_id = fixture.file_id.unwrap();
    let hardlink_id = fixture.hardlink_id.unwrap();
    let symlink_id = fixture.symlink_id.unwrap();

    assert_eq!(
        repo.get_dir_id(&directory_path)?,
        Some(fixture.directory_id)
    );
    assert_eq!(repo.resolve_path("/")?.kind.as_deref(), Some("dir"));
    let root_entries: Vec<DirectoryEntryMetadata> = repo.list_directory_entry_metadata("/")?;
    assert!(root_entries.iter().any(|entry| {
        entry.name == fixture.directory_name
            && entry.kind == "dir"
            && entry.entry_id == fixture.directory_id
    }));

    let entries: Vec<DirectoryEntryMetadata> =
        repo.list_directory_entry_metadata(&directory_path)?;
    assert_eq!(entries.len(), 3);
    for (name, kind, entry_id) in [
        ("file", "file", file_id),
        ("hardlink", "hardlink", hardlink_id),
        ("symlink", "symlink", symlink_id),
    ] {
        let path = format!("{directory_path}/{name}");
        assert_eq!(
            repo.resolve_path(&path)?,
            ResolvedPath {
                parent_id: Some(fixture.directory_id),
                kind: Some(kind.to_string()),
                entry_id: Some(entry_id),
            }
        );
        let metadata = entries.iter().find(|entry| entry.name == name).unwrap();
        assert_eq!(metadata.kind, kind);
        assert_eq!(metadata.entry_id, entry_id);
        assert!(!metadata.inode_seed.is_empty());
        assert_eq!(metadata.special_type, None);
        let attrs = repo
            .fetch_path_attrs_blob(&path)?
            .expect("entry attributes");
        assert_eq!(attrs.split(|byte| *byte == 0).next(), Some(kind.as_bytes()));
    }
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.kind == "file")
            .unwrap()
            .inode_seed,
        entries
            .iter()
            .find(|entry| entry.kind == "hardlink")
            .unwrap()
            .inode_seed
    );

    let blob = repo
        .list_directory_entries_blob(&directory_path)?
        .expect("directory entries");
    let mut names: Vec<_> = blob
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            b"file".as_slice(),
            b"hardlink".as_slice(),
            b"symlink".as_slice()
        ]
    );
    assert_eq!(
        repo.get_file_id(&format!("{directory_path}/file"))?,
        Some(file_id)
    );
    assert_eq!(
        repo.get_file_id(&format!("{directory_path}/hardlink"))?,
        Some(file_id)
    );
    assert_eq!(
        repo.get_file_mode_value(&format!("{directory_path}/file"))?
            .as_deref(),
        Some("644")
    );
    assert_eq!(
        repo.get_hardlink_id(&format!("{directory_path}/hardlink"))?,
        Some(hardlink_id)
    );
    assert_eq!(repo.get_hardlink_file_id(hardlink_id)?, Some(file_id));
    assert_eq!(
        repo.get_symlink_id(&format!("{directory_path}/symlink"))?,
        Some(symlink_id)
    );
    assert_eq!(
        repo.load_symlink_target(symlink_id)?.as_deref(),
        Some("file")
    );
    assert_eq!(repo.get_special_file_metadata(file_id)?, None);

    let missing = format!("{directory_path}/missing");
    assert_eq!(repo.resolve_path(&missing)?.kind, None);
    assert_eq!(repo.fetch_path_attrs_blob(&missing)?, None);
    assert_eq!(repo.get_file_id(&missing)?, None);
    assert_eq!(repo.get_hardlink_id(&missing)?, None);
    assert_eq!(repo.get_symlink_id(&missing)?, None);
    Ok(())
}

#[test]
fn directory_lookup_rejects_missing_path_components() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let mut fixture = NamespaceFixture::new()?;
    let child_path = format!("/{}/child", fixture.directory_name);
    assert_eq!(fixture.repo.get_dir_id(&child_path)?, None);
    let child_id = fixture.repo.create_directory(
        Some(fixture.directory_id),
        "child",
        0o755,
        1000,
        1000,
        &unique_name("lookup_child_seed"),
    )?;
    fixture.child_directory_id = Some(child_id);
    assert_eq!(fixture.repo.get_dir_id(&child_path)?, Some(child_id));
    assert_eq!(
        fixture.repo.get_dir_id(&format!("{child_path}/"))?,
        Some(child_id)
    );
    assert_eq!(
        fixture.repo.get_dir_id(&fixture.directory_name)?,
        Some(fixture.directory_id)
    );
    // The public API keeps SQL NULL as the parent of entries in the logical root.
    assert_eq!(fixture.repo.get_dir_id("/")?, None);
    assert_eq!(fixture.repo.get_dir_id("")?, None);
    for suffix in [
        "missing",
        "missing/child",
        "child/missing",
        "child/missing/grandchild",
    ] {
        let path = format!("/{}/{suffix}", fixture.directory_name);
        assert_eq!(fixture.repo.get_dir_id(&path)?, None, "path={path}");
    }
    Ok(())
}

#[test]
fn missing_namespace_parents_never_alias_existing_entries() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let fixture = NamespaceFixture::new()?;
    let repo = &fixture.repo;
    repo.store_xattr_value_for_owner("dir", fixture.directory_id, "user.lookup_guard", b"canary")?;
    repo.store_xattr_value_for_owner(
        "file",
        fixture.file_id.unwrap(),
        "user.lookup_guard",
        b"canary",
    )?;

    // A missing component must not select either the existing ancestor's
    // children or the same-named directory at the logical root.
    for parent in [
        format!("/{}/missing", fixture.directory_name),
        format!("/{}_missing", fixture.directory_name),
    ] {
        assert!(
            repo.list_directory_entry_metadata(&parent)?.is_empty(),
            "path={parent}"
        );
        assert_eq!(
            repo.list_directory_entries_blob(&parent)?,
            None,
            "path={parent}"
        );
        for name in [
            "file",
            "hardlink",
            "symlink",
            fixture.directory_name.as_str(),
        ] {
            let path = format!("{parent}/{name}");
            assert_eq!(repo.resolve_path(&path)?.kind, None, "path={path}");
            assert_eq!(repo.fetch_path_attrs_blob(&path)?, None, "path={path}");
            assert_eq!(repo.get_file_id(&path)?, None, "path={path}");
            assert_eq!(repo.get_file_mode_value(&path)?, None, "path={path}");
            assert_eq!(repo.get_hardlink_id(&path)?, None, "path={path}");
            assert_eq!(repo.get_symlink_id(&path)?, None, "path={path}");
            assert_eq!(
                repo.fetch_xattr_value(&path, "user.lookup_guard")?,
                None,
                "path={path}"
            );
            assert_eq!(repo.list_xattr_names(&path)?, None, "path={path}");
        }
    }
    assert_eq!(
        repo.fetch_xattr_value(&format!("/{}", fixture.directory_name), "user.lookup_guard")?,
        Some(b"canary".to_vec())
    );
    assert_eq!(repo.resolve_path("/")?.kind.as_deref(), Some("dir"));
    assert!(repo.list_directory_entries_blob("/")?.is_some());
    Ok(())
}

#[test]
fn xattrs_share_path_and_owner_semantics_across_entry_kinds() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let fixture = NamespaceFixture::new()?;
    let directory_name = &fixture.directory_name;
    let directory_id = fixture.directory_id;
    let file_id = fixture.file_id.unwrap();
    let symlink_id = fixture.symlink_id.unwrap();
    let repo = &fixture.repo;

    for (kind, id, path) in [
        ("file", file_id, format!("/{directory_name}/file")),
        ("dir", directory_id, format!("/{directory_name}")),
        ("symlink", symlink_id, format!("/{directory_name}/symlink")),
    ] {
        assert_eq!(repo.list_xattr_names(&path)?, Some(vec![]));
        assert_eq!(repo.fetch_xattr_value(&path, "user.absent")?, None);
        repo.store_xattr_value_for_owner(kind, id, "user.z", &[0, 255, 128, 10])?;
        repo.store_xattr_value_for_owner(kind, id, "user.a", b"")?;
        let names = vec!["user.a".to_string(), "user.z".to_string()];
        assert_eq!(repo.list_xattr_names_for_owner(kind, id)?, names);
        assert_eq!(repo.list_xattr_names(&path)?, Some(names));
        assert_eq!(repo.fetch_xattr_value(&path, "user.a")?, Some(vec![]));
        assert_eq!(
            repo.fetch_xattr_value(&path, "user.z")?,
            Some(vec![0, 255, 128, 10])
        );
        repo.store_xattr_value_for_owner(kind, id, "user.z", b"updated")?;
        assert_eq!(
            repo.fetch_xattr_value(&path, "user.z")?,
            Some(b"updated".to_vec())
        );

        if kind == "file" {
            let alias = format!("/{directory_name}/hardlink");
            assert_eq!(
                repo.list_xattr_names(&alias)?,
                repo.list_xattr_names(&path)?
            );
            assert_eq!(
                repo.fetch_xattr_value(&alias, "user.z")?,
                Some(b"updated".to_vec())
            );
        }

        assert_eq!(repo.remove_xattr_for_owner(kind, id, "user.absent")?, 0);
        assert_eq!(repo.remove_xattr_for_owner(kind, id, "user.a")?, 1);
        assert_eq!(repo.fetch_xattr_value(&path, "user.a")?, None);
        repo.delete_owner_xattrs(kind, id)?;
        assert_eq!(repo.list_xattr_names(&path)?, Some(vec![]));
    }

    let missing = format!("/{directory_name}/missing");
    assert_eq!(repo.list_xattr_names(&missing)?, None);
    assert_eq!(repo.fetch_xattr_value(&missing, "user.z")?, None);
    assert_eq!(
        repo.list_xattr_names("/")?,
        Some(repo.list_xattr_names_for_owner("dir", 0)?)
    );
    Ok(())
}

#[test]
fn fused_block_range_returns_file_size_and_payload() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let block_size = repo.startup_snapshot()?.block_size.unwrap_or(4096) as usize;
    let bs = block_size as u64;
    let dir_id = repo.create_directory(
        None,
        &unique_name("rust_pg_fused_read_dir"),
        0o755,
        1000,
        1000,
        &unique_name("rust_pg_fused_read_dir_seed"),
    )?;
    let file_id = repo.create_file(
        Some(dir_id),
        &unique_name("rust_pg_fused_read_file"),
        0o644,
        1000,
        1000,
        &unique_name("rust_pg_fused_read_file_seed"),
    )?;
    let first = vec![0x41; block_size];
    let second = vec![0x42; block_size];
    let rows = [
        PersistBlockRow {
            block_index: 0,
            data: &first,
            used_len: bs,
        },
        PersistBlockRow {
            block_index: 1,
            data: &second,
            used_len: bs,
        },
    ];
    let result = (|| {
        repo.persist_file_blocks_with_crc_flag(file_id, bs * 2, bs, 2, false, &rows, false)?;
        let (size, blocks) = repo
            .fetch_block_range_with_size_shared(file_id, 0, 1, bs)?
            .ok_or_else(|| "fused read did not return file metadata".to_string())?;
        if size != bs * 2 || blocks.len() != 2 {
            return Err(format!(
                "invalid fused read size={size} blocks={}",
                blocks.len()
            ));
        }
        if blocks[0].0 != 0 || blocks[0].1.as_ref() != first.as_slice() {
            return Err("fused read first block mismatch".to_string());
        }
        if blocks[1].0 != 1 || blocks[1].1.as_ref() != second.as_slice() {
            return Err("fused read second block mismatch".to_string());
        }
        let (size2, blocks2) = repo
            .fetch_block_range_with_size_shared(file_id, 20, 21, bs)?
            .ok_or_else(|| "fused read lost metadata beyond EOF".to_string())?;
        if size2 != bs * 2 || !blocks2.is_empty() {
            return Err("invalid fused read beyond EOF".to_string());
        }
        // files.id_file is PostgreSQL SERIAL/int4. Use a valid int4 value
        // that is practically impossible in this isolated test database.
        let missing = repo.fetch_block_range_with_size_shared(i32::MAX as u64, 0, 1, bs)?;
        if missing.is_some() {
            return Err("fused read returned metadata for a missing file".to_string());
        }
        Ok(())
    })();
    let cleanup = repo
        .purge_primary_file(file_id)
        .and_then(|_| repo.delete_directory_entry(dir_id));
    result.and(cleanup)
}

#[test]
fn generated_block_snapshot_matches_committed_payload() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    // The generation-bearing query requires schema v26 or newer.
    if !matches!(repo.startup_snapshot()?.schema_version, Some(26..)) {
        return Err("TX03J generation snapshot test requires FOD schema 26 or newer".to_string());
    }
    let bs = u64::from(repo.startup_snapshot()?.block_size.unwrap_or(32768));
    let file_id = repo.create_file(
        None,
        &unique_name("tx03j_generated_read"),
        0o644,
        1000,
        1000,
        &unique_name("tx03j_generated_read_seed"),
    )?;
    let payload = vec![0x5a; bs as usize];
    let rows = [PersistBlockRow {
        block_index: 0,
        data: &payload,
        used_len: bs,
    }];
    let result = (|| {
        let (_, initial_generation, empty) = repo
            .fetch_block_range_with_generation_shared(file_id, 0, 0, bs)?
            .ok_or_else(|| "missing initial generation metadata".to_string())?;
        if !empty.is_empty() {
            return Err("new file unexpectedly has content blocks".to_string());
        }
        repo.persist_file_blocks_with_crc_flag(file_id, bs, bs, 1, false, &rows, false)?;
        let (size, generation, blocks) = repo
            .fetch_block_range_with_generation_shared(file_id, 0, 0, bs)?
            .ok_or_else(|| "missing committed snapshot metadata".to_string())?;
        if size != bs || generation <= initial_generation {
            return Err(format!(
                "committed content generation not advanced: initial={initial_generation} current={generation} size={size}"
            ));
        }
        if blocks.len() != 1 || blocks[0].0 != 0 || blocks[0].1.as_ref() != payload.as_slice() {
            return Err("generation snapshot returned incorrect committed block".to_string());
        }
        let (eof_size, eof_generation, eof_blocks) = repo
            .fetch_block_range_with_generation_shared(file_id, 10, 11, bs)?
            .ok_or_else(|| "missing metadata after EOF".to_string())?;
        if eof_size != bs || eof_generation != generation || !eof_blocks.is_empty() {
            return Err("generation snapshot EOF metadata mismatch".to_string());
        }
        if repo
            .fetch_block_range_with_generation_shared(i32::MAX as u64, 0, 0, bs)?
            .is_some()
        {
            return Err("generation snapshot returned a nonexistent file".to_string());
        }
        Ok(())
    })();
    let cleanup = repo.purge_primary_file(file_id);
    result.and(cleanup)
}

#[test]
fn block_read_decoders_reject_null_payloads() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let block_size = u64::from(repo.startup_snapshot()?.block_size.unwrap_or(4096));
    let file_id = repo.create_file(
        None,
        &unique_name("null_block_payload"),
        0o644,
        1000,
        1000,
        &unique_name("null_block_payload_seed"),
    )?;
    let payload = vec![0x5a; block_size as usize];
    let row = PersistBlockRow {
        block_index: 0,
        data: &payload,
        used_len: block_size,
    };

    let result = (|| {
        repo.persist_file_blocks_with_crc_flag(
            file_id,
            block_size,
            block_size,
            1,
            false,
            &[row],
            false,
        )?;
        repo.exec("ALTER TABLE data_blocks ALTER COLUMN data DROP NOT NULL")?;
        repo.exec(&format!(
            "UPDATE data_blocks SET data = NULL WHERE data_object_id = (SELECT data_object_id FROM files WHERE id_file = {file_id}) AND _order = 0"
        ))?;

        fn expect_null_payload_error<T>(
            result: Result<T, String>,
            operation: &str,
        ) -> Result<(), String> {
            match result {
                Err(error) if error.contains("NULL") => Ok(()),
                Err(error) => Err(format!("{operation} returned the wrong error: {error}")),
                Ok(_) => Err(format!("{operation} accepted a NULL persisted payload")),
            }
        }

        expect_null_payload_error(repo.load_block(file_id, 0, block_size), "load_block")?;
        expect_null_payload_error(
            repo.fetch_block_range_shared(file_id, 0, 0, block_size),
            "fetch_block_range",
        )?;
        expect_null_payload_error(
            repo.fetch_block_range_with_size_shared(file_id, 0, 0, block_size),
            "fetch_block_range_with_size",
        )?;
        expect_null_payload_error(
            repo.fetch_block_range_with_generation_shared(file_id, 0, 0, block_size),
            "fetch_block_range_with_generation",
        )?;
        Ok(())
    })();

    let cleanup = (|| {
        repo.purge_primary_file(file_id)?;
        repo.exec("ALTER TABLE data_blocks ALTER COLUMN data SET NOT NULL")
    })();
    result.and(cleanup)
}

#[test]
fn rename_replace_rejects_missing_target_data_object() -> Result<(), String> {
    use fod_rust_hotpath::pg::{RenameNamespaceSource, RenameNamespaceTarget};

    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let source_name = unique_name("missing_object_rename_source");
    let target_name = unique_name("missing_object_rename_target");
    let source_id = repo.create_file(
        None,
        &source_name,
        0o644,
        1000,
        1000,
        &unique_name("missing_object_source_seed"),
    )?;
    let target_id = repo.create_file(
        None,
        &target_name,
        0o644,
        1000,
        1000,
        &unique_name("missing_object_target_seed"),
    )?;
    let original_object_id = repo
        .query_scalar_text(&format!(
            "SELECT data_object_id::text FROM files WHERE id_file = {target_id}"
        ))?
        .parse::<u64>()
        .map_err(|err| format!("invalid target data object id: {err}"))?;
    let missing_object_id = repo
        .query_scalar_text("SELECT COALESCE(MAX(id_data_object), 0) + 1000000 FROM data_objects")?
        .parse::<u64>()
        .map_err(|err| format!("invalid missing data object id: {err}"))?;
    let mut file_foreign_key_dropped = false;

    let result = (|| {
        repo.exec("ALTER TABLE files DROP CONSTRAINT files_data_object_id_fkey")?;
        file_foreign_key_dropped = true;
        repo.exec(&format!(
            "UPDATE files SET data_object_id = {missing_object_id} WHERE id_file = {target_id}"
        ))?;

        let error = repo
            .rename_namespace_replace_with_ownership_guard(
                RenameNamespaceSource::File { file_id: source_id },
                None,
                &source_name,
                None,
                &target_name,
                Some(RenameNamespaceTarget::File { file_id: target_id }),
                1000,
            )
            .err()
            .ok_or_else(|| "rename replaced a target with a missing data object".to_string())?;
        if !error.contains("data-object reference count") {
            return Err(format!("unexpected missing-object error: {error}"));
        }

        if repo.resolve_path(&format!("/{target_name}"))?.entry_id != Some(target_id)
            || repo.resolve_path(&format!("/{source_name}"))?.entry_id != Some(source_id)
        {
            return Err("failed rename changed one of its namespace entries".to_string());
        }
        Ok(())
    })();

    let cleanup = (|| {
        if file_foreign_key_dropped {
            repo.exec(&format!(
                "UPDATE files SET data_object_id = {original_object_id} WHERE id_file = {target_id}"
            ))?;
            repo.exec(
                "ALTER TABLE files ADD CONSTRAINT files_data_object_id_fkey FOREIGN KEY (data_object_id) REFERENCES data_objects(id_data_object)",
            )?;
        }
        repo.purge_primary_file(target_id)?;
        repo.purge_primary_file(source_id)?;
        repo.exec(&format!(
            "DELETE FROM data_objects WHERE id_data_object = {original_object_id} AND NOT EXISTS (SELECT 1 FROM files WHERE data_object_id = {original_object_id})"
        ))
    })();
    result.and(cleanup)
}

#[test]
fn symlink_null_target_is_not_reported_as_missing_or_empty() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let name = unique_name("null_symlink_target");
    let symlink_id = repo.create_symlink(
        None,
        &name,
        "  valid-target  ",
        1000,
        1000,
        &unique_name("null_symlink_target_seed"),
    )?;
    let path = format!("/{name}");
    let mut target_not_null_dropped = false;

    let result = (|| {
        if repo.load_symlink_target(symlink_id)?.as_deref() != Some("  valid-target  ") {
            return Err("readlink did not preserve target whitespace".to_string());
        }
        repo.exec("ALTER TABLE symlinks ALTER COLUMN target DROP NOT NULL")?;
        target_not_null_dropped = true;
        repo.exec(&format!(
            "UPDATE symlinks SET target = NULL WHERE id_symlink = {symlink_id}"
        ))?;

        match repo.load_symlink_target(symlink_id) {
            Err(error) if error.contains("NULL") => {}
            Err(error) => return Err(format!("unexpected readlink target error: {error}")),
            Ok(target) => {
                return Err(format!(
                    "NULL symlink target was treated as an ordinary result: {target:?}"
                ));
            }
        }
        match repo.fetch_path_attrs_blob(&path) {
            Err(error) if error.contains("NULL") => {}
            Err(error) => return Err(format!("unexpected symlink metadata error: {error}")),
            Ok(attrs) => {
                return Err(format!(
                    "NULL symlink target produced an attribute blob: {}",
                    attrs.is_some()
                ));
            }
        }

        repo.exec(&format!(
            "UPDATE symlinks SET target = '' WHERE id_symlink = {symlink_id}"
        ))?;
        match repo.load_symlink_target(symlink_id) {
            Err(error) if error.contains("empty") => {}
            Err(error) => return Err(format!("unexpected empty-target error: {error}")),
            Ok(target) => {
                return Err(format!(
                    "empty symlink target was treated as an ordinary result: {target:?}"
                ));
            }
        }
        match repo.fetch_path_attrs_blob(&path) {
            Err(error) if error.contains("empty") => {}
            Err(error) => return Err(format!("unexpected empty-target metadata error: {error}")),
            Ok(attrs) => {
                return Err(format!(
                    "empty symlink target produced an attribute blob: {}",
                    attrs.is_some()
                ));
            }
        }
        Ok(())
    })();

    let cleanup = (|| {
        repo.delete_symlink_entry(symlink_id)?;
        if target_not_null_dropped {
            repo.exec("ALTER TABLE symlinks ALTER COLUMN target SET NOT NULL")?;
        }
        Ok(())
    })();
    result.and(cleanup)
}

#[test]
fn null_file_unlinked_flag_is_not_reported_as_missing() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let name = unique_name("null_file_unlinked");
    let file_id = repo.create_file(
        None,
        &name,
        0o644,
        1000,
        1000,
        &unique_name("null_file_unlinked_seed"),
    )?;
    let mut unlinked_not_null_dropped = false;

    let result = (|| {
        if repo.file_is_unlinked(file_id)? != Some(false) {
            return Err("new file did not report unlinked=false".to_string());
        }
        if repo.file_link_count_for_stat(file_id)? != Some(1) {
            return Err("new file did not report a single link".to_string());
        }
        repo.exec("ALTER TABLE files ALTER COLUMN unlinked DROP NOT NULL")?;
        unlinked_not_null_dropped = true;
        repo.exec(&format!(
            "UPDATE files SET unlinked = NULL WHERE id_file = {file_id}"
        ))?;
        match repo.file_is_unlinked(file_id) {
            Err(error) if error.contains("NULL") => {}
            Err(error) => return Err(format!("unexpected unlinked-state error: {error}")),
            Ok(value) => {
                return Err(format!(
                    "NULL unlinked flag was treated as an ordinary result: {value:?}"
                ));
            }
        }
        match repo.file_link_count_for_stat(file_id) {
            Err(error) if error.contains("unlinked") => {}
            Err(error) => return Err(format!("unexpected link-count error: {error}")),
            Ok(value) => {
                return Err(format!(
                    "NULL unlinked flag produced an ordinary link count: {value:?}"
                ));
            }
        }
        Ok(())
    })();

    let cleanup = (|| {
        if unlinked_not_null_dropped {
            repo.exec(&format!(
                "UPDATE files SET unlinked = FALSE WHERE id_file = {file_id}"
            ))?;
            repo.exec("ALTER TABLE files ALTER COLUMN unlinked SET NOT NULL")?;
        }
        repo.purge_primary_file(file_id)
    })();
    result.and(cleanup)?;
    if repo.file_is_unlinked(file_id)?.is_some() {
        return Err("purged file was still reported by file_is_unlinked".to_string());
    }
    if repo.file_link_count_for_stat(file_id)?.is_some() {
        return Err("purged file was still reported by file_link_count_for_stat".to_string());
    }
    Ok(())
}

#[test]
fn count_file_links_does_not_report_link_for_missing_file() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let file_id = repo.create_file(
        None,
        &unique_name("count_missing_links"),
        0o644,
        1000,
        1000,
        &unique_name("count_missing_links_seed"),
    )?;
    let links_before_purge = repo.count_file_links(file_id);
    let cleanup = repo.purge_primary_file(file_id);
    let links_before_purge = links_before_purge?;
    cleanup?;
    if links_before_purge != 1 {
        return Err("new file did not report one link".to_string());
    }
    match repo.count_file_links(file_id) {
        Err(_) => Ok(()),
        Ok(links) => Err(format!(
            "missing file was reported with {links} link(s)"
        )),
    }
}

#[test]
fn null_data_object_id_is_not_reported_as_missing_file() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let file_id = repo.create_file(
        None,
        &unique_name("null_data_object_id"),
        0o644,
        1000,
        1000,
        &unique_name("null_data_object_id_seed"),
    )?;
    let data_object_id = repo
        .file_data_object_id(file_id)?
        .ok_or_else(|| "new file has no data object".to_string())?;
    let mut not_null_dropped = false;

    let result = (|| {
        repo.exec("ALTER TABLE files ALTER COLUMN data_object_id DROP NOT NULL")?;
        not_null_dropped = true;
        repo.exec(&format!(
            "UPDATE files SET data_object_id = NULL WHERE id_file = {file_id}"
        ))?;
        match repo.file_data_object_id(file_id) {
            Err(error) if error.to_ascii_lowercase().contains("null") => {}
            Err(error) => return Err(format!("unexpected data-object error: {error}")),
            Ok(value) => {
                return Err(format!(
                    "NULL data_object_id was treated as an ordinary result: {value:?}"
                ));
            }
        }
        Ok(())
    })();

    let cleanup = (|| {
        if not_null_dropped {
            repo.exec(&format!(
                "UPDATE files SET data_object_id = {data_object_id} WHERE id_file = {file_id}"
            ))?;
            repo.exec("ALTER TABLE files ALTER COLUMN data_object_id SET NOT NULL")?;
        }
        repo.purge_primary_file(file_id)
    })();
    result.and(cleanup)?;
    if repo.file_data_object_id(file_id)?.is_some() {
        return Err("purged file still had a data object".to_string());
    }
    Ok(())
}

#[test]
fn sparse_seek_does_not_treat_missing_file_as_hole() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let file_id = repo.create_file(
        None,
        &unique_name("sparse_seek_missing"),
        0o644,
        1000,
        1000,
        &unique_name("sparse_seek_missing_seed"),
    )?;
    repo.purge_primary_file(file_id)?;
    match repo.sparse_seek_blockwise(file_id, 4096, 0, 4096, false) {
        Err(_) => Ok(()),
        Ok(value) => Err(format!(
            "missing file was treated as a valid sparse-file hole: {value:?}"
        )),
    }
}

#[test]
fn startup_snapshot_uses_single_connection_acquisition() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;

    // observability_snapshot() czyta lokalny stan puli i nie pozyskuje PGconn.
    // Dlatego delta acquisition_count izoluje wywolanie startup_snapshot().
    let before = repo.observability_snapshot()?.pool.acquisition_count;
    let snapshot = repo.startup_snapshot()?;
    let after = repo.observability_snapshot()?.pool.acquisition_count;

    let acquisitions = after.saturating_sub(before);
    if acquisitions != 1 {
        return Err(format!(
            "startup_snapshot should use one PostgreSQL connection acquisition, got {acquisitions}"
        ));
    }

    // Test nie wymusza konkretnego stanu schematu. Zarowno pusty, jak i
    // zainicjalizowany FOD maja byc odczytane w ramach jednego PGconn.
    let _ = snapshot;
    Ok(())
}

#[test]
fn startup_snapshot_matches_individual_live_queries() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;

    let snapshot = repo.startup_snapshot()?;
    let schema_is_initialized = repo.schema_is_initialized()?;
    let is_in_recovery = repo.is_in_recovery()?;

    if snapshot.schema_is_initialized != schema_is_initialized {
        return Err(format!(
            "startup snapshot schema state mismatch: snapshot={} helper={}",
            snapshot.schema_is_initialized, schema_is_initialized
        ));
    }
    if snapshot.is_in_recovery != is_in_recovery {
        return Err(format!(
            "startup snapshot recovery state mismatch: snapshot={} helper={}",
            snapshot.is_in_recovery, is_in_recovery
        ));
    }

    if !schema_is_initialized {
        if snapshot.block_size.is_some()
            || snapshot.schema_version.is_some()
            || snapshot.minimum_client_version.is_some()
            || snapshot.schema_upgrade_in_progress
        {
            return Err(
                "uninitialized startup snapshot unexpectedly contains schema values".to_string(),
            );
        }
        return Ok(());
    }

    let block_size = repo
        .query_config_value("block_size")?
        .ok_or_else(|| "missing config.block_size".to_string())?
        .trim()
        .parse::<u32>()
        .map_err(|err| format!("invalid config.block_size value in comparison test: {err}"))?;
    let schema_version = repo
        .schema_version()?
        .ok_or_else(|| "missing schema version in comparison test".to_string())?;
    let minimum_client_version = repo.query_scalar_text(
        "SELECT minimum_client_version FROM schema_version ORDER BY applied_at DESC LIMIT 1",
    )?;
    let schema_upgrade_in_progress = matches!(
        repo.query_scalar_text(
            "SELECT upgrade_in_progress::text FROM schema_version ORDER BY applied_at DESC LIMIT 1",
        )?
        .trim(),
        "t" | "true" | "1"
    );

    if snapshot.block_size != Some(block_size) {
        return Err(format!(
            "startup snapshot block_size mismatch: snapshot={:?} helper={block_size}",
            snapshot.block_size
        ));
    }
    if snapshot.schema_version != Some(schema_version) {
        return Err(format!(
            "startup snapshot schema_version mismatch: snapshot={:?} helper={schema_version}",
            snapshot.schema_version
        ));
    }
    if snapshot.minimum_client_version.as_deref() != Some(minimum_client_version.as_str()) {
        return Err(format!(
            "startup snapshot minimum_client_version mismatch: snapshot={:?} helper={minimum_client_version}",
            snapshot.minimum_client_version
        ));
    }
    if snapshot.schema_upgrade_in_progress != schema_upgrade_in_progress {
        return Err(format!(
            "startup snapshot upgrade state mismatch: snapshot={} helper={schema_upgrade_in_progress}",
            snapshot.schema_upgrade_in_progress
        ));
    }

    Ok(())
}

#[test]
fn client_session_registration_rechecks_schema_upgrade_gate() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    repo.exec("UPDATE schema_version SET upgrade_in_progress = TRUE")?;

    let registration = repo.register_client_session(
        "schema-upgrade-gate-test",
        "/mnt/fod-schema-upgrade-gate-test",
        "primary",
        "postgres",
        u64::from(std::process::id()),
        30,
    );
    let clear_gate = repo.exec("UPDATE schema_version SET upgrade_in_progress = FALSE");
    clear_gate?;

    match registration {
        Err(error) if error.contains("FOD schema upgrade is in progress or was interrupted") => {
            Ok(())
        }
        Err(error) => Err(format!(
            "client session registration failed for the wrong reason: {error}"
        )),
        Ok(session_id) => {
            let _ = repo.exec(&format!(
                "DELETE FROM client_sessions WHERE session_id = {session_id}"
            ));
            Err(format!(
                "client session {session_id} registered while schema upgrade gate was set"
            ))
        }
    }
}

#[test]
fn payload_capacity_reservations_serialize_across_repositories() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo_a = repo_with_runtime()?;
    let repo_b = repo_with_runtime()?;
    let original_limit = repo_a
        .query_config_value("max_fs_size_bytes")?
        .ok_or_else(|| "missing max_fs_size_bytes config".to_string())?;
    let used_bytes = repo_a
        .query_scalar_text(
            "SELECT (\
                (SELECT COUNT(*)::bigint FROM data_blocks) \
                    * (SELECT value FROM config WHERE key = 'block_size') \
             )::text",
        )?
        .trim()
        .parse::<u64>()
        .map_err(|_| "invalid payload usage".to_string())?;

    repo_a.exec("DELETE FROM payload_capacity_reservations")?;
    repo_a.exec(&format!(
        "UPDATE config SET value = {} WHERE key = 'max_fs_size_bytes'",
        used_bytes.saturating_add(4096)
    ))?;

    let result = (|| {
        let token_a = repo_a
            .reserve_payload_capacity(4096)?
            .ok_or_else(|| "expected first capacity reservation".to_string())?;
        let second = repo_b.reserve_payload_capacity(1);
        if !matches!(
            second,
            Err(ref err) if err.starts_with(STORAGE_QUOTA_EXCEEDED_PREFIX)
        ) {
            return Err(format!(
                "second repository should receive ENOSPC while reservation is active: {second:?}"
            ));
        }
        repo_a.release_payload_capacity_reservation(&token_a)?;

        let token_b = repo_b
            .reserve_payload_capacity(1)?
            .ok_or_else(|| "expected reservation after release".to_string())?;
        repo_b.release_payload_capacity_reservation(&token_b)?;
        Ok(())
    })();

    let cleanup_result = repo_a
        .exec("DELETE FROM payload_capacity_reservations")
        .and_then(|_| {
            repo_a.exec(&format!(
                "UPDATE config SET value = {} WHERE key = 'max_fs_size_bytes'",
                original_limit
            ))
        });
    result.and(cleanup_result)
}

#[test]
fn expired_payload_capacity_reservation_is_renewed_before_persistence() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let block_size = repo.startup_snapshot()?.block_size.unwrap_or(4096) as usize;
    verify_expired_reservation_renewal(&repo, block_size)
}

fn persist_reservation_payload(
    repo: &DbRepo,
    file_id: u64,
    block_size: usize,
    marker: u8,
    reservation_token: Option<&str>,
) -> Result<Vec<u8>, String> {
    let block_size_u64 = block_size as u64;
    let payload = repeated_block(marker, block_size);
    let rows = [PersistBlockRow {
        block_index: 0,
        data: &payload,
        used_len: block_size_u64,
    }];
    repo.persist_file_blocks_with_crc_flag_and_reservation(
        file_id,
        block_size_u64,
        block_size_u64,
        1,
        false,
        &rows,
        true,
        reservation_token,
    )?;
    Ok(payload)
}

fn verify_expired_reservation_renewal(repo: &DbRepo, block_size: usize) -> Result<(), String> {
    let block_size_u64 = block_size as u64;
    let original_limit = repo
        .query_config_value("max_fs_size_bytes")?
        .ok_or_else(|| "missing max_fs_size_bytes config".to_string())?;
    let used_bytes = persisted_payload_bytes(repo)?;
    let dir_id = repo.create_directory(
        None,
        &unique_name("rust_pg_reservation_renew_dir"),
        0o755,
        1000,
        1000,
        &unique_name("reservation_renew_dir_seed"),
    )?;
    let file_id = repo.create_file(
        Some(dir_id),
        &unique_name("rust_pg_reservation_renew_file"),
        0o644,
        1000,
        1000,
        &unique_name("reservation_renew_file_seed"),
    )?;

    repo.exec("DELETE FROM payload_capacity_reservations")?;
    repo.exec(&format!(
        "UPDATE config SET value = {} WHERE key = 'max_fs_size_bytes'",
        used_bytes.saturating_add(block_size_u64)
    ))?;

    let result = (|| {
        let token = repo
            .reserve_payload_capacity(block_size_u64)?
            .ok_or_else(|| "expected payload capacity reservation".to_string())?;
        repo.exec(&format!(
            "UPDATE payload_capacity_reservations \
             SET expires_at = NOW() - INTERVAL '1 second' \
             WHERE request_token = '{token}'"
        ))?;

        let payload = persist_reservation_payload(repo, file_id, block_size, b'R', Some(&token))?;

        let active_count = repo.query_scalar_text(&format!(
            "SELECT COUNT(*) FROM payload_capacity_reservations \
             WHERE request_token = '{token}' AND expires_at > NOW()"
        ))?;
        if active_count.trim() != "1" {
            return Err("expired payload reservation was not renewed".to_string());
        }
        assert_block_range_matches(repo, file_id, block_size_u64, &[(0, payload.as_slice())])?;
        repo.release_payload_capacity_reservation(&token)
    })();

    let cleanup_result = repo
        .exec("DELETE FROM payload_capacity_reservations")
        .and_then(|_| repo.purge_primary_file(file_id))
        .and_then(|_| repo.delete_directory_entry(dir_id))
        .and_then(|_| {
            repo.exec(&format!(
                "UPDATE config SET value = {} WHERE key = 'max_fs_size_bytes'",
                original_limit
            ))
        });
    result.and(cleanup_result)
}

#[test]
fn expired_payload_capacity_reservation_cannot_reclaim_committed_capacity() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let block_size = repo.startup_snapshot()?.block_size.unwrap_or(4096) as usize;
    verify_expired_reservation_rejection(&repo, block_size)
}

fn verify_expired_reservation_rejection(repo: &DbRepo, block_size: usize) -> Result<(), String> {
    let block_size_u64 = block_size as u64;
    let original_limit = repo
        .query_config_value("max_fs_size_bytes")?
        .ok_or_else(|| "missing max_fs_size_bytes config".to_string())?;
    let used_bytes = persisted_payload_bytes(repo)?;
    let dir_id = repo.create_directory(
        None,
        &unique_name("rust_pg_reservation_reclaim_dir"),
        0o755,
        1000,
        1000,
        &unique_name("reservation_reclaim_dir_seed"),
    )?;
    let reserved_file_id = repo.create_file(
        Some(dir_id),
        &unique_name("rust_pg_reservation_reclaim_reserved"),
        0o644,
        1000,
        1000,
        &unique_name("reservation_reclaim_reserved_seed"),
    )?;
    let competing_file_id = repo.create_file(
        Some(dir_id),
        &unique_name("rust_pg_reservation_reclaim_competing"),
        0o644,
        1000,
        1000,
        &unique_name("reservation_reclaim_competing_seed"),
    )?;

    repo.exec("DELETE FROM payload_capacity_reservations")?;
    repo.exec(&format!(
        "UPDATE config SET value = {} WHERE key = 'max_fs_size_bytes'",
        used_bytes.saturating_add(block_size_u64)
    ))?;

    let result = (|| {
        let token = repo
            .reserve_payload_capacity(block_size_u64)?
            .ok_or_else(|| "expected payload capacity reservation".to_string())?;
        repo.exec(&format!(
            "UPDATE payload_capacity_reservations \
             SET expires_at = NOW() - INTERVAL '1 second' \
             WHERE request_token = '{token}'"
        ))?;

        persist_reservation_payload(repo, competing_file_id, block_size, b'C', None)?;

        let rejected =
            persist_reservation_payload(repo, reserved_file_id, block_size, b'X', Some(&token));
        if !matches!(
            rejected,
            Err(ref err) if err.starts_with(STORAGE_QUOTA_EXCEEDED_PREFIX)
        ) {
            return Err(format!(
                "expired reservation should receive ENOSPC after capacity is committed: {rejected:?}"
            ));
        }

        let reserved_state = repo.query_scalar_text(&format!(
            "SELECT size::text || ':' || \
                (SELECT COUNT(*) FROM data_blocks \
                  WHERE data_object_id = files.data_object_id)::text \
             FROM files WHERE id_file = {reserved_file_id}"
        ))?;
        if reserved_state.trim() != "0:0" {
            return Err(format!(
                "rejected reservation changed destination state: {reserved_state}"
            ));
        }
        repo.release_payload_capacity_reservation(&token)
    })();

    let cleanup_result = repo
        .exec("DELETE FROM payload_capacity_reservations")
        .and_then(|_| repo.purge_primary_file(reserved_file_id))
        .and_then(|_| repo.purge_primary_file(competing_file_id))
        .and_then(|_| repo.delete_directory_entry(dir_id))
        .and_then(|_| {
            repo.exec(&format!(
                "UPDATE config SET value = {} WHERE key = 'max_fs_size_bytes'",
                original_limit
            ))
        });
    result.and(cleanup_result)
}

#[test]
fn full_block_rewrite_detaches_shared_object_and_preserves_hardlink() -> Result<(), String> {
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
    let new_rows = [
        PersistBlockRow {
            block_index: 0,
            data: &new_block0,
            used_len: block_size_u64,
        },
        PersistBlockRow {
            block_index: 1,
            data: &new_block1,
            used_len: block_size_u64,
        },
    ];
    repo.persist_file_blocks(
        dst_file_id,
        2 * block_size_u64,
        block_size_u64,
        2,
        false,
        &new_rows,
    )?;
    let new_data_object_id = repo
        .file_data_object_id(dst_file_id)?
        .ok_or_else(|| "missing replacement data object".to_string())?;

    if new_data_object_id == shared_data_object_id
        || repo.file_data_object_id(dst_file_id)? != Some(new_data_object_id)
        || repo.file_data_object_id(src_file_id)? != Some(shared_data_object_id)
    {
        return Err("full block rewrite did not detach the shared object".to_string());
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
        return Err(format!("expected two replacement CRC rows, got {crc_rows}"));
    }

    repo.delete_hardlink_entry(hardlink_id)?;
    repo.purge_primary_file(dst_file_id)?;
    repo.purge_primary_file(src_file_id)?;
    repo.delete_directory_entry(dir_id)?;
    Ok(())
}

#[test]
fn full_block_rewrites_follow_immediate_and_deferred_cleanup() -> Result<(), String> {
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
        let new_rows = [PersistBlockRow {
            block_index: 0,
            data: &new_payload,
            used_len: block_size_u64,
        }];
        repo.persist_file_blocks(file_id, block_size_u64, block_size_u64, 1, false, &new_rows)?;
        let new_data_object_id = repo
            .file_data_object_id(file_id)?
            .ok_or_else(|| "missing replacement data object".to_string())?;
        if new_data_object_id == old_data_object_id {
            return Err("full block rewrite reused the old data object".to_string());
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
                "DELETE FROM data_blocks WHERE data_object_id = {old_data_object_id}; DELETE FROM copy_block_crc WHERE data_object_id = {old_data_object_id}; DELETE FROM data_objects WHERE id_data_object = {old_data_object_id}"
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

fn persisted_payload_bytes(repo: &DbRepo) -> Result<u64, String> {
    repo.query_scalar_text(
        "SELECT (\
            (SELECT COUNT(*)::bigint FROM data_blocks) \
                * (SELECT value FROM config WHERE key = 'block_size') \
         )::text",
    )?
    .trim()
    .parse::<u64>()
    .map_err(|_| "invalid payload usage".to_string())
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
fn count_file_blocks_query_failure_is_not_zero() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let temporary_db = TemporaryDatabase::create()?;
    let mut runtime = RuntimeConfig::from_env()?;
    runtime.copy_dedupe_min_blocks = runtime.copy_dedupe_min_blocks.max(1);
    let repo = DbRepo::with_runtime(&conninfo_for_database(&temporary_db.name), &runtime)?;

    repo.exec("CREATE SCHEMA fod")?;
    repo.exec(
        "CREATE TABLE fod.files (id_file BIGINT PRIMARY KEY, data_object_id BIGINT NOT NULL)",
    )?;
    repo.exec("CREATE TABLE fod.data_blocks (data_object_id BIGINT NOT NULL)")?;
    repo.exec("INSERT INTO fod.files VALUES (17, 29)")?;
    repo.exec("INSERT INTO fod.data_blocks VALUES (29), (29)")?;

    assert_eq!(repo.count_file_blocks(17)?, 2);
    repo.exec(
        "ALTER TABLE fod.data_blocks RENAME COLUMN data_object_id TO data_object_id_fault_probe",
    )?;
    let error = repo
        .count_file_blocks(17)
        .expect_err("a failed block-count query must not be reported as zero");
    assert!(
        error.contains("SQLSTATE 42703"),
        "expected PostgreSQL undefined-column error with SQLSTATE 42703, got: {error}"
    );

    Ok(())
}

#[test]
fn directory_count_query_failures_are_not_empty() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let temporary_db = TemporaryDatabase::create()?;
    let mut runtime = RuntimeConfig::from_env()?;
    runtime.copy_dedupe_min_blocks = runtime.copy_dedupe_min_blocks.max(1);
    let repo = DbRepo::with_runtime(&conninfo_for_database(&temporary_db.name), &runtime)?;

    repo.exec("CREATE SCHEMA fod")?;
    repo.exec("CREATE TABLE fod.files (id_directory BIGINT, unlinked BOOLEAN NOT NULL)")?;
    repo.exec("CREATE TABLE fod.directories (id_parent BIGINT)")?;
    repo.exec("CREATE TABLE fod.hardlinks (id_directory BIGINT)")?;
    repo.exec("CREATE TABLE fod.symlinks (id_parent BIGINT)")?;
    assert!(!repo.path_has_children(41)?);
    assert_eq!(repo.count_directory_children(41)?, 0);

    repo.exec("INSERT INTO fod.files VALUES (41, FALSE)")?;
    assert!(repo.path_has_children(41)?);
    assert_eq!(repo.count_directory_children(41)?, 1);

    repo.exec("ALTER TABLE fod.files RENAME COLUMN unlinked TO unlinked_fault_probe")?;
    let has_children_error = repo
        .path_has_children(41)
        .expect_err("a failed child query must not report an empty directory");
    assert!(
        has_children_error.contains("SQLSTATE 42703"),
        "expected PostgreSQL undefined-column error, got: {has_children_error}"
    );
    let child_count_error = repo
        .count_directory_children(41)
        .expect_err("a failed child-count query must not report zero");
    assert!(
        child_count_error.contains("SQLSTATE 42703"),
        "expected PostgreSQL undefined-column error, got: {child_count_error}"
    );

    Ok(())
}

#[test]
fn directory_link_count_query_failures_are_not_zero() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let temporary_db = TemporaryDatabase::create()?;
    let mut runtime = RuntimeConfig::from_env()?;
    runtime.copy_dedupe_min_blocks = runtime.copy_dedupe_min_blocks.max(1);
    let repo = DbRepo::with_runtime(&conninfo_for_database(&temporary_db.name), &runtime)?;

    repo.exec("CREATE SCHEMA fod")?;
    repo.exec("CREATE TABLE fod.directories (id_parent BIGINT, name TEXT NOT NULL)")?;
    repo.exec(
        "INSERT INTO fod.directories VALUES (NULL, '/'), (NULL, 'top-level'), (41, 'child')",
    )?;

    assert_eq!(repo.count_directory_subdirs(41)?, 1);
    assert_eq!(repo.count_root_directory_children()?, 1);

    repo.exec("ALTER TABLE fod.directories RENAME COLUMN id_parent TO id_parent_fault_probe")?;
    let subdirectory_error = repo
        .count_directory_subdirs(41)
        .expect_err("a failed subdirectory query must not be reported as zero");
    assert!(
        subdirectory_error.contains("SQLSTATE 42703"),
        "expected PostgreSQL undefined-column error, got: {subdirectory_error}"
    );
    let root_child_error = repo
        .count_root_directory_children()
        .expect_err("a failed root child query must not be reported as zero");
    assert!(
        root_child_error.contains("SQLSTATE 42703"),
        "expected PostgreSQL undefined-column error, got: {root_child_error}"
    );

    Ok(())
}

#[test]
fn xattr_null_value_is_not_reported_as_missing_attribute() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let temporary_db = TemporaryDatabase::create()?;
    let mut runtime = RuntimeConfig::from_env()?;
    runtime.copy_dedupe_min_blocks = runtime.copy_dedupe_min_blocks.max(1);
    let repo = DbRepo::with_runtime(&conninfo_for_database(&temporary_db.name), &runtime)?;

    repo.exec("CREATE SCHEMA fod")?;
    repo.exec(
        "CREATE TABLE fod.xattrs (owner_kind TEXT NOT NULL, owner_id BIGINT NOT NULL, name TEXT NOT NULL, value BYTEA)",
    )?;
    repo.exec(
        "INSERT INTO fod.xattrs (owner_kind, owner_id, name, value) VALUES ('file', 17, 'user.corrupt', NULL)",
    )?;

    let error = repo
        .fetch_xattr_value_for_owner("file", 17, "user.corrupt")
        .expect_err("NULL xattr payload must be rejected, not reported as absent");
    if !error.contains("NULL") {
        return Err(format!("unexpected malformed xattr error: {error}"));
    }
    Ok(())
}

#[test]
fn xattr_null_or_empty_name_is_not_reported_as_valid_name() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let temporary_db = TemporaryDatabase::create()?;
    let mut runtime = RuntimeConfig::from_env()?;
    runtime.copy_dedupe_min_blocks = runtime.copy_dedupe_min_blocks.max(1);
    let repo = DbRepo::with_runtime(&conninfo_for_database(&temporary_db.name), &runtime)?;

    repo.exec("CREATE SCHEMA fod")?;
    repo.exec(
        "CREATE TABLE fod.xattrs (owner_kind TEXT NOT NULL, owner_id BIGINT NOT NULL, name TEXT, value BYTEA NOT NULL)",
    )?;
    repo.exec(
        "INSERT INTO fod.xattrs (owner_kind, owner_id, name, value) VALUES ('file', 17, NULL, '\\x00')",
    )?;

    let null_error = repo
        .list_xattr_names_for_owner("file", 17)
        .expect_err("NULL xattr name must be rejected");
    if !null_error.contains("NULL") {
        return Err(format!("unexpected NULL xattr-name error: {null_error}"));
    }

    repo.exec("UPDATE fod.xattrs SET name = ''")?;
    let empty_error = repo
        .list_xattr_names_for_owner("file", 17)
        .expect_err("empty xattr name must be rejected");
    if !empty_error.contains("empty name") {
        return Err(format!("unexpected empty xattr-name error: {empty_error}"));
    }
    Ok(())
}

#[test]
fn directory_entry_metadata_rejects_missing_required_fields() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let temporary_db = TemporaryDatabase::create()?;
    let mut runtime = RuntimeConfig::from_env()?;
    runtime.copy_dedupe_min_blocks = runtime.copy_dedupe_min_blocks.max(1);
    let repo = DbRepo::with_runtime(&conninfo_for_database(&temporary_db.name), &runtime)?;

    repo.exec("CREATE SCHEMA fod")?;
    repo.exec(
        "CREATE TABLE fod.directories (name TEXT, id_parent BIGINT, id_directory BIGINT, inode_seed TEXT)",
    )?;
    repo.exec(
        "CREATE TABLE fod.files (name TEXT, id_directory BIGINT, unlinked BOOLEAN, id_file BIGINT, inode_seed TEXT)",
    )?;
    repo.exec("CREATE TABLE fod.special_files (id_file BIGINT, file_type TEXT)")?;
    repo.exec(
        "CREATE TABLE fod.hardlinks (name TEXT, id_directory BIGINT, id_hardlink BIGINT, id_file BIGINT)",
    )?;
    repo.exec(
        "CREATE TABLE fod.symlinks (name TEXT, id_parent BIGINT, id_symlink BIGINT, inode_seed TEXT)",
    )?;

    repo.exec("INSERT INTO fod.files VALUES ('null-seed', NULL, FALSE, 17, NULL)")?;
    let null_seed_error = repo
        .list_directory_entry_metadata_for_id(None)
        .expect_err("NULL inode seed must be rejected");
    assert!(
        null_seed_error.contains("empty inode seed"),
        "{null_seed_error}"
    );

    repo.exec("TRUNCATE fod.files")?;
    repo.exec("INSERT INTO fod.files VALUES (NULL, NULL, FALSE, 18, 'valid-seed')")?;
    let null_name_error = repo
        .list_directory_entry_metadata_for_id(None)
        .expect_err("NULL entry name must be rejected");
    assert!(null_name_error.contains("empty name"), "{null_name_error}");
    let null_name_snapshot_error = repo
        .list_directory_entries_blob_for_id(None)
        .expect_err("NULL entry name must not become an empty readdir entry");
    assert!(
        null_name_snapshot_error.contains("empty name"),
        "{null_name_snapshot_error}"
    );

    Ok(())
}

#[test]
fn unlink_preserves_alias_and_rejects_stale_binding_and_unsafe_purge() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let name = unique_name("unlink_primary");
    let alias = unique_name("unlink_alias");
    let file_id = repo.create_file(None, &name, 0o644, 1000, 1000, &name)?;
    repo.create_hardlink(file_id, None, &alias, 1000, 1000)?;
    let purge = repo.purge_primary_file(file_id).unwrap_err();
    assert!(purge.contains("surviving hardlinks"), "{purge}");
    assert_eq!(
        repo.unlink_file_entry(file_id, ".fod-unlinked-test", None, &name, 1000)?,
        FileUnlinkOutcome::PromotedHardlink
    );
    assert_eq!(repo.get_file_id(&format!("/{alias}"))?, Some(file_id));
    let replacement = repo.create_file(None, &name, 0o644, 1000, 1000, &name)?;
    assert_ne!(replacement, file_id);
    assert_eq!(
        repo.unlink_file_entry(file_id, ".fod-unlinked-test", None, &name, 1000)?,
        FileUnlinkOutcome::SourceChanged
    );
    assert_eq!(repo.get_file_id(&format!("/{name}"))?, Some(replacement));
    assert_eq!(repo.get_file_id(&format!("/{alias}"))?, Some(file_id));
    repo.purge_primary_file(file_id)?;
    repo.purge_primary_file(replacement)
}

#[test]
fn unlinked_open_inode_rejects_new_hardlinks_and_purge_until_last_close() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    // Match writable FUSE startup: mkfs alone does not initialize runtime
    // session replay tokens and the complete session/lock schema.
    repo.ensure_client_session_schema()
        .map_err(|err| format!("initialize client session schema: {err}"))?;
    let name = unique_name("unlink_open_primary");
    let alias = unique_name("unlink_open_alias");
    let file_id = repo.create_file(None, &name, 0o644, 1000, 1000, &name)?;
    let session = repo
        .register_client_session(
            "unlink-test",
            &unique_name("mount"),
            "primary",
            "postgres_lease",
            u64::from(std::process::id()),
            60,
        )
        .map_err(|err| format!("register client session: {err}"))?;
    repo.register_file_open_lease(file_id, session, 1, 60)
        .map_err(|err| format!("register file open lease: {err}"))?;
    assert!(repo.has_active_file_open_leases(file_id)?);
    let result = (|| {
        assert_eq!(
            repo.unlink_file_entry(file_id, ".fod-unlinked-open-test", None, &name, 1000)
                .map_err(|err| format!("unlink leased file: {err}"))?,
            FileUnlinkOutcome::Deferred
        );
        assert!(repo
            .create_hardlink(file_id, None, &alias, 1000, 1000)
            .unwrap_err()
            .contains("FOD_LINK_SOURCE_UNAVAILABLE"));
        assert!(repo
            .purge_primary_file(file_id)
            .unwrap_err()
            .contains("active open leases"));
        Ok(())
    })();
    repo.release_file_open_lease(session, 1)
        .map_err(|err| format!("release file open lease: {err}"))?;
    assert!(!repo.has_active_file_open_leases(file_id)?);
    repo.purge_primary_file(file_id)
        .map_err(|err| format!("purge file after last close: {err}"))?;
    repo.exec(&format!(
        "DELETE FROM client_sessions WHERE session_id = {session}"
    ))?;
    result
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
fn root_hardlink_promotion_accepts_null_parent_and_rejects_malformed_name() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;

    let primary_name = unique_name("rust_pg_root_primary");
    let alias_name = unique_name("rust_pg_root_hardlink");
    let file_id = repo
        .create_file(
            None,
            &primary_name,
            0o644,
            1000,
            1000,
            &unique_name("root_primary_seed"),
        )
        .map_err(|err| format!("create root primary file: {err}"))?;
    let hardlink_id = repo
        .create_hardlink(file_id, None, &alias_name, 1000, 1000)
        .map_err(|err| format!("create root hardlink: {err}"))?;

    let selected = repo
        .choose_primary_hardlink(file_id)
        .map_err(|err| format!("choose root hardlink: {err}"))?;
    if selected != Some((hardlink_id, None, alias_name.clone())) {
        return Err(format!(
            "root hardlink with a NULL parent was not selected: {selected:?}"
        ));
    }
    if !repo
        .promote_hardlink_to_primary(file_id)
        .map_err(|err| format!("promote root hardlink: {err}"))?
    {
        return Err("expected root hardlink promotion to succeed".to_string());
    }
    if repo
        .get_file_id(&format!("/{alias_name}"))
        .map_err(|err| format!("lookup promoted root hardlink: {err}"))?
        != Some(file_id)
    {
        return Err("promoted root hardlink does not resolve to the original file".to_string());
    }
    if repo
        .get_file_id(&format!("/{primary_name}"))
        .map_err(|err| format!("lookup old root primary: {err}"))?
        .is_some()
    {
        return Err("old root primary path still exists after promotion".to_string());
    }

    let malformed_primary_name = unique_name("rust_pg_bad_root_primary");
    let malformed_alias_name = unique_name("rust_pg_bad_root_hardlink");
    let malformed_file_id = repo
        .create_file(
            None,
            &malformed_primary_name,
            0o644,
            1000,
            1000,
            &unique_name("bad_root_primary_seed"),
        )
        .map_err(|err| format!("create primary for malformed hardlink: {err}"))?;
    let malformed_hardlink_id = repo
        .create_hardlink(malformed_file_id, None, &malformed_alias_name, 1000, 1000)
        .map_err(|err| format!("create hardlink for malformed row: {err}"))?;
    repo.exec(&format!(
        "UPDATE hardlinks SET name = '' WHERE id_hardlink = {malformed_hardlink_id}"
    ))
    .map_err(|err| format!("inject malformed hardlink name: {err}"))?;

    let choose_error = repo
        .choose_primary_hardlink(malformed_file_id)
        .expect_err("empty persisted hardlink name must fail closed");
    if !choose_error.contains("hardlink row has an empty name") {
        return Err(format!(
            "unexpected malformed hardlink error: {choose_error}"
        ));
    }
    let promote_error = repo
        .promote_hardlink_to_primary(malformed_file_id)
        .expect_err("promotion must not treat a malformed row as no hardlink");
    if !promote_error.contains("hardlink row has an empty name") {
        return Err(format!("unexpected promotion error: {promote_error}"));
    }
    if repo
        .get_file_id(&format!("/{malformed_primary_name}"))
        .map_err(|err| format!("verify malformed row did not replace primary: {err}"))?
        != Some(malformed_file_id)
    {
        return Err("malformed hardlink row changed the primary namespace".to_string());
    }

    repo.delete_hardlink_entry(malformed_hardlink_id)
        .map_err(|err| format!("delete malformed test hardlink: {err}"))?;
    repo.purge_primary_file(malformed_file_id)
        .map_err(|err| format!("purge malformed test file: {err}"))?;
    repo.purge_primary_file(file_id)
        .map_err(|err| format!("purge promoted test file: {err}"))?;

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
fn source_adoption_rejects_malformed_file_info_instead_of_falling_back() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;

    let source_name = unique_name("rust_pg_bad_adoption_source");
    let destination_name = unique_name("rust_pg_bad_adoption_destination");
    let source_file_id = repo.create_file(
        None,
        &source_name,
        0o644,
        1000,
        1000,
        &unique_name("bad_adoption_source_seed"),
    )?;
    let destination_file_id = repo.create_file(
        None,
        &destination_name,
        0o644,
        1000,
        1000,
        &unique_name("bad_adoption_destination_seed"),
    )?;
    let destination_object_id = repo
        .file_data_object_id(destination_file_id)?
        .ok_or_else(|| "destination is missing its data object".to_string())?;

    repo.exec(&format!(
        "UPDATE files SET size = -1 WHERE id_file = {source_file_id}"
    ))?;
    let error = repo
        .adopt_source_data_object(source_file_id, destination_file_id)
        .expect_err("negative persisted file size must fail adoption");
    if !error.contains("invalid file size") {
        return Err(format!("unexpected adoption error: {error}"));
    }
    if repo.file_data_object_id(destination_file_id)? != Some(destination_object_id) {
        return Err("malformed source metadata changed destination data object".to_string());
    }
    if repo.query_scalar_text(&format!(
        "SELECT size FROM files WHERE id_file = {destination_file_id}"
    ))? != "0"
    {
        return Err("malformed source metadata changed destination size".to_string());
    }

    repo.exec(&format!(
        "UPDATE files SET size = 0 WHERE id_file = {source_file_id}"
    ))?;
    repo.purge_primary_file(source_file_id)?;
    repo.purge_primary_file(destination_file_id)?;
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

#[test]
fn namespace_creation_rejects_every_cross_type_collision() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let fixture = NamespaceFixture::new()?;
    let repo = &fixture.repo;
    let create = |kind, parent, name: &str| {
        let seed = unique_name("cross_type_seed");
        match kind {
            0 => repo.create_directory(parent, name, 0o755, 1000, 1000, &seed),
            1 => repo.create_symlink(parent, name, "missing", 1000, 1000, &seed),
            2 => repo.create_hardlink(fixture.file_id.unwrap(), parent, name, 1000, 1000),
            3 => repo.create_file(parent, name, 0o644, 1000, 1000, &seed),
            _ => repo.create_special_file(parent, name, 0o600, 1000, 1000, &seed, "fifo", 0, 0),
        }
    };
    for parent in [None, Some(fixture.directory_id)] {
        for winner in 0..5 {
            for loser in 0..5 {
                if winner == loser {
                    continue;
                }
                let name = unique_name("cross_type_name");
                let id = create(winner, parent, &name)?;
                let result = create(loser, parent, &name);
                let cleanup = match winner {
                    0 => repo.delete_directory_entry(id),
                    1 => repo.delete_symlink_entry(id),
                    2 => repo.delete_hardlink_entry(id),
                    _ => repo.purge_primary_file(id),
                };
                cleanup?;
                assert!(
                    result.as_ref().err().is_some_and(|error| error
                        .starts_with(fod_rust_hotpath::pg::NAMESPACE_ENTRY_EXISTS_PREFIX)),
                    "winner={winner} loser={loser} parent={parent:?}: {result:?}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn namespace_creators_serialize_on_the_same_postgres_lock() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let fixture = NamespaceFixture::new()?;
    let gate = repo_with_runtime()?;
    let directory_repo = repo_with_runtime()?;
    let symlink_repo = repo_with_runtime()?;
    let parent = fixture.directory_id;
    let name = unique_name("namespace_race");
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let key = format!("fod:write:destination:{parent}:{}:{name}", name.len());
    let gate_thread = std::thread::spawn(move || {
        gate.with_transaction(|tx| {
            tx.query_scalar_text(&format!(
                "SELECT pg_advisory_xact_lock(hashtextextended('{key}', 0))"
            ))?;
            ready_tx.send(()).map_err(|err| err.to_string())?;
            release_rx
                .recv_timeout(std::time::Duration::from_secs(30))
                .map_err(|err| err.to_string())?;
            Ok(())
        })
    });
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .map_err(|err| err.to_string())?;
    let directory_name = name.clone();
    let directory_thread = std::thread::spawn(move || {
        directory_repo.create_directory(
            Some(parent),
            &directory_name,
            0o755,
            1000,
            1000,
            &unique_name("race_dir_seed"),
        )
    });
    let symlink_name = name.clone();
    let symlink_thread = std::thread::spawn(move || {
        symlink_repo.create_symlink(
            Some(parent),
            &symlink_name,
            "missing",
            1000,
            1000,
            &unique_name("race_link_seed"),
        )
    });
    let observed_wait = (|| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let count = fixture.repo.query_scalar_text("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND wait_event = 'advisory'")?;
            if count.trim() == "2" {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(
                    "both namespace creators did not wait for the shared advisory lock".to_string(),
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    })();
    release_tx.send(()).map_err(|err| err.to_string())?;
    gate_thread
        .join()
        .map_err(|_| "gate panicked".to_string())??;
    let directory = directory_thread
        .join()
        .map_err(|_| "directory creator panicked".to_string())?;
    let symlink = symlink_thread
        .join()
        .map_err(|_| "symlink creator panicked".to_string())?;
    if let Ok(id) = directory.as_ref() {
        fixture.repo.delete_directory_entry(*id)?;
    }
    if let Ok(id) = symlink.as_ref() {
        fixture.repo.delete_symlink_entry(*id)?;
    }
    observed_wait?;
    assert_ne!(
        directory.is_ok(),
        symlink.is_ok(),
        "exactly one creator must win: {directory:?}, {symlink:?}"
    );
    let error = directory.err().or_else(|| symlink.err()).unwrap();
    assert!(
        error.starts_with(fod_rust_hotpath::pg::NAMESPACE_ENTRY_EXISTS_PREFIX),
        "{error}"
    );
    Ok(())
}

#[test]
fn namespace_creation_rejects_missing_parent_and_special_file_replay() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let fixture = NamespaceFixture::new()?;
    let repo = &fixture.repo;
    let name = unique_name("fifo_replay_guard");
    let seed = unique_name("fifo_replay_seed");
    let id = repo.create_special_file(
        Some(fixture.directory_id),
        &name,
        0o600,
        1000,
        1000,
        &seed,
        "fifo",
        0,
        0,
    )?;
    // Identical file metadata cannot confirm a regular-file replay over a FIFO.
    let result = repo.create_file(Some(fixture.directory_id), &name, 0o600, 1000, 1000, &seed);
    repo.purge_primary_file(id)?;
    assert!(result
        .unwrap_err()
        .starts_with(fod_rust_hotpath::pg::NAMESPACE_ENTRY_EXISTS_PREFIX));
    let parent = repo.create_directory(
        Some(fixture.directory_id),
        "removed-parent",
        0o755,
        1000,
        1000,
        &unique_name("removed_parent_seed"),
    )?;
    repo.delete_directory_entry(parent)?;
    let result = repo.create_directory(
        Some(parent),
        "child",
        0o755,
        1000,
        1000,
        &unique_name("missing_parent_child"),
    );
    assert!(result
        .unwrap_err()
        .starts_with(fod_rust_hotpath::pg::NAMESPACE_PARENT_MISSING_PREFIX));
    Ok(())
}

#[test]
fn create_entries_check_locked_parent_write_and_search_permissions() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let parent_name = unique_name("mkdir_parent_access");
    let parent_id = repo.create_directory(
        None,
        &parent_name,
        0o700,
        1000,
        3000,
        &unique_name("mkdir_parent_access_seed"),
    )?;

    let denied_rmdir_name = unique_name("rmdir_parent_access_denied");
    let denied_rmdir_id = repo.create_directory(
        Some(parent_id),
        &denied_rmdir_name,
        0o700,
        1000,
        3000,
        &unique_name("rmdir_parent_access_denied_seed"),
    )?;
    assert_eq!(
        repo.unlink_directory_entry_for_subject(
            denied_rmdir_id,
            Some(parent_id),
            &denied_rmdir_name,
            2000,
            2000,
            &[2000],
        )
        .unwrap_err(),
        fod_rust_hotpath::pg::PARENT_ACCESS_DENIED
    );

    let denied = repo.create_directory_for_subject(
        Some(parent_id),
        "denied-child",
        0o700,
        2000,
        2000,
        &unique_name("mkdir_denied_child_seed"),
        &[2000],
    );
    assert_eq!(
        denied.unwrap_err(),
        fod_rust_hotpath::pg::PARENT_ACCESS_DENIED
    );
    assert_eq!(repo.count_directory_children(parent_id)?, 1);
    let unlink_file_name = unique_name("unlink_parent_access_file");
    let unlink_file = repo.create_file(
        Some(parent_id),
        &unlink_file_name,
        0o600,
        1000,
        3000,
        &unique_name("unlink_parent_access_file_seed"),
    )?;
    let denied_file_unlink = repo.unlink_file_entry_for_subject(
        unlink_file,
        &unique_name("unlink_hidden_denied"),
        Some(parent_id),
        &unlink_file_name,
        2000,
        2000,
        &[2000],
    );
    assert_eq!(
        denied_file_unlink.unwrap_err(),
        fod_rust_hotpath::pg::PARENT_ACCESS_DENIED
    );
    assert_eq!(repo.count_file_links(unlink_file)?, 1);

    let unlink_symlink_name = unique_name("unlink_parent_access_symlink");
    let unlink_symlink = repo.create_symlink(
        Some(parent_id),
        &unlink_symlink_name,
        "target",
        1000,
        3000,
        &unique_name("unlink_parent_access_symlink_seed"),
    )?;
    let files_before = repo.count_files()?;
    let denied_symlink_unlink = repo.unlink_symlink_entry_for_subject(
        unlink_symlink,
        Some(parent_id),
        &unlink_symlink_name,
        2000,
        2000,
        &[2000],
    );
    assert_eq!(
        denied_symlink_unlink.unwrap_err(),
        fod_rust_hotpath::pg::PARENT_ACCESS_DENIED
    );

    let denied_file = repo.create_file_for_subject(
        Some(parent_id),
        "denied-file",
        0o600,
        2000,
        2000,
        &unique_name("create_denied_file_seed"),
        &[2000],
    );
    assert_eq!(
        denied_file.unwrap_err(),
        fod_rust_hotpath::pg::PARENT_ACCESS_DENIED
    );
    assert_eq!(repo.count_files()?, files_before);

    let hardlink_source = repo.create_file(
        None,
        &unique_name("hardlink_parent_access_source"),
        0o600,
        2000,
        2000,
        &unique_name("hardlink_parent_access_source_seed"),
    )?;
    let files_after_hardlink_source = repo.count_files()?;
    let hardlinks_before = repo.count_file_links(hardlink_source)?;
    let denied_hardlink = repo.create_hardlink_for_subject(
        hardlink_source,
        Some(parent_id),
        "denied-hardlink",
        2000,
        2000,
        &[2000],
    );
    assert_eq!(
        denied_hardlink.unwrap_err(),
        fod_rust_hotpath::pg::PARENT_ACCESS_DENIED
    );
    assert_eq!(repo.count_file_links(hardlink_source)?, hardlinks_before);

    let symlinks_before = repo.count_symlinks()?;
    let denied_symlink = repo.create_symlink_for_subject(
        Some(parent_id),
        "denied-symlink",
        "target",
        2000,
        2000,
        &unique_name("symlink_denied_seed"),
        &[2000],
    );
    assert_eq!(
        denied_symlink.unwrap_err(),
        fod_rust_hotpath::pg::PARENT_ACCESS_DENIED
    );
    assert_eq!(repo.count_symlinks()?, symlinks_before);

    let denied_special = repo.create_special_file_for_subject(
        Some(parent_id),
        "denied-fifo",
        0o600,
        2000,
        2000,
        &unique_name("mknod_denied_fifo_seed"),
        "fifo",
        0,
        0,
        &[2000],
    );
    assert_eq!(
        denied_special.unwrap_err(),
        fod_rust_hotpath::pg::PARENT_ACCESS_DENIED
    );
    assert_eq!(repo.count_files()?, files_after_hardlink_source);

    let rename_source_name = unique_name("rename_parent_access_source");
    let rename_source_id = repo.create_file(
        None,
        &rename_source_name,
        0o600,
        2000,
        2000,
        &unique_name("rename_parent_access_source_seed"),
    )?;
    let rename_target_name = unique_name("rename_parent_access_target");
    let denied_rename = repo.rename_namespace_replace_with_ownership_guard_for_subject(
        fod_rust_hotpath::pg::RenameNamespaceSource::File {
            file_id: rename_source_id,
        },
        None,
        &rename_source_name,
        Some(parent_id),
        &rename_target_name,
        None,
        2000,
        2000,
        &[2000],
    );
    assert_eq!(
        denied_rename.unwrap_err(),
        fod_rust_hotpath::pg::PARENT_ACCESS_DENIED
    );

    repo.update_directory_mode(parent_id, "730")?;
    assert_eq!(
        repo.unlink_directory_entry_for_subject(
            denied_rmdir_id,
            Some(parent_id),
            &denied_rmdir_name,
            2000,
            2000,
            &[2000, 3000],
        )?,
        fod_rust_hotpath::pg::DirectoryUnlinkOutcome::Removed
    );
    repo.unlink_symlink_entry_for_subject(
        unlink_symlink,
        Some(parent_id),
        &unlink_symlink_name,
        2000,
        2000,
        &[2000, 3000],
    )?;
    repo.unlink_file_entry_for_subject(
        unlink_file,
        &unique_name("unlink_hidden_allowed"),
        Some(parent_id),
        &unlink_file_name,
        2000,
        2000,
        &[2000, 3000],
    )?;
    assert_eq!(
        repo.rename_namespace_replace_with_ownership_guard_for_subject(
            fod_rust_hotpath::pg::RenameNamespaceSource::File {
                file_id: rename_source_id,
            },
            None,
            &rename_source_name,
            Some(parent_id),
            &rename_target_name,
            None,
            2000,
            2000,
            &[2000, 3000],
        )?,
        fod_rust_hotpath::pg::RenameFileReplaceOutcome::Applied
    );
    let allowed_child = repo.create_directory_for_subject(
        Some(parent_id),
        "supplementary-group-child",
        0o700,
        2000,
        2000,
        &unique_name("mkdir_allowed_child_seed"),
        &[2000, 3000],
    )?;
    assert_eq!(repo.count_directory_children(parent_id)?, 2);
    let allowed_file = repo.create_file_for_subject(
        Some(parent_id),
        "supplementary-group-file",
        0o600,
        2000,
        2000,
        &unique_name("create_allowed_file_seed"),
        &[2000, 3000],
    )?;
    let allowed_special = repo.create_special_file_for_subject(
        Some(parent_id),
        "supplementary-group-fifo",
        0o600,
        2000,
        2000,
        &unique_name("mknod_allowed_fifo_seed"),
        "fifo",
        0,
        0,
        &[2000, 3000],
    )?;
    assert!(allowed_special > 0);
    repo.purge_primary_file(allowed_special)?;
    let allowed_symlink = repo.create_symlink_for_subject(
        Some(parent_id),
        "supplementary-group-symlink",
        "target",
        2000,
        2000,
        &unique_name("symlink_allowed_seed"),
        &[2000, 3000],
    )?;
    repo.delete_symlink_entry(allowed_symlink)?;
    let allowed_hardlink = repo.create_hardlink_for_subject(
        hardlink_source,
        Some(parent_id),
        "supplementary-group-hardlink",
        2000,
        2000,
        &[2000, 3000],
    )?;
    repo.delete_hardlink_entry(allowed_hardlink)?;
    repo.purge_primary_file(hardlink_source)?;
    repo.purge_primary_file(unlink_file)?;
    repo.purge_primary_file(allowed_file)?;
    repo.purge_primary_file(rename_source_id)?;
    repo.delete_directory_entry(allowed_child)?;
    repo.delete_directory_entry(parent_id)?;
    Ok(())
}

#[test]
fn access_acl_xattr_and_mode_are_updated_together() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    let repo = repo_with_runtime()?;
    let name = unique_name("access_acl_atomic");
    let file_id = repo.create_file(
        None,
        &name,
        0o6755,
        1000,
        1000,
        &unique_name("access_acl_atomic_seed"),
    )?;
    let acl = b"valid-acl-payload";

    assert_eq!(
        repo.store_posix_acl_for_owner(
            "file",
            file_id,
            "system.posix_acl_access",
            acl,
            Some(0o640),
            false,
            false,
            2000,
        )
        .unwrap_err(),
        "FOD_XATTR_PERMISSION_DENIED"
    );
    assert_eq!(
        repo.fetch_xattr_value_for_owner("file", file_id, "system.posix_acl_access")?,
        None
    );

    assert_eq!(
        repo.store_posix_acl_for_owner(
            "file",
            file_id,
            "system.posix_acl_access",
            acl,
            Some(0o640),
            false,
            true,
            1000,
        )
        .unwrap_err(),
        "FOD_XATTR_MISSING"
    );
    assert_eq!(
        repo.get_file_mode_value(&format!("/{name}"))?.as_deref(),
        Some("6755")
    );
    repo.store_posix_acl_for_owner(
        "file",
        file_id,
        "system.posix_acl_access",
        acl,
        Some(0o640),
        true,
        false,
        1000,
    )?;

    assert_eq!(
        repo.fetch_xattr_value_for_owner("file", file_id, "system.posix_acl_access")?,
        Some(acl.to_vec())
    );
    assert_eq!(
        repo.get_file_mode_value(&format!("/{name}"))?.as_deref(),
        Some("6640")
    );
    assert_eq!(
        repo.store_posix_acl_for_owner(
            "file",
            file_id,
            "system.posix_acl_access",
            b"conflicting-acl",
            Some(0o600),
            true,
            false,
            1000,
        )
        .unwrap_err(),
        "FOD_XATTR_EXISTS"
    );
    assert_eq!(
        repo.get_file_mode_value(&format!("/{name}"))?.as_deref(),
        Some("6640")
    );
    repo.store_posix_acl_for_owner(
        "file",
        file_id,
        "system.posix_acl_access",
        b"replacement-acl",
        Some(0o600),
        false,
        true,
        1000,
    )?;
    assert_eq!(
        repo.get_file_mode_value(&format!("/{name}"))?.as_deref(),
        Some("6600")
    );

    let directory_name = unique_name("default_acl_owner");
    let directory_id = repo.create_directory(
        None,
        &directory_name,
        0o750,
        1000,
        1000,
        &unique_name("default_acl_owner_seed"),
    )?;
    assert_eq!(
        repo.store_posix_acl_for_owner(
            "dir",
            directory_id,
            "system.posix_acl_default",
            acl,
            None,
            false,
            false,
            2000,
        )
        .unwrap_err(),
        "FOD_XATTR_PERMISSION_DENIED"
    );
    repo.store_posix_acl_for_owner(
        "dir",
        directory_id,
        "system.posix_acl_default",
        acl,
        None,
        false,
        false,
        1000,
    )?;
    assert_eq!(
        repo.fetch_xattr_value_for_owner("dir", directory_id, "system.posix_acl_default")?,
        Some(acl.to_vec())
    );
    repo.delete_owner_xattrs("dir", directory_id)?;
    repo.delete_directory_entry(directory_id)?;
    repo.purge_primary_file(file_id)?;
    Ok(())
}

#[test]
fn inode_permission_changes_authorize_locked_owner_and_preserve_fields() -> Result<(), String> {
    use fod_rust_hotpath::pg::{InodePermissionChange, InodePermissionOutcome};
    let _guard = ENV_LOCK.lock().unwrap();
    let fixture = NamespaceFixture::new()?;
    let repo = &fixture.repo;
    for (kind, table, key, id) in [
        ("file", "files", "id_file", fixture.file_id.unwrap()),
        ("dir", "directories", "id_directory", fixture.directory_id),
        (
            "symlink",
            "symlinks",
            "id_symlink",
            fixture.symlink_id.unwrap(),
        ),
    ] {
        let seed =
            repo.query_scalar_text(&format!("SELECT inode_seed FROM {table} WHERE {key}={id}"))?;
        let mut change = InodePermissionChange {
            actor_uid: 1000,
            actor_groups: &[1000, 2000],
            mode: Some(0o6755),
            uid: None,
            gid: Some(2000),
        };
        assert_eq!(
            repo.change_inode_permissions(kind, id, "wrong-seed", &change)?,
            InodePermissionOutcome::Missing
        );
        assert_eq!(
            repo.change_inode_permissions(kind, id, &seed, &change)?,
            InodePermissionOutcome::Changed
        );
        assert_eq!(
            repo.query_rows_text(&format!("SELECT uid,gid FROM {table} WHERE {key}={id}"))?,
            vec![vec!["1000".to_string(), "2000".to_string()]]
        );
        if kind != "symlink" {
            let mode =
                repo.query_scalar_text(&format!("SELECT mode FROM {table} WHERE {key}={id}"))?;
            assert_eq!(mode, if kind == "file" { "755" } else { "6755" });
        }
        change.actor_uid = 0;
        change.uid = Some(3000);
        change.mode = None;
        change.gid = None;
        assert_eq!(
            repo.change_inode_permissions(kind, id, &seed, &change)?,
            InodePermissionOutcome::Changed
        );
        change.actor_uid = 1000;
        change.uid = None;
        change.mode = Some(0o777);
        assert_eq!(
            repo.change_inode_permissions(kind, id, &seed, &change)?,
            InodePermissionOutcome::Denied
        );
        change.actor_uid = 3000;
        change.uid = Some(4000);
        // A rejected combined chmod/chown must not partially change mode or owner.
        let before = repo.query_rows_text(&format!("SELECT * FROM {table} WHERE {key}={id}"))?;
        assert_eq!(
            repo.change_inode_permissions(kind, id, &seed, &change)?,
            InodePermissionOutcome::Denied
        );
        assert_eq!(
            repo.query_rows_text(&format!("SELECT * FROM {table} WHERE {key}={id}"))?,
            before
        );
    }
    Ok(())
}

#[test]
fn chmod_waits_for_concurrent_chown_and_rejects_previous_owner() -> Result<(), String> {
    use fod_rust_hotpath::pg::{InodePermissionChange, InodePermissionOutcome};
    let _guard = ENV_LOCK.lock().unwrap();
    let fixture = NamespaceFixture::new()?;
    let file_id = fixture.file_id.unwrap();
    let seed = fixture.repo.query_scalar_text(&format!(
        "SELECT inode_seed FROM files WHERE id_file={file_id}"
    ))?;
    let writer = repo_with_runtime()?;
    let changer = repo_with_runtime()?;
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let owner_thread = std::thread::spawn(move || {
        writer.with_transaction(|tx| {
            tx.exec(&format!(
                "UPDATE files SET uid=2000 WHERE id_file={file_id}"
            ))?;
            ready_tx.send(()).map_err(|err| err.to_string())?;
            release_rx
                .recv_timeout(std::time::Duration::from_secs(30))
                .map_err(|err| err.to_string())?;
            Ok(())
        })
    });
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .map_err(|err| err.to_string())?;
    let chmod_thread = std::thread::spawn(move || {
        changer.change_inode_permissions(
            "file",
            file_id,
            &seed,
            &InodePermissionChange {
                actor_uid: 1000,
                actor_groups: &[1000],
                mode: Some(0o777),
                uid: None,
                gid: None,
            },
        )
    });
    let observed_wait = (|| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let count = fixture.repo.query_scalar_text("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'SELECT uid, gid,%FOR UPDATE'")?;
            if count.trim() != "0" {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err("chmod did not wait for concurrent chown".to_string());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    })();
    release_tx.send(()).map_err(|err| err.to_string())?;
    owner_thread
        .join()
        .map_err(|_| "owner thread panicked".to_string())??;
    let outcome = chmod_thread
        .join()
        .map_err(|_| "chmod thread panicked".to_string())??;
    observed_wait?;
    assert_eq!(outcome, InodePermissionOutcome::Denied);
    assert_eq!(
        fixture
            .repo
            .query_scalar_text(&format!("SELECT mode FROM files WHERE id_file={file_id}"))?,
        "644"
    );
    Ok(())
}

fn sticky_fixture() -> Result<NamespaceFixture, String> {
    let mut fixture = NamespaceFixture::new()?;
    fixture.child_directory_id = Some(fixture.repo.create_directory(
        Some(fixture.directory_id),
        "child",
        0o755,
        1000,
        1000,
        &unique_name("sticky_child"),
    )?);
    fixture.repo.exec(&format!(
        "UPDATE hardlinks SET uid=3000 WHERE id_hardlink={}",
        fixture.hardlink_id.unwrap()
    ))?;
    fixture.repo.ensure_client_session_schema()?;
    Ok(fixture)
}

fn sticky_remove(
    repo: &DbRepo,
    operation: usize,
    parent: u64,
    file: u64,
    symlink: u64,
    child: u64,
    actor: u32,
) -> Result<(), String> {
    use fod_rust_hotpath::pg::{RenameFileReplaceOutcome, RenameNamespaceSource};
    match operation {
        0 => repo
            .unlink_file_entry(file, ".sticky-hidden", Some(parent), "file", actor)
            .map(|outcome| assert_eq!(outcome, FileUnlinkOutcome::PromotedHardlink)),
        1 => repo
            .unlink_file_entry(file, ".sticky-hidden", Some(parent), "hardlink", actor)
            .map(|outcome| assert_eq!(outcome, FileUnlinkOutcome::RemovedHardlink)),
        2 => repo.unlink_symlink_entry(symlink, Some(parent), "symlink", actor),
        3 => repo
            .unlink_directory_entry(child, Some(parent), "child", actor)
            .map(|outcome| assert_eq!(outcome, DirectoryUnlinkOutcome::Removed)),
        _ => repo
            .rename_namespace_replace_with_ownership_guard(
                RenameNamespaceSource::File { file_id: file },
                Some(parent),
                "file",
                Some(parent),
                "renamed",
                None,
                actor,
            )
            .map(|outcome| assert_eq!(outcome, RenameFileReplaceOutcome::Applied)),
    }
}

#[test]
fn sticky_removal_checks_all_kinds_and_parent_owner_exemption() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    for operation in 0..5 {
        for allowed_actor in [0, 1000, 2000] {
            let fixture = sticky_fixture()?;
            let parent = fixture.directory_id;
            fixture.repo.exec(&format!(
                "UPDATE directories SET uid=2000,mode='1777' WHERE id_directory={parent}"
            ))?;
            let file = fixture.file_id.unwrap();
            let symlink = fixture.symlink_id.unwrap();
            let child = fixture.child_directory_id.unwrap();
            assert_eq!(
                sticky_remove(&fixture.repo, operation, parent, file, symlink, child, 3000)
                    .unwrap_err(),
                fod_rust_hotpath::pg::STICKY_PERMISSION_DENIED
            );
            if operation == 2 {
                assert_eq!(
                    fixture
                        .repo
                        .unlink_symlink_entry(symlink, Some(parent), "wrong-binding", allowed_actor)
                        .unwrap_err(),
                    fod_rust_hotpath::pg::NAMESPACE_SOURCE_CHANGED
                );
            }
            // Root, victim owner and directory owner are all valid exemptions.
            sticky_remove(
                &fixture.repo,
                operation,
                parent,
                file,
                symlink,
                child,
                allowed_actor,
            )?;
        }
    }
    Ok(())
}

#[test]
fn sticky_rename_checks_replaced_target_owner_for_every_kind() -> Result<(), String> {
    use fod_rust_hotpath::pg::{RenameNamespaceSource as Source, RenameNamespaceTarget as Target};
    let _guard = ENV_LOCK.lock().unwrap();
    for kind in 0..4 {
        let source = sticky_fixture()?;
        let target = sticky_fixture()?;
        let parent = target.directory_id;
        target.repo.exec(&format!(
            "UPDATE directories SET uid=2000,mode='1777' WHERE id_directory={parent}"
        ))?;
        target.repo.exec(&format!(
            "UPDATE files SET uid=3000 WHERE id_file={}",
            target.file_id.unwrap()
        ))?;
        target.repo.exec(&format!(
            "UPDATE symlinks SET uid=3000 WHERE id_symlink={}",
            target.symlink_id.unwrap()
        ))?;
        target.repo.exec(&format!(
            "UPDATE directories SET uid=3000 WHERE id_directory={}",
            target.child_directory_id.unwrap()
        ))?;
        let (source_entry, target_entry, name) = match kind {
            0 => (
                Source::File {
                    file_id: source.file_id.unwrap(),
                },
                Target::File {
                    file_id: target.file_id.unwrap(),
                },
                "file",
            ),
            1 => (
                Source::Hardlink {
                    hardlink_id: source.hardlink_id.unwrap(),
                    file_id: source.file_id.unwrap(),
                },
                Target::Hardlink {
                    hardlink_id: target.hardlink_id.unwrap(),
                    file_id: target.file_id.unwrap(),
                },
                "hardlink",
            ),
            2 => (
                Source::Symlink {
                    symlink_id: source.symlink_id.unwrap(),
                },
                Target::Symlink {
                    symlink_id: target.symlink_id.unwrap(),
                },
                "symlink",
            ),
            _ => (
                Source::Directory {
                    directory_id: source.child_directory_id.unwrap(),
                },
                Target::Directory {
                    directory_id: target.child_directory_id.unwrap(),
                },
                "child",
            ),
        };
        source.repo.exec(&format!(
            "UPDATE directories SET uid=2000,mode='1777' WHERE id_directory={}",
            source.directory_id
        ))?;
        let denied_source = source.repo.rename_namespace_replace_with_ownership_guard(
            source_entry,
            Some(source.directory_id),
            name,
            Some(source.directory_id),
            "source-moved",
            None,
            3000,
        );
        assert_eq!(
            denied_source.unwrap_err(),
            fod_rust_hotpath::pg::STICKY_PERMISSION_DENIED
        );
        source.repo.exec(&format!(
            "UPDATE directories SET uid=1000,mode='755' WHERE id_directory={}",
            source.directory_id
        ))?;
        let result = source.repo.rename_namespace_replace_with_ownership_guard(
            source_entry,
            Some(source.directory_id),
            name,
            Some(parent),
            name,
            Some(target_entry),
            1000,
        );
        assert_eq!(
            result.unwrap_err(),
            fod_rust_hotpath::pg::STICKY_PERMISSION_DENIED
        );
        assert!(source
            .repo
            .resolve_path(&format!("/{}/{name}", source.directory_name))?
            .entry_id
            .is_some());
        assert!(target
            .repo
            .resolve_path(&format!("/{}/{name}", target.directory_name))?
            .entry_id
            .is_some());
    }
    Ok(())
}

#[test]
fn sticky_mutations_wait_for_parent_permission_commit_and_reject() -> Result<(), String> {
    let _guard = ENV_LOCK.lock().unwrap();
    for operation in 0..5 {
        let fixture = sticky_fixture()?;
        let parent = fixture.directory_id;
        let file = fixture.file_id.unwrap();
        let symlink = fixture.symlink_id.unwrap();
        let child = fixture.child_directory_id.unwrap();
        fixture
            .repo
            .exec(&format!("UPDATE files SET uid=3000 WHERE id_file={file}"))?;
        fixture.repo.exec(&format!(
            "UPDATE symlinks SET uid=3000 WHERE id_symlink={symlink}"
        ))?;
        fixture.repo.exec(&format!(
            "UPDATE directories SET uid=3000 WHERE id_directory={child}"
        ))?;
        let writer = repo_with_runtime()?;
        let remover = repo_with_runtime()?;
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let writer_thread = std::thread::spawn(move || {
            writer.with_transaction(|tx| {
                tx.exec(&format!(
                    "UPDATE directories SET uid=2000,mode='1777' WHERE id_directory={parent}"
                ))?;
                ready_tx.send(()).map_err(|err| err.to_string())?;
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(30))
                    .map_err(|err| err.to_string())?;
                Ok(())
            })
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|err| err.to_string())?;
        let remove_thread = std::thread::spawn(move || {
            sticky_remove(&remover, operation, parent, file, symlink, child, 1000)
        });
        let observed_wait = (|| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let count = fixture.repo.query_scalar_text("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%FROM directories%FOR SHARE'")?;
                if count.trim() != "0" {
                    return Ok(());
                }
                if std::time::Instant::now() >= deadline {
                    return Err(format!(
                        "sticky operation {operation} did not wait on parent permissions"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        })();
        release_tx.send(()).map_err(|err| err.to_string())?;
        writer_thread
            .join()
            .map_err(|_| "parent writer panicked".to_string())??;
        let outcome = remove_thread
            .join()
            .map_err(|_| "sticky remover panicked".to_string())?;
        observed_wait?;
        assert_eq!(
            outcome.unwrap_err(),
            fod_rust_hotpath::pg::STICKY_PERMISSION_DENIED
        );
    }
    Ok(())
}
