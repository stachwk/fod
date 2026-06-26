// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_hotpath::pg::{DbRepo, PersistBlockRow, PersistExtentRow};
use std::env;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    GUARD
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

fn dbname() -> String {
    env::var("POSTGRES_DB").unwrap_or_else(|_| "foddbname".to_string())
}

fn user() -> String {
    env::var("POSTGRES_USER").unwrap_or_else(|_| "foduser".to_string())
}

fn password() -> String {
    env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "cichosza".to_string())
}

fn backend_host() -> String {
    env::var("POSTGRES_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
}

fn backend_port() -> String {
    env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string())
}

fn direct_conninfo() -> String {
    format!(
        "host={host} port={port} dbname={dbname} user={user} password={password} connect_timeout=5 sslmode=disable",
        host = backend_host(),
        port = backend_port(),
        dbname = dbname(),
        user = user(),
        password = password(),
    )
}

fn unique_name(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}_{}_{}", std::process::id(), nanos)
}

fn proxy_conninfo(port: u16) -> String {
    format!(
        "host=127.0.0.1 port={port} dbname={dbname} user={user} password={password} connect_timeout=5 sslmode=disable",
        dbname = dbname(),
        user = user(),
        password = password(),
    )
}

fn backend_addr() -> String {
    format!("{}:{}", backend_host(), backend_port())
}

fn read_startup_packet(client: &mut TcpStream, backend: &mut TcpStream) -> Result<(), String> {
    let mut len_buf = [0u8; 4];
    client
        .read_exact(&mut len_buf)
        .map_err(|err| format!("read startup length: {err}"))?;
    let len = u32::from_be_bytes(len_buf);
    if len < 8 {
        return Err("startup packet too short".to_string());
    }
    let mut body = vec![0u8; len as usize - 4];
    client
        .read_exact(&mut body)
        .map_err(|err| format!("read startup body: {err}"))?;
    backend
        .write_all(&len_buf)
        .and_then(|_| backend.write_all(&body))
        .and_then(|_| backend.flush())
        .map_err(|err| format!("forward startup packet: {err}"))?;
    Ok(())
}

fn extract_query_text(message_type: u8, body: &[u8]) -> Option<String> {
    match message_type {
        b'Q' => {
            let query_bytes = body.split_last().map(|(_, rest)| rest).unwrap_or(body);
            Some(String::from_utf8_lossy(query_bytes).to_string())
        }
        b'P' => {
            let mut parts = body.split(|byte| *byte == 0);
            let _statement_name = parts.next()?;
            let query = parts.next()?;
            Some(String::from_utf8_lossy(query).to_string())
        }
        _ => None,
    }
}

fn relay_server_to_client(
    mut backend: TcpStream,
    mut client: TcpStream,
    stop_forwarding: std::sync::Arc<AtomicBool>,
) {
    let mut buffer = [0u8; 8192];
    loop {
        if stop_forwarding.load(Ordering::SeqCst) {
            break;
        }
        let read = match backend.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) => break,
        };
        if stop_forwarding.load(Ordering::SeqCst) {
            break;
        }
        if client.write_all(&buffer[..read]).is_err() {
            break;
        }
    }
    let _ = client.flush();
}

