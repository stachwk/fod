// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! Mode, ownership and timestamp updates through the repository's authority pool.
//! File content generation belongs to content mutations, not metadata updates.

use super::{
    exec_command_params, exec_params, fetch_first_row_texts, parse_persisted_posix_mode, DbRepo,
    PGconn,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use std::ffi::CString;

#[derive(Clone, Copy)]
pub enum InodeTimeValue<'a> {
    Now,
    Specific(&'a str),
}

pub struct InodeTimesChange<'a> {
    pub actor_uid: u32,
    pub actor_groups: &'a [u32],
    pub use_posix_acl: bool,
    pub atime: Option<InodeTimeValue<'a>>,
    pub mtime: Option<InodeTimeValue<'a>>,
}

#[derive(Debug, Clone)]
pub struct FileWriteAuthorization {
    pub actor_uid: u32,
    pub actor_groups: Vec<u32>,
    pub expected_inode_seed: String,
    pub use_posix_acl: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InodeTimesOutcome {
    Changed,
    Missing,
    Denied,
    WriteDenied,
}

fn acl_has_permissions(
    acl: &[u8],
    actor_uid: u32,
    actor_groups: &[u32],
    owner_uid: u32,
    owner_gid: u32,
    requested: u16,
) -> Result<bool, String> {
    if acl.len() < 4 || (acl.len() - 4) % 8 != 0 {
        return Err("invalid POSIX ACL xattr length".to_string());
    }
    if u32::from_le_bytes(acl[..4].try_into().unwrap()) != 2 {
        return Err("unsupported POSIX ACL xattr version".to_string());
    }
    let mut user_obj = None;
    let mut group_obj = None;
    let mut mask = None;
    let mut other = None;
    let mut named_user = None;
    let mut has_named_entry = false;
    let mut matched_group = false;
    let mut group_permissions = 0u16;
    let mut stage = 0u8;
    let mut named_user_ids = Vec::new();
    let mut named_group_ids = Vec::new();
    for entry in acl[4..].chunks_exact(8) {
        let tag = u16::from_le_bytes(entry[0..2].try_into().unwrap());
        let permissions = u16::from_le_bytes(entry[2..4].try_into().unwrap());
        if permissions & !0o7 != 0 {
            return Err("invalid POSIX ACL permission bits".to_string());
        }
        let id = u32::from_le_bytes(entry[4..8].try_into().unwrap());
        match tag {
            0x0001 if stage == 0 => {
                stage = 1;
                user_obj = Some(permissions);
            }
            0x0002 if stage == 1 && id != u32::MAX => {
                if named_user_ids.contains(&id) {
                    return Err("duplicate POSIX ACL named user".to_string());
                }
                named_user_ids.push(id);
                has_named_entry = true;
                if id == actor_uid {
                    named_user = Some(permissions);
                }
            }
            0x0004 if stage == 1 => {
                stage = 2;
                if actor_groups.contains(&owner_gid) {
                    matched_group = true;
                    group_permissions |= permissions;
                }
                group_obj = Some(permissions);
            }
            0x0008 if stage == 2 && id != u32::MAX => {
                if named_group_ids.contains(&id) {
                    return Err("duplicate POSIX ACL named group".to_string());
                }
                named_group_ids.push(id);
                has_named_entry = true;
                if actor_groups.contains(&id) {
                    matched_group = true;
                    group_permissions |= permissions;
                }
            }
            0x0010 if stage == 2 => {
                stage = 3;
                mask = Some(permissions);
            }
            0x0020 if stage == 2 || stage == 3 => {
                stage = 4;
                other = Some(permissions);
            }
            _ => return Err("invalid POSIX ACL entry".to_string()),
        }
    }
    if stage != 4 || user_obj.is_none() || group_obj.is_none() || other.is_none() {
        return Err("incomplete POSIX ACL xattr".to_string());
    }
    if has_named_entry && mask.is_none() {
        return Err("incomplete POSIX ACL xattr".to_string());
    }
    if actor_uid == owner_uid {
        return Ok(user_obj.unwrap_or(0) & requested == requested);
    }
    if let Some(permissions) = named_user {
        return Ok(permissions & mask.unwrap_or(0o7) & requested == requested);
    }
    if matched_group {
        let permissions = if actor_groups.contains(&owner_gid) {
            group_permissions | group_obj.unwrap_or(0)
        } else {
            group_permissions
        };
        return Ok(permissions & mask.unwrap_or(0o7) & requested == requested);
    }
    Ok(other.unwrap_or(0) & requested == requested)
}

#[cfg(test)]
mod tests {
    use super::acl_has_permissions;

