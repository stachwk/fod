// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! File metadata, sparse seeks and payload reads through repository read routing.
//! Block and range decoders own and clear their PGresult; field helpers borrow it.
//! Generation-bearing reads retain one statement snapshot of metadata and blocks.

use super::{
    conn_error, exec_params, exec_prepared_params, exec_prepared_params_with_result_format,
    fetch_optional_single_row_texts_exact, fetch_single_text_option, fod_profile_io_enabled,
    fod_profile_result_decode_observe, normalize_block_bytes, result_error, DbRepo, PGresult,
    PQclear, PQexecParams, PQgetisnull, PQgetlength, PQgetvalue, PQnfields, PQntuples,
    PQresultStatus, PreparedStatement, PGRES_TUPLES_OK,
};
use std::ffi::{CStr, CString};
use std::os::raw::c_int;
use std::sync::Arc;
use std::time::Instant;

type SharedBlockRows = Vec<(u64, Arc<[u8]>)>;
type SizedSharedBlockRows = Option<(u64, SharedBlockRows)>;
// (file size, committed content generation, blocks) from one PostgreSQL statement.
pub type GeneratedSharedBlockRows = Option<(u64, u64, Vec<(u64, Arc<[u8]>)>)>;

unsafe fn decode_optional_file_size(res: *mut PGresult) -> Result<Option<u64>, String> {
    let result = match PQresultStatus(res) {
        PGRES_TUPLES_OK => {
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            if cols != 1 {
                Err(format!(
                    "invalid file size result: expected 1 column, got {cols}"
                ))
            } else if rows == 0 {
                Ok(None)
            } else if rows != 1 {
                Err(format!(
                    "invalid file size row: expected 1 row, got {rows} rows"
                ))
            } else if PQgetisnull(res, 0, 0) != 0 {
                Err("file size is NULL".to_string())
            } else {
                let value = PQgetvalue(res, 0, 0);
                if value.is_null() {
                    Err("file size value is missing".to_string())
                } else {
                    CStr::from_ptr(value)
                        .to_string_lossy()
                        .trim()
                        .parse::<u64>()
                        .map(Some)
                        .map_err(|err| format!("invalid file size value: {err}"))
                }
            }
        }
        _ => Err(result_error(res)),
    };
    PQclear(res);
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReadMetadata {
    pub size: u64,
    pub access_date: String,
    pub modification_date: String,
    pub change_date: String,
}

fn normalize_block_arc(bytes: &[u8], target_len: usize) -> Arc<[u8]> {
    let payload_len = bytes.len().min(target_len);
    let mut out = Arc::<[u8]>::new_uninit_slice(target_len);
    let out_mut = Arc::get_mut(&mut out).expect("newly allocated Arc slice is unique");
    unsafe {
        // Initialize every byte before converting the slice to Arc<[u8]>.
        let dst = out_mut.as_mut_ptr().cast::<u8>();
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, payload_len);
        std::ptr::write_bytes(
            dst.add(payload_len),
            0,
            target_len.saturating_sub(payload_len),
        );
        out.assume_init()
    }
}

unsafe fn read_result_binary_field<T, F>(
    res: *const PGresult,
    row: c_int,
    col: c_int,
    decode: F,
) -> Result<Option<T>, String>
where
    F: FnOnce(&[u8]) -> Result<T, String>,
{
    if PQgetisnull(res, row, col) != 0 {
        return Ok(None);
    }
    let len = PQgetlength(res, row, col);
    if len < 0 {
        return Err("invalid binary field length".to_string());
    }
    let value_ptr = PQgetvalue(res, row, col);
    if value_ptr.is_null() {
        return Err("binary field pointer is null".to_string());
    }
    let bytes = std::slice::from_raw_parts(value_ptr.cast::<u8>(), len as usize);
    decode(bytes).map(Some)
}

