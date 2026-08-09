// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use fod_rust_hotpath::pg::{
    DbRepo, DbRepoConnectionTarget, DbRepoConnectionTargetRequirement, DbRepoConnectionTargets,
};
use fod_rust_runtime::RuntimeConfig;

fn base_conninfo() -> String {
    let dbname = env::var("POSTGRES_DB").unwrap_or_else(|_| "foddbname".to_string());
    let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "foduser".to_string());
    let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "cichosza".to_string());
    let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
    format!(
        "host={host} port={port} dbname={dbname} user={user} password={password} connect_timeout=5"
    )
}

fn target(authority: &str, conninfo: String) -> DbRepoConnectionTarget {
    DbRepoConnectionTarget {
        authority: authority.to_string(),
        conninfo,
    }
}

#[test]
fn stale_sensitive_read_uses_replica_then_falls_back_when_replay_lsn_is_behind() {
    let base = base_conninfo();
    let primary_targets = Arc::new(
        DbRepoConnectionTargets::new(
            vec![target(
                "primary",
                format!("{base} application_name=fod-primary-consistency-test"),
            )],
            DbRepoConnectionTargetRequirement::WritablePrimary,
            true,
            false,
        )
        .unwrap(),
    );
    // Local PostgreSQL is deliberately a synthetic read target here. Production
    // pg_lanes always uses ReadOnlyReplica and validates the physical role.
    let synthetic_replica_targets = Arc::new(
        DbRepoConnectionTargets::new(
            vec![target(
                "synthetic-replica",
                format!("{base} application_name=fod-replica-consistency-test"),
            )],
            DbRepoConnectionTargetRequirement::Any,
            true,
            false,
        )
        .unwrap(),
    );
    let runtime = RuntimeConfig::from_runtime_map(&HashMap::new()).unwrap();
    let repo = DbRepo::with_runtime_connection_targets_and_replica_read_targets(
        primary_targets,
        Some(synthetic_replica_targets),
        2,
        &runtime,
    )
    .unwrap();

    let first = repo
        .query_scalar_text_stale_sensitive("SELECT current_setting('application_name')")
        .unwrap();
    assert_eq!(first.trim(), "fod-replica-consistency-test");

    // Successful primary work captures pg_current_wal_lsn(). The synthetic
    // target is actually a primary, so pg_last_wal_replay_lsn() is NULL and
    // the next stale-sensitive read must fall back to the primary.
    assert_eq!(repo.query_scalar_text("SELECT 1").unwrap().trim(), "1");
    let second = repo
        .query_scalar_text_stale_sensitive("SELECT current_setting('application_name')")
        .unwrap();
    assert_eq!(second.trim(), "fod-primary-consistency-test");

    let snapshot = repo.observability_snapshot().unwrap();
    assert!(snapshot.routing.replica_read_routing_enabled);
    assert_eq!(snapshot.routing.replica_target_count, 1);
    assert_eq!(
        snapshot.routing.replica_active_authority.as_deref(),
        Some("synthetic-replica")
    );
    assert!(snapshot.routing.required_primary_wal_lsn.is_some());
    assert!(snapshot.routing.primary_wal_lsn_updates >= 1);
    assert!(snapshot.routing.replica_reads >= 1);
    assert!(snapshot.routing.replica_consistency_checks >= 1);
    assert!(snapshot.routing.replica_lag_fallbacks >= 1);
    assert!(snapshot.routing.primary_read_fallbacks >= 1);
}

#[test]
fn production_replica_role_validation_falls_back_to_primary() {
    let base = base_conninfo();
    let primary_targets = Arc::new(
        DbRepoConnectionTargets::new(
            vec![target(
                "primary",
                format!("{base} application_name=fod-primary-role-test"),
            )],
            DbRepoConnectionTargetRequirement::WritablePrimary,
            true,
            false,
        )
        .unwrap(),
    );
    let invalid_replica_targets = Arc::new(
        DbRepoConnectionTargets::new(
            vec![target(
                "not-a-replica",
                format!("{base} application_name=fod-invalid-replica-role-test"),
            )],
            DbRepoConnectionTargetRequirement::ReadOnlyReplica,
            true,
            false,
        )
        .unwrap(),
    );
    let runtime = RuntimeConfig::from_runtime_map(&HashMap::new()).unwrap();
    let repo = DbRepo::with_runtime_connection_targets_and_replica_read_targets(
        primary_targets,
        Some(invalid_replica_targets),
        1,
        &runtime,
    )
    .unwrap();

    let observed = repo
        .query_scalar_text_stale_sensitive("SELECT current_setting('application_name')")
        .unwrap();
    assert_eq!(observed.trim(), "fod-primary-role-test");
    let snapshot = repo.observability_snapshot().unwrap();
    assert!(snapshot.routing.replica_read_failures >= 1);
    assert!(snapshot.routing.primary_read_fallbacks >= 1);
}
