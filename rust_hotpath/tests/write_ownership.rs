// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::env;
use std::fs;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fod_rust_hotpath::pg::{
    DbRepo, PersistBlockRow, WriteOwnershipLease, WritePersistenceFence,
    WRITE_OWNERSHIP_FENCE_REJECTED_PREFIX,
};

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

// Proces pomocniczy C2 jest uruchamiany oddzielnie przez test nadrzedny.
// Domyslnie jest ignorowany, aby nie wykonywac przypadkowych operacji.
#[test]
#[ignore = "Uruchamiany przez c2_independent_processes"]
fn c2_worker_entrypoint() {
    assert_isolated_test_database();

    let mode = env::var("FOD_C2_MODE").expect("C2 mode");
    let name = env::var("FOD_C2_NAME").expect("C2 name");
    let file_id: u64 = env::var("FOD_C2_FILE_ID")
        .expect("C2 file ID")
        .parse()
        .expect("numeric C2 file ID");
    let owner: u64 = env::var("FOD_C2_OWNER")
        .expect("C2 owner")
        .parse()
        .expect("numeric C2 owner");

    let repo = DbRepo::new(&conninfo()).expect("C2 worker connect");
    require_schema(&repo);

    // Kazdy proces otrzymuje wlasna sesje i unikatowy klucz wlasciciela.
    let session = repo
        .register_client_session(
            "write-ownership-c2-worker",
            "/tmp/write-ownership-c2-worker",
            "primary",
            "postgres_lease",
            u64::from(std::process::id()),
            SESSION_TTL_SECONDS,
        )
        .expect("C2 register worker session");
    repo.touch_client_session_owner_key(session, owner)
        .expect("C2 touch worker owner");

    let result = (|| -> Result<(), String> {
        let started = Instant::now();
        let lease =
            repo.acquire_write_ownership(None, &name, Some(file_id), owner, WRITE_TTL_SECONDS)?;

        match mode.as_str() {
            "reject" => {
                // W razie regresji nie pozostawiamy przypadkowej dzierzawy.
                if let Some(unexpected) = lease {
                    repo.release_write_ownership(owner, unexpected)?;
                    return Err("C2: second process acquired active ownership".into());
                }
                if started.elapsed() >= FAIL_FAST_LIMIT {
                    return Err("C2: second process did not fail fast".into());
                }
                println!("C2_WORKER_REJECT=PASS");
            }
            "takeover" => {
                let lease = lease.ok_or_else(|| {
                    "C2: second process could not acquire released ownership".to_string()
                })?;
                // Zwracamy obydwa tokeny do procesu nadrzednego.
                println!(
                    "C2_WORKER_TOKENS={} {}",
                    lease.destination_fencing_token,
                    lease
                        .file_fencing_token
                        .ok_or_else(|| "C2: missing file fencing token".to_string())?
                );
                repo.release_write_ownership(owner, lease)?;
                println!("C2_WORKER_TAKEOVER=PASS");
            }
            "hold" => {
                let lease = lease.ok_or_else(|| "C3: holder could not acquire".to_string())?;
                let ready = env::var("FOD_C3_READY_FILE").map_err(|err| err.to_string())?;
                // Sygnal gotowosci jest zapisywany dopiero po uzyskaniu dzierzawy.
                fs::write(
                    ready,
                    format!(
                        "{} {} {} {}",
                        session,
                        lease.destination_fencing_token,
                        lease
                            .file_fencing_token
                            .ok_or_else(|| "C3: missing file token".to_string())?,
                        owner
                    ),
                )
                .map_err(|err| err.to_string())?;
                // Proces nadrzedny zakonczy nas SIGKILL, bez zwalniania dzierzawy.
                thread::sleep(Duration::from_secs(60));
                return Err("C3: holder survived without SIGKILL".into());
            }
            _ => return Err(format!("C2: unsupported mode {mode}")),
        }
        Ok(())
    })();

    // Sprzatamy sesje rowniez, gdy samo sprawdzenie nie przeszlo.
    let cleanup_result = repo.exec(&format!(
        "DELETE FROM client_sessions WHERE session_id = {session}"
    ));
    if let Err(err) = result {
        panic!("{err}");
    }
    cleanup_result.expect("C2 worker session cleanup");
}