fn handle_proxy_connection(
    mut client: TcpStream,
    marker: String,
    drop_delay: Duration,
    match_hits: std::sync::Arc<AtomicUsize>,
    drop_hits: std::sync::Arc<AtomicUsize>,
    triggered: std::sync::Arc<AtomicBool>,
) -> Result<(), String> {
    let mut backend = TcpStream::connect(backend_addr())
        .map_err(|err| format!("connect backend PostgreSQL: {err}"))?;
    client
        .set_nodelay(true)
        .and_then(|_| backend.set_nodelay(true))
        .map_err(|err| format!("set_nodelay: {err}"))?;

    read_startup_packet(&mut client, &mut backend)?;

    let stop_forwarding = std::sync::Arc::new(AtomicBool::new(false));
    let server_thread = {
        let stop_forwarding = stop_forwarding.clone();
        let backend_reader = backend
            .try_clone()
            .map_err(|err| format!("clone backend: {err}"))?;
        let client_writer = client
            .try_clone()
            .map_err(|err| format!("clone client: {err}"))?;
        thread::spawn(move || {
            relay_server_to_client(backend_reader, client_writer, stop_forwarding)
        })
    };

    let mut client_reader = client;
    let mut backend_writer = backend;
    let mut drop_on_next_execute = false;
    loop {
        let mut type_buf = [0u8; 1];
        match client_reader.read_exact(&mut type_buf) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(format!("read message type: {err}")),
        }

        let mut len_buf = [0u8; 4];
        client_reader
            .read_exact(&mut len_buf)
            .map_err(|err| format!("read message length: {err}"))?;
        let len = u32::from_be_bytes(len_buf);
        if len < 4 {
            return Err("postgres message length too short".to_string());
        }
        let mut body = vec![0u8; len as usize - 4];
        client_reader
            .read_exact(&mut body)
            .map_err(|err| format!("read message body: {err}"))?;

        let mut message = Vec::with_capacity(len as usize + 1);
        message.push(type_buf[0]);
        message.extend_from_slice(&len_buf);
        message.extend_from_slice(&body);

        let query_text = extract_query_text(type_buf[0], &body);

        backend_writer
            .write_all(&message)
            .and_then(|_| backend_writer.flush())
            .map_err(|err| format!("forward message: {err}"))?;

        if let Some(query) = query_text {
            if query.contains(&marker) {
                match_hits.fetch_add(1, Ordering::SeqCst);
                if type_buf[0] == b'Q' {
                    if !triggered.swap(true, Ordering::SeqCst) {
                        drop_hits.fetch_add(1, Ordering::SeqCst);
                        stop_forwarding.store(true, Ordering::SeqCst);
                        if !drop_delay.is_zero() {
                            thread::sleep(drop_delay);
                        }
                        let _ = client_reader.shutdown(Shutdown::Both);
                        let _ = backend_writer.shutdown(Shutdown::Both);
                        break;
                    }
                } else if type_buf[0] == b'P' {
                    drop_on_next_execute = true;
                }
            }
        }

        if type_buf[0] == b'E' && drop_on_next_execute {
            if !triggered.swap(true, Ordering::SeqCst) {
                drop_hits.fetch_add(1, Ordering::SeqCst);
                stop_forwarding.store(true, Ordering::SeqCst);
                if !drop_delay.is_zero() {
                    thread::sleep(drop_delay);
                }
                let _ = client_reader.shutdown(Shutdown::Both);
                let _ = backend_writer.shutdown(Shutdown::Both);
                break;
            }
        }
    }

    stop_forwarding.store(true, Ordering::SeqCst);
    let _ = client_reader.shutdown(Shutdown::Both);
    let _ = backend_writer.shutdown(Shutdown::Both);
    drop(server_thread);
    Ok(())
}

struct QueryDropProxy {
    port: u16,
    stop: std::sync::Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    match_hits: std::sync::Arc<AtomicUsize>,
    drop_hits: std::sync::Arc<AtomicUsize>,
}

impl QueryDropProxy {
    fn start(marker: &str, drop_delay: Duration) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|err| format!("bind proxy listener: {err}"))?;
        let port = listener
            .local_addr()
            .map_err(|err| format!("proxy local addr: {err}"))?
            .port();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let match_hits = std::sync::Arc::new(AtomicUsize::new(0));
        let drop_hits = std::sync::Arc::new(AtomicUsize::new(0));
        let triggered = std::sync::Arc::new(AtomicBool::new(false));

        let thread_stop = stop.clone();
        let thread_match_hits = match_hits.clone();
        let thread_drop_hits = drop_hits.clone();
        let marker = marker.to_string();
        let handle = thread::spawn(move || {
            let _ = listener.set_nonblocking(false);
            while !thread_stop.load(Ordering::SeqCst) {
                let (client, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(err) => {
                        eprintln!("FOD transactional replay proxy accept error: {err}");
                        break;
                    }
                };
                if thread_stop.load(Ordering::SeqCst) {
                    break;
                }
                let _ = handle_proxy_connection(
                    client,
                    marker.clone(),
                    drop_delay,
                    thread_match_hits.clone(),
                    thread_drop_hits.clone(),
                    triggered.clone(),
                );
            }
        });

        Ok(Self {
            port,
            stop,
            handle: Some(handle),
            match_hits,
            drop_hits,
        })
    }

    fn conninfo(&self) -> String {
        proxy_conninfo(self.port)
    }

    fn match_hits(&self) -> usize {
        self.match_hits.load(Ordering::SeqCst)
    }

    fn drop_hits(&self) -> usize {
        self.drop_hits.load(Ordering::SeqCst)
    }
}

