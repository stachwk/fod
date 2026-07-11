// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use chrono::{DateTime, NaiveDateTime, Utc};
use fuser::consts::{FUSE_FLOCK_LOCKS, FUSE_POSIX_LOCKS};
use fuser::{
    FileAttr, FileType, Filesystem, KernelConfig, ReplyAttr, ReplyBmap, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyIoctl, ReplyLock, ReplyOpen, ReplyPoll,
    ReplyStatfs, ReplyWrite, ReplyXattr, Request, TimeOrNow,
};
use libc::{EIO, ENOENT, ENOTEMPTY, ENOTTY, POLLIN, POLLOUT};
use log::{debug, info, warn};
use rust_hotpath::assemble_read_slice;
use rust_hotpath::pg::{DbRepo, PersistBlockRow, PersistExtentRow};
use std::collections::{HashMap, HashSet};
use std::convert::TryInto;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fod_rust_runtime::{
    duration_to_micros, reloadable_snapshot_from_json, AtimeStat, RuntimeConfig,
    RuntimeReloadableSettings, FOD_SCHEMA_NAME,
};
pub use fod_rust_runtime::{AtimePolicy, LockBackend};

use crate::copy_plan::{copy_range_bounds, pack_copy_skip_unchanged_runs, CopyRangeBounds};
use crate::read_cache::{ReadBlockCache, ReadSequenceState};
use crate::startup::FodFuseSettings;
pub(crate) use crate::write_buffer::WriteState;

const ROOT_INO: u64 = 1;
const REQUEST_PID_GROUPS_CACHE_TTL: Duration = Duration::from_millis(500);
// `last_write_at` is diagnostic only, so we update it at most every few seconds.
const CLIENT_WRITE_TOUCH_INTERVAL: Duration = Duration::from_secs(5);
// Default atime updates are throttled so repeated reads do not hammer PostgreSQL.
const ATIME_TOUCH_INTERVAL: Duration = Duration::from_secs(5);
// Linux FIGETBSZ is _IO(0x00, 2) and reports the filesystem block size.
const IOCTL_FIGETBSZ: u32 = 2;
// linux/fs.h also exposes the generic XFS-style fsxattr ioctls.
const IOCTL_FS_IOC_FSGETXATTR: u32 = libc::_IOR::<[u8; IOCTL_FSXATTR_BYTES]>('X' as u32, 31) as u32;
const IOCTL_FS_IOC_FSSETXATTR: u32 = libc::_IOW::<[u8; IOCTL_FSXATTR_BYTES]>('X' as u32, 32) as u32;
const IOCTL_FSXATTR_BYTES: usize = 28;

// FOPEN_DIRECT_IO = 1 << 0.
// Uzywamy lokalnej stalej, zeby nie zalezec od eksportu tej stalej przez wersje fuser.
// Bez direct_io kernel moze buforowac zapisy miedzy uchwytami i test multi_open
// widzi potem zera zamiast danych zapisanych przez pierwszy fh.
const FOD_FOPEN_DIRECT_IO: u32 = 1;

#[derive(Debug, Clone)]
struct ParsedAttrs {
    file_attr: FileAttr,
}

#[derive(Debug, Clone)]
struct StatfsSnapshot {
    files: u64,
    dirs: u64,
    total_data_size: u64,
    blocks: u64,
    loaded_at: SystemTime,
}

#[derive(Debug, Clone)]
struct PosixLockRecord {
    owner: u64,
    typ: i32,
    start: u64,
    end: Option<u64>,
    pid: u32,
}

#[derive(Debug, Clone)]
struct SubjectIdentity {
    uid: u32,
    gid: u32,
    groups: HashSet<u32>,
}

impl SubjectIdentity {
    fn is_root(&self) -> bool {
        self.uid == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
// Pair PID with /proc/<pid>/stat starttime so a reused PID cannot hit a stale cache entry.
struct RequestPidGroupsCacheKey {
    pid: u32,
    starttime: u64,
}

#[derive(Debug, Clone)]
struct CachedPidGroups {
    groups: HashSet<u32>,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct FileHandleState {
    path: String,
    file_id: Option<u64>,
    flags: i32,
    atime_touched: bool,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct FileCloneRangeIoctl {
    src_fd: i64,
    src_offset: u64,
    src_length: u64,
    dest_offset: u64,
}

struct LockHeartbeatHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

const RUNTIME_RELOAD_POLL_INTERVAL: Duration = Duration::from_secs(2);

struct RuntimeReloadHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LockHeartbeatHandle {
    fn spawn(
        repo: DbRepo,
        session_id: u64,
        posix_locks: Arc<Mutex<HashMap<String, Vec<PosixLockRecord>>>>,
        local_lock_owners: Arc<Mutex<HashSet<u64>>>,
        interval: Duration,
        lease_ttl: Duration,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let lease_ttl_seconds = lease_ttl.as_secs_f64().ceil().max(1.0) as u64;
        let thread = thread::Builder::new()
            .name("fod-lock-heartbeat".to_string())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    thread::park_timeout(interval);
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    heartbeat_pg_mount_state(
                        &repo,
                        session_id,
                        &posix_locks,
                        &local_lock_owners,
                        lease_ttl_seconds,
                    );
                }
            })
            .map_err(|err| format!("failed to spawn lock heartbeat thread: {err}"))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

impl Drop for LockHeartbeatHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

impl RuntimeReloadHandle {
    fn spawn(
        repo: DbRepo,
        base_runtime: RuntimeConfig,
        live_runtime: Arc<RwLock<RuntimeReloadableSettings>>,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("fod-runtime-reload".to_string())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    thread::park_timeout(RUNTIME_RELOAD_POLL_INTERVAL);
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    match runtime_override_snapshot(&repo) {
                        Ok(snapshot) => match base_runtime.with_reloadable_overrides(&snapshot) {
                            Ok(updated_runtime) => {
                                let updated_settings = updated_runtime.reloadable_settings();
                                apply_reloadable_runtime(&live_runtime, updated_settings);
                            }
                            Err(err) => warn!("FOD runtime reload rejected snapshot: {}", err),
                        },
                        Err(err) => warn!("FOD runtime reload poll failed: {}", err),
                    }
                }
            })
            .map_err(|err| format!("failed to spawn runtime reload thread: {err}"))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

impl Drop for RuntimeReloadHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn heartbeat_pg_mount_state(
    repo: &DbRepo,
    session_id: u64,
    posix_locks: &Arc<Mutex<HashMap<String, Vec<PosixLockRecord>>>>,
    local_lock_owners: &Arc<Mutex<HashSet<u64>>>,
    lease_ttl_seconds: u64,
) {
    let local_owners = match local_lock_owners.lock() {
        Ok(guard) => guard.iter().copied().collect::<HashSet<_>>(),
        Err(err) => {
            warn!("FOD lock heartbeat skipped: poisoned owner set: {}", err);
            return;
        }
    };

    if let Err(err) = repo.heartbeat_client_session(session_id, lease_ttl_seconds) {
        warn!(
            "FOD session heartbeat failed session_id={} err={}",
            session_id, err
        );
        return;
    }

    for owner in local_owners.iter().copied() {
        if let Err(err) = repo.touch_client_session_owner_key(session_id, owner) {
            warn!(
                "FOD session owner heartbeat failed session_id={} owner={} err={}",
                session_id, owner, err
            );
        }
    }

    let snapshot = match posix_locks.lock() {
        Ok(guard) => guard
            .iter()
            .map(|(resource_key, records)| (resource_key.clone(), records.clone()))
            .collect::<Vec<_>>(),
        Err(err) => {
            warn!("FOD lock heartbeat skipped: poisoned lock map: {}", err);
            return;
        }
    };

    for (resource_key, records) in snapshot {
        let (resource_kind, resource_id) = match FodFuse::lock_resource_kind_id(&resource_key) {
            Ok(value) => value,
            Err(errno) => {
                warn!(
                    "FOD lock heartbeat skipped resource_key={} errno={}",
                    resource_key, errno
                );
                continue;
            }
        };
        for record in records {
            if !local_owners.contains(&record.owner) {
                continue;
            }
            if let Err(err) = repo.heartbeat_lock_range_lease(
                &resource_kind,
                resource_id,
                record.owner,
                record.start,
                record.end,
                lease_ttl_seconds,
            ) {
                warn!(
                    "FOD lock heartbeat failed resource_key={} owner={} start={} end={:?} err={}",
                    resource_key, record.owner, record.start, record.end, err
                );
            }
        }
    }

    match repo.prune_expired_client_sessions() {
        Ok(true) => {}
        Ok(false) => {}
        Err(err) => warn!(
            "FOD session prune failed session_id={} err={}",
            session_id, err
        ),
    }
}

fn runtime_override_snapshot(repo: &DbRepo) -> Result<HashMap<String, String>, String> {
    let table_exists = repo.query_scalar_text(&format!(
        "SELECT to_regclass('{}.runtime_overrides') IS NOT NULL",
        FOD_SCHEMA_NAME
    ))?;
    if !matches!(
        table_exists.trim().to_ascii_lowercase().as_str(),
        "t" | "true" | "1" | "on"
    ) {
        return Ok(HashMap::new());
    }
    let payload = repo.query_scalar_text(&format!(
        "SELECT payload_json FROM {}.runtime_overrides WHERE id = 1",
        FOD_SCHEMA_NAME
    ))?;
    if payload.trim().is_empty() {
        return Ok(HashMap::new());
    }
    reloadable_snapshot_from_json(&payload)
}

fn apply_reloadable_runtime(
    live_runtime: &Arc<RwLock<RuntimeReloadableSettings>>,
    settings: RuntimeReloadableSettings,
) -> bool {
    let changed = match live_runtime.write() {
        Ok(mut guard) => {
            if *guard == settings {
                false
            } else {
                *guard = settings.clone();
                true
            }
        }
        Err(err) => {
            let mut guard = err.into_inner();
            if *guard == settings {
                false
            } else {
                *guard = settings.clone();
                true
            }
        }
    };
    if changed {
        log::set_max_level(
            settings
                .log_level
                .parse::<log::LevelFilter>()
                .unwrap_or(log::LevelFilter::Info),
        );
        info!(
            "FOD runtime reload applied profile={:?} log_level={} metadata_cache_ttl={}s statfs_cache_ttl={}s read_cache_blocks={} read_ahead_blocks={} sequential_read_ahead_blocks={} small_file_read_threshold_blocks={} workers_read={} workers_read_min_blocks={} workers_write={} workers_write_min_blocks={} persist_buffer_chunk_blocks={} copy_dedupe_enabled={} copy_dedupe_min_blocks={} copy_dedupe_max_blocks={} copy_dedupe_crc_table={}",
            settings.profile,
            settings.log_level,
            settings.metadata_cache_ttl.as_secs(),
            settings.statfs_cache_ttl.as_secs(),
            settings.read_cache_blocks,
            settings.read_ahead_blocks,
            settings.sequential_read_ahead_blocks,
            settings.small_file_read_threshold_blocks,
            settings.workers_read,
            settings.workers_read_min_blocks,
            settings.workers_write,
            settings.workers_write_min_blocks,
            settings.persist_buffer_chunk_blocks,
            settings.copy_dedupe_enabled,
            settings.copy_dedupe_min_blocks,
            settings.copy_dedupe_max_blocks,
            settings.copy_dedupe_crc_table
        );
    }
    changed
}

#[derive(Debug, Clone)]
struct PosixAclEntry {
    tag: u16,
    perm: u16,
    id: i32,
}

struct FileAttrTime<'a>(&'a FileAttr);

impl AtimeStat for FileAttrTime<'_> {
    fn atime(&self) -> SystemTime {
        self.0.atime
    }

    fn mtime(&self) -> SystemTime {
        self.0.mtime
    }
}

fn should_update_atime(policy: AtimePolicy, is_dir: bool, attrs: &FileAttr) -> bool {
    policy.should_update(is_dir, &FileAttrTime(attrs))
}

#[derive(Debug, Default)]
pub(crate) struct FodFuseProfileCounters {
    fuse_read_total_us: AtomicU64,
    fuse_write_total_us: AtomicU64,
    read_block_map_us: AtomicU64,
    fetch_block_range_chunk_us: AtomicU64,
    fetch_block_range_parallel_us: AtomicU64,
    assemble_read_slice_us: AtomicU64,
    repo_fetch_block_range_us: AtomicU64,
    repo_assemble_file_slice_us: AtomicU64,
    repo_persist_blocks_us: AtomicU64,
    repo_persist_extents_us: AtomicU64,
    read_cache_lock_us: AtomicU64,
    read_block_cache_lock_us: AtomicU64,
    cached_read_block_us: AtomicU64,
    recent_write_blocks_lock_us: AtomicU64,
    recent_write_block_us: AtomicU64,
    write_state_lock_us: AtomicU64,
    write_state_clone_us: AtomicU64,
    update_write_buffer_us: AtomicU64,
    flush_write_state_us: AtomicU64,
    prepare_persist_rows_from_block_plan_us: AtomicU64,
    prepare_persist_extent_rows_from_extent_ranges_us: AtomicU64,
    prepare_persist_extent_rows_peak_payload_bytes: AtomicU64,
    prepare_persist_segment_rows_us: AtomicU64,
    segment_mode_entries: AtomicU64,
    segment_mode_downgrades: AtomicU64,
    segment_payload_bytes: AtomicU64,
    segment_count: AtomicU64,
    clear_read_cache_for_file_us: AtomicU64,
    store_recent_write_blocks_us: AtomicU64,
    reply_data_us: AtomicU64,
    reply_write_us: AtomicU64,
}

#[derive(Debug, Clone, Copy)]
enum FodFuseProfileTimerKind {
    FuseReadTotal,
    FuseWriteTotal,
}

#[derive(Debug)]
struct FodFuseProfileTimer {
    counters: Arc<FodFuseProfileCounters>,
    kind: FodFuseProfileTimerKind,
    started: Instant,
}

impl FodFuseProfileTimer {
    fn new(counters: Arc<FodFuseProfileCounters>, kind: FodFuseProfileTimerKind) -> Self {
        Self {
            counters,
            kind,
            started: Instant::now(),
        }
    }
}

impl Drop for FodFuseProfileTimer {
    fn drop(&mut self) {
        let elapsed = duration_to_micros(self.started.elapsed());
        let target = match self.kind {
            FodFuseProfileTimerKind::FuseReadTotal => &self.counters.fuse_read_total_us,
            FodFuseProfileTimerKind::FuseWriteTotal => &self.counters.fuse_write_total_us,
        };
        target.fetch_add(elapsed, Ordering::Relaxed);
    }
}

impl FodFuseProfileCounters {
    pub(crate) fn add(counter: &AtomicU64, elapsed: Duration) {
        counter.fetch_add(duration_to_micros(elapsed), Ordering::Relaxed);
    }

    pub(crate) fn record_repo_fetch_block_range_elapsed(&self, elapsed: Duration) {
        Self::add(&self.repo_fetch_block_range_us, elapsed);
    }

    pub(crate) fn record_repo_assemble_file_slice_elapsed(&self, elapsed: Duration) {
        Self::add(&self.repo_assemble_file_slice_us, elapsed);
    }

    pub(crate) fn record_repo_persist_blocks_elapsed(&self, elapsed: Duration) {
        Self::add(&self.repo_persist_blocks_us, elapsed);
    }

    pub(crate) fn record_repo_persist_extents_elapsed(&self, elapsed: Duration) {
        Self::add(&self.repo_persist_extents_us, elapsed);
    }

    pub(crate) fn record_read_block_map_elapsed(&self, elapsed: Duration) {
        Self::add(&self.read_block_map_us, elapsed);
    }

    pub(crate) fn record_fetch_block_range_chunk_elapsed(&self, elapsed: Duration) {
        Self::add(&self.fetch_block_range_chunk_us, elapsed);
    }

    pub(crate) fn record_fetch_block_range_parallel_elapsed(&self, elapsed: Duration) {
        Self::add(&self.fetch_block_range_parallel_us, elapsed);
    }

    pub(crate) fn record_assemble_read_slice_elapsed(&self, elapsed: Duration) {
        Self::add(&self.assemble_read_slice_us, elapsed);
    }

    pub(crate) fn record_read_cache_lock_elapsed(&self, elapsed: Duration) {
        Self::add(&self.read_cache_lock_us, elapsed);
    }

    pub(crate) fn record_read_block_cache_lock_elapsed(&self, elapsed: Duration) {
        Self::add(&self.read_block_cache_lock_us, elapsed);
    }

    pub(crate) fn record_cached_read_block_elapsed(&self, elapsed: Duration) {
        Self::add(&self.cached_read_block_us, elapsed);
    }

    pub(crate) fn record_recent_write_blocks_lock_elapsed(&self, elapsed: Duration) {
        Self::add(&self.recent_write_blocks_lock_us, elapsed);
    }

    pub(crate) fn record_recent_write_block_elapsed(&self, elapsed: Duration) {
        Self::add(&self.recent_write_block_us, elapsed);
    }

