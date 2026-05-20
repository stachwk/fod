// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

#![allow(dead_code)]

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;

use crate::pg::DbConn;
use crate::runtime::FOD_SCHEMA_NAME;

pub fn format_schema_admin_source_message(source: &str) -> String {
    format!(
        "Schema admin password source: {} (no prompt needed)",
        source
    )
}

pub fn schema_admin_secret_required_message(action_name: &str) -> String {
    format!(
        "Schema admin password is required for {}; pass --schema-admin-password.",
        action_name
    )
}

pub fn derive_schema_admin_secret(
    password: &str,
    salt: Option<&[u8]>,
    iterations: u32,
) -> (String, String, u32) {
    let mut salt_bytes = [0u8; 16];
    if let Some(source) = salt {
        let copy_len = source.len().min(salt_bytes.len());
        salt_bytes[..copy_len].copy_from_slice(&source[..copy_len]);
    } else {
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut salt_bytes);
    }
    let mut output = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt_bytes, iterations, &mut output);
    (
        BASE64_STANDARD.encode(salt_bytes),
        BASE64_STANDARD.encode(output),
        iterations,
    )
}

pub fn verify_schema_admin_secret(
    password: &str,
    salt_b64: &str,
    hash_b64: &str,
    iterations: u32,
) -> bool {
    let salt = BASE64_STANDARD
        .decode(salt_b64.as_bytes())
        .unwrap_or_default();
    let (_, derived_hash, _) = derive_schema_admin_secret(password, Some(&salt), iterations);
    derived_hash == hash_b64
}

pub fn schema_admin_secret_exists(conn: &DbConn) -> Result<bool, String> {
    if !conn.query_exists(&format!(
        "SELECT to_regclass({}) IS NOT NULL",
        quote_schema_regclass(FOD_SCHEMA_NAME, "schema_admin")
    ))? {
        return Ok(false);
    }
    conn.query_exists(&format!(
        "SELECT EXISTS (SELECT 1 FROM {} WHERE id = 1)",
        quote_schema_qualified_ident(FOD_SCHEMA_NAME, "schema_admin")
    ))
}

pub fn parse_schema_admin_secret_row(row: &str) -> Result<(String, String, u32), String> {
    let mut parts = row.splitn(3, '\n');
    let hash = parts.next().unwrap_or_default();
    let salt = parts.next().unwrap_or_default();
    let iterations = parts.next().unwrap_or_default();
    if hash.is_empty() || salt.is_empty() || iterations.is_empty() {
        return Err("schema-admin secret row is malformed".to_string());
    }
    let iterations = iterations
        .parse::<u32>()
        .map_err(|_| "schema-admin secret row has invalid iterations".to_string())?;
    Ok((hash.to_string(), salt.to_string(), iterations))
}

pub fn ensure_schema_admin_secret(conn: &DbConn, password: Option<&str>) -> Result<bool, String> {
    conn.exec(&format!("CREATE SCHEMA IF NOT EXISTS {}", FOD_SCHEMA_NAME))?;
    conn.exec(
        "CREATE TABLE IF NOT EXISTS schema_admin (\
            id INTEGER PRIMARY KEY CHECK (id = 1),\
            password_hash TEXT NOT NULL,\
            password_salt TEXT NOT NULL,\
            password_iterations INTEGER NOT NULL,\
            created_at TIMESTAMP NOT NULL DEFAULT NOW(),\
            updated_at TIMESTAMP NOT NULL DEFAULT NOW()\
        )",
    )?;

    if let Some(row) = conn.query_scalar_text(
        "SELECT password_hash || E'\\n' || password_salt || E'\\n' || password_iterations::text FROM schema_admin WHERE id = 1",
    )? {
        let (hash, salt, iterations) = parse_schema_admin_secret_row(&row)?;
        let Some(password) = password else {
            return Err("Schema admin password is required for this existing database; pass --schema-admin-password.".to_string());
        };
        if !verify_schema_admin_secret(password, &salt, &hash, iterations) {
            return Err("Schema admin password does not match the schema-admin secret currently stored in the FOD database. This usually means you are using a secret from a different bootstrap; rerun init to generate a new secret or provide the current one.".to_string());
        }
        return Ok(false);
    }

    let Some(password) = password else {
        return Err("Schema admin password is required for the first FOD bootstrap; pass --schema-admin-password.".to_string());
    };
    let (salt_b64, hash_b64, iterations) = derive_schema_admin_secret(password, None, 200_000);
    let salt_sql = DbConn::quote_literal(&salt_b64);
    let hash_sql = DbConn::quote_literal(&hash_b64);
    conn.exec(&format!(
        "INSERT INTO schema_admin (id, password_hash, password_salt, password_iterations, created_at, updated_at) \
         VALUES (1, {}, {}, {}, NOW(), NOW()) \
         ON CONFLICT (id) DO UPDATE SET \
            password_hash = EXCLUDED.password_hash, \
            password_salt = EXCLUDED.password_salt, \
            password_iterations = EXCLUDED.password_iterations, \
            updated_at = NOW()",
        hash_sql, salt_sql, iterations
    ))?;
    Ok(true)
}

pub fn verify_existing_schema_admin_secret(
    conn: &DbConn,
    password: Option<&str>,
) -> Result<(), String> {
    if !schema_admin_secret_exists(conn)? {
        return Err(
            "Schema admin secret is missing from the FOD schema; run fod-mkfs init or upgrade first."
                .to_string(),
        );
    }
    let Some(password) = password else {
        return Err(
            "Schema admin password is required for this existing database; pass --schema-admin-password."
                .to_string(),
        );
    };
    let row = conn
        .query_scalar_text(
            "SELECT password_hash || E'\\n' || password_salt || E'\\n' || password_iterations::text FROM schema_admin WHERE id = 1",
        )?
        .ok_or_else(|| {
            "Schema admin secret is missing from the FOD schema; run fod-mkfs init or upgrade first."
                .to_string()
        })?;
    let (hash, salt, iterations) = parse_schema_admin_secret_row(&row)?;
    if !verify_schema_admin_secret(password, &salt, &hash, iterations) {
        return Err("Schema admin password does not match the schema-admin secret currently stored in the FOD database. This usually means you are using a secret from a different bootstrap; rerun init to generate a new secret or provide the current one.".to_string());
    }
    Ok(())
}

fn quote_schema_qualified_ident(schema: &str, object: &str) -> String {
    format!(
        "{}.{}",
        DbConn::quote_identifier(schema),
        DbConn::quote_identifier(object)
    )
}

fn quote_schema_regclass(schema: &str, object: &str) -> String {
    DbConn::quote_literal(&quote_schema_qualified_ident(schema, object))
}
