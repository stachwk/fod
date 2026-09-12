// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::env;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fod_rust_hotpath::pg::{DbRepo, WriteOwnershipLease};

const FAIL_FAST_LIMIT: Duration = Duration::from_secs(1);
const SESSION_TTL_SECONDS: u64 = 120;
const WRITE_TTL_SECONDS: u64 = 30;

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

fn test_db_name() -> String {
    selected_env_value("FOD_PG_DBNAME", "POSTGRES_DB", "foddbname")
}

fn assert_isolated_test_database() {
    let dbname = test_db_name();
    assert!(
        dbname != "foddbname" && dbname.to_ascii_lowercase().contains("test"),
        "write_ownership.rs mutates test rows and must run only against an isolated test database; set FOD_PG_DBNAME/POSTGRES_DB to a dedicated database such as foddbname_hotpath_test (current database: {dbname})"
    );
}

fn conninfo() -> String {
    let dbname = test_db_name();
    let user = selected_env_value("FOD_PG_USER", "POSTGRES_USER", "foduser");
    let password = selected_env_value("FOD_PG_PASSWORD", "POSTGRES_PASSWORD", "cichosza");
    let host = selected_env_value("FOD_PG_HOST", "POSTGRES_HOST", "127.0.0.1");
    let port = selected_env_value("FOD_PG_PORT", "POSTGRES_PORT", "5432");
    format!(
        "host={host} port={port} dbname={dbname} user={user} password={password} connect_timeout=5"
    )
}

fn unique_suffix() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock went backwards")
        .as_nanos();
    (nanos as u64) ^ u64::from(std::process::id())
}

fn scalar_u64(repo: &DbRepo, sql: &str, label: &str) -> u64 {
    repo.query_scalar_text(sql)
        .unwrap_or_else(|err| panic!("{label}: {err}"))
        .trim()
        .parse::<u64>()
        .unwrap_or_else(|err| panic!("{label}: invalid integer: {err}"))
}

fn require_schema(repo: &DbRepo) {
    let ready = repo
        .query_scalar_text(
            "
            SELECT (
                to_regclass('fod.client_sessions') IS NOT NULL
                AND to_regclass('fod.destination_write_leases') IS NOT NULL
                AND to_regclass('fod.file_write_leases') IS NOT NULL
            )::text
            ",
        )
        .expect("inspect write ownership schema");
    assert_eq!(
        ready.trim(),
        "true",
        "isolated database must be initialized to FOD schema 24 before this test"
    );
}

fn create_test_file(repo: &DbRepo, suffix: u64) -> (String, u64, u64) {
    let name = format!("write-ownership-{suffix}.bin");
    let inode_seed = format!("write-ownership-{suffix}");

    repo.exec(&format!(
        "
        WITH new_object AS (
            INSERT INTO data_objects (
                file_size,
                content_hash,
                reference_count,
                creation_date,
                modification_date
            )
            VALUES (0, NULL, 1, clock_timestamp(), clock_timestamp())
            RETURNING id_data_object
        )
        INSERT INTO files (
            data_object_id,
            id_directory,
            name,
            size,
            mode,
            uid,
            gid,
            inode_seed,
            modification_date,
            access_date,
            change_date,
            creation_date
        )
        SELECT
            id_data_object,
            NULL,
            '{name}',
            0,
            '644',
            0,
            0,
            '{inode_seed}',
            clock_timestamp(),
            clock_timestamp(),
            clock_timestamp(),
            clock_timestamp()
        FROM new_object
        "
    ))
    .expect("create write ownership test file");

    let file_id = scalar_u64(
        repo,
        &format!(
            "SELECT id_file FROM files WHERE id_directory IS NULL AND name = '{name}' LIMIT 1"
        ),
        "load test file id",
    );
    let data_object_id = scalar_u64(
        repo,
        &format!("SELECT data_object_id FROM files WHERE id_file = {file_id}"),
        "load test data object id",
    );

    (name, file_id, data_object_id)
}

fn cleanup(repo: &DbRepo, session_a: u64, session_b: u64, file_id: u64, data_object_id: u64) {
    let _ = repo.exec(&format!(
        "
        DELETE FROM client_sessions WHERE session_id IN ({session_a}, {session_b});
        DELETE FROM files WHERE id_file = {file_id};
        DELETE FROM data_objects WHERE id_data_object = {data_object_id};
        "
    ));
}

fn assert_token_advanced(old: WriteOwnershipLease, new: WriteOwnershipLease) {
    assert!(
        new.destination_fencing_token > old.destination_fencing_token,
        "destination fencing token did not advance: {} -> {}",
        old.destination_fencing_token,
        new.destination_fencing_token
    );
    match (old.file_fencing_token, new.file_fencing_token) {
        (Some(old_file), Some(new_file)) => assert!(
            new_file > old_file,
            "file fencing token did not advance: {old_file} -> {new_file}"
        ),
        other => panic!("unexpected file fencing token pair: {other:?}"),
    }
}

