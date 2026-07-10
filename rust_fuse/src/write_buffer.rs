// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use crate::fs::FodFuse;
use libc::EIO;
use log::{debug, warn};
use rust_hotpath::pg::{PersistBlockRow, PersistExtentRow};
use rust_hotpath::{
    choose_persist_execution_plan, dirty_block_size, PersistBlockPlanEntry, PersistExecutionPlan,
    PersistPayloadPlan, PersistPlanInput,
};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Debug, Clone)]
pub(crate) struct WriteState {
    pub(crate) file_id: u64,
    pub(crate) file_size: u64,
    pub(crate) truncate_pending: bool,
    pub(crate) buffered_bytes: u64,
    pub(crate) load_error: bool,
    pub(crate) blocks: BTreeMap<u64, Vec<u8>>,
}

impl FodFuse {
    pub(crate) fn new_write_state(
        file_id: u64,
        file_size: u64,
        truncate_pending: bool,
    ) -> WriteState {
        WriteState {
            file_id,
            file_size,
            truncate_pending,
            buffered_bytes: 0,
            load_error: false,
            blocks: BTreeMap::new(),
        }
    }

    fn load_write_block(
        &self,
        state: &mut WriteState,
        block_index: u64,
    ) -> Result<Vec<u8>, libc::c_int> {
        if let Some(block) = state.blocks.get(&block_index) {
            return Ok(block.clone());
        }
        if let Some(block) = self.recent_write_block(state.file_id, block_index) {
            return Ok(block.as_ref().to_vec());
        }
        Self::load_write_block_from_repo(
            state.file_id,
            block_index,
            self.block_size as usize,
            self.load_block_profiled(state.file_id, block_index, self.block_size),
        )
    }

    fn load_write_block_from_repo(
        file_id: u64,
        block_index: u64,
        block_size: usize,
        load_result: Result<Option<Vec<u8>>, String>,
    ) -> Result<Vec<u8>, libc::c_int> {
        match load_result {
            Ok(Some(block)) => Ok(block),
            Ok(None) => Ok(vec![0u8; block_size]),
            Err(err) => {
                warn!(
                    "FOD load_write_block failed file_id={} block_index={} err={}",
                    file_id, block_index, err
                );
                Err(EIO)
            }
        }
    }

    pub(crate) fn update_write_buffer(
        &self,
        state: &mut WriteState,
        offset: u64,
        data: &[u8],
    ) -> Result<(), libc::c_int> {
        let started = Instant::now();
        let block_size = self.block_size.max(1);
        let block_size_usize = block_size as usize;

        if data.is_empty() {
            self.record_update_write_buffer_elapsed(started.elapsed());
            return Ok(());
        }

        // File size before this write.
        // Blocks starting past the previous EOF are new and do not need a DB read.
        let original_file_size = state.file_size;

        let end = offset.saturating_add(data.len() as u64);
        if end > state.file_size {
            state.file_size = end;
        }

        let result = (|| -> Result<(), libc::c_int> {
            let first_block = offset / block_size;
            let last_block = (end.saturating_sub(1)) / block_size;
            let mut src_cursor = 0usize;

            for block_index in first_block..=last_block {
                let block_start = block_index * block_size;
                let block_end = block_start.saturating_add(block_size);
                let write_start = offset.max(block_start);
                let write_end = end.min(block_end);

                if write_end <= write_start {
                    continue;
                }

                let block_slice_start = (write_start - block_start) as usize;
                let block_slice_end = (write_end - block_start) as usize;
                let src_len = block_slice_end.saturating_sub(block_slice_start);
                let src_end = src_cursor.saturating_add(src_len);

                if src_end > data.len() {
                    break;
                }

                let full_block_write =
                    block_slice_start == 0 && block_slice_end == block_size_usize;
                let brand_new_block = block_start >= original_file_size;

                let mut block = if full_block_write {
                    // Full overwrites do not need to clone the previous block.
                    data[src_cursor..src_end].to_vec()
                } else if let Some(existing_block) = state.blocks.get(&block_index) {
                    // Already buffered data has priority.
                    existing_block.clone()
                } else if brand_new_block && block_slice_start == 0 {
                    // A brand-new block written from the start does not need a DB read.
                    vec![0u8; block_size_usize]
                } else {
                    // Partial writes to existing blocks must load the current DB state.
                    self.load_write_block(state, block_index)?
                };

                if block.len() < block_size_usize {
                    block.resize(block_size_usize, 0);
                }

                // Copy the user data into the prepared block buffer.
                // This preserves the expected partial-write semantics.
                block[block_slice_start..block_slice_end]
                    .copy_from_slice(&data[src_cursor..src_end]);

                state.blocks.insert(block_index, block);
                src_cursor = src_end;
            }

            Ok(())
        })();

        if result.is_ok() {
            self.clear_read_cache_for_file(state.file_id);
        }
        self.record_update_write_buffer_elapsed(started.elapsed());
        result
    }