    fn make_acl(entries: &[(u16, u16, u32)]) -> Vec<u8> {
        let mut acl = 2u32.to_le_bytes().to_vec();
        for (tag, permissions, id) in entries {
            acl.extend_from_slice(&tag.to_le_bytes());
            acl.extend_from_slice(&permissions.to_le_bytes());
            acl.extend_from_slice(&id.to_le_bytes());
        }
        acl
    }

    fn acl_with_named_user(user_entries: &[(u16, u32)]) -> Vec<u8> {
        let mut entries = vec![(0x0001, 0o7, u32::MAX)];
        entries.extend(
            user_entries
                .iter()
                .map(|(permissions, id)| (0x0002, *permissions, *id)),
        );
        entries.extend([
            (0x0004, 0o0, u32::MAX),
            (0x0010, 0o7, u32::MAX),
            (0x0020, 0o0, u32::MAX),
        ]);
        make_acl(&entries)
    }

    #[test]
    fn acl_permissions_reject_invalid_permission_bits() {
        let acl = make_acl(&[
            (0x0001, 0o7, u32::MAX),
            (0x0004, 0o0, u32::MAX),
            (0x0020, 0o10, u32::MAX),
        ]);
        assert!(acl_has_permissions(&acl, 1000, &[], 2000, 3000, 0o4)
            .unwrap_err()
            .contains("permission bits"));
    }

    #[test]
    fn acl_permissions_reject_duplicate_entries_but_allow_unsorted_ids() {
        let duplicate = acl_with_named_user(&[(0o0, 4000), (0o7, 4000)]);
        assert!(acl_has_permissions(&duplicate, 4000, &[], 2000, 3000, 0o4).is_err());

        let unordered = acl_with_named_user(&[(0o0, 4001), (0o7, 4000)]);
        assert_eq!(
            acl_has_permissions(&unordered, 4000, &[], 2000, 3000, 0o4),
            Ok(true)
        );
    }

