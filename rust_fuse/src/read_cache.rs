// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use crate::fs::{FodFuse, FodFuseProfileCounters};
use libc::EIO;
use linked_hash_map::LinkedHashMap;
use log::warn;
use rust_hotpath::pg::DbRepo;
use rust_hotpath::read_missing_range_worker_count;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub(crate) struct ReadSequenceState {
    pub(crate) last_end: u64,
    pub(crate) streak: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadCacheEvictionPolicy {
    Fifo,
    Lru,
}

impl ReadCacheEvictionPolicy {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "fifo" => Self::Fifo,
            "lru" => Self::Lru,
            other => {
                warn!(
                    "unknown read cache eviction policy '{}' - falling back to fifo",
                    other
                );
                Self::Fifo
            }
        }
    }
}

#[derive(Debug, Clone)]
enum ReadCacheStore {
    Fifo {
        entries: HashMap<(u64, u64), Arc<[u8]>>,
        order: VecDeque<(u64, u64)>,
    },
    Lru {
        entries: LinkedHashMap<(u64, u64), Arc<[u8]>>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ReadBlockCache {
    store: ReadCacheStore,
}

impl Default for ReadBlockCache {
    fn default() -> Self {
        Self::new("fifo")
    }
}

impl ReadBlockCache {
    pub(crate) fn new(policy: impl AsRef<str>) -> Self {
        let policy = ReadCacheEvictionPolicy::parse(policy.as_ref());
        let store = match policy {
            ReadCacheEvictionPolicy::Fifo => ReadCacheStore::Fifo {
                entries: HashMap::new(),
                order: VecDeque::new(),
            },
            ReadCacheEvictionPolicy::Lru => ReadCacheStore::Lru {
                entries: LinkedHashMap::new(),
            },
        };
        Self { store }
    }

    pub(crate) fn len(&self) -> usize {
        match &self.store {
            ReadCacheStore::Fifo { entries, .. } => entries.len(),
            ReadCacheStore::Lru { entries } => entries.len(),
        }
    }

    fn get(&mut self, file_id: u64, block_index: u64) -> Option<Arc<[u8]>> {
        let key = (file_id, block_index);
        match &mut self.store {
            ReadCacheStore::Fifo { entries, .. } => entries.get(&key).cloned(),
            ReadCacheStore::Lru { entries } => entries.get_refresh(&key).cloned(),
        }
    }

    fn insert(&mut self, file_id: u64, block_index: u64, data: Arc<[u8]>, limit_blocks: usize) {
        let key = (file_id, block_index);
        match &mut self.store {
            ReadCacheStore::Fifo { entries, order } => {
                if !entries.contains_key(&key) {
                    order.push_back(key);
                }
                entries.insert(key, data);
                while entries.len() > limit_blocks {
                    if let Some(oldest) = order.pop_front() {
                        entries.remove(&oldest);
                    } else {
                        break;
                    }
                }
            }
            ReadCacheStore::Lru { entries } => {
                let existed = entries.contains_key(&key);
                entries.insert(key, data);
                if existed {
                    let _ = entries.get_refresh(&key);
                }
                while entries.len() > limit_blocks {
                    if entries.pop_front().is_none() {
                        break;
                    }
                }
            }
        }
    }

    fn clear_file(&mut self, file_id: u64) {
        match &mut self.store {
            ReadCacheStore::Fifo { entries, order } => {
                entries.retain(|(cached_file_id, _), _| *cached_file_id != file_id);
                order.retain(|(cached_file_id, _)| *cached_file_id != file_id);
            }
            ReadCacheStore::Lru { entries } => {
                let mut filtered = LinkedHashMap::new();
                for (key, value) in entries.iter() {
                    if key.0 != file_id {
                        filtered.insert(*key, Arc::clone(value));
                    }
                }
                *entries = filtered;
            }
        }
    }
}

impl FodFuse {
    fn recent_write_blocks_limit(&self, total_blocks: u64) -> usize {
        let cache_limit = self.read_cache_limit_blocks();
        let recent_limit = cache_limit.saturating_mul(16).max(256);
        let total_blocks = total_blocks.min(usize::MAX as u64) as usize;
        total_blocks.min(recent_limit)
    }

    pub(crate) fn read_cache_limit_blocks(&self) -> usize {
        self.reloadable_runtime()
            .read_cache_blocks
            .max(1)
            .min(usize::MAX as u64) as usize
    }

    pub(crate) fn read_workers(&self) -> usize {
        self.reloadable_runtime()
            .workers_read
            .max(1)
            .min(usize::MAX as u64) as usize
    }

    pub(crate) fn read_workers_min_blocks(&self) -> usize {
        self.reloadable_runtime()
            .workers_read_min_blocks
            .max(1)
            .min(usize::MAX as u64) as usize
    }

    pub(crate) fn read_sequence_state_for_file(
        &self,
        file_id: u64,
        offset: u64,
        end_offset: u64,
    ) -> (bool, u64) {
        let started = Instant::now();
        let result = self.read_sequence_state.lock();
        self.record_read_cache_lock_elapsed(started.elapsed());
        if let Ok(mut guard) = result {
            let previous = guard.get(&file_id).cloned();
            let sequential = previous
                .as_ref()
                .map(|value| value.last_end == offset)
                .unwrap_or(false);
            let streak = if sequential {
                previous
                    .map(|value| value.streak.saturating_add(1))
                    .unwrap_or(0)
            } else {
                0
            };
            guard.insert(
                file_id,
                ReadSequenceState {
                    last_end: end_offset,
                    streak,
                },
            );
            (sequential, streak)
        } else {
            (false, 0)
        }
    }

    pub(crate) fn recent_write_block(&self, file_id: u64, block_index: u64) -> Option<Arc<[u8]>> {
        // Overlay widocznosci miedzy fh.
        // Chroni przed sytuacja, gdzie drugi uchwyt buduje czesciowy blok
        // zanim kernel/FUSE/warstwa repo zwroci swiezo zapisany blok.
        let started = Instant::now();
        if self.recent_write_blocks_len.load(Ordering::Relaxed) == 0 {
            self.record_recent_write_block_elapsed(started.elapsed());
            return None;
        }
        let result = self.recent_write_blocks.lock();
        self.record_recent_write_blocks_lock_elapsed(started.elapsed());
        let block = result
            .ok()
            .and_then(|guard| guard.get(&(file_id, block_index)).cloned());
        self.record_recent_write_block_elapsed(started.elapsed());
        block
    }

    pub(crate) fn cached_read_block(&self, file_id: u64, block_index: u64) -> Option<Arc<[u8]>> {
        let started = Instant::now();
        if let Some(block) = self.recent_write_block(file_id, block_index) {
            self.record_cached_read_block_elapsed(started.elapsed());
            return Some(block);
        }
        let lock_started = Instant::now();
        let result = self.read_block_cache.lock();
        self.record_read_block_cache_lock_elapsed(lock_started.elapsed());
        let block = result
            .ok()
            .and_then(|mut guard| guard.get(file_id, block_index));
        self.record_cached_read_block_elapsed(started.elapsed());
        block
    }

    pub(crate) fn store_read_block(&self, file_id: u64, block_index: u64, data: Arc<[u8]>) {
        let started = Instant::now();
        let result = self.read_block_cache.lock();
        self.record_read_block_cache_lock_elapsed(started.elapsed());
        if let Ok(mut guard) = result {
            guard.insert(file_id, block_index, data, self.read_cache_limit_blocks());
        }
    }

    pub(crate) fn store_recent_write_blocks(
        &self,
        file_id: u64,
        file_size: u64,
        truncate_pending: bool,
        blocks: std::collections::BTreeMap<u64, Vec<u8>>,
    ) {
        let started = Instant::now();
        let block_size = self.block_size.max(1);
        let total_blocks = if file_size == 0 {
            0
        } else {
            1 + (file_size - 1) / block_size
        };
        let result = self.recent_write_blocks.lock();
        self.record_recent_write_blocks_lock_elapsed(started.elapsed());
        let Ok(mut guard) = result else {
            self.record_store_recent_write_blocks_elapsed(started.elapsed());
            return;
        };

        if truncate_pending || total_blocks == 0 {
            guard.retain(|(cached_file_id, cached_block_index), _| {
                *cached_file_id != file_id || *cached_block_index < total_blocks
            });
        }

        for (block_index, block) in blocks {
            if block_index < total_blocks {
                guard.insert((file_id, block_index), Arc::from(block));
            }
        }
        // Evict in deterministic key order so cache behavior is reproducible.
        let keep_limit = self.recent_write_blocks_limit(total_blocks);
        let over_limit = guard.len().saturating_sub(keep_limit);
        if over_limit > 0 {
            let mut keys = guard.keys().copied().collect::<Vec<_>>();
            keys.sort_unstable();
            for key in keys.into_iter().take(over_limit) {
                guard.remove(&key);
            }
        }
        self.recent_write_blocks_len
            .store(guard.len() as u64, Ordering::Relaxed);
        self.record_store_recent_write_blocks_elapsed(started.elapsed());
    }

    pub(crate) fn clear_read_cache_for_file(&self, file_id: u64) {
        let started = Instant::now();
        let result = self.read_block_cache.lock();
        self.record_read_block_cache_lock_elapsed(started.elapsed());
        if let Ok(mut guard) = result {
            guard.clear_file(file_id);
        }
        self.record_clear_read_cache_for_file_elapsed(started.elapsed());
    }

    fn missing_block_ranges(missing: &[u64]) -> Vec<(u64, u64)> {
        if missing.is_empty() {
            return Vec::new();
        }
        let mut ranges = Vec::new();
        let mut range_start = missing[0];
        let mut range_end = missing[0];
        for block_index in missing.iter().copied().skip(1) {
            if block_index == range_end + 1 {
                range_end = block_index;
            } else {
                ranges.push((range_start, range_end));
                range_start = block_index;
                range_end = block_index;
            }
        }
        ranges.push((range_start, range_end));
        ranges
    }

    fn merge_sorted_blocks(
        mut left: Vec<(u64, Arc<[u8]>)>,
        mut right: Vec<(u64, Arc<[u8]>)>,
    ) -> Vec<(u64, Arc<[u8]>)> {
        if left.is_empty() {
            return right;
        }
        if right.is_empty() {
            return left;
        }
        let mut merged = Vec::with_capacity(left.len().saturating_add(right.len()));
        let mut left_item = left.drain(..).peekable();
        let mut right_item = right.drain(..).peekable();

        loop {
            match (left_item.peek(), right_item.peek()) {
                (Some((left_index, _)), Some((right_index, _))) => {
                    if left_index <= right_index {
                        if let Some(item) = left_item.next() {
                            merged.push(item);
                        }
                    } else if let Some(item) = right_item.next() {
                        merged.push(item);
                    }
                }
                (Some(_), None) => {
                    merged.extend(left_item);
                    break;
                }
                (None, Some(_)) => {
                    merged.extend(right_item);
                    break;
                }
                (None, None) => break,
            }
        }

        merged
    }

    fn fetch_block_range_chunk(
        repo: &DbRepo,
        profile: &FodFuseProfileCounters,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        block_size: u64,
    ) -> Result<Vec<(u64, Arc<[u8]>)>, libc::c_int> {
        let started = Instant::now();
        if last_block < first_block {
            profile.record_fetch_block_range_chunk_elapsed(started.elapsed());
            return Ok(Vec::new());
        }
        let result = repo.fetch_block_range_shared(file_id, first_block, last_block, block_size);
        profile.record_repo_fetch_block_range_elapsed(started.elapsed());
        let mapped = match result {
            Ok(rows) => Ok(rows),
            Err(err) => {
                warn!(
                    "FOD read block range fetch failed file_id={} first_block={} last_block={} err={}",
                    file_id,
                    first_block,
                    last_block,
                    err
                );
                Err(EIO)
            }
        };
        profile.record_fetch_block_range_chunk_elapsed(started.elapsed());
        mapped
    }

    fn fetch_block_range_parallel(
        profile: Arc<FodFuseProfileCounters>,
        repo: DbRepo,
        file_id: u64,
        ranges: Vec<(u64, u64)>,
        block_size: u64,
        workers: usize,
    ) -> Result<Vec<(u64, Arc<[u8]>)>, libc::c_int> {
        let started = Instant::now();
        if ranges.is_empty() {
            profile.record_fetch_block_range_parallel_elapsed(started.elapsed());
            return Ok(Vec::new());
        }
        let worker_count = workers.max(1).min(ranges.len()).max(1);
        let queue = Arc::new(Mutex::new(VecDeque::from(ranges)));
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let repo = repo.clone();
            let profile = Arc::clone(&profile);
            let queue = Arc::clone(&queue);
            handles.push(thread::spawn(
                move || -> Result<Vec<(u64, Arc<[u8]>)>, libc::c_int> {
                    let mut collected = Vec::new();
                    loop {
                        let next = {
                            let mut guard = queue.lock().map_err(|_| EIO)?;
                            guard.pop_front()
                        };
                        let Some((first_block, last_block)) = next else {
                            break;
                        };
                        let mut chunk = Self::fetch_block_range_chunk(
                            &repo,
                            profile.as_ref(),
                            file_id,
                            first_block,
                            last_block,
                            block_size,
                        )?;
                        collected.extend(chunk.drain(..));
                    }
                    Ok(collected)
                },
            ));
        }
        let mut blocks = Vec::new();
        for handle in handles {
            let collected = handle.join().map_err(|_| EIO)??;
            blocks.extend(collected);
        }
        blocks.sort_unstable_by_key(|(block_index, _)| *block_index);
        profile.record_fetch_block_range_parallel_elapsed(started.elapsed());
        Ok(blocks)
    }

    pub(crate) fn read_block_map(
        &self,
        file_id: u64,
        fetch_first: u64,
        fetch_last: u64,
    ) -> Result<Vec<(u64, Arc<[u8]>)>, libc::c_int> {
        let started = Instant::now();
        if fetch_last < fetch_first {
            self.record_read_block_map_elapsed(started.elapsed());
            return Ok(Vec::new());
        }
        let total_blocks = fetch_last.saturating_sub(fetch_first).saturating_add(1);
        let mut cached = Vec::with_capacity(total_blocks.min(usize::MAX as u64) as usize);
        let mut missing = Vec::new();
        for block_index in fetch_first..=fetch_last {
            if let Some(block) = self.cached_read_block(file_id, block_index) {
                cached.push((block_index, block));
            } else {
                missing.push(block_index);
            }
        }
        if missing.is_empty() {
            self.record_read_block_map_elapsed(started.elapsed());
            return Ok(cached);
        }
        let contiguous_ranges = Self::missing_block_ranges(&missing);
        let workers = read_missing_range_worker_count(
            self.read_workers() as u64,
            self.read_workers_min_blocks() as u64,
            missing.len() as u64,
            contiguous_ranges.len() as u64,
        ) as usize;
        let fetched = if workers <= 1 {
            let first = *missing.first().unwrap_or(&fetch_first);
            let last = *missing.last().unwrap_or(&fetch_last);
            self.fetch_block_range_profiled(file_id, first, last, self.block_size)
                .map_err(|_| EIO)?
        } else {
            Self::fetch_block_range_parallel(
                self.profile_counters(),
                self.repo.clone(),
                file_id,
                contiguous_ranges,
                self.block_size,
                workers,
            )?
        };
        for (block_index, block) in fetched.iter() {
            self.store_read_block(file_id, *block_index, Arc::clone(block));
        }
        cached = Self::merge_sorted_blocks(cached, fetched);
        self.record_read_block_map_elapsed(started.elapsed());
        Ok(cached)
    }
}

#[cfg(test)]
mod tests {
    use super::ReadBlockCache;
    use std::sync::Arc;

    fn block(value: u8) -> Arc<[u8]> {
        Arc::from(vec![value])
    }

    #[test]
    fn fifo_cache_evicts_in_insertion_order() {
        let mut cache = ReadBlockCache::new("fifo");
        cache.insert(1, 1, block(1), 2);
        cache.insert(1, 2, block(2), 2);
        assert!(cache.get(1, 1).is_some());
        cache.insert(1, 3, block(3), 2);
        assert!(cache.get(1, 1).is_none());
        assert!(cache.get(1, 2).is_some());
        assert!(cache.get(1, 3).is_some());
    }

    #[test]
    fn lru_cache_refreshes_recent_hits() {
        let mut cache = ReadBlockCache::new("lru");
        cache.insert(1, 1, block(1), 2);
        cache.insert(1, 2, block(2), 2);
        assert!(cache.get(1, 1).is_some());
        cache.insert(1, 3, block(3), 2);
        assert!(cache.get(1, 1).is_some());
        assert!(cache.get(1, 2).is_none());
        assert!(cache.get(1, 3).is_some());
    }
}
