// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! Namespace counts and filesystem capacity snapshots using repository read routing.
//! Mutation callers remain responsible for validation in their authority transaction.

use super::{
    exec_params, exec_prepared_params, fetch_first_row_texts, fetch_single_text, DbRepo,
    PreparedStatement, StatfsSnapshotRows,
};
use std::ffi::CString;

impl DbRepo {
    pub fn count_file_links(&self, file_id: u64) -> Result<u64, String> {
        let sql = CString::new(
            "SELECT 1 + COUNT(h.id_hardlink) \
             FROM files f LEFT JOIN hardlinks h ON h.id_file = f.id_file \
             WHERE f.id_file = $1 GROUP BY f.id_file",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id];
            let res = exec_params(conn, &sql, &params)?;
            let text = fetch_single_text(res)?;
            text.trim()
                .parse::<u64>()
                .map_err(|_| "invalid file link count".to_string())
        })
    }

    pub fn count_file_blocks(&self, file_id: u64) -> Result<u64, String> {
        let sql_blocks = CString::new(
            "
            SELECT COUNT(*)
            FROM data_blocks db
            JOIN files f ON f.data_object_id = db.data_object_id
            WHERE f.id_file = $1
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id];
            let res = exec_params(conn, &sql_blocks, &params)?;
            let text = fetch_single_text(res)?;
            text.trim()
                .parse::<u64>()
                .map_err(|_| "invalid file block count".to_string())
        })
    }

    pub fn path_has_children(&self, directory_id: u64) -> Result<bool, String> {
        let sql = CString::new(
            "
            SELECT 1
            FROM files
            WHERE id_directory = $1 AND NOT unlinked
            UNION ALL
            SELECT 1
            FROM directories
            WHERE id_parent = $1
            UNION ALL
            SELECT 1
            FROM hardlinks
            WHERE id_directory = $1
            UNION ALL
            SELECT 1
            FROM symlinks
            WHERE id_parent = $1
            LIMIT 1
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&directory_id];
            let res = exec_params(conn, &sql, &params)?;
            Ok(super::fetch_single_optional_text(res)?.is_some())
        })
    }

    pub fn count_directory_children(&self, directory_id: u64) -> Result<u64, String> {
        let sql = CString::new(
            "
            SELECT
                (SELECT COUNT(*) FROM directories WHERE id_parent = $1)
              + (SELECT COUNT(*) FROM files WHERE id_directory = $1 AND NOT unlinked)
              + (SELECT COUNT(*) FROM hardlinks WHERE id_directory = $1)
              + (SELECT COUNT(*) FROM symlinks WHERE id_parent = $1)
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&directory_id];
            let res = exec_params(conn, &sql, &params)?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid directory children count".to_string())?;
            Ok(value)
        })
    }

    pub fn count_directory_subdirs(&self, directory_id: u64) -> Result<u64, String> {
        let sql = CString::new("SELECT COUNT(*) FROM directories WHERE id_parent = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let directory_id = CString::new(directory_id.to_string())
            .map_err(|_| "directory id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&directory_id];
            let res = exec_params(conn, &sql, &params)?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid directory subdir count".to_string())?;
            Ok(value)
        })
    }

    pub fn count_root_directory_children(&self) -> Result<u64, String> {
        let sql = CString::new(
            "
            SELECT COUNT(*)
            FROM directories
            WHERE id_parent IS NULL AND name != '/'
            ",
        )
        .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = exec_params(conn, &sql, &[])?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid root directory children count".to_string())?;
            Ok(value)
        })
    }

    pub fn count_symlinks(&self) -> Result<u64, String> {
        let sql = CString::new("SELECT COUNT(*) FROM symlinks")
            .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = exec_params(conn, &sql, &[])?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid symlink count".to_string())?;
            Ok(value)
        })
    }

    pub fn count_files(&self) -> Result<u64, String> {
        let sql = CString::new("SELECT COUNT(*) FROM files")
            .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = exec_params(conn, &sql, &[])?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid file count".to_string())?;
            Ok(value)
        })
    }

    pub fn count_directories(&self) -> Result<u64, String> {
        let sql = CString::new("SELECT COUNT(*) FROM directories")
            .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = exec_params(conn, &sql, &[])?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid directory count".to_string())?;
            Ok(value)
        })
    }

    pub fn total_data_size(&self) -> Result<u64, String> {
        let sql = CString::new("SELECT COALESCE(SUM(LENGTH(data)), 0) FROM data_blocks")
            .map_err(|_| "SQL contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let res = exec_params(conn, &sql, &[])?;
            let text = fetch_single_text(res)?;
            let value = text
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid total data size".to_string())?;
            Ok(value)
        })
    }

    pub fn statfs_snapshot(&self) -> Result<StatfsSnapshotRows, String> {
        self.with_read_connection(|conn| unsafe {
            let res = exec_prepared_params(conn, PreparedStatement::StatfsSnapshot, &[])?;
            let values = fetch_first_row_texts(res)?;
            if values.len() < 6 {
                return Err("invalid statfs snapshot".to_string());
            }
            let files = values[0]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid file count".to_string())?;
            let dirs = values[1]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid directory count".to_string())?;
            let symlinks = values[2]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid symlink count".to_string())?;
            let total_data_size = values[3]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid total data size".to_string())?;
            let reserved_data_size = values[4]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid reserved data size".to_string())?;
            let max_fs_size_bytes = values[5]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid max filesystem size".to_string())?;
            Ok((
                files,
                dirs,
                symlinks,
                total_data_size,
                reserved_data_size,
                (max_fs_size_bytes > 0).then_some(max_fs_size_bytes),
            ))
        })
    }
}
