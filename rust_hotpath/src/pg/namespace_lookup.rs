// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! Read-only namespace lookups and metadata decoding. All reads use the parent
//! repository's connection routing, including an authoritative view when selected
//! by the caller for mutation preconditions.

use super::{
    conn_error, exec_params, exec_prepared_params, fetch_first_column_texts, fetch_first_row_texts,
    fetch_rows_text, fetch_single_text, join_nul_text, result_error, DbRepo, PQclear, PQgetisnull,
    PQgetvalue, PQnfields, PQntuples, PQresultStatus, PreparedStatement, PGRES_TUPLES_OK,
};
use std::ffi::{CStr, CString};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    pub parent_id: Option<u64>,
    pub kind: Option<String>,
    pub entry_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntryMetadata {
    pub name: String,
    pub kind: String,
    pub entry_id: u64,
    pub inode_seed: String,
    pub special_type: Option<String>,
}

enum DirectoryLookup {
    Root,
    Directory(u64),
    Missing,
}

unsafe fn parse_optional_namespace_id(
    res: *mut super::PGresult,
    description: &str,
) -> Result<Option<u64>, String> {
    if PQntuples(res) < 1 {
        return Ok(None);
    }
    if PQnfields(res) < 1 {
        return Err(format!("{description} row is missing its id column"));
    }
    if PQgetisnull(res, 0, 0) != 0 {
        return Err(format!("{description} returned a NULL id"));
    }
    let value_ptr = PQgetvalue(res, 0, 0);
    if value_ptr.is_null() {
        return Err(format!("{description} returned a null id pointer"));
    }
    let value = CStr::from_ptr(value_ptr).to_string_lossy();
    value
        .trim()
        .parse::<u64>()
        .map(Some)
        .map_err(|err| format!("{description} returned an invalid id: {err}"))
}

fn parse_directory_entry_metadata_rows(
    rows: Vec<Vec<String>>,
) -> Result<Vec<DirectoryEntryMetadata>, String> {
    rows.into_iter()
        .map(|row| {
            if row.len() < 5 {
                return Err("directory entry metadata row is missing fields".to_string());
            }
            let entry_id = row[2]
                .trim()
                .parse::<u64>()
                .map_err(|err| format!("invalid directory entry id: {err}"))?;
            let special_type = match row[4].trim() {
                "" => None,
                value => Some(value.to_string()),
            };
            Ok(DirectoryEntryMetadata {
                name: row[0].clone(),
                kind: row[1].clone(),
                entry_id,
                inode_seed: row[3].clone(),
                special_type,
            })
        })
        .collect()
}

