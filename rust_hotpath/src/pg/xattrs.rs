// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! Extended attributes share the same owner for a file and all of its hardlinks.
//! Keep path resolution here; connection routing and SQL execution stay in `pg`.

use super::{
    exec_command_params, exec_params, fetch_first_row_texts, is_retryable_connection_error,
    parse_persisted_posix_mode, replayable_sql_error, result_error, sql_is_replayable_command,
    DbRepo, PQclear, PQgetisnull, PQgetvalue, PQnfields, PQntuples, PQresultStatus,
    PGRES_TUPLES_OK,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use std::ffi::CString;

pub(super) struct InheritedPosixAcl {
    pub(super) access_acl: Vec<u8>,
    pub(super) default_acl: Option<Vec<u8>>,
}

/// Apply chmod's owner, group-class and other bits to an existing access ACL.
/// With named entries, the group-class bits represent `ACL_MASK`; otherwise
/// they represent `ACL_GROUP_OBJ`.
pub(super) fn chmod_access_acl(value: &[u8], mode: u32) -> Result<Vec<u8>, String> {
    if value.len() < 4 || (value.len() - 4) % 8 != 0 {
        return Err("invalid POSIX ACL xattr length".to_string());
    }
    if u32::from_le_bytes(value[..4].try_into().unwrap()) != 2 {
        return Err("unsupported POSIX ACL xattr version".to_string());
    }

    let owner = ((mode >> 6) & 0o7) as u16;
    let group = ((mode >> 3) & 0o7) as u16;
    let other = (mode & 0o7) as u16;
    let mut acl = value.to_vec();
    let mut user_obj = false;
    let mut group_obj = None;
    let mut has_mask = false;
    let mut other_entry = false;
    let mut has_named_entry = false;

    for (index, entry) in acl[4..].chunks_exact_mut(8).enumerate() {
        let tag = u16::from_le_bytes(entry[0..2].try_into().unwrap());
        let permissions = u16::from_le_bytes(entry[2..4].try_into().unwrap());
        if permissions & !0o7 != 0 {
            return Err("invalid POSIX ACL permission bits".to_string());
        }
        match tag {
            0x0001 if !user_obj => {
                user_obj = true;
                entry[2..4].copy_from_slice(&owner.to_le_bytes());
            }
            0x0002 | 0x0008 => has_named_entry = true,
            0x0004 if group_obj.is_none() => group_obj = Some(index),
            0x0010 if !has_mask => {
                has_mask = true;
                entry[2..4].copy_from_slice(&group.to_le_bytes());
            }
            0x0020 if !other_entry => {
                other_entry = true;
                entry[2..4].copy_from_slice(&other.to_le_bytes());
            }
            _ => return Err("invalid or duplicate POSIX ACL entry".to_string()),
        }
    }

    if !user_obj || group_obj.is_none() || !other_entry || (has_named_entry && !has_mask) {
        return Err("incomplete POSIX ACL xattr".to_string());
    }
    if !has_mask {
        let group_obj = group_obj.unwrap();
        let offset = 4 + group_obj * 8 + 2;
        acl[offset..offset + 2].copy_from_slice(&group.to_le_bytes());
    }
    Ok(acl)
}

fn xattr_owner_table(owner_kind: &str) -> Result<(&'static str, &'static str), String> {
    match owner_kind {
        "file" => Ok(("files", "id_file")),
        "dir" => Ok(("directories", "id_directory")),
        "symlink" => Ok(("symlinks", "id_symlink")),
        _ => Err("unsupported xattr owner kind".to_string()),
    }
}

unsafe fn decode_xattr_name_rows(res: *mut super::PGresult) -> Result<Vec<String>, String> {
    let result = match PQresultStatus(res) {
        PGRES_TUPLES_OK => {
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            if rows == 0 {
                Ok(Vec::new())
            } else if rows < 0 || cols < 1 {
                Err("invalid xattr name rows returned by PostgreSQL".to_string())
            } else {
                let mut names = Vec::with_capacity(rows as usize);
                let mut decode_error = None;
                for row in 0..rows {
                    if PQgetisnull(res, row, 0) != 0 {
                        decode_error = Some("xattr name row contains NULL".to_string());
                        break;
                    }
                    let value = PQgetvalue(res, row, 0);
                    if value.is_null() {
                        decode_error = Some("xattr name row contains a missing value".to_string());
                        break;
                    }
                    let name = std::ffi::CStr::from_ptr(value)
                        .to_string_lossy()
                        .to_string();
                    if name.is_empty() {
                        decode_error = Some("xattr name row contains an empty name".to_string());
                        break;
                    }
                    names.push(name);
                }
                match decode_error {
                    Some(error) => Err(error),
                    None => Ok(names),
                }
            }
        }
        _ => Err(result_error(res)),
    };
    PQclear(res);
    result
}

unsafe fn authorize_xattr_mutation_on_conn(
    conn: *mut super::PGconn,
    owner_kind: &str,
    owner_id: u64,
    name: &str,
    actor_uid: u32,
    actor_gid: u32,
    actor_groups: &[u32],
    use_posix_acl: bool,
) -> Result<(), String> {
    let (table, key) = xattr_owner_table(owner_kind)?;
    let sql = CString::new(format!("SELECT uid, gid, mode FROM {table} WHERE {key}=$1")).unwrap();
    let row = fetch_first_row_texts(exec_params(
        conn,
        &sql,
        &[&CString::new(owner_id.to_string()).unwrap()],
    )?)?;
    if row.len() != 3 {
        return Err("FOD_XATTR_OWNER_MISSING".to_string());
    }
    let owner_uid = row[0].parse::<u32>().map_err(|err| err.to_string())?;
    let owner_gid = row[1].parse::<u32>().map_err(|err| err.to_string())?;
    let mode = parse_persisted_posix_mode(&row[2])?;

    if name.starts_with("trusted.") && actor_uid != 0 {
        return Err("FOD_XATTR_PERMISSION_DENIED".to_string());
    }
    if name.starts_with("user.") {
        if !matches!(owner_kind, "file" | "dir") {
            return Err("FOD_XATTR_NOT_SUPPORTED".to_string());
        }
        if owner_kind == "dir" && mode & 0o1000 != 0 && actor_uid != 0 && actor_uid != owner_uid {
            return Err("FOD_XATTR_PERMISSION_DENIED".to_string());
        }
        if actor_uid != 0 {
            let acl_allowed = if use_posix_acl {
                super::metadata_updates::inode_acl_allows_on_conn(
                    conn,
                    owner_kind,
                    owner_id,
                    actor_uid,
                    actor_groups,
                    owner_uid,
                    owner_gid,
                    0o2,
                )?
            } else {
                None
            };
            let allowed = acl_allowed.unwrap_or_else(|| {
                let bits = if actor_uid == owner_uid {
                    (mode >> 6) & 0o7
                } else if actor_gid == owner_gid || actor_groups.contains(&owner_gid) {
                    (mode >> 3) & 0o7
                } else {
                    mode & 0o7
                };
                bits & 0o2 != 0
            });
            if !allowed {
                return Err("FOD_XATTR_ACCESS_DENIED".to_string());
            }
        }
        return Ok(());
    }
    if actor_uid != 0 && actor_uid != owner_uid {
        return Err("FOD_XATTR_PERMISSION_DENIED".to_string());
    }
    Ok(())
}

fn constrain_default_acl_to_mode(value: &[u8], mode: u32) -> Result<(Vec<u8>, u32), String> {
    if value.len() < 4 || (value.len() - 4) % 8 != 0 {
        return Err("invalid POSIX default ACL xattr length".to_string());
    }
    if u32::from_le_bytes(value[..4].try_into().unwrap()) != 2 {
        return Err("unsupported POSIX ACL xattr version".to_string());
    }

    let owner_mask = ((mode >> 6) & 0o7) as u16;
    let group_mask = ((mode >> 3) & 0o7) as u16;
    let other_mask = (mode & 0o7) as u16;
    let mut access_acl = value.to_vec();
    let mut has_mask = false;
    let mut has_named_entry = false;
    let mut user_obj = None;
    let mut group_obj = None;
    let mut acl_mask = None;
    let mut other = None;

    for entry in access_acl[4..].chunks_exact_mut(8) {
        let tag = u16::from_le_bytes(entry[0..2].try_into().unwrap());
        let mut perm = u16::from_le_bytes(entry[2..4].try_into().unwrap());
        match tag {
            0x0001 => {
                perm &= owner_mask;
                user_obj = Some(perm);
            }
            0x0002 | 0x0008 => has_named_entry = true,
            0x0004 => group_obj = Some(perm),
            0x0010 => {
                has_mask = true;
                perm &= group_mask;
                acl_mask = Some(perm);
            }
            0x0020 => {
                perm &= other_mask;
                other = Some(perm);
            }
            _ => return Err("invalid POSIX ACL entry tag".to_string()),
        }
        entry[2..4].copy_from_slice(&perm.to_le_bytes());
    }

    if has_named_entry && !has_mask {
        return Err("POSIX ACL with named entries is missing ACL_MASK".to_string());
    }
    let owner_perm = user_obj.ok_or_else(|| "POSIX ACL is missing ACL_USER_OBJ".to_string())?;
    let group_perm = acl_mask
        .or(group_obj)
        .ok_or_else(|| "POSIX ACL is missing ACL_GROUP_OBJ".to_string())?;
    let other_perm = other.ok_or_else(|| "POSIX ACL is missing ACL_OTHER".to_string())?;
    let mode = (mode & !0o777)
        | (u32::from(owner_perm) << 6)
        | (u32::from(group_perm) << 3)
        | u32::from(other_perm);
    Ok((access_acl, mode))
}

pub(super) fn prepare_default_acl_inheritance_on_conn(
    conn: *mut super::PGconn,
    parent_id: Option<u64>,
    requested_mode: u32,
    umask: u32,
    child_is_directory: bool,
) -> Result<(u32, Option<InheritedPosixAcl>), String> {
    let parent_id = if let Some(parent_id) = parent_id {
        CString::new(parent_id.to_string())
            .map_err(|_| "parent id contains NUL byte".to_string())?
    } else {
        let root_sql = CString::new(
            "SELECT id_directory FROM directories WHERE id_parent IS NULL AND name='/'",
        )
        .unwrap();
        let root_id = unsafe { fetch_first_row_texts(exec_params(conn, &root_sql, &[])?)? }
            .into_iter()
            .next()
            .ok_or_else(|| "FOD_NAMESPACE_PARENT_MISSING: logical root is missing".to_string())?;
        CString::new(root_id).map_err(|_| "root id contains NUL byte".to_string())?
    };
    let lock_parent =
        CString::new("SELECT id_directory FROM directories WHERE id_directory = $1 FOR SHARE")
            .unwrap();
    let parent_row = unsafe { exec_params(conn, &lock_parent, &[&parent_id])? };
    if unsafe { fetch_first_row_texts(parent_row) }?.is_empty() {
        return Err("FOD_NAMESPACE_PARENT_MISSING".to_string());
    }

    let fetch_acl = CString::new(
        "SELECT encode(value, 'base64') FROM xattrs WHERE owner_kind = 'dir' AND owner_id = $1 AND name = 'system.posix_acl_default'",
    )
    .unwrap();
    let encoded = unsafe { fetch_first_row_texts(exec_params(conn, &fetch_acl, &[&parent_id])?)? }
        .first()
        .cloned();
    let Some(encoded) = encoded else {
        return Ok((requested_mode & !umask, None));
    };
    let default_acl = BASE64_STANDARD
        .decode(encoded.trim())
        .map_err(|err| format!("invalid parent POSIX ACL payload: {err}"))?;
    let (access_acl, mode) = constrain_default_acl_to_mode(&default_acl, requested_mode)?;
    Ok((
        mode,
        Some(InheritedPosixAcl {
            access_acl,
            default_acl: child_is_directory.then_some(default_acl),
        }),
    ))
}

pub(super) fn store_inherited_default_acl_on_conn(
    conn: *mut super::PGconn,
    owner_kind: &str,
    owner_id: u64,
    inherited: &InheritedPosixAcl,
) -> Result<(), String> {
    let store_acl = CString::new(
        "INSERT INTO xattrs (owner_kind, owner_id, name, value) VALUES ($1, $2, $3, decode($4, 'base64')) ON CONFLICT (owner_kind, owner_id, name) DO UPDATE SET value = EXCLUDED.value",
    )
    .unwrap();
    let owner_kind =
        CString::new(owner_kind).map_err(|_| "xattr owner kind contains NUL byte".to_string())?;
    let owner_id = CString::new(owner_id.to_string())
        .map_err(|_| "xattr owner id contains NUL byte".to_string())?;
    let access_name = CString::new("system.posix_acl_access").unwrap();
    let access_value = CString::new(BASE64_STANDARD.encode(&inherited.access_acl))
        .map_err(|_| "inherited access ACL contains NUL byte".to_string())?;
    unsafe {
        exec_command_params(
            conn,
            &store_acl,
            &[&owner_kind, &owner_id, &access_name, &access_value],
        )?;
    }
    if let Some(default_acl) = inherited.default_acl.as_deref() {
        let default_name = CString::new("system.posix_acl_default").unwrap();
        let default_value = CString::new(BASE64_STANDARD.encode(default_acl))
            .map_err(|_| "inherited default ACL contains NUL byte".to_string())?;
        unsafe {
            exec_command_params(
                conn,
                &store_acl,
                &[&owner_kind, &owner_id, &default_name, &default_value],
            )?;
        }
    }
    Ok(())
}

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
            Some("dir") => (
                "dir",
                resolved
                    .entry_id
                    .ok_or_else(|| "directory lookup returned no id".to_string())?,
            ),
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
            let encoded = match PQresultStatus(res) {
                PGRES_TUPLES_OK => {
                    let rows = PQntuples(res);
                    let cols = PQnfields(res);
                    if rows == 0 {
                        Ok(None)
                    } else if rows != 1 || cols < 1 {
                        Err("invalid xattr value row returned by PostgreSQL".to_string())
                    } else if PQgetisnull(res, 0, 0) != 0 {
                        Err("xattr value row contains a NULL payload".to_string())
                    } else {
                        let value = PQgetvalue(res, 0, 0);
                        if value.is_null() {
                            Err("xattr value row contains a missing payload".to_string())
                        } else {
                            Ok(Some(
                                std::ffi::CStr::from_ptr(value)
                                    .to_string_lossy()
                                    .to_string(),
                            ))
                        }
                    }
                }
                _ => Err(result_error(res)),
            };
            PQclear(res);
            let encoded = encoded?;
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

    /// Set an xattr after authorizing its inode owner in the same transaction.
    /// The inode lock also makes CREATE/REPLACE atomic with concurrent writers.
    pub fn store_xattr_value_for_owner_as(
        &self,
        owner_kind: &str,
        owner_id: u64,
        name: &str,
        value: &[u8],
        create_only: bool,
        replace_only: bool,
        actor_uid: u32,
        actor_gid: u32,
        actor_groups: &[u32],
        use_posix_acl: bool,
    ) -> Result<(), String> {
        let (table, key) = xattr_owner_table(owner_kind)?;
        let owner_id_value = owner_id;
        let owner_id = CString::new(owner_id_value.to_string())
            .map_err(|_| "xattr owner id contains NUL byte".to_string())?;
        let name = CString::new(name).map_err(|_| "xattr name contains NUL byte".to_string())?;
        let value = CString::new(BASE64_STANDARD.encode(value))
            .map_err(|_| "xattr value contains NUL byte".to_string())?;
        let select_owner =
            CString::new(format!("SELECT 1 FROM {table} WHERE {key} = $1 FOR UPDATE")).unwrap();
        let exists = CString::new(
            "SELECT EXISTS (SELECT 1 FROM xattrs WHERE owner_kind=$1 AND owner_id=$2 AND name=$3)::text",
        )
        .unwrap();
        let upsert = CString::new(
            "INSERT INTO xattrs (owner_kind, owner_id, name, value)
             VALUES ($1, $2, $3, decode($4, 'base64'))
             ON CONFLICT (owner_kind, owner_id, name) DO UPDATE SET value=EXCLUDED.value",
        )
        .unwrap();
        let owner_kind = CString::new(owner_kind)
            .map_err(|_| "xattr owner kind contains NUL byte".to_string())?;

        self.with_transaction(|tx| unsafe {
            let row = fetch_first_row_texts(exec_params(tx.conn, &select_owner, &[&owner_id])?)?;
            if row.is_empty() {
                return Err("FOD_XATTR_OWNER_MISSING".to_string());
            }
            authorize_xattr_mutation_on_conn(
                tx.conn,
                owner_kind.to_str().map_err(|err| err.to_string())?,
                owner_id_value,
                name.to_str().map_err(|err| err.to_string())?,
                actor_uid,
                actor_gid,
                actor_groups,
                use_posix_acl,
            )?;
            let present = fetch_first_row_texts(exec_params(
                tx.conn,
                &exists,
                &[&owner_kind, &owner_id, &name],
            )?)?
            .first()
            .is_some_and(|value| matches!(value.as_str(), "t" | "true"));
            if create_only && present {
                return Err("FOD_XATTR_EXISTS".to_string());
            }
            if replace_only && !present {
                return Err("FOD_XATTR_MISSING".to_string());
            }
            exec_command_params(tx.conn, &upsert, &[&owner_kind, &owner_id, &name, &value])
        })
    }

    /// Remove an xattr after authorization against the locked inode row.
    /// Linux VFS also sends REMOVEXATTR for `security.capability` as an
    /// internal write/truncate side effect (`cap_inode_killpriv()` uses
    /// `__vfs_removexattr()`), with the writer's fsuid. That operation is
    /// intentionally allowed here because it can only discard privileges;
    /// explicit unprivileged removexattr syscalls are rejected in the kernel
    /// before they reach FUSE.
    pub fn remove_xattr_for_owner_as(
        &self,
        owner_kind: &str,
        owner_id: u64,
        name: &str,
        actor_uid: u32,
        actor_gid: u32,
        actor_groups: &[u32],
        use_posix_acl: bool,
    ) -> Result<u64, String> {
        let (table, key) = xattr_owner_table(owner_kind)?;
        let owner_id_value = owner_id;
        let owner_id = CString::new(owner_id_value.to_string())
            .map_err(|_| "xattr owner id contains NUL byte".to_string())?;
        let name = CString::new(name).map_err(|_| "xattr name contains NUL byte".to_string())?;
        let select_owner =
            CString::new(format!("SELECT 1 FROM {table} WHERE {key} = $1 FOR UPDATE")).unwrap();
        let delete = CString::new(
            "DELETE FROM xattrs WHERE owner_kind=$1 AND owner_id=$2 AND name=$3 RETURNING 1",
        )
        .unwrap();
        let owner_kind = CString::new(owner_kind)
            .map_err(|_| "xattr owner kind contains NUL byte".to_string())?;

        self.with_transaction(|tx| unsafe {
            let row = fetch_first_row_texts(exec_params(tx.conn, &select_owner, &[&owner_id])?)?;
            if row.is_empty() {
                return Err("FOD_XATTR_OWNER_MISSING".to_string());
            }
            if name.to_bytes() != b"security.capability" {
                authorize_xattr_mutation_on_conn(
                    tx.conn,
                    owner_kind.to_str().map_err(|err| err.to_string())?,
                    owner_id_value,
                    name.to_str().map_err(|err| err.to_string())?,
                    actor_uid,
                    actor_gid,
                    actor_groups,
                    use_posix_acl,
                )?;
            }
            let result = exec_params(tx.conn, &delete, &[&owner_kind, &owner_id, &name])?;
            if PQresultStatus(result) != PGRES_TUPLES_OK {
                let error = result_error(result);
                PQclear(result);
                return Err(error);
            }
            let rows = PQntuples(result);
            PQclear(result);
            Ok(rows as u64)
        })
    }

    /// Store a POSIX ACL with inode ownership checks in one primary transaction.
    /// Access ACL mode bits are synchronized under the same inode row lock.
    pub fn store_posix_acl_for_owner(
        &self,
        owner_kind: &str,
        owner_id: u64,
        name: &str,
        value: &[u8],
        permission_bits: Option<u16>,
        create_only: bool,
        replace_only: bool,
        actor_uid: u32,
    ) -> Result<(), String> {
        self.store_posix_acl_for_owner_with_kill_sgid(
            owner_kind,
            owner_id,
            name,
            value,
            permission_bits,
            create_only,
            replace_only,
            actor_uid,
            false,
        )
    }

    pub fn store_posix_acl_for_owner_with_kill_sgid(
        &self,
        owner_kind: &str,
        owner_id: u64,
        name: &str,
        value: &[u8],
        permission_bits: Option<u16>,
        create_only: bool,
        replace_only: bool,
        actor_uid: u32,
        kill_sgid: bool,
    ) -> Result<(), String> {
        let (table, key) = match owner_kind {
            "file" => ("files", "id_file"),
            "dir" => ("directories", "id_directory"),
            _ => return Err("access ACL mode sync requires a file or directory".to_string()),
        };
        if (name == "system.posix_acl_access") != permission_bits.is_some()
            || (name == "system.posix_acl_default" && owner_kind != "dir")
            || !matches!(name, "system.posix_acl_access" | "system.posix_acl_default")
        {
            return Err("invalid POSIX ACL owner/name/mode combination".to_string());
        }
        let owner_kind = CString::new(owner_kind)
            .map_err(|_| "xattr owner kind contains NUL byte".to_string())?;
        let owner_id_text = owner_id.to_string();
        let owner_id_param = CString::new(owner_id_text.clone())
            .map_err(|_| "xattr owner id contains NUL byte".to_string())?;
        let name = CString::new(name).map_err(|_| "xattr name contains NUL byte".to_string())?;
        let value_b64 = CString::new(BASE64_STANDARD.encode(value))
            .map_err(|_| "xattr value contains NUL byte".to_string())?;
        let select_mode = CString::new(format!(
            "SELECT uid, mode FROM {table} WHERE {key} = $1 FOR UPDATE"
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
            let Some(owner_uid_text) = mode_row.first() else {
                return Err("FOD_XATTR_OWNER_MISSING".to_string());
            };
            let owner_uid = owner_uid_text
                .parse::<u32>()
                .map_err(|err| format!("invalid inode owner: {err}"))?;
            if actor_uid != 0 && actor_uid != owner_uid {
                return Err("FOD_XATTR_PERMISSION_DENIED".to_string());
            }
            let mode_text = mode_row
                .get(1)
                .ok_or_else(|| "inode mode missing from locked row".to_string())?;
            let current_mode = parse_persisted_posix_mode(mode_text)?;
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
            exec_command_params(
                tx.conn,
                &store_acl,
                &[&owner_kind, &owner_id_param, &name, &value_b64],
            )?;
            if let Some(permission_bits) = permission_bits {
                let mut synchronized_mode =
                    (current_mode & !0o777) | (u32::from(permission_bits) & 0o777);
                if kill_sgid {
                    synchronized_mode &= !0o2000;
                }
                let mode_param = CString::new(format!("{synchronized_mode:o}"))
                    .map_err(|_| "mode contains NUL byte".to_string())?;
                let update_mode = CString::new(format!(
                    "UPDATE {table} SET mode = $1, change_date = NOW(), modification_date = NOW() WHERE {key} = $2"
                ))
                .unwrap();
                exec_command_params(tx.conn, &update_mode, &[&mode_param, &owner_id_param])
            } else {
                Ok(())
            }
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
            decode_xattr_name_rows(res)
        })
    }
}