    #[test]
    fn acl_permissions_preserve_valid_named_user_and_full_u32_identity() {
        let acl = acl_with_named_user(&[(0o4, 4_000_000_000)]);
        assert_eq!(
            acl_has_permissions(&acl, 4_000_000_000, &[], 2000, 3000, 0o4),
            Ok(true)
        );
        assert_eq!(
            acl_has_permissions(&acl, 4_000_000_000, &[], 2000, 3000, 0o2),
            Ok(false)
        );
    }
}

fn acl_write_access(
    acl: &[u8],
    actor_uid: u32,
    actor_groups: &[u32],
    owner_uid: u32,
    owner_gid: u32,
) -> Result<bool, String> {
    acl_has_permissions(acl, actor_uid, actor_groups, owner_uid, owner_gid, 0o2)
}

pub(super) unsafe fn directory_acl_allows_on_conn(
    conn: *mut PGconn,
    directory_id: u64,
    actor_uid: u32,
    actor_groups: &[u32],
    owner_uid: u32,
    owner_gid: u32,
    requested: u16,
) -> Result<Option<bool>, String> {
    inode_acl_allows_on_conn(
        conn,
        "dir",
        directory_id,
        actor_uid,
        actor_groups,
        owner_uid,
        owner_gid,
        requested,
    )
}

pub(super) unsafe fn inode_acl_allows_on_conn(
    conn: *mut PGconn,
    owner_kind: &str,
    inode_id: u64,
    actor_uid: u32,
    actor_groups: &[u32],
    owner_uid: u32,
    owner_gid: u32,
    requested: u16,
) -> Result<Option<bool>, String> {
    let rows = fetch_first_row_texts(exec_params(
        conn,
        &CString::new(
            "SELECT encode(value, 'base64') FROM xattrs WHERE owner_kind=$1 AND owner_id=$2 AND name='system.posix_acl_access'",
        )
        .unwrap(),
        &[
            &CString::new(owner_kind).map_err(|err| err.to_string())?,
            &CString::new(inode_id.to_string()).unwrap(),
        ],
    )?)?;
    let Some(encoded) = rows.first() else {
        return Ok(None);
    };
    let acl = BASE64_STANDARD
        .decode(encoded.trim())
        .map_err(|err| format!("invalid POSIX ACL payload: {err}"))?;
    acl_has_permissions(
        &acl,
        actor_uid,
        actor_groups,
        owner_uid,
        owner_gid,
        requested,
    )
    .map(Some)
}

fn time_sql_value(value: InodeTimeValue<'_>) -> String {
    match value {
        InodeTimeValue::Now => "clock_timestamp()".to_string(),
        InodeTimeValue::Specific(value) => {
            format!("'{}'::timestamp", value.replace('\'', "''"))
        }
    }
}

pub(super) unsafe fn validate_file_write_authorization_on_conn(
    conn: *mut PGconn,
    file_id: u64,
    authorization: &FileWriteAuthorization,
) -> Result<(), String> {
    let rows = fetch_first_row_texts(exec_params(
        conn,
        &CString::new("SELECT uid, gid, mode, inode_seed FROM files WHERE id_file = $1 FOR UPDATE")
            .unwrap(),
        &[&CString::new(file_id.to_string()).unwrap()],
    )?)?;
    if rows.len() < 4 || rows[3] != authorization.expected_inode_seed {
        return Err("FOD_TRUNCATE_TARGET_MISSING".to_string());
    }
    let owner_uid = rows[0].parse::<u32>().map_err(|err| err.to_string())?;
    let owner_gid = rows[1].parse::<u32>().map_err(|err| err.to_string())?;
    if authorization.actor_uid == 0 {
        return Ok(());
    }
    let mode = parse_persisted_posix_mode(&rows[2])?;
    let acl_rows = if authorization.use_posix_acl {
        fetch_first_row_texts(exec_params(
            conn,
            &CString::new(
                "SELECT encode(value, 'base64') FROM xattrs WHERE owner_kind='file' AND owner_id=$1 AND name='system.posix_acl_access'",
            )
            .unwrap(),
            &[&CString::new(file_id.to_string()).unwrap()],
        )?)?
    } else {
        Vec::new()
    };
    let has_write = if let Some(encoded) = acl_rows.first() {
        let acl = BASE64_STANDARD
            .decode(encoded.trim())
            .map_err(|err| format!("invalid POSIX ACL payload: {err}"))?;
        acl_write_access(
            &acl,
            authorization.actor_uid,
            &authorization.actor_groups,
            owner_uid,
            owner_gid,
        )?
    } else {
        let allowed = if authorization.actor_uid == owner_uid {
            (mode >> 6) & 0o7
        } else if authorization.actor_groups.contains(&owner_gid) {
            (mode >> 3) & 0o7
        } else {
            mode & 0o7
        };
        allowed & 0o2 != 0
    };
    if has_write {
        Ok(())
    } else {
        Err("FOD_TRUNCATE_WRITE_DENIED".to_string())
    }
}

/// Caller identity and requested fields for one authoritative chmod/chown.
pub struct InodePermissionChange<'a> {
    pub actor_uid: u32,
    pub actor_groups: &'a [u32],
    pub use_posix_acl: bool,
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InodePermissionOutcome {
    Changed,
    Missing,
    Denied,
}

impl DbRepo {
    /// Authorize utimens semantics and update only requested fields under the
    /// inode row lock. ACL writers take the same row lock, so permission
    /// evaluation is serialized with access ACL changes on the primary.
    pub fn change_inode_times(
        &self,
        kind: &str,
        id: u64,
        expected_seed: &str,
        change: &InodeTimesChange<'_>,
    ) -> Result<InodeTimesOutcome, String> {
        if change.atime.is_none() && change.mtime.is_none() {
            return Ok(InodeTimesOutcome::Changed);
        }
        let (table, key, acl_kind) = match kind {
            "file" => ("files", "id_file", Some("file")),
            "dir" => ("directories", "id_directory", Some("dir")),
            "symlink" => ("symlinks", "id_symlink", None),
            _ => return Err("invalid timestamp owner kind".to_string()),
        };
        self.with_transaction(|tx| {
            let mode_column = if kind == "symlink" { "'777'" } else { "mode" };
            let rows = tx.query_rows_text(&format!(
                "SELECT uid, gid, {mode_column}, inode_seed FROM {table} WHERE {key}={id} FOR UPDATE"
            ))?;
            let Some(row) = rows.first() else {
                return Ok(InodeTimesOutcome::Missing);
            };
            if row.len() != 4 || row[3] != expected_seed {
                return Ok(InodeTimesOutcome::Missing);
            }
            let owner_uid = row[0].parse::<u32>().map_err(|err| err.to_string())?;
            let owner_gid = row[1].parse::<u32>().map_err(|err| err.to_string())?;
            let mode = parse_persisted_posix_mode(&row[2])?;
            if change.actor_uid != 0 && change.actor_uid != owner_uid {
                let only_now_or_omit = [change.atime, change.mtime]
                    .into_iter()
                    .flatten()
                    .all(|value| matches!(value, InodeTimeValue::Now));
                if !only_now_or_omit || acl_kind.is_none() {
                    return Ok(if only_now_or_omit {
                        InodeTimesOutcome::WriteDenied
                    } else {
                        InodeTimesOutcome::Denied
                    });
                }
                let acl_rows = if change.use_posix_acl {
                    tx.query_rows_text(&format!(
                        "SELECT encode(value, 'base64') FROM xattrs WHERE owner_kind='{}' AND owner_id={id} AND name='system.posix_acl_access'",
                        acl_kind.unwrap()
                    ))?
                } else {
                    Vec::new()
                };
                let has_write = if let Some(encoded) = acl_rows.first().and_then(|row| row.first()) {
                    let acl = BASE64_STANDARD
                        .decode(encoded.trim())
                        .map_err(|err| format!("invalid POSIX ACL payload: {err}"))?;
                    acl_write_access(
                        &acl,
                        change.actor_uid,
                        change.actor_groups,
                        owner_uid,
                        owner_gid,
                    )?
                } else {
                    let allowed = if change.actor_groups.contains(&owner_gid) {
                        (mode >> 3) & 0o7
                    } else {
                        mode & 0o7
                    };
                    allowed & 0o2 != 0
                };
                if !has_write {
                    return Ok(InodeTimesOutcome::WriteDenied);
                }
            }

            let mut assignments = Vec::new();
            if let Some(atime) = change.atime {
                assignments.push(format!("access_date={}", time_sql_value(atime)));
            }
            if let Some(mtime) = change.mtime {
                assignments.push(format!(
                    "modification_date={}",
                    time_sql_value(mtime)
                ));
            }
            assignments.push("change_date=clock_timestamp()".to_string());
            tx.exec(&format!(
                "UPDATE {table} SET {} WHERE {key}={id}",
                assignments.join(",")
            ))?;
            Ok(InodeTimesOutcome::Changed)
        })
    }