#[test]
fn first_writer_wins_heartbeat_release_and_stale_fencing() {
    assert_isolated_test_database();

    let repo_a = DbRepo::new(&conninfo()).expect("connect repo A");
    let repo_b = DbRepo::new(&conninfo()).expect("connect repo B");

    // Odwzoruj rzeczywista sciezke runtime przed rejestracja sesji.
    repo_a
        .ensure_client_session_schema()
        .expect("ensure client session schema A");
    repo_b
        .ensure_client_session_schema()
        .expect("ensure client session schema B");
    require_schema(&repo_a);

    let suffix = unique_suffix();
    let owner_a = suffix.wrapping_add(10_001);
    let owner_b = suffix.wrapping_add(20_001);

    let session_a = repo_a
        .register_client_session(
            "write-ownership-a",
            "/tmp/write-ownership-a",
            "primary",
            "postgres_lease",
            u64::from(std::process::id()),
            SESSION_TTL_SECONDS,
        )
        .expect("register session A");

    let session_b = repo_b
        .register_client_session(
            "write-ownership-b",
            "/tmp/write-ownership-b",
            "primary",
            "postgres_lease",
            u64::from(std::process::id()).saturating_add(1),
            SESSION_TTL_SECONDS,
        )
        .expect("register session B");

    repo_a
        .touch_client_session_owner_key(session_a, owner_a)
        .expect("touch owner A");
    repo_b
        .touch_client_session_owner_key(session_b, owner_b)
        .expect("touch owner B");

    // Tworz zasob dopiero po poprawnym zestawieniu obu sesji.
    let (name, file_id, data_object_id) = create_test_file(&repo_a, suffix);

    let result = (|| -> Result<(), String> {
        let lease_a = repo_a
            .acquire_write_ownership(None, &name, Some(file_id), owner_a, WRITE_TTL_SECONDS)?
            .ok_or_else(|| "writer A failed to acquire free destination".to_string())?;

        let loser_started = Instant::now();
        let loser = repo_b.acquire_write_ownership(
            None,
            &name,
            Some(file_id),
            owner_b,
            WRITE_TTL_SECONDS,
        )?;
        let loser_elapsed = loser_started.elapsed();

        if loser.is_some() {
            return Err("writer B acquired destination while writer A was active".to_string());
        }
        if loser_elapsed >= FAIL_FAST_LIMIT {
            return Err(format!(
                "writer B did not fail fast: elapsed={loser_elapsed:?}"
            ));
        }

        let heartbeat_ok = repo_a.heartbeat_write_ownership(
            None,
            &name,
            Some(file_id),
            owner_a,
            lease_a,
            WRITE_TTL_SECONDS,
        )?;
        if !heartbeat_ok {
            return Err("writer A heartbeat unexpectedly lost ownership".to_string());
        }

        let destination_token_db = scalar_u64(
            &repo_a,
            &format!(
                "SELECT fencing_token FROM destination_write_leases                  WHERE parent_key = 0 AND name = '{name}'"
            ),
            "destination token after heartbeat",
        );
        if destination_token_db != lease_a.destination_fencing_token {
            return Err(format!(
                "heartbeat changed destination fencing token: {} -> {}",
                lease_a.destination_fencing_token, destination_token_db
            ));
        }

        let file_token_db = scalar_u64(
            &repo_a,
            &format!("SELECT fencing_token FROM file_write_leases WHERE file_id = {file_id}"),
            "file token after heartbeat",
        );
        if Some(file_token_db) != lease_a.file_fencing_token {
            return Err(format!(
                "heartbeat changed file fencing token: {:?} -> {file_token_db}",
                lease_a.file_fencing_token
            ));
        }

        repo_a.release_write_ownership(owner_a, lease_a)?;

        let lease_b = repo_b
            .acquire_write_ownership(None, &name, Some(file_id), owner_b, WRITE_TTL_SECONDS)?
            .ok_or_else(|| "writer B failed to acquire after release".to_string())?;
        assert_token_advanced(lease_a, lease_b);

        let stale_after_release = repo_a.heartbeat_write_ownership(
            None,
            &name,
            Some(file_id),
            owner_a,
            lease_a,
            WRITE_TTL_SECONDS,
        )?;
        if stale_after_release {
            return Err("stale writer A refreshed ownership after writer B takeover".to_string());
        }

        repo_b.release_write_ownership(owner_b, lease_b)?;

        let lease_a_expiring = repo_a
            .acquire_write_ownership(None, &name, Some(file_id), owner_a, WRITE_TTL_SECONDS)?
            .ok_or_else(|| "writer A failed to reacquire before expiry test".to_string())?;

        repo_a.exec(&format!(
            "
            UPDATE destination_write_leases
            SET lease_expires_at = clock_timestamp() - interval '1 second'
            WHERE fencing_token = {};
            UPDATE file_write_leases
            SET lease_expires_at = clock_timestamp() - interval '1 second'
            WHERE fencing_token = {};
            ",
            lease_a_expiring.destination_fencing_token,
            lease_a_expiring
                .file_fencing_token
                .expect("file token for expiring lease")
        ))?;

        let lease_b_after_expiry = repo_b
            .acquire_write_ownership(None, &name, Some(file_id), owner_b, WRITE_TTL_SECONDS)?
            .ok_or_else(|| "writer B failed to acquire expired ownership".to_string())?;
        assert_token_advanced(lease_a_expiring, lease_b_after_expiry);

        let stale_after_expiry = repo_a.heartbeat_write_ownership(
            None,
            &name,
            Some(file_id),
            owner_a,
            lease_a_expiring,
            WRITE_TTL_SECONDS,
        )?;
        if stale_after_expiry {
            return Err("expired stale writer A refreshed ownership after takeover".to_string());
        }

        repo_b.release_write_ownership(owner_b, lease_b_after_expiry)?;

        println!(
            "OK write ownership loser_elapsed_ms={} release_tokens={}->{} expiry_tokens={}->{}",
            loser_elapsed.as_millis(),
            lease_a.destination_fencing_token,
            lease_b.destination_fencing_token,
            lease_a_expiring.destination_fencing_token,
            lease_b_after_expiry.destination_fencing_token
        );

        Ok(())
    })();

    cleanup(&repo_a, session_a, session_b, file_id, data_object_id);

    if let Err(err) = result {
        panic!("{err}");
    }
}
