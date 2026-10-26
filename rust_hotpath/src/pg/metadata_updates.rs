// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! Mode, ownership and timestamp updates through the repository's authority pool.
//! File content generation belongs to content mutations, not metadata updates.

use super::{exec_command_params, DbRepo};
use std::ffi::CString;

impl DbRepo {
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