unsafe fn result_binary_i64(res: *const PGresult, row: c_int, col: c_int) -> Result<i64, String> {
    read_result_binary_field(res, row, col, |bytes| match bytes.len() {
        8 => Ok(i64::from_be_bytes(
            bytes
                .try_into()
                .map_err(|_| "invalid BIGINT binary field".to_string())?,
        )),
        4 => Ok(i32::from_be_bytes(
            bytes
                .try_into()
                .map_err(|_| "invalid INTEGER binary field".to_string())?,
        ) as i64),
        other => Err(format!("unexpected integer binary field length: {other}")),
    })?
    .ok_or_else(|| "integer field is null".to_string())
}

unsafe fn fetch_single_block_binary(
    res: *mut PGresult,
    block_size: usize,
) -> Result<Option<Vec<u8>>, String> {
    let result = match PQresultStatus(res) {
        PGRES_TUPLES_OK => {
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            if cols != 1 {
                Err(format!(
                    "invalid single block result: expected 1 column, got {cols}"
                ))
            } else if rows == 0 {
                Ok(None)
            } else if rows != 1 {
                Err(format!(
                    "invalid single block result: expected 1 row, got {rows} rows"
                ))
            } else if PQgetisnull(res, 0, 0) != 0 {
                Err("data block payload is NULL".to_string())
            } else {
                read_result_binary_field(res, 0, 0, |bytes| {
                    Ok(normalize_block_bytes(bytes, block_size))
                })
                .and_then(|value| {
                    value
                        .map(Some)
                        .ok_or_else(|| "data block payload is NULL".to_string())
                })
            }
        }
        _ => Err(result_error(res)),
    };
    PQclear(res);
    result
}

unsafe fn fetch_block_range_rows_shared(
    res: *mut PGresult,
    block_size: usize,
) -> Result<SharedBlockRows, String> {
    let profile_enabled = fod_profile_io_enabled();
    let started = if profile_enabled {
        Some(Instant::now())
    } else {
        None
    };
    let mut decoded_rows = 0u64;
    let mut decoded_bytes = 0u64;
    let result = (|| match PQresultStatus(res) {
        PGRES_TUPLES_OK => {
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            if cols != 2 {
                Err(format!(
                    "invalid block range result: expected 2 columns, got {cols}"
                ))
            } else if rows == 0 {
                Ok(Vec::new())
            } else if rows < 0 {
                Err("invalid block range row count returned by PostgreSQL".to_string())
            } else {
                let mut blocks = Vec::with_capacity(rows as usize);
                for row in 0..rows {
                    let index = result_binary_i64(res, row, 0)?;
                    let index = u64::try_from(index)
                        .map_err(|_| "negative block index value".to_string())?;
                    let bytes = match read_result_binary_field(res, row, 1, |bytes| {
                        Ok(normalize_block_arc(bytes, block_size))
                    })? {
                        Some(bytes) => bytes,
                        None => return Err("data block payload is NULL".to_string()),
                    };
                    if profile_enabled {
                        decoded_rows = decoded_rows.saturating_add(1);
                        decoded_bytes = decoded_bytes.saturating_add(bytes.len() as u64);
                    }
                    blocks.push((index, bytes));
                }
                Ok(blocks)
            }
        }
        _ => Err(result_error(res)),
    })();
    if let Some(started) = started {
        fod_profile_result_decode_observe(
            PreparedStatement::FetchBlockRange.name(),
            started.elapsed(),
            decoded_rows,
            decoded_bytes,
            result.is_err(),
        );
    }
    PQclear(res);
    result
}