impl Drop for QueryDropProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        let _ = self.handle.take();
    }
}

fn repo_from_conninfo(conninfo: &str) -> DbRepo {
    DbRepo::new(conninfo).expect("failed to connect to PostgreSQL")
}

fn ensure_commit_smoke_table(repo: &DbRepo, table_name: &str) {
    repo.exec(&format!(
        "
        CREATE TABLE IF NOT EXISTS {table_name} (
            request_token TEXT PRIMARY KEY,
            marker TEXT NOT NULL,
            updated_at TIMESTAMP NOT NULL DEFAULT NOW()
        )
        "
    ))
    .expect("create commit smoke table");
}

#[test]
fn transactional_body_disconnect_is_replayed_once() {
    let _guard = test_guard();
    let proxy = QueryDropProxy::start("INSERT INTO directories", Duration::from_millis(0))
        .expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());

    let dirname = unique_name("transactional_body");
    let seed = unique_name("transactional_body_seed");
    let directory_id = repo
        .create_directory(None, &dirname, 0o755, 1000, 1000, &seed)
        .expect("create directory with replay");

    assert_eq!(proxy.drop_hits(), 1);
    assert!(proxy.match_hits() >= 2);
    assert_eq!(
        repo.get_dir_id(&format!("/{dirname}"))
            .expect("lookup created directory"),
        Some(directory_id)
    );
    repo.delete_directory_entry(directory_id)
        .expect("cleanup created directory");
}

#[test]
fn transactional_multistatement_disconnect_is_replayed_once() {
    let _guard = test_guard();
    let direct_repo = repo_from_conninfo(&direct_conninfo());
    let parent_name = unique_name("transactional_multi_parent");
    let parent_seed = unique_name("transactional_multi_parent_seed");
    let parent_id = direct_repo
        .create_directory(None, &parent_name, 0o755, 1000, 1000, &parent_seed)
        .expect("create parent directory");

    let proxy =
        QueryDropProxy::start("INSERT INTO files", Duration::from_millis(0)).expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());

    let file_name = unique_name("transactional_multi_file");
    let file_seed = unique_name("transactional_multi_seed");
    let file_id = repo
        .create_file(Some(parent_id), &file_name, 0o644, 1000, 1000, &file_seed)
        .expect("create file with replay");

    assert_eq!(proxy.drop_hits(), 1);
    assert!(proxy.match_hits() >= 2);
    assert_eq!(
        direct_repo
            .get_file_id(&format!("/{parent_name}/{file_name}"))
            .expect("lookup created file"),
        Some(file_id)
    );

    direct_repo
        .purge_primary_file(file_id)
        .expect("cleanup created file");
    direct_repo
        .delete_directory_entry(parent_id)
        .expect("cleanup parent directory");
}

#[test]
fn transactional_commit_disconnect_is_confirmed_by_request_token_probe() {
    let _guard = test_guard();
    let smoke_table = unique_name("transactional_commit_smoke");
    let direct_repo = repo_from_conninfo(&direct_conninfo());
    ensure_commit_smoke_table(&direct_repo, &smoke_table);

    let proxy = QueryDropProxy::start("COMMIT", Duration::from_millis(50)).expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());
    let request_token = unique_name("commit_token");
    let request_token_sql = DbRepo::quote_literal(&request_token);

    repo.exec("BEGIN").expect("begin transaction");
    repo.exec(&format!(
        "
        INSERT INTO {smoke_table} (request_token, marker, updated_at)
        VALUES ({request_token_sql}, 'committed', NOW())
        ON CONFLICT (request_token) DO UPDATE SET
            marker = EXCLUDED.marker,
            updated_at = NOW()
        "
    ))
    .expect("insert smoke row");

    let commit_err = repo
        .exec("COMMIT")
        .expect_err("commit should lose its acknowledgement");
    assert!(!commit_err.trim().is_empty());
    assert_eq!(proxy.drop_hits(), 1);
    assert_eq!(proxy.match_hits(), 1);

    let confirmed = repo
        .query_scalar_text(&format!(
            "SELECT COUNT(*) FROM {smoke_table} WHERE request_token = {request_token_sql}"
        ))
        .expect("probe committed row");
    assert_eq!(confirmed.trim(), "1");

    let _ = direct_repo.exec(&format!("DROP TABLE IF EXISTS {smoke_table}"));
}