    pub(crate) fn flush_write_state(&self, state: &mut WriteState) -> Result<(), libc::c_int> {
        let started = Instant::now();
        if state.load_error {
            self.record_flush_write_state_elapsed(started.elapsed());
            return Err(EIO);
        }
        let block_size = self.block_size.max(1);
        let dirty_blocks = state.blocks.keys().copied().collect::<Vec<_>>();
        let persist_plan = choose_persist_execution_plan(PersistPlanInput {
            enable_extents: self.enable_extents,
            extent_target_bytes: self.extent_target_bytes,
            file_size: state.file_size,
            block_size,
            truncate_pending: state.truncate_pending,
            dirty_blocks: &dirty_blocks,
        });
        if let Err(errno) = self.execute_persist_plan(state, block_size, persist_plan) {
            self.record_flush_write_state_elapsed(started.elapsed());
            return Err(errno);
        }
        let flushed_blocks = std::mem::take(&mut state.blocks);
        self.store_recent_write_blocks(
            state.file_id,
            state.file_size,
            state.truncate_pending,
            flushed_blocks,
        );
        self.maybe_touch_client_session_write();
        self.clear_read_cache_for_file(state.file_id);
        self.invalidate_statfs_cache();
        state.truncate_pending = false;
        state.buffered_bytes = 0;
        state.load_error = false;
        self.record_flush_write_state_elapsed(started.elapsed());
        Ok(())
    }

