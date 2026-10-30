// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::env;
use std::sync::{Barrier, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use fod_rust_hotpath::pg::DbRepo;

fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    GUARD
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

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

fn conninfo() -> String {
    let dbname = selected_env_value("FOD_PG_DBNAME", "POSTGRES_DB", "foddbname");
    let user = selected_env_value("FOD_PG_USER", "POSTGRES_USER", "foduser");
    let password = selected_env_value("FOD_PG_PASSWORD", "POSTGRES_PASSWORD", "cichosza");
    let host = selected_env_value("FOD_PG_HOST", "POSTGRES_HOST", "127.0.0.1");
    let port = selected_env_value("FOD_PG_PORT", "POSTGRES_PORT", "5432");
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
fn connection_value_prefers_fod_endpoint_over_legacy_postgres_env() {
    assert_eq!(
        prefer_connection_value(Some(" postgresql "), Some("foduser"), "fallback"),
        "postgresql"
    );
    assert_eq!(
        prefer_connection_value(Some("   "), Some(" cichosza "), "fallback"),
        "cichosza"
    );
    assert_eq!(prefer_connection_value(None, None, "fallback"), "fallback");
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
fn invalid_active_flock_lease_type_fails_closed() {
    let _guard = test_guard();
    let repo = repo();
    repo.exec("CREATE SCHEMA IF NOT EXISTS fod")
        .expect("create FOD schema");
    repo.ensure_lock_schema().expect("create lock schema");
    let rid = resource_id();
    let invalid_owner = rid.saturating_add(1);
    let requester = rid.saturating_add(2);

    repo.exec(&format!(
        "INSERT INTO lock_leases (resource_kind, resource_id, session_id, owner_key, lease_kind, lock_type, lease_expires_at, heartbeat_at, created_at, updated_at) VALUES ('file', {rid}, 0, {invalid_owner}, 'flock', 0, NOW() + INTERVAL '1 minute', NOW(), NOW(), NOW())"
    ))
    .expect("seed malformed active flock lease");

    let result = repo.acquire_flock_lease("file", rid, requester, 1, 30, 1);
    let failed_closed = result
        .as_ref()
        .is_err_and(|err| err.contains("unsupported active flock lease type: 0"));

    repo.exec(&format!(
        "DELETE FROM lock_leases WHERE resource_kind = 'file' AND resource_id = {rid}"
    ))
    .expect("remove malformed lease fixture");
    assert!(
        failed_closed,
        "invalid active lease should fail closed, got {result:?}"
    );
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
fn concurrent_range_lock_mutations_serialize_conflict_check_with_write() {
    let _guard = test_guard();
    let repo_a = repo();
    let repo_b = repo();
    ensure_lock_schema(&repo_a);
    repo_a
        .ensure_client_session_schema()
        .expect("create client session schema");
    let rid = resource_id();
    let owner_a = rid.saturating_add(1);
    let owner_b = rid.saturating_add(2);
    let session_a = repo_a
        .register_client_session(
            "lock-a",
            "/mnt/lock-a",
            "primary",
            "postgres_lease",
            101,
            60,
        )
        .expect("register session a");
    let session_b = repo_b
        .register_client_session(
            "lock-b",
            "/mnt/lock-b",
            "primary",
            "postgres_lease",
            102,
            60,
        )
        .expect("register session b");
    repo_a
        .touch_client_session_owner_key(session_a, owner_a)
        .expect("track owner a");
    repo_b
        .touch_client_session_owner_key(session_b, owner_b)
        .expect("track owner b");

    let barrier = std::sync::Arc::new(Barrier::new(3));
    let barrier_a = std::sync::Arc::clone(&barrier);
    let repo_a_thread = repo_a.clone();
    let thread_a = std::thread::spawn(move || {
        barrier_a.wait();
        repo_a_thread
            .update_lock_range_state_blob_for_owner("file", rid, owner_a, 60, |state| {
                if !state.is_empty() {
                    return Ok(None);
                }
                Ok(Some(format!("{owner_a}\t1\t4\t12")))
            })
            .expect("atomic update from session a")
    });
    let barrier_b = std::sync::Arc::clone(&barrier);
    let repo_b_thread = repo_b.clone();
    let thread_b = std::thread::spawn(move || {
        barrier_b.wait();
        repo_b_thread
            .update_lock_range_state_blob_for_owner("file", rid, owner_b, 60, |state| {
                if !state.is_empty() {
                    return Ok(None);
                }
                Ok(Some(format!("{owner_b}\t1\t4\t12")))
            })
            .expect("atomic update from session b")
    });
    barrier.wait();

    let acquired_a = thread_a.join().expect("join lock session a");
    let acquired_b = thread_b.join().expect("join lock session b");
    assert_ne!(
        acquired_a, acquired_b,
        "exactly one overlapping writer wins"
    );
    let state = repo_a
        .load_lock_range_state_blob("file", rid)
        .expect("load serialized lock state");
    assert_eq!(state.iter().filter(|byte| **byte == b'\n').count(), 0);
    assert!(!state.is_empty(), "winning lock must be persisted");
    repo_a
        .delete_range_leases("file", rid, Some(owner_a))
        .expect("clean up owner a range leases");
    repo_b
        .delete_range_leases("file", rid, Some(owner_b))
        .expect("clean up owner b range leases");
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
fn monitor_publish_does_not_refresh_client_session_expiry() {
    let _guard = test_guard();
    let repo = repo();
    ensure_lock_schema(&repo);
    repo.ensure_client_session_schema()
        .expect("create client session schema");

    let session_id = repo
        .register_client_session("host-a", "/mnt/fod", "primary", "postgres_lease", 1234, 180)
        .expect("register session");
    repo.exec(&format!(
        "
        UPDATE client_sessions
        SET lease_expires_at = TIMESTAMP '2099-01-01 00:00:00',
            heartbeat_at = TIMESTAMP '2000-01-01 00:00:00'
        WHERE session_id = {session_id}
        "
    ))
    .expect("pin session times");
    let before = repo
        .query_scalar_text(&format!(
            "
            SELECT lease_expires_at::text || '|' || heartbeat_at::text
            FROM client_sessions
            WHERE session_id = {session_id}
            "
        ))
        .expect("read initial session times");

    repo.publish_monitor_session_stats(
        session_id,
        "3.3.5-test",
        1,
        r#"{"schema_version":1,"sample_seq":1,"publish_interval_millis":60000}"#,
    )
    .expect("publish monitor stats");

    let after = repo
        .query_scalar_text(&format!(
            "
            SELECT lease_expires_at::text || '|' || heartbeat_at::text
            FROM client_sessions
            WHERE session_id = {session_id}
            "
        ))
        .expect("read final session times");
    assert_eq!(after, before);
    let monitor_rows = repo
        .query_scalar_text(&format!(
            "SELECT COUNT(*)::text FROM monitor_session_stats WHERE session_id = {session_id}"
        ))
        .expect("count monitor rows");
    assert_eq!(monitor_rows.trim(), "1");
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
