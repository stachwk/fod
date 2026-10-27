// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! Extended attributes share the same owner for a file and all of its hardlinks.
//! Keep path resolution here; connection routing and SQL execution stay in `pg`.

use super::{
    exec_command_params, exec_params, fetch_first_row_texts, is_retryable_connection_error,
    replayable_sql_error, result_error, sql_is_replayable_command, DbRepo, PQclear, PQgetvalue,
    PQnfields, PQntuples, PQresultStatus, PGRES_TUPLES_OK,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use std::ffi::{CStr, CString};

impl DbRepo {
    fn xattr_owner_for_path(&self, path: &str) -> Result<Option<(&'static str, u64)>, String> {
        let resolved = self.resolve_path(path)?;
        let (owner_kind, owner_id) = match resolved.kind.as_deref() {
            Some("hardlink") => {
                let file_id = self.get_file_id(path)?;
                match file_id {
                    Some(file_id) => ("file", file_id),
                    None => return Ok(None),
                }
            }
            Some("file") => match resolved.entry_id {
                Some(entry_id) => ("file", entry_id),
                None => return Ok(None),
            },
            Some("dir") => ("dir", resolved.entry_id.unwrap_or(0)),
            Some("symlink") => match resolved.entry_id {
                Some(entry_id) => ("symlink", entry_id),
                None => return Ok(None),
            },
            _ => return Ok(None),
        };

        Ok(Some((owner_kind, owner_id)))
    }

    pub fn fetch_xattr_value(&self, path: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
        let Some((owner_kind, owner_id)) = self.xattr_owner_for_path(path)? else {
            return Ok(None);
        };
        self.fetch_xattr_value_for_owner(owner_kind, owner_id, name)
    }

    pub fn fetch_xattr_value_for_owner(
        &self,
        owner_kind: &str,
        owner_id: u64,
        name: &str,
    ) -> Result<Option<Vec<u8>>, String> {
        let sql = CString::new(
            "SELECT encode(value, 'base64') FROM xattrs WHERE owner_kind = $1 AND owner_id = $2 AND name = $3",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let owner_kind =
            CString::new(owner_kind).map_err(|_| "owner kind contains NUL byte".to_string())?;
        let owner_id = CString::new(owner_id.to_string())
            .map_err(|_| "owner id contains NUL byte".to_string())?;
        let name = CString::new(name).map_err(|_| "xattr name contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&owner_kind, &owner_id, &name];
            let res = exec_params(conn, &sql, &params)?;
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            let encoded = if rows < 1 || cols < 1 {
                None
            } else {
                let value_ptr = PQgetvalue(res, 0, 0);
                if value_ptr.is_null() {
                    None
                } else {
                    Some(CStr::from_ptr(value_ptr).to_string_lossy().to_string())
                }
            };
            PQclear(res);
            let value = match encoded {
                None => None,
                Some(encoded) => {
                    let decoded = BASE64_STANDARD.decode(encoded.trim()).map_err(|err| {
                        format!("invalid xattr payload returned by PostgreSQL: {err}")
                    })?;
                    Some(decoded)
                }
            };
            Ok(value)
        })
    }

    pub fn list_xattr_names(&self, path: &str) -> Result<Option<Vec<String>>, String> {
        let Some((owner_kind, owner_id)) = self.xattr_owner_for_path(path)? else {
            return Ok(None);
        };
        self.list_xattr_names_for_owner(owner_kind, owner_id)
            .map(Some)
    }

    pub fn store_xattr_value_for_owner(
        &self,
        owner_kind: &str,
        owner_id: u64,
        name: &str,
        value: &[u8],
    ) -> Result<(), String> {
        let sql = CString::new(
            "
            INSERT INTO xattrs (owner_kind, owner_id, name, value)
            VALUES ($1, $2, $3, decode($4, 'base64'))
            ON CONFLICT (owner_kind, owner_id, name) DO UPDATE
            SET value = EXCLUDED.value
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let owner_kind =
            CString::new(owner_kind).map_err(|_| "owner kind contains NUL byte".to_string())?;
        let owner_id = CString::new(owner_id.to_string())
            .map_err(|_| "owner id contains NUL byte".to_string())?;
        let name = CString::new(name).map_err(|_| "xattr name contains NUL byte".to_string())?;
        let value_b64 = CString::new(BASE64_STANDARD.encode(value))
            .map_err(|_| "xattr value contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            let params = [&owner_kind, &owner_id, &name, &value_b64];
            exec_command_params(conn, &sql, &params)
        })
    }

    /// Store an access ACL and synchronize its basic permission bits atomically.
    /// The inode row lock serializes this update with chmod/chown.
    pub fn store_access_acl_and_mode_for_owner(
        &self,
        owner_kind: &str,
        owner_id: u64,
        name: &str,
        value: &[u8],
        permission_bits: u16,
        create_only: bool,
        replace_only: bool,
    ) -> Result<(), String> {
        let (table, key) = match owner_kind {
            "file" => ("files", "id_file"),
            "dir" => ("directories", "id_directory"),
            _ => return Err("access ACL mode sync requires a file or directory".to_string()),
        };
        let owner_kind = CString::new(owner_kind)
            .map_err(|_| "xattr owner kind contains NUL byte".to_string())?;
        let owner_id_text = owner_id.to_string();
        let owner_id_param = CString::new(owner_id_text.clone())
            .map_err(|_| "xattr owner id contains NUL byte".to_string())?;
        let name = CString::new(name).map_err(|_| "xattr name contains NUL byte".to_string())?;
        let value_b64 = CString::new(BASE64_STANDARD.encode(value))
            .map_err(|_| "xattr value contains NUL byte".to_string())?;
        let select_mode = CString::new(format!(
            "SELECT mode FROM {table} WHERE {key} = $1 FOR UPDATE"
        ))
        .unwrap();
        let store_acl = CString::new(
            "INSERT INTO xattrs (owner_kind, owner_id, name, value)
             VALUES ($1, $2, $3, decode($4, 'base64'))
             ON CONFLICT (owner_kind, owner_id, name) DO UPDATE
             SET value = EXCLUDED.value",
        )
        .unwrap();
        let check_acl = CString::new(
            "SELECT EXISTS (
                SELECT 1 FROM xattrs WHERE owner_kind = $1 AND owner_id = $2 AND name = $3
             )::text",
        )
        .unwrap();

        self.with_transaction(|tx| unsafe {
            let mode_row = fetch_first_row_texts(exec_params(
                tx.conn,
                &select_mode,
                &[&owner_id_param],
            )?)?;
            let Some(mode_text) = mode_row.first() else {
                return Err("FOD_XATTR_OWNER_MISSING".to_string());
            };
            let current_mode = u32::from_str_radix(mode_text, 8)
                .map_err(|err| format!("invalid inode mode: {err}"))?;
            let acl_exists = fetch_first_row_texts(exec_params(
                tx.conn,
                &check_acl,
                &[&owner_kind, &owner_id_param, &name],
            )?)?
            .first()
            .is_some_and(|value| matches!(value.as_str(), "t" | "true"));
            if create_only && acl_exists {
                return Err("FOD_XATTR_EXISTS".to_string());
            }
            if replace_only && !acl_exists {
                return Err("FOD_XATTR_MISSING".to_string());
            }
            let synchronized_mode = (current_mode & !0o777) | (u32::from(permission_bits) & 0o777);
            let mode_param = CString::new(format!("{synchronized_mode:o}"))
                .map_err(|_| "mode contains NUL byte".to_string())?;
            exec_command_params(
                tx.conn,
                &store_acl,
                &[&owner_kind, &owner_id_param, &name, &value_b64],
            )?;
            let update_mode = CString::new(format!(
                "UPDATE {table} SET mode = $1, change_date = NOW(), modification_date = NOW() WHERE {key} = $2"
            ))
            .unwrap();
            exec_command_params(tx.conn, &update_mode, &[&mode_param, &owner_id_param])
        })
    }

    pub fn delete_owner_xattrs(&self, owner_kind: &str, owner_id: u64) -> Result<(), String> {
        let sql = CString::new("DELETE FROM xattrs WHERE owner_kind = $1 AND owner_id = $2")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let owner_kind =
            CString::new(owner_kind).map_err(|_| "owner kind contains NUL byte".to_string())?;
        let owner_id = CString::new(owner_id.to_string())
            .map_err(|_| "owner id contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            let params = [&owner_kind, &owner_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn remove_xattr_for_owner(
        &self,
        owner_kind: &str,
        owner_id: u64,
        name: &str,
    ) -> Result<u64, String> {
        let sql = CString::new(
            "DELETE FROM xattrs WHERE owner_kind = $1 AND owner_id = $2 AND name = $3 RETURNING 1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let owner_kind =
            CString::new(owner_kind).map_err(|_| "owner kind contains NUL byte".to_string())?;
        let owner_id = CString::new(owner_id.to_string())
            .map_err(|_| "owner id contains NUL byte".to_string())?;
        let name = CString::new(name).map_err(|_| "xattr name contains NUL byte".to_string())?;

        self.with_cached_connection(|conn| unsafe {
            let params = [&owner_kind, &owner_id, &name];
            let res = exec_params(conn, &sql, &params)?;
            if PQresultStatus(res) != PGRES_TUPLES_OK {
                let error = result_error(res);
                PQclear(res);
                if sql_is_replayable_command(&sql) && is_retryable_connection_error(conn, &error) {
                    return Err(replayable_sql_error(error));
                }
                return Err(error);
            }
            let rows = PQntuples(res);
            PQclear(res);
            Ok(rows as u64)
        })
    }

    pub fn list_xattr_names_for_owner(
        &self,
        owner_kind: &str,
        owner_id: u64,
    ) -> Result<Vec<String>, String> {
        let sql = CString::new(
            "SELECT name FROM xattrs WHERE owner_kind = $1 AND owner_id = $2 ORDER BY name",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let owner_kind =
            CString::new(owner_kind).map_err(|_| "owner kind contains NUL byte".to_string())?;
        let owner_id = CString::new(owner_id.to_string())
            .map_err(|_| "owner id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&owner_kind, &owner_id];
            let res = exec_params(conn, &sql, &params)?;
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            let mut names = Vec::with_capacity(rows.max(0) as usize);
            if rows >= 1 && cols >= 1 {
                for row in 0..rows {
                    let value_ptr = PQgetvalue(res, row, 0);
                    if !value_ptr.is_null() {
                        names.push(CStr::from_ptr(value_ptr).to_string_lossy().to_string());
                    }
                }
            }
            PQclear(res);
            Ok(names)
        })
    }
}
