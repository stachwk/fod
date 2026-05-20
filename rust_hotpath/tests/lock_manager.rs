// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::env;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use fod_rust_hotpath::pg::DbRepo;

fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    GUARD
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

fn conninfo() -> String {
    let dbname = env::var("POSTGRES_DB").unwrap_or_else(|_| "foddbname".to_string());
    let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "foduser".to_string());
    let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "cichosza".to_string());
    let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
    format!(
        "host={host} port={port} dbname={dbname} user={user} password={password} connect_timeout=5"
    )
}

fn repo() -> DbRepo {
    DbRepo::new(&conninfo()).expect("failed to connect to PostgreSQL")
}

fn resource_id() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock went backwards")
        .as_nanos() as u64
}

fn ensure_lock_schema(repo: &DbRepo) {
    repo.exec(
        r#"
        CREATE TABLE IF NOT EXISTS lock_leases (
            id_lock SERIAL PRIMARY KEY,
            resource_kind VARCHAR(20) NOT NULL,
            resource_id BIGINT NOT NULL,
            owner_key BIGINT NOT NULL,
            lease_kind VARCHAR(20) NOT NULL,
            lock_type INTEGER NOT NULL,
            lease_expires_at TIMESTAMP NOT NULL,
            heartbeat_at TIMESTAMP NOT NULL,
            created_at TIMESTAMP NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
            UNIQUE(resource_kind, resource_id, owner_key, lease_kind)
        );
        CREATE INDEX IF NOT EXISTS idx_lock_leases_resource
            ON lock_leases (resource_kind, resource_id, lease_kind);
        CREATE INDEX IF NOT EXISTS idx_lock_leases_expires
            ON lock_leases (lease_expires_at);
        CREATE TABLE IF NOT EXISTS lock_range_leases (
            id_lock SERIAL PRIMARY KEY,
            resource_kind VARCHAR(20) NOT NULL,
            resource_id BIGINT NOT NULL,
            owner_key BIGINT NOT NULL,
            lock_type INTEGER NOT NULL,
            range_start BIGINT NOT NULL,
            range_end BIGINT NULL,
            lease_expires_at TIMESTAMP NOT NULL,
            heartbeat_at TIMESTAMP NOT NULL,
            created_at TIMESTAMP NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMP NOT NULL DEFAULT NOW()
        );
        CREATE INDEX IF NOT EXISTS idx_lock_range_leases_resource
            ON lock_range_leases (resource_kind, resource_id);
        CREATE INDEX IF NOT EXISTS idx_lock_range_leases_expires
            ON lock_range_leases (lease_expires_at);
        "#,
    )
    .expect("create lock schema");
}

#[test]
fn flock_leases_conflict_and_release() {
    let _guard = test_guard();
    let repo_a = repo();
    let repo_b = repo();
    ensure_lock_schema(&repo_a);
    ensure_lock_schema(&repo_b);
    let rid = resource_id();
    let owner_a = rid.saturating_add(1);
    let owner_b = rid.saturating_add(2);

    assert!(repo_a
        .acquire_flock_lease("file", rid, owner_a, 2, 2, 1)
        .expect("acquire owner_a"));
    assert!(!repo_b
        .acquire_flock_lease("file", rid, owner_b, 2, 2, 1)
        .expect("acquire owner_b conflict"));
    repo_a
        .heartbeat_lock_lease("file", rid, owner_a, 2)
        .expect("heartbeat");
    repo_a
        .release_flock_lease("file", rid, owner_a)
        .expect("release owner_a");
    assert!(repo_b
        .acquire_flock_lease("file", rid, owner_b, 2, 2, 1)
        .expect("acquire owner_b after release"));
    repo_b
        .release_flock_lease("file", rid, owner_b)
        .expect("release owner_b");
}

#[test]
fn expired_flock_lease_is_pruned_on_reacquire() {
    let _guard = test_guard();
    let repo = repo();
    ensure_lock_schema(&repo);
    let rid = resource_id();
    let owner_a = rid.saturating_add(1);
    let owner_b = rid.saturating_add(2);
    let write_lock = 2;

    assert!(repo
        .acquire_flock_lease("file", rid, owner_a, write_lock, 1, 1)
        .expect("acquire owner_a"));
    repo.exec(&format!(
        "UPDATE lock_leases SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE resource_kind = 'file' AND resource_id = {} AND owner_key = {} AND lease_kind = 'flock'",
        rid, owner_a
    ))
    .expect("expire owner_a lease");

    assert!(repo
        .acquire_flock_lease("file", rid, owner_b, write_lock, 1, 1)
        .expect("acquire owner_b after expiry"));

    let count = repo
        .query_scalar_text(&format!(
            "SELECT COUNT(*) FROM lock_leases WHERE resource_kind = 'file' AND resource_id = {}",
            rid
        ))
        .expect("count rows")
        .trim()
        .parse::<u64>()
        .expect("parse row count");
    assert_eq!(count, 1);

    repo.release_flock_lease("file", rid, owner_b)
        .expect("release owner_b");
}