    /// Lock the inode row before authorization and preserve unspecified fields.
    /// A stale namespace resolution cannot authorize a different inode seed.
    pub fn change_inode_permissions(
        &self,
        kind: &str,
        id: u64,
        expected_seed: &str,
        change: &InodePermissionChange<'_>,
    ) -> Result<InodePermissionOutcome, String> {
        let (table, key) = match kind {
            "file" => ("files", "id_file"),
            "dir" => ("directories", "id_directory"),
            "symlink" => ("symlinks", "id_symlink"),
            _ => return Err("invalid permission owner kind".to_string()),
        };
        self.with_transaction(|tx| {
            let mode_column = if kind == "symlink" { "'777'" } else { "mode" };
            let rows = tx.query_rows_text(&format!(
                "SELECT uid, gid, {mode_column}, inode_seed FROM {table} WHERE {key}={id} FOR UPDATE"
            ))?;
            let Some(row) = rows.first() else {
                return Ok(InodePermissionOutcome::Missing);
            };
            if row.len() != 4 || row[3] != expected_seed {
                return Ok(InodePermissionOutcome::Missing);
            }
            let uid = row[0].parse::<u32>().map_err(|err| err.to_string())?;
            let gid = row[1].parse::<u32>().map_err(|err| err.to_string())?;
            let current_mode = parse_persisted_posix_mode(&row[2])?;
            if change.actor_uid != 0 {
                if change.actor_uid != uid
                    || change.uid.is_some_and(|new_uid| new_uid != uid)
                    || change.gid.is_some_and(|new_gid| new_gid != gid && !change.actor_groups.contains(&new_gid))
                {
                    return Ok(InodePermissionOutcome::Denied);
                }
            }
            let new_uid = change.uid.unwrap_or(uid);
            let new_gid = change.gid.unwrap_or(gid);
            let mut mode = change.mode.map(|mode| mode & 0o7777).unwrap_or(current_mode);
            if change.mode.is_some() && change.actor_uid != 0 && !change.actor_groups.contains(&new_gid) {
                mode &= !0o2000;
            }
            // Linux clears privilege bits on file chown; directories retain them.
            if kind == "file" && (change.uid.is_some() || change.gid.is_some()) {
                mode &= !0o6000;
            }
            if change.mode.is_some() && change.use_posix_acl && matches!(kind, "file" | "dir") {
                let acl_rows = tx.query_rows_text(&format!(
                    "SELECT encode(value, 'base64') FROM xattrs WHERE owner_kind='{kind}' AND owner_id={id} AND name='system.posix_acl_access'"
                ))?;
                if let Some(encoded) = acl_rows.first().and_then(|row| row.first()) {
                    let acl = BASE64_STANDARD
                        .decode(encoded.trim())
                        .map_err(|err| format!("invalid POSIX ACL payload: {err}"))?;
                    let chmod_acl = super::xattrs::chmod_access_acl(&acl, mode)?;
                    let encoded_acl = BASE64_STANDARD.encode(chmod_acl);
                    tx.exec(&format!(
                        "UPDATE xattrs SET value=decode('{encoded_acl}', 'base64') WHERE owner_kind='{kind}' AND owner_id={id} AND name='system.posix_acl_access'"
                    ))?;
                }
            }
            let mode_update = if kind == "symlink" {
                String::new()
            } else {
                format!(", mode='{:o}'", mode)
            };
            tx.exec(&format!(
                "UPDATE {table} SET uid={new_uid}, gid={new_gid}{mode_update}, change_date=NOW(), modification_date=NOW() WHERE {key}={id}"
            ))?;
            if kind == "file" && (change.uid.is_some() || change.gid.is_some()) {
                tx.exec(&format!(
                    "DELETE FROM xattrs WHERE owner_kind='file' AND owner_id={id} AND name='security.capability'"
                ))?;
            }
            Ok(InodePermissionOutcome::Changed)
        })
    }