// TX03J: decode a single-statement snapshot without changing legacy read APIs.
unsafe fn decode_generation_block_snapshot(
    res: *mut PGresult,
    block_size: usize,
) -> Result<GeneratedSharedBlockRows, String> {
    let result = (|| {
        if PQresultStatus(res) != PGRES_TUPLES_OK {
            return Err(result_error(res));
        }
        let cols = PQnfields(res);
        if cols != 4 {
            return Err(format!(
                "generation block snapshot column count mismatch: expected 4, got {cols}"
            ));
        }
        let mut metadata = None;
        let mut blocks = Vec::new();
        for row in 0..PQntuples(res) {
            if PQgetisnull(res, row, 0) == 0 {
                if PQgetisnull(res, row, 1) != 0
                    || PQgetisnull(res, row, 2) == 0
                    || PQgetisnull(res, row, 3) == 0
                {
                    return Err("malformed generation metadata row".to_string());
                }
                let size = u64::try_from(result_binary_i64(res, row, 0)?)
                    .map_err(|_| "negative snapshot file size".to_string())?;
                let generation = u64::try_from(result_binary_i64(res, row, 1)?)
                    .map_err(|_| "negative snapshot content generation".to_string())?;
                if metadata.replace((size, generation)).is_some() {
                    return Err("duplicate generation metadata row".to_string());
                }
            } else {
                if PQgetisnull(res, row, 1) == 0 || PQgetisnull(res, row, 2) != 0 {
                    return Err("malformed generation block row".to_string());
                }
                let index = u64::try_from(result_binary_i64(res, row, 2)?)
                    .map_err(|_| "negative snapshot block index".to_string())?;
                if let Some(data) = read_result_binary_field(res, row, 3, |bytes| {
                    Ok(normalize_block_arc(bytes, block_size))
                })? {
                    blocks.push((index, data));
                } else {
                    return Err("data block payload is NULL".to_string());
                }
            }
        }
        blocks.sort_unstable_by_key(|(index, _)| *index);
        Ok(metadata.map(|(size, generation)| (size, generation, blocks)))
    })();
    PQclear(res);
    result
}

unsafe fn fetch_block_range_with_size_rows_shared(
    res: *mut PGresult,
    block_size: usize,
) -> Result<SizedSharedBlockRows, String> {
    let profile_enabled = fod_profile_io_enabled();
    let started = profile_enabled.then(Instant::now);
    let mut decoded_rows = 0u64;
    let mut decoded_bytes = 0u64;
    let result = (|| match PQresultStatus(res) {
        PGRES_TUPLES_OK => {
            let rows = PQntuples(res);
            let cols = PQnfields(res);
            if cols != 3 {
                return Err(format!(
                    "invalid fused file read result: expected 3 columns, got {cols}"
                ));
            }
            if rows == 0 {
                return Ok(None);
            }
            if rows < 0 {
                return Err("invalid fused file read row count returned by PostgreSQL".to_string());
            }
            let mut file_size = None;
            let mut blocks = Vec::with_capacity(rows.saturating_sub(1) as usize);
            for row in 0..rows {
                if PQgetisnull(res, row, 0) == 0 {
                    if PQgetisnull(res, row, 1) == 0 || PQgetisnull(res, row, 2) == 0 {
                        return Err("malformed fused file metadata row".to_string());
                    }
                    let value = result_binary_i64(res, row, 0)?;
                    let value =
                        u64::try_from(value).map_err(|_| "negative file size value".to_string())?;
                    if file_size.replace(value).is_some() {
                        return Err("fused file read returned duplicate metadata rows".to_string());
                    }
                    continue;
                }
                if PQgetisnull(res, row, 1) != 0 {
                    return Err("fused block row is missing its block index".to_string());
                }
                let index = u64::try_from(result_binary_i64(res, row, 1)?)
                    .map_err(|_| "negative block index value".to_string())?;
                let bytes = match read_result_binary_field(res, row, 2, |bytes| {
                    Ok(normalize_block_arc(bytes, block_size))
                })? {
                    Some(bytes) => bytes,
                    None => return Err("data block payload is NULL".to_string()),
                };
                if profile_enabled {
                    decoded_rows = decoded_rows.saturating_add(1);
                    decoded_bytes = decoded_bytes.saturating_add(bytes.len() as u64);
                }
                blocks.push((index, bytes));
            }
            let Some(file_size) = file_size else {
                return Ok(None);
            };
            blocks.sort_unstable_by_key(|(index, _)| *index);
            Ok(Some((file_size, blocks)))
        }
        status => Err(format!(
            "fused file read PostgreSQL status={} error={}",
            status,
            result_error(res)
        )),
    })();
    if let Some(started) = started {
        fod_profile_result_decode_observe(
            PreparedStatement::FetchBlockRangeWithSize.name(),
            started.elapsed(),
            decoded_rows,
            decoded_bytes,
            result.is_err(),
        );
    }
    PQclear(res);
    result
}