#[test]
fn transactional_commit_disconnect_is_replayed_for_set_file_size() {
    let _guard = test_guard();
    let direct_repo = repo_from_conninfo(&direct_conninfo());
    let parent_name = unique_name("transactional_size_parent");
    let parent_seed = unique_name("transactional_size_parent_seed");
    let parent_id = direct_repo
        .create_directory(None, &parent_name, 0o755, 1000, 1000, &parent_seed)
        .expect("create parent directory");

    let file_name = unique_name("transactional_size_file");
    let file_seed = unique_name("transactional_size_file_seed");
    let file_id = direct_repo
        .create_file(Some(parent_id), &file_name, 0o644, 1000, 1000, &file_seed)
        .expect("create target file");

    let proxy = QueryDropProxy::start("COMMIT", Duration::from_millis(50)).expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());
    repo.set_file_size(file_id, 12_345)
        .expect("set file size with replay");

    assert_eq!(proxy.drop_hits(), 1);
    assert_eq!(proxy.match_hits(), 2);
    assert_eq!(
        direct_repo.file_size(file_id).expect("file size after set"),
        Some(12_345)
    );

    direct_repo
        .purge_primary_file(file_id)
        .expect("cleanup file");
    direct_repo
        .delete_directory_entry(parent_id)
        .expect("cleanup parent directory");
}

#[test]
fn transactional_commit_disconnect_is_replayed_for_create_data_object() {
    let _guard = test_guard();
    let direct_repo = repo_from_conninfo(&direct_conninfo());
    let proxy = QueryDropProxy::start("COMMIT", Duration::from_millis(50)).expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());

    let content_hash = unique_name("transactional_data_object_hash");
    let data_object_id = repo
        .create_data_object(8_192, Some(content_hash.as_str()))
        .expect("create data object with replay");

    assert_eq!(proxy.drop_hits(), 1);
    assert_eq!(proxy.match_hits(), 2);

    let reference_count = direct_repo
        .query_scalar_text(&format!(
            "SELECT reference_count FROM data_objects WHERE id_data_object = {}",
            data_object_id
        ))
        .expect("query data object reference count")
        .trim()
        .parse::<u64>()
        .expect("parse reference count");
    assert_eq!(
        reference_count, 1,
        "replayed create_data_object should not double increment reference_count"
    );

    let token_rows = direct_repo
        .query_scalar_text(&format!(
            "SELECT COUNT(*) FROM data_object_request_tokens WHERE id_data_object = {}",
            data_object_id
        ))
        .expect("query data object request token count");
    assert_eq!(token_rows.trim(), "1");

    direct_repo
        .query_scalar_text(&format!(
            "DELETE FROM data_objects WHERE id_data_object = {} RETURNING id_data_object::text",
            data_object_id
        ))
        .expect("cleanup data object");
}

