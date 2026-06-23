use fod_rust_hotpath::pg::DbRepo;
use std::env;

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