impl DbRepo {
    pub fn file_data_object_id(&self, file_id: u64) -> Result<Option<u64>, String> {
        self.with_read_connection(|conn| self.file_data_object_id_on_conn(conn, file_id))
    }

    pub fn sparse_seek_blockwise(
        &self,
        file_id: u64,
        file_size: u64,
        offset: u64,
        block_size: u64,
        seek_data: bool,
    ) -> Result<Option<u64>, String> {
        if block_size == 0 {
            return Err("sparse seek block_size must be positive".to_string());
        }
        if offset >= file_size {
            return Ok(None);
        }

        let Some(data_object_id) = self.file_data_object_id(file_id)? else {
            return Err("file data object is missing during sparse seek".to_string());
        };

        let current_block = offset / block_size;
        let data_object_id_text = CString::new(data_object_id.to_string())
            .map_err(|_| "data object id contains NUL byte".to_string())?;
        let current_block_text = CString::new(current_block.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let next_sql = CString::new(
                "SELECT _order                  FROM data_blocks                  WHERE data_object_id = $1 AND _order >= $2                  ORDER BY _order                  LIMIT 1",
            )
            .map_err(|_| "SQL contains NUL byte".to_string())?;
            let params = [&data_object_id_text, &current_block_text];
            let next_res = exec_params(conn, &next_sql, &params)?;
            let next_order = fetch_single_text_option(next_res)?
                .map(|value| {
                    value
                        .trim()
                        .parse::<u64>()
                        .map_err(|_| "invalid data block order".to_string())
                })
                .transpose()?;

            if seek_data {
                let Some(order) = next_order else {
                    return Ok(None);
                };
                if order == current_block {
                    return Ok(Some(offset));
                }
                let candidate = order
                    .checked_mul(block_size)
                    .ok_or_else(|| "sparse seek data offset overflow".to_string())?;
                return if candidate < file_size {
                    Ok(Some(candidate))
                } else {
                    Ok(None)
                };
            }

            if next_order != Some(current_block) {
                return Ok(Some(offset));
            }

            let hole_sql = CString::new(
                "SELECT db._order                  FROM data_blocks db                  WHERE db.data_object_id = $1                    AND db._order >= $2                    AND NOT EXISTS (                        SELECT 1                        FROM data_blocks next                        WHERE next.data_object_id = db.data_object_id                          AND next._order = db._order + 1                    )                  ORDER BY db._order                  LIMIT 1",
            )
            .map_err(|_| "SQL contains NUL byte".to_string())?;
            let hole_res = exec_params(conn, &hole_sql, &params)?;
            let last_data_order = fetch_single_text_option(hole_res)?
                .map(|value| {
                    value
                        .trim()
                        .parse::<u64>()
                        .map_err(|_| "invalid data block order".to_string())
                })
                .transpose()?;

            let Some(last_data_order) = last_data_order else {
                return Ok(Some(file_size));
            };
            let next_block = last_data_order
                .checked_add(1)
                .ok_or_else(|| "sparse seek hole block overflow".to_string())?;
            let candidate = next_block
                .checked_mul(block_size)
                .ok_or_else(|| "sparse seek hole offset overflow".to_string())?;

            Ok(Some(candidate.min(file_size)))
        })
    }

    pub fn file_size(&self, file_id: u64) -> Result<Option<u64>, String> {
        let sql = CString::new("SELECT size FROM files WHERE id_file = $1")
            .map_err(|_| "SQL contains NUL byte".to_string())?;
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id];
            let res = exec_params(conn, &sql, &params)?;
            decode_optional_file_size(res)
        })
    }

    /// Reads file size from the write authority for mutation planning.
    /// Replica-routed size reads must not determine the extent persisted by a
    /// write or copy operation.
    pub fn file_size_authoritative(&self, file_id: u64) -> Result<Option<u64>, String> {
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        self.with_connection(super::ConnectionLane::Write, |conn| unsafe {
            let sql = CString::new("SELECT size FROM files WHERE id_file = $1")
                .map_err(|_| "SQL contains NUL byte".to_string())?;
            let res = exec_params(conn, &sql, &[&file_id])?;
            decode_optional_file_size(res)
        })
    }

    pub fn file_read_metadata(&self, file_id: u64) -> Result<Option<FileReadMetadata>, String> {
        let file_id = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id];
            let res = exec_prepared_params(conn, PreparedStatement::FileReadMetadata, &params)?;
            let Some(row) = fetch_optional_single_row_texts_exact(res, 4, "file read metadata")?
            else {
                return Ok(None);
            };
            let size = row[0]
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid file size value".to_string())?;
            for (field, value) in [
                ("access_date", &row[1]),
                ("modification_date", &row[2]),
                ("change_date", &row[3]),
            ] {
                if value.trim().is_empty() {
                    return Err(format!("file read metadata {field} is empty"));
                }
            }
            Ok(Some(FileReadMetadata {
                size,
                access_date: row[1].clone(),
                modification_date: row[2].clone(),
                change_date: row[3].clone(),
            }))
        })
    }

    pub fn load_block(
        &self,
        file_id: u64,
        block_index: u64,
        block_size: u64,
    ) -> Result<Option<Vec<u8>>, String> {
        let file_id_text = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let block_index_text = CString::new(block_index.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;
        let block_size = block_size.max(1) as usize;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id_text, &block_index_text];
            let res = exec_prepared_params_with_result_format(
                conn,
                PreparedStatement::LoadBlock,
                &params,
                1,
            )?;
            fetch_single_block_binary(res, block_size)
        })
    }

    /// Loads a block from the write authority for read-modify-write operations.
    /// Replica-routed reads are suitable for ordinary reads, but must not seed
    /// a mutation with potentially stale bytes.
    pub fn load_block_authoritative(
        &self,
        file_id: u64,
        block_index: u64,
        block_size: u64,
    ) -> Result<Option<Vec<u8>>, String> {
        let file_id_text = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let block_index_text = CString::new(block_index.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;
        let block_size = block_size.max(1) as usize;

        self.with_connection(super::ConnectionLane::Write, |conn| unsafe {
            let params = [&file_id_text, &block_index_text];
            let res = exec_prepared_params_with_result_format(
                conn,
                PreparedStatement::LoadBlock,
                &params,
                1,
            )?;
            fetch_single_block_binary(res, block_size)
        })
    }

    pub fn fetch_block_range_shared(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        block_size: u64,
    ) -> Result<SharedBlockRows, String> {
        if last_block < first_block {
            return Ok(Vec::new());
        }
        let file_id_text = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let first_block_text = CString::new(first_block.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;
        let last_block_text = CString::new(last_block.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;
        let block_size = block_size.max(1) as usize;

        self.with_read_connection(|conn| unsafe {
            let params = [&file_id_text, &first_block_text, &last_block_text];
            let res = exec_prepared_params_with_result_format(
                conn,
                PreparedStatement::FetchBlockRange,
                &params,
                1,
            )?;
            fetch_block_range_rows_shared(res, block_size)
        })
    }

    /// Loads block rows from the primary for operations that copy bytes into a
    /// mutation. Ordinary reads may use replicas, but copy input must not.
    pub fn fetch_block_range_authoritative_shared(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        block_size: u64,
    ) -> Result<SharedBlockRows, String> {
        if last_block < first_block {
            return Ok(Vec::new());
        }
        let file_id_text = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let first_block_text = CString::new(first_block.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;
        let last_block_text = CString::new(last_block.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;
        let block_size = block_size.max(1) as usize;

        self.with_connection(super::ConnectionLane::Write, |conn| unsafe {
            let params = [&file_id_text, &first_block_text, &last_block_text];
            let res = exec_prepared_params_with_result_format(
                conn,
                PreparedStatement::FetchBlockRange,
                &params,
                1,
            )?;
            fetch_block_range_rows_shared(res, block_size)
        })
    }

    /// Reads size, committed content generation and blocks from one MVCC
    /// statement snapshot. Requires schema v26; opt-in until all mounts upgrade.
    pub fn fetch_block_range_with_generation_shared(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        block_size: u64,
    ) -> Result<GeneratedSharedBlockRows, String> {
        let sql = CString::new(
            "WITH snapshot AS MATERIALIZED (
                 SELECT size, content_generation, data_object_id
                 FROM files WHERE id_file = $1
             )
             SELECT size::bigint, content_generation::bigint,
                    NULL::bigint, NULL::bytea FROM snapshot
             UNION ALL
             SELECT NULL::bigint, NULL::bigint, db._order::bigint, db.data
             FROM snapshot s JOIN data_blocks db ON db.data_object_id = s.data_object_id
             WHERE db._order BETWEEN $2 AND $3",
        )
        .map_err(|_| "invalid generation SQL".to_string())?;
        let file = CString::new(file_id.to_string()).map_err(|err| err.to_string())?;
        let first = CString::new(first_block.to_string()).map_err(|err| err.to_string())?;
        let last = CString::new(last_block.to_string()).map_err(|err| err.to_string())?;
        self.with_read_connection(|conn| unsafe {
            let params = [&file, &first, &last];
            let values = params.iter().map(|p| p.as_ptr()).collect::<Vec<_>>();
            let result = PQexecParams(
                conn,
                sql.as_ptr(),
                3,
                std::ptr::null(),
                values.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            );
            if result.is_null() {
                return Err(conn_error(conn));
            }
            decode_generation_block_snapshot(result, block_size.max(1) as usize)
        })
    }

    pub fn fetch_block_range_with_size_shared(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        block_size: u64,
    ) -> Result<SizedSharedBlockRows, String> {
        self.fetch_block_range_with_size_shared_on(
            file_id,
            first_block,
            last_block,
            block_size,
            false,
        )
    }

    /// Reads the file size and payload blocks from the write authority in one
    /// snapshot. Use this when a stale replica read would violate the caller's
    /// consistency contract.
    pub fn fetch_block_range_with_size_authoritative_shared(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        block_size: u64,
    ) -> Result<SizedSharedBlockRows, String> {
        self.fetch_block_range_with_size_shared_on(
            file_id,
            first_block,
            last_block,
            block_size,
            true,
        )
    }

    fn fetch_block_range_with_size_shared_on(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        block_size: u64,
        authoritative: bool,
    ) -> Result<SizedSharedBlockRows, String> {
        let file_id_text = CString::new(file_id.to_string())
            .map_err(|_| "file id contains NUL byte".to_string())?;
        let first_block_text = CString::new(first_block.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;
        let last_block_text = CString::new(last_block.to_string())
            .map_err(|_| "block index contains NUL byte".to_string())?;
        let block_size = block_size.max(1) as usize;
        let query = |conn| unsafe {
            let params = [&file_id_text, &first_block_text, &last_block_text];
            let res = exec_prepared_params_with_result_format(
                conn,
                PreparedStatement::FetchBlockRangeWithSize,
                &params,
                1,
            )?;
            fetch_block_range_with_size_rows_shared(res, block_size)
        };
        if authoritative {
            self.with_cached_connection(query)
        } else {
            self.with_read_connection(query)
        }
    }

    pub fn fetch_block_range(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        block_size: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, String> {
        let blocks = self.fetch_block_range_shared(file_id, first_block, last_block, block_size)?;
        Ok(blocks
            .into_iter()
            .map(|(index, data)| (index, data.as_ref().to_vec()))
            .collect())
    }

    pub fn assemble_file_slice(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        offset: u64,
        end_offset: u64,
        block_size: u64,
    ) -> Result<Vec<u8>, String> {
        let blocks = self.fetch_block_range(file_id, first_block, last_block, block_size)?;
        Ok(crate::assemble_read_slice(
            first_block,
            last_block,
            offset,
            end_offset,
            block_size,
            &blocks,
        ))
    }

    pub fn assemble_file_slice_authoritative(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        offset: u64,
        end_offset: u64,
        block_size: u64,
    ) -> Result<Vec<u8>, String> {
        let blocks = self.fetch_block_range_authoritative_shared(
            file_id,
            first_block,
            last_block,
            block_size,
        )?;
        Ok(crate::assemble_read_slice(
            first_block,
            last_block,
            offset,
            end_offset,
            block_size,
            &blocks,
        ))
    }
}