#[test]
fn transactional_commit_disconnect_is_replayed_for_promote_hardlink_to_primary() {
    let _guard = test_guard();
    let direct_repo = repo_from_conninfo(&direct_conninfo());
    let parent_name = unique_name("transactional_promote_parent");
    let parent_seed = unique_name("transactional_promote_parent_seed");
    let parent_id = direct_repo
        .create_directory(None, &parent_name, 0o755, 1000, 1000, &parent_seed)
        .expect("create parent directory");

    let file_name = unique_name("transactional_promote_file");
    let file_seed = unique_name("transactional_promote_file_seed");
    let file_id = direct_repo
        .create_file(Some(parent_id), &file_name, 0o644, 1000, 1000, &file_seed)
        .expect("create primary file");
    let hardlink_name = unique_name("transactional_promote_hardlink");
    let hardlink_id = direct_repo
        .create_hardlink(file_id, Some(parent_id), &hardlink_name, 1000, 1000)
        .expect("create hardlink");

    let proxy = QueryDropProxy::start("COMMIT", Duration::from_millis(50)).expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());
    assert!(
        repo.promote_hardlink_to_primary(file_id)
            .expect("promote hardlink with replay"),
        "promotion should succeed"
    );

    assert_eq!(proxy.drop_hits(), 1);
    assert!(proxy.match_hits() >= 2);
    assert_eq!(
        direct_repo
            .get_file_id(&format!("/{parent_name}/{hardlink_name}"))
            .expect("lookup promoted file"),
        Some(file_id)
    );
    assert_eq!(
        direct_repo
            .get_hardlink_id(&format!("/{parent_name}/{hardlink_name}"))
            .expect("lookup promoted hardlink"),
        None
    );
    assert_eq!(
        direct_repo
            .get_file_id(&format!("/{parent_name}/{file_name}"))
            .expect("lookup original path"),
        None
    );
    assert_eq!(
        direct_repo
            .get_hardlink_file_id(hardlink_id)
            .expect("lookup promoted hardlink row"),
        None
    );
    assert_eq!(
        direct_repo
            .count_file_links(file_id)
            .expect("count links after promotion"),
        1
    );
    let request_tokens = direct_repo
        .query_scalar_text(&format!(
            "SELECT COUNT(*) FROM hardlink_promotion_request_tokens WHERE id_file = {file_id}"
        ))
        .expect("count hardlink promotion request tokens");
    assert_eq!(request_tokens.trim(), "1");

    direct_repo
        .purge_primary_file(file_id)
        .expect("cleanup promoted file");
    direct_repo
        .delete_directory_entry(parent_id)
        .expect("cleanup parent directory");
}

#[test]
fn transactional_commit_disconnect_is_replayed_for_lock_range_state_blob() {
    let _guard = test_guard();
    let direct_repo = repo_from_conninfo(&direct_conninfo());
    direct_repo
        .ensure_lock_schema()
        .expect("ensure lock schema");

    let resource_kind = "tx_lock_range".to_string();
    let resource_id = 424_242_u64;
    let payload = "1001\t1\t0\t5\n1002\t2\t5\t";

    let proxy = QueryDropProxy::start("COMMIT", Duration::from_millis(50)).expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());
    repo.persist_lock_range_state_blob(&resource_kind, resource_id, 2, payload)
        .expect("persist lock range state with replay");

    assert_eq!(proxy.drop_hits(), 1);
    assert_eq!(proxy.match_hits(), 2);
    let loaded = direct_repo
        .load_lock_range_state_blob(&resource_kind, resource_id)
        .expect("load lock range state");
    assert_eq!(loaded, payload.as_bytes());

    direct_repo
        .exec(&format!(
            "DELETE FROM lock_range_leases WHERE resource_kind = '{}' AND resource_id = {}",
            resource_kind, resource_id
        ))
        .expect("cleanup range leases");
}

#[test]
fn transactional_commit_disconnect_is_replayed_for_replace_lock_range_state_blob_for_owner() {
    let _guard = test_guard();
    let direct_repo = repo_from_conninfo(&direct_conninfo());
    direct_repo
        .ensure_lock_schema()
        .expect("ensure lock schema");

    let resource_kind = "tx_lock_owner".to_string();
    let resource_id = 434_343_u64;
    let owner_key = 1001_u64;
    let payload = "1001\t1\t0\t5\n1001\t2\t5\t10";

    let proxy = QueryDropProxy::start("COMMIT", Duration::from_millis(50)).expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());
    repo.replace_lock_range_state_blob_for_owner(
        &resource_kind,
        resource_id,
        owner_key,
        2,
        payload,
    )
    .expect("replace lock range state with replay");

    assert_eq!(proxy.drop_hits(), 1);
    assert_eq!(proxy.match_hits(), 2);
    let loaded = direct_repo
        .load_lock_range_state_blob(&resource_kind, resource_id)
        .expect("load replaced lock range state");
    assert_eq!(loaded, payload.as_bytes());

    direct_repo
        .exec(&format!(
            "DELETE FROM lock_range_leases WHERE resource_kind = '{}' AND resource_id = {}",
            resource_kind, resource_id
        ))
        .expect("cleanup range leases");
}

