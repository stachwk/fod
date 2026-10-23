// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

#[path = "../src/pg.rs"]
mod pg;

use pg::DbConn;
use std::env;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MIGRATION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../migrations/0026_file_content_generation.sql"
));
const MIGRATION_27: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../migrations/0027_explicit_file_content_generation.sql"
));

fn conninfo() -> String {
    fn value(primary: &str, fallback: &str, default: &str) -> String {
        env::var(primary)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| env::var(fallback).ok().filter(|v| !v.trim().is_empty()))
            .unwrap_or_else(|| default.to_string())
    }
    let db = value("FOD_PG_DBNAME", "POSTGRES_DB", "foddbname");
    assert!(
        db != "foddbname" && db.to_ascii_lowercase().contains("test"),
        "TX03J generation tests require a dedicated test database, got {db}"
    );
    format!(
        "host={} port={} dbname={} user={} password={}",
        DbConn::quote_literal(&value("FOD_PG_HOST", "POSTGRES_HOST", "127.0.0.1")),
        DbConn::quote_literal(&value("FOD_PG_PORT", "POSTGRES_PORT", "5432")),
        DbConn::quote_literal(&db),
        DbConn::quote_literal(&value("FOD_PG_USER", "POSTGRES_USER", "foduser")),
        DbConn::quote_literal(&value("FOD_PG_PASSWORD", "POSTGRES_PASSWORD", "cichosza")),
    )
}

struct Fixture {
    db: DbConn,
    schema: String,
}

impl Fixture {
    fn new() -> Self {
        let conninfo = conninfo();
        let db = DbConn::connect(&conninfo).expect("connect test PostgreSQL");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let schema = format!("tx03j_gen_{}_{}", std::process::id(), nanos);
        // Deliberately isolated from the real fod schema.
        db.exec(&format!(
            "CREATE SCHEMA {schema};
             CREATE TABLE {schema}.files (
                 id_file BIGINT PRIMARY KEY,
                 data_object_id BIGINT NOT NULL,
                 size BIGINT NOT NULL,
                 content_generation BIGINT NOT NULL DEFAULT 1,
                 modification_date TIMESTAMP NOT NULL DEFAULT NOW(),
                 change_date TIMESTAMP NOT NULL DEFAULT NOW()
             );
             INSERT INTO {schema}.files(id_file, data_object_id, size, content_generation)
             VALUES (1, 101, 4096, 1), (2, 201, 4096, 1),
                    (3, 301, 4096, 9223372036854775807);
             SET search_path TO {schema}, public;"
        ))
        .expect("create isolated schema fixture");
        db.exec(MIGRATION)
            .expect("apply migration 26 against fixture");
        db.exec(MIGRATION_27)
            .expect("apply migration 27 against fixture");
        Self { db, schema }
    }

    fn generation(&self, file_id: u64) -> u64 {
        self.db
            .query_scalar_u64(&format!(
                "SELECT content_generation FROM {}.files WHERE id_file = {file_id}",
                self.schema
            ))
            .expect("query generation")
            .expect("file exists")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.db
            .exec(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.schema))
            .expect("drop isolated schema fixture");
    }
}

#[test]
fn content_generation_atomic_rollback_and_file_isolation() {
    let fixture = Fixture::new();
    assert_eq!(fixture.generation(1), 1);
    assert_eq!(fixture.generation(2), 1);

    // Metadata-only changes do not invalidate content.
    fixture
        .db
        .exec("UPDATE files SET change_date = change_date + interval '1 second' WHERE id_file = 1")
        .unwrap();
    assert_eq!(fixture.generation(1), 1);

    fixture.db.exec("BEGIN").unwrap();
    fixture
        .db
        .exec("UPDATE files SET data_object_id = 102 WHERE id_file = 1")
        .unwrap();
    fixture
        .db
        .exec("UPDATE files SET content_generation = content_generation + 1 WHERE id_file = 1")
        .unwrap();
    assert_eq!(fixture.generation(1), 2);
    fixture.db.exec("ROLLBACK").unwrap();
    assert_eq!(fixture.generation(1), 1);
    assert_eq!(
        fixture
            .db
            .query_scalar_u64("SELECT data_object_id FROM files WHERE id_file = 1")
            .unwrap(),
        Some(101)
    );

    fixture.db.exec("BEGIN").unwrap();
    fixture
        .db
        .exec("UPDATE files SET data_object_id = 102, size = 8192, content_generation = content_generation + 1 WHERE id_file = 1")
        .unwrap();
    assert_eq!(fixture.generation(1), 2);
    fixture.db.exec("COMMIT").unwrap();
    assert_eq!(fixture.generation(1), 2);
    assert_eq!(fixture.generation(2), 1);

    // No-op update must not invent a content change.
    fixture
        .db
        .exec("UPDATE files SET size = size WHERE id_file = 1")
        .unwrap();
    assert_eq!(fixture.generation(1), 2);

    // A generic client cannot lower, skip, or overflow the generation.
    assert!(fixture
        .db
        .exec("UPDATE files SET content_generation = 0 WHERE id_file = 1")
        .is_err());
    assert!(fixture
        .db
        .exec("UPDATE files SET content_generation = content_generation + 2 WHERE id_file = 1")
        .is_err());
    assert_eq!(fixture.generation(1), 2);

    assert!(fixture
        .db
        .exec("UPDATE files SET content_generation = content_generation + 1 WHERE id_file = 3")
        .is_err());
}

#[test]
fn content_generation_concurrent_commits_are_monotonic() {
    let fixture = Fixture::new();
    let conninfo = conninfo();
    let schema = fixture.schema.clone();
    let (started_tx, started_rx) = mpsc::channel();

    fixture.db.exec("BEGIN").unwrap();
    fixture
        .db
        .exec("UPDATE files SET data_object_id = 102, content_generation = content_generation + 1 WHERE id_file = 1")
        .unwrap();

    let worker = thread::spawn(move || {
        let db = DbConn::connect(&conninfo).expect("second PostgreSQL connection");
        db.exec(&format!(
            "SET search_path TO {schema}, public; SET lock_timeout = '5s'"
        ))
        .unwrap();
        started_tx.send(()).unwrap();
        db.exec("UPDATE files SET data_object_id = 103, content_generation = content_generation + 1 WHERE id_file = 1")
            .expect("second writer waits for first commit");
    });
    started_rx.recv().unwrap();
    thread::sleep(Duration::from_millis(75));
    fixture.db.exec("COMMIT").unwrap();
    worker.join().expect("second writer thread");

    assert_eq!(fixture.generation(1), 3);
    assert_eq!(fixture.generation(2), 1);
    assert_eq!(
        fixture
            .db
            .query_scalar_u64("SELECT data_object_id FROM files WHERE id_file = 1")
            .unwrap(),
        Some(103)
    );
}
