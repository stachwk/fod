// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! Startup metadata, configuration and PostgreSQL diagnostics. A startup snapshot
//! keeps all probes on one checked-out authority connection, including before mkfs.

use super::{
    exec_params, fetch_single_optional_text, postgres_runtime_requirements_on_conn,
    postgres_version_diagnostics_on_conn, query_rows_text_on_conn, query_scalar_text_on_conn,
    schema_is_initialized_on_conn, DbRepo, PGconn,
};
use fod_rust_runtime::{PostgresRuntimeRequirements, PostgresVersionDiagnostics, FOD_SCHEMA_NAME};
use std::ffi::CString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupSnapshot {
    pub block_size: Option<u32>,
    pub is_in_recovery: bool,
    pub schema_version: Option<u32>,
    pub schema_is_initialized: bool,
}

impl DbRepo {
    pub fn postgres_runtime_requirements(&self) -> Result<PostgresRuntimeRequirements, String> {
        self.postgres_runtime_requirements_for_pool_limit(self.pool.limit as u64)
    }

    pub fn postgres_runtime_requirements_for_pool_limit(
        &self,
        pool_max_connections: u64,
    ) -> Result<PostgresRuntimeRequirements, String> {
        self.with_control_connection(|conn| unsafe {
            postgres_runtime_requirements_on_conn(conn, pool_max_connections)
        })
    }

    pub fn postgres_version_diagnostics(&self) -> Result<PostgresVersionDiagnostics, String> {
        self.with_control_connection(|conn| unsafe { postgres_version_diagnostics_on_conn(conn) })
    }

    unsafe fn query_config_value_on_conn(
        conn: *mut PGconn,
        key: &str,
    ) -> Result<Option<String>, String> {
        let sql = CString::new("SELECT value FROM config WHERE key = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let key = CString::new(key).map_err(|_| "config key contains NUL byte".to_string())?;
        let params = [&key];
        let res = exec_params(conn, &sql, &params)?;
        fetch_single_optional_text(res)
    }

    pub fn query_config_value(&self, key: &str) -> Result<Option<String>, String> {
        self.with_cached_connection(|conn| unsafe { Self::query_config_value_on_conn(conn, key) })
    }

    unsafe fn is_in_recovery_on_conn(conn: *mut PGconn) -> Result<bool, String> {
        let sql = CString::new("SELECT pg_is_in_recovery()")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let value = query_scalar_text_on_conn(conn, &sql)?;
        Ok(matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        ))
    }

    pub fn is_in_recovery(&self) -> Result<bool, String> {
        self.with_cached_connection(|conn| unsafe { Self::is_in_recovery_on_conn(conn) })
    }

    unsafe fn schema_version_on_conn(conn: *mut PGconn) -> Result<Option<u32>, String> {
        let sql =
            CString::new("SELECT version FROM schema_version ORDER BY applied_at DESC LIMIT 1")
                .map_err(|_| "SQL contains NUL byte".to_string())?;
        let value = query_scalar_text_on_conn(conn, &sql)?;
        if value.trim().is_empty() {
            return Ok(None);
        }
        value
            .trim()
            .parse::<u32>()
            .map(Some)
            .map_err(|err| format!("invalid schema version returned by PostgreSQL: {err}"))
    }

    pub fn schema_version(&self) -> Result<Option<u32>, String> {
        self.with_cached_connection(|conn| unsafe { Self::schema_version_on_conn(conn) })
    }

    pub fn schema_is_initialized(&self) -> Result<bool, String> {
        self.with_cached_connection(|conn| unsafe { schema_is_initialized_on_conn(conn) })
    }

    unsafe fn startup_probe_on_conn(conn: *mut PGconn) -> Result<(bool, bool), String> {
        // Pierwszy round-trip jest bezpieczny takze przed inicjalizacja FOD:
        // odwoluje sie tylko do pg_catalog/to_regclass i pg_is_in_recovery().
        let sql = CString::new(format!(
            "SELECT \
                (\
                    to_regclass('{schema}.directories') IS NOT NULL AND \
                    to_regclass('{schema}.files') IS NOT NULL AND \
                    to_regclass('{schema}.schema_version') IS NOT NULL\
                )::text, \
                pg_is_in_recovery()::text",
            schema = FOD_SCHEMA_NAME
        ))
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let rows = query_rows_text_on_conn(conn, &sql)?;
        let row = rows
            .first()
            .ok_or_else(|| "startup probe returned no rows".to_string())?;
        if row.len() < 2 {
            return Err(format!(
                "startup probe returned {} columns, expected at least 2",
                row.len()
            ));
        }

        let schema_is_initialized = matches!(
            row[0].trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        );
        let is_in_recovery = matches!(
            row[1].trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        );
        Ok((schema_is_initialized, is_in_recovery))
    }