    fn persist_row_for_block<'a>(
        &self,
        state: &'a WriteState,
        block_index: u64,
        used_len: u64,
    ) -> Option<PersistBlockRow<'a>> {
        if used_len == 0 {
            return None;
        }
        let block = state.blocks.get(&block_index)?;
        Some(PersistBlockRow {
            block_index,
            data: block.as_slice(),
            used_len,
        })
    }

    fn prepare_persist_rows_from_block_plan<'a>(
        &self,
        state: &'a WriteState,
        plan: &[PersistBlockPlanEntry],
    ) -> Vec<PersistBlockRow<'a>> {
        let started = Instant::now();
        let rows = plan
            .iter()
            .filter_map(|entry| {
                self.persist_row_for_block(state, entry.block_index, entry.used_len)
            })
            .collect::<Vec<_>>();
        self.record_prepare_persist_rows_from_block_plan_elapsed(started.elapsed());
        rows
    }

    fn prepare_persist_extent_rows_from_extent_ranges(
        &self,
        state: &WriteState,
        block_size: u64,
        extents: &[(u64, u64)],
    ) -> Vec<PersistExtentRow> {
        let started = Instant::now();
        let rows = extents
            .iter()
            .filter_map(|(start_block, end_block)| {
                let mut used_bytes = 0u64;
                let mut segments = Vec::new();
                for block_index in *start_block..=*end_block {
                    let Some(block) = state.blocks.get(&block_index) else {
                        return None;
                    };
                    let used_len = dirty_block_size(state.file_size, block_index, block_size);
                    if used_len == 0 {
                        continue;
                    }
                    let used_len = used_len as usize;
                    used_bytes = used_bytes.saturating_add(used_len as u64);
                    segments.push((block_index, used_len, block.as_slice()));
                }

                if used_bytes == 0 {
                    return None;
                }

                let mut payload = Vec::with_capacity(used_bytes as usize);
                for (_block_index, used_len, block) in segments {
                    payload.extend_from_slice(&block[..used_len]);
                }

                Some(PersistExtentRow {
                    start_block: *start_block,
                    block_count: end_block.saturating_sub(*start_block).saturating_add(1),
                    used_bytes,
                    payload,
                })
            })
            .collect::<Vec<_>>();
        self.record_prepare_persist_extent_rows_from_extent_ranges_elapsed(started.elapsed());
        rows
    }

    fn execute_persist_plan(
        &self,
        state: &WriteState,
        block_size: u64,
        execution_plan: PersistExecutionPlan,
    ) -> Result<(), libc::c_int> {
        let live = self.reloadable_runtime();
        match execution_plan.payload {
            PersistPayloadPlan::Blocks(blocks) => {
                let rows = self.prepare_persist_rows_from_block_plan(state, &blocks);
                self.persist_file_blocks_profiled(
                    state.file_id,
                    state.file_size,
                    block_size,
                    execution_plan.total_blocks,
                    state.truncate_pending,
                    &rows,
                    live.copy_dedupe_crc_table,
                )
                .map_err(|_| EIO)
            }
            PersistPayloadPlan::Extents(extents) => {
                let rows = self.prepare_persist_extent_rows_from_extent_ranges(
                    state,
                    block_size,
                    &extents.into_ranges(),
                );
                debug!(
                    "FOD extent PoC execution file_id={} extent_rows={:?}",
                    state.file_id,
                    rows.iter()
                        .map(|row| (row.start_block, row.block_count, row.used_bytes))
                        .collect::<Vec<_>>()
                );
                self.persist_file_extents_profiled(
                    state.file_id,
                    state.file_size,
                    block_size,
                    execution_plan.total_blocks,
                    state.truncate_pending,
                    &rows,
                    live.copy_dedupe_crc_table,
                )
                .map_err(|_| EIO)
            }
        }
    }

    pub(crate) fn read_from_write_state(
        &self,
        state: &mut WriteState,
        offset: u64,
        size: u64,
    ) -> Result<Vec<u8>, libc::c_int> {
        let block_size = self.block_size.max(1);
        if offset >= state.file_size {
            return Ok(Vec::new());
        }
        let end_offset = offset.saturating_add(size).min(state.file_size);
        let mut output = vec![0u8; (end_offset - offset) as usize];
        let first_block = offset / block_size;
        let last_block = (end_offset.saturating_sub(1)) / block_size;
        for block_index in first_block..=last_block {
            let block_start = block_index * block_size;
            let block_end = block_start.saturating_add(block_size);
            let read_start = offset.max(block_start);
            let read_end = end_offset.min(block_end);
            if read_end <= read_start {
                continue;
            }
            let block = self.load_write_block(state, block_index)?;
            let block_slice_start = (read_start - block_start) as usize;
            let block_slice_end = (read_end - block_start) as usize;
            let out_start = (read_start - offset) as usize;
            let out_end = out_start + block_slice_end - block_slice_start;
            output[out_start..out_end].copy_from_slice(&block[block_slice_start..block_slice_end]);
        }
        Ok(output)
    }

    pub(crate) fn read_copy_destination_slice(
        &self,
        dst_file_id: u64,
        state: Option<&mut WriteState>,
        dst_first_block: u64,
        dst_last_block: u64,
        dst_offset: u64,
        size: u64,
        current_size: u64,
    ) -> Result<Vec<u8>, libc::c_int> {
        if size == 0 {
            return Ok(Vec::new());
        }

        let mut current = match state {
            Some(state) => self.read_from_write_state(state, dst_offset, size)?,
            None if dst_offset >= current_size => Vec::new(),
            None => {
                let end_offset = dst_offset.saturating_add(size).min(current_size);
                self.assemble_file_slice_profiled(
                    dst_file_id,
                    dst_first_block,
                    dst_last_block,
                    dst_offset,
                    end_offset,
                    self.block_size,
                )
                .map_err(|_| EIO)?
            }
        };

        if current.len() < size as usize {
            current.resize(size as usize, 0);
        }

        Ok(current)
    }

    pub(crate) fn write_state_for_handle(&self, fh: u64) -> Option<WriteState> {
        let started = Instant::now();
        let result = self.write_states.lock();
        self.record_write_state_lock_elapsed(started.elapsed());
        result.ok().and_then(|guard| {
            guard
                .get(&fh)
                .map(|state| self.clone_write_state_profiled(state))
        })
    }

    pub(crate) fn take_write_state_for_handle(&self, fh: u64) -> Option<WriteState> {
        // Wyjmij stan zapisu bez klonowania calego bufora.
        // To jest wazne dla malych zapisow 4K, bo klonowanie narastajacego
        // BTreeMap<u64, Vec<u8>> przy kazdym write() robi koszt O(n^2).
        let started = Instant::now();
        let result = self.write_states.lock();
        self.record_write_state_lock_elapsed(started.elapsed());
        result.ok().and_then(|mut guard| guard.remove(&fh))
    }

    pub(crate) fn write_state_has_pending_changes(state: &WriteState) -> bool {
        state.truncate_pending
            || state.buffered_bytes > 0
            || state.load_error
            || !state.blocks.is_empty()
    }

    pub(crate) fn write_flush_threshold_reached(&self, buffered_bytes: u64) -> bool {
        self.write_flush_threshold_bytes > 0 && buffered_bytes >= self.write_flush_threshold_bytes
    }

    pub(crate) fn should_flush_write_state(
        &self,
        buffered_bytes: u64,
        shared_open_handles: usize,
        partial_block_visibility_write: bool,
    ) -> bool {
        self.write_flush_threshold_reached(buffered_bytes)
            || shared_open_handles > 1
            || partial_block_visibility_write
    }

    pub(crate) fn update_write_state(&self, fh: u64, state: WriteState) {
        let started = Instant::now();
        let result = self.write_states.lock();
        self.record_write_state_lock_elapsed(started.elapsed());
        if let Ok(mut guard) = result {
            guard.insert(fh, state);
        }
    }

    pub(crate) fn flush_pending_write_states_for_file_except(
        &self,
        file_id: u64,
        except_fh: u64,
    ) -> Result<(), libc::c_int> {
        // FOD buforuje zapisy per fh. Przy wielu otwarciach tego samego pliku
        // drugi fh musi widziec zapis pierwszego fh przed czesciowym nadpisaniem bloku.
        // W przeciwnym razie partial write laduje stary blok z DB i moze wyzerowac
        // bajty zapisane chwile wczesniej przez inny uchwyt.
        let pending = {
            let started = Instant::now();
            let result = self.write_states.lock();
            self.record_write_state_lock_elapsed(started.elapsed());
            let guard = result.map_err(|_| EIO)?;
            guard
                .iter()
                .filter(|(fh, state)| {
                    **fh != except_fh
                        && state.file_id == file_id
                        && Self::write_state_has_pending_changes(state)
                })
                .map(|(fh, state)| (*fh, self.clone_write_state_profiled(state)))
                .collect::<Vec<_>>()
        };

        for (fh, mut state) in pending {
            self.flush_write_state(&mut state)?;
            if Self::write_state_has_pending_changes(&state) {
                self.update_write_state(fh, state);
            } else {
                let started = Instant::now();
                let result = self.write_states.lock();
                self.record_write_state_lock_elapsed(started.elapsed());
                if let Ok(mut guard) = result {
                    guard.remove(&fh);
                }
            }
        }

        Ok(())
    }

    pub(crate) fn drain_pending_write_states_for_file_except(
        &self,
        file_id: u64,
        except_fh: u64,
    ) -> Result<Vec<(u64, WriteState)>, libc::c_int> {
        // Dla write() lepiej scalic bufory w pamieci niz przepychac je przez DB.
        // Dzieki temu drugi uchwyt widzi czesciowy blok zapisany przez pierwszy uchwyt.
        let started = Instant::now();
        let result = self.write_states.lock();
        self.record_write_state_lock_elapsed(started.elapsed());
        let mut guard = result.map_err(|_| EIO)?;
        let handles = guard
            .iter()
            .filter(|(fh, state)| {
                **fh != except_fh
                    && state.file_id == file_id
                    && Self::write_state_has_pending_changes(state)
            })
            .map(|(fh, _)| *fh)
            .collect::<Vec<_>>();

        Ok(handles
            .into_iter()
            .filter_map(|fh| guard.remove(&fh).map(|state| (fh, state)))
            .collect::<Vec<_>>())
    }

    pub(crate) fn merge_write_state_into(
        target: &mut WriteState,
        source: WriteState,
        block_size: u64,
    ) {
        // Zachowujemy efekt wczesniejszych zapisow z innych fh.
        // Potem aktualny write() naklada swoje dane na zmergowany blok.
        let block_size = block_size.max(1);

        if source.truncate_pending {
            target.truncate_pending = true;
            target.file_size = source.file_size;
            target
                .blocks
                .retain(|block_index, _| block_index.saturating_mul(block_size) < target.file_size);
        } else if source.file_size > target.file_size {
            target.file_size = source.file_size;
        }

        target.buffered_bytes = target.buffered_bytes.saturating_add(source.buffered_bytes);
        target.load_error |= source.load_error;

        for (block_index, block) in source.blocks {
            target.blocks.insert(block_index, block);
        }
    }

    pub(crate) fn remove_write_state(&self, fh: u64) {
        let started = Instant::now();
        let result = self.write_states.lock();
        self.record_write_state_lock_elapsed(started.elapsed());
        if let Ok(mut guard) = result {
            guard.remove(&fh);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_write_block_from_repo_returns_zero_block_for_missing_row() {
        let block = FodFuse::load_write_block_from_repo(7, 2, 4, Ok(None))
            .expect("missing row should produce a zero block");
        assert_eq!(block, vec![0, 0, 0, 0]);
    }

    #[test]
    fn load_write_block_from_repo_returns_eio_on_db_error() {
        let err = FodFuse::load_write_block_from_repo(7, 2, 4, Err("boom".to_string()))
            .expect_err("database error should not be masked as zeroes");
        assert_eq!(err, EIO);
    }
}
