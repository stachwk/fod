// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1
//
// TX-02 characterization: exposing BEGIN/COMMIT as independent DbRepo calls
// can share an open PG transaction between unrelated pool borrowers.
//
// This test deliberately uses no durable SQL writes. It acquires one
// transaction-scoped PostgreSQL advisory lock and checks it from a separate
// independent connection. Do not treat a PASS as proof of transaction safety.

use fod_rust_hotpath::pg::DbRepo;
use std::env;
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn environment_value(primary: &str, alternate: &str, fallback: &str) -> String {
    env::var(primary)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var(alternate)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| fallback.to_string())
}

fn conninfo() -> String {
    let database = environment_value("FOD_PG_DBNAME", "POSTGRES_DB", "foddbname");
    assert!(
        database != "foddbname" && database.to_ascii_lowercase().contains("test"),
        "TX-02 diagnostic must run on an isolated test DB, got {database}"
    );
    let host = environment_value("FOD_PG_HOST", "POSTGRES_HOST", "127.0.0.1");
    let port = environment_value("FOD_PG_PORT", "POSTGRES_PORT", "5432");
    let user = environment_value("FOD_PG_USER", "POSTGRES_USER", "foduser");
    let password = environment_value("FOD_PG_PASSWORD", "POSTGRES_PASSWORD", "cichosza");
    format!("host={host} port={port} dbname={database} user={user} password={password} connect_timeout=5")
}

fn scalar(repo: &DbRepo, sql: &str) -> Result<String, String> {
    Ok(repo.query_scalar_text(sql)?.trim().to_ascii_lowercase())
}

#[test]
fn tx02_open_transaction_can_be_borrowed_by_another_thread() -> Result<(), String> {
    let repo = Arc::new(DbRepo::new(&conninfo())?);
    let observer = DbRepo::new(&conninfo())?;

    // A dedicated, improbable test lock key; no FOD data/lease rows are touched.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_nanos();
    let key = ((nanos as u64) ^ u64::from(std::process::id())) as i64;

    repo.exec("BEGIN")?;
    let check = (|| -> Result<(String, String, String), String> {
        let owner_pid = scalar(&repo, "SELECT pg_backend_pid()")?;
        let acquired = scalar(&repo, &format!("SELECT pg_try_advisory_xact_lock({key})"))?;
        if acquired != "true" && acquired != "t" {
            return Err("TX-02 failed to acquire isolated transaction advisory lock".to_string());
        }

        let external_locked = scalar(
            &observer,
            &format!("SELECT pg_try_advisory_xact_lock({key})"),
        )?;
        if external_locked != "false" && external_locked != "f" {
            return Err(format!(
                "TX-02 expected transaction lock to remain held after returning connection to pool, got {external_locked}"
            ));
        }

        let worker = Arc::clone(&repo);
        let borrowed_pid = thread::spawn(move || scalar(&worker, "SELECT pg_backend_pid()"))
            .join()
            .map_err(|_| "TX-02 worker thread panicked".to_string())??;
        Ok((owner_pid, borrowed_pid, external_locked))
    })();

    // Attempt rollback even if diagnostics fail. With a single cached writer
    // connection and no other active borrowers it should return to this backend.
    let rollback = repo.exec("ROLLBACK");
    let (owner_pid, borrowed_pid, external_locked) = check?;
    rollback?;

    let external_unlocked = scalar(
        &observer,
        &format!("SELECT pg_try_advisory_xact_lock({key})"),
    )?;
    if external_unlocked != "true" && external_unlocked != "t" {
        return Err(format!(
            "TX-02 transaction advisory lock remained held after ROLLBACK: {external_unlocked}"
        ));
    }

    let same_backend = owner_pid == borrowed_pid;
    println!(
        "TX02_POOL_TRANSACTION_SHARING=OBSERVED same_backend={} xact_lock_held=1 xact_lock_released=1",
        u8::from(same_backend)
    );
    // Characterization of the current unsafe multi-call usage, not a
    // desired safety contract. A future session-pinned transaction API
    // must provide its own positive isolation test and change indexer cleanup.
    assert!(
        same_backend,
        "TX-02 shared-transaction probe did not reproduce backend reuse; inspect pool behavior before interpreting cleanup risk"
    );
    let _ = external_locked;
    Ok(())
}