#[test]
fn transactional_commit_disconnect_is_replayed_for_lock_lease_prune() {
    let _guard = test_guard();
    let direct_repo = repo_from_conninfo(&direct_conninfo());
    direct_repo
        .ensure_lock_schema()
        .expect("ensure lock schema");

    let proxy = QueryDropProxy::start("COMMIT", Duration::from_millis(50)).expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());
    let resource_kind = unique_name("transactional_prune_kind");
    let resource_id = 424242_u64;

    repo.prune_lock_leases(Some(&resource_kind), Some(resource_id))
        .expect("prune lock leases with replay");

    assert_eq!(proxy.drop_hits(), 1);
    assert!(proxy.match_hits() >= 2);
}

#[test]
fn transactional_commit_disconnect_is_replayed_for_copy_block_crc_persist() {
    let _guard = test_guard();
    let direct_repo = repo_from_conninfo(&direct_conninfo());
    let parent_name = unique_name("transactional_crc_parent");
    let parent_seed = unique_name("transactional_crc_parent_seed");
    let parent_id = direct_repo
        .create_directory(None, &parent_name, 0o755, 1000, 1000, &parent_seed)
        .expect("create parent directory");

    let file_name = unique_name("transactional_crc_file");
    let file_seed = unique_name("transactional_crc_file_seed");
    let file_id = direct_repo
        .create_file(Some(parent_id), &file_name, 0o644, 1000, 1000, &file_seed)
        .expect("create target file");
    let file_data_object_id = direct_repo
        .file_data_object_id(file_id)
        .expect("lookup file data object id")
        .expect("file should have a data object");

    let block_size = 4usize;
    let block0 = vec![b'A'; block_size];
    let block1 = vec![b'B'; block_size];
    let blocks = vec![
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
    ];

    let proxy = QueryDropProxy::start("COMMIT", Duration::from_millis(50)).expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());
    repo.persist_copy_block_crc_rows(file_id, block_size as u64, &blocks)
        .expect("persist copy block crc rows with replay");

    assert_eq!(proxy.drop_hits(), 1);
    assert_eq!(proxy.match_hits(), 2);
    let crc_rows = direct_repo
        .query_scalar_text(&format!(
            "SELECT COUNT(*) FROM copy_block_crc WHERE data_object_id = {file_data_object_id}"
        ))
        .expect("count copy_block_crc rows");
    assert_eq!(crc_rows.trim(), "2");

    direct_repo
        .purge_primary_file(file_id)
        .expect("cleanup file");
    direct_repo
        .delete_directory_entry(parent_id)
        .expect("cleanup parent directory");
}

#[test]
fn transactional_commit_disconnect_is_replayed_for_block_persist() {
    let _guard = test_guard();
    let direct_repo = repo_from_conninfo(&direct_conninfo());
    let parent_name = unique_name("transactional_blocks_parent");
    let parent_seed = unique_name("transactional_blocks_parent_seed");
    let parent_id = direct_repo
        .create_directory(None, &parent_name, 0o755, 1000, 1000, &parent_seed)
        .expect("create parent directory");

    let file_name = unique_name("transactional_blocks_file");
    let file_seed = unique_name("transactional_blocks_file_seed");
    let file_id = direct_repo
        .create_file(Some(parent_id), &file_name, 0o644, 1000, 1000, &file_seed)
        .expect("create target file");

    let block_size = 4usize;
    let block0 = vec![b'A'; block_size];
    let block1 = vec![b'B'; block_size];
    let block2 = vec![b'C'; block_size];
    let block_payload = [block0.clone(), block1.clone(), block2.clone()].concat();
    let blocks = vec![
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

    let proxy = QueryDropProxy::start("COMMIT", Duration::from_millis(50)).expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());
    repo.persist_file_blocks(
        file_id,
        block_payload.len() as u64,
        block_size as u64,
        3,
        false,
        &blocks,
    )
    .expect("persist file blocks with replay");

    assert_eq!(proxy.drop_hits(), 1);
    assert!(proxy.match_hits() >= 2);
    assert_eq!(
        direct_repo
            .file_size(file_id)
            .expect("file size after block persist"),
        Some(block_payload.len() as u64)
    );
    assert_eq!(
        direct_repo
            .count_file_blocks(file_id)
            .expect("count file blocks after persist"),
        3
    );
    assert_eq!(
        direct_repo
            .fetch_block_range(file_id, 0, 2, block_size as u64)
            .expect("fetch persisted blocks"),
        vec![
            (0, block0.clone()),
            (1, block1.clone()),
            (2, block2.clone()),
        ]
    );

    let crc_rows = direct_repo
        .query_scalar_text(&format!(
            "SELECT COUNT(*) FROM copy_block_crc WHERE data_object_id = (SELECT data_object_id FROM files WHERE id_file = {file_id})"
        ))
        .expect("count copy_block_crc rows");
    assert_eq!(crc_rows.trim(), "3");

    direct_repo
        .purge_primary_file(file_id)
        .expect("cleanup file");
    direct_repo
        .delete_directory_entry(parent_id)
        .expect("cleanup parent directory");
}