    pub(crate) fn record_write_state_lock_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_state_lock_us, elapsed);
    }

    pub(crate) fn record_write_state_clone_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_state_clone_us, elapsed);
    }

    pub(crate) fn record_update_write_buffer_elapsed(&self, elapsed: Duration) {
        Self::add(&self.update_write_buffer_us, elapsed);
    }

    pub(crate) fn record_flush_write_state_elapsed(&self, elapsed: Duration) {
        Self::add(&self.flush_write_state_us, elapsed);
    }

    pub(crate) fn record_prepare_persist_rows_from_block_plan_elapsed(&self, elapsed: Duration) {
        Self::add(&self.prepare_persist_rows_from_block_plan_us, elapsed);
    }

    pub(crate) fn record_prepare_persist_extent_rows_from_extent_ranges_elapsed(
        &self,
        elapsed: Duration,
    ) {
        Self::add(
            &self.prepare_persist_extent_rows_from_extent_ranges_us,
            elapsed,
        );
    }

    pub(crate) fn record_prepare_persist_extent_rows_peak_payload_bytes(&self, bytes: u64) {
        self.prepare_persist_extent_rows_peak_payload_bytes
            .fetch_max(bytes, Ordering::Relaxed);
    }

    pub(crate) fn record_prepare_persist_segment_rows_elapsed(&self, elapsed: Duration) {
        Self::add(&self.prepare_persist_segment_rows_us, elapsed);
    }

    pub(crate) fn record_segment_mode_entry(&self) {
        self.segment_mode_entries.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_segment_mode_downgrade(&self) {
        self.segment_mode_downgrades.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_segment_payload_bytes(&self, bytes: u64) {
        self.segment_payload_bytes
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn record_segment_count(&self, count: u64) {
        self.segment_count.fetch_add(count, Ordering::Relaxed);
    }

    pub(crate) fn record_clear_read_cache_for_file_elapsed(&self, elapsed: Duration) {
        Self::add(&self.clear_read_cache_for_file_us, elapsed);
    }

    pub(crate) fn record_store_recent_write_blocks_elapsed(&self, elapsed: Duration) {
        Self::add(&self.store_recent_write_blocks_us, elapsed);
    }

    pub(crate) fn record_reply_data_elapsed(&self, elapsed: Duration) {
        Self::add(&self.reply_data_us, elapsed);
    }

    pub(crate) fn record_reply_write_elapsed(&self, elapsed: Duration) {
        Self::add(&self.reply_write_us, elapsed);
    }

    pub(crate) fn has_activity(&self) -> bool {
        self.fuse_read_total_us.load(Ordering::Relaxed) > 0
            || self.fuse_write_total_us.load(Ordering::Relaxed) > 0
            || self.read_block_map_us.load(Ordering::Relaxed) > 0
            || self.fetch_block_range_chunk_us.load(Ordering::Relaxed) > 0
            || self.fetch_block_range_parallel_us.load(Ordering::Relaxed) > 0
            || self.assemble_read_slice_us.load(Ordering::Relaxed) > 0
            || self.repo_fetch_block_range_us.load(Ordering::Relaxed) > 0
            || self.repo_assemble_file_slice_us.load(Ordering::Relaxed) > 0
            || self.repo_persist_blocks_us.load(Ordering::Relaxed) > 0
            || self.repo_persist_extents_us.load(Ordering::Relaxed) > 0
            || self.read_cache_lock_us.load(Ordering::Relaxed) > 0
            || self.read_block_cache_lock_us.load(Ordering::Relaxed) > 0
            || self.cached_read_block_us.load(Ordering::Relaxed) > 0
            || self.recent_write_blocks_lock_us.load(Ordering::Relaxed) > 0
            || self.recent_write_block_us.load(Ordering::Relaxed) > 0
            || self.write_state_lock_us.load(Ordering::Relaxed) > 0
            || self.write_state_clone_us.load(Ordering::Relaxed) > 0
            || self.update_write_buffer_us.load(Ordering::Relaxed) > 0
            || self.flush_write_state_us.load(Ordering::Relaxed) > 0
            || self
                .prepare_persist_rows_from_block_plan_us
                .load(Ordering::Relaxed)
                > 0
            || self
                .prepare_persist_extent_rows_from_extent_ranges_us
                .load(Ordering::Relaxed)
                > 0
            || self
                .prepare_persist_extent_rows_peak_payload_bytes
                .load(Ordering::Relaxed)
                > 0
            || self.prepare_persist_segment_rows_us.load(Ordering::Relaxed) > 0
            || self.segment_mode_entries.load(Ordering::Relaxed) > 0
            || self.segment_mode_downgrades.load(Ordering::Relaxed) > 0
            || self.segment_payload_bytes.load(Ordering::Relaxed) > 0
            || self.segment_count.load(Ordering::Relaxed) > 0
            || self.clear_read_cache_for_file_us.load(Ordering::Relaxed) > 0
            || self.store_recent_write_blocks_us.load(Ordering::Relaxed) > 0
            || self.reply_data_us.load(Ordering::Relaxed) > 0
            || self.reply_write_us.load(Ordering::Relaxed) > 0
    }

    pub(crate) fn snapshot_lines(&self) -> Vec<String> {
        vec![
            format!(
                "fuse_read_total_us={}",
                self.fuse_read_total_us.load(Ordering::Relaxed)
            ),
            format!(
                "fuse_write_total_us={}",
                self.fuse_write_total_us.load(Ordering::Relaxed)
            ),
            format!(
                "read_block_map_us={}",
                self.read_block_map_us.load(Ordering::Relaxed)
            ),
            format!(
                "fetch_block_range_chunk_us={}",
                self.fetch_block_range_chunk_us.load(Ordering::Relaxed)
            ),
            format!(
                "fetch_block_range_parallel_us={}",
                self.fetch_block_range_parallel_us.load(Ordering::Relaxed)
            ),
            format!(
                "assemble_read_slice_us={}",
                self.assemble_read_slice_us.load(Ordering::Relaxed)
            ),
            format!(
                "repo_fetch_block_range_us={}",
                self.repo_fetch_block_range_us.load(Ordering::Relaxed)
            ),
            format!(
                "repo_assemble_file_slice_us={}",
                self.repo_assemble_file_slice_us.load(Ordering::Relaxed)
            ),
            format!(
                "repo_persist_blocks_us={}",
                self.repo_persist_blocks_us.load(Ordering::Relaxed)
            ),
            format!(
                "repo_persist_extents_us={}",
                self.repo_persist_extents_us.load(Ordering::Relaxed)
            ),
            format!(
                "read_cache_lock_us={}",
                self.read_cache_lock_us.load(Ordering::Relaxed)
            ),
            format!(
                "read_block_cache_lock_us={}",
                self.read_block_cache_lock_us.load(Ordering::Relaxed)
            ),
            format!(
                "cached_read_block_us={}",
                self.cached_read_block_us.load(Ordering::Relaxed)
            ),
            format!(
                "recent_write_blocks_lock_us={}",
                self.recent_write_blocks_lock_us.load(Ordering::Relaxed)
            ),
            format!(
                "recent_write_block_us={}",
                self.recent_write_block_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_state_lock_us={}",
                self.write_state_lock_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_state_clone_us={}",
                self.write_state_clone_us.load(Ordering::Relaxed)
            ),
            format!(
                "update_write_buffer_us={}",
                self.update_write_buffer_us.load(Ordering::Relaxed)
            ),
            format!(
                "flush_write_state_us={}",
                self.flush_write_state_us.load(Ordering::Relaxed)
            ),
            format!(
                "prepare_persist_rows_from_block_plan_us={}",
                self.prepare_persist_rows_from_block_plan_us
                    .load(Ordering::Relaxed)
            ),
            format!(
                "prepare_persist_extent_rows_from_extent_ranges_us={}",
                self.prepare_persist_extent_rows_from_extent_ranges_us
                    .load(Ordering::Relaxed)
            ),
            format!(
                "prepare_persist_extent_rows_peak_payload_bytes={}",
                self.prepare_persist_extent_rows_peak_payload_bytes
                    .load(Ordering::Relaxed)
            ),
            format!(
                "prepare_persist_segment_rows_us={}",
                self.prepare_persist_segment_rows_us.load(Ordering::Relaxed)
            ),
            format!(
                "segment_mode_entries={}",
                self.segment_mode_entries.load(Ordering::Relaxed)
            ),
            format!(
                "segment_mode_downgrades={}",
                self.segment_mode_downgrades.load(Ordering::Relaxed)
            ),
            format!(
                "segment_payload_bytes={}",
                self.segment_payload_bytes.load(Ordering::Relaxed)
            ),
            format!(
                "segment_count={}",
                self.segment_count.load(Ordering::Relaxed)
            ),
            format!(
                "clear_read_cache_for_file_us={}",
                self.clear_read_cache_for_file_us.load(Ordering::Relaxed)
            ),
            format!(
                "store_recent_write_blocks_us={}",
                self.store_recent_write_blocks_us.load(Ordering::Relaxed)
            ),
            format!(
                "reply_data_us={}",
                self.reply_data_us.load(Ordering::Relaxed)
            ),
            format!(
                "reply_write_us={}",
                self.reply_write_us.load(Ordering::Relaxed)
            ),
        ]
    }
}

pub struct FodFuse {
    pub repo: DbRepo,
    pub block_size: u64,
    pub write_flush_threshold_bytes: u64,
    pub max_fs_size_bytes: Option<u64>,
    pub pg_visible_path: Option<PathBuf>,
    pub lock_backend: LockBackend,
    pub lock_lease_ttl: Duration,
    pub lock_heartbeat_interval: Duration,
    pub lock_poll_interval: Duration,
    pub atime_policy: AtimePolicy,
    pub enable_extents: bool,
    pub extent_target_bytes: u64,
    reloadable_runtime: Arc<RwLock<RuntimeReloadableSettings>>,
    pub read_only: bool,
    pub use_fuse_context: bool,
    pub fopen_direct_io: bool,
    pub selinux_enabled: bool,
    pub acl_enabled: bool,
    inode_to_path: RwLock<HashMap<u64, String>>,
    path_to_inode: RwLock<HashMap<String, u64>>,
    fh_table: Mutex<HashMap<u64, FileHandleState>>,
    pub(crate) write_states: Mutex<HashMap<u64, WriteState>>,
    pub(crate) read_block_cache: Mutex<ReadBlockCache>,
    pub(crate) recent_write_blocks: Mutex<HashMap<(u64, u64), Arc<[u8]>>>,
    pub(crate) recent_write_blocks_len: AtomicU64,
    pub(crate) read_sequence_state: Mutex<HashMap<u64, ReadSequenceState>>,
    posix_locks: Arc<Mutex<HashMap<String, Vec<PosixLockRecord>>>>,
    local_lock_owners: Arc<Mutex<HashSet<u64>>>,
    pid_groups_cache: Mutex<HashMap<RequestPidGroupsCacheKey, CachedPidGroups>>,
    statfs_cache: Mutex<Option<StatfsSnapshot>>,
    last_write_session_touch: Mutex<Option<Instant>>,
    profile: Arc<FodFuseProfileCounters>,
    next_fh: Mutex<u64>,
    request_seq: AtomicU64,
    session_id: Option<u64>,
    lock_heartbeat: Option<LockHeartbeatHandle>,
    runtime_reload: Option<RuntimeReloadHandle>,
}

impl FodFuse {
    pub fn new(repo: DbRepo, settings: FodFuseSettings, runtime: &RuntimeConfig) -> Self {
        let FodFuseSettings {
            storage,
            cache,
            workers: _workers,
            locks,
            security,
            atime_policy,
            read_only,
            use_fuse_context,
            fopen_direct_io,
        } = settings;
        let mut inode_to_path = HashMap::new();
        let mut path_to_inode = HashMap::new();
        inode_to_path.insert(ROOT_INO, "/".to_string());
        path_to_inode.insert("/".to_string(), ROOT_INO);
        Self {
            repo,
            block_size: storage.block_size.max(1),
            write_flush_threshold_bytes: storage.write_flush_threshold_bytes,
            max_fs_size_bytes: storage.max_fs_size_bytes.filter(|value| *value > 0),
            pg_visible_path: storage.pg_visible_path,
            lock_backend: locks.lock_backend,
            lock_lease_ttl: locks.lock_lease_ttl,
            lock_heartbeat_interval: locks.lock_heartbeat_interval,
            lock_poll_interval: locks.lock_poll_interval,
            atime_policy,
            enable_extents: storage.enable_extents,
            extent_target_bytes: storage.extent_target_bytes,
            reloadable_runtime: Arc::new(RwLock::new(runtime.reloadable_settings())),
            read_only,
            use_fuse_context,
            fopen_direct_io,
            selinux_enabled: security.selinux_enabled,
            acl_enabled: security.acl_enabled,
            inode_to_path: RwLock::new(inode_to_path),
            path_to_inode: RwLock::new(path_to_inode),
            fh_table: Mutex::new(HashMap::new()),
            write_states: Mutex::new(HashMap::new()),
            read_block_cache: Mutex::new(ReadBlockCache::new(
                cache.read_cache_eviction_policy.as_str(),
            )),
            recent_write_blocks: Mutex::new(HashMap::new()),
            recent_write_blocks_len: AtomicU64::new(0),
            read_sequence_state: Mutex::new(HashMap::new()),
            posix_locks: Arc::new(Mutex::new(HashMap::new())),
            local_lock_owners: Arc::new(Mutex::new(HashSet::new())),
            pid_groups_cache: Mutex::new(HashMap::new()),
            statfs_cache: Mutex::new(None),
            last_write_session_touch: Mutex::new(None),
            profile: Arc::new(FodFuseProfileCounters::default()),
            next_fh: Mutex::new(1),
            request_seq: AtomicU64::new(1),
            session_id: None,
            lock_heartbeat: None,
            runtime_reload: None,
        }
    }

    fn next_request_id(&self) -> u64 {
        self.request_seq.fetch_add(1, Ordering::Relaxed)
    }

    fn request_prefix(&self, req_id: u64, op: &str) -> String {
        format!("req={} op={}", req_id, op)
    }

    pub(crate) fn reloadable_runtime(&self) -> RuntimeReloadableSettings {
        match self.reloadable_runtime.read() {
            Ok(guard) => guard.clone(),
            Err(err) => err.into_inner().clone(),
        }
    }

    pub(crate) fn metadata_cache_ttl_live(&self) -> Duration {
        self.reloadable_runtime().metadata_cache_ttl
    }

    pub(crate) fn statfs_cache_ttl_live(&self) -> Duration {
        self.reloadable_runtime().statfs_cache_ttl
    }

    fn start_fuse_read_profile(&self) -> FodFuseProfileTimer {
        FodFuseProfileTimer::new(
            Arc::clone(&self.profile),
            FodFuseProfileTimerKind::FuseReadTotal,
        )
    }

    fn start_fuse_write_profile(&self) -> FodFuseProfileTimer {
        FodFuseProfileTimer::new(
            Arc::clone(&self.profile),
            FodFuseProfileTimerKind::FuseWriteTotal,
        )
    }

    pub(crate) fn profile_counters(&self) -> Arc<FodFuseProfileCounters> {
        Arc::clone(&self.profile)
    }

    pub(crate) fn record_read_cache_lock_elapsed(&self, elapsed: Duration) {
        self.profile.record_read_cache_lock_elapsed(elapsed);
    }

    pub(crate) fn record_read_block_cache_lock_elapsed(&self, elapsed: Duration) {
        self.profile.record_read_block_cache_lock_elapsed(elapsed);
    }

    pub(crate) fn record_cached_read_block_elapsed(&self, elapsed: Duration) {
        self.profile.record_cached_read_block_elapsed(elapsed);
    }

    pub(crate) fn record_recent_write_blocks_lock_elapsed(&self, elapsed: Duration) {
        self.profile
            .record_recent_write_blocks_lock_elapsed(elapsed);
    }

    pub(crate) fn record_recent_write_block_elapsed(&self, elapsed: Duration) {
        self.profile.record_recent_write_block_elapsed(elapsed);
    }

    pub(crate) fn record_write_state_lock_elapsed(&self, elapsed: Duration) {
        self.profile.record_write_state_lock_elapsed(elapsed);
    }

    pub(crate) fn record_write_state_clone_elapsed(&self, elapsed: Duration) {
        self.profile.record_write_state_clone_elapsed(elapsed);
    }

    pub(crate) fn clone_write_state_profiled(&self, state: &WriteState) -> WriteState {
        let started = Instant::now();
        let cloned = state.clone();
        self.record_write_state_clone_elapsed(started.elapsed());
        cloned
    }

    pub(crate) fn record_update_write_buffer_elapsed(&self, elapsed: Duration) {
        self.profile.record_update_write_buffer_elapsed(elapsed);
    }

    pub(crate) fn record_flush_write_state_elapsed(&self, elapsed: Duration) {
        self.profile.record_flush_write_state_elapsed(elapsed);
    }

    pub(crate) fn record_read_block_map_elapsed(&self, elapsed: Duration) {
        self.profile.record_read_block_map_elapsed(elapsed);
    }

    pub(crate) fn record_assemble_read_slice_elapsed(&self, elapsed: Duration) {
        self.profile.record_assemble_read_slice_elapsed(elapsed);
    }

    fn record_repo_fetch_block_range_elapsed(&self, elapsed: Duration) {
        self.profile.record_repo_fetch_block_range_elapsed(elapsed);
    }

    fn record_repo_assemble_file_slice_elapsed(&self, elapsed: Duration) {
        self.profile
            .record_repo_assemble_file_slice_elapsed(elapsed);
    }

    fn record_repo_persist_blocks_elapsed(&self, elapsed: Duration) {
        self.profile.record_repo_persist_blocks_elapsed(elapsed);
    }

    fn record_repo_persist_extents_elapsed(&self, elapsed: Duration) {
        self.profile.record_repo_persist_extents_elapsed(elapsed);
    }

    pub(crate) fn record_prepare_persist_rows_from_block_plan_elapsed(&self, elapsed: Duration) {
        self.profile
            .record_prepare_persist_rows_from_block_plan_elapsed(elapsed);
    }

    pub(crate) fn record_prepare_persist_extent_rows_from_extent_ranges_elapsed(
        &self,
        elapsed: Duration,
    ) {
        self.profile
            .record_prepare_persist_extent_rows_from_extent_ranges_elapsed(elapsed);
    }

    pub(crate) fn record_prepare_persist_extent_rows_peak_payload_bytes(&self, bytes: u64) {
        self.profile
            .record_prepare_persist_extent_rows_peak_payload_bytes(bytes);
    }

    pub(crate) fn record_prepare_persist_segment_rows_elapsed(&self, elapsed: Duration) {
        self.profile
            .record_prepare_persist_segment_rows_elapsed(elapsed);
    }

    pub(crate) fn record_segment_mode_entry(&self) {
        self.profile.record_segment_mode_entry();
    }

    pub(crate) fn record_segment_mode_downgrade(&self) {
        self.profile.record_segment_mode_downgrade();
    }

    pub(crate) fn record_segment_payload_bytes(&self, bytes: u64) {
        self.profile.record_segment_payload_bytes(bytes);
    }

    pub(crate) fn record_segment_count(&self, count: u64) {
        self.profile.record_segment_count(count);
    }

    pub(crate) fn record_clear_read_cache_for_file_elapsed(&self, elapsed: Duration) {
        self.profile
            .record_clear_read_cache_for_file_elapsed(elapsed);
    }

    pub(crate) fn record_store_recent_write_blocks_elapsed(&self, elapsed: Duration) {
        self.profile
            .record_store_recent_write_blocks_elapsed(elapsed);
    }

    fn record_reply_data_elapsed(&self, elapsed: Duration) {
        self.profile.record_reply_data_elapsed(elapsed);
    }

    fn record_reply_write_elapsed(&self, elapsed: Duration) {
        self.profile.record_reply_write_elapsed(elapsed);
    }

    pub(crate) fn fetch_block_range_profiled(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        block_size: u64,
    ) -> Result<Vec<(u64, Arc<[u8]>)>, String> {
        let started = Instant::now();
        let result = self
            .repo
            .fetch_block_range(file_id, first_block, last_block, block_size);
        self.record_repo_fetch_block_range_elapsed(started.elapsed());
        result.map(|rows| {
            let mut blocks = rows
                .into_iter()
                .map(|(block_index, block)| (block_index, Arc::<[u8]>::from(block)))
                .collect::<Vec<_>>();
            blocks.sort_unstable_by_key(|(block_index, _)| *block_index);
            blocks
        })
    }

    pub(crate) fn load_block_profiled(
        &self,
        file_id: u64,
        block_index: u64,
        block_size: u64,
    ) -> Result<Option<Vec<u8>>, String> {
        let started = Instant::now();
        let result = self.repo.load_block(file_id, block_index, block_size);
        self.record_repo_fetch_block_range_elapsed(started.elapsed());
        result
    }

    pub(crate) fn assemble_file_slice_profiled(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        offset: u64,
        end_offset: u64,
        block_size: u64,
    ) -> Result<Vec<u8>, String> {
        let started = Instant::now();
        let result = self.repo.assemble_file_slice(
            file_id,
            first_block,
            last_block,
            offset,
            end_offset,
            block_size,
        );
        self.record_repo_assemble_file_slice_elapsed(started.elapsed());
        result
    }

    pub(crate) fn persist_file_blocks_profiled<'a>(
        &self,
        file_id: u64,
        file_size: u64,
        block_size: u64,
        total_blocks: u64,
        truncate_pending: bool,
        blocks: &[PersistBlockRow<'a>],
        maintain_copy_crc_table: bool,
    ) -> Result<(), String> {
        let started = Instant::now();
        let result = self.repo.persist_file_blocks_with_crc_flag(
            file_id,
            file_size,
            block_size,
            total_blocks,
            truncate_pending,
            blocks,
            maintain_copy_crc_table,
        );
        self.record_repo_persist_blocks_elapsed(started.elapsed());
        result
    }

    pub(crate) fn persist_file_extents_profiled(
        &self,
        file_id: u64,
        file_size: u64,
        block_size: u64,
        total_blocks: u64,
        truncate_pending: bool,
        extents: &[PersistExtentRow],
        maintain_copy_crc_table: bool,
    ) -> Result<(), String> {
        let started = Instant::now();
        let result = self.repo.persist_file_extents_native(
            file_id,
            file_size,
            block_size,
            total_blocks,
            truncate_pending,
            extents,
            maintain_copy_crc_table,
        );
        self.record_repo_persist_extents_elapsed(started.elapsed());
        result
    }

    pub(crate) fn persist_new_object_extents_profiled(
        &self,
        file_id: u64,
        file_size: u64,
        block_size: u64,
        total_blocks: u64,
        extents: &[PersistExtentRow],
        maintain_copy_crc_table: bool,
    ) -> Result<u64, String> {
        let started = Instant::now();
        let result = self.repo.persist_new_object_extents(
            file_id,
            file_size,
            block_size,
            total_blocks,
            extents,
            maintain_copy_crc_table,
        );
        self.record_repo_persist_extents_elapsed(started.elapsed());
        result
    }

    fn reply_data_profiled(&self, reply: ReplyData, data: &[u8]) {
        let started = Instant::now();
        reply.data(data);
        self.record_reply_data_elapsed(started.elapsed());
    }

    fn reply_xattr_profiled(&self, reply: ReplyXattr, data: &[u8]) {
        let started = Instant::now();
        reply.data(data);
        self.record_reply_data_elapsed(started.elapsed());
    }

    fn reply_written_profiled(&self, reply: ReplyWrite, written: u32) {
        let started = Instant::now();
        reply.written(written);
        self.record_reply_write_elapsed(started.elapsed());
    }

    pub(crate) fn maybe_touch_client_session_write(&self) {
        let Some(session_id) = self.session_id else {
            return;
        };

        let now = Instant::now();
        let mut guard = match self.last_write_session_touch.lock() {
            Ok(guard) => guard,
            Err(err) => {
                warn!(
                    "FOD client session write touch skipped: poisoned throttle state: {}",
                    err
                );
                return;
            }
        };

        // Keep the extra write-side session update out of the hot path.
        if guard
            .as_ref()
            .is_some_and(|last| now.duration_since(*last) < CLIENT_WRITE_TOUCH_INTERVAL)
        {
            return;
        }

        if let Err(err) = self.repo.touch_client_session_write(session_id) {
            warn!(
                "FOD client session write tracking failed session_id={} err={}",
                session_id, err
            );
            return;
        }

        *guard = Some(now);
    }

    fn log_request_start(&self, req_id: u64, op: &str, details: impl AsRef<str>) {
        debug!(
            "FOD {} {}",
            self.request_prefix(req_id, op),
            details.as_ref()
        );
    }

    fn log_request_error(
        &self,
        req_id: u64,
        op: &str,
        errno: libc::c_int,
        details: impl AsRef<str>,
    ) {
        warn!(
            "FOD {} errno={} {}",
            self.request_prefix(req_id, op),
            errno,
            details.as_ref()
        );
    }

    pub(crate) fn debug_snapshot(&self) -> String {
        let live = self.reloadable_runtime();
        let path_count = self
            .path_to_inode
            .read()
            .map(|guard| guard.len())
            .unwrap_or(0);
        let inode_count = self
            .inode_to_path
            .read()
            .map(|guard| guard.len())
            .unwrap_or(0);
        let (fh_table_count, fh_file_id_count, fh_flags_count, fh_atime_touched_count) = self
            .fh_table
            .lock()
            .map(|guard| {
                let total = guard.len();
                let file_ids = guard
                    .values()
                    .filter(|state| state.file_id.is_some())
                    .count();
                let flags = guard.values().filter(|state| state.flags != 0).count();
                let atime_touched = guard.values().filter(|state| state.atime_touched).count();
                (total, file_ids, flags, atime_touched)
            })
            .unwrap_or((0, 0, 0, 0));
        let write_state_count = self
            .write_states
            .lock()
            .map(|guard| guard.len())
            .unwrap_or(0);
        let read_cache_count = self
            .read_block_cache
            .lock()
            .map(|guard| guard.len())
            .unwrap_or(0);
        let read_sequence_count = self
            .read_sequence_state
            .lock()
            .map(|guard| guard.len())
            .unwrap_or(0);
        let lock_count = self
            .posix_locks
            .lock()
            .map(|guard| guard.len())
            .unwrap_or(0);
        let mut samples = Vec::new();
        if let Ok(guard) = self.path_to_inode.read() {
            for (path, ino) in guard.iter().take(5) {
                samples.push(format!("{path}->{ino}"));
            }
        }
        format!(
            "FodFuseSnapshot{{read_only={}, use_fuse_context={}, fopen_direct_io={}, block_size={}, write_flush_threshold_bytes={}, read_cache_blocks={}, read_ahead_blocks={}, sequential_read_ahead_blocks={}, small_file_read_threshold_blocks={}, workers_read={}, workers_read_min_blocks={}, workers_write={}, workers_write_min_blocks={}, atime_policy={:?}, lock_backend={:?}, lock_lease_ttl_secs={}, lock_heartbeat_interval_secs={}, lock_poll_interval_secs={}, copy_dedupe_enabled={}, copy_dedupe_min_blocks={}, copy_dedupe_max_blocks={}, copy_dedupe_crc_table={}, enable_extents={}, extent_target_bytes={}, selinux_enabled={}, acl_enabled={}, inode_to_path={}, path_to_inode={}, fh_table={}, fh_table_file_ids={}, fh_table_flags={}, fh_table_atime_touched={}, write_states={}, read_cache_entries={}, read_sequences={}, posix_locks={}, samples=[{}]}}",
            self.read_only,
            self.use_fuse_context,
            self.fopen_direct_io,
            self.block_size,
            self.write_flush_threshold_bytes,
            live.read_cache_blocks,
            live.read_ahead_blocks,
            live.sequential_read_ahead_blocks,
            live.small_file_read_threshold_blocks,
            live.workers_read,
            live.workers_read_min_blocks,
            live.workers_write,
            live.workers_write_min_blocks,
            self.atime_policy,
            self.lock_backend,
            self.lock_lease_ttl.as_secs_f64(),
            self.lock_heartbeat_interval.as_secs_f64(),
            self.lock_poll_interval.as_secs_f64(),
            live.copy_dedupe_enabled,
            live.copy_dedupe_min_blocks,
            live.copy_dedupe_max_blocks,
            live.copy_dedupe_crc_table,
            self.enable_extents,
            self.extent_target_bytes,
            self.selinux_enabled,
            self.acl_enabled,
            inode_count,
            path_count,
            fh_table_count,
            fh_file_id_count,
            fh_flags_count,
            fh_atime_touched_count,
            write_state_count,
            read_cache_count,
            read_sequence_count,
            lock_count,
            samples.join(", ")
        )
    }

    fn current_statfs_snapshot(&self) -> StatfsSnapshot {
        let (files, dirs, total_data_size) = self.repo.statfs_snapshot().unwrap_or((0, 0, 0));
        let blocks = (total_data_size + self.block_size.saturating_sub(1)) / self.block_size;
        StatfsSnapshot {
            files,
            dirs,
            total_data_size,
            blocks,
            loaded_at: SystemTime::now(),
        }
    }

    pub(crate) fn invalidate_statfs_cache(&self) {
        if let Ok(mut guard) = self.statfs_cache.lock() {
            *guard = None;
        }
    }

    #[cfg(unix)]
    fn statvfs_total_bytes(path: &Path) -> Result<u64, String> {
        use std::ffi::CString;
        use std::mem::MaybeUninit;

        let c_path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| format!("path contains NUL byte: {}", path.display()))?;
        let mut stats = MaybeUninit::<libc::statvfs>::uninit();
        let rc = unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) };
        if rc != 0 {
            return Err(format!("statvfs failed for {}", path.display()));
        }
        let stats = unsafe { stats.assume_init() };
        Ok((stats.f_frsize as u64).saturating_mul(stats.f_blocks as u64))
    }

    #[cfg(not(unix))]
    fn statvfs_total_bytes(_path: &Path) -> Result<u64, String> {
        Err("statvfs is unavailable on this platform".to_string())
    }

    fn statfs_capacity_bytes(&self) -> Option<u64> {
        let visible_total = self
            .pg_visible_path
            .as_ref()
            .and_then(|path| Self::statvfs_total_bytes(path).ok());
        match (self.max_fs_size_bytes, visible_total) {
            (Some(requested), Some(visible)) => Some(requested.min(visible)),
            (Some(requested), None) => Some(requested),
            (None, Some(visible)) => Some(visible),
            (None, None) => None,
        }
    }

    fn cached_statfs_snapshot(&self) -> StatfsSnapshot {
        let ttl = self.statfs_cache_ttl_live();
        if ttl.is_zero() {
            return self.current_statfs_snapshot();
        }

        if let Ok(guard) = self.statfs_cache.lock() {
            if let Some(snapshot) = guard.as_ref() {
                if let Ok(age) = snapshot.loaded_at.elapsed() {
                    if age <= ttl {
                        return snapshot.clone();
                    }
                } else {
                    return snapshot.clone();
                }
            }
        }

        let snapshot = self.current_statfs_snapshot();
        if let Ok(mut guard) = self.statfs_cache.lock() {
            *guard = Some(snapshot.clone());
        }
        snapshot
    }

    fn normalize_path(path: &str) -> String {
        let mut value = path.trim().to_string();
        if value.is_empty() {
            return "/".to_string();
        }
        if !value.starts_with('/') {
            value.insert(0, '/');
        }
        if value.len() > 1 && value.ends_with('/') {
            while value.len() > 1 && value.ends_with('/') {
                value.pop();
            }
        }
        if value.is_empty() {
            "/".to_string()
        } else {
            value
        }
    }

    fn join_path(parent: &str, name: &OsStr) -> String {
        let name = name.to_string_lossy();
        if parent == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", parent.trim_end_matches('/'), name)
        }
    }

    fn copy_dedupe_enabled_for_len(&self, len: u64) -> bool {
        let live = self.reloadable_runtime();
        if !live.copy_dedupe_enabled || len == 0 {
            return false;
        }
        let total_blocks = 1 + (len - 1) / self.block_size.max(1);
        if total_blocks < live.copy_dedupe_min_blocks.max(1) {
            return false;
        }
        if live.copy_dedupe_max_blocks != 0 && total_blocks > live.copy_dedupe_max_blocks {
            return false;
        }
        true
    }

    fn logical_inode(&self, obj_type: &str, entry_id: u64) -> u64 {
        match obj_type {
            "file" | "hardlink" => 1_000_000 + entry_id,
            "dir" => 2_000_000 + entry_id,
            "symlink" => 3_000_000 + entry_id,
            _ => entry_id,
        }
    }

    fn hash_inode64(data: &[u8]) -> u64 {
        // FNV-1a 64-bit keeps inode generation deterministic while making collisions
        // much less likely than the previous CRC32-based scheme.
        const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

        let mut hash = FNV_OFFSET_BASIS;
        for &byte in data {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash
    }

    fn stable_inode(&self, obj_type: &str, inode_seed: &str, entry_id: u64) -> u64 {
        if inode_seed.is_empty() {
            return self.logical_inode(obj_type, entry_id);
        }
        let normalized_obj_type = if obj_type == "hardlink" {
            "file"
        } else {
            obj_type
        };
        let payload = format!("{normalized_obj_type}:{inode_seed}");
        let inode = Self::hash_inode64(payload.as_bytes()) & i64::MAX as u64;
        if inode == 0 {
            self.logical_inode(obj_type, entry_id)
        } else {
            inode
        }
    }

    fn parse_time(value: &str) -> SystemTime {
        let value = value.trim();
        if value.is_empty() {
            return UNIX_EPOCH;
        }
        let normalized = value.replace('T', " ");
        let naive_formats = ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"];
        for fmt in naive_formats {
            if let Ok(naive) = NaiveDateTime::parse_from_str(&normalized, fmt) {
                let dt = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);
                return SystemTime::UNIX_EPOCH
                    + Duration::from_secs(dt.timestamp().max(0) as u64)
                    + Duration::from_nanos(dt.timestamp_subsec_nanos() as u64);
            }
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(&value.replace(' ', "T")) {
            return SystemTime::UNIX_EPOCH
                + Duration::from_secs(dt.timestamp().max(0) as u64)
                + Duration::from_nanos(dt.timestamp_subsec_nanos() as u64);
        }
        UNIX_EPOCH
    }

    fn file_type_from_special(special_type: &str) -> FileType {
        match special_type {
            "fifo" => FileType::NamedPipe,
            "char" => FileType::CharDevice,
            "block" => FileType::BlockDevice,
            _ => FileType::RegularFile,
        }
    }

    fn decode_nul_fields(blob: &[u8]) -> Vec<String> {
        if blob.is_empty() {
            return Vec::new();
        }
        blob.split(|b| *b == 0)
            .map(|part| String::from_utf8_lossy(part).to_string())
            .collect()
    }

    fn file_id_for_path(&self, path: &str) -> Result<Option<u64>, libc::c_int> {
        let resolved = self.repo.resolve_path(path).map_err(|_| EIO)?;
        match resolved.kind.as_deref() {
            Some("hardlink") => {
                let hardlink_id = resolved.entry_id.ok_or(EIO)?;
                self.repo.get_hardlink_file_id(hardlink_id).map_err(|_| EIO)
            }
            Some("file") => Ok(resolved.entry_id),
            _ => Ok(None),
        }
    }

    fn resolved_entry_for_path(
        &self,
        path: &str,
    ) -> Result<(Option<String>, Option<u64>), libc::c_int> {
        let resolved = self.repo.resolve_path(path).map_err(|_| EIO)?;
        Ok((resolved.kind, resolved.entry_id))
    }

    fn raw_mode_for_path(&self, path: &str) -> Result<Option<String>, libc::c_int> {
        let blob = self.repo.fetch_path_attrs_blob(path).map_err(|_| EIO)?;
        let Some(blob) = blob else {
            return Ok(None);
        };
        let fields = Self::decode_nul_fields(&blob);
        if fields.len() < 4 {
            return Ok(None);
        }
        Ok(Some(fields[3].clone()))
    }

    fn current_group_ids() -> HashSet<u32> {
        let mut group_ids = HashSet::new();
        unsafe {
            let count = libc::getgroups(0, std::ptr::null_mut());
            if count > 0 {
                let mut groups = vec![0 as libc::gid_t; count as usize];
                let rc = libc::getgroups(count, groups.as_mut_ptr());
                if rc >= 0 {
                    for group in groups {
                        group_ids.insert(group as u32);
                    }
                }
            }
        }
        group_ids
    }

    fn process_identity() -> SubjectIdentity {
        let uid = unsafe { libc::geteuid() } as u32;
        let gid = unsafe { libc::getegid() } as u32;
        let mut groups = Self::current_group_ids();
        groups.insert(gid);
        SubjectIdentity { uid, gid, groups }
    }

    fn groups_from_request_pid(pid: u32) -> HashSet<u32> {
        let mut groups = HashSet::new();
        let status_path = format!("/proc/{pid}/status");
        let Ok(status) = fs::read_to_string(status_path) else {
            return groups;
        };
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("Groups:") {
                for token in rest.split_whitespace() {
                    if let Ok(group) = token.parse::<u32>() {
                        groups.insert(group);
                    }
                }
                break;
            }
        }
        groups
    }

    fn request_pid_starttime(pid: u32) -> Option<u64> {
        let stat_path = format!("/proc/{pid}/stat");
        let stat = fs::read_to_string(stat_path).ok()?;
        let (_, remainder) = stat.rsplit_once(") ")?;
        let mut fields = remainder.split_whitespace();
        for _ in 0..19 {
            fields.next()?;
        }
        fields.next()?.parse::<u64>().ok()
    }

    fn request_pid_cache_key(pid: u32) -> Option<RequestPidGroupsCacheKey> {
        Some(RequestPidGroupsCacheKey {
            pid,
            starttime: Self::request_pid_starttime(pid)?,
        })
    }

    fn cached_groups_from_request_pid(&self, pid: u32) -> HashSet<u32> {
        let now = Instant::now();
        let cache_key = Self::request_pid_cache_key(pid);
        if let Some(cache_key) = cache_key.as_ref() {
            if let Ok(cache) = self.pid_groups_cache.lock() {
                if let Some(entry) = cache.get(cache_key) {
                    if entry.expires_at > now {
                        return entry.groups.clone();
                    }
                }
            }
        }

        let groups = Self::groups_from_request_pid(pid);
        if let Some(cache_key) = cache_key {
            if let Ok(mut cache) = self.pid_groups_cache.lock() {
                cache.retain(|_, entry| entry.expires_at > now);
                cache.insert(
                    cache_key,
                    CachedPidGroups {
                        groups: groups.clone(),
                        expires_at: now + REQUEST_PID_GROUPS_CACHE_TTL,
                    },
                );
            }
        }
        groups
    }

    fn request_identity(&self, req: &Request<'_>) -> SubjectIdentity {
        if !self.use_fuse_context {
            return Self::process_identity();
        }
        let uid = req.uid();
        let gid = req.gid();
        let mut groups = self.cached_groups_from_request_pid(req.pid());
        groups.insert(gid);
        SubjectIdentity { uid, gid, groups }
    }

    fn can_access(&self, subject: &SubjectIdentity, attrs: &FileAttr, mode: i32) -> bool {
        if subject.is_root() {
            return true;
        }
        let required = Self::access_mask_from_mode(mode);
        let allowed = if subject.uid == attrs.uid {
            (attrs.perm >> 6) & 0o7
        } else if subject.groups.contains(&attrs.gid) {
            (attrs.perm >> 3) & 0o7
        } else {
            attrs.perm & 0o7
        };
        (allowed & required) == required
    }

    fn access_mask_from_mode(mask: i32) -> u16 {
        let mut required = 0u16;
        if mask & libc::R_OK != 0 {
            required |= 0o4;
        }
        if mask & libc::W_OK != 0 {
            required |= 0o2;
        }
        if mask & libc::X_OK != 0 {
            required |= 0o1;
        }
        required
    }

    fn parse_posix_acl_xattr(value: &[u8]) -> Result<Vec<PosixAclEntry>, libc::c_int> {
        if value.len() < 4 {
            return Err(libc::EINVAL);
        }
        let version = u32::from_le_bytes(value[0..4].try_into().map_err(|_| libc::EINVAL)?);
        if version != 0x0002 {
            return Err(libc::EINVAL);
        }
        let mut entries = Vec::new();
        let mut idx = 4usize;
        while idx + 8 <= value.len() {
            let tag = u16::from_le_bytes(value[idx..idx + 2].try_into().map_err(|_| libc::EINVAL)?);
            let perm = u16::from_le_bytes(
                value[idx + 2..idx + 4]
                    .try_into()
                    .map_err(|_| libc::EINVAL)?,
            );
            let mut id = i32::from_le_bytes(
                value[idx + 4..idx + 8]
                    .try_into()
                    .map_err(|_| libc::EINVAL)?,
            );
            if tag == 0x0001 || tag == 0x0004 || tag == 0x0020 || tag == 0x0010 {
                id = -1;
            }
            entries.push(PosixAclEntry { tag, perm, id });
            idx += 8;
        }
        if idx != value.len() {
            return Err(libc::EINVAL);
        }
        Ok(entries)
    }

    fn acl_permission_from_entries(
        &self,
        subject: &SubjectIdentity,
        entries: &[PosixAclEntry],
        attrs: &FileAttr,
        mode: i32,
    ) -> bool {
        if subject.is_root() {
            return true;
        }
        let required = Self::access_mask_from_mode(mode);
        let mut mask_perm = 0o7u16;
        let mut user_obj_perm = None;
        let mut group_obj_perm = None;
        let mut other_perm = None;
        let mut named_user_perm = None;
        let mut named_group_matches: Vec<(u32, u16)> = Vec::new();
        for entry in entries {
            let perm = entry.perm & 0o7;
            match entry.tag {
                0x0001 => user_obj_perm = Some(perm),
                0x0002 if entry.id >= 0 && entry.id as u32 == subject.uid => {
                    named_user_perm = Some(perm)
                }
                0x0004 => group_obj_perm = Some(perm),
                0x0008 if entry.id >= 0 => named_group_matches.push((entry.id as u32, perm)),
                0x0010 => mask_perm = perm,
                0x0020 => other_perm = Some(perm),
                _ => {}
            }
        }
        if attrs.uid == subject.uid {
            return user_obj_perm.unwrap_or(0) & required == required;
        }
        if let Some(named_user_perm) = named_user_perm {
            let allowed = named_user_perm & mask_perm;
            return (allowed & required) == required;
        }
        let mut group_allowed = 0u16;
        if subject.groups.contains(&attrs.gid) {
            group_allowed |= group_obj_perm.unwrap_or(0);
        }
        for (group_id, perm) in named_group_matches {
            if subject.groups.contains(&group_id) {
                group_allowed |= perm;
            }
        }
        if group_allowed != 0 {
            let allowed = group_allowed & mask_perm;
            return (allowed & required) == required;
        }
        (other_perm.unwrap_or(0) & required) == required
    }

    fn acl_allows(
        &self,
        path: &str,
        attrs: &FileAttr,
        mode: i32,
        subject: &SubjectIdentity,
    ) -> Result<bool, libc::c_int> {
        if !self.acl_enabled {
            return Ok(self.can_access(subject, attrs, mode));
        }
        let acl_value = self
            .repo
            .fetch_xattr_value(path, "system.posix_acl_access")
            .map_err(|_| EIO)?;
        if let Some(value) = acl_value {
            let entries = Self::parse_posix_acl_xattr(&value)?;
            Ok(self.acl_permission_from_entries(subject, &entries, attrs, mode))
        } else {
            Ok(self.can_access(subject, attrs, mode))
        }
    }

    fn copy_default_acl_to_child(
        &self,
        parent_path: &str,
        owner_kind: &str,
        owner_id: u64,
        child_is_dir: bool,
    ) -> Result<(), libc::c_int> {
        if !self.acl_enabled {
            return Ok(());
        }
        let default_acl = self
            .repo
            .fetch_xattr_value(parent_path, "system.posix_acl_default")
            .map_err(|_| EIO)?;
        let Some(default_acl) = default_acl else {
            return Ok(());
        };
        self.repo
            .store_xattr_value_for_owner(
                owner_kind,
                owner_id,
                "system.posix_acl_access",
                &default_acl,
            )
            .map_err(|_| EIO)?;
        if child_is_dir {
            self.repo
                .store_xattr_value_for_owner(
                    owner_kind,
                    owner_id,
                    "system.posix_acl_default",
                    &default_acl,
                )
                .map_err(|_| EIO)?;
        }
        Ok(())
    }

    fn enforce_sticky_bit(
        &self,
        parent_path: &str,
        entry_attrs: &FileAttr,
        subject: &SubjectIdentity,
    ) -> Result<(), libc::c_int> {
        let parent_path = Self::normalize_path(parent_path);
        if parent_path == "/" {
            return Ok(());
        }
        let parent_attrs = match self.lookup_path(&parent_path) {
            Ok(Some(attrs)) => attrs.file_attr,
            Ok(None) => return Err(ENOENT),
            Err(errno) => return Err(errno),
        };
        if (parent_attrs.perm & libc::S_ISVTX as u16) == 0 {
            return Ok(());
        }
        if subject.is_root() {
            return Ok(());
        }
        if subject.uid == entry_attrs.uid || subject.uid == parent_attrs.uid {
            return Ok(());
        }
        Err(libc::EPERM)
    }

    fn can_modify_mode(subject: &SubjectIdentity, attrs: &FileAttr) -> bool {
        subject.is_root() || subject.uid == attrs.uid
    }

    fn can_change_owner(
        subject: &SubjectIdentity,
        attrs: &FileAttr,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> bool {
        if subject.is_root() {
            return true;
        }
        if subject.uid != attrs.uid {
            return false;
        }
        if let Some(new_uid) = uid {
            if new_uid != attrs.uid {
                return false;
            }
        }
        if let Some(new_gid) = gid {
            if new_gid != attrs.gid && !subject.groups.contains(&new_gid) {
                return false;
            }
        }
        true
    }

    fn append_journal_event(
        &self,
        uid: u32,
        action: &str,
        path: &str,
        file_id: Option<u64>,
        directory_id: Option<u64>,
    ) -> Result<(), libc::c_int> {
        self.repo
            .append_journal_event(uid, directory_id, file_id, &format!("{action}:{path}"))
            .map_err(|_| EIO)
    }

    fn posix_lock_type_conflicts(existing: i32, requested: i32) -> bool {
        if requested == libc::F_RDLCK {
            existing == libc::F_WRLCK
        } else if requested == libc::F_WRLCK {
            existing == libc::F_RDLCK || existing == libc::F_WRLCK
        } else {
            false
        }
    }

    fn lock_conflict(
        records: &[PosixLockRecord],
        owner: u64,
        requested_type: i32,
        requested_start: u64,
        requested_end: u64,
    ) -> Option<PosixLockRecord> {
        for record in records {
            if record.owner == owner {
                continue;
            }
            if !Self::range_overlaps(
                requested_start,
                Some(requested_end),
                record.start,
                record.end,
            ) {
                continue;
            }
            if Self::posix_lock_type_conflicts(record.typ, requested_type) {
                return Some(record.clone());
            }
        }
        None
    }

    fn range_overlaps(start_a: u64, end_a: Option<u64>, start_b: u64, end_b: Option<u64>) -> bool {
        let end_a = end_a.unwrap_or(u64::MAX);
        let end_b = end_b.unwrap_or(u64::MAX);
        start_a < end_b && start_b < end_a
    }

    fn resource_key_for_lock(&self, path: &str) -> Result<String, libc::c_int> {
        let resolved = self.repo.resolve_path(path).map_err(|_| EIO)?;
        match resolved.kind.as_deref() {
            Some("file") => {
                let file_id = resolved.entry_id.ok_or(EIO)?;
                Ok(format!("file:{file_id}"))
            }
            Some("hardlink") => {
                let hardlink_id = resolved.entry_id.ok_or(EIO)?;
                let file_id = self
                    .repo
                    .get_hardlink_file_id(hardlink_id)
                    .map_err(|_| EIO)?
                    .ok_or(EIO)?;
                Ok(format!("file:{file_id}"))
            }
            Some("dir") => {
                let dir_id = resolved.entry_id.ok_or(EIO)?;
                Ok(format!("dir:{dir_id}"))
            }
            _ => Ok(format!("path:{path}")),
        }
    }

    fn lock_resource_kind_id(resource_key: &str) -> Result<(String, u64), libc::c_int> {
        let (kind, id) = resource_key.split_once(':').ok_or(EIO)?;
        let resource_id = id.parse::<u64>().map_err(|_| EIO)?;
        Ok((kind.to_string(), resource_id))
    }

    fn lock_backend_is_pg(&self) -> bool {
        matches!(self.lock_backend, LockBackend::PostgresLease) && !self.read_only
    }

    fn lock_records_to_blob(records: &[PosixLockRecord]) -> String {
        records
            .iter()
            .map(|record| {
                format!(
                    "{}\t{}\t{}\t{}",
                    record.owner,
                    record.typ,
                    record.start,
                    record
                        .end
                        .map(|value| value.to_string())
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn lock_records_from_blob(blob: &[u8]) -> Result<Vec<PosixLockRecord>, libc::c_int> {
        let text = String::from_utf8(blob.to_vec()).map_err(|_| EIO)?;
        let mut records = Vec::new();
        for line in text.lines() {
            let mut parts = line.split('\t');
            let owner = parts.next().ok_or(EIO)?.parse::<u64>().map_err(|_| EIO)?;
            let typ = parts.next().ok_or(EIO)?.parse::<i32>().map_err(|_| EIO)?;
            let start = parts.next().ok_or(EIO)?.parse::<u64>().map_err(|_| EIO)?;
            let end = match parts.next().ok_or(EIO)? {
                "" => None,
                value => Some(value.parse::<u64>().map_err(|_| EIO)?),
            };
            if parts.next().is_some() {
                return Err(EIO);
            }
            records.push(PosixLockRecord {
                owner,
                typ,
                start,
                end,
                pid: 0,
            });
        }
        Ok(records)
    }

    fn load_lock_records_for_key(
        &self,
        resource_key: &str,
    ) -> Result<Vec<PosixLockRecord>, libc::c_int> {
        if self.lock_backend_is_pg() {
            let (resource_kind, resource_id) = Self::lock_resource_kind_id(resource_key)?;
            let blob = self.repo.load_lock_range_state_blob(&resource_kind, resource_id).map_err(|err| {
                warn!(
                    "FOD load_lock_records failed resource_key={} resource_kind={} resource_id={} err={}",
                    resource_key,
                    resource_kind,
                    resource_id,
                    err
                );
                EIO
            })?;
            let records = Self::lock_records_from_blob(&blob)?;
            if let Ok(mut guard) = self.posix_locks.lock() {
                if records.is_empty() {
                    guard.remove(resource_key);
                } else {
                    guard.insert(resource_key.to_string(), records.clone());
                }
            }
            Ok(records)
        } else {
            Ok(self
                .posix_locks
                .lock()
                .ok()
                .and_then(|guard| guard.get(resource_key).cloned())
                .unwrap_or_default())
        }
    }

    fn persist_lock_records_for_key(
        &self,
        resource_key: &str,
        owner: u64,
        records: &[PosixLockRecord],
    ) -> Result<(), libc::c_int> {
        if self.lock_backend_is_pg() {
            let (resource_kind, resource_id) = Self::lock_resource_kind_id(resource_key)?;
            let owner_records = records
                .iter()
                .filter(|record| record.owner == owner)
                .cloned()
                .collect::<Vec<_>>();
            let blob = Self::lock_records_to_blob(&owner_records);
            let lease_ttl_seconds = self.lock_lease_ttl.as_secs_f64().ceil().max(1.0) as u64;
            self.repo
                .replace_lock_range_state_blob_for_owner(
                    &resource_kind,
                    resource_id,
                    owner,
                    lease_ttl_seconds,
                    &blob,
                )
                .map_err(|err| {
                    warn!(
                        "FOD persist_lock_records failed resource_key={} resource_kind={} resource_id={} owner={} err={}",
                        resource_key,
                        resource_kind,
                        resource_id,
                        owner,
                        err
                    );
                    EIO
                })?;
        }
        if let Ok(mut guard) = self.posix_locks.lock() {
            if records.is_empty() {
                guard.remove(resource_key);
            } else {
                guard.insert(resource_key.to_string(), records.to_vec());
            }
        }
        Ok(())
    }

    fn register_local_lock_owner(&self, owner: u64) {
        if let Ok(mut guard) = self.local_lock_owners.lock() {
            guard.insert(owner);
        }
        if let Some(session_id) = self.session_id {
            if let Err(err) = self.repo.touch_client_session_owner_key(session_id, owner) {
                warn!(
                    "FOD session owner tracking failed session_id={} owner={} err={}",
                    session_id, owner, err
                );
            }
        }
    }

    fn refresh_local_lock_owner_state(&self, owner: u64) {
        let still_present = self
            .posix_locks
            .lock()
            .ok()
            .map(|guard| {
                guard
                    .values()
                    .any(|records| records.iter().any(|record| record.owner == owner))
            })
            .unwrap_or(false);
        if !still_present {
            if let Ok(mut guard) = self.local_lock_owners.lock() {
                guard.remove(&owner);
            }
        }
    }

    fn clear_locks_for_owner(&self, owner: u64) -> Vec<String> {
        let mut changed = Vec::new();
        if let Ok(mut guard) = self.posix_locks.lock() {
            let keys: Vec<String> = guard
                .iter_mut()
                .filter_map(|(resource_key, records)| {
                    let before = records.len();
                    records.retain(|record| record.owner != owner);
                    if records.len() != before {
                        changed.push(resource_key.clone());
                    }
                    if records.is_empty() {
                        Some(resource_key.clone())
                    } else {
                        None
                    }
                })
                .collect();
            for key in keys {
                guard.remove(&key);
            }
        }
        changed
    }

    fn xattr_owner_for_path(&self, path: &str) -> Result<Option<(String, u64)>, libc::c_int> {
        if path == "/" {
            return Ok(Some(("dir".to_string(), 0)));
        }
        let resolved = self.repo.resolve_path(path).map_err(|_| EIO)?;
        let owner = match resolved.kind.as_deref() {
            Some("file") => resolved.entry_id.map(|id| ("file".to_string(), id)),
            Some("hardlink") => match resolved.entry_id {
                Some(hardlink_id) => self
                    .repo
                    .get_hardlink_file_id(hardlink_id)
                    .map_err(|_| EIO)?
                    .map(|file_id| ("file".to_string(), file_id)),
                None => None,
            },
            Some("dir") => resolved.entry_id.map(|id| ("dir".to_string(), id)),
            Some("symlink") => resolved.entry_id.map(|id| ("symlink".to_string(), id)),
            _ => None,
        };
        Ok(owner)
    }

    fn file_handle_state_for_handle(&self, fh: u64) -> Option<FileHandleState> {
        self.fh_table
            .lock()
            .ok()
            .and_then(|guard| guard.get(&fh).cloned())
    }

    fn file_id_for_handle(&self, fh: u64, ino: u64) -> Result<Option<u64>, libc::c_int> {
        if let Some(state) = self.file_handle_state_for_handle(fh) {
            if let Some(file_id) = state.file_id {
                return Ok(Some(file_id));
            }
            return self.file_id_for_path(&state.path);
        }
        if let Some(path) = self.path_for_inode(ino) {
            return self.file_id_for_path(&path);
        }
        Ok(None)
    }

    fn file_id_for_handle_or_errno(&self, fh: u64, ino: u64) -> Result<u64, libc::c_int> {
        self.file_id_for_handle(fh, ino)?.ok_or(ENOENT)
    }

    fn file_size_for_file_id_or_errno(&self, file_id: u64) -> Result<u64, libc::c_int> {
        match self.repo.file_size(file_id) {
            Ok(Some(size)) => Ok(size),
            Ok(None) => Err(ENOENT),
            Err(_) => Err(EIO),
        }
    }

    fn ioctl_ficlone_source_fd(in_data: &[u8]) -> Result<i64, libc::c_int> {
        match in_data.len() {
            4 => {
                let fd_bytes: [u8; 4] = in_data.try_into().map_err(|_| libc::EINVAL)?;
                Ok(i64::from(i32::from_ne_bytes(fd_bytes)))
            }
            8 => {
                let fd_bytes: [u8; 8] = in_data.try_into().map_err(|_| libc::EINVAL)?;
                Ok(i64::from_ne_bytes(fd_bytes))
            }
            _ => Err(libc::EINVAL),
        }
    }

    fn ioctl_ficlonerange_args(in_data: &[u8]) -> Result<FileCloneRangeIoctl, libc::c_int> {
        if in_data.len() != std::mem::size_of::<FileCloneRangeIoctl>() {
            return Err(libc::EINVAL);
        }
        let src_fd_bytes: [u8; 8] = in_data[0..8].try_into().map_err(|_| libc::EINVAL)?;
        let src_offset_bytes: [u8; 8] = in_data[8..16].try_into().map_err(|_| libc::EINVAL)?;
        let src_length_bytes: [u8; 8] = in_data[16..24].try_into().map_err(|_| libc::EINVAL)?;
        let dest_offset_bytes: [u8; 8] = in_data[24..32].try_into().map_err(|_| libc::EINVAL)?;
        Ok(FileCloneRangeIoctl {
            src_fd: i64::from_ne_bytes(src_fd_bytes),
            src_offset: u64::from_ne_bytes(src_offset_bytes),
            src_length: u64::from_ne_bytes(src_length_bytes),
            dest_offset: u64::from_ne_bytes(dest_offset_bytes),
        })
    }

    fn ioctl_flags_value(in_data: &[u8]) -> Result<u32, libc::c_int> {
        match in_data.len() {
            4 => {
                let flags_bytes: [u8; 4] = in_data.try_into().map_err(|_| libc::EINVAL)?;
                Ok(u32::from_ne_bytes(flags_bytes))
            }
            8 => {
                let flags_bytes: [u8; 8] = in_data.try_into().map_err(|_| libc::EINVAL)?;
                let raw_flags = u64::from_ne_bytes(flags_bytes);
                if raw_flags > u32::MAX as u64 {
                    return Err(libc::EINVAL);
                }
                Ok(raw_flags as u32)
            }
            _ => Err(libc::EINVAL),
        }
    }

    fn ioctl_clone_source_path(req: &Request<'_>, src_fd: i64) -> Result<String, libc::c_int> {
        if src_fd < 0 {
            return Err(libc::EINVAL);
        }
        let fd_path = PathBuf::from(format!("/proc/{}/fd/{}", req.pid(), src_fd));
        let resolved = fs::read_link(&fd_path).map_err(|_| libc::EBADF)?;
        Ok(resolved.to_string_lossy().into_owned())
    }

    fn copy_range_from_states(
        &mut self,
        req_id: u64,
        op: &'static str,
        src_file_id: u64,
        dst_file_id: u64,
        fh_out: u64,
        src_state: Option<WriteState>,
        dst_state: Option<WriteState>,
        src_offset: u64,
        dst_offset: u64,
        len: u64,
        truncate_destination: bool,
    ) -> Result<u32, libc::c_int> {
        let dst_size = if let Some(state) = dst_state.as_ref() {
            state.file_size
        } else {
            match self.file_size_for_file_id_or_errno(dst_file_id) {
                Ok(value) => value,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        op,
                        errno,
                        format!("dst_file_id={} size", dst_file_id),
                    );
                    return Err(errno);
                }
            }
        };
        let src_size = if let Some(state) = src_state.as_ref() {
            state.file_size
        } else {
            match self.file_size_for_file_id_or_errno(src_file_id) {
                Ok(value) => value,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        op,
                        errno,
                        format!("src_file_id={} size", src_file_id),
                    );
                    return Err(errno);
                }
            }
        };
        let Some(bounds) =
            copy_range_bounds(self.block_size, src_offset, dst_offset, len, src_size)
        else {
            if truncate_destination && src_offset == 0 && dst_offset == 0 && src_size == 0 {
                let mut state =
                    dst_state.unwrap_or_else(|| Self::new_write_state(dst_file_id, 0, true));
                state.file_id = dst_file_id;
                state.file_size = 0;
                state.truncate_pending = true;
                state.buffered_bytes = 0;
                state.load_error = false;
                state.clear_payload();
                if let Err(errno) = self.flush_write_state(&mut state) {
                    self.log_request_error(req_id, op, errno, format!("fh_out={} flush", fh_out));
                    return Err(errno);
                }
                if Self::write_state_has_pending_changes(&state) {
                    self.update_write_state(fh_out, state);
                } else {
                    self.remove_write_state(fh_out);
                }
                debug!(
                    "FOD req={} op={} truncated destination for empty source src_file_id={} dst_file_id={}",
                    req_id, op, src_file_id, dst_file_id
                );
            }
            return Ok(0);
        };
        let src_dirty = src_state
            .as_ref()
            .map(Self::write_state_has_pending_changes)
            .unwrap_or(false);
        let dst_dirty = dst_state
            .as_ref()
            .map(Self::write_state_has_pending_changes)
            .unwrap_or(false);
        let adopt_whole_object =
            bounds.can_adopt_whole_object(src_size, dst_size, src_dirty, dst_dirty);
        let CopyRangeBounds {
            src_offset,
            dst_offset,
            copy_len,
            src_end_offset,
            dst_end_offset: _,
            src_first_block,
            src_last_block,
            dst_first_block,
            dst_last_block,
        } = bounds;
        let dedupe_enabled = self.copy_dedupe_enabled_for_len(copy_len);

        if adopt_whole_object {
            match self.repo.adopt_source_data_object(src_file_id, dst_file_id) {
                Ok(true) => {
                    if let Some(state) = dst_state.as_ref() {
                        let mut state = self.clone_write_state_profiled(state);
                        state.file_size = src_size;
                        self.update_write_state(fh_out, state);
                    }
                    debug!(
                        "FOD req={} op={} adopted source data object src_file_id={} dst_file_id={} len={}",
                        req_id, op, src_file_id, dst_file_id, copy_len
                    );
                    return Ok(copy_len as u32);
                }
                Ok(false) => {}
                Err(_) => {
                    self.log_request_error(
                        req_id,
                        op,
                        EIO,
                        format!(
                            "src_file_id={} dst_file_id={} adopt",
                            src_file_id, dst_file_id
                        ),
                    );
                    return Err(EIO);
                }
            }
        }

        let data = if let Some(state) = src_state.as_ref() {
            let mut state = self.clone_write_state_profiled(state);
            match self.read_from_write_state(&mut state, src_offset, copy_len) {
                Ok(data) => data,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        op,
                        errno,
                        format!("src_file_id={} load_write_state", src_file_id),
                    );
                    return Err(errno);
                }
            }
        } else {
            match self.repo.assemble_file_slice(
                src_file_id,
                src_first_block,
                src_last_block,
                src_offset,
                src_end_offset,
                self.block_size,
            ) {
                Ok(data) => data,
                Err(_) => {
                    self.log_request_error(
                        req_id,
                        op,
                        EIO,
                        format!("src_file_id={} assemble", src_file_id),
                    );
                    return Err(EIO);
                }
            }
        };
        let dst_initial_size = dst_state
            .as_ref()
            .map(|state| state.file_size)
            .unwrap_or(dst_size);
        let had_dst_state = dst_state.is_some();
        let mut state = dst_state
            .unwrap_or_else(|| Self::new_write_state(dst_file_id, dst_initial_size, false));
        state.file_id = dst_file_id;
        let current_size = state.file_size;
        let target_end = dst_offset.saturating_add(copy_len);

        if dedupe_enabled && dst_offset < current_size {
            let current = if had_dst_state {
                let mut compare_state = self.clone_write_state_profiled(&state);
                if target_end > compare_state.file_size {
                    compare_state.file_size = target_end;
                }
                self.read_copy_destination_slice(
                    dst_file_id,
                    Some(&mut compare_state),
                    dst_first_block,
                    dst_last_block,
                    dst_offset,
                    copy_len,
                    current_size,
                )
            } else {
                self.read_copy_destination_slice(
                    dst_file_id,
                    None,
                    dst_first_block,
                    dst_last_block,
                    dst_offset,
                    copy_len,
                    current_size,
                )
            };
            let current = match current {
                Ok(data) => data,
                Err(_) => {
                    self.log_request_error(
                        req_id,
                        op,
                        EIO,
                        format!("dst_file_id={} assemble", dst_file_id),
                    );
                    return Err(EIO);
                }
            };
            let runs = pack_copy_skip_unchanged_runs(dst_offset, self.block_size, &data, &current);
            if runs.is_empty() {
                if truncate_destination && target_end != current_size {
                    state.file_size = target_end;
                    state.truncate_pending = true;
                    if let Err(errno) = self.flush_write_state(&mut state) {
                        self.log_request_error(
                            req_id,
                            op,
                            errno,
                            format!("fh_out={} flush", fh_out),
                        );
                        return Err(errno);
                    }
                    self.update_write_state(fh_out, state);
                    debug!(
                        "FOD req={} op={} adjusted size without changed blocks src_file_id={} dst_file_id={} len={}",
                        req_id, op, src_file_id, dst_file_id, copy_len
                    );
                } else if target_end > current_size {
                    state.file_size = target_end;
                    if let Err(errno) = self.flush_write_state(&mut state) {
                        self.log_request_error(
                            req_id,
                            op,
                            errno,
                            format!("fh_out={} flush", fh_out),
                        );
                        return Err(errno);
                    }
                    self.update_write_state(fh_out, state);
                    debug!(
                        "FOD req={} op={} dedupe extended size without changed blocks src_file_id={} dst_file_id={} len={}",
                        req_id, op, src_file_id, dst_file_id, copy_len
                    );
                } else {
                    debug!(
                        "FOD req={} op={} dedupe skipped unchanged blocks src_file_id={} dst_file_id={} len={}",
                        req_id, op, src_file_id, dst_file_id, copy_len
                    );
                }
                return Ok(copy_len as u32);
            }
            if target_end > state.file_size {
                state.file_size = target_end;
            }
            for (run_start, run_payload) in runs {
                if let Err(errno) = self.update_write_buffer(&mut state, run_start, &run_payload) {
                    self.log_request_error(
                        req_id,
                        op,
                        errno,
                        format!("fh_out={} update_write_buffer", fh_out),
                    );
                    return Err(errno);
                }
                state.buffered_bytes = state
                    .buffered_bytes
                    .saturating_add(run_payload.len() as u64);
            }
            if truncate_destination {
                state.truncate_pending = true;
            }
            if let Err(errno) = self.flush_write_state(&mut state) {
                self.log_request_error(req_id, op, errno, format!("fh_out={} flush", fh_out));
                return Err(errno);
            }
            self.update_write_state(fh_out, state);
            debug!(
                "FOD req={} op={} dedupe wrote changed blocks src_file_id={} dst_file_id={} len={}",
                req_id, op, src_file_id, dst_file_id, copy_len
            );
            return Ok(copy_len as u32);
        }

        if let Err(errno) = self.update_write_buffer(&mut state, dst_offset, &data) {
            self.log_request_error(
                req_id,
                op,
                errno,
                format!("fh_out={} update_write_buffer", fh_out),
            );
            return Err(errno);
        }
        state.buffered_bytes = state.buffered_bytes.saturating_add(data.len() as u64);
        if truncate_destination {
            state.truncate_pending = true;
        }
        if let Err(errno) = self.flush_write_state(&mut state) {
            self.log_request_error(req_id, op, errno, format!("fh_out={} flush", fh_out));
            return Err(errno);
        }
        self.update_write_state(fh_out, state);
        debug!(
            "FOD req={} op={} completed src_file_id={} dst_file_id={} len={}",
            req_id, op, src_file_id, dst_file_id, copy_len
        );
        Ok(copy_len as u32)
    }

    fn remove_primary_file_or_promote_hardlink(&self, file_id: u64) -> Result<(), String> {
        match self.repo.count_file_links(file_id) {
            Ok(links) if links > 1 => self
                .repo
                .promote_hardlink_to_primary(file_id)
                .map(|_| ())
                .map_err(|err| {
                    warn!(
                        "FOD promote_hardlink_to_primary failed file_id={} err={}",
                        file_id, err
                    );
                    err
                }),
            Ok(_) => self.repo.purge_primary_file(file_id).map_err(|err| {
                warn!(
                    "FOD purge_primary_file failed file_id={} err={}",
                    file_id, err
                );
                err
            }),
            Err(err) => {
                warn!(
                    "FOD count_file_links failed file_id={} err={}",
                    file_id, err
                );
                Err(err)
            }
        }
    }

    fn ioctl_fsxattr_values(in_data: &[u8]) -> Result<(u32, u32, u32, u32, u32), libc::c_int> {
        if in_data.len() != IOCTL_FSXATTR_BYTES {
            return Err(libc::EINVAL);
        }
        let xflags = u32::from_ne_bytes(in_data[0..4].try_into().map_err(|_| libc::EINVAL)?);
        let extsize = u32::from_ne_bytes(in_data[4..8].try_into().map_err(|_| libc::EINVAL)?);
        let nextents = u32::from_ne_bytes(in_data[8..12].try_into().map_err(|_| libc::EINVAL)?);
        let projid = u32::from_ne_bytes(in_data[12..16].try_into().map_err(|_| libc::EINVAL)?);
        let cowextsize = u32::from_ne_bytes(in_data[16..20].try_into().map_err(|_| libc::EINVAL)?);
        Ok((xflags, extsize, nextents, projid, cowextsize))
    }

    fn entry_path_for_ino(&self, ino: u64) -> Result<String, libc::c_int> {
        if ino == ROOT_INO {
            Ok("/".to_string())
        } else {
            self.path_for_inode(ino).ok_or(ENOENT)
        }
    }

    fn entry_attrs_for_ino(&self, ino: u64) -> Result<(String, ParsedAttrs), libc::c_int> {
        let path = self.entry_path_for_ino(ino)?;
        let attrs = self.lookup_path(&path)?.ok_or(ENOENT)?;
        Ok((path, attrs))
    }

    fn parent_entry_id_for_inode(&self, ino: u64) -> Result<Option<u64>, libc::c_int> {
        if ino == ROOT_INO {
            return Ok(None);
        }
        let path = self.entry_path_for_ino(ino)?;
        let resolved = self.repo.resolve_path(&path).map_err(|_| EIO)?;
        match (resolved.kind.as_deref(), resolved.entry_id) {
            (Some("dir"), entry_id) => Ok(entry_id),
            _ => Err(ENOENT),
        }
    }

    fn remove_cached_path(&self, path: &str) {
        // Ten sam inode moze miec wiecej niz jedna sciezke, np. primary file i hardlink.
        // Usuwamy odwrotna mape inode->path tylko wtedy, gdy nadal wskazuje dokladnie
        // na usuwana sciezke. Inaczej unlink primary kasowal cache hardlinka.
        let removed_ino = self
            .path_to_inode
            .write()
            .ok()
            .and_then(|mut guard| guard.remove(path));

        if let Some(ino) = removed_ino {
            if let Ok(mut inode_guard) = self.inode_to_path.write() {
                let should_remove = inode_guard
                    .get(&ino)
                    .map(|cached_path| cached_path == path)
                    .unwrap_or(false);
                if should_remove {
                    inode_guard.remove(&ino);
                }
            }
        }
    }

    fn remove_cached_handle_paths(&self, path: &str) {
        if let Ok(mut guard) = self.fh_table.lock() {
            guard.retain(|_, state| state.path != path);
        }
    }

    fn move_cached_path(&self, old_path: &str, new_path: &str, ino: u64) {
        self.remove_cached_path(old_path);
        self.register_path(new_path, ino);
        if let Ok(mut guard) = self.fh_table.lock() {
            for state in guard.values_mut() {
                if state.path == old_path {
                    state.path = new_path.to_string();
                }
            }
        }
    }

    fn attrs_for_path(&self, path: &str) -> Result<Option<ParsedAttrs>, libc::c_int> {
        let blob = self.repo.fetch_path_attrs_blob(path).map_err(|_| EIO)?;
        let Some(blob) = blob else {
            return Ok(None);
        };
        let fields = Self::decode_nul_fields(&blob);
        if fields.is_empty() {
            return Ok(None);
        }
        let obj_type = fields[0].clone();
        let process_identity = Self::process_identity();
        let file_attr = if obj_type == "symlink" {
            if fields.len() < 9 {
                return Err(EIO);
            }
            let raw_inode = fields[1].parse::<u64>().map_err(|_| EIO)?;
            let target = fields[2].clone();
            let mod_date = fields[3].clone();
            let acc_date = fields[4].clone();
            let chg_date = fields[5].clone();
            let uid = fields[6].parse::<u32>().unwrap_or(process_identity.uid);
            let gid = fields[7].parse::<u32>().unwrap_or(process_identity.gid);
            let inode_seed = fields[8].clone();
            let inode = self.stable_inode(&obj_type, &inode_seed, raw_inode);
            FileAttr {
                ino: inode,
                size: target.len() as u64,
                blocks: self.block_count(target.len() as u64, "symlink"),
                atime: Self::parse_time(&acc_date),
                mtime: Self::parse_time(&mod_date),
                ctime: Self::parse_time(&chg_date),
                crtime: Self::parse_time(&chg_date),
                kind: FileType::Symlink,
                perm: 0o777,
                nlink: 1,
                uid,
                gid,
                rdev: 0,
                flags: 0,
                blksize: self.block_size as u32,
            }
        } else {
            if fields.len() < 10 {
                return Err(EIO);
            }
            let raw_inode = fields[1].parse::<u64>().map_err(|_| EIO)?;
            let size = fields[2].parse::<u64>().unwrap_or(0);
            let mode = fields[3].clone();
            let mod_date = fields[4].clone();
            let acc_date = fields[5].clone();
            let chg_date = fields[6].clone();
            let uid = fields[7].parse::<u32>().unwrap_or(process_identity.uid);
            let gid = fields[8].parse::<u32>().unwrap_or(process_identity.gid);
            let inode_seed = fields[9].clone();
            let mut kind = if obj_type == "dir" {
                FileType::Directory
            } else {
                FileType::RegularFile
            };
            let mut perm =
                u16::from_str_radix(mode.trim_start_matches("0o"), 8).unwrap_or(0o644) as u16;
            let mut rdev = 0;
            if obj_type == "hardlink" {
                let file_id = self.repo.get_hardlink_file_id(raw_inode).map_err(|_| EIO)?;
                if let Some(file_id) = file_id {
                    if let Some((special_type, major, minor)) = self
                        .repo
                        .get_special_file_metadata(file_id)
                        .map_err(|_| EIO)?
                    {
                        kind = Self::file_type_from_special(&special_type);
                        rdev = ((major as u64) << 8) | (minor as u64);
                    }
                }
            } else if let Some((special_type, major, minor)) = self
                .repo
                .get_special_file_metadata(raw_inode)
                .map_err(|_| EIO)?
            {
                kind = Self::file_type_from_special(&special_type);
                rdev = ((major as u64) << 8) | (minor as u64);
            }
            if obj_type == "dir" && perm == 0o644 {
                perm = 0o755;
            }
            let inode = self.stable_inode(&obj_type, &inode_seed, raw_inode);
            let nlink = match obj_type.as_str() {
                "hardlink" => {
                    let file_id = self.repo.get_hardlink_file_id(raw_inode).map_err(|_| EIO)?;
                    let file_id = file_id.unwrap_or(raw_inode);
                    self.repo.count_file_links(file_id).map_err(|_| EIO)?
                }
                "file" => self.repo.count_file_links(raw_inode).map_err(|_| EIO)?,
                "dir" => {
                    2 + self
                        .repo
                        .count_directory_subdirs(raw_inode)
                        .map_err(|_| EIO)?
                }
                _ => 1,
            };
            FileAttr {
                ino: inode,
                size,
                blocks: self.block_count(size, &obj_type),
                atime: Self::parse_time(&acc_date),
                mtime: Self::parse_time(&mod_date),
                ctime: Self::parse_time(&chg_date),
                crtime: Self::parse_time(&chg_date),
                kind,
                perm,
                nlink: nlink.try_into().unwrap_or(u32::MAX),
                uid,
                gid,
                rdev: rdev.try_into().unwrap_or(u32::MAX),
                flags: 0,
                blksize: self.block_size as u32,
            }
        };
        Ok(Some(ParsedAttrs { file_attr }))
    }

    fn block_count(&self, size: u64, kind: &str) -> u64 {
        if kind == "dir" {
            return 1;
        }
        let block_size = self.block_size.max(1);
        1 + size.saturating_sub(1) / block_size
    }

    fn lookup_path(&self, path: &str) -> Result<Option<ParsedAttrs>, libc::c_int> {
        let path = Self::normalize_path(path);
        if path == "/" {
            return Ok(Some(self.root_attr()));
        }
        self.attrs_for_path(&path)
    }

    fn root_attr(&self) -> ParsedAttrs {
        let now = SystemTime::now();
        let child_dirs = self.repo.count_root_directory_children().unwrap_or(0);
        let process_identity = Self::process_identity();
        ParsedAttrs {
            file_attr: FileAttr {
                ino: ROOT_INO,
                size: 0,
                blocks: self.block_count(0, "dir"),
                atime: now,
                mtime: now,
                ctime: now,
                crtime: now,
                kind: FileType::Directory,
                perm: 0o755,
                nlink: (2 + child_dirs).try_into().unwrap_or(u32::MAX),
                uid: process_identity.uid,
                gid: process_identity.gid,
                rdev: 0,
                flags: 0,
                blksize: self.block_size as u32,
            },
        }
    }

    fn inode_for_path(&self, path: &str) -> Option<u64> {
        self.path_to_inode
            .read()
            .ok()
            .and_then(|cache| cache.get(path).copied())
    }

    fn path_for_inode(&self, ino: u64) -> Option<String> {
        self.inode_to_path
            .read()
            .ok()
            .and_then(|cache| cache.get(&ino).cloned())
    }

    fn register_path(&self, path: &str, ino: u64) {
        if let Ok(mut cache) = self.path_to_inode.write() {
            cache.insert(path.to_string(), ino);
        }
        if let Ok(mut cache) = self.inode_to_path.write() {
            cache.insert(ino, path.to_string());
        }
    }

    fn touch_access_time(&self, path: &str, file_attr: &FileAttr) {
        if !should_update_atime(
            self.atime_policy,
            file_attr.kind == FileType::Directory,
            file_attr,
        ) {
            return;
        }
        if self.atime_policy == AtimePolicy::Default {
            if let Ok(age) = Self::current_time().duration_since(file_attr.atime) {
                if age < ATIME_TOUCH_INTERVAL {
                    return;
                }
            }
        }
        let atime = Self::system_time_to_db_string(Self::current_time());
        let result = match file_attr.kind {
            FileType::Directory => {
                let (kind, entry_id) = match self.resolved_entry_for_path(path) {
                    Ok(value) => value,
                    Err(errno) => {
                        warn!("FOD atime touch skipped path={} errno={}", path, errno);
                        return;
                    }
                };
                match (kind.as_deref(), entry_id) {
                    (Some("dir"), Some(directory_id)) => {
                        self.repo.update_directory_access_date(directory_id, &atime)
                    }
                    _ if path == "/" => Ok(()),
                    _ => Ok(()),
                }
            }
            FileType::Symlink => match self.repo.resolve_path(path) {
                Ok(resolved) if resolved.kind.as_deref() == Some("symlink") => match resolved
                    .entry_id
                {
                    Some(symlink_id) => self.repo.update_symlink_access_date(symlink_id, &atime),
                    None => Ok(()),
                },
                Ok(_) => Ok(()),
                Err(_) => Ok(()),
            },
            _ => {
                let file_id = match self.file_id_for_path(path) {
                    Ok(Some(value)) => value,
                    Ok(None) => return,
                    Err(errno) => {
                        warn!("FOD atime touch skipped path={} errno={}", path, errno);
                        return;
                    }
                };
                self.repo.update_file_access_date(file_id, &atime)
            }
        };
        if let Err(err) = result {
            warn!("FOD atime touch failed path={} err={}", path, err);
        }
    }

    fn fopen_flags(&self) -> u32 {
        // Domyslnie nie wymuszamy direct_io.
        // direct_io jest trybem diagnostycznym/zgodnosciowym, bo potrafi mocno spowolnic
        // sekwencyjny zapis malymi blokami przez ominiecie cache/writeback kernela.
        if self.fopen_direct_io {
            FOD_FOPEN_DIRECT_IO
        } else {
            0
        }
    }

    fn next_handle(&self) -> u64 {
        let mut guard = self.next_fh.lock().unwrap();
        let fh = *guard;
        *guard += 1;
        fh
    }

    fn current_time() -> SystemTime {
        SystemTime::now()
    }

    fn system_time_to_db_string(value: SystemTime) -> String {
        let dt = DateTime::<Utc>::from(value);
        dt.format("%Y-%m-%d %H:%M:%S%.f").to_string()
    }

    fn time_or_now_to_db_string(value: Option<TimeOrNow>) -> Option<String> {
        match value {
            Some(TimeOrNow::SpecificTime(time)) => Some(Self::system_time_to_db_string(time)),
            Some(TimeOrNow::Now) => Some(Self::system_time_to_db_string(Self::current_time())),
            None => None,
        }
    }

    fn create_handle_for_file(&self, path: String, file_id: Option<u64>, flags: i32) -> u64 {
        let fh = self.next_handle();
        if let Ok(mut guard) = self.fh_table.lock() {
            guard.insert(
                fh,
                FileHandleState {
                    path,
                    file_id,
                    flags,
                    atime_touched: false,
                },
            );
        }
        fh
    }

    fn open_handle_count_for_file(&self, file_id: u64) -> usize {
        self.fh_table
            .lock()
            .map(|guard| {
                guard
                    .values()
                    .filter(|state| state.file_id == Some(file_id))
                    .count()
            })
            .unwrap_or(0)
    }

    fn remove_handle_state(&self, fh: u64) {
        if let Ok(mut guard) = self.fh_table.lock() {
            guard.remove(&fh);
        }
        if let Ok(mut guard) = self.write_states.lock() {
            guard.remove(&fh);
        }
    }

    pub fn register_client_session(
        &mut self,
        mountpoint: &Path,
        mount_mode: &str,
    ) -> Result<(), String> {
        if self.session_id.is_some() || !self.lock_backend_is_pg() {
            return Ok(());
        }
        let host_name = std::env::var("HOSTNAME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "unknown".to_string());
        let mountpoint = mountpoint.to_string_lossy().to_string();
        let pid = u64::from(std::process::id());
        let lease_ttl_seconds = self.lock_lease_ttl.as_secs_f64().ceil().max(1.0) as u64;
        let session_id = self.repo.register_client_session(
            &host_name,
            &mountpoint,
            mount_mode,
            self.lock_backend.as_str(),
            pid,
            lease_ttl_seconds,
        )?;
        self.session_id = Some(session_id);
        Ok(())
    }

    pub fn start_lock_heartbeat(&mut self) -> Result<(), String> {
        if self.lock_heartbeat.is_some() || !self.lock_backend_is_pg() {
            return Ok(());
        }
        let Some(session_id) = self.session_id else {
            return Ok(());
        };
        if self.lock_heartbeat_interval.is_zero() {
            return Ok(());
        }
        let handle = LockHeartbeatHandle::spawn(
            self.repo.clone(),
            session_id,
            Arc::clone(&self.posix_locks),
            Arc::clone(&self.local_lock_owners),
            self.lock_heartbeat_interval,
            self.lock_lease_ttl,
        )?;
        self.lock_heartbeat = Some(handle);
        Ok(())
    }

    pub fn start_runtime_reload(&mut self, base_runtime: &RuntimeConfig) -> Result<(), String> {
        if self.runtime_reload.is_some() {
            return Ok(());
        }
        let handle = RuntimeReloadHandle::spawn(
            self.repo.clone(),
            base_runtime.clone(),
            Arc::clone(&self.reloadable_runtime),
        )?;
        self.runtime_reload = Some(handle);
        Ok(())
    }
}