    unsafe fn startup_initialized_values_on_conn(conn: *mut PGconn) -> Result<(u32, u32), String> {
        // Drugie zapytanie jest wykonywane dopiero po potwierdzeniu schematu.
        // EXISTS rozroznia brak rekordu config od obecnego, ale pustego rekordu,
        // aby zachowac komunikaty i walidacje z poprzedniej implementacji.
        let sql = CString::new(
            "SELECT \
                (EXISTS (SELECT 1 FROM config WHERE key = 'block_size'))::text, \
                COALESCE((SELECT value::text FROM config WHERE key = 'block_size' LIMIT 1), ''), \
                (EXISTS (SELECT 1 FROM config WHERE key = 'max_fs_size_bytes'))::text, \
                COALESCE((SELECT value::text FROM config WHERE key = 'max_fs_size_bytes' LIMIT 1), ''), \
                COALESCE((\
                    SELECT version::text \
                    FROM schema_version \
                    ORDER BY applied_at DESC \
                    LIMIT 1\
                ), '')",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let rows = query_rows_text_on_conn(conn, &sql)?;
        let row = rows
            .first()
            .ok_or_else(|| "initialized startup values query returned no rows".to_string())?;
        if row.len() < 5 {
            return Err(format!(
                "initialized startup values query returned {} columns, expected at least 5",
                row.len()
            ));
        }

        let block_size_present = matches!(
            row[0].trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        );
        if !block_size_present {
            return Err("initialized FOD schema is missing config.block_size".to_string());
        }
        let block_size = row[1]
            .trim()
            .parse::<u32>()
            .map_err(|err| format!("invalid config.block_size value: {err}"))?;
        if block_size == 0 {
            return Err("config.block_size must be greater than zero".to_string());
        }

        let max_fs_size_bytes_present = matches!(
            row[2].trim().to_ascii_lowercase().as_str(),
            "t" | "true" | "1" | "on"
        );
        if !max_fs_size_bytes_present {
            return Err("initialized FOD schema is missing config.max_fs_size_bytes".to_string());
        }
        row[3]
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid config.max_fs_size_bytes value: {err}"))?;

        let schema_version_value = row[4].trim();
        if schema_version_value.is_empty() {
            return Err("initialized FOD schema is missing schema version".to_string());
        }
        let schema_version = schema_version_value
            .parse::<u32>()
            .map_err(|err| format!("invalid schema version returned by PostgreSQL: {err}"))?;

        Ok((block_size, schema_version))
    }

    unsafe fn startup_snapshot_on_conn(
        &self,
        conn: *mut PGconn,
    ) -> Result<StartupSnapshot, String> {
        // Caly logiczny snapshot nadal pozostaje na jednym PGconn.
        // Niezainicjalizowany schemat konczy sie po jednym zapytaniu.
        // Zainicjalizowany schemat wymaga maksymalnie dwoch zapytan.
        let (schema_is_initialized, is_in_recovery) = Self::startup_probe_on_conn(conn)?;

        if !schema_is_initialized {
            return Ok(StartupSnapshot {
                block_size: None,
                is_in_recovery,
                schema_version: None,
                schema_is_initialized: false,
            });
        }

        let (block_size, schema_version) = Self::startup_initialized_values_on_conn(conn)?;

        Ok(StartupSnapshot {
            block_size: Some(block_size),
            is_in_recovery,
            schema_version: Some(schema_version),
            schema_is_initialized: true,
        })
    }

    pub fn startup_snapshot(&self) -> Result<StartupSnapshot, String> {
        self.with_cached_connection(|conn| unsafe { self.startup_snapshot_on_conn(conn) })
    }
}