// C2: odrebne procesy OS konkuruja o jeden zasob, PostgreSQL rozstrzyga wlasnosc.
#[test]
fn c2_independent_processes() {
    assert_isolated_test_database();
    let repo = DbRepo::new(&conninfo()).expect("C2 parent connect");
    repo.ensure_client_session_schema()
        .expect("C2 ensure session schema");
    require_schema(&repo);

    let suffix = unique_suffix();
    let owner_a = suffix.wrapping_add(30_001);
    let owner_b = suffix.wrapping_add(40_001);
    let session_a = repo
        .register_client_session(
            "write-ownership-c2-parent",
            "/tmp/write-ownership-c2-parent",
            "primary",
            "postgres_lease",
            u64::from(std::process::id()),
            SESSION_TTL_SECONDS,
        )
        .expect("C2 register parent session");
    repo.touch_client_session_owner_key(session_a, owner_a)
        .expect("C2 touch parent owner");

    let (name, file_id, data_object_id) = create_test_file(&repo, suffix);
    let result = (|| -> Result<(), String> {
        // A uzyskuje dzierzawe zanim B zostanie uruchomiony.
        let lease_a = repo
            .acquire_write_ownership(None, &name, Some(file_id), owner_a, WRITE_TTL_SECONDS)?
            .ok_or_else(|| "C2: parent failed to acquire".to_string())?;

        let run_worker = |mode: &str| -> Result<String, String> {
            let binary = env::current_exe().map_err(|err| err.to_string())?;
            let output = Command::new(binary)
                .args([
                    "--exact",
                    "c2_worker_entrypoint",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("FOD_C2_MODE", mode)
                .env("FOD_C2_NAME", &name)
                .env("FOD_C2_FILE_ID", file_id.to_string())
                .env("FOD_C2_OWNER", owner_b.to_string())
                .output()
                .map_err(|err| format!("C2 worker {mode} launch: {err}"))?;

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !output.status.success() {
                return Err(format!("C2 worker {mode} failed:\n{stdout}\n{stderr}"));
            }
            let marker = match mode {
                "reject" => "C2_WORKER_REJECT=PASS",
                "takeover" => "C2_WORKER_TAKEOVER=PASS",
                _ => return Err(format!("C2: unsupported worker mode {mode}")),
            };
            if !stdout.contains(marker) {
                return Err(format!("C2 worker {mode} missing marker:\n{stdout}"));
            }
            Ok(stdout)
        };

        // B musi szybko przegrac z aktywnym A.
        run_worker("reject")?;
        repo.release_write_ownership(owner_a, lease_a)?;

        // B przejmuje zwolniony zasob; porownujemy tokeny obu poziomow.
        let stdout = run_worker("takeover")?;
        let tokens_line = stdout
            .lines()
            .find_map(|line| {
                line.split_once("C2_WORKER_TOKENS=")
                    .map(|(_, value)| value.trim())
            })
            .ok_or_else(|| format!("C2: worker tokens missing:\n{stdout}"))?;
        let tokens = tokens_line
            .split_whitespace()
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("C2: invalid worker tokens: {err}"))?;
        if tokens.len() != 2 {
            return Err(format!("C2: expected two fencing tokens, got {tokens:?}"));
        }
        if tokens[0] <= lease_a.destination_fencing_token
            || Some(tokens[1]) <= lease_a.file_fencing_token
        {
            return Err(format!(
                "C2: fencing did not advance: old={:?}, new={tokens:?}",
                lease_a
            ));
        }

        // Stary wlasciciel nie moze odnowic nieaktualnego tokenu.
        if repo.heartbeat_write_ownership(
            None,
            &name,
            Some(file_id),
            owner_a,
            lease_a,
            WRITE_TTL_SECONDS,
        )? {
            return Err("C2: stale parent heartbeat unexpectedly succeeded".into());
        }
        println!(
            "C2_PROCESSES=PASS fencing={}->{}",
            lease_a.destination_fencing_token, tokens[0]
        );
        Ok(())
    })();

    // Nie zostawiamy plikow ani sesji w bazie testowej.
    cleanup(&repo, session_a, session_a, file_id, data_object_id);
    if let Err(err) = result {
        panic!("{err}");
    }
}

// C3: awaria procesu-wlasciciela i odzyskanie po wygasnieciu lease.
// Wygasniecie jest przyspieszane wylacznie w izolowanej bazie testowej.
#[cfg(unix)]
#[test]
fn c3_sigkill_holder_then_reacquire() {
    assert_isolated_test_database();
    let repo = DbRepo::new(&conninfo()).expect("C3 parent connect");
    repo.ensure_client_session_schema()
        .expect("C3 ensure schema");
    require_schema(&repo);

    let suffix = unique_suffix();
    let owner_b = suffix.wrapping_add(50_001);
    let session_b = repo
        .register_client_session(
            "write-ownership-c3-parent",
            "/tmp/write-ownership-c3-parent",
            "primary",
            "postgres_lease",
            u64::from(std::process::id()),
            SESSION_TTL_SECONDS,
        )
        .expect("C3 register parent session");
    repo.touch_client_session_owner_key(session_b, owner_b)
        .expect("C3 touch parent owner");
    let (name, file_id, data_object_id) = create_test_file(&repo, suffix);
    let ready = env::temp_dir().join(format!("fod-c3-ready-{}-{suffix}", std::process::id()));
    let _ = fs::remove_file(&ready);
    let mut child = Command::new(env::current_exe().expect("C3 executable"))
        .args([
            "--exact",
            "c2_worker_entrypoint",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("FOD_C2_MODE", "hold")
        .env("FOD_C2_NAME", &name)
        .env("FOD_C2_FILE_ID", file_id.to_string())
        .env("FOD_C2_OWNER", suffix.wrapping_add(60_001).to_string())
        .env("FOD_C3_READY_FILE", &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("C3 spawn holder");

    // Ograniczony czas oczekiwania i bezwarunkowe zakonczenie dziecka.
    let result = (|| -> Result<(), String> {
        let started = Instant::now();
        while !ready.exists() {
            if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
                return Err(format!("C3 holder exited early: {status}"));
            }
            if started.elapsed() > Duration::from_secs(5) {
                return Err("C3 holder readiness timeout".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        let contents = fs::read_to_string(&ready).map_err(|err| err.to_string())?;
        let values = contents
            .split_whitespace()
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("C3 invalid readiness tokens: {err}"))?;
        if values.len() != 4 {
            return Err(format!("C3 invalid readiness payload: {contents}"));
        }
        let (session_a, destination_token, file_token, owner_a) =
            (values[0], values[1], values[2], values[3]);

        child.kill().map_err(|err| format!("C3 SIGKILL: {err}"))?;
        child.wait().map_err(|err| format!("C3 wait: {err}"))?;

        // Sam SIGKILL nie upowaznia drugiego procesu do przejecia.
        if repo
            .acquire_write_ownership(None, &name, Some(file_id), owner_b, WRITE_TTL_SECONDS)?
            .is_some()
        {
            return Err("C3: resource acquired before lease expiry".into());
        }

        // Testowe przyspieszenie zegara dzierzawy, bez 30-sekundowego sleep.
        repo.exec(&format!(
            "UPDATE destination_write_leases SET lease_expires_at = clock_timestamp() - interval '1 second' WHERE name = '{name}' AND fencing_token = {destination_token};              UPDATE file_write_leases SET lease_expires_at = clock_timestamp() - interval '1 second' WHERE file_id = {file_id} AND fencing_token = {file_token};"
        ))?;

        let lease_b = repo
            .acquire_write_ownership(None, &name, Some(file_id), owner_b, WRITE_TTL_SECONDS)?
            .ok_or_else(|| "C3: could not acquire after expiry".to_string())?;
        assert!(
            lease_b.destination_fencing_token > destination_token
                && lease_b.file_fencing_token > Some(file_token),
            "C3: fencing tokens did not advance"
        );
        // Stary token musi pozostac nieaktywny nawet po przejeciu przez B.
        let old = WriteOwnershipLease {
            destination_fencing_token: destination_token,
            file_fencing_token: Some(file_token),
        };
        if repo.heartbeat_write_ownership(
            None,
            &name,
            Some(file_id),
            owner_a,
            old,
            WRITE_TTL_SECONDS,
        )? {
            return Err("C3: stale holder heartbeat succeeded".into());
        }
        repo.release_write_ownership(owner_b, lease_b)?;
        repo.exec(&format!(
            "DELETE FROM client_sessions WHERE session_id = {session_a}"
        ))?;
        println!(
            "C3_SIGKILL=PASS fencing={destination_token}->{}",
            lease_b.destination_fencing_token
        );
        Ok(())
    })();

    // Sprzatanie po bledzie, lacznie z przypadkiem timeout.
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_file(&ready);
    let _ = repo.exec(&format!(
        "DELETE FROM client_sessions WHERE session_id IN          (SELECT session_id FROM client_session_owner_keys WHERE owner_key = {})",
        suffix.wrapping_add(60_001)
    ));
    // Awaryjne usuniecie dzierzaw dotyczy tylko identyfikatora testowego pliku.
    let _ = repo.exec(&format!(
        "DELETE FROM destination_write_leases WHERE name = '{name}';          DELETE FROM file_write_leases WHERE file_id = {file_id};"
    ));
    cleanup(&repo, session_b, session_b, file_id, data_object_id);
    if let Err(err) = result {
        panic!("{err}");
    }
}

// C4 uzywa rzeczywistej sciezki utrwalania blokow, bez recznego UPDATE data_blocks.
fn c4_persist_payload(
    repo: &DbRepo,
    file_id: u64,
    block_size: u64,
    data: &[u8],
    fence: WritePersistenceFence,
) -> Result<(), String> {
    let blocks = [PersistBlockRow {
        block_index: 0,
        data,
        used_len: block_size,
    }];
    repo.persist_file_blocks_with_crc_flag_reservation_and_fence(
        file_id,
        block_size,
        block_size,
        1,
        true,
        &blocks,
        false,
        None,
        Some(fence),
    )
}

// C4 sprawdza bajty odczytane przez API PostgreSQL, a nie stan lokalnego bufora.
fn c4_assert_payload(
    repo: &DbRepo,
    file_id: u64,
    block_size: u64,
    expected: &[u8],
) -> Result<(), String> {
    let actual = repo
        .load_block(file_id, 0, block_size)?
        .ok_or_else(|| "C4: persisted data block is missing".to_string())?;
    if actual != expected {
        return Err("C4: persisted payload differs from expected bytes".into());
    }
    Ok(())
}

// C4: przejecie wlasnosci nie moze pozwolic staremu klientowi nadpisac danych.
#[test]
fn c4_stale_fence_rejects_persisted_payload() {
    assert_isolated_test_database();

    let repo_a = DbRepo::new(&conninfo()).expect("C4 connect A");
    let repo_b = DbRepo::new(&conninfo()).expect("C4 connect B");
    repo_a
        .ensure_client_session_schema()
        .expect("C4 ensure session schema A");
    repo_b
        .ensure_client_session_schema()
        .expect("C4 ensure session schema B");
    require_schema(&repo_a);

    let block_size = scalar_u64(
        &repo_a,
        "SELECT value FROM config WHERE key = 'block_size'",
        "C4 read configured block size",
    );
    assert!(
        (1024..=1024 * 1024).contains(&block_size),
        "C4: unexpected test block size: {block_size}"
    );
    let block_size_usize = usize::try_from(block_size).expect("C4 block size fits usize");
    let data_a = vec![b'A'; block_size_usize];
    let data_b = vec![b'B'; block_size_usize];
    let data_stale = vec![b'C'; block_size_usize];

    let suffix = unique_suffix();
    let owner_a = suffix.wrapping_add(70_001);
    let owner_b = suffix.wrapping_add(80_001);
    let session_a = repo_a
        .register_client_session(
            "write-ownership-c4-a",
            "/tmp/write-ownership-c4-a",
            "primary",
            "postgres_lease",
            u64::from(std::process::id()),
            SESSION_TTL_SECONDS,
        )
        .expect("C4 register A");
    let session_b = repo_b
        .register_client_session(
            "write-ownership-c4-b",
            "/tmp/write-ownership-c4-b",
            "primary",
            "postgres_lease",
            u64::from(std::process::id()).saturating_add(1),
            SESSION_TTL_SECONDS,
        )
        .expect("C4 register B");
    repo_a
        .touch_client_session_owner_key(session_a, owner_a)
        .expect("C4 touch owner A");
    repo_b
        .touch_client_session_owner_key(session_b, owner_b)
        .expect("C4 touch owner B");

    let (name, file_id, initial_data_object_id) = create_test_file(&repo_a, suffix);
    // Zapis pelnego bloku moze zamieniac data_object_id (copy-on-write).
    let mut data_object_ids = vec![initial_data_object_id];

    let result = (|| -> Result<(), String> {
        let lease_a = repo_a
            .acquire_write_ownership(None, &name, Some(file_id), owner_a, WRITE_TTL_SECONDS)?
            .ok_or_else(|| "C4: writer A could not acquire".to_string())?;
        let file_token_a = lease_a
            .file_fencing_token
            .ok_or_else(|| "C4: writer A has no file fencing token".to_string())?;
        let fence_a = WritePersistenceFence {
            session_id: session_a,
            owner_key: owner_a,
            file_fencing_token: file_token_a,
        };

        // A utrwala dane z aktywnym tokenem i odczytuje je przez PostgreSQL.
        c4_persist_payload(&repo_a, file_id, block_size, &data_a, fence_a)?;
        data_object_ids.push(scalar_u64(
            &repo_a,
            &format!("SELECT data_object_id FROM files WHERE id_file = {file_id}"),
            "C4 data object after A",
        ));
        c4_assert_payload(&repo_b, file_id, block_size, &data_a)?;

        // B nie moze przejac aktywnego lease A.
        if repo_b
            .acquire_write_ownership(None, &name, Some(file_id), owner_b, WRITE_TTL_SECONDS)?
            .is_some()
        {
            return Err("C4: B acquired active A ownership".into());
        }

        // Przyspieszamy wygasniecie tylko w izolowanej bazie testowej.
        repo_a.exec(&format!(
            "UPDATE destination_write_leases \
             SET lease_expires_at = clock_timestamp() - interval '1 second' \
             WHERE parent_key = 0 AND name = '{name}' \
               AND fencing_token = {}; \
             UPDATE file_write_leases \
             SET lease_expires_at = clock_timestamp() - interval '1 second' \
             WHERE file_id = {file_id} AND fencing_token = {file_token_a};",
            lease_a.destination_fencing_token
        ))?;

        let lease_b = repo_b
            .acquire_write_ownership(None, &name, Some(file_id), owner_b, WRITE_TTL_SECONDS)?
            .ok_or_else(|| "C4: B could not acquire expired ownership".to_string())?;
        assert_token_advanced(lease_a, lease_b);
        let file_token_b = lease_b
            .file_fencing_token
            .ok_or_else(|| "C4: writer B has no file fencing token".to_string())?;
        let fence_b = WritePersistenceFence {
            session_id: session_b,
            owner_key: owner_b,
            file_fencing_token: file_token_b,
        };

        // B zatwierdza swoj blok. Musi on pozostac niezmieniony po probie A.
        c4_persist_payload(&repo_b, file_id, block_size, &data_b, fence_b)?;
        let object_after_b = scalar_u64(
            &repo_b,
            &format!("SELECT data_object_id FROM files WHERE id_file = {file_id}"),
            "C4 data object after B",
        );
        data_object_ids.push(object_after_b);
        c4_assert_payload(&repo_a, file_id, block_size, &data_b)?;

        // A probuje zapisac CCCC z nieaktualnym tokenem A.
        let stale_result = c4_persist_payload(&repo_a, file_id, block_size, &data_stale, fence_a);
        let stale_error = match stale_result {
            Ok(()) => return Err("C4: stale writer A persisted data".into()),
            Err(err) => err,
        };
        if !stale_error.contains(WRITE_OWNERSHIP_FENCE_REJECTED_PREFIX) {
            return Err(format!(
                "C4: stale write failed for an unexpected reason: {stale_error}"
            ));
        }

        // Sprawdzamy bajty, liczbe blokow oraz brak zmiany data_object_id.
        c4_assert_payload(&repo_b, file_id, block_size, &data_b)?;
        let object_after_rejection = scalar_u64(
            &repo_b,
            &format!("SELECT data_object_id FROM files WHERE id_file = {file_id}"),
            "C4 data object after stale rejection",
        );
        if object_after_rejection != object_after_b {
            return Err("C4: stale write changed data_object_id".into());
        }
        if repo_b.count_file_blocks(file_id)? != 1 {
            return Err("C4: expected exactly one persisted block".into());
        }
        let metadata = repo_b
            .file_read_metadata(file_id)?
            .ok_or_else(|| "C4: file metadata missing".to_string())?;
        if metadata.size != block_size {
            return Err("C4: persisted file size changed".into());
        }

        // A po utracie lease nie moze rowniez odnowic starego tokenu.
        if repo_a.heartbeat_write_ownership(
            None,
            &name,
            Some(file_id),
            owner_a,
            lease_a,
            WRITE_TTL_SECONDS,
        )? {
            return Err("C4: stale A heartbeat succeeded".into());
        }
        repo_b.release_write_ownership(owner_b, lease_b)?;
        println!(
            "C4_STALE_PERSIST=PASS fencing={}->{} bytes={} stale_rejected=1",
            file_token_a, file_token_b, block_size
        );
        Ok(())
    })();

    // Sprzatanie obiektow utworzonych przez copy-on-write, nawet po bledzie testu.
    if let Ok(current) = repo_a.query_scalar_text(&format!(
        "SELECT data_object_id FROM files WHERE id_file = {file_id}"
    )) {
        if let Ok(id) = current.trim().parse::<u64>() {
            data_object_ids.push(id);
        }
    }
    data_object_ids.sort_unstable();
    data_object_ids.dedup();
    let object_ids = data_object_ids
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let cleanup_result = repo_a.exec(&format!(
        "DELETE FROM destination_write_leases WHERE parent_key = 0 AND name = '{name}'; \
         DELETE FROM file_write_leases WHERE file_id = {file_id}; \
         DELETE FROM client_sessions WHERE session_id IN ({session_a}, {session_b}); \
         DELETE FROM files WHERE id_file = {file_id}; \
         DELETE FROM data_objects WHERE id_data_object IN ({object_ids});"
    ));
    if let Err(err) = cleanup_result {
        panic!("C4 cleanup failed: {err}; test result: {result:?}");
    }
    if let Err(err) = result {
        panic!("{err}");
    }
}

// C5: proces A zapisuje blok podczas kontrolowanego wyczekiwania na blokade w PG.
// Wlasna sesja procesu i identyfikator aplikacji pozwalaja obserwowac rzeczywisty zapis.
#[cfg(unix)]
#[test]
#[ignore = "Uruchamiany przez c5_inflight_persist_blocks_takeover_until_commit"]
fn c5_persist_worker_entrypoint() {
    assert_isolated_test_database();
    let name = env::var("FOD_C5_NAME").expect("C5 name");
    let ready = env::var("FOD_C5_READY_FILE").expect("C5 ready path");
    let app_name = env::var("FOD_C5_WRITER_APP").expect("C5 writer app");
    let file_id: u64 = env::var("FOD_C5_FILE_ID")
        .expect("C5 file ID")
        .parse()
        .expect("C5 numeric file ID");
    let owner: u64 = env::var("FOD_C5_OWNER")
        .expect("C5 owner")
        .parse()
        .expect("C5 numeric owner");
    let repo = DbRepo::new(&format!("{} application_name={app_name}", conninfo()))
        .expect("C5 connect writer");
    require_schema(&repo);
    let session = repo
        .register_client_session(
            "write-ownership-c5-writer",
            "/tmp/write-ownership-c5-writer",
            "primary",
            "postgres_lease",
            u64::from(std::process::id()),
            SESSION_TTL_SECONDS,
        )
        .expect("C5 register writer session");
    repo.touch_client_session_owner_key(session, owner)
        .expect("C5 touch writer owner");
    let lease = repo
        .acquire_write_ownership(None, &name, Some(file_id), owner, WRITE_TTL_SECONDS)
        .expect("C5 writer acquire query")
        .expect("C5 writer acquire lease");
    let token = lease.file_fencing_token.expect("C5 file token");
    let fence = WritePersistenceFence {
        session_id: session,
        owner_key: owner,
        file_fencing_token: token,
    };
    let block_size = scalar_u64(
        &repo,
        "SELECT value FROM config WHERE key = 'block_size'",
        "C5 writer block size",
    );
    // Sygnal zawiera identyfikatory dzierzawy przed wywolaniem prawdziwego persist.
    fs::write(
        ready,
        format!("{session} {} {token} {owner}", lease.destination_fencing_token),
    )
    .expect("C5 publish ready");
    let data = vec![b'A'; usize::try_from(block_size).expect("C5 block size usize")];
    c4_persist_payload(&repo, file_id, block_size, &data, fence)
        .expect("C5 writer persist");
    // Celowo zostawiamy wygasly lease i aktywna sesje do czasu przejecia przez B.
    // Test nadrzedny usuwa sesje po sprawdzeniu odrzucenia starego tokenu.
    println!("C5_WORKER_COMMIT=PASS");
}

// C5: testowa obserwacja stanu backendu PG o unikatowym application_name.
#[cfg(unix)]
fn c5_wait_for_pg_activity(
    repo: &DbRepo,
    application: &str,
    event_type: &str,
    event_name: &str,
) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        let active = repo.query_scalar_text(&format!(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
             WHERE datname = current_database() \
               AND application_name = '{application}' \
               AND wait_event_type = '{event_type}' \
               AND ('{event_name}' = '*' OR wait_event = '{event_name}'))::text"
        ))?;
        if matches!(active.trim(), "t" | "true") {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err(format!(
        "C5: backend {application} did not reach {event_type}/{event_name}"
    ))
}

// C5: transakcja A posiada advisory lock i czeka na blokade rekordu.
// B musi przegrac nawet wtedy, gdy dzierzawa A wygasla w trakcie zapisu.
#[cfg(unix)]
#[test]
fn c5_inflight_persist_blocks_takeover_until_commit() {
    assert_isolated_test_database();
    let repo_b = DbRepo::new(&conninfo()).expect("C5 connect contender");
    repo_b
        .ensure_client_session_schema()
        .expect("C5 ensure schema");
    require_schema(&repo_b);

    let block_size = scalar_u64(
        &repo_b,
        "SELECT value FROM config WHERE key = 'block_size'",
        "C5 read block size",
    );
    assert!((1024..=1024 * 1024).contains(&block_size));
    let suffix = unique_suffix();
    let owner_a = suffix.wrapping_add(90_001);
    let owner_b = suffix.wrapping_add(100_001);
    let (name, file_id, initial_object) = create_test_file(&repo_b, suffix);
    let session_b = repo_b
        .register_client_session(
            "write-ownership-c5-contender",
            "/tmp/write-ownership-c5-contender",
            "primary",
            "postgres_lease",
            u64::from(std::process::id()),
            SESSION_TTL_SECONDS,
        )
        .expect("C5 register contender");
    repo_b
        .touch_client_session_owner_key(session_b, owner_b)
        .expect("C5 touch contender owner");

    let ready = env::temp_dir().join(format!("fod-c5-ready-{}-{suffix}", std::process::id()));
    let _ = fs::remove_file(&ready);
    let guard_name = format!("fod_c5_guard_{suffix}");
    let writer_name = format!("fod_c5_writer_{suffix}");

    // Oddzielne polaczenie blokuje rekord pliku, ale nie dzierzawy.
    // Writer wejdzie w persist, sprawdzi fencing, uzyska advisory lock,
    // a nastepnie zatrzyma sie na UPDATE files do czasu odblokowania rekordu.
    let guard_conninfo = format!("{} application_name={guard_name}", conninfo());
    let guard = thread::spawn(move || -> Result<(), String> {
        let guard_repo = DbRepo::new(&guard_conninfo)?;
        // NO KEY UPDATE blokuje UPDATE files w persist, ale dopuszcza
        // KEY SHARE uzywany przez sprawdzenie FK przy zakladaniu lease.
        // FOR UPDATE blokowalo takze FK i writer nie docieral do READY.
        let result = guard_repo.exec(&format!(
            "BEGIN; SELECT 1 FROM files WHERE id_file = {file_id} FOR NO KEY UPDATE; \
             SELECT pg_sleep(15); COMMIT;"
        ));
        // pg_cancel_backend przerywa pg_sleep. Jawny ROLLBACK zwalnia
        // blokade rekordu jeszcze przed zakonczeniem watku.
        let rollback = guard_repo.exec("ROLLBACK");
        result?;
        rollback
    });

    let mut worker: Option<std::process::Child> = None;
    let mut data_object_ids = vec![initial_object];
    let result = (|| -> Result<(), String> {
        c5_wait_for_pg_activity(&repo_b, &guard_name, "Timeout", "PgSleep")?;

        let process = Command::new(env::current_exe().map_err(|err| err.to_string())?)
            .args([
                "--exact",
                "c5_persist_worker_entrypoint",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("FOD_C5_NAME", &name)
            .env("FOD_C5_FILE_ID", file_id.to_string())
            .env("FOD_C5_OWNER", owner_a.to_string())
            .env("FOD_C5_READY_FILE", &ready)
            .env("FOD_C5_WRITER_APP", &writer_name)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("C5 worker spawn: {err}"))?;
        worker = Some(process);

        let started = Instant::now();
        while !ready.exists() {
            if started.elapsed() > Duration::from_secs(5) {
                let diagnostics = repo_b
                    .query_scalar_text(&format!(
                        "SELECT COALESCE((SELECT                          'state=' || COALESCE(state, '') ||                          ' wait=' || COALESCE(wait_event_type, '') || '/' || COALESCE(wait_event, '') ||                          ' blockers=' || array_to_string(pg_blocking_pids(pid), ',')                          FROM pg_stat_activity                          WHERE application_name = '{writer_name}'                            AND datname = current_database() LIMIT 1), 'backend_not_found')"
                    ))
                    .unwrap_or_else(|error| format!("diagnostics_failed={error}"));
                return Err(format!("C5: writer readiness timeout: {diagnostics}"));
            }
            if let Some(status) = worker
                .as_mut()
                .expect("C5 worker")
                .try_wait()
                .map_err(|err| err.to_string())?
            {
                return Err(format!("C5: worker exited before ready: {status}"));
            }
            thread::sleep(Duration::from_millis(20));
        }
        let values = fs::read_to_string(&ready)
            .map_err(|err| err.to_string())?
            .split_whitespace()
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        if values.len() != 4 {
            return Err(format!("C5: invalid worker readiness: {values:?}"));
        }
        let (session_a, destination_token, file_token, owner_from_worker) =
            (values[0], values[1], values[2], values[3]);
        if owner_from_worker != owner_a {
            return Err("C5: worker owner mismatch".into());
        }

        // Warunek obserwowalny: rzeczywisty SQL writer A czeka na blokade files.
        c5_wait_for_pg_activity(&repo_b, &writer_name, "Lock", "*")?;

        // Dzierzawa wygasa dopiero po walidacji w transakcji A.
        repo_b.exec(&format!(
            "UPDATE destination_write_leases \
             SET lease_expires_at = clock_timestamp() - interval '1 second' \
             WHERE parent_key = 0 AND name = '{name}' AND fencing_token = {destination_token}; \
             UPDATE file_write_leases \
             SET lease_expires_at = clock_timestamp() - interval '1 second' \
             WHERE file_id = {file_id} AND fencing_token = {file_token};"
        ))?;

        let takeover_started = Instant::now();
        let premature = repo_b.acquire_write_ownership(
            None,
            &name,
            Some(file_id),
            owner_b,
            WRITE_TTL_SECONDS,
        )?;
        if let Some(unexpected) = premature {
            repo_b.release_write_ownership(owner_b, unexpected)?;
            return Err("C5: B took over while A persisted within a transaction".into());
        }
        if takeover_started.elapsed() >= FAIL_FAST_LIMIT {
            return Err("C5: takeover did not fail fast".into());
        }

        // Anulujemy tylko unikatowe zapytanie straznika testowego; jego
        // transakcja zostanie wycofana i zwolni blokade files.
        let cancel = repo_b.query_scalar_text(&format!(
            "SELECT COALESCE((SELECT pg_cancel_backend(pid) \
             FROM pg_stat_activity WHERE datname = current_database() \
             AND application_name = '{guard_name}' LIMIT 1), false)::text"
        ))?;
        if !matches!(cancel.trim(), "t" | "true") {
            return Err(format!("C5: could not cancel row guard: {cancel}"));
        }

        let output = worker
            .take()
            .expect("C5 worker exists")
            .wait_with_output()
            .map_err(|err| format!("C5 worker wait: {err}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || !stdout.contains("C5_WORKER_COMMIT=PASS") {
            return Err(format!(
                "C5: writer A failed: {}\n{}\n{}",
                output.status,
                stdout,
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let after_a = vec![b'A'; usize::try_from(block_size).map_err(|err| err.to_string())?];
        c4_assert_payload(&repo_b, file_id, block_size, &after_a)?;
        data_object_ids.push(scalar_u64(
            &repo_b,
            &format!("SELECT data_object_id FROM files WHERE id_file = {file_id}"),
            "C5 data object after A",
        ));

        // Dopiero po COMMIT A nowy klient moze przejac i zapisac blok B.
        let lease_b = repo_b
            .acquire_write_ownership(None, &name, Some(file_id), owner_b, WRITE_TTL_SECONDS)?
            .ok_or_else(|| "C5: B failed to acquire after A committed".to_string())?;
        let new_token = lease_b
            .file_fencing_token
            .ok_or_else(|| "C5: missing new file token".to_string())?;
        if new_token <= file_token {
            return Err("C5: new file fencing token did not advance".into());
        }
        let fence_b = WritePersistenceFence {
            session_id: session_b,
            owner_key: owner_b,
            file_fencing_token: new_token,
        };
        let after_b = vec![b'B'; usize::try_from(block_size).map_err(|err| err.to_string())?];
        c4_persist_payload(&repo_b, file_id, block_size, &after_b, fence_b)?;
        c4_assert_payload(&repo_b, file_id, block_size, &after_b)?;
        data_object_ids.push(scalar_u64(
            &repo_b,
            &format!("SELECT data_object_id FROM files WHERE id_file = {file_id}"),
            "C5 data object after B",
        ));

        // Odrzucenie starego tokenu z A nie moze nadpisac zatwierdzonych B.
        let old_fence = WritePersistenceFence {
            session_id: session_a,
            owner_key: owner_a,
            file_fencing_token: file_token,
        };
        let stale = c4_persist_payload(&repo_b, file_id, block_size, &after_a, old_fence);
        match stale {
            Err(error) if error.contains(WRITE_OWNERSHIP_FENCE_REJECTED_PREFIX) => {}
            other => return Err(format!("C5: stale writer was not fenced: {other:?}")),
        }
        c4_assert_payload(&repo_b, file_id, block_size, &after_b)?;
        repo_b.release_write_ownership(owner_b, lease_b)?;
        println!(
            "C5_INFLIGHT_PERSIST=PASS fencing={file_token}->{new_token} bytes={block_size}"
        );
        Ok(())
    })();

    // Cleanup jest wykonywany rowniez po timeout lub nieoczekiwanym wyniku.
    let _ = repo_b.query_scalar_text(&format!(
        "SELECT COALESCE((SELECT pg_cancel_backend(pid) \
         FROM pg_stat_activity WHERE datname = current_database() \
         AND application_name = '{guard_name}' LIMIT 1), false)::text"
    ));
    if let Some(mut child) = worker {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = guard.join();
    let _ = fs::remove_file(&ready);

    // Zbieramy wszystkie obiekty danych, ktore mogly powstac po swapach.
    // Jedynie obiekty tego pliku sa usuwane, inne testy pozostaja nietkniete.
    let current_object = repo_b
        .query_scalar_text(&format!(
            "SELECT data_object_id FROM files WHERE id_file = {file_id}"
        ))
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok());
    if let Some(id) = current_object {
        data_object_ids.push(id);
    }
    data_object_ids.sort_unstable();
    data_object_ids.dedup();
    let object_list = data_object_ids
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let cleanup_result = repo_b.exec(&format!(
        "DELETE FROM destination_write_leases WHERE parent_key = 0 AND name = '{name}'; \
         DELETE FROM file_write_leases WHERE file_id = {file_id}; \
         DELETE FROM client_sessions WHERE session_id IN \
           (SELECT session_id FROM client_session_owner_keys WHERE owner_key = {owner_a}) \
           OR session_id = {session_b}; \
         DELETE FROM files WHERE id_file = {file_id}; \
         DELETE FROM data_objects WHERE id_data_object IN ({object_list});"
    ));
    if let Err(error) = cleanup_result {
        panic!("C5 cleanup failed: {error}; test result: {result:?}");
    }
    if let Err(error) = result {
        panic!("{error}");
    }
}
