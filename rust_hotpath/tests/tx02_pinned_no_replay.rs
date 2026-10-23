// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1
//
// Fail closed: DbRepo::with_transaction must never replay an arbitrary
// transaction body because its SQL operations need not be idempotent.

use fod_rust_hotpath::pg::DbRepo;
use std::env;

fn from_env(primary: &str, fallback_name: &str, fallback: &str) -> String {
    env::var(primary)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var(fallback_name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| fallback.to_string())
}

fn conninfo() -> String {
    let database = from_env("FOD_PG_DBNAME", "POSTGRES_DB", "foddbname");
    assert!(
        database != "foddbname" && database.to_ascii_lowercase().contains("test"),
        "TX-02 non-replay regression must run on a dedicated test database, got {database}"
    );
    let host = from_env("FOD_PG_HOST", "POSTGRES_HOST", "127.0.0.1");
    let port = from_env("FOD_PG_PORT", "POSTGRES_PORT", "5432");
    let user = from_env("FOD_PG_USER", "POSTGRES_USER", "foduser");
    let password = from_env("FOD_PG_PASSWORD", "POSTGRES_PASSWORD", "cichosza");
    format!(
        "host={host} port={port} dbname={database} user={user} password={password} connect_timeout=5"
    )
}

#[test]
fn pinned_transaction_body_is_not_replayed_on_connection_error_marker() -> Result<(), String> {
    let repo = DbRepo::new(&conninfo())?;
    let mut executions = 0u32;

    // Simulate the private replayable error marker without dropping a live
    // connection. The generic pool retries this marker, but an arbitrary
    // transaction callback must instead return the original error once.
    let result: Result<(), String> = repo.with_transaction(|tx| {
        let pid = tx.query_scalar_text("SELECT pg_backend_pid()")?;
        if pid.trim().is_empty() {
            return Err("TX-02 did not obtain a database backend".to_string());
        }
        executions += 1;
        Err("__FOD_REPLAYABLE_SQL_ERROR__: tx02 synthetic disconnect".to_string())
    });

    if executions != 1 {
        return Err(format!(
            "TX02_PINNED_NO_REPLAY=FAIL executions={executions} expected=1"
        ));
    }
    match result {
        Err(err) if err == "tx02 synthetic disconnect" => {}
        other => return Err(format!("TX-02 unexpected error: {other:?}")),
    }

    // The caller can continue after the rejected transaction. The failed
    // pool operation discards the connection rather than lending it out.
    let probe = repo.query_scalar_text("SELECT 1")?;
    if probe.trim() != "1" {
        return Err(format!("TX-02 follow-up SQL returned {probe:?}"));
    }
    println!("TX02_PINNED_NO_REPLAY=PASS executions=1 followup=PASS");
    Ok(())
}
