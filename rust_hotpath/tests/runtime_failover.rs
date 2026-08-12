// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex, OnceLock};

use fod_rust_hotpath::pg::{
    DbRepo, DbRepoConnectionTarget, DbRepoConnectionTargetRequirement, DbRepoConnectionTargets,
};
use fod_rust_runtime::RuntimeConfig;

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

#[test]
fn replayable_connection_failure_rotates_to_second_primary_target() {
    let _guard = test_guard();
    let base = conninfo();
    let targets = Arc::new(
        DbRepoConnectionTargets::new(
            vec![
                DbRepoConnectionTarget {
                    authority: "primary-entry-a".to_string(),
                    conninfo: base.clone(),
                },
                DbRepoConnectionTarget {
                    authority: "primary-entry-b".to_string(),
                    conninfo: base.clone(),
                },
            ],
            DbRepoConnectionTargetRequirement::WritablePrimary,
            true,
            true,
        )
        .expect("connection targets"),
    );
    let runtime = RuntimeConfig::from_runtime_map(&HashMap::new()).expect("runtime config");
    let repo =
        DbRepo::with_runtime_connection_targets(targets, &runtime).expect("routed repository");

    let backend_pid = repo
        .query_scalar_text("SELECT pg_backend_pid()")
        .expect("warmup backend pid")
        .trim()
        .parse::<i64>()
        .expect("numeric backend pid");

    let killer = DbRepo::new(&base).expect("killer repository");
    let terminated = killer
        .query_scalar_text(&format!("SELECT pg_terminate_backend({backend_pid})::text"))
        .expect("terminate routed backend");
    assert!(matches!(
        terminated.trim().to_ascii_lowercase().as_str(),
        "t" | "true" | "1" | "on"
    ));

    let recovered = repo
        .query_scalar_text("SELECT 42")
        .expect("query should replay through second primary target");
    assert_eq!(recovered.trim(), "42");

    let snapshot = repo.observability_snapshot().expect("observability");
    assert!(snapshot.routing.endpoint_routing_enabled);
    assert!(snapshot.routing.runtime_failover_enabled);
    assert!(snapshot.routing.primary_promotion_guard_enabled);
    assert!(snapshot.routing.primary_system_identifier.is_some());
    assert!(snapshot.routing.primary_server_fingerprint.is_some());
    assert!(snapshot.routing.primary_guard_scans >= 2);
    assert_eq!(snapshot.routing.primary_guard_split_brain_rejections, 0);
    assert_eq!(
        snapshot.routing.primary_guard_cluster_identity_rejections,
        0
    );
    assert_eq!(snapshot.routing.target_count, 2);
    assert_eq!(snapshot.routing.active_authority, "primary-entry-b");
    assert!(snapshot.routing.generation >= 2);
    assert!(snapshot.routing.failover_count >= 1);
    assert!(snapshot.routing.connection_failures >= 1);
    assert_eq!(snapshot.routing.role_rejections, 0);
    assert_eq!(
        snapshot.routing.last_failed_authority.as_deref(),
        Some("primary-entry-a")
    );
    assert!(snapshot.pool.replay_count >= 1);
}
