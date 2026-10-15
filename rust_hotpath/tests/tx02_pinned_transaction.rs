// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1
//
// TX-02B: session-pinned multi-statement transaction and rollback.
//
// This suite uses only transaction-scoped locks and SET LOCAL. No FOD
// namespace, file-content or lease rows are modified.

use fod_rust_hotpath::pg::DbRepo;
use std::env;
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn selected(primary: &str, legacy: &str, fallback: &str) -> String {
    env::var(primary)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var(legacy)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| fallback.to_string())
}

fn connection_string() -> String {
    let database = selected("FOD_PG_DBNAME", "POSTGRES_DB", "foddbname");
    assert!(
        database != "foddbname" && database.to_ascii_lowercase().contains("test"),
        "TX-02B must run against an isolated test database; got {database}"
    );
    let host = selected("FOD_PG_HOST", "POSTGRES_HOST", "127.0.0.1");
    let port = selected("FOD_PG_PORT", "POSTGRES_PORT", "5432");
    let user = selected("FOD_PG_USER", "POSTGRES_USER", "foduser");
    let password = selected("FOD_PG_PASSWORD", "POSTGRES_PASSWORD", "cichosza");
    format!(
        "host={host} port={port} dbname={database} user={user} password={password} connect_timeout=5"
    )
}

fn scalar(repo: &DbRepo, query: &str) -> Result<String, String> {
    Ok(repo.query_scalar_text(query)?.trim().to_ascii_lowercase())
}

fn fresh_advisory_key() -> Result<i64, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_nanos();
    Ok(((now as u64) ^ u64::from(std::process::id())) as i64)
}

#[test]
fn tx02_pinned_session_blocks_cross_thread_transaction_borrow() -> Result<(), String> {
    let repo = Arc::new(DbRepo::new(&connection_string())?);
    let observer = DbRepo::new(&connection_string())?;
    let pool_limit = repo.observability_snapshot()?.pool.connection_limit;
    if pool_limit < 2 {
        return Err(format!(
            "TX-02B requires at least two pooled connections, configured limit={pool_limit}"
        ));
    }
    let key = fresh_advisory_key()?;
    let before_application_name = scalar(&repo, "SHOW application_name")?;

    let owner_pid = repo.with_transaction(|tx| {
        let before = tx.query_scalar_text("SELECT pg_backend_pid()")?;
        tx.exec("SET LOCAL application_name = 'fod_tx02_pinned'")?;
        let inside = tx.query_scalar_text("SHOW application_name")?;
        if inside.trim() != "fod_tx02_pinned" {
            return Err(format!(
                "SET LOCAL not visible inside pinned transaction: {inside}"
            ));
        }
        let acquired = tx.query_scalar_text(&format!("SELECT pg_try_advisory_xact_lock({key})"))?;
        if !matches!(acquired.trim(), "true" | "t") {
            return Err(format!(
                "TX-02B failed to acquire advisory lock: {acquired}"
            ));
        }

        let external = scalar(
            &observer,
            &format!("SELECT pg_try_advisory_xact_lock({key})"),
        )?;
        if !matches!(external.as_str(), "false" | "f") {
            return Err(format!("TX-02B expected held lock, observed {external}"));
        }

        let worker = Arc::clone(&repo);
        let other_pid = thread::spawn(move || scalar(&worker, "SELECT pg_backend_pid()"))
            .join()
            .map_err(|_| "TX-02B worker thread panicked".to_string())??;
        if other_pid == before.trim() {
            return Err(format!(
                "TX-02B worker borrowed pinned backend {other_pid} while transaction active"
            ));
        }

        let after = tx.query_scalar_text("SELECT pg_backend_pid()")?;
        if after.trim() != before.trim() {
            return Err(format!(
                "TX-02B transaction changed backend: {} -> {}",
                before.trim(),
                after.trim()
            ));
        }
        Ok(before.trim().to_string())
    })?;

    let released = scalar(
        &observer,
        &format!("SELECT pg_try_advisory_xact_lock({key})"),
    )?;
    if !matches!(released.as_str(), "true" | "t") {
        return Err(format!("TX-02B lock not released after commit: {released}"));
    }
    let after_application_name = scalar(&repo, "SHOW application_name")?;
    if after_application_name != before_application_name {
        return Err(format!(
            "TX-02B SET LOCAL leaked after commit: {after_application_name}"
        ));
    }
    println!(
        "TX02_PINNED_COMMIT=PASS owner_pid={} worker_separate=1 lock_released=1",
        owner_pid
    );
    Ok(())
}

#[test]
fn tx02_pinned_error_rolls_back_and_releases_lock() -> Result<(), String> {
    let repo = DbRepo::new(&connection_string())?;
    let observer = DbRepo::new(&connection_string())?;
    let key = fresh_advisory_key()?;
    let before_application_name = scalar(&repo, "SHOW application_name")?;
    let result: Result<(), String> = repo.with_transaction(|tx| {
        tx.exec("SET LOCAL application_name = 'fod_tx02_rollback'")?;
        let acquired = tx.query_scalar_text(&format!("SELECT pg_try_advisory_xact_lock({key})"))?;
        if !matches!(acquired.trim(), "true" | "t") {
            return Err(format!(
                "TX-02B failed to acquire rollback lock: {acquired}"
            ));
        }
        let held = scalar(
            &observer,
            &format!("SELECT pg_try_advisory_xact_lock({key})"),
        )?;
        if !matches!(held.as_str(), "false" | "f") {
            return Err(format!("TX-02B rollback lock is not held: {held}"));
        }
        Err("TX02_INTENTIONAL_ROLLBACK".to_string())
    });
    if result.as_ref().err().map(String::as_str) != Some("TX02_INTENTIONAL_ROLLBACK") {
        return Err(format!("TX-02B unexpected rollback result: {result:?}"));
    }

    let unlocked = scalar(
        &observer,
        &format!("SELECT pg_try_advisory_xact_lock({key})"),
    )?;
    if !matches!(unlocked.as_str(), "true" | "t") {
        return Err(format!("TX-02B rollback did not release lock: {unlocked}"));
    }
    let application_name = scalar(&repo, "SHOW application_name")?;
    if application_name != before_application_name {
        return Err(format!(
            "TX-02B rollback leaked SET LOCAL: {application_name}"
        ));
    }
    println!("TX02_PINNED_ROLLBACK=PASS lock_released=1 local_setting_reset=1");
    Ok(())
}