    /// Clear Linux privilege metadata while a content mutation holds the
    /// authoritative file row lock and persistence fence.
    pub(super) unsafe fn clear_file_privileges_on_conn(
        conn: *mut PGconn,
        file_id: u64,
    ) -> Result<(), String> {
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let select_mode = CString::new("SELECT mode FROM files WHERE id_file=$1 FOR UPDATE")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let Some(row) = fetch_first_row_texts(exec_params(conn, &select_mode, &[&file_id])?)?
            .first()
            .cloned()
        else {
            return Err("file missing while clearing privilege metadata".to_string());
        };
        let current_mode = parse_persisted_posix_mode(&row)?;
        let mut mode = current_mode & !0o4000;
        if current_mode & 0o2010 == 0o2010 {
            mode &= !0o2000;
        }
        let mode =
            CString::new(format!("{mode:o}")).map_err(|_| "mode contains NUL byte".to_string())?;
        let update_mode = CString::new(
            "UPDATE files SET mode=$1, change_date=clock_timestamp() WHERE id_file=$2",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        exec_command_params(conn, &update_mode, &[&mode, &file_id])?;

        let delete_capability = CString::new(
            "DELETE FROM xattrs WHERE owner_kind='file' AND owner_id=$1 AND name='security.capability'",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        exec_command_params(conn, &delete_capability, &[&file_id])
    }

    pub fn touch_file_entry(&self, file_id: u64) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE files SET modification_date = NOW(), change_date = NOW() WHERE id_file = $1",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&file_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn touch_directory_entry(&self, directory_id: u64) -> Result<(), String> {
        let sql = CString::new("UPDATE directories SET modification_date = NOW(), change_date = NOW() WHERE id_directory = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&directory_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn touch_symlink_entry(&self, symlink_id: u64) -> Result<(), String> {
        let sql = CString::new("UPDATE symlinks SET modification_date = NOW(), change_date = NOW() WHERE id_symlink = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let symlink_id = CString::new(symlink_id.to_string())
            .map_err(|_| "symlink id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&symlink_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_file_mode(&self, file_id: u64, mode: &str) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE files SET mode = $1, change_date = NOW(), modification_date = NOW() WHERE id_file = $2",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let mode = CString::new(mode).map_err(|_| "mode contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&mode, &file_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_directory_mode(&self, directory_id: u64, mode: &str) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE directories SET mode = $1, change_date = NOW(), modification_date = NOW() WHERE id_directory = $2",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let mode = CString::new(mode).map_err(|_| "mode contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&mode, &directory_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_file_owner(
        &self,
        file_id: u64,
        uid: u32,
        gid: u32,
        mode: &str,
    ) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE files SET uid = $1, gid = $2, mode = $3, change_date = NOW(), modification_date = NOW() WHERE id_file = $4",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let uid = CString::new(uid.to_string()).map_err(|_| "uid contains NUL byte".to_string())?;
        let gid = CString::new(gid.to_string()).map_err(|_| "gid contains NUL byte".to_string())?;
        let mode = CString::new(mode).map_err(|_| "mode contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&uid, &gid, &mode, &file_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_directory_owner(
        &self,
        directory_id: u64,
        uid: u32,
        gid: u32,
        mode: &str,
    ) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE directories SET uid = $1, gid = $2, mode = $3, change_date = NOW(), modification_date = NOW() WHERE id_directory = $4",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let uid = CString::new(uid.to_string()).map_err(|_| "uid contains NUL byte".to_string())?;
        let gid = CString::new(gid.to_string()).map_err(|_| "gid contains NUL byte".to_string())?;
        let mode = CString::new(mode).map_err(|_| "mode contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&uid, &gid, &mode, &directory_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_symlink_owner(&self, symlink_id: u64, uid: u32, gid: u32) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE symlinks SET uid = $1, gid = $2, change_date = NOW(), modification_date = NOW() WHERE id_symlink = $3",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let uid = CString::new(uid.to_string()).map_err(|_| "uid contains NUL byte".to_string())?;
        let gid = CString::new(gid.to_string()).map_err(|_| "gid contains NUL byte".to_string())?;
        let symlink_id = CString::new(symlink_id.to_string())
            .map_err(|_| "symlink id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&uid, &gid, &symlink_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_symlink_access_date(&self, symlink_id: u64, atime: &str) -> Result<(), String> {
        let sql = CString::new("UPDATE symlinks SET access_date = $1 WHERE id_symlink = $2")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let atime = CString::new(atime).map_err(|_| "atime contains NUL byte".to_string())?;
        let symlink_id = CString::new(symlink_id.to_string())
            .map_err(|_| "symlink id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&atime, &symlink_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn touch_file_times(&self, file_id: u64, atime: &str, mtime: &str) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE files SET access_date = $1, modification_date = $2, change_date = NOW() WHERE id_file = $3",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let atime = CString::new(atime).map_err(|_| "atime contains NUL byte".to_string())?;
        let mtime = CString::new(mtime).map_err(|_| "mtime contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&atime, &mtime, &file_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn touch_directory_times(
        &self,
        directory_id: u64,
        atime: &str,
        mtime: &str,
    ) -> Result<(), String> {
        let sql = CString::new(
            "UPDATE directories SET access_date = $1, modification_date = $2, change_date = NOW() WHERE id_directory = $3",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let atime = CString::new(atime).map_err(|_| "atime contains NUL byte".to_string())?;
        let mtime = CString::new(mtime).map_err(|_| "mtime contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&atime, &mtime, &directory_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_file_access_date(&self, file_id: u64, atime: &str) -> Result<(), String> {
        let sql = CString::new("UPDATE files SET access_date = $1 WHERE id_file = $2")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let atime = CString::new(atime).map_err(|_| "atime contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&atime, &file_id];
            exec_command_params(conn, &sql, &params)
        })
    }

    pub fn update_directory_access_date(
        &self,
        directory_id: u64,
        atime: &str,
    ) -> Result<(), String> {
        let sql = CString::new("UPDATE directories SET access_date = $1 WHERE id_directory = $2")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let atime = CString::new(atime).map_err(|_| "atime contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        self.with_cached_connection(|conn| unsafe {
            let params = [&atime, &directory_id];
            exec_command_params(conn, &sql, &params)
        })
    }
}