impl Drop for FodFuse {
    fn drop(&mut self) {
        if self.profile.has_activity() {
            info!("FOD boundary profile:");
            for line in self.profile.snapshot_lines() {
                info!("  {}", line);
            }
        }
    }
}

impl Filesystem for FodFuse {
    fn init(&mut self, _req: &Request<'_>, config: &mut KernelConfig) -> Result<(), libc::c_int> {
        let _ = config.add_capabilities(FUSE_POSIX_LOCKS | FUSE_FLOCK_LOCKS);
        Ok(())
    }

    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let req_id = self.next_request_id();
        let Some(parent_path) = self.path_for_inode(parent) else {
            self.log_request_error(
                req_id,
                "lookup",
                ENOENT,
                format!("parent={} name={}", parent, name.to_string_lossy()),
            );
            reply.error(ENOENT);
            return;
        };
        let child_path = Self::join_path(&parent_path, name);
        self.log_request_start(
            req_id,
            "lookup",
            format!("parent={} child={}", parent_path, child_path),
        );
        match self.lookup_path(&child_path) {
            Ok(Some(attrs)) => {
                self.register_path(&child_path, attrs.file_attr.ino);
                reply.entry(&self.metadata_cache_ttl_live(), &attrs.file_attr, 0);
            }
            Ok(None) => reply.error(ENOENT),
            Err(errno) => reply.error(errno),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        let req_id = self.next_request_id();
        self.log_request_start(req_id, "getattr", format!("ino={}", ino));
        match self.entry_attrs_for_ino(ino) {
            Ok((path, attrs)) => {
                self.register_path(&path, attrs.file_attr.ino);
                reply.attr(&self.metadata_cache_ttl_live(), &attrs.file_attr);
            }
            Err(errno) => {
                self.log_request_error(req_id, "getattr", errno, format!("ino={}", ino));
                reply.error(errno)
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let req_id = self.next_request_id();
        let path = if ino == ROOT_INO {
            "/".to_string()
        } else {
            match self.path_for_inode(ino) {
                Some(path) => path,
                None => {
                    self.log_request_error(
                        req_id,
                        "readdir",
                        ENOENT,
                        format!("ino={} offset={}", ino, offset),
                    );
                    reply.error(ENOENT);
                    return;
                }
            }
        };
        self.log_request_start(
            req_id,
            "readdir",
            format!("path={} ino={} offset={}", path, ino, offset),
        );
        let current_attrs = if path == "/" {
            None
        } else {
            match self.lookup_path(&path) {
                Ok(Some(attrs)) => Some(attrs.file_attr),
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "readdir",
                        errno,
                        format!("path={} attrs skipped", path),
                    );
                    None
                }
                Ok(None) => None,
            }
        };
        let blob = match self.repo.list_directory_entries_blob(&path) {
            Ok(Some(blob)) => blob,
            Ok(None) => {
                debug!("FOD readdir path={} empty", path);
                if let Some(attrs) = current_attrs.as_ref() {
                    self.touch_access_time(&path, attrs);
                }
                reply.ok();
                return;
            }
            Err(_) => {
                reply.error(EIO);
                return;
            }
        };
        let entries = Self::decode_nul_fields(&blob);
        debug!("FOD readdir path={} entries={}", path, entries.len());
        let mut next_offset = 1i64;
        if offset == 0 {
            let _ = reply.add(ino, 1, FileType::Directory, ".");
            let parent_ino = if ino == ROOT_INO {
                ROOT_INO
            } else {
                let parent_path = path
                    .rsplit_once('/')
                    .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
                    .unwrap_or("/");
                self.inode_for_path(parent_path).unwrap_or(ROOT_INO)
            };
            let _ = reply.add(parent_ino, 2, FileType::Directory, "..");
            next_offset = 2;
        }
        for (index, name) in entries.into_iter().enumerate().skip(offset.max(0) as usize) {
            let child_path = Self::join_path(&path, OsStr::from_bytes(name.as_bytes()));
            match self.lookup_path(&child_path) {
                Ok(Some(attrs)) => {
                    self.register_path(&child_path, attrs.file_attr.ino);
                    let kind = attrs.file_attr.kind;
                    let added = reply.add(attrs.file_attr.ino, (index + 3) as i64, kind, name);
                    if added {
                        break;
                    }
                }
                _ => {
                    continue;
                }
            }
            next_offset = (index + 3) as i64;
        }
        let _ = next_offset;
        if let Some(attrs) = current_attrs.as_ref() {
            self.touch_access_time(&path, attrs);
        }
        reply.ok();
    }

    fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
        let _read_profile = self.start_fuse_read_profile();
        let path = match self.path_for_inode(ino) {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        match self.repo.resolve_path(&path) {
            Ok(resolved) => {
                if let Some("symlink") = resolved.kind.as_deref() {
                    let symlink_id = match resolved.entry_id {
                        Some(value) => value,
                        None => {
                            reply.error(ENOENT);
                            return;
                        }
                    };
                    match self.repo.load_symlink_target(symlink_id) {
                        Ok(Some(target)) => {
                            if let Ok(Some(attrs)) = self.lookup_path(&path) {
                                self.touch_access_time(&path, &attrs.file_attr);
                            }
                            self.reply_data_profiled(reply, target.as_bytes())
                        }
                        Ok(None) => reply.error(ENOENT),
                        Err(_) => reply.error(EIO),
                    }
                } else {
                    reply.error(ENOENT);
                }
            }
            Err(_) => reply.error(EIO),
        }
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
        let snapshot = self.cached_statfs_snapshot();
        let total_blocks = self
            .statfs_capacity_bytes()
            .map(|bytes| (bytes + self.block_size.saturating_sub(1)) / self.block_size)
            .unwrap_or(snapshot.blocks);
        let used_blocks = snapshot.blocks.min(total_blocks);
        let free_blocks = total_blocks.saturating_sub(used_blocks);
        debug!(
            "FOD statfs blocks={} used_blocks={} files={} dirs={} total_data_size={} block_size={} max_fs_size_bytes={:?} pg_visible_path={:?}",
            total_blocks,
            used_blocks,
            snapshot.files,
            snapshot.dirs,
            snapshot.total_data_size,
            self.block_size,
            self.max_fs_size_bytes,
            self.pg_visible_path.as_deref().map(|path| path.display().to_string())
        );
        reply.statfs(
            total_blocks,
            free_blocks,
            free_blocks,
            snapshot.files + snapshot.dirs,
            0,
            self.block_size as u32,
            self.block_size as u32,
            255,
        );
    }