#[test]
fn range_leases_roundtrip_and_cleanup() {
    let _guard = test_guard();
    let repo = repo();
    ensure_lock_schema(&repo);
    let rid = resource_id();

    let payload = "1001\t1\t0\t5\n1002\t2\t5\t";
    repo.persist_lock_range_state_blob("file", rid, 2, payload)
        .expect("persist range state");
    let loaded = repo
        .load_lock_range_state_blob("file", rid)
        .expect("load range state");
    assert_eq!(loaded, payload.as_bytes());

    repo.delete_range_leases("file", rid, None)
        .expect("delete range leases");
    let after_prune = repo
        .load_lock_range_state_blob("file", rid)
        .expect("load after prune");
    assert!(after_prune.is_empty(), "{after_prune:?}");
}

#[test]
fn expired_range_lease_is_hidden_from_load() {
    let _guard = test_guard();
    let repo = repo();
    ensure_lock_schema(&repo);
    let rid = resource_id();

    let payload = "1001\t1\t0\t5";
    repo.persist_lock_range_state_blob("file", rid, 1, payload)
        .expect("persist range state");
    repo.exec(&format!(
        "UPDATE lock_range_leases SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE resource_kind = 'file' AND resource_id = {}",
        rid
    ))
    .expect("expire range lease");

    let loaded = repo
        .load_lock_range_state_blob("file", rid)
        .expect("load expired range state");
    assert!(loaded.is_empty(), "{loaded:?}");
}

#[test]
fn client_session_heartbeat_extends_expiry() {
    let _guard = test_guard();
    let repo = repo();
    ensure_lock_schema(&repo);
    repo.ensure_client_session_schema()
        .expect("create client session schema");

    let session_id = repo
        .register_client_session("host-a", "/mnt/fod", "primary", "postgres_lease", 1234, 1)
        .expect("register session");

    repo.exec(&format!(
        "UPDATE client_sessions SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE session_id = {}",
        session_id
    ))
    .expect("expire session");

    repo.heartbeat_client_session(session_id, 10)
        .expect("heartbeat session");
    let alive = repo
        .query_scalar_text(&format!(
            "SELECT CASE WHEN lease_expires_at > NOW() THEN 't' ELSE 'f' END FROM client_sessions WHERE session_id = {}",
            session_id
        ))
        .expect("query session expiry");
    assert_eq!(alive.trim(), "t");
}

#[test]
fn expired_client_session_prunes_only_its_owners() {
    let _guard = test_guard();
    let repo = repo();
    ensure_lock_schema(&repo);
    repo.ensure_client_session_schema()
        .expect("create client session schema");

    let rid = resource_id();
    let owner_a = 1001;
    let owner_b = 1002;

    let session_a = repo
        .register_client_session("host-a", "/mnt/a", "primary", "postgres_lease", 1001, 10)
        .expect("register session a");
    let session_b = repo
        .register_client_session("host-b", "/mnt/b", "primary", "postgres_lease", 1002, 10)
        .expect("register session b");

    repo.touch_client_session_owner_key(session_a, owner_a)
        .expect("track owner a");
    repo.touch_client_session_owner_key(session_b, owner_b)
        .expect("track owner b");

    repo.replace_lock_range_state_blob_for_owner("file", rid, owner_a, 10, "1001\t1\t0\t5")
        .expect("persist owner a");
    repo.replace_lock_range_state_blob_for_owner("file", rid, owner_b, 10, "1002\t1\t5\t10")
        .expect("persist owner b");

    repo.exec(&format!(
        "UPDATE client_sessions SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE session_id = {}",
        session_a
    ))
    .expect("expire session a");

    assert!(repo
        .prune_expired_client_sessions()
        .expect("prune expired sessions"));

    let remaining = repo
        .load_lock_range_state_blob("file", rid)
        .expect("load remaining range locks");
    assert_eq!(remaining, b"1002\t1\t5\t10");

    let session_a_count = repo
        .query_scalar_text(&format!(
            "SELECT COUNT(*) FROM client_sessions WHERE session_id = {}",
            session_a
        ))
        .expect("count session a");
    assert_eq!(session_a_count.trim(), "0");

    let session_b_count = repo
        .query_scalar_text(&format!(
            "SELECT COUNT(*) FROM client_sessions WHERE session_id = {}",
            session_b
        ))
        .expect("count session b");
    assert_eq!(session_b_count.trim(), "1");
}
