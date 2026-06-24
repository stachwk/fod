use fod_rust_hotpath::pg::DbRepo;
use fod_rust_runtime::FOD_SCHEMA_NAME;
use std::env;

const INDEXER_SCHEMA_REQUIRED_COLUMNS: &[(&str, &[&str])] = &[
    ("index_scan_runs", &["request_token", "updated_at"]),
    ("index_import_plans", &["request_token"]),
];

pub fn open_repo(conninfo: Option<&str>) -> Result<DbRepo, String> {
    let conninfo = resolve_conninfo(conninfo);
    DbRepo::new(&conninfo)
}

pub fn resolve_conninfo(conninfo: Option<&str>) -> String {
    if let Some(value) = conninfo.filter(|value| !value.trim().is_empty()) {
        return value.to_string();
    }
    if let Ok(value) = env::var("FOD_INDEXER_CONNINFO") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    if let Ok(value) = env::var("DATABASE_URL") {
        if !value.trim().is_empty() {
            return value;
        }
    }

    let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
    let dbname = env::var("POSTGRES_DB").unwrap_or_else(|_| "foddbname".to_string());
    let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "foduser".to_string());
    let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "cichosza".to_string());

    format!(
        "host='{}' port='{}' dbname='{}' user='{}' password='{}'",
        host, port, dbname, user, password
    )
}

pub fn sql_quote_literal(value: &str) -> String {
    DbRepo::quote_literal(value)
}

fn column_exists(repo: &DbRepo, table_name: &str, column_name: &str) -> Result<bool, String> {
    let sql = format!(
        "
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = {}
          AND table_name = {}
          AND column_name = {}
        LIMIT 1
        ",
        sql_quote_literal(FOD_SCHEMA_NAME),
        sql_quote_literal(table_name),
        sql_quote_literal(column_name),
    );
    Ok(!repo.query_rows_text(&sql)?.is_empty())
}

pub fn ensure_indexer_request_token_schema(repo: &DbRepo, operation: &str) -> Result<(), String> {
    let mut missing = Vec::new();
    for &(table_name, columns) in INDEXER_SCHEMA_REQUIRED_COLUMNS {
        for &column_name in columns {
            if !column_exists(repo, table_name, column_name)? {
                missing.push(format!("{table_name}.{column_name}"));
            }
        }
    }

    if missing.is_empty() {
        return Ok(());
    }

    Err(format!(
        "{operation} requires the FOD schema columns {}. Run `mkfs.fod upgrade` so migration 0014_indexer_request_tokens.sql is applied, then retry.",
        missing.join(", ")
    ))
}

pub fn sql_bytea_hex(bytes: &[u8]) -> String {
    format!("decode('{}', 'hex')", hex_encode(bytes))
}

pub fn sql_nullable_string(value: Option<&str>) -> String {
    value
        .map(sql_quote_literal)
        .unwrap_or_else(|| "NULL".to_string())
}

pub fn sql_nullable_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

pub fn sql_nullable_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}