    fn setxattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        name: &OsStr,
        value: &[u8],
        flags: i32,
        position: u32,
        reply: ReplyEmpty,
    ) {
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        if position != 0 {
            reply.error(libc::EOPNOTSUPP);
            return;
        }
        let path = match self.entry_path_for_ino(ino) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let name = name.to_string_lossy().to_string();
        debug!(
            "FOD setxattr path={} name={} flags={} size={}",
            path,
            name,
            flags,
            value.len()
        );
        if !self.selinux_enabled && name == "security.selinux" {
            warn!(
                "FOD security.selinux xattr rejected because SELinux support is disabled path={}",
                path
            );
            reply.error(libc::EOPNOTSUPP);
            return;
        }
        if !self.acl_enabled && name == "system.posix_acl_access" {
            warn!(
                "FOD system.posix_acl_access xattr rejected because ACL support is disabled path={}",
                path
            );
            reply.error(libc::EOPNOTSUPP);
            return;
        }
        let owner = match self.xattr_owner_for_path(&path) {
            Ok(Some(owner)) => owner,
            Ok(None) => {
                reply.error(ENOENT);
                return;
            }
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        if flags & libc::XATTR_CREATE != 0 {
            if self
                .repo
                .fetch_xattr_value(&path, &name)
                .ok()
                .flatten()
                .is_some()
            {
                reply.error(libc::EEXIST);
                return;
            }
        }
        if flags & libc::XATTR_REPLACE != 0 {
            if self
                .repo
                .fetch_xattr_value(&path, &name)
                .ok()
                .flatten()
                .is_none()
            {
                reply.error(libc::ENODATA);
                return;
            }
        }
        let result = self
            .repo
            .store_xattr_value_for_owner(&owner.0, owner.1, &name, value);
        match result {
            Ok(_) => {
                debug!("FOD setxattr stored path={} name={}", path, name);
                reply.ok()
            }
            Err(_) => {
                warn!("FOD setxattr failed path={} name={}", path, name);
                reply.error(EIO)
            }
        }
    }

    fn getxattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        name: &OsStr,
        size: u32,
        reply: ReplyXattr,
    ) {
        let _read_profile = self.start_fuse_read_profile();
        let path = match self.entry_path_for_ino(ino) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let name = name.to_string_lossy().to_string();
        debug!("FOD getxattr path={} name={} size={}", path, name, size);
        match self.repo.fetch_xattr_value(&path, &name) {
            Ok(Some(value)) => {
                if size == 0 {
                    reply.size(value.len() as u32);
                } else if size < value.len() as u32 {
                    reply.error(libc::ERANGE);
                } else {
                    self.reply_xattr_profiled(reply, &value);
                }
            }
            Ok(None) => {
                debug!("FOD getxattr missing path={} name={}", path, name);
                reply.error(libc::ENODATA)
            }
            Err(_) => {
                warn!("FOD getxattr failed path={} name={}", path, name);
                reply.error(EIO)
            }
        }
    }

    fn listxattr(&mut self, _req: &Request<'_>, ino: u64, size: u32, reply: ReplyXattr) {
        let _read_profile = self.start_fuse_read_profile();
        let path = match self.entry_path_for_ino(ino) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let owner = match self.xattr_owner_for_path(&path) {
            Ok(Some(owner)) => owner,
            Ok(None) => {
                reply.size(0);
                return;
            }
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        debug!(
            "FOD listxattr path={} owner_kind={} owner_id={}",
            path, owner.0, owner.1
        );
        match self.repo.list_xattr_names_for_owner(&owner.0, owner.1) {
            Ok(names) => {
                let mut payload = Vec::new();
                for name in names {
                    payload.extend_from_slice(name.as_bytes());
                    payload.push(0);
                }
                if size == 0 {
                    reply.size(payload.len() as u32);
                } else if size < payload.len() as u32 {
                    reply.error(libc::ERANGE);
                } else {
                    self.reply_xattr_profiled(reply, &payload);
                }
            }
            Err(_) => {
                warn!("FOD listxattr failed path={}", path);
                reply.error(EIO)
            }
        }
    }

    fn removexattr(&mut self, _req: &Request<'_>, ino: u64, name: &OsStr, reply: ReplyEmpty) {
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        let path = match self.entry_path_for_ino(ino) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let owner = match self.xattr_owner_for_path(&path) {
            Ok(Some(owner)) => owner,
            Ok(None) => {
                reply.error(ENOENT);
                return;
            }
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let name = name.to_string_lossy().to_string();
        debug!("FOD removexattr path={} name={}", path, name);
        match self.repo.remove_xattr_for_owner(&owner.0, owner.1, &name) {
            Ok(deleted) if deleted > 0 => {
                debug!("FOD removexattr removed path={} name={}", path, name);
                reply.ok()
            }
            Ok(_) => {
                debug!("FOD removexattr missing path={} name={}", path, name);
                reply.error(libc::ENODATA)
            }
            Err(_) => {
                warn!("FOD removexattr failed path={} name={}", path, name);
                reply.error(EIO)
            }
        }
    }

    fn access(&mut self, req: &Request<'_>, ino: u64, mask: i32, reply: ReplyEmpty) {
        let subject = self.request_identity(req);
        let (path, attrs) = match self.entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        debug!("FOD access path={} mask={:#x}", path, mask);
        match self.acl_allows(&path, &attrs.file_attr, mask, &subject) {
            Ok(true) => {
                debug!("FOD access granted path={} mask={:#x}", path, mask);
                reply.ok()
            }
            Ok(false) => {
                debug!("FOD access denied path={} mask={:#x}", path, mask);
                reply.error(libc::EACCES)
            }
            Err(errno) => {
                warn!(
                    "FOD access check failed path={} mask={:#x} errno={}",
                    path, mask, errno
                );
                reply.error(errno)
            }
        }
    }

    fn ioctl(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        _flags: u32,
        cmd: u32,
        in_data: &[u8],
        out_size: u32,
        reply: ReplyIoctl,
    ) {
        let (path, attrs) = match self.entry_attrs_for_ino(ino) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        match cmd {
            value if value == IOCTL_FIGETBSZ => {
                let block_size = attrs.file_attr.blksize;
                debug!(
                    "FOD ioctl path={} fh={} cmd={} out_size={} blksize={}",
                    path, fh, cmd, out_size, block_size
                );
                reply.ioctl(0, &block_size.to_ne_bytes());
            }
            value
                if value == libc::FS_IOC_GETFLAGS as u32
                    || value == libc::FS_IOC32_GETFLAGS as u32 =>
            {
                // FOD does not persist inode flags yet, so return the default zero bitset.
                let flags: u32 = 0;
                debug!(
                    "FOD ioctl path={} fh={} cmd={} out_size={} flags={:#x}",
                    path, fh, cmd, out_size, flags
                );
                reply.ioctl(0, &flags.to_ne_bytes());
            }
            value
                if value == libc::FS_IOC_SETFLAGS as u32
                    || value == libc::FS_IOC32_SETFLAGS as u32 =>
            {
                let requested_flags = match Self::ioctl_flags_value(in_data) {
                    Ok(value) => value,
                    Err(errno) => {
                        reply.error(errno);
                        return;
                    }
                };
                debug!(
                    "FOD ioctl path={} fh={} cmd={} out_size={} requested_flags={:#x}",
                    path, fh, cmd, out_size, requested_flags
                );
                if requested_flags != 0 {
                    warn!(
                        "FOD FS_IOC_SETFLAGS rejected path={} requested_flags={:#x}: inode flags are not persisted yet",
                        path, requested_flags
                    );
                    reply.error(libc::EOPNOTSUPP);
                    return;
                }
                reply.ioctl(0, &[]);
            }
            value if value == IOCTL_FS_IOC_FSGETXATTR => {
                let fsxattr = [0u8; IOCTL_FSXATTR_BYTES];
                debug!(
                    "FOD ioctl path={} fh={} cmd={} out_size={} fsxattr=zeroed",
                    path, fh, cmd, out_size
                );
                reply.ioctl(0, &fsxattr);
            }
            value if value == IOCTL_FS_IOC_FSSETXATTR => {
                let (xflags, extsize, nextents, projid, cowextsize) =
                    match Self::ioctl_fsxattr_values(in_data) {
                        Ok(values) => values,
                        Err(errno) => {
                            reply.error(errno);
                            return;
                        }
                    };
                debug!(
                    "FOD ioctl path={} fh={} cmd={} out_size={} xflags={:#x} extsize={} nextents={} projid={} cowextsize={}",
                    path, fh, cmd, out_size, xflags, extsize, nextents, projid, cowextsize
                );
                if xflags != 0
                    || extsize != 0
                    || nextents != 0
                    || projid != 0
                    || cowextsize != 0
                    || in_data[20..].iter().any(|byte| *byte != 0)
                {
                    warn!(
                        "FOD FS_IOC_FSSETXATTR rejected path={} xflags={:#x} extsize={} nextents={} projid={} cowextsize={}: fsxattr is not persisted yet",
                        path, xflags, extsize, nextents, projid, cowextsize
                    );
                    reply.error(libc::EOPNOTSUPP);
                    return;
                }
                reply.ioctl(0, &[]);
            }
            value if value == libc::FIONREAD as u32 => {
                let size = match self.write_state_for_handle(fh) {
                    Some(state) => state.file_size,
                    None => attrs.file_attr.size,
                };
                debug!(
                    "FOD ioctl path={} fh={} cmd={} size={}",
                    path, fh, cmd, size
                );
                if attrs.file_attr.kind != FileType::RegularFile {
                    reply.error(ENOTTY);
                    return;
                }
                let available = size.min(u32::MAX as u64) as u32;
                reply.ioctl(0, &available.to_ne_bytes());
            }
            value if value == libc::FICLONE as u32 => {
                if attrs.file_attr.kind != FileType::RegularFile {
                    reply.error(ENOTTY);
                    return;
                }
                let src_fd = match Self::ioctl_ficlone_source_fd(in_data) {
                    Ok(value) => value,
                    Err(errno) => {
                        reply.error(errno);
                        return;
                    }
                };
                let src_path = match Self::ioctl_clone_source_path(req, src_fd) {
                    Ok(value) => value,
                    Err(errno) => {
                        reply.error(errno);
                        return;
                    }
                };
                let src_file_id = match self.file_id_for_path(&src_path) {
                    Ok(Some(value)) => value,
                    Ok(None) => {
                        reply.error(libc::EXDEV);
                        return;
                    }
                    Err(errno) => {
                        reply.error(errno);
                        return;
                    }
                };
                let dst_file_id = match self.file_id_for_handle(fh, ino) {
                    Ok(Some(value)) => value,
                    Ok(None) => {
                        reply.error(ENOENT);
                        return;
                    }
                    Err(errno) => {
                        reply.error(errno);
                        return;
                    }
                };
                if src_file_id == dst_file_id {
                    reply.error(libc::EINVAL);
                    return;
                }
                if let Err(errno) =
                    self.flush_pending_write_states_for_file_except(src_file_id, u64::MAX)
                {
                    reply.error(errno);
                    return;
                }
                if let Err(errno) = self.flush_pending_write_states_for_file_except(dst_file_id, fh)
                {
                    reply.error(errno);
                    return;
                }
                let dst_state = self.write_state_for_handle(fh);
                let src_size = match self.file_size_for_file_id_or_errno(src_file_id) {
                    Ok(value) => value,
                    Err(errno) => {
                        reply.error(errno);
                        return;
                    }
                };
                let req_id = self.next_request_id();
                self.log_request_start(
                    req_id,
                    "ioctl_ficlone",
                    format!(
                        "src_path={} src_file_id={} dst_path={} dst_file_id={} src_fd={} len={}",
                        src_path, src_file_id, path, dst_file_id, src_fd, src_size
                    ),
                );
                match self.copy_range_from_states(
                    req_id,
                    "ioctl_ficlone",
                    src_file_id,
                    dst_file_id,
                    fh,
                    None,
                    dst_state,
                    0,
                    0,
                    src_size,
                    true,
                ) {
                    Ok(_) => reply.ioctl(0, &[]),
                    Err(errno) => reply.error(errno),
                }
            }
            value if value == libc::FICLONERANGE as u32 => {
                if attrs.file_attr.kind != FileType::RegularFile {
                    reply.error(ENOTTY);
                    return;
                }
                let args = match Self::ioctl_ficlonerange_args(in_data) {
                    Ok(value) => value,
                    Err(errno) => {
                        reply.error(errno);
                        return;
                    }
                };
                let src_path = match Self::ioctl_clone_source_path(req, args.src_fd) {
                    Ok(value) => value,
                    Err(errno) => {
                        reply.error(errno);
                        return;
                    }
                };
                let src_file_id = match self.file_id_for_path(&src_path) {
                    Ok(Some(value)) => value,
                    Ok(None) => {
                        reply.error(libc::EXDEV);
                        return;
                    }
                    Err(errno) => {
                        reply.error(errno);
                        return;
                    }
                };
                let dst_file_id = match self.file_id_for_handle(fh, ino) {
                    Ok(Some(value)) => value,
                    Ok(None) => {
                        reply.error(ENOENT);
                        return;
                    }
                    Err(errno) => {
                        reply.error(errno);
                        return;
                    }
                };
                if src_file_id == dst_file_id {
                    reply.error(libc::EINVAL);
                    return;
                }
                if let Err(errno) =
                    self.flush_pending_write_states_for_file_except(src_file_id, u64::MAX)
                {
                    reply.error(errno);
                    return;
                }
                if let Err(errno) = self.flush_pending_write_states_for_file_except(dst_file_id, fh)
                {
                    reply.error(errno);
                    return;
                }
                let dst_state = self.write_state_for_handle(fh);
                let src_size = match self.file_size_for_file_id_or_errno(src_file_id) {
                    Ok(value) => value,
                    Err(errno) => {
                        reply.error(errno);
                        return;
                    }
                };
                let src_offset = args.src_offset;
                let len = if args.src_length == 0 {
                    src_size.saturating_sub(src_offset)
                } else {
                    args.src_length.min(src_size.saturating_sub(src_offset))
                };
                let req_id = self.next_request_id();
                self.log_request_start(
                    req_id,
                    "ioctl_ficlonerange",
                    format!(
                        "src_path={} src_file_id={} dst_path={} dst_file_id={} src_fd={} src_offset={} dest_offset={} len={}",
                        src_path,
                        src_file_id,
                        path,
                        dst_file_id,
                        args.src_fd,
                        src_offset,
                        args.dest_offset,
                        len
                    ),
                );
                match self.copy_range_from_states(
                    req_id,
                    "ioctl_ficlonerange",
                    src_file_id,
                    dst_file_id,
                    fh,
                    None,
                    dst_state,
                    src_offset,
                    args.dest_offset,
                    len,
                    false,
                ) {
                    Ok(_) => reply.ioctl(0, &[]),
                    Err(errno) => reply.error(errno),
                }
            }
            _ => reply.error(ENOTTY),
        }
    }

    fn poll(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        _kh: u64,
        _events: u32,
        _flags: u32,
        reply: ReplyPoll,
    ) {
        let (path, attrs) = match self.entry_attrs_for_ino(ino) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        if attrs.file_attr.kind != FileType::RegularFile {
            reply.error(ENOTTY);
            return;
        }
        let size = self
            .write_state_for_handle(fh)
            .map(|state| state.file_size)
            .unwrap_or(attrs.file_attr.size);
        let mut revents = 0u32;
        if size > 0 {
            revents |= POLLIN as u32;
        }
        if !self.read_only {
            revents |= POLLOUT as u32;
        }
        debug!(
            "FOD poll path={} fh={} size={} revents={:#x}",
            path, fh, size, revents
        );
        reply.poll(revents);
    }

    fn open(&mut self, req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        let req_id = self.next_request_id();
        let subject = self.request_identity(req);
        let (path, attrs) = match self.entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "open",
                    errno,
                    format!("ino={} flags={:#x}", ino, flags),
                );
                reply.error(errno);
                return;
            }
        };
        self.log_request_start(
            req_id,
            "open",
            format!("path={} ino={} flags={:#x}", path, ino, flags),
        );
        let file_id = match self.file_id_for_path(&path) {
            Ok(Some(value)) => value,
            Ok(None) => {
                reply.error(ENOENT);
                return;
            }
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let attrs = attrs.file_attr;
        let access_mode = match flags & libc::O_ACCMODE {
            libc::O_WRONLY => libc::W_OK,
            libc::O_RDWR => libc::R_OK | libc::W_OK,
            _ => libc::R_OK,
        };
        if !self
            .acl_allows(&path, &attrs, access_mode, &subject)
            .unwrap_or(false)
        {
            self.log_request_error(
                req_id,
                "open",
                libc::EACCES,
                format!("path={} denied_by_acl flags={:#x}", path, flags),
            );
            reply.error(libc::EACCES);
            return;
        }
        if self.read_only && (flags & libc::O_ACCMODE) != libc::O_RDONLY {
            self.log_request_error(
                req_id,
                "open",
                libc::EROFS,
                format!("path={} read_only flags={:#x}", path, flags),
            );
            reply.error(libc::EROFS);
            return;
        }
        let writable = (flags & libc::O_ACCMODE) != libc::O_RDONLY;
        let fh = self.create_handle_for_file(path, Some(file_id), flags);
        debug!("FOD open granted fh={} writable={}", fh, writable);
        reply.opened(fh, self.fopen_flags());
    }

    fn getlk(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: i32,
        _pid: u32,
        reply: ReplyLock,
    ) {
        let path = match self.entry_path_for_ino(ino) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let kind = match self.repo.resolve_path(&path) {
            Ok(resolved) => resolved.kind.unwrap_or_default(),
            Err(_) => {
                reply.error(EIO);
                return;
            }
        };
        if kind != "file" && kind != "hardlink" {
            reply.error(ENOENT);
            return;
        }
        let owner = lock_owner;
        debug!(
            "FOD getlk path={} owner={} start={} end={} typ={}",
            path, owner, start, end, typ
        );
        let resource_key = match self.resource_key_for_lock(&path) {
            Ok(value) => value,
            Err(errno) => {
                warn!(
                    "FOD getlk resource key failed path={} errno={}",
                    path, errno
                );
                reply.error(errno);
                return;
            }
        };
        let records = match self.load_lock_records_for_key(&resource_key) {
            Ok(records) => records,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        if let Some(conflict) = Self::lock_conflict(&records, owner, typ, start, end) {
            debug!(
                "FOD getlk conflict path={} owner={} conflict_owner={} start={} end={} typ={}",
                path,
                owner,
                conflict.owner,
                conflict.start,
                conflict.end.unwrap_or(0),
                conflict.typ
            );
            reply.locked(
                conflict.start,
                conflict.end.unwrap_or(0),
                conflict.typ,
                conflict.pid,
            );
        } else {
            debug!(
                "FOD getlk unlocked path={} owner={} start={} end={} typ={}",
                path, owner, start, end, typ
            );
            reply.locked(start, end, libc::F_UNLCK, 0);
        }
    }

    fn setlk(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: i32,
        pid: u32,
        sleep_flag: bool,
        reply: ReplyEmpty,
    ) {
        let path = match self.entry_path_for_ino(ino) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let kind = match self.repo.resolve_path(&path) {
            Ok(resolved) => resolved.kind.unwrap_or_default(),
            Err(err) => {
                warn!("FOD setlk resolve_path failed path={} err={}", path, err);
                reply.error(EIO);
                return;
            }
        };
        if kind != "file" && kind != "hardlink" {
            reply.error(ENOENT);
            return;
        }
        let resource_key = match self.resource_key_for_lock(&path) {
            Ok(value) => value,
            Err(errno) => {
                warn!(
                    "FOD setlk resource key failed path={} errno={}",
                    path, errno
                );
                reply.error(errno);
                return;
            }
        };
        let owner = lock_owner;
        self.register_local_lock_owner(owner);
        debug!(
            "FOD setlk path={} owner={} start={} end={} typ={} pid={} sleep_flag={}",
            path, owner, start, end, typ, pid, sleep_flag
        );
        if typ == libc::F_UNLCK {
            let mut records = match self.load_lock_records_for_key(&resource_key) {
                Ok(records) => records,
                Err(errno) => {
                    reply.error(errno);
                    return;
                }
            };
            records.retain(|record| {
                record.owner != owner
                    || !Self::range_overlaps(start, Some(end), record.start, record.end)
            });
            if let Err(errno) = self.persist_lock_records_for_key(&resource_key, owner, &records) {
                reply.error(errno);
                return;
            }
            self.refresh_local_lock_owner_state(owner);
            debug!("FOD setlk unlock path={} owner={}", path, owner);
            reply.ok();
            return;
        }
        if self.read_only && typ == libc::F_WRLCK {
            warn!(
                "FOD setlk denied in read_only mode path={} owner={}",
                path, owner
            );
            reply.error(libc::EROFS);
            return;
        }

        loop {
            let mut records = match self.load_lock_records_for_key(&resource_key) {
                Ok(records) => records,
                Err(errno) => {
                    reply.error(errno);
                    return;
                }
            };
            if Self::lock_conflict(&records, owner, typ, start, end).is_none() {
                records.retain(|record| {
                    record.owner != owner
                        || !Self::range_overlaps(start, Some(end), record.start, record.end)
                });
                if typ != libc::F_UNLCK {
                    records.push(PosixLockRecord {
                        owner,
                        typ,
                        start,
                        end: Some(end),
                        pid,
                    });
                }
                if let Err(errno) =
                    self.persist_lock_records_for_key(&resource_key, owner, &records)
                {
                    reply.error(errno);
                    return;
                }
                self.refresh_local_lock_owner_state(owner);
                debug!(
                    "FOD setlk granted path={} owner={} typ={}",
                    path, owner, typ
                );
                reply.ok();
                return;
            }
            if !sleep_flag {
                debug!(
                    "FOD setlk would block path={} owner={} typ={}",
                    path, owner, typ
                );
                reply.error(libc::EWOULDBLOCK);
                return;
            }
            debug!(
                "FOD setlk waiting path={} owner={} typ={}",
                path, owner, typ
            );
            thread::park_timeout(self.lock_poll_interval);
        }
    }

    fn bmap(&mut self, _req: &Request<'_>, ino: u64, blocksize: u32, idx: u64, reply: ReplyBmap) {
        if blocksize == 0 {
            reply.error(libc::EINVAL);
            return;
        }
        let (path, attrs) = match self.entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let size = attrs.file_attr.size;
        let logical_blocks = if size == 0 {
            0
        } else {
            (size + u64::from(blocksize) - 1) / u64::from(blocksize)
        }
        .max(1);
        debug!(
            "FOD bmap path={} ino={} blocksize={} idx={} logical_blocks={}",
            path, ino, blocksize, idx, logical_blocks
        );
        if idx >= logical_blocks {
            reply.error(libc::EINVAL);
            return;
        }
        reply.bmap(idx);
    }

    fn flush(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        lock_owner: u64,
        reply: ReplyEmpty,
    ) {
        let _write_profile = self.start_fuse_write_profile();
        debug!(
            "FOD flush fh={} lock_owner={} read_only={}",
            fh, lock_owner, self.read_only
        );
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        self.register_local_lock_owner(lock_owner);
        let Some(mut state) = self.write_state_for_handle(fh) else {
            debug!("FOD flush no write state fh={}", fh);
            reply.ok();
            return;
        };
        if !Self::write_state_has_pending_changes(&state) {
            debug!("FOD flush clean state fh={}", fh);
            reply.ok();
            return;
        }
        if let Err(errno) = self.flush_write_state(&mut state) {
            warn!("FOD flush failed fh={} errno={}", fh, errno);
            reply.error(errno);
            return;
        }
        if Self::write_state_has_pending_changes(&state) {
            self.update_write_state(fh, state);
        } else {
            self.remove_write_state(fh);
        }
        let changed = self.clear_locks_for_owner(lock_owner);
        for resource_key in changed {
            let records = self
                .posix_locks
                .lock()
                .ok()
                .and_then(|guard| guard.get(&resource_key).cloned())
                .unwrap_or_default();
            if let Err(errno) =
                self.persist_lock_records_for_key(&resource_key, lock_owner, &records)
            {
                warn!(
                    "FOD flush lock sync failed fh={} resource_key={} errno={}",
                    fh, resource_key, errno
                );
            }
        }
        self.refresh_local_lock_owner_state(lock_owner);
        debug!("FOD flush completed fh={} lock_owner={}", fh, lock_owner);
        reply.ok();
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let req_id = self.next_request_id();
        let _read_profile = self.start_fuse_read_profile();
        let file_id = match self.file_id_for_handle_or_errno(fh, ino) {
            Ok(value) => value,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "read",
                    errno,
                    format!("ino={} fh={} offset={} size={}", ino, fh, offset, size),
                );
                reply.error(errno);
                return;
            }
        };
        self.log_request_start(
            req_id,
            "read",
            format!(
                "ino={} fh={} file_id={} offset={} size={}",
                ino, fh, file_id, offset, size
            ),
        );
        if let Err(errno) = self.flush_pending_write_states_for_file_except(file_id, fh) {
            self.log_request_error(
                req_id,
                "read",
                errno,
                format!("file_id={} flush pending sibling fh states", file_id),
            );
            reply.error(errno);
            return;
        }
        let offset = offset.max(0) as u64;
        let size = size as u64;
        let current_attrs = match self.entry_attrs_for_ino(ino) {
            Ok((path, attrs)) => Some((path, attrs.file_attr)),
            Err(errno) => {
                self.log_request_error(req_id, "read", errno, format!("ino={} attrs skipped", ino));
                None
            }
        };
        if let Some(mut state) = self.write_state_for_handle(fh) {
            let data = match self.read_from_write_state(&mut state, offset, size) {
                Ok(data) => data,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "read",
                        errno,
                        format!("fh={} read_from_write_state", fh),
                    );
                    reply.error(errno);
                    return;
                }
            };
            if let Some((path, attrs)) = current_attrs.as_ref() {
                self.touch_access_time(path, attrs);
            }
            self.reply_data_profiled(reply, &data);
            return;
        }
        let file_size = match current_attrs.as_ref() {
            Some((_, attrs)) => attrs.size,
            None => match self.file_size_for_file_id_or_errno(file_id) {
                Ok(value) => value,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "read",
                        errno,
                        format!("file_id={} file_size", file_id),
                    );
                    reply.error(errno);
                    return;
                }
            },
        };
        if offset >= file_size {
            self.reply_data_profiled(reply, &[]);
            return;
        }
        let end_offset = offset.saturating_add(size).min(file_size);
        let live = self.reloadable_runtime();
        let first_block = offset / self.block_size;
        let last_block = (end_offset.saturating_sub(1)) / self.block_size;
        if first_block == last_block {
            if let Some(block) = self.cached_read_block(file_id, first_block) {
                let block_offset = first_block.saturating_mul(self.block_size);
                let slice_start = (offset.saturating_sub(block_offset)) as usize;
                let slice_end = (end_offset.saturating_sub(block_offset)) as usize;
                let data = block.as_ref();
                if slice_end <= data.len() && slice_start <= slice_end {
                    self.reply_data_profiled(reply, &data[slice_start..slice_end]);
                    if let Some((path, attrs)) = current_attrs.as_ref() {
                        self.touch_access_time(path, attrs);
                    }
                    return;
                }
            }
        }
        let total_blocks = 1 + (file_size - 1) / self.block_size.max(1);
        let (sequential, streak) = self.read_sequence_state_for_file(file_id, offset, end_offset);
        let (fetch_first, fetch_last) = if total_blocks <= live.small_file_read_threshold_blocks {
            (0, total_blocks.saturating_sub(1))
        } else {
            let mut read_ahead_blocks = live.read_ahead_blocks;
            if sequential {
                let dynamic_ahead = live
                    .sequential_read_ahead_blocks
                    .saturating_mul(streak.max(1));
                read_ahead_blocks = read_ahead_blocks.max(dynamic_ahead);
            }
            let cache_cap = self.read_cache_limit_blocks().saturating_sub(1) as u64;
            read_ahead_blocks = read_ahead_blocks.min(cache_cap);
            (
                first_block,
                (last_block + read_ahead_blocks).min(total_blocks.saturating_sub(1)),
            )
        };
        match self.read_block_map(file_id, fetch_first, fetch_last) {
            Ok(blocks) => {
                if first_block == last_block {
                    let block_offset = first_block.saturating_mul(self.block_size);
                    let slice_start = (offset.saturating_sub(block_offset)) as usize;
                    let slice_end = (end_offset.saturating_sub(block_offset)) as usize;
                    let block_index = (first_block.saturating_sub(fetch_first)) as usize;
                    if let Some((actual_block_index, block)) = blocks.get(block_index) {
                        if *actual_block_index == first_block {
                            let data = block.as_ref();
                            if slice_end <= data.len() && slice_start <= slice_end {
                                self.reply_data_profiled(reply, &data[slice_start..slice_end]);
                                if let Some((path, attrs)) = current_attrs.as_ref() {
                                    self.touch_access_time(path, attrs);
                                }
                                return;
                            }
                        }
                    }
                }
                let started = Instant::now();
                let data = assemble_read_slice(
                    fetch_first,
                    fetch_last,
                    offset,
                    end_offset,
                    self.block_size,
                    &blocks,
                );
                self.record_assemble_read_slice_elapsed(started.elapsed());
                if let Some((path, attrs)) = current_attrs.as_ref() {
                    self.touch_access_time(path, attrs);
                }
                self.reply_data_profiled(reply, &data)
            }
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "read",
                    errno,
                    format!("file_id={} read_block_map", file_id),
                );
                reply.error(errno)
            }
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _flags: i32,
        lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let _write_profile = self.start_fuse_write_profile();
        debug!(
            "FOD release fh={} flags={:#x} lock_owner={:?} flush={} read_only={}",
            fh, _flags, lock_owner, _flush, self.read_only
        );
        if self.read_only {
            if let Some(owner) = lock_owner {
                self.register_local_lock_owner(owner);
                let changed = self.clear_locks_for_owner(owner);
                for resource_key in changed {
                    let records = self
                        .posix_locks
                        .lock()
                        .ok()
                        .and_then(|guard| guard.get(&resource_key).cloned())
                        .unwrap_or_default();
                    if let Err(errno) =
                        self.persist_lock_records_for_key(&resource_key, owner, &records)
                    {
                        warn!(
                            "FOD release lock sync failed fh={} resource_key={} errno={}",
                            fh, resource_key, errno
                        );
                    }
                }
                self.refresh_local_lock_owner_state(owner);
            }
            self.remove_handle_state(fh);
            reply.ok();
            return;
        }
        if let Some(mut state) = self.write_state_for_handle(fh) {
            if Self::write_state_has_pending_changes(&state)
                && self.flush_write_state(&mut state).is_ok()
            {
                self.update_write_state(fh, state);
            }
        }
        if let Some(owner) = lock_owner {
            self.register_local_lock_owner(owner);
            let changed = self.clear_locks_for_owner(owner);
            for resource_key in changed {
                let records = self
                    .posix_locks
                    .lock()
                    .ok()
                    .and_then(|guard| guard.get(&resource_key).cloned())
                    .unwrap_or_default();
                if let Err(errno) =
                    self.persist_lock_records_for_key(&resource_key, owner, &records)
                {
                    warn!(
                        "FOD release lock sync failed fh={} resource_key={} errno={}",
                        fh, resource_key, errno
                    );
                }
            }
            self.refresh_local_lock_owner_state(owner);
        }
        self.remove_handle_state(fh);
        debug!("FOD release completed fh={}", fh);
        reply.ok();
    }

    fn setattr(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        let subject = self.request_identity(req);
        let _write_profile = size.is_some().then(|| self.start_fuse_write_profile());
        if self.read_only && size.is_some() {
            reply.error(libc::EROFS);
            return;
        }
        let (path, current_attrs) = match self.entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let (kind, entry_id) = match self.resolved_entry_for_path(&path) {
            Ok(value) => value,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let kind = kind.unwrap_or_default();
        let current_attrs = current_attrs.file_attr;

        if let Some(new_size) = size {
            if kind == "dir" {
                reply.error(libc::EISDIR);
                return;
            }
            let file_id = match self.file_id_for_path(&path) {
                Ok(Some(value)) => value,
                Ok(None) => {
                    reply.error(ENOENT);
                    return;
                }
                Err(errno) => {
                    reply.error(errno);
                    return;
                }
            };
            if let Some(fh) = fh {
                let mut state = self
                    .write_state_for_handle(fh)
                    .unwrap_or_else(|| Self::new_write_state(file_id, new_size, true));
                state.file_id = file_id;
                state.file_size = new_size;
                state.truncate_pending = true;
                if let Err(errno) = self.flush_write_state(&mut state) {
                    reply.error(errno);
                    return;
                }
                if Self::write_state_has_pending_changes(&state) {
                    self.update_write_state(fh, state);
                } else {
                    self.remove_write_state(fh);
                }
            } else {
                let mut state = Self::new_write_state(file_id, new_size, true);
                if let Err(errno) = self.flush_write_state(&mut state) {
                    reply.error(errno);
                    return;
                }
            }
        }

        if let Some(new_mode) = mode {
            if !Self::can_modify_mode(&subject, &current_attrs) {
                reply.error(libc::EPERM);
                return;
            }
            let mode_text = format!("{:o}", new_mode & 0o7777);
            let result = match kind.as_str() {
                "file" | "hardlink" => {
                    let file_id = match self.file_id_for_path(&path) {
                        Ok(Some(value)) => value,
                        Ok(None) => {
                            reply.error(ENOENT);
                            return;
                        }
                        Err(errno) => {
                            reply.error(errno);
                            return;
                        }
                    };
                    self.repo.update_file_mode(file_id, &mode_text)
                }
                "dir" => match entry_id {
                    Some(directory_id) => self.repo.update_directory_mode(directory_id, &mode_text),
                    None => Err("missing directory id".to_string()),
                },
                "symlink" => Ok(()),
                _ => Ok(()),
            };
            if result.is_err() {
                reply.error(EIO);
                return;
            }
        }

        if uid.is_some() || gid.is_some() {
            if !Self::can_change_owner(&subject, &current_attrs, uid, gid) {
                reply.error(libc::EPERM);
                return;
            }
            let new_uid = uid.unwrap_or(subject.uid);
            let new_gid = gid.unwrap_or(subject.gid);
            let default_mode = if kind == "dir" { 0o755 } else { 0o644 };
            let mode_text = match self.raw_mode_for_path(&path) {
                Ok(Some(value)) => value,
                _ => format!("{:o}", mode.unwrap_or(default_mode) & 0o7777),
            };
            let result = match kind.as_str() {
                "file" | "hardlink" => {
                    let file_id = match self.file_id_for_path(&path) {
                        Ok(Some(value)) => value,
                        Ok(None) => {
                            reply.error(ENOENT);
                            return;
                        }
                        Err(errno) => {
                            reply.error(errno);
                            return;
                        }
                    };
                    self.repo
                        .update_file_owner(file_id, new_uid, new_gid, &mode_text)
                }
                "dir" => match entry_id {
                    Some(directory_id) => {
                        self.repo
                            .update_directory_owner(directory_id, new_uid, new_gid, &mode_text)
                    }
                    None => Err("missing directory id".to_string()),
                },
                "symlink" => match entry_id {
                    Some(symlink_id) => {
                        self.repo.update_symlink_owner(symlink_id, new_uid, new_gid)
                    }
                    None => Err("missing symlink id".to_string()),
                },
                _ => Ok(()),
            };
            if result.is_err() {
                reply.error(EIO);
                return;
            }
        }

        if atime.is_some() || mtime.is_some() {
            let atime_needs_update = match atime {
                Some(TimeOrNow::SpecificTime(time)) => time != current_attrs.atime,
                Some(TimeOrNow::Now) => true,
                None => false,
            };
            let mtime_needs_update = match mtime {
                Some(TimeOrNow::SpecificTime(time)) => time != current_attrs.mtime,
                Some(TimeOrNow::Now) => true,
                None => false,
            };
            if !atime_needs_update && !mtime_needs_update {
                match self.lookup_path(&path) {
                    Ok(Some(attrs)) => {
                        reply.attr(&self.metadata_cache_ttl_live(), &attrs.file_attr)
                    }
                    Ok(None) => reply.error(ENOENT),
                    Err(errno) => reply.error(errno),
                }
                return;
            }
            let atime_text = Self::time_or_now_to_db_string(atime)
                .unwrap_or_else(|| Self::system_time_to_db_string(Self::current_time()));
            let mtime_text = Self::time_or_now_to_db_string(mtime)
                .unwrap_or_else(|| Self::system_time_to_db_string(Self::current_time()));
            let result = match kind.as_str() {
                "file" | "hardlink" => {
                    let file_id = match self.file_id_for_path(&path) {
                        Ok(Some(value)) => value,
                        Ok(None) => {
                            reply.error(ENOENT);
                            return;
                        }
                        Err(errno) => {
                            reply.error(errno);
                            return;
                        }
                    };
                    self.repo
                        .touch_file_times(file_id, &atime_text, &mtime_text)
                }
                "dir" => match entry_id {
                    Some(directory_id) => {
                        self.repo
                            .touch_directory_times(directory_id, &atime_text, &mtime_text)
                    }
                    None => Err("missing directory id".to_string()),
                },
                "symlink" => match entry_id {
                    Some(symlink_id) => self.repo.touch_symlink_entry(symlink_id),
                    None => Err("missing symlink id".to_string()),
                },
                _ => Ok(()),
            };
            if result.is_err() {
                reply.error(EIO);
                return;
            }
        }

        match self.lookup_path(&path) {
            Ok(Some(attrs)) => reply.attr(&self.metadata_cache_ttl_live(), &attrs.file_attr),
            Ok(None) => reply.error(ENOENT),
            Err(errno) => reply.error(errno),
        }
    }

    fn mkdir(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        let req_id = self.next_request_id();
        let subject = self.request_identity(req);
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        let parent_path = match self.entry_path_for_ino(parent) {
            Ok(path) => path,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "mkdir",
                    errno,
                    format!("parent={} name={}", parent, name.to_string_lossy()),
                );
                reply.error(errno);
                return;
            }
        };
        let child_path = Self::join_path(&parent_path, name);
        if let Ok(Some(_)) = self.lookup_path(&child_path) {
            reply.error(libc::EEXIST);
            return;
        }
        let parent_id = match self.parent_entry_id_for_inode(parent) {
            Ok(value) => value,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let mut mode = mode & !umask;
        if !subject.is_root() {
            mode &= !(libc::S_ISUID | libc::S_ISGID) as u32;
        }
        self.log_request_start(
            req_id,
            "mkdir",
            format!(
                "parent={} child={} mode={:#o} uid={} gid={}",
                parent_path, child_path, mode, subject.uid, subject.gid
            ),
        );
        match self.repo.create_directory(
            parent_id,
            name.to_string_lossy().as_ref(),
            mode,
            subject.uid,
            subject.gid,
            &child_path,
        ) {
            Ok(directory_id) => {
                let _ = self.copy_default_acl_to_child(&parent_path, "dir", directory_id, true);
                let _ = self.append_journal_event(
                    subject.uid,
                    "mkdir",
                    &child_path,
                    None,
                    Some(directory_id),
                );
                self.invalidate_statfs_cache();
                match self.lookup_path(&child_path) {
                    Ok(Some(attrs)) => {
                        self.register_path(&child_path, attrs.file_attr.ino);
                        debug!(
                            "FOD req={} op=mkdir created path={} directory_id={}",
                            req_id, child_path, directory_id
                        );
                        reply.entry(&self.metadata_cache_ttl_live(), &attrs.file_attr, 0);
                    }
                    Ok(None) => reply.error(EIO),
                    Err(errno) => reply.error(errno),
                }
            }
            Err(_) => reply.error(EIO),
        }
    }

    fn unlink(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let req_id = self.next_request_id();
        let subject = self.request_identity(req);
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        let parent_path = match self.entry_path_for_ino(parent) {
            Ok(path) => path,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "unlink",
                    errno,
                    format!("parent={} name={}", parent, name.to_string_lossy()),
                );
                reply.error(errno);
                return;
            }
        };
        let child_path = Self::join_path(&parent_path, name);
        self.log_request_start(
            req_id,
            "unlink",
            format!("parent={} child={}", parent_path, child_path),
        );
        let (kind, entry_id) = match self.resolved_entry_for_path(&child_path) {
            Ok(value) => value,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let result = match kind.as_deref() {
            Some("file") => {
                let file_id = match self.file_id_for_path(&child_path) {
                    Ok(Some(value)) => value,
                    Ok(None) => {
                        self.log_request_error(
                            req_id,
                            "unlink",
                            ENOENT,
                            format!("child={} missing file id", child_path),
                        );
                        reply.error(ENOENT);
                        return;
                    }
                    Err(errno) => {
                        self.log_request_error(
                            req_id,
                            "unlink",
                            errno,
                            format!("child={}", child_path),
                        );
                        reply.error(errno);
                        return;
                    }
                };
                let entry_attrs = match self.lookup_path(&child_path) {
                    Ok(Some(attrs)) => attrs.file_attr,
                    _ => {
                        self.log_request_error(
                            req_id,
                            "unlink",
                            EIO,
                            format!("child={} lookup", child_path),
                        );
                        reply.error(EIO);
                        return;
                    }
                };
                if let Err(errno) = self.enforce_sticky_bit(&parent_path, &entry_attrs, &subject) {
                    self.log_request_error(
                        req_id,
                        "unlink",
                        errno,
                        format!("child={} sticky bit", child_path),
                    );
                    reply.error(errno);
                    return;
                }
                self.remove_primary_file_or_promote_hardlink(file_id)
            }
            Some("hardlink") => {
                let entry_attrs = match self.lookup_path(&child_path) {
                    Ok(Some(attrs)) => attrs.file_attr,
                    _ => {
                        self.log_request_error(
                            req_id,
                            "unlink",
                            EIO,
                            format!("child={} lookup", child_path),
                        );
                        reply.error(EIO);
                        return;
                    }
                };
                if let Err(errno) = self.enforce_sticky_bit(&parent_path, &entry_attrs, &subject) {
                    self.log_request_error(
                        req_id,
                        "unlink",
                        errno,
                        format!("child={} sticky bit", child_path),
                    );
                    reply.error(errno);
                    return;
                }
                match entry_id {
                    Some(hardlink_id) => self.repo.delete_hardlink_entry(hardlink_id),
                    None => Err("missing hardlink id".to_string()),
                }
            }
            Some("symlink") => {
                let entry_attrs = match self.lookup_path(&child_path) {
                    Ok(Some(attrs)) => attrs.file_attr,
                    _ => {
                        self.log_request_error(
                            req_id,
                            "unlink",
                            EIO,
                            format!("child={} lookup", child_path),
                        );
                        reply.error(EIO);
                        return;
                    }
                };
                if let Err(errno) = self.enforce_sticky_bit(&parent_path, &entry_attrs, &subject) {
                    self.log_request_error(
                        req_id,
                        "unlink",
                        errno,
                        format!("child={} sticky bit", child_path),
                    );
                    reply.error(errno);
                    return;
                }
                match entry_id {
                    Some(symlink_id) => self.repo.delete_symlink_entry(symlink_id),
                    None => Err("missing symlink id".to_string()),
                }
            }
            Some("dir") => {
                self.log_request_error(
                    req_id,
                    "unlink",
                    libc::EISDIR,
                    format!("child={} dir", child_path),
                );
                reply.error(libc::EISDIR);
                return;
            }
            _ => {
                self.log_request_error(
                    req_id,
                    "unlink",
                    ENOENT,
                    format!("child={} missing", child_path),
                );
                reply.error(ENOENT);
                return;
            }
        };
        match result {
            Ok(_) => {
                let file_id = self.file_id_for_path(&child_path).ok().flatten();
                let dir_id = if parent == ROOT_INO {
                    None
                } else {
                    self.parent_entry_id_for_inode(parent).ok().flatten()
                };
                let _ =
                    self.append_journal_event(subject.uid, "unlink", &child_path, file_id, dir_id);
                self.remove_cached_path(&child_path);
                self.remove_cached_handle_paths(&child_path);
                self.invalidate_statfs_cache();
                debug!("FOD req={} op=unlink completed path={}", req_id, child_path);
                reply.ok();
            }
            Err(_) => {
                self.log_request_error(
                    req_id,
                    "unlink",
                    EIO,
                    format!("child={} delete", child_path),
                );
                reply.error(EIO)
            }
        }
    }

    fn rmdir(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let req_id = self.next_request_id();
        let subject = self.request_identity(req);
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        let parent_path = match self.entry_path_for_ino(parent) {
            Ok(path) => path,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "rmdir",
                    errno,
                    format!("parent={} name={}", parent, name.to_string_lossy()),
                );
                reply.error(errno);
                return;
            }
        };
        let child_path = Self::join_path(&parent_path, name);
        self.log_request_start(
            req_id,
            "rmdir",
            format!("parent={} child={}", parent_path, child_path),
        );
        let (kind, entry_id) = match self.resolved_entry_for_path(&child_path) {
            Ok(value) => value,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        if kind.as_deref() != Some("dir") {
            self.log_request_error(
                req_id,
                "rmdir",
                libc::ENOTDIR,
                format!("child={} kind={:?}", child_path, kind),
            );
            reply.error(libc::ENOTDIR);
            return;
        }
        let entry_attrs = match self.lookup_path(&child_path) {
            Ok(Some(attrs)) => attrs.file_attr,
            _ => {
                self.log_request_error(
                    req_id,
                    "rmdir",
                    EIO,
                    format!("child={} lookup", child_path),
                );
                reply.error(EIO);
                return;
            }
        };
        if let Err(errno) = self.enforce_sticky_bit(&parent_path, &entry_attrs, &subject) {
            self.log_request_error(
                req_id,
                "rmdir",
                errno,
                format!("child={} sticky bit", child_path),
            );
            reply.error(errno);
            return;
        }
        match self.repo.list_directory_entries_blob(&child_path) {
            Ok(Some(blob)) if !blob.is_empty() => {
                self.log_request_error(
                    req_id,
                    "rmdir",
                    libc::ENOTEMPTY,
                    format!("child={} not empty", child_path),
                );
                reply.error(libc::ENOTEMPTY);
                return;
            }
            Err(_) => {
                self.log_request_error(
                    req_id,
                    "rmdir",
                    EIO,
                    format!("child={} list entries", child_path),
                );
                reply.error(EIO);
                return;
            }
            _ => {}
        }
        match entry_id {
            Some(directory_id) => match self.repo.delete_directory_entry(directory_id) {
                Ok(_) => {
                    let _ = self.append_journal_event(
                        subject.uid,
                        "rmdir",
                        &child_path,
                        None,
                        Some(directory_id),
                    );
                    self.remove_cached_path(&child_path);
                    self.remove_cached_handle_paths(&child_path);
                    self.invalidate_statfs_cache();
                    debug!(
                        "FOD req={} op=rmdir completed path={} directory_id={}",
                        req_id, child_path, directory_id
                    );
                    reply.ok();
                }
                Err(_) => {
                    self.log_request_error(
                        req_id,
                        "rmdir",
                        EIO,
                        format!("child={} delete", child_path),
                    );
                    reply.error(EIO)
                }
            },
            None => {
                self.log_request_error(
                    req_id,
                    "rmdir",
                    ENOENT,
                    format!("child={} missing directory id", child_path),
                );
                reply.error(ENOENT)
            }
        }
    }

    fn rename(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        let req_id = self.next_request_id();
        let subject = self.request_identity(req);
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        let old_parent_path = match self.entry_path_for_ino(parent) {
            Ok(path) => path,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "rename",
                    errno,
                    format!("parent={} name={}", parent, name.to_string_lossy()),
                );
                reply.error(errno);
                return;
            }
        };
        let new_parent_path = match self.entry_path_for_ino(newparent) {
            Ok(path) => path,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "rename",
                    errno,
                    format!(
                        "newparent={} newname={}",
                        newparent,
                        newname.to_string_lossy()
                    ),
                );
                reply.error(errno);
                return;
            }
        };
        let old_path = Self::join_path(&old_parent_path, name);
        let new_path = Self::join_path(&new_parent_path, newname);
        self.log_request_start(
            req_id,
            "rename",
            format!(
                "old_parent={} old_path={} new_parent={} new_path={}",
                old_parent_path, old_path, new_parent_path, new_path
            ),
        );
        let (kind, entry_id) = match self.resolved_entry_for_path(&old_path) {
            Ok(value) => value,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "rename",
                    errno,
                    format!("old_path={} resolve", old_path),
                );
                reply.error(errno);
                return;
            }
        };
        let new_parent_id = match self.parent_entry_id_for_inode(newparent) {
            Ok(value) => value,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "rename",
                    errno,
                    format!("newparent={} parent_id", newparent),
                );
                reply.error(errno);
                return;
            }
        };
        let new_name = newname.to_string_lossy().to_string();
        let source_attrs = self.lookup_path(&old_path).ok().flatten();
        let old_ino = source_attrs
            .as_ref()
            .map(|attrs| attrs.file_attr.ino)
            .unwrap_or(ROOT_INO);
        if let Some(source_attrs) = source_attrs.as_ref() {
            if let Err(errno) =
                self.enforce_sticky_bit(&old_parent_path, &source_attrs.file_attr, &subject)
            {
                self.log_request_error(
                    req_id,
                    "rename",
                    errno,
                    format!("old_path={} sticky bit", old_path),
                );
                reply.error(errno);
                return;
            }
        }
        if old_path != new_path {
            let existing = match self.resolved_entry_for_path(&new_path) {
                Ok(value) => value,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "rename",
                        errno,
                        format!("new_path={} resolve", new_path),
                    );
                    reply.error(errno);
                    return;
                }
            };
            if let Some(existing_kind) = existing.0.as_deref() {
                let existing_attrs = self.lookup_path(&new_path).ok().flatten();
                if matches!(existing_kind, "file" | "hardlink" | "symlink") {
                    if let Some(existing_attrs) = existing_attrs.as_ref() {
                        if let Err(errno) = self.enforce_sticky_bit(
                            &new_parent_path,
                            &existing_attrs.file_attr,
                            &subject,
                        ) {
                            self.log_request_error(
                                req_id,
                                "rename",
                                errno,
                                format!("new_path={} sticky bit", new_path),
                            );
                            reply.error(errno);
                            return;
                        }
                    }
                }
                let removal_result = match existing_kind {
                    "file" => match self.file_id_for_path(&new_path) {
                        Ok(Some(file_id)) => self.remove_primary_file_or_promote_hardlink(file_id),
                        Ok(None) => Err("missing file id".to_string()),
                        Err(errno) => return reply.error(errno),
                    },
                    "hardlink" => match existing.1 {
                        Some(hardlink_id) => self.repo.delete_hardlink_entry(hardlink_id),
                        None => Err("missing hardlink id".to_string()),
                    },
                    "symlink" => match existing.1 {
                        Some(symlink_id) => self.repo.delete_symlink_entry(symlink_id),
                        None => Err("missing symlink id".to_string()),
                    },
                    "dir" => match (kind.as_deref(), existing.1) {
                        (Some("dir"), Some(directory_id)) => {
                            match self.repo.count_directory_children(directory_id) {
                                Ok(0) => self.repo.delete_directory_entry(directory_id),
                                Ok(_) => Err("target directory not empty".to_string()),
                                Err(_) => Err("failed to inspect target directory".to_string()),
                            }
                        }
                        (Some("dir"), None) => Err("missing directory id".to_string()),
                        _ => {
                            self.log_request_error(
                                req_id,
                                "rename",
                                libc::EISDIR,
                                format!("new_path={} existing dir", new_path),
                            );
                            reply.error(libc::EISDIR);
                            return;
                        }
                    },
                    _ => Ok(()),
                };
                if removal_result.is_err() {
                    if matches!(existing_kind, "dir") && matches!(kind.as_deref(), Some("dir")) {
                        if let Some(directory_id) = existing.1 {
                            if let Ok(count) = self.repo.count_directory_children(directory_id) {
                                reply.error(if count == 0 { EIO } else { ENOTEMPTY });
                                return;
                            }
                        }
                    }
                    self.log_request_error(
                        req_id,
                        "rename",
                        EIO,
                        format!("new_path={} removal failed", new_path),
                    );
                    reply.error(EIO);
                    return;
                }
                self.remove_cached_path(&new_path);
                self.remove_cached_handle_paths(&new_path);
            }
        }
        let result = match kind.as_deref() {
            Some("file") => match self.file_id_for_path(&old_path) {
                Ok(Some(file_id)) => self
                    .repo
                    .rename_file_entry(file_id, new_parent_id, &new_name),
                Ok(None) => Err("missing file id".to_string()),
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "rename",
                        errno,
                        format!("old_path={} file id", old_path),
                    );
                    return reply.error(errno);
                }
            },
            Some("hardlink") => match entry_id {
                Some(hardlink_id) => {
                    self.repo
                        .rename_hardlink_entry(hardlink_id, new_parent_id, &new_name)
                }
                None => Err("missing hardlink id".to_string()),
            },
            Some("symlink") => match entry_id {
                Some(symlink_id) => {
                    self.repo
                        .rename_symlink_entry(symlink_id, new_parent_id, &new_name)
                }
                None => Err("missing symlink id".to_string()),
            },
            Some("dir") => match entry_id {
                Some(directory_id) => {
                    self.repo
                        .rename_directory_entry(directory_id, new_parent_id, &new_name)
                }
                None => Err("missing directory id".to_string()),
            },
            _ => Err("unsupported rename kind".to_string()),
        };
        match result {
            Ok(_) => {
                let _ = self.append_journal_event(
                    subject.uid,
                    "rename",
                    &format!("{old_path}->{new_path}"),
                    None,
                    None,
                );
                self.move_cached_path(&old_path, &new_path, old_ino);
                self.invalidate_statfs_cache();
                debug!(
                    "FOD req={} op=rename completed old_path={} new_path={}",
                    req_id, old_path, new_path
                );
                reply.ok();
            }
            Err(_) => {
                self.log_request_error(
                    req_id,
                    "rename",
                    EIO,
                    format!("old_path={} new_path={}", old_path, new_path),
                );
                reply.error(EIO)
            }
        }
    }

    fn create(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let req_id = self.next_request_id();
        let subject = self.request_identity(req);
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        let parent_path = match self.entry_path_for_ino(parent) {
            Ok(path) => path,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "create",
                    errno,
                    format!("parent={} name={}", parent, name.to_string_lossy()),
                );
                reply.error(errno);
                return;
            }
        };
        let child_path = Self::join_path(&parent_path, name);
        self.log_request_start(
            req_id,
            "create",
            format!(
                "parent={} child={} mode={:#o} flags={:#x} uid={} gid={}",
                parent_path, child_path, mode, flags, subject.uid, subject.gid
            ),
        );
        if let Ok(Some(_)) = self.lookup_path(&child_path) {
            reply.error(libc::EEXIST);
            return;
        }
        let parent_id = match self.parent_entry_id_for_inode(parent) {
            Ok(value) => value,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let mut mode = mode & !umask;
        if !subject.is_root() {
            mode &= !(libc::S_ISUID | libc::S_ISGID) as u32;
        }
        let file_id = match self.repo.create_file(
            parent_id,
            name.to_string_lossy().as_ref(),
            mode,
            subject.uid,
            subject.gid,
            &child_path,
        ) {
            Ok(file_id) => file_id,
            Err(_) => {
                reply.error(EIO);
                return;
            }
        };
        let _ = self.copy_default_acl_to_child(&parent_path, "file", file_id, false);
        let fh = self.create_handle_for_file(child_path.clone(), Some(file_id), flags);
        match self.lookup_path(&child_path) {
            Ok(Some(attrs)) => {
                self.register_path(&child_path, attrs.file_attr.ino);
                let _ = self.append_journal_event(
                    subject.uid,
                    "create",
                    &child_path,
                    Some(file_id),
                    None,
                );
                self.invalidate_statfs_cache();
                debug!(
                    "FOD req={} op=create completed path={} file_id={} fh={}",
                    req_id, child_path, file_id, fh
                );
                reply.created(
                    &self.metadata_cache_ttl_live(),
                    &attrs.file_attr,
                    0,
                    fh,
                    self.fopen_flags(),
                );
            }
            Ok(None) => reply.error(EIO),
            Err(errno) => reply.error(errno),
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        let req_id = self.next_request_id();
        let _write_profile = self.start_fuse_write_profile();
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        let file_id = match self.file_id_for_handle_or_errno(fh, ino) {
            Ok(value) => value,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "write",
                    errno,
                    format!("ino={} fh={} offset={} len={}", ino, fh, offset, data.len()),
                );
                reply.error(errno);
                return;
            }
        };
        let offset = offset.max(0) as u64;
        self.log_request_start(
            req_id,
            "write",
            format!(
                "ino={} fh={} file_id={} offset={} len={}",
                ino,
                fh,
                file_id,
                offset,
                data.len()
            ),
        );
        if data.is_empty() {
            self.reply_written_profiled(reply, 0);
            return;
        }
        let sibling_states = match self.drain_pending_write_states_for_file_except(file_id, fh) {
            Ok(states) => states,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "write",
                    errno,
                    format!("file_id={} take pending sibling fh states", file_id),
                );
                reply.error(errno);
                return;
            }
        };
        let existing_state = self.take_write_state_for_handle(fh);
        let existing_size = if let Some(state) = existing_state.as_ref() {
            state.file_size
        } else {
            match self.entry_attrs_for_ino(ino) {
                Ok((_, attrs)) => attrs.file_attr.size,
                Err(errno) => {
                    if let Some(existing_state) = existing_state {
                        self.update_write_state(fh, existing_state);
                    }
                    for (sibling_fh, sibling_state) in sibling_states {
                        self.update_write_state(sibling_fh, sibling_state);
                    }
                    self.log_request_error(
                        req_id,
                        "write",
                        errno,
                        format!("ino={} fh={} attrs", ino, fh),
                    );
                    reply.error(errno);
                    return;
                }
            }
        };
        let existing_size = sibling_states
            .iter()
            .map(|(_, state)| state.file_size)
            .max()
            .map(|size| existing_size.max(size))
            .unwrap_or(existing_size);
        let has_sibling_states = !sibling_states.is_empty();
        let end_offset = offset.saturating_add(data.len() as u64);
        if existing_state.is_none()
            && !has_sibling_states
            && offset <= existing_size
            && end_offset <= existing_size
        {
            let first_block = offset / self.block_size;
            let last_block = (end_offset.saturating_sub(1)) / self.block_size;
            if let Ok(existing) = self.repo.assemble_file_slice(
                file_id,
                first_block,
                last_block,
                offset,
                end_offset,
                self.block_size,
            ) {
                if existing == data {
                    debug!(
                        "FOD req={} op=write skipped unchanged slice ino={} fh={} offset={} len={}",
                        req_id,
                        ino,
                        fh,
                        offset,
                        data.len()
                    );
                    self.reply_written_profiled(reply, data.len() as u32);
                    return;
                }
            }
        }
        let mut state =
            existing_state.unwrap_or_else(|| Self::new_write_state(file_id, existing_size, false));
        state.file_id = file_id;
        for (_sibling_fh, sibling_state) in sibling_states.iter() {
            let sibling_state = self.clone_write_state_profiled(sibling_state);
            self.merge_write_state_into(&mut state, sibling_state, self.block_size);
        }
        if let Err(errno) = self.update_write_buffer(&mut state, offset, data) {
            self.log_request_error(req_id, "write", errno, format!("fh={} update", fh));
            reply.error(errno);
            return;
        }
        state.buffered_bytes = state.buffered_bytes.saturating_add(data.len() as u64);

        let shared_open_handles = self.open_handle_count_for_file(file_id);
        let block_size = self.block_size.max(1);
        let write_len = data.len() as u64;
        let partial_block_visibility_write = write_len > 0
            && (offset % block_size != 0 || write_len < block_size || write_len % block_size != 0);
        // Przy wielu aktywnych fh publikujemy zmiany od razu, zeby kolejny open
        // nie czytal jeszcze starego stanu z repo albo cache kernela.
        // Kazdy zapis czesciowego bloku tez publikujemy od razu. Inaczej drugi fh
        // moze zbudowac blok na zerach i nadpisac poczatek pliku.
        let should_flush_now = self.should_flush_write_state(
            state.buffered_bytes,
            shared_open_handles,
            partial_block_visibility_write,
        );
        let mut flushed_now = false;

        if should_flush_now {
            if let Err(errno) = self.flush_write_state(&mut state) {
                self.update_write_state(fh, state);
                for (sibling_fh, sibling_state) in sibling_states {
                    self.update_write_state(sibling_fh, sibling_state);
                }
                self.log_request_error(req_id, "write", errno, format!("fh={} flush", fh));
                reply.error(errno);
                return;
            }

            flushed_now = true;
        }

        if has_sibling_states {
            if let Ok(mut guard) = self.write_states.lock() {
                for (sibling_fh, _) in sibling_states.iter() {
                    guard.remove(sibling_fh);
                }
            }
        }

        // Po auto-flushu nie zostawiamy pustego WriteState pod tym fh.
        // Inaczej read() moze wejsc w read_from_write_state() zamiast czytac normalnie z repo.
        if flushed_now && !Self::write_state_has_pending_changes(&state) {
            if let Ok(mut guard) = self.write_states.lock() {
                guard.remove(&fh);
            }
        } else {
            self.update_write_state(fh, state);
        }

        debug!(
            "FOD req={} op=write buffered ino={} fh={} bytes={}",
            req_id,
            ino,
            fh,
            data.len()
        );
        self.reply_written_profiled(reply, data.len() as u32);
    }

    fn copy_file_range(
        &mut self,
        _req: &Request<'_>,
        ino_in: u64,
        fh_in: u64,
        offset_in: i64,
        ino_out: u64,
        fh_out: u64,
        offset_out: i64,
        len: u64,
        _flags: u32,
        reply: ReplyWrite,
    ) {
        let req_id = self.next_request_id();
        let _write_profile = self.start_fuse_write_profile();
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        if offset_in < 0 || offset_out < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        if len == 0 {
            self.reply_written_profiled(reply, 0);
            return;
        }
        self.log_request_start(
            req_id,
            "copy_file_range",
            format!(
                "ino_in={} fh_in={} ino_out={} fh_out={} offset_in={} offset_out={} len={}",
                ino_in, fh_in, ino_out, fh_out, offset_in, offset_out, len
            ),
        );
        let src_file_id = match self.file_id_for_handle(fh_in, ino_in) {
            Ok(Some(value)) => value,
            Ok(None) => {
                self.log_request_error(
                    req_id,
                    "copy_file_range",
                    ENOENT,
                    format!("src ino={} fh={}", ino_in, fh_in),
                );
                reply.error(ENOENT);
                return;
            }
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "copy_file_range",
                    errno,
                    format!("src ino={} fh={}", ino_in, fh_in),
                );
                reply.error(errno);
                return;
            }
        };
        let dst_file_id = match self.file_id_for_handle(fh_out, ino_out) {
            Ok(Some(value)) => value,
            Ok(None) => {
                self.log_request_error(
                    req_id,
                    "copy_file_range",
                    ENOENT,
                    format!("dst ino={} fh={}", ino_out, fh_out),
                );
                reply.error(ENOENT);
                return;
            }
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "copy_file_range",
                    errno,
                    format!("dst ino={} fh={}", ino_out, fh_out),
                );
                reply.error(errno);
                return;
            }
        };
        let src_state = self.write_state_for_handle(fh_in);
        let dst_state = self.write_state_for_handle(fh_out);
        let dst_size = if let Some(state) = dst_state.as_ref() {
            state.file_size
        } else {
            match self.file_size_for_file_id_or_errno(dst_file_id) {
                Ok(value) => value,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "copy_file_range",
                        errno,
                        format!("dst_file_id={} size", dst_file_id),
                    );
                    reply.error(errno);
                    return;
                }
            }
        };
        let src_size = if let Some(state) = src_state.as_ref() {
            state.file_size
        } else {
            match self.file_size_for_file_id_or_errno(src_file_id) {
                Ok(value) => value,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "copy_file_range",
                        errno,
                        format!("src_file_id={} size", src_file_id),
                    );
                    reply.error(errno);
                    return;
                }
            }
        };
        let src_offset = offset_in as u64;
        let dst_offset = offset_out as u64;
        let Some(bounds) =
            copy_range_bounds(self.block_size, src_offset, dst_offset, len, src_size)
        else {
            self.reply_written_profiled(reply, 0);
            return;
        };
        let src_dirty = src_state
            .as_ref()
            .map(Self::write_state_has_pending_changes)
            .unwrap_or(false);
        let dst_dirty = dst_state
            .as_ref()
            .map(Self::write_state_has_pending_changes)
            .unwrap_or(false);
        let adopt_whole_object =
            bounds.can_adopt_whole_object(src_size, dst_size, src_dirty, dst_dirty);
        let CopyRangeBounds {
            src_offset,
            dst_offset,
            copy_len,
            src_end_offset,
            dst_end_offset: _,
            src_first_block,
            src_last_block,
            dst_first_block,
            dst_last_block,
        } = bounds;
        let dedupe_enabled = self.copy_dedupe_enabled_for_len(copy_len);

        if adopt_whole_object {
            match self.repo.adopt_source_data_object(src_file_id, dst_file_id) {
                Ok(true) => {
                    if let Some(state) = dst_state.as_ref() {
                        let mut state = self.clone_write_state_profiled(state);
                        state.file_size = src_size;
                        self.update_write_state(fh_out, state);
                    }
                    debug!(
                        "FOD req={} op=copy_file_range adopted source data object src_file_id={} dst_file_id={} len={}",
                        req_id, src_file_id, dst_file_id, copy_len
                    );
                    self.reply_written_profiled(reply, copy_len as u32);
                    return;
                }
                Ok(false) => {}
                Err(_) => {
                    self.log_request_error(
                        req_id,
                        "copy_file_range",
                        EIO,
                        format!(
                            "src_file_id={} dst_file_id={} adopt",
                            src_file_id, dst_file_id
                        ),
                    );
                    reply.error(EIO);
                    return;
                }
            }
        }

        let data = if let Some(state) = src_state.as_ref() {
            let mut state = self.clone_write_state_profiled(state);
            match self.read_from_write_state(&mut state, src_offset, copy_len) {
                Ok(data) => data,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "copy_file_range",
                        errno,
                        format!("src_file_id={} load_write_state", src_file_id),
                    );
                    reply.error(errno);
                    return;
                }
            }
        } else {
            match self.repo.assemble_file_slice(
                src_file_id,
                src_first_block,
                src_last_block,
                src_offset,
                src_end_offset,
                self.block_size,
            ) {
                Ok(data) => data,
                Err(_) => {
                    self.log_request_error(
                        req_id,
                        "copy_file_range",
                        EIO,
                        format!("src_file_id={} assemble", src_file_id),
                    );
                    reply.error(EIO);
                    return;
                }
            }
        };
        let dst_initial_size = dst_state
            .as_ref()
            .map(|state| state.file_size)
            .unwrap_or(dst_size);
        let had_dst_state = dst_state.is_some();
        let mut state = dst_state
            .unwrap_or_else(|| Self::new_write_state(dst_file_id, dst_initial_size, false));
        state.file_id = dst_file_id;
        let current_size = state.file_size;
        let target_end = dst_offset.saturating_add(copy_len);

        if dedupe_enabled && dst_offset < current_size {
            let current = if had_dst_state {
                let mut compare_state = self.clone_write_state_profiled(&state);
                if target_end > compare_state.file_size {
                    compare_state.file_size = target_end;
                }
                self.read_copy_destination_slice(
                    dst_file_id,
                    Some(&mut compare_state),
                    dst_first_block,
                    dst_last_block,
                    dst_offset,
                    copy_len,
                    current_size,
                )
            } else {
                self.read_copy_destination_slice(
                    dst_file_id,
                    None,
                    dst_first_block,
                    dst_last_block,
                    dst_offset,
                    copy_len,
                    current_size,
                )
            };
            let current = match current {
                Ok(data) => data,
                Err(_) => {
                    self.log_request_error(
                        req_id,
                        "copy_file_range",
                        EIO,
                        format!("dst_file_id={} assemble", dst_file_id),
                    );
                    reply.error(EIO);
                    return;
                }
            };
            let runs = pack_copy_skip_unchanged_runs(dst_offset, self.block_size, &data, &current);
            if runs.is_empty() {
                if target_end > current_size {
                    state.file_size = target_end;
                    if let Err(errno) = self.flush_write_state(&mut state) {
                        self.log_request_error(
                            req_id,
                            "copy_file_range",
                            errno,
                            format!("fh_out={} flush", fh_out),
                        );
                        reply.error(errno);
                        return;
                    }
                    self.update_write_state(fh_out, state);
                    debug!(
                        "FOD req={} op=copy_file_range dedupe extended size without changed blocks src_file_id={} dst_file_id={} len={}",
                        req_id, src_file_id, dst_file_id, copy_len
                    );
                } else {
                    debug!(
                        "FOD req={} op=copy_file_range dedupe skipped unchanged blocks src_file_id={} dst_file_id={} len={}",
                        req_id, src_file_id, dst_file_id, copy_len
                    );
                }
                self.reply_written_profiled(reply, copy_len as u32);
                return;
            }
            if target_end > state.file_size {
                state.file_size = target_end;
            }
            for (run_start, run_payload) in runs {
                if let Err(errno) = self.update_write_buffer(&mut state, run_start, &run_payload) {
                    self.log_request_error(
                        req_id,
                        "copy_file_range",
                        errno,
                        format!("fh_out={} update_write_buffer", fh_out),
                    );
                    reply.error(errno);
                    return;
                }
                state.buffered_bytes = state
                    .buffered_bytes
                    .saturating_add(run_payload.len() as u64);
            }
            if let Err(errno) = self.flush_write_state(&mut state) {
                self.log_request_error(
                    req_id,
                    "copy_file_range",
                    errno,
                    format!("fh_out={} flush", fh_out),
                );
                reply.error(errno);
                return;
            }
            self.update_write_state(fh_out, state);
            debug!(
                "FOD req={} op=copy_file_range dedupe wrote changed blocks src_file_id={} dst_file_id={} len={}",
                req_id, src_file_id, dst_file_id, copy_len
            );
            self.reply_written_profiled(reply, copy_len as u32);
            return;
        }

        if let Err(errno) = self.update_write_buffer(&mut state, dst_offset, &data) {
            self.log_request_error(
                req_id,
                "copy_file_range",
                errno,
                format!("fh_out={} update_write_buffer", fh_out),
            );
            reply.error(errno);
            return;
        }
        state.buffered_bytes = state.buffered_bytes.saturating_add(data.len() as u64);
        if let Err(errno) = self.flush_write_state(&mut state) {
            self.log_request_error(
                req_id,
                "copy_file_range",
                errno,
                format!("fh_out={} flush", fh_out),
            );
            reply.error(errno);
            return;
        }
        self.update_write_state(fh_out, state);
        debug!(
            "FOD req={} op=copy_file_range completed src_file_id={} dst_file_id={} len={}",
            req_id, src_file_id, dst_file_id, copy_len
        );
        self.reply_written_profiled(reply, copy_len as u32);
    }

    fn mknod(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        rdev: u32,
        reply: ReplyEntry,
    ) {
        let subject = self.request_identity(req);
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        let file_type = mode & libc::S_IFMT as u32;
        if file_type == libc::S_IFREG as u32 {
            let parent_path = match self.entry_path_for_ino(parent) {
                Ok(path) => path,
                Err(errno) => {
                    reply.error(errno);
                    return;
                }
            };
            let child_path = Self::join_path(&parent_path, name);
            if let Ok(Some(_)) = self.lookup_path(&child_path) {
                reply.error(libc::EEXIST);
                return;
            }
            let parent_id = match self.parent_entry_id_for_inode(parent) {
                Ok(value) => value,
                Err(errno) => {
                    reply.error(errno);
                    return;
                }
            };
            let mut mode = mode & !umask;
            if !subject.is_root() {
                mode &= !(libc::S_ISUID | libc::S_ISGID) as u32;
            }
            match self.repo.create_file(
                parent_id,
                name.to_string_lossy().as_ref(),
                mode,
                subject.uid,
                subject.gid,
                &child_path,
            ) {
                Ok(_) => match self.lookup_path(&child_path) {
                    Ok(Some(attrs)) => {
                        self.register_path(&child_path, attrs.file_attr.ino);
                        self.invalidate_statfs_cache();
                        reply.entry(&self.metadata_cache_ttl_live(), &attrs.file_attr, 0);
                    }
                    Ok(None) => reply.error(EIO),
                    Err(errno) => reply.error(errno),
                },
                Err(_) => reply.error(EIO),
            }
            return;
        }
        let parent_path = match self.entry_path_for_ino(parent) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let child_path = Self::join_path(&parent_path, name);
        if let Ok(Some(_)) = self.lookup_path(&child_path) {
            reply.error(libc::EEXIST);
            return;
        }
        let parent_id = match self.parent_entry_id_for_inode(parent) {
            Ok(value) => value,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let mut mode = mode & !umask;
        if !subject.is_root() {
            mode &= !(libc::S_ISUID | libc::S_ISGID) as u32;
        }
        let file_kind = if file_type == libc::S_IFIFO as u32 {
            "fifo"
        } else if file_type == libc::S_IFCHR as u32 {
            "char"
        } else if file_type == libc::S_IFBLK as u32 {
            "block"
        } else {
            reply.error(libc::EINVAL);
            return;
        };
        match self.repo.create_special_file(
            parent_id,
            name.to_string_lossy().as_ref(),
            mode,
            subject.uid,
            subject.gid,
            &child_path,
            file_kind,
            libc::major(rdev as libc::dev_t) as u32,
            libc::minor(rdev as libc::dev_t) as u32,
        ) {
            Ok(_) => {}
            Err(_) => {
                reply.error(EIO);
                return;
            }
        };
        match self.lookup_path(&child_path) {
            Ok(Some(attrs)) => {
                self.register_path(&child_path, attrs.file_attr.ino);
                self.invalidate_statfs_cache();
                reply.entry(&self.metadata_cache_ttl_live(), &attrs.file_attr, 0);
            }
            Ok(None) => reply.error(EIO),
            Err(errno) => reply.error(errno),
        }
    }

    fn symlink(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        link_name: &OsStr,
        target: &Path,
        reply: ReplyEntry,
    ) {
        let subject = self.request_identity(req);
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        let parent_path = match self.entry_path_for_ino(parent) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let child_path = Self::join_path(&parent_path, link_name);
        if let Ok(Some(_)) = self.lookup_path(&child_path) {
            reply.error(libc::EEXIST);
            return;
        }
        let parent_id = match self.parent_entry_id_for_inode(parent) {
            Ok(value) => value,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let inode_seed = child_path.as_str();
        match self.repo.create_symlink(
            parent_id,
            link_name.to_string_lossy().as_ref(),
            &target.to_string_lossy(),
            subject.uid,
            subject.gid,
            inode_seed,
        ) {
            Ok(_) => match self.lookup_path(&child_path) {
                Ok(Some(attrs)) => {
                    self.register_path(&child_path, attrs.file_attr.ino);
                    self.invalidate_statfs_cache();
                    reply.entry(&self.metadata_cache_ttl_live(), &attrs.file_attr, 0);
                }
                Ok(None) => reply.error(EIO),
                Err(errno) => reply.error(errno),
            },
            Err(_) => reply.error(EIO),
        }
    }

    fn link(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        newparent: u64,
        newname: &OsStr,
        reply: ReplyEntry,
    ) {
        let subject = self.request_identity(req);
        if self.read_only {
            reply.error(libc::EROFS);
            return;
        }
        let source_path = match self.entry_path_for_ino(ino) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let source_file_id = match self.file_id_for_path(&source_path) {
            Ok(Some(value)) => value,
            Ok(None) => {
                reply.error(ENOENT);
                return;
            }
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let new_parent_path = match self.entry_path_for_ino(newparent) {
            Ok(path) => path,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        let child_path = Self::join_path(&new_parent_path, newname);
        if let Ok(Some(_)) = self.lookup_path(&child_path) {
            reply.error(libc::EEXIST);
            return;
        }
        let new_parent_id = match self.parent_entry_id_for_inode(newparent) {
            Ok(value) => value,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };
        match self.repo.create_hardlink(
            source_file_id,
            new_parent_id,
            newname.to_string_lossy().as_ref(),
            subject.uid,
            subject.gid,
        ) {
            Ok(_) => match self.lookup_path(&child_path) {
                Ok(Some(attrs)) => {
                    self.register_path(&child_path, attrs.file_attr.ino);
                    self.invalidate_statfs_cache();
                    reply.entry(&self.metadata_cache_ttl_live(), &attrs.file_attr, 0);
                }
                Ok(None) => reply.error(EIO),
                Err(errno) => reply.error(errno),
            },
            Err(_) => reply.error(EIO),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{should_update_atime, AtimePolicy, FodFuseProfileCounters, WriteState};
    use crate::write_payload::{BlockWriteState, WritePayloadState};
    use fuser::FileAttr;
    use fuser::FileType;
    use std::collections::BTreeMap;
    use std::time::{Duration, SystemTime};

    fn file_attr(kind: FileType, atime_age_secs: u64, mtime_age_secs: u64) -> FileAttr {
        let now = SystemTime::now();
        FileAttr {
            ino: 1,
            size: 0,
            blocks: 0,
            atime: now - Duration::from_secs(atime_age_secs),
            mtime: now - Duration::from_secs(mtime_age_secs),
            ctime: now,
            crtime: now,
            kind,
            perm: 0o644,
            nlink: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
            flags: 0,
            blksize: 4096,
        }
    }

    #[test]
    fn parses_atime_policy_values() {
        assert_eq!(AtimePolicy::parse("default").unwrap(), AtimePolicy::Default);
        assert_eq!(AtimePolicy::parse("noatime").unwrap(), AtimePolicy::NoAtime);
        assert_eq!(
            AtimePolicy::parse("nodiratime").unwrap(),
            AtimePolicy::Nodiratime
        );
        assert_eq!(
            AtimePolicy::parse("relatime").unwrap(),
            AtimePolicy::Relatime
        );
        assert_eq!(
            AtimePolicy::parse("strictatime").unwrap(),
            AtimePolicy::StrictAtime
        );
        assert!(AtimePolicy::parse("bad").is_err());
    }

    #[test]
    fn relatime_only_touches_stale_entries() {
        let recent = file_attr(FileType::RegularFile, 10, 20);
        let stale = file_attr(FileType::RegularFile, 86_401, 20);
        let mtime_newer = file_attr(FileType::RegularFile, 10, 5);

        assert!(!should_update_atime(AtimePolicy::Relatime, false, &recent));
        assert!(should_update_atime(AtimePolicy::Relatime, false, &stale));
        assert!(should_update_atime(
            AtimePolicy::Relatime,
            false,
            &mtime_newer
        ));
    }

    #[test]
    fn nodiratime_skips_directories_only() {
        let dir = file_attr(FileType::Directory, 10, 20);
        let file = file_attr(FileType::RegularFile, 10, 20);

        assert!(!should_update_atime(AtimePolicy::Nodiratime, true, &dir));
        assert!(should_update_atime(AtimePolicy::Nodiratime, false, &file));
    }

    #[test]
    fn stable_inode_hash64_is_deterministic_and_seed_sensitive() {
        let same_a = super::FodFuse::hash_inode64(b"file:/alpha");
        let same_b = super::FodFuse::hash_inode64(b"file:/alpha");
        let other = super::FodFuse::hash_inode64(b"file:/beta");

        assert_eq!(same_a, same_b);
        assert_ne!(same_a, other);
    }

    #[test]
    fn write_state_dirty_detection_matches_pending_work() {
        let clean = WriteState {
            file_id: 1,
            file_size: 128,
            truncate_pending: false,
            buffered_bytes: 0,
            load_error: false,
            payload: WritePayloadState::default(),
        };
        let buffered = WriteState {
            buffered_bytes: 16,
            ..clean.clone()
        };
        let truncated = WriteState {
            truncate_pending: true,
            ..clean.clone()
        };
        let blocked = WriteState {
            payload: WritePayloadState::BlockOverlay(BlockWriteState {
                blocks: {
                    let mut blocks = BTreeMap::new();
                    blocks.insert(0, vec![1, 2, 3, 4]);
                    blocks
                },
            }),
            ..clean
        };

        assert!(!super::FodFuse::write_state_has_pending_changes(&clean));
        assert!(super::FodFuse::write_state_has_pending_changes(&buffered));
        assert!(super::FodFuse::write_state_has_pending_changes(&truncated));
        assert!(super::FodFuse::write_state_has_pending_changes(&blocked));
    }

    #[test]
    fn extent_payload_profile_records_the_largest_payload() {
        let counters = FodFuseProfileCounters::default();
        counters.record_prepare_persist_extent_rows_peak_payload_bytes(1024 * 1024);
        counters.record_prepare_persist_extent_rows_peak_payload_bytes(256 * 1024);

        assert!(counters.has_activity());
        assert!(counters
            .snapshot_lines()
            .iter()
            .any(|line| line == "prepare_persist_extent_rows_peak_payload_bytes=1048576"));
    }

    #[test]
    fn segment_profile_records_entries_downgrades_payload_and_rows() {
        let counters = FodFuseProfileCounters::default();
        counters.record_segment_mode_entry();
        counters.record_segment_mode_downgrade();
        counters.record_segment_payload_bytes(64 * 1024);
        counters.record_segment_count(4);
        counters.record_prepare_persist_segment_rows_elapsed(Duration::from_micros(17));

        let lines = counters.snapshot_lines();
        assert!(lines.iter().any(|line| line == "segment_mode_entries=1"));
        assert!(lines.iter().any(|line| line == "segment_mode_downgrades=1"));
        assert!(lines
            .iter()
            .any(|line| line == "segment_payload_bytes=65536"));
        assert!(lines.iter().any(|line| line == "segment_count=4"));
        assert!(lines
            .iter()
            .any(|line| line == "prepare_persist_segment_rows_us=17"));
    }
}