impl DbRepo {
    /// Resolve the entire directory path. The logical root and a missing path
    /// both return `None`; namespace callers must distinguish them before using
    /// SQL NULL as the parent of root-level entries.
    pub fn get_dir_id(&self, path: &str) -> Result<Option<u64>, String> {
        let path = CString::new(path).map_err(|_| "path contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let params = [&path];
                let res = exec_prepared_params(conn, PreparedStatement::GetDirId, &params)?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let value = parse_optional_namespace_id(res, "directory lookup");
                            PQclear(res);
                            value
                        }
                        _ => {
                            let error = result_error(res);
                            PQclear(res);
                            Err(error)
                        }
                    }
                }
            };
            result
        })
    }

    fn lookup_directory(&self, path: &str) -> Result<DirectoryLookup, String> {
        if path.trim_matches('/').is_empty() {
            return Ok(DirectoryLookup::Root);
        }
        Ok(match self.get_dir_id(path)? {
            Some(id) => DirectoryLookup::Directory(id),
            None => DirectoryLookup::Missing,
        })
    }

    pub fn get_file_id(&self, path: &str) -> Result<Option<u64>, String> {
        let normalized = path.trim();
        let (parent_path, file_name) = match normalized.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => {
                (if parent.is_empty() { "/" } else { parent }, name)
            }
            _ => ("/", normalized),
        };
        let parent_id = match self.lookup_directory(parent_path)? {
            DirectoryLookup::Root => None,
            DirectoryLookup::Directory(id) => Some(id),
            DirectoryLookup::Missing => return Ok(None),
        };
        let file_name =
            CString::new(file_name).map_err(|_| "path contains NUL byte".to_string())?;
        let parent_id_text = parent_id
            .map(|value| {
                CString::new(value.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())
            })
            .transpose()?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let res = if let Some(ref parent_id_text) = parent_id_text {
                    let params = [&file_name, parent_id_text];
                    exec_prepared_params(conn, PreparedStatement::GetFileIdNested, &params)
                } else {
                    let params = [&file_name];
                    exec_prepared_params(conn, PreparedStatement::GetFileIdRoot, &params)
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let value = parse_optional_namespace_id(res, "file lookup");
                            PQclear(res);
                            value
                        }
                        _ => {
                            let error = result_error(res);
                            PQclear(res);
                            Err(error)
                        }
                    }
                }
            };
            result
        })
    }

    pub fn get_file_mode_value(&self, path: &str) -> Result<Option<String>, String> {
        let normalized = path.trim();
        let (parent_path, file_name) = match normalized.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => {
                (if parent.is_empty() { "/" } else { parent }, name)
            }
            _ => ("/", normalized),
        };
        let parent_id = match self.lookup_directory(parent_path)? {
            DirectoryLookup::Root => None,
            DirectoryLookup::Directory(id) => Some(id),
            DirectoryLookup::Missing => return Ok(None),
        };
        let file_name =
            CString::new(file_name).map_err(|_| "path contains NUL byte".to_string())?;
        let parent_id_text = parent_id
            .map(|value| {
                CString::new(value.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())
            })
            .transpose()?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let res = if let Some(ref parent_id_text) = parent_id_text {
                    let params = [&file_name, parent_id_text];
                    exec_prepared_params(conn, PreparedStatement::GetFileModeNested, &params)
                } else {
                    let params = [&file_name];
                    exec_prepared_params(conn, PreparedStatement::GetFileModeRoot, &params)
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 1 {
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
                            Ok(value)
                        }
                        _ => {
                            let error = result_error(res);
                            PQclear(res);
                            Err(error)
                        }
                    }
                }
            };
            result
        })
    }

    pub fn get_hardlink_id(&self, path: &str) -> Result<Option<u64>, String> {
        let normalized = path.trim();
        let (parent_path, link_name) = match normalized.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => {
                (if parent.is_empty() { "/" } else { parent }, name)
            }
            _ => ("/", normalized),
        };
        let parent_id = match self.lookup_directory(parent_path)? {
            DirectoryLookup::Root => None,
            DirectoryLookup::Directory(id) => Some(id),
            DirectoryLookup::Missing => return Ok(None),
        };
        let link_name =
            CString::new(link_name).map_err(|_| "path contains NUL byte".to_string())?;
        let parent_id_text = parent_id
            .map(|value| {
                CString::new(value.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())
            })
            .transpose()?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let res = if let Some(ref parent_id_text) = parent_id_text {
                    let params = [&link_name, parent_id_text];
                    exec_prepared_params(conn, PreparedStatement::GetHardlinkIdNested, &params)
                } else {
                    let params = [&link_name];
                    exec_prepared_params(conn, PreparedStatement::GetHardlinkIdRoot, &params)
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let value = parse_optional_namespace_id(res, "hardlink lookup");
                            PQclear(res);
                            value
                        }
                        _ => {
                            let error = result_error(res);
                            PQclear(res);
                            Err(error)
                        }
                    }
                }
            };
            result
        })
    }

    pub fn get_hardlink_file_id(&self, hardlink_id: u64) -> Result<Option<u64>, String> {
        let hardlink_id = CString::new(hardlink_id.to_string())
            .map_err(|_| "hardlink id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let params = [&hardlink_id];
                let res =
                    exec_prepared_params(conn, PreparedStatement::GetHardlinkFileId, &params)?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let value = parse_optional_namespace_id(res, "hardlink target lookup");
                            PQclear(res);
                            value
                        }
                        _ => {
                            let error = result_error(res);
                            PQclear(res);
                            Err(error)
                        }
                    }
                }
            };
            result
        })
    }

    pub fn load_symlink_target(&self, symlink_id: u64) -> Result<Option<String>, String> {
        let symlink_id = CString::new(symlink_id.to_string())
            .map_err(|_| "symlink id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&symlink_id];
            let res = exec_prepared_params(conn, PreparedStatement::LoadSymlinkTarget, &params)?;
            let text = fetch_single_text(res)?;
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        })
    }

    pub fn get_special_file_metadata(
        &self,
        file_id: u64,
    ) -> Result<Option<(String, u32, u32)>, String> {
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id];
            let res =
                exec_prepared_params(conn, PreparedStatement::GetSpecialFileMetadata, &params)?;
            let result = {
                if PQresultStatus(res) != PGRES_TUPLES_OK {
                    let error = result_error(res);
                    PQclear(res);
                    return Err(error);
                }
                let rows = PQntuples(res);
                let cols = PQnfields(res);
                if rows < 1 || cols < 3 {
                    PQclear(res);
                    return Ok(None);
                }
                let file_type_ptr = PQgetvalue(res, 0, 0);
                let major_ptr = PQgetvalue(res, 0, 1);
                let minor_ptr = PQgetvalue(res, 0, 2);
                if file_type_ptr.is_null() || major_ptr.is_null() || minor_ptr.is_null() {
                    PQclear(res);
                    return Ok(None);
                }
                let file_type = CStr::from_ptr(file_type_ptr).to_string_lossy().to_string();
                let rdev_major = CStr::from_ptr(major_ptr)
                    .to_string_lossy()
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| "invalid special file major".to_string())?;
                let rdev_minor = CStr::from_ptr(minor_ptr)
                    .to_string_lossy()
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| "invalid special file minor".to_string())?;
                PQclear(res);
                Ok(Some((file_type, rdev_major, rdev_minor)))
            };
            result
        })
    }

    pub fn get_symlink_id(&self, path: &str) -> Result<Option<u64>, String> {
        let normalized = path.trim();
        let (parent_path, link_name) = match normalized.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => {
                (if parent.is_empty() { "/" } else { parent }, name)
            }
            _ => ("/", normalized),
        };
        let parent_id = match self.lookup_directory(parent_path)? {
            DirectoryLookup::Root => None,
            DirectoryLookup::Directory(id) => Some(id),
            DirectoryLookup::Missing => return Ok(None),
        };
        let link_name =
            CString::new(link_name).map_err(|_| "path contains NUL byte".to_string())?;
        let parent_id_text = parent_id
            .map(|value| {
                CString::new(value.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())
            })
            .transpose()?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let res = if let Some(ref parent_id_text) = parent_id_text {
                    let params = [&link_name, parent_id_text];
                    exec_prepared_params(conn, PreparedStatement::GetSymlinkIdNested, &params)
                } else {
                    let params = [&link_name];
                    exec_prepared_params(conn, PreparedStatement::GetSymlinkIdRoot, &params)
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let value = parse_optional_namespace_id(res, "symlink lookup");
                            PQclear(res);
                            value
                        }
                        _ => {
                            let error = result_error(res);
                            PQclear(res);
                            Err(error)
                        }
                    }
                }
            };
            result
        })
    }

    pub fn resolve_path(&self, path: &str) -> Result<ResolvedPath, String> {
        let normalized = path.trim();
        if normalized.is_empty() {
            return Ok(ResolvedPath {
                parent_id: None,
                kind: None,
                entry_id: None,
            });
        }

        let (parent_path, name) = match normalized.rsplit_once('/') {
            Some((parent, name)) if !name.is_empty() => {
                (if parent.is_empty() { "/" } else { parent }, name)
            }
            _ => ("/", normalized),
        };
        let parent_id = match self.lookup_directory(parent_path)? {
            DirectoryLookup::Root => None,
            DirectoryLookup::Directory(id) => Some(id),
            DirectoryLookup::Missing => {
                return Ok(ResolvedPath {
                    parent_id: None,
                    kind: None,
                    entry_id: None,
                });
            }
        };

        let name = CString::new(name).map_err(|_| "path contains NUL byte".to_string())?;
        let parent_id_text = parent_id
            .map(|value| {
                CString::new(value.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())
            })
            .transpose()?;

        self.with_read_connection(|conn| unsafe {
            let result = {
                let res = if let Some(ref parent_id_text) = parent_id_text {
                    let params = [&name, parent_id_text];
                    exec_prepared_params(conn, PreparedStatement::ResolvePathNested, &params)
                } else {
                    let params = [&name];
                    exec_prepared_params(conn, PreparedStatement::ResolvePathRoot, &params)
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let rows = PQntuples(res);
                            let cols = PQnfields(res);
                            let value = if rows < 1 || cols < 2 {
                                None
                            } else {
                                let kind_ptr = PQgetvalue(res, 0, 0);
                                let entry_ptr = PQgetvalue(res, 0, 1);
                                if kind_ptr.is_null() || entry_ptr.is_null() {
                                    None
                                } else {
                                    let kind =
                                        CStr::from_ptr(kind_ptr).to_string_lossy().to_string();
                                    let entry_id = CStr::from_ptr(entry_ptr)
                                        .to_string_lossy()
                                        .parse::<u64>()
                                        .ok();
                                    entry_id.map(|entry_id| (kind, entry_id))
                                }
                            };
                            PQclear(res);
                            Ok(value)
                        }
                        _ => {
                            let error = result_error(res);
                            PQclear(res);
                            Err(error)
                        }
                    }
                }
            };
            result.map(|entry| ResolvedPath {
                parent_id,
                kind: entry.as_ref().map(|(kind, _)| kind.clone()),
                entry_id: entry.map(|(_, entry_id)| entry_id),
            })
        })
    }

    pub fn list_directory_entry_metadata(
        &self,
        path: &str,
    ) -> Result<Vec<DirectoryEntryMetadata>, String> {
        let normalized = path.trim();
        let parent_id = match self.lookup_directory(normalized)? {
            DirectoryLookup::Root => None,
            DirectoryLookup::Directory(id) => Some(id),
            DirectoryLookup::Missing => return Ok(Vec::new()),
        };
        self.list_directory_entry_metadata_for_id(parent_id)
    }

    pub fn list_directory_entry_metadata_for_id(
        &self,
        parent_id: Option<u64>,
    ) -> Result<Vec<DirectoryEntryMetadata>, String> {
        let (sql, params) = if let Some(parent_id) = parent_id {
            (
                CString::new(
                    "
                    SELECT f.name, 'file', f.id_file, f.inode_seed, COALESCE(sf.file_type, '')
                    FROM files f
                    LEFT JOIN special_files sf ON sf.id_file = f.id_file
                    WHERE f.id_directory = $1 AND NOT f.unlinked
                    UNION ALL
                    SELECT h.name, 'hardlink', h.id_hardlink, f.inode_seed, COALESCE(sf.file_type, '')
                    FROM hardlinks h
                    JOIN files f ON f.id_file = h.id_file
                    LEFT JOIN special_files sf ON sf.id_file = f.id_file
                    WHERE h.id_directory = $1 AND NOT f.unlinked
                    UNION ALL
                    SELECT d.name, 'dir', d.id_directory, d.inode_seed, ''
                    FROM directories d
                    WHERE d.id_parent = $1
                    UNION ALL
                    SELECT s.name, 'symlink', s.id_symlink, s.inode_seed, ''
                    FROM symlinks s
                    WHERE s.id_parent = $1
                    ORDER BY name
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![CString::new(parent_id.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())?],
            )
        } else {
            (
                CString::new(
                    "
                    SELECT d.name, 'dir', d.id_directory, d.inode_seed, ''
                    FROM directories d
                    WHERE d.id_parent IS NULL AND d.name != '/'
                    UNION ALL
                    SELECT f.name, 'file', f.id_file, f.inode_seed, COALESCE(sf.file_type, '')
                    FROM files f
                    LEFT JOIN special_files sf ON sf.id_file = f.id_file
                    WHERE f.id_directory IS NULL AND NOT f.unlinked
                    UNION ALL
                    SELECT h.name, 'hardlink', h.id_hardlink, f.inode_seed, COALESCE(sf.file_type, '')
                    FROM hardlinks h
                    JOIN files f ON f.id_file = h.id_file
                    LEFT JOIN special_files sf ON sf.id_file = f.id_file
                    WHERE h.id_directory IS NULL AND NOT f.unlinked
                    UNION ALL
                    SELECT s.name, 'symlink', s.id_symlink, s.inode_seed, ''
                    FROM symlinks s
                    WHERE s.id_parent IS NULL
                    ORDER BY name
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?,
                Vec::new(),
            )
        };

        self.with_read_connection(|conn| unsafe {
            let param_refs = params.iter().collect::<Vec<_>>();
            let res = exec_params(conn, &sql, &param_refs)?;
            parse_directory_entry_metadata_rows(fetch_rows_text(res)?)
        })
    }

    pub fn list_directory_entries_blob(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let normalized = path.trim();
        let parent_id = match self.lookup_directory(normalized)? {
            DirectoryLookup::Root => None,
            DirectoryLookup::Directory(id) => Some(id),
            DirectoryLookup::Missing => return Ok(None),
        };
        self.list_directory_entries_blob_for_id(parent_id)
    }

    pub fn list_directory_entries_blob_for_id(
        &self,
        parent_id: Option<u64>,
    ) -> Result<Option<Vec<u8>>, String> {
        let (sql, params) = if let Some(parent_id) = parent_id {
            (
                CString::new(
                    "
                    SELECT name FROM files WHERE id_directory = $1 AND NOT unlinked
                    UNION ALL
                    SELECT name FROM hardlinks WHERE id_directory = $1
                    UNION ALL
                    SELECT name FROM directories WHERE id_parent = $1
                    UNION ALL
                    SELECT name FROM symlinks WHERE id_parent = $1
                    ORDER BY name
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?,
                vec![CString::new(parent_id.to_string())
                    .map_err(|_| "parent id contains NUL byte".to_string())?],
            )
        } else {
            (
                CString::new(
                    "
                    SELECT name FROM directories WHERE id_parent IS NULL AND name != '/'
                    UNION ALL
                    SELECT name FROM files WHERE id_directory IS NULL AND NOT unlinked
                    UNION ALL
                    SELECT name FROM hardlinks WHERE id_directory IS NULL
                    UNION ALL
                    SELECT name FROM symlinks WHERE id_parent IS NULL
                    ORDER BY name
                    ",
                )
                .map_err(|_| "SQL contains NUL byte".to_string())?,
                Vec::new(),
            )
        };

        self.with_read_connection(|conn| unsafe {
            let param_refs = params.iter().collect::<Vec<_>>();
            let res = exec_params(conn, &sql, &param_refs)?;
            let names = fetch_first_column_texts(res)?;
            Ok(Some(join_nul_text(&names)))
        })
    }

    pub fn directory_entry_metadata_for_id_and_name(
        &self,
        parent_id: Option<u64>,
        name: &str,
    ) -> Result<Option<DirectoryEntryMetadata>, String> {
        let parent_id = CString::new(parent_id.unwrap_or(0).to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;
        let name = CString::new(name).map_err(|_| "entry name contains NUL byte".to_string())?;
        let sql = CString::new(
            "
            SELECT name, kind, entry_id, inode_seed, special_type
            FROM (
                SELECT f.name, 'file'::text AS kind, f.id_file AS entry_id,
                       f.inode_seed, COALESCE(sf.file_type, '') AS special_type
                FROM files f
                LEFT JOIN special_files sf ON sf.id_file = f.id_file
                WHERE COALESCE(f.id_directory, 0) = $1 AND NOT f.unlinked
                UNION ALL
                SELECT h.name, 'hardlink'::text, h.id_hardlink, f.inode_seed,
                       COALESCE(sf.file_type, '')
                FROM hardlinks h
                JOIN files f ON f.id_file = h.id_file
                LEFT JOIN special_files sf ON sf.id_file = f.id_file
                WHERE COALESCE(h.id_directory, 0) = $1 AND NOT f.unlinked
                UNION ALL
                SELECT d.name, 'dir'::text, d.id_directory, d.inode_seed, ''::text
                FROM directories d
                WHERE COALESCE(d.id_parent, 0) = $1 AND d.name != '/'
                UNION ALL
                SELECT s.name, 'symlink'::text, s.id_symlink, s.inode_seed, ''::text
                FROM symlinks s
                WHERE COALESCE(s.id_parent, 0) = $1
            ) entries
            WHERE name = $2
            ORDER BY kind
            LIMIT 1
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let rows = fetch_rows_text(exec_params(conn, &sql, &[&parent_id, &name])?)?;
            let mut entries = parse_directory_entry_metadata_rows(rows)?;
            Ok(if entries.is_empty() {
                None
            } else {
                Some(entries.remove(0))
            })
        })
    }

    pub fn fetch_path_attrs_blob(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let resolved = self.resolve_path(path)?;
        let kind = match resolved.kind.as_deref() {
            Some(kind) => kind,
            None => return Ok(None),
        };
        let entry_id = match resolved.entry_id {
            Some(entry_id) => entry_id,
            None => return Ok(None),
        };
        let entry_id = CString::new(entry_id.to_string())
            .map_err(|_| "entry id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = {
                let res = match kind {
                    "file" => {
                        let params = [&entry_id];
                        exec_prepared_params(
                            conn,
                            PreparedStatement::FetchPathAttrsBlobFile,
                            &params,
                        )
                    }
                    "dir" => {
                        let params = [&entry_id];
                        exec_prepared_params(
                            conn,
                            PreparedStatement::FetchPathAttrsBlobDir,
                            &params,
                        )
                    }
                    "symlink" => {
                        let params = [&entry_id];
                        exec_prepared_params(
                            conn,
                            PreparedStatement::FetchPathAttrsBlobSymlink,
                            &params,
                        )
                    }
                    "hardlink" => {
                        let params = [&entry_id];
                        exec_prepared_params(
                            conn,
                            PreparedStatement::FetchPathAttrsBlobHardlink,
                            &params,
                        )
                    }
                    _ => return Ok(None),
                }?;
                if res.is_null() {
                    Err(conn_error(conn))
                } else {
                    match PQresultStatus(res) {
                        PGRES_TUPLES_OK => {
                            let row = fetch_first_row_texts(res)?;
                            if row.is_empty() {
                                Ok(None)
                            } else {
                                let mut output = Vec::new();
                                output.extend_from_slice(kind.as_bytes());
                                output.push(0);
                                output.extend_from_slice(&join_nul_text(&row));
                                Ok(Some(output))
                            }
                        }
                        _ => {
                            let error = result_error(res);
                            PQclear(res);
                            Err(error)
                        }
                    }
                }
            };
            res
        })
    }
}