#[test]
fn transactional_commit_disconnect_is_replayed_for_extent_persist() {
    let _guard = test_guard();
    let direct_repo = repo_from_conninfo(&direct_conninfo());
    let parent_name = unique_name("transactional_extents_parent");
    let parent_seed = unique_name("transactional_extents_parent_seed");
    let parent_id = direct_repo
        .create_directory(None, &parent_name, 0o755, 1000, 1000, &parent_seed)
        .expect("create parent directory");

    let file_name = unique_name("transactional_extents_file");
    let file_seed = unique_name("transactional_extents_file_seed");
    let file_id = direct_repo
        .create_file(Some(parent_id), &file_name, 0o644, 1000, 1000, &file_seed)
        .expect("create target file");

    let block_size = 4u64;
    let extent_block0 = vec![b'X'; block_size as usize];
    let extent_block1 = vec![b'Y'; block_size as usize];
    let extent_payload = [extent_block0.clone(), extent_block1.clone()].concat();
    let extents = vec![PersistExtentRow {
        start_block: 0,
        block_count: 2,
        used_bytes: extent_payload.len() as u64,
        payload: extent_payload.clone(),
    }];

    let proxy = QueryDropProxy::start("COMMIT", Duration::from_millis(50)).expect("start proxy");
    let repo = repo_from_conninfo(&proxy.conninfo());
    repo.persist_file_extents_native(
        file_id,
        extent_payload.len() as u64,
        block_size,
        2,
        false,
        &extents,
        true,
    )
    .expect("persist file extents with replay");

    assert_eq!(proxy.drop_hits(), 1);
    assert_eq!(proxy.match_hits(), 2);
    assert_eq!(
        direct_repo
            .file_size(file_id)
            .expect("file size after extent persist"),
        Some(extent_payload.len() as u64)
    );
    assert_eq!(
        direct_repo
            .fetch_block_range(file_id, 0, 1, block_size)
            .expect("fetch persisted extents"),
        vec![(0, extent_block0.clone()), (1, extent_block1.clone())]
    );
    let crc_rows = direct_repo
        .query_scalar_text(&format!(
            "SELECT COUNT(*) FROM copy_block_crc WHERE data_object_id = (SELECT data_object_id FROM files WHERE id_file = {file_id})"
        ))
        .expect("count copy_block_crc rows after extent persist");
    assert_eq!(crc_rows.trim(), "2");
    let extent_rows = direct_repo
        .query_scalar_text(&format!(
            "SELECT COUNT(*) FROM data_extents WHERE data_object_id = (SELECT data_object_id FROM files WHERE id_file = {file_id})"
        ))
        .expect("count data_extents rows");
    assert_eq!(extent_rows.trim(), "1");
    assert_eq!(
        direct_repo
            .count_file_blocks(file_id)
            .expect("count file blocks after extent persist"),
        2
    );

    direct_repo
        .purge_primary_file(file_id)
        .expect("cleanup file");
    direct_repo
        .delete_directory_entry(parent_id)
        .expect("cleanup parent directory");
}
