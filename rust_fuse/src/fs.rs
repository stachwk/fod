// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use chrono::{DateTime, NaiveDateTime, Utc};
use fod_rust_monitor::{
    current_process_rss_bytes, log_logical_task_observability as log_logical_task_snapshots,
    LogicalTaskAdmissionGate, LogicalTaskClass, LogicalTaskLane, LogicalTaskObservabilitySampler,
    LogicalTaskOperation, LogicalTaskQueueObservability, SharedMonitorFuseCompatibilityStats,
    SharedMonitorSessionStats, SharedMonitorSessionStatsInput, SharedMonitorSourceStats,
    SharedMonitorTimingStats,
};
use fuser::{
    AccessFlags, BsdFileFlags, CopyFileRangeFlags, Errno, FileAttr, FileHandle, FileType,
    Filesystem, FopenFlags, Generation, INodeNo, InitFlags, IoctlFlags, KernelConfig, LockOwner,
    OpenFlags, PollEvents, PollFlags, PollNotifier, RenameFlags, ReplyAttr, ReplyBmap, ReplyCreate,
    ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyIoctl, ReplyLock, ReplyLseek,
    ReplyOpen, ReplyPoll, ReplyStatfs, ReplyWrite, ReplyXattr, Request, RequestId, TimeOrNow,
    WriteFlags,
};
use crate::errno::{
    EACCES, EBADF, EBUSY, EDEADLK, EEXIST, EFBIG, EILSEQ, EINTR, EINVAL, EIO, EISDIR,
    ENAMETOOLONG, ENODATA, ENOENT, ENOTDIR, ENOTEMPTY, ENOTTY, ENXIO, EOPNOTSUPP, EOVERFLOW, EPERM,
    ERANGE, EROFS, ESTALE, EWOULDBLOCK,
};
use libc::{POLLIN, POLLOUT};
use log::{debug, info, warn};
use rust_hotpath::assemble_read_slice;
use rust_hotpath::pg::{
    prepared_statement_profile_snapshot_lines, DbRepo, DbRepoSourceSnapshot,
    DirectoryEntryMetadata, DirectoryUnlinkOutcome, FileReadMetadata, FileUnlinkOutcome,
    FileWriteAuthorization, GeneratedSharedBlockRows, InodePermissionChange,
    InodePermissionOutcome, InodeTimeValue, InodeTimesChange, InodeTimesOutcome, PersistBlockRow,
    RenameFileReplaceOutcome, RenameNamespaceSource, RenameNamespaceTarget, WriteOwnershipLease,
    WritePersistenceFence,
};
use rust_hotpath::pg::{
    result_decode_profile_snapshot_lines, sql_statement_profile_snapshot_lines,
};
use std::collections::{HashMap, HashSet};
use std::convert::TryInto;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fod_rust_runtime::{
    current_hostname, duration_to_micros, env_var_truthy_with_legacy_alias,
    reloadable_snapshot_from_json, AtimeStat, RuntimeConfig, RuntimeReloadableSettings,
    RuntimeTaskSettings, FOD_SCHEMA_NAME,
};
pub use fod_rust_runtime::{AtimePolicy, LockBackend};

use crate::compatibility::FuseCompatibilitySnapshot;
use crate::copy_plan::{copy_range_bounds, pack_copy_skip_unchanged_runs, CopyRangeBounds};
use crate::errno::persist_error_errno;
use crate::read_cache::{
    FileCacheGenerationGate, ReadBlockCache, ReadFillCoordinator, ReadSequenceState,
};
use crate::startup::FodFuseSettings;
pub(crate) use crate::write_buffer::{ReadCopyDestinationSlice, WriteState};

const ROOT_INO: u64 = 1;
const REQUEST_PID_GROUPS_CACHE_TTL: Duration = Duration::from_millis(500);
// `last_write_at` is diagnostic only, so we update it at most every few seconds.
const CLIENT_WRITE_TOUCH_INTERVAL: Duration = Duration::from_secs(5);
// Default atime updates are throttled so repeated reads do not hammer PostgreSQL.
const ATIME_TOUCH_INTERVAL: Duration = Duration::from_secs(5);
const DEFERRED_UNLINK_NAME_PREFIX: &str = ".fod-unlinked-";
// Linux FIGETBSZ is _IO(0x00, 2) and reports the filesystem block size.
const IOCTL_FIGETBSZ: u32 = 2;
// linux/fs.h also exposes the generic XFS-style fsxattr ioctls.
const IOCTL_FS_IOC_FSGETXATTR: u32 = libc::_IOR::<[u8; IOCTL_FSXATTR_BYTES]>('X' as u32, 31) as u32;
const IOCTL_FS_IOC_FSSETXATTR: u32 = libc::_IOW::<[u8; IOCTL_FSXATTR_BYTES]>('X' as u32, 32) as u32;
const IOCTL_FSXATTR_BYTES: usize = 28;

fn parse_file_mode(mode: &str) -> Result<u16, libc::c_int> {
    rust_hotpath::pg::parse_persisted_posix_mode(mode)
        .map(|mode| mode as u16)
        .map_err(|_| EIO)
}

type SharedBlockRows = Vec<(u64, Arc<[u8]>)>;
type RecentWriteBlockKey = (u64, u64);
type PendingStatfsReply = Arc<Mutex<Option<ReplyStatfs>>>;

#[derive(Debug, Clone, Copy)]
struct StatfsReplyData {
    blocks: u64,
    free_blocks: u64,
    total_inodes: u64,
    free_inodes: u64,
    block_size: u32,
    name_max: u32,
}

struct PendingLockWait {
    cancelled: AtomicBool,
    waiter: thread::Thread,
}

struct PendingLockWaitGuard<'a> {
    waits: &'a Mutex<HashMap<RequestId, Arc<PendingLockWait>>>,
    request_id: RequestId,
}

impl Drop for PendingLockWaitGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut waits) = self.waits.lock() {
            waits.remove(&self.request_id);
        }
    }
}

pub(crate) struct PersistFileBlocksProfileInput<'rows, 'data> {
    pub(crate) file_id: u64,
    pub(crate) file_size: u64,
    pub(crate) block_size: u64,
    pub(crate) total_blocks: u64,
    pub(crate) truncate_pending: bool,
    pub(crate) blocks: &'rows [PersistBlockRow<'data>],
    pub(crate) maintain_copy_crc_table: bool,
    pub(crate) capacity_reservation_token: Option<&'rows str>,
    pub(crate) write_fence: Option<WritePersistenceFence>,
    pub(crate) write_authorization: Option<FileWriteAuthorization>,
}

fn fod_fuse_profile_io_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| env_var_truthy_with_legacy_alias("FOD_PROFILE_IO", false))
}

fn fod_fuse_profile_metadata_cache_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| env_var_truthy_with_legacy_alias("FOD_PROFILE_METADATA_CACHE", false))
}

fn fod_fuse_readdir_batch_metadata_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| env_var_truthy_with_legacy_alias("FOD_READDIR_BATCH_METADATA", true))
}

fn fod_fuse_readdir_register_paths_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| env_var_truthy_with_legacy_alias("FOD_READDIR_REGISTER_PATHS", false))
}

fn readdir_child_entries_to_skip(offset: u64) -> usize {
    offset
        .saturating_sub(2)
        .try_into()
        .unwrap_or(usize::MAX)
}

#[derive(Debug, Clone, Copy, Default)]
struct SetattrMutationFlags {
    mode: bool,
    uid: bool,
    gid: bool,
    size: bool,
    atime: bool,
    mtime: bool,
    ctime: bool,
    crtime: bool,
    chgtime: bool,
    bkuptime: bool,
    flags: bool,
}

impl SetattrMutationFlags {
    fn any(self) -> bool {
        self.mode
            || self.uid
            || self.gid
            || self.size
            || self.atime
            || self.mtime
            || self.ctime
            || self.crtime
            || self.chgtime
            || self.bkuptime
            || self.flags
    }
}

fn ioctl_is_mutating(cmd: u32) -> bool {
    cmd == libc::FS_IOC_SETFLAGS as u32
        || cmd == libc::FS_IOC32_SETFLAGS as u32
        || cmd == IOCTL_FS_IOC_FSSETXATTR
}

macro_rules! fuse_reply_error {
    ($reply:expr, $errno:expr) => {
        $reply.error(Errno::from_i32($errno))
    };
}

#[derive(Debug, Clone)]
struct ParsedAttrs {
    file_attr: FileAttr,
    inode_seed: String,
    xattr_owner_kind: String,
    xattr_owner_id: u64,
}

#[derive(Debug, Clone)]
struct StatfsSnapshot {
    files: u64,
    dirs: u64,
    symlinks: u64,
    total_data_size: u64,
    reserved_data_size: u64,
    max_fs_size_bytes: Option<u64>,
    blocks: u64,
    loaded_at: SystemTime,
}

#[derive(Debug, Clone, Copy)]
struct StatfsCapacity {
    total_bytes: u64,
    available_bytes: u64,
}

struct PayloadCapacityReservation<'a> {
    repo: &'a DbRepo,
    token: Option<String>,
}

impl<'a> PayloadCapacityReservation<'a> {
    fn reserve(repo: &'a DbRepo, bytes: u64) -> Result<Self, String> {
        Ok(Self {
            repo,
            token: repo.reserve_payload_capacity(bytes)?,
        })
    }

    fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }
}

impl Drop for PayloadCapacityReservation<'_> {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            if let Err(err) = self.repo.release_payload_capacity_reservation(&token) {
                warn!(
                    "FOD payload capacity reservation cleanup failed token={} err={}",
                    token, err
                );
            }
        }
    }
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
struct FileWriteOwnership {
    parent_id: Option<u64>,
    name: String,
    file_id: u64,
    owner_key: u64,
    lease: WriteOwnershipLease,
    destination_released: bool,
}

#[derive(Debug, Clone)]
struct FileHandleState {
    path: String,
    file_id: Option<u64>,
    flags: i32,
    atime_touched: bool,
    read_metadata: Option<FileReadMetadata>,
    read_metadata_loaded_at: Option<Instant>,
    write_ownership: Option<FileWriteOwnership>,
}

struct DirectoryHandleState {
    ino: u64,
    path: String,
    directory_id: Option<u64>,
    attrs: FileAttr,
    parent_ino: u64,
    entries: DirectoryEntrySnapshot,
    highest_requested_offset: u64,
}

#[derive(Clone)]
enum DirectoryEntrySnapshot {
    Metadata(Vec<DirectoryEntryMetadata>),
    Names(Vec<String>),
}

struct LockHeartbeatHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct ClientSessionHeartbeatHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn heartbeat_active_write_ownerships(
    repo: &DbRepo,
    fh_table: &Arc<Mutex<HashMap<u64, FileHandleState>>>,
    lease_ttl_seconds: u64,
) -> bool {
    let ownerships = match fh_table.lock() {
        Ok(guard) => {
            let mut seen = HashSet::new();
            guard
                .values()
                .filter_map(|state| state.write_ownership.clone())
                .filter(|ownership| seen.insert(ownership.owner_key))
                .collect::<Vec<_>>()
        }
        Err(_) => {
            warn!("FOD write ownership heartbeat failed to lock fh_table");
            return true;
        }
    };

    let mut failed = false;
    for ownership in ownerships {
        let heartbeat = if ownership.destination_released {
            match ownership.lease.file_fencing_token {
                Some(file_fencing_token) => repo.heartbeat_file_write_ownership(
                    ownership.file_id,
                    ownership.owner_key,
                    file_fencing_token,
                    lease_ttl_seconds,
                ),
                None => Err("deferred unlink writer is missing file fencing token".to_string()),
            }
        } else {
            repo.heartbeat_write_ownership(
                ownership.parent_id,
                &ownership.name,
                Some(ownership.file_id),
                ownership.owner_key,
                ownership.lease,
                lease_ttl_seconds,
            )
        };

        match heartbeat {
            Ok(true) => {
                debug!(
                    "FOD write ownership heartbeat renewed file_id={} owner_key={} destination_released={} destination_token={} file_token={:?}",
                    ownership.file_id,
                    ownership.owner_key,
                    ownership.destination_released,
                    ownership.lease.destination_fencing_token,
                    ownership.lease.file_fencing_token
                );
            }
            Ok(false) => {
                failed = true;
                warn!(
                    "FOD write ownership heartbeat not renewed file_id={} owner_key={} destination_released={} destination_token={} file_token={:?}",
                    ownership.file_id,
                    ownership.owner_key,
                    ownership.destination_released,
                    ownership.lease.destination_fencing_token,
                    ownership.lease.file_fencing_token
                );
            }
            Err(err) => {
                failed = true;
                warn!(
                    "FOD write ownership heartbeat failed file_id={} owner_key={} destination_released={} destination_token={} file_token={:?} err={}",
                    ownership.file_id,
                    ownership.owner_key,
                    ownership.destination_released,
                    ownership.lease.destination_fencing_token,
                    ownership.lease.file_fencing_token,
                    err
                );
            }
        }
    }
    failed
}

struct ClientSessionMaintenanceHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

const RUNTIME_RELOAD_POLL_INTERVAL: Duration = Duration::from_secs(2);
const LOGICAL_TASK_OBSERVABILITY_INTERVAL: Duration = Duration::from_secs(30);
const SHARED_MONITOR_PUBLISH_INTERVAL_MS_DEFAULT: u64 = 5_000;
const SHARED_MONITOR_PUBLISH_INTERVAL_MS_MIN: u64 = 500;
const SHARED_MONITOR_PUBLISH_INTERVAL_MS_MAX: u64 = 60_000;
const SHARED_MONITOR_PUBLISH_INTERVAL_MS_ENV: &str = "FOD_MONITOR_PUBLISH_INTERVAL_MS";
const CLIENT_SESSION_LEASE_TTL_MIN_SECONDS: u64 = 30;
const CLIENT_SESSION_LEASE_TTL_HEARTBEAT_INTERVAL_MULTIPLIER: u64 = 3;
const CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_DEFAULT: u64 = 5_000;
const CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_MIN: u64 = 500;
const CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_MAX: u64 = 60_000;
const CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_ENV: &str = "FOD_SESSION_MAINTENANCE_INTERVAL_MS";

struct SharedMonitorPublisherHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

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
                    let expected_wake = Instant::now() + interval;
                    thread::park_timeout(interval);
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    let schedule_delay = Instant::now().saturating_duration_since(expected_wake);
                    let execution_started = Instant::now();
                    let failed = heartbeat_pg_mount_state(
                        &repo,
                        session_id,
                        &posix_locks,
                        &local_lock_owners,
                        lease_ttl_seconds,
                    );
                    repo.record_heartbeat_observability(
                        schedule_delay,
                        execution_started.elapsed(),
                        failed,
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

fn client_session_maintenance_interval() -> Duration {
    let default = CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_DEFAULT;
    let value = match std::env::var(CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_ENV) {
        Ok(value) => match value.trim().parse::<u64>() {
            Ok(value)
                if (CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_MIN
                    ..=CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_MAX)
                    .contains(&value) =>
            {
                value
            }
            Ok(value) => {
                warn!(
                    "FOD session maintenance interval {}={} outside {}..={}; using {}",
                    CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_ENV,
                    value,
                    CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_MIN,
                    CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_MAX,
                    default
                );
                default
            }
            Err(err) => {
                warn!(
                    "FOD session maintenance interval {} invalid: {}; using {}",
                    CLIENT_SESSION_MAINTENANCE_INTERVAL_MS_ENV, err, default
                );
                default
            }
        },
        Err(_) => default,
    };
    Duration::from_millis(value)
}

fn duration_ceil_seconds(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0))
        .max(1)
}

fn client_session_lease_ttl_seconds(heartbeat_interval: Duration) -> u64 {
    duration_ceil_seconds(heartbeat_interval)
        .saturating_mul(CLIENT_SESSION_LEASE_TTL_HEARTBEAT_INTERVAL_MULTIPLIER)
        .max(CLIENT_SESSION_LEASE_TTL_MIN_SECONDS)
}

impl ClientSessionHeartbeatHandle {
    fn spawn(
        repo: DbRepo,
        session_id: u64,
        fh_table: Arc<Mutex<HashMap<u64, FileHandleState>>>,
        interval: Duration,
        lease_ttl_seconds: u64,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        info!(
            "FOD client session heartbeat: session_id={} interval_ms={} lease_ttl_seconds={}",
            session_id,
            interval.as_millis(),
            lease_ttl_seconds
        );
        let thread = thread::Builder::new()
            .name("fod-session-heartbeat".to_string())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    let session_failed =
                        if let Err(err) = repo.heartbeat_client_session(session_id, lease_ttl_seconds)
                        {
                            warn!(
                                "FOD session heartbeat failed session_id={} err={}",
                                session_id, err
                            );
                            true
                        } else {
                            false
                        };
                    let ownership_failed = heartbeat_active_write_ownerships(
                        &repo,
                        &fh_table,
                        lease_ttl_seconds,
                    );
                    let open_lease_failed =
                        match repo.heartbeat_file_open_leases(session_id, lease_ttl_seconds) {
                            Ok(()) => false,
                            Err(err) => {
                                warn!(
                                    "FOD file open lease heartbeat failed session_id={} err={}",
                                    session_id, err
                                );
                                true
                            }
                        };
                    if session_failed || ownership_failed || open_lease_failed {
                        debug!(
                            "FOD client heartbeat cycle completed with failures session_id={} session_failed={} ownership_failed={} open_lease_failed={}",
                            session_id,
                            session_failed,
                            ownership_failed,
                            open_lease_failed
                        );
                    }
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    thread::park_timeout(interval);
                }
            })
            .map_err(|err| format!("failed to spawn client session heartbeat thread: {err}"))?;
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

impl Drop for ClientSessionHeartbeatHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

impl ClientSessionMaintenanceHandle {
    fn spawn(repo: DbRepo) -> Result<Self, String> {
        let interval = client_session_maintenance_interval();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        info!(
            "FOD client session maintenance: interval_ms={}",
            interval.as_millis()
        );
        let thread = thread::Builder::new()
            .name("fod-session-maintenance".to_string())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    thread::park_timeout(interval);
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    match repo.prune_expired_client_sessions() {
                        Ok(true) => {
                            debug!("FOD client session maintenance pruned expired sessions");
                        }
                        Ok(false) => {}
                        Err(err) => {
                            warn!("FOD client session maintenance prune failed: {}", err);
                        }
                    }
                    match repo.purge_orphaned_unlinked_files(64) {
                        Ok(count) if count > 0 => {
                            info!(
                                "FOD client session maintenance purged deferred unlinks count={}",
                                count
                            );
                        }
                        Ok(_) => {}
                        Err(err) => {
                            warn!("FOD deferred unlink maintenance purge failed: {}", err);
                        }
                    }
                }
            })
            .map_err(|err| format!("failed to spawn client session maintenance thread: {err}"))?;
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

impl Drop for ClientSessionMaintenanceHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn shared_monitor_publish_interval() -> Duration {
    let default = SHARED_MONITOR_PUBLISH_INTERVAL_MS_DEFAULT;
    let value = match std::env::var(SHARED_MONITOR_PUBLISH_INTERVAL_MS_ENV) {
        Ok(value) => match value.trim().parse::<u64>() {
            Ok(value)
                if (SHARED_MONITOR_PUBLISH_INTERVAL_MS_MIN
                    ..=SHARED_MONITOR_PUBLISH_INTERVAL_MS_MAX)
                    .contains(&value) =>
            {
                value
            }
            Ok(value) => {
                warn!(
                    "FOD shared monitor interval {}={} outside {}..={}; using {}",
                    SHARED_MONITOR_PUBLISH_INTERVAL_MS_ENV,
                    value,
                    SHARED_MONITOR_PUBLISH_INTERVAL_MS_MIN,
                    SHARED_MONITOR_PUBLISH_INTERVAL_MS_MAX,
                    default
                );
                default
            }
            Err(err) => {
                warn!(
                    "FOD shared monitor interval {} invalid: {}; using {}",
                    SHARED_MONITOR_PUBLISH_INTERVAL_MS_ENV, err, default
                );
                default
            }
        },
        Err(_) => default,
    };
    Duration::from_millis(value)
}

struct SharedMonitorPublishInput<'a> {
    observability_repo: &'a DbRepo,
    publish_repo: &'a DbRepo,
    session_id: u64,
    publish_interval_millis: u64,
    sample_seq: u64,
    read: &'a LogicalTaskQueueObservability,
    write: &'a LogicalTaskQueueObservability,
    copy: &'a LogicalTaskQueueObservability,
    profile: &'a FodFuseProfileCounters,
    fuse_compatibility: &'a RwLock<Option<SharedMonitorFuseCompatibilityStats>>,
}

fn publish_shared_monitor_sample(input: SharedMonitorPublishInput<'_>) -> Result<(), String> {
    let read_snapshot = input.read.snapshot()?;
    let write_snapshot = input.write.snapshot()?;
    let copy_snapshot = input.copy.snapshot()?;
    let database_snapshot = input.observability_repo.observability_snapshot()?;
    let source_snapshot = input.observability_repo.source_snapshot()?;
    let fuse_compatibility = input
        .fuse_compatibility
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_else(|err| err.into_inner().clone());
    let stats = SharedMonitorSessionStats::from_snapshots(SharedMonitorSessionStatsInput {
        sample_seq: input.sample_seq,
        publish_interval_millis: input.publish_interval_millis,
        read: &read_snapshot,
        write: &write_snapshot,
        copy: &copy_snapshot,
        database: &database_snapshot,
        source: shared_monitor_source_stats(source_snapshot),
        fuse_compatibility,
        timings: input.profile.shared_monitor_timing_stats(),
    });
    input.publish_repo.publish_monitor_session_stats(
        input.session_id,
        env!("CARGO_PKG_VERSION"),
        input.sample_seq,
        &stats.to_json()?,
    )
}

fn shared_monitor_source_stats(snapshot: DbRepoSourceSnapshot) -> SharedMonitorSourceStats {
    SharedMonitorSourceStats {
        data_source_role: snapshot.data_source_role,
        data_source_transaction_read_only: snapshot.transaction_read_only,
        wal_replay_lag_bytes: snapshot.wal_replay_lag_bytes,
    }
}

impl SharedMonitorPublisherHandle {
    fn spawn(input: SharedMonitorPublisherInput) -> Result<Self, String> {
        let SharedMonitorPublisherInput {
            observability_repo,
            publish_repo,
            session_id,
            interval,
            read,
            write,
            copy,
            profile,
            fuse_compatibility,
        } = input;
        let publish_interval_millis = interval.as_millis().min(u128::from(u64::MAX)) as u64;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        info!("FOD shared monitor publisher: session_id={} interval_ms={} table={}.monitor_session_stats", session_id, interval.as_millis(), FOD_SCHEMA_NAME);
        let thread = thread::Builder::new().name("fod-shared-monitor".to_string()).spawn(move || {
            let mut sample_seq = 0_u64;
            loop {
                sample_seq = sample_seq.saturating_add(1);
                if let Err(err) = publish_shared_monitor_sample(SharedMonitorPublishInput {
                        observability_repo: &observability_repo,
                        publish_repo: &publish_repo,
                        session_id,
                        publish_interval_millis,
                        sample_seq,
                        read: read.as_ref(),
                        write: write.as_ref(),
                        copy: copy.as_ref(),
                        profile: profile.as_ref(),
                        fuse_compatibility: fuse_compatibility.as_ref(),
                    }) {
                    warn!("FOD shared monitor publish failed session_id={} sample_seq={} err={}", session_id, sample_seq, err);
                }
                if stop_thread.load(Ordering::Relaxed) { break; }
                thread::park_timeout(interval);
                if stop_thread.load(Ordering::Relaxed) {
                    sample_seq = sample_seq.saturating_add(1);
                    if let Err(err) = publish_shared_monitor_sample(SharedMonitorPublishInput {
                        observability_repo: &observability_repo,
                        publish_repo: &publish_repo,
                        session_id,
                        publish_interval_millis,
                        sample_seq,
                        read: read.as_ref(),
                        write: write.as_ref(),
                        copy: copy.as_ref(),
                        profile: profile.as_ref(),
                        fuse_compatibility: fuse_compatibility.as_ref(),
                    }) {
                        warn!("FOD shared monitor final publish failed session_id={} sample_seq={} err={}", session_id, sample_seq, err);
                    }
                    break;
                }
            }
        }).map_err(|err| format!("failed to spawn shared monitor thread: {err}"))?;
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

struct SharedMonitorPublisherInput {
    observability_repo: DbRepo,
    publish_repo: DbRepo,
    session_id: u64,
    interval: Duration,
    read: Arc<LogicalTaskQueueObservability>,
    write: Arc<LogicalTaskQueueObservability>,
    copy: Arc<LogicalTaskQueueObservability>,
    profile: Arc<FodFuseProfileCounters>,
    fuse_compatibility: Arc<RwLock<Option<SharedMonitorFuseCompatibilityStats>>>,
}

impl Drop for SharedMonitorPublisherHandle {
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

fn client_session_hostname(
    env_hostname: Option<String>,
    system_hostname: Result<String, String>,
) -> String {
    env_hostname
        .and_then(|value| {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        })
        .or_else(|| {
            system_hostname.ok().and_then(|value| {
                let value = value.trim().to_string();
                (!value.is_empty()).then_some(value)
            })
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn heartbeat_pg_mount_state(
    repo: &DbRepo,
    session_id: u64,
    posix_locks: &Arc<Mutex<HashMap<String, Vec<PosixLockRecord>>>>,
    local_lock_owners: &Arc<Mutex<HashSet<u64>>>,
    lock_lease_ttl_seconds: u64,
) -> bool {
    let mut failed = false;

    let local_owners = match local_lock_owners.lock() {
        Ok(guard) => guard.iter().copied().collect::<HashSet<_>>(),
        Err(err) => {
            warn!("FOD lock heartbeat skipped: poisoned owner set: {}", err);
            return true;
        }
    };

    for owner in local_owners.iter().copied() {
        if let Err(err) = repo.touch_client_session_owner_key(session_id, owner) {
            failed = true;
            warn!(
                "FOD session owner heartbeat failed session_id={} owner={} err={}",
                session_id, owner, err
            );
        }
    }

    let (snapshot, lock_map_poisoned) = snapshot_posix_locks(posix_locks);
    failed |= lock_map_poisoned;

    for (resource_key, records) in snapshot {
        let (resource_kind, resource_id) = match FodFuse::lock_resource_kind_id(&resource_key) {
            Ok(value) => value,
            Err(errno) => {
                failed = true;
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
                lock_lease_ttl_seconds,
            ) {
                failed = true;
                warn!(
                    "FOD lock heartbeat failed resource_key={} owner={} start={} end={:?} err={}",
                    resource_key, record.owner, record.start, record.end, err
                );
            }
        }
    }

    failed
}

fn snapshot_posix_locks(
    posix_locks: &Mutex<HashMap<String, Vec<PosixLockRecord>>>,
) -> (Vec<(String, Vec<PosixLockRecord>)>, bool) {
    match posix_locks.lock() {
        Ok(guard) => (
            guard
                .iter()
                .map(|(resource_key, records)| (resource_key.clone(), records.clone()))
                .collect(),
            false,
        ),
        Err(err) => {
            warn!("FOD lock heartbeat recovering poisoned lock map: {}", err);
            let guard = err.into_inner();
            (
                guard
                    .iter()
                    .map(|(resource_key, records)| (resource_key.clone(), records.clone()))
                    .collect(),
                true,
            )
        }
    }
}

fn local_lock_records_for_key(
    posix_locks: &Mutex<HashMap<String, Vec<PosixLockRecord>>>,
    resource_key: &str,
) -> Result<Vec<PosixLockRecord>, libc::c_int> {
    posix_locks
        .lock()
        .map(|guard| guard.get(resource_key).cloned().unwrap_or_default())
        .map_err(|err| {
            warn!(
                "FOD local lock lookup failed resource_key={} err=poisoned lock map: {}",
                resource_key, err
            );
            EIO
        })
}

fn refresh_local_lock_owner_state(
    posix_locks: &Mutex<HashMap<String, Vec<PosixLockRecord>>>,
    local_lock_owners: &Mutex<HashSet<u64>>,
    owner: u64,
) {
    let still_present = match posix_locks.lock() {
        Ok(guard) => guard
            .values()
            .any(|records| records.iter().any(|record| record.owner == owner)),
        Err(err) => {
            // A poisoned local mirror does not prove PostgreSQL lock leases ended.
            warn!(
                "FOD lock owner refresh skipped owner={} err=poisoned lock map: {}",
                owner, err
            );
            return;
        }
    };
    if !still_present {
        if let Ok(mut guard) = local_lock_owners.lock() {
            guard.remove(&owner);
        }
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
            "FOD runtime reload applied profile={:?} log_level={} metadata_cache_ttl={}s statfs_cache_ttl={}s read_cache_blocks={} read_ahead_blocks={} sequential_read_ahead_blocks={} direct_io_read_prefetch_blocks={} small_file_read_threshold_blocks={} workers_read={} workers_read_min_blocks={} workers_write={} workers_write_min_blocks={} persist_buffer_chunk_blocks={} copy_dedupe_enabled={} copy_dedupe_min_blocks={} copy_dedupe_max_blocks={} copy_dedupe_crc_table={}",
            settings.profile,
            settings.log_level,
            settings.metadata_cache_ttl.as_secs(),
            settings.statfs_cache_ttl.as_secs(),
            settings.read_cache_blocks,
            settings.read_ahead_blocks,
            settings.sequential_read_ahead_blocks,
            settings.direct_io_read_prefetch_blocks,
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

struct FileReadMetadataTime {
    atime: SystemTime,
    mtime: SystemTime,
}

impl AtimeStat for FileReadMetadataTime {
    fn atime(&self) -> SystemTime {
        self.atime
    }

    fn mtime(&self) -> SystemTime {
        self.mtime
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
    read_fill_wait_count: AtomicU64,
    read_fill_wait_us: AtomicU64,
    getxattr_count: AtomicU64,
    getxattr_open_file_owner_fast_path_count: AtomicU64,
    getxattr_path_resolution_fallback_count: AtomicU64,
    fetch_block_range_chunk_us: AtomicU64,
    fetch_block_range_parallel_us: AtomicU64,
    assemble_read_slice_us: AtomicU64,
    repo_fetch_block_range_us: AtomicU64,
    repo_assemble_file_slice_us: AtomicU64,
    repo_persist_blocks_us: AtomicU64,
    read_cache_lock_us: AtomicU64,
    read_block_cache_lock_us: AtomicU64,
    cached_read_block_us: AtomicU64,
    recent_write_blocks_lock_us: AtomicU64,
    recent_write_block_us: AtomicU64,
    write_state_lock_us: AtomicU64,
    write_state_clone_us: AtomicU64,
    write_admission_us: AtomicU64,
    write_preflight_us: AtomicU64,
    write_handle_lookup_us: AtomicU64,
    write_start_log_us: AtomicU64,
    write_drain_sibling_states_us: AtomicU64,
    write_take_state_us: AtomicU64,
    write_existing_size_lookup_us: AtomicU64,
    write_unchanged_probe_us: AtomicU64,
    write_merge_sibling_states_us: AtomicU64,
    write_flush_decision_us: AtomicU64,
    write_state_update_us: AtomicU64,
    write_task_complete_us: AtomicU64,
    update_write_buffer_us: AtomicU64,
    flush_write_state_us: AtomicU64,
    flush_prepare_plan_us: AtomicU64,
    flush_execute_persist_plan_us: AtomicU64,
    flush_post_persist_us: AtomicU64,
    prepare_persist_rows_from_block_plan_us: AtomicU64,
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

    pub(crate) fn record_read_block_map_elapsed(&self, elapsed: Duration) {
        Self::add(&self.read_block_map_us, elapsed);
    }

    pub(crate) fn record_read_fill_wait_elapsed(&self, elapsed: Duration) {
        self.read_fill_wait_count.fetch_add(1, Ordering::Relaxed);
        Self::add(&self.read_fill_wait_us, elapsed);
    }

    fn record_getxattr(&self) {
        self.getxattr_count.fetch_add(1, Ordering::Relaxed);
    }

    fn record_getxattr_open_file_owner_fast_path(&self) {
        self.getxattr_open_file_owner_fast_path_count
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_getxattr_path_resolution_fallback(&self) {
        self.getxattr_path_resolution_fallback_count
            .fetch_add(1, Ordering::Relaxed);
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

    pub(crate) fn record_write_admission_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_admission_us, elapsed);
    }

    pub(crate) fn record_write_preflight_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_preflight_us, elapsed);
    }

    pub(crate) fn record_write_handle_lookup_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_handle_lookup_us, elapsed);
    }

    pub(crate) fn record_write_start_log_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_start_log_us, elapsed);
    }

    pub(crate) fn record_write_drain_sibling_states_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_drain_sibling_states_us, elapsed);
    }

    pub(crate) fn record_write_take_state_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_take_state_us, elapsed);
    }

    pub(crate) fn record_write_existing_size_lookup_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_existing_size_lookup_us, elapsed);
    }

    pub(crate) fn record_write_unchanged_probe_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_unchanged_probe_us, elapsed);
    }

    pub(crate) fn record_write_merge_sibling_states_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_merge_sibling_states_us, elapsed);
    }

    pub(crate) fn record_write_flush_decision_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_flush_decision_us, elapsed);
    }

    pub(crate) fn record_write_state_update_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_state_update_us, elapsed);
    }

    pub(crate) fn record_write_task_complete_elapsed(&self, elapsed: Duration) {
        Self::add(&self.write_task_complete_us, elapsed);
    }

    pub(crate) fn record_update_write_buffer_elapsed(&self, elapsed: Duration) {
        Self::add(&self.update_write_buffer_us, elapsed);
    }

    pub(crate) fn record_flush_write_state_elapsed(&self, elapsed: Duration) {
        Self::add(&self.flush_write_state_us, elapsed);
    }

    pub(crate) fn record_flush_prepare_plan_elapsed(&self, elapsed: Duration) {
        Self::add(&self.flush_prepare_plan_us, elapsed);
    }

    pub(crate) fn record_flush_execute_persist_plan_elapsed(&self, elapsed: Duration) {
        Self::add(&self.flush_execute_persist_plan_us, elapsed);
    }

    pub(crate) fn record_flush_post_persist_elapsed(&self, elapsed: Duration) {
        Self::add(&self.flush_post_persist_us, elapsed);
    }

    pub(crate) fn record_prepare_persist_rows_from_block_plan_elapsed(&self, elapsed: Duration) {
        Self::add(&self.prepare_persist_rows_from_block_plan_us, elapsed);
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

    pub(crate) fn shared_monitor_timing_stats(&self) -> SharedMonitorTimingStats {
        SharedMonitorTimingStats {
            fuse_read_total_us: self.fuse_read_total_us.load(Ordering::Relaxed),
            fuse_write_total_us: self.fuse_write_total_us.load(Ordering::Relaxed),
            read_block_map_us: self.read_block_map_us.load(Ordering::Relaxed),
            assemble_read_slice_us: self.assemble_read_slice_us.load(Ordering::Relaxed),
            repo_fetch_block_range_us: self.repo_fetch_block_range_us.load(Ordering::Relaxed),
            repo_persist_blocks_us: self.repo_persist_blocks_us.load(Ordering::Relaxed),
            update_write_buffer_us: self.update_write_buffer_us.load(Ordering::Relaxed),
            flush_write_state_us: self.flush_write_state_us.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn has_activity(&self) -> bool {
        self.fuse_read_total_us.load(Ordering::Relaxed) > 0
            || self.fuse_write_total_us.load(Ordering::Relaxed) > 0
            || self.read_block_map_us.load(Ordering::Relaxed) > 0
            || self.read_fill_wait_count.load(Ordering::Relaxed) > 0
            || self.read_fill_wait_us.load(Ordering::Relaxed) > 0
            || self.getxattr_count.load(Ordering::Relaxed) > 0
            || self
                .getxattr_open_file_owner_fast_path_count
                .load(Ordering::Relaxed)
                > 0
            || self
                .getxattr_path_resolution_fallback_count
                .load(Ordering::Relaxed)
                > 0
            || self.fetch_block_range_chunk_us.load(Ordering::Relaxed) > 0
            || self.fetch_block_range_parallel_us.load(Ordering::Relaxed) > 0
            || self.assemble_read_slice_us.load(Ordering::Relaxed) > 0
            || self.repo_fetch_block_range_us.load(Ordering::Relaxed) > 0
            || self.repo_assemble_file_slice_us.load(Ordering::Relaxed) > 0
            || self.repo_persist_blocks_us.load(Ordering::Relaxed) > 0
            || self.read_cache_lock_us.load(Ordering::Relaxed) > 0
            || self.read_block_cache_lock_us.load(Ordering::Relaxed) > 0
            || self.cached_read_block_us.load(Ordering::Relaxed) > 0
            || self.recent_write_blocks_lock_us.load(Ordering::Relaxed) > 0
            || self.recent_write_block_us.load(Ordering::Relaxed) > 0
            || self.write_state_lock_us.load(Ordering::Relaxed) > 0
            || self.write_state_clone_us.load(Ordering::Relaxed) > 0
            || self.write_admission_us.load(Ordering::Relaxed) > 0
            || self.write_preflight_us.load(Ordering::Relaxed) > 0
            || self.write_handle_lookup_us.load(Ordering::Relaxed) > 0
            || self.write_start_log_us.load(Ordering::Relaxed) > 0
            || self.write_drain_sibling_states_us.load(Ordering::Relaxed) > 0
            || self.write_take_state_us.load(Ordering::Relaxed) > 0
            || self.write_existing_size_lookup_us.load(Ordering::Relaxed) > 0
            || self.write_unchanged_probe_us.load(Ordering::Relaxed) > 0
            || self.write_merge_sibling_states_us.load(Ordering::Relaxed) > 0
            || self.write_flush_decision_us.load(Ordering::Relaxed) > 0
            || self.write_state_update_us.load(Ordering::Relaxed) > 0
            || self.write_task_complete_us.load(Ordering::Relaxed) > 0
            || self.update_write_buffer_us.load(Ordering::Relaxed) > 0
            || self.flush_write_state_us.load(Ordering::Relaxed) > 0
            || self.flush_prepare_plan_us.load(Ordering::Relaxed) > 0
            || self.flush_execute_persist_plan_us.load(Ordering::Relaxed) > 0
            || self.flush_post_persist_us.load(Ordering::Relaxed) > 0
            || self
                .prepare_persist_rows_from_block_plan_us
                .load(Ordering::Relaxed)
                > 0
            || self.clear_read_cache_for_file_us.load(Ordering::Relaxed) > 0
            || self.store_recent_write_blocks_us.load(Ordering::Relaxed) > 0
            || self.reply_data_us.load(Ordering::Relaxed) > 0
            || self.reply_write_us.load(Ordering::Relaxed) > 0
    }

    pub(crate) fn snapshot_lines(&self) -> Vec<String> {
        let mut lines = vec![
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
                "read_fill_wait_count={}",
                self.read_fill_wait_count.load(Ordering::Relaxed)
            ),
            format!(
                "read_fill_wait_us={}",
                self.read_fill_wait_us.load(Ordering::Relaxed)
            ),
            format!(
                "getxattr_count={}",
                self.getxattr_count.load(Ordering::Relaxed)
            ),
            format!(
                "getxattr_open_file_owner_fast_path_count={}",
                self.getxattr_open_file_owner_fast_path_count
                    .load(Ordering::Relaxed)
            ),
            format!(
                "getxattr_path_resolution_fallback_count={}",
                self.getxattr_path_resolution_fallback_count
                    .load(Ordering::Relaxed)
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
                "write_admission_us={}",
                self.write_admission_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_preflight_us={}",
                self.write_preflight_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_handle_lookup_us={}",
                self.write_handle_lookup_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_start_log_us={}",
                self.write_start_log_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_drain_sibling_states_us={}",
                self.write_drain_sibling_states_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_take_state_us={}",
                self.write_take_state_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_existing_size_lookup_us={}",
                self.write_existing_size_lookup_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_unchanged_probe_us={}",
                self.write_unchanged_probe_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_merge_sibling_states_us={}",
                self.write_merge_sibling_states_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_flush_decision_us={}",
                self.write_flush_decision_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_state_update_us={}",
                self.write_state_update_us.load(Ordering::Relaxed)
            ),
            format!(
                "write_task_complete_us={}",
                self.write_task_complete_us.load(Ordering::Relaxed)
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
                "flush_prepare_plan_us={}",
                self.flush_prepare_plan_us.load(Ordering::Relaxed)
            ),
            format!(
                "flush_execute_persist_plan_us={}",
                self.flush_execute_persist_plan_us.load(Ordering::Relaxed)
            ),
            format!(
                "flush_post_persist_us={}",
                self.flush_post_persist_us.load(Ordering::Relaxed)
            ),
            format!(
                "prepare_persist_rows_from_block_plan_us={}",
                self.prepare_persist_rows_from_block_plan_us
                    .load(Ordering::Relaxed)
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
        ];
        lines.extend(prepared_statement_profile_snapshot_lines());
        lines.extend(result_decode_profile_snapshot_lines());
        lines.extend(sql_statement_profile_snapshot_lines());
        lines
    }
}

pub struct FodFuse {
    pub repo: DbRepo,
    telemetry_repo: Option<DbRepo>,
    pub block_size: u64,
    pub write_flush_threshold_bytes: u64,
    pub pg_visible_path: Option<PathBuf>,
    pub lock_backend: LockBackend,
    pub lock_lease_ttl: Duration,
    pub lock_heartbeat_interval: Duration,
    pub lock_poll_interval: Duration,
    pub atime_policy: AtimePolicy,
    entry_cache_ttl: Duration,
    attr_cache_ttl: Duration,
    negative_cache_ttl: Duration,
    reloadable_runtime: Arc<RwLock<RuntimeReloadableSettings>>,
    pub read_only: bool,
    degraded_read_only: bool,
    default_permissions: bool,
    pub use_fuse_context: bool,
    pub fopen_direct_io: bool,
    pub selinux_enabled: bool,
    pub acl_enabled: bool,
    inode_to_path: RwLock<HashMap<u64, String>>,
    path_to_inode: RwLock<HashMap<String, u64>>,
    lookup_refs: Mutex<HashMap<u64, u64>>,
    fh_table: Arc<Mutex<HashMap<u64, FileHandleState>>>,
    handle_operation_locks: Mutex<HashMap<u64, Arc<Mutex<()>>>>,
    directory_fh_table: Mutex<HashMap<u64, Arc<Mutex<DirectoryHandleState>>>>,
    write_ownership_gate: Mutex<()>,
    pub(crate) write_states: Mutex<HashMap<u64, WriteState>>,
    pub(crate) read_block_cache: Mutex<ReadBlockCache>,
    pub(crate) read_fill_coordinator: ReadFillCoordinator,
    pub(crate) file_cache_generations: FileCacheGenerationGate,
    pub(crate) recent_write_blocks: Mutex<HashMap<RecentWriteBlockKey, Arc<[u8]>>>,
    pub(crate) recent_write_blocks_len: AtomicU64,
    pub(crate) read_sequence_state: Mutex<HashMap<u64, ReadSequenceState>>,
    posix_locks: Arc<Mutex<HashMap<String, Vec<PosixLockRecord>>>>,
    pending_lock_waits: Mutex<HashMap<RequestId, Arc<PendingLockWait>>>,
    pending_statfs_replies: Mutex<HashMap<RequestId, PendingStatfsReply>>,
    local_lock_owners: Arc<Mutex<HashSet<u64>>>,
    pid_groups_cache: Mutex<HashMap<RequestPidGroupsCacheKey, CachedPidGroups>>,
    statfs_cache: Mutex<Option<StatfsSnapshot>>,
    last_write_session_touch: Mutex<Option<Instant>>,
    profile: Arc<FodFuseProfileCounters>,
    metadata_profile_readdir_calls: AtomicU64,
    metadata_profile_readdir_entries: AtomicU64,
    metadata_profile_readdir_total_us: AtomicU64,
    metadata_profile_readdir_parent_lookup_us: AtomicU64,
    metadata_profile_readdir_list_us: AtomicU64,
    metadata_profile_readdir_child_lookup_us: AtomicU64,
    metadata_profile_readdir_register_us: AtomicU64,
    metadata_profile_forget_calls: AtomicU64,
    metadata_profile_forget_nlookup: AtomicU64,
    fuse_compatibility: Arc<RwLock<Option<SharedMonitorFuseCompatibilityStats>>>,
    logical_read_tasks: Arc<LogicalTaskQueueObservability>,
    logical_write_tasks: Arc<LogicalTaskQueueObservability>,
    logical_copy_tasks: Arc<LogicalTaskQueueObservability>,
    logical_read_gate: Arc<LogicalTaskAdmissionGate>,
    logical_write_gate: Arc<LogicalTaskAdmissionGate>,
    logical_task_observability: Option<LogicalTaskObservabilitySampler>,
    shared_monitor_publisher: Option<SharedMonitorPublisherHandle>,
    client_session_heartbeat: Option<ClientSessionHeartbeatHandle>,
    client_session_maintenance: Option<ClientSessionMaintenanceHandle>,
    client_session_heartbeat_interval: Duration,
    client_session_lease_ttl_seconds: u64,
    next_fh: Mutex<u64>,
    request_seq: AtomicU64,
    session_id: Option<u64>,
    lock_heartbeat: Option<LockHeartbeatHandle>,
    runtime_reload: Option<RuntimeReloadHandle>,
}

fn assemble_degraded_snapshot(
    snapshot: GeneratedSharedBlockRows,
    offset: u64,
    size: u64,
    block_size: u64,
) -> Result<(Vec<u8>, u64), libc::c_int> {
    if size == 0 {
        return Ok((Vec::new(), 0));
    }
    let requested_end = offset.checked_add(size).ok_or(EINVAL)?;
    let Some((file_size, generation, blocks)) = snapshot else {
        return Err(ESTALE);
    };
    if offset >= file_size {
        return Ok((Vec::new(), generation));
    }
    let end_offset = requested_end.min(file_size);
    let block_size = block_size.max(1);
    let first_block = offset / block_size;
    let last_block = (end_offset - 1) / block_size;
    let blocks = blocks
        .into_iter()
        .filter(|(block, _)| *block >= first_block && *block <= last_block)
        .collect::<Vec<_>>();
    let data = assemble_read_slice(
        first_block,
        last_block,
        offset,
        end_offset,
        block_size,
        &blocks,
    );
    Ok((data, generation))
}

impl FodFuse {
    pub fn new_with_task_settings(
        repo: DbRepo,
        telemetry_repo: Option<DbRepo>,
        settings: FodFuseSettings,
        runtime: &RuntimeConfig,
        task_settings: RuntimeTaskSettings,
    ) -> Self {
        let FodFuseSettings {
            storage,
            cache,
            workers: _workers,
            locks,
            security,
            atime_policy,
            read_only,
            degraded_read_only,
            default_permissions,
            use_fuse_context,
            fopen_direct_io,
        } = settings;
        let mut inode_to_path = HashMap::new();
        let mut path_to_inode = HashMap::new();
        inode_to_path.insert(ROOT_INO, "/".to_string());
        path_to_inode.insert("/".to_string(), ROOT_INO);

        // Zero keeps the existing observation-only fast path. Positive values
        // enable process-local logical admission without changing DB pool limits.
        let logical_read_tasks = Arc::new(LogicalTaskQueueObservability::new(
            LogicalTaskClass::new(LogicalTaskLane::Read, LogicalTaskOperation::FileRead),
            0,
            0,
            0,
        ));
        let logical_write_tasks = Arc::new(LogicalTaskQueueObservability::new(
            LogicalTaskClass::new(LogicalTaskLane::Write, LogicalTaskOperation::FileWrite),
            0,
            0,
            storage.write_flush_threshold_bytes,
        ));
        let logical_copy_tasks = Arc::new(LogicalTaskQueueObservability::new(
            LogicalTaskClass::new(LogicalTaskLane::Write, LogicalTaskOperation::FileCopy),
            0,
            0,
            storage.write_flush_threshold_bytes,
        ));
        let logical_read_gate = Arc::new(LogicalTaskAdmissionGate::new(
            task_settings.read_active_limit,
        ));
        // File writes and copy_file_range share one write-lane gate.
        let logical_write_gate = Arc::new(LogicalTaskAdmissionGate::new(
            task_settings.write_active_limit,
        ));
        let client_session_heartbeat_interval = shared_monitor_publish_interval();
        let client_session_lease_ttl_seconds =
            client_session_lease_ttl_seconds(client_session_heartbeat_interval);

        Self {
            repo,
            telemetry_repo,
            block_size: storage.block_size.max(1),
            write_flush_threshold_bytes: storage.write_flush_threshold_bytes,
            pg_visible_path: storage.pg_visible_path,
            lock_backend: locks.lock_backend,
            lock_lease_ttl: locks.lock_lease_ttl,
            lock_heartbeat_interval: locks.lock_heartbeat_interval,
            lock_poll_interval: locks.lock_poll_interval,
            atime_policy,
            entry_cache_ttl: cache.entry_cache_ttl,
            attr_cache_ttl: cache.attr_cache_ttl,
            negative_cache_ttl: cache.negative_cache_ttl,
            reloadable_runtime: Arc::new(RwLock::new(runtime.reloadable_settings())),
            read_only,
            degraded_read_only,
            default_permissions,
            use_fuse_context,
            fopen_direct_io,
            selinux_enabled: security.selinux_enabled,
            acl_enabled: security.acl_enabled,
            inode_to_path: RwLock::new(inode_to_path),
            path_to_inode: RwLock::new(path_to_inode),
            lookup_refs: Mutex::new(HashMap::new()),
            fh_table: Arc::new(Mutex::new(HashMap::new())),
            handle_operation_locks: Mutex::new(HashMap::new()),
            directory_fh_table: Mutex::new(HashMap::new()),
            write_ownership_gate: Mutex::new(()),
            write_states: Mutex::new(HashMap::new()),
            read_block_cache: Mutex::new(ReadBlockCache::new(
                cache.read_cache_eviction_policy.as_str(),
            )),
            read_fill_coordinator: ReadFillCoordinator::default(),
            file_cache_generations: FileCacheGenerationGate::default(),
            recent_write_blocks: Mutex::new(HashMap::new()),
            recent_write_blocks_len: AtomicU64::new(0),
            read_sequence_state: Mutex::new(HashMap::new()),
            posix_locks: Arc::new(Mutex::new(HashMap::new())),
            pending_lock_waits: Mutex::new(HashMap::new()),
            pending_statfs_replies: Mutex::new(HashMap::new()),
            local_lock_owners: Arc::new(Mutex::new(HashSet::new())),
            pid_groups_cache: Mutex::new(HashMap::new()),
            statfs_cache: Mutex::new(None),
            last_write_session_touch: Mutex::new(None),
            profile: Arc::new(FodFuseProfileCounters::default()),
            metadata_profile_readdir_calls: AtomicU64::new(0),
            metadata_profile_readdir_entries: AtomicU64::new(0),
            metadata_profile_readdir_total_us: AtomicU64::new(0),
            metadata_profile_readdir_parent_lookup_us: AtomicU64::new(0),
            metadata_profile_readdir_list_us: AtomicU64::new(0),
            metadata_profile_readdir_child_lookup_us: AtomicU64::new(0),
            metadata_profile_readdir_register_us: AtomicU64::new(0),
            metadata_profile_forget_calls: AtomicU64::new(0),
            metadata_profile_forget_nlookup: AtomicU64::new(0),
            fuse_compatibility: Arc::new(RwLock::new(None)),
            logical_read_tasks,
            logical_write_tasks,
            logical_copy_tasks,
            logical_read_gate,
            logical_write_gate,
            logical_task_observability: None,
            shared_monitor_publisher: None,
            client_session_heartbeat: None,
            client_session_maintenance: None,
            client_session_heartbeat_interval,
            client_session_lease_ttl_seconds,
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

    fn client_session_repo(&self) -> Option<&DbRepo> {
        // Kazdy mount potrzebuje centralnej authority dla sesji i open leases.
        // Read-only mount preferuje osobny writable control repo, ale gdy jego
        // data repo samo wskazuje primary, moze bezpiecznie uzyc go do DML
        // sterujacego lifecycle. Proba uzycia repliki zostanie odrzucona przez
        // PostgreSQL podczas obowiazkowej rejestracji sesji przy starcie.
        self.telemetry_repo.as_ref().or(Some(&self.repo))
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
        if self.degraded_read_only {
            Duration::ZERO
        } else {
            self.reloadable_runtime().metadata_cache_ttl
        }
    }

    pub(crate) fn entry_cache_ttl_live(&self) -> Duration {
        if self.degraded_read_only {
            Duration::ZERO
        } else {
            self.entry_cache_ttl
        }
    }

    pub(crate) fn attr_cache_ttl_live(&self) -> Duration {
        // Linux chooses an O_APPEND write offset from the cached inode size
        // before sending FUSE_WRITE. A writable mount therefore cannot keep
        // attributes cached across mutations made through another mount.
        if self.degraded_read_only || !self.read_only || self.default_permissions {
            Duration::ZERO
        } else {
            self.attr_cache_ttl
        }
    }

    pub(crate) fn negative_cache_ttl_live(&self) -> Duration {
        if self.degraded_read_only {
            Duration::ZERO
        } else {
            self.negative_cache_ttl
        }
    }

    pub(crate) fn statfs_cache_ttl_live(&self) -> Duration {
        if self.degraded_read_only {
            Duration::ZERO
        } else {
            self.reloadable_runtime().statfs_cache_ttl
        }
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

    fn record_write_admission_elapsed(&self, elapsed: Duration) {
        self.profile.record_write_admission_elapsed(elapsed);
    }

    fn record_write_preflight_elapsed(&self, elapsed: Duration) {
        self.profile.record_write_preflight_elapsed(elapsed);
    }

    fn record_write_handle_lookup_elapsed(&self, elapsed: Duration) {
        self.profile.record_write_handle_lookup_elapsed(elapsed);
    }

    fn record_write_start_log_elapsed(&self, elapsed: Duration) {
        self.profile.record_write_start_log_elapsed(elapsed);
    }

    fn record_write_drain_sibling_states_elapsed(&self, elapsed: Duration) {
        self.profile
            .record_write_drain_sibling_states_elapsed(elapsed);
    }

    fn record_write_take_state_elapsed(&self, elapsed: Duration) {
        self.profile.record_write_take_state_elapsed(elapsed);
    }

    fn record_write_existing_size_lookup_elapsed(&self, elapsed: Duration) {
        self.profile
            .record_write_existing_size_lookup_elapsed(elapsed);
    }

    fn record_write_unchanged_probe_elapsed(&self, elapsed: Duration) {
        self.profile.record_write_unchanged_probe_elapsed(elapsed);
    }

    fn record_write_merge_sibling_states_elapsed(&self, elapsed: Duration) {
        self.profile
            .record_write_merge_sibling_states_elapsed(elapsed);
    }

    fn record_write_flush_decision_elapsed(&self, elapsed: Duration) {
        self.profile.record_write_flush_decision_elapsed(elapsed);
    }

    fn record_write_state_update_elapsed(&self, elapsed: Duration) {
        self.profile.record_write_state_update_elapsed(elapsed);
    }

    fn record_write_task_complete_elapsed(&self, elapsed: Duration) {
        self.profile.record_write_task_complete_elapsed(elapsed);
    }

    pub(crate) fn record_update_write_buffer_elapsed(&self, elapsed: Duration) {
        self.profile.record_update_write_buffer_elapsed(elapsed);
    }

    pub(crate) fn record_flush_write_state_elapsed(&self, elapsed: Duration) {
        self.profile.record_flush_write_state_elapsed(elapsed);
    }

    pub(crate) fn record_flush_prepare_plan_elapsed(&self, elapsed: Duration) {
        self.profile.record_flush_prepare_plan_elapsed(elapsed);
    }

    pub(crate) fn record_flush_execute_persist_plan_elapsed(&self, elapsed: Duration) {
        self.profile
            .record_flush_execute_persist_plan_elapsed(elapsed);
    }

    pub(crate) fn record_flush_post_persist_elapsed(&self, elapsed: Duration) {
        self.profile.record_flush_post_persist_elapsed(elapsed);
    }

    pub(crate) fn record_read_block_map_elapsed(&self, elapsed: Duration) {
        self.profile.record_read_block_map_elapsed(elapsed);
    }

    pub(crate) fn record_read_fill_wait_elapsed(&self, elapsed: Duration) {
        self.profile.record_read_fill_wait_elapsed(elapsed);
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

    pub(crate) fn record_prepare_persist_rows_from_block_plan_elapsed(&self, elapsed: Duration) {
        self.profile
            .record_prepare_persist_rows_from_block_plan_elapsed(elapsed);
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
    ) -> Result<SharedBlockRows, String> {
        let started = Instant::now();
        let result =
            self.repo
                .fetch_block_range_shared(file_id, first_block, last_block, block_size);
        self.record_repo_fetch_block_range_elapsed(started.elapsed());
        result.map(|mut blocks| {
            blocks.sort_unstable_by_key(|(block_index, _)| *block_index);
            blocks
        })
    }

    pub(crate) fn load_block_for_update_profiled(
        &self,
        file_id: u64,
        block_index: u64,
        block_size: u64,
    ) -> Result<Option<Vec<u8>>, String> {
        let started = Instant::now();
        let result = self
            .repo
            .load_block_authoritative(file_id, block_index, block_size);
        self.record_repo_fetch_block_range_elapsed(started.elapsed());
        result
    }

    pub(crate) fn assemble_file_slice_for_update_profiled(
        &self,
        file_id: u64,
        first_block: u64,
        last_block: u64,
        offset: u64,
        end_offset: u64,
        block_size: u64,
    ) -> Result<Vec<u8>, String> {
        let started = Instant::now();
        let result = self.repo.assemble_file_slice_authoritative(
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

    pub(crate) fn persist_file_blocks_profiled<'rows, 'data>(
        &self,
        input: PersistFileBlocksProfileInput<'rows, 'data>,
    ) -> Result<(), String> {
        let PersistFileBlocksProfileInput {
            file_id,
            file_size,
            block_size,
            total_blocks,
            truncate_pending,
            blocks,
            maintain_copy_crc_table,
            capacity_reservation_token,
            write_fence,
            write_authorization,
        } = input;
        let started = Instant::now();
        let result = self
            .repo
            .persist_file_blocks_with_crc_flag_reservation_fence_and_authorization(
                file_id,
                file_size,
                block_size,
                total_blocks,
                truncate_pending,
                blocks,
                maintain_copy_crc_table,
                capacity_reservation_token,
                write_fence,
                write_authorization.as_ref(),
            );
        self.record_repo_persist_blocks_elapsed(started.elapsed());
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

    pub(crate) fn logical_task_active_limits(&self) -> (u64, u64) {
        (
            self.logical_read_gate.active_task_limit(),
            self.logical_write_gate.active_task_limit(),
        )
    }

    fn logical_task_trackers(&self) -> Vec<Arc<LogicalTaskQueueObservability>> {
        vec![
            Arc::clone(&self.logical_read_tasks),
            Arc::clone(&self.logical_write_tasks),
            Arc::clone(&self.logical_copy_tasks),
        ]
    }

    fn log_logical_task_observability(&self, stage: &str) {
        let trackers = self.logical_task_trackers();
        log_logical_task_snapshots(stage, &trackers);
    }

    pub(crate) fn maybe_touch_client_session_write(&self) {
        if self.read_only {
            return;
        }
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

        let Some(repo) = self.client_session_repo() else {
            return;
        };
        if let Err(err) = repo.touch_client_session_write(session_id) {
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
        let (lookup_ref_inodes, lookup_ref_total) = self
            .lookup_refs
            .lock()
            .map(|guard| {
                (
                    guard.len(),
                    guard.values().copied().fold(0u64, u64::saturating_add),
                )
            })
            .unwrap_or((0, 0));
        let mut samples = Vec::new();
        if let Ok(guard) = self.path_to_inode.read() {
            for (path, ino) in guard.iter().take(5) {
                samples.push(format!("{path}->{ino}"));
            }
        }
        format!(
            "FodFuseSnapshot{{read_only={}, use_fuse_context={}, fopen_direct_io={}, block_size={}, write_flush_threshold_bytes={}, read_cache_blocks={}, read_ahead_blocks={}, sequential_read_ahead_blocks={}, direct_io_read_prefetch_blocks={}, small_file_read_threshold_blocks={}, workers_read={}, workers_read_min_blocks={}, workers_write={}, workers_write_min_blocks={}, atime_policy={:?}, lock_backend={:?}, lock_lease_ttl_secs={}, lock_heartbeat_interval_secs={}, lock_poll_interval_secs={}, client_session_heartbeat_interval_secs={}, client_session_lease_ttl_secs={}, copy_dedupe_enabled={}, copy_dedupe_min_blocks={}, copy_dedupe_max_blocks={}, copy_dedupe_crc_table={}, selinux_enabled={}, acl_enabled={}, inode_to_path={}, path_to_inode={}, fh_table={}, fh_table_file_ids={}, fh_table_flags={}, fh_table_atime_touched={}, write_states={}, read_cache_entries={}, read_sequences={}, posix_locks={}, readdir_calls={}, readdir_entries={}, readdir_total_us={}, readdir_parent_lookup_us={}, readdir_list_us={}, readdir_child_lookup_us={}, readdir_register_us={}, forget_calls={}, forget_nlookup={}, lookup_ref_inodes={}, lookup_ref_total={}, samples=[{}]}}",
            self.read_only,
            self.use_fuse_context,
            self.fopen_direct_io,
            self.block_size,
            self.write_flush_threshold_bytes,
            live.read_cache_blocks,
            live.read_ahead_blocks,
            live.sequential_read_ahead_blocks,
            live.direct_io_read_prefetch_blocks,
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
            self.client_session_heartbeat_interval.as_secs_f64(),
            self.client_session_lease_ttl_seconds,
            live.copy_dedupe_enabled,
            live.copy_dedupe_min_blocks,
            live.copy_dedupe_max_blocks,
            live.copy_dedupe_crc_table,
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
            self.metadata_profile_readdir_calls.load(Ordering::Relaxed),
            self.metadata_profile_readdir_entries.load(Ordering::Relaxed),
            self.metadata_profile_readdir_total_us.load(Ordering::Relaxed),
            self.metadata_profile_readdir_parent_lookup_us
                .load(Ordering::Relaxed),
            self.metadata_profile_readdir_list_us.load(Ordering::Relaxed),
            self.metadata_profile_readdir_child_lookup_us
                .load(Ordering::Relaxed),
            self.metadata_profile_readdir_register_us
                .load(Ordering::Relaxed),
            self.metadata_profile_forget_calls.load(Ordering::Relaxed),
            self.metadata_profile_forget_nlookup.load(Ordering::Relaxed),
            lookup_ref_inodes,
            lookup_ref_total,
            samples.join(", ")
        )
    }

    fn current_statfs_snapshot(&self) -> Result<StatfsSnapshot, String> {
        let (files, dirs, symlinks, total_data_size, reserved_data_size, max_fs_size_bytes) =
            self.repo.statfs_snapshot()?;
        let accounted_data_size = total_data_size.saturating_add(reserved_data_size);
        let blocks =
            accounted_data_size.saturating_add(self.block_size.saturating_sub(1)) / self.block_size;
        Ok(StatfsSnapshot {
            files,
            dirs,
            symlinks,
            total_data_size,
            reserved_data_size,
            max_fs_size_bytes,
            blocks,
            loaded_at: SystemTime::now(),
        })
    }

    fn register_statfs_reply(&self, request_id: RequestId, reply: ReplyStatfs) {
        let pending = Arc::new(Mutex::new(Some(reply)));
        let previous = self
            .pending_statfs_replies
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(request_id, pending);
        debug_assert!(previous.is_none(), "duplicate in-flight FUSE request id");
    }

    fn finish_statfs_reply(
        &self,
        request_id: RequestId,
        result: Result<StatfsReplyData, libc::c_int>,
    ) {
        let pending = self
            .pending_statfs_replies
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&request_id);
        let Some(pending) = pending else {
            // The kernel interrupted this request while the statfs query was
            // in flight. Its EINTR reply has already been sent.
            return;
        };
        let reply = pending
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take();
        let Some(reply) = reply else {
            return;
        };
        match result {
            Ok(stats) => reply.statfs(
                stats.blocks,
                stats.free_blocks,
                stats.free_blocks,
                stats.total_inodes,
                stats.free_inodes,
                stats.block_size,
                stats.name_max,
                stats.block_size,
            ),
            Err(errno) => fuse_reply_error!(reply, errno),
        }
    }

    fn interrupt_statfs(&self, request_id: RequestId) -> bool {
        let pending = self
            .pending_statfs_replies
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&request_id);
        let Some(pending) = pending else {
            return false;
        };
        if let Some(reply) = pending
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take()
        {
            fuse_reply_error!(reply, EINTR);
        }
        true
    }

    pub(crate) fn invalidate_statfs_cache(&self) {
        if let Ok(mut guard) = self.statfs_cache.lock() {
            *guard = None;
        }
    }

    #[cfg(unix)]
    fn statvfs_capacity(path: &Path) -> Result<StatfsCapacity, String> {
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
        let fragment_size = stats.f_frsize;
        Ok(StatfsCapacity {
            total_bytes: fragment_size.saturating_mul(stats.f_blocks),
            available_bytes: fragment_size.saturating_mul(stats.f_bavail),
        })
    }

    #[cfg(not(unix))]
    fn statvfs_capacity(_path: &Path) -> Result<StatfsCapacity, String> {
        Err("statvfs is unavailable on this platform".to_string())
    }

    fn statfs_capacity(
        &self,
        max_fs_size_bytes: Option<u64>,
    ) -> Result<Option<StatfsCapacity>, String> {
        let visible = self
            .pg_visible_path
            .as_ref()
            .map(|path| Self::statvfs_capacity(path))
            .transpose()?;
        Ok(match (max_fs_size_bytes, visible) {
            (Some(requested), Some(visible)) => Some(StatfsCapacity {
                total_bytes: requested.min(visible.total_bytes),
                available_bytes: visible.available_bytes,
            }),
            (Some(requested), None) => Some(StatfsCapacity {
                total_bytes: requested,
                available_bytes: requested,
            }),
            (None, Some(visible)) => Some(visible),
            (None, None) => None,
        })
    }

    fn cached_statfs_snapshot(&self) -> Result<StatfsSnapshot, String> {
        let ttl = self.statfs_cache_ttl_live();
        if ttl.is_zero() {
            return self.current_statfs_snapshot();
        }

        if let Ok(guard) = self.statfs_cache.lock() {
            if let Some(snapshot) = guard.as_ref() {
                if let Ok(age) = snapshot.loaded_at.elapsed() {
                    if age <= ttl {
                        return Ok(snapshot.clone());
                    }
                } else {
                    return Ok(snapshot.clone());
                }
            }
        }

        let snapshot = self.current_statfs_snapshot()?;
        if let Ok(mut guard) = self.statfs_cache.lock() {
            *guard = Some(snapshot.clone());
        }
        Ok(snapshot)
    }

    fn normalize_path(path: &str) -> String {
        // Whitespace is valid pathname data. Normalize separators only; trim()
        // would redirect names whose final component ends in whitespace.
        let mut value = path.to_string();
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

    fn validate_component_name(name: &OsStr) -> Result<(), libc::c_int> {
        const NAME_MAX: usize = 255;
        if name.as_bytes().len() > NAME_MAX {
            Err(ENAMETOOLONG)
        } else if Self::os_str_as_utf8(name).is_err() {
            // PostgreSQL text cannot preserve invalid UTF-8 path bytes. Reject
            // them before any lossy conversion could alias another name.
            Err(EILSEQ)
        } else {
            Ok(())
        }
    }

    fn os_str_as_utf8(value: &OsStr) -> Result<&str, libc::c_int> {
        value.to_str().ok_or(EILSEQ)
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

    fn stable_inode(
        &self,
        obj_type: &str,
        inode_seed: &str,
        entry_id: u64,
    ) -> Result<u64, libc::c_int> {
        if inode_seed.trim().is_empty() {
            return Err(EIO);
        }
        let normalized_obj_type = if obj_type == "hardlink" {
            "file"
        } else {
            obj_type
        };
        let payload = format!("{normalized_obj_type}:{inode_seed}");
        let inode = Self::hash_inode64(payload.as_bytes()) & i64::MAX as u64;
        if inode == 0 {
            Ok(self.logical_inode(obj_type, entry_id))
        } else {
            Ok(inode)
        }
    }

    fn parse_time(value: &str) -> Result<SystemTime, libc::c_int> {
        let value = value.trim();
        if value.is_empty() {
            return Err(EIO);
        }
        let normalized = value.replace('T', " ");
        let naive_formats = ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"];
        for fmt in naive_formats {
            if let Ok(naive) = NaiveDateTime::parse_from_str(&normalized, fmt) {
                let dt = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);
                return Self::system_time_from_unix_parts(
                    dt.timestamp(),
                    dt.timestamp_subsec_nanos(),
                )
                .ok_or(EIO);
            }
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(&value.replace(' ', "T")) {
            return Self::system_time_from_unix_parts(
                dt.timestamp(),
                dt.timestamp_subsec_nanos(),
            )
            .ok_or(EIO);
        }
        Err(EIO)
    }

    fn system_time_from_unix_parts(seconds: i64, nanoseconds: u32) -> Option<SystemTime> {
        if seconds >= 0 {
            return UNIX_EPOCH.checked_add(Duration::new(seconds as u64, nanoseconds));
        }

        let absolute_seconds = seconds.unsigned_abs();
        let since_epoch = if nanoseconds == 0 {
            Duration::from_secs(absolute_seconds)
        } else {
            Duration::new(absolute_seconds.checked_sub(1)?, 1_000_000_000 - nanoseconds)
        };
        UNIX_EPOCH.checked_sub(since_epoch)
    }

    fn file_type_from_special(special_type: &str) -> Option<FileType> {
        match special_type {
            "fifo" => Some(FileType::NamedPipe),
            "char" => Some(FileType::CharDevice),
            "block" => Some(FileType::BlockDevice),
            _ => None,
        }
    }

    fn decode_fuse_rdev(rdev: u32) -> (u32, u32) {
        let major = (rdev >> 8) & 0x0fff;
        let minor = (rdev & 0x00ff) | ((rdev >> 12) & 0x0fff00);
        (major, minor)
    }

    fn encode_fuse_rdev(major: u32, minor: u32) -> Result<u32, libc::c_int> {
        if major > 0x0fff || minor > 0x0fffff {
            return Err(EIO);
        }
        Ok((major << 8) | (minor & 0x00ff) | ((minor & 0x0fff00) << 12))
    }

    fn directory_entry_file_type(
        entry_kind: &str,
        special_type: Option<&str>,
    ) -> Result<FileType, libc::c_int> {
        match entry_kind {
            "dir" => Ok(FileType::Directory),
            "symlink" => Ok(FileType::Symlink),
            "file" | "hardlink" => match special_type {
                Some(special_type) => Self::file_type_from_special(special_type).ok_or(EIO),
                None => Ok(FileType::RegularFile),
            },
            _ => Err(EIO),
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
        let authority = self.repo.authoritative_view();
        let resolved = authority.resolve_path(path).map_err(|_| EIO)?;
        match resolved.kind.as_deref() {
            Some("hardlink") => {
                let hardlink_id = resolved.entry_id.ok_or(EIO)?;
                authority.get_hardlink_file_id(hardlink_id).map_err(|_| EIO)
            }
            Some("file") => Ok(resolved.entry_id),
            _ => Ok(None),
        }
    }

    fn resolved_entry_for_path(
        &self,
        path: &str,
    ) -> Result<(Option<String>, Option<u64>), libc::c_int> {
        let resolved = self
            .repo
            .authoritative_view()
            .resolve_path(path)
            .map_err(|_| EIO)?;
        Ok((resolved.kind, resolved.entry_id))
    }

    fn current_group_ids() -> HashSet<u32> {
        let mut group_ids = HashSet::new();
        unsafe {
            let count = libc::getgroups(0, std::ptr::null_mut());
            if count > 0 {
                let mut groups = vec![0 as libc::gid_t; count as usize];
                let rc = libc::getgroups(count, groups.as_mut_ptr());
                if rc >= 0 {
                    group_ids.extend(groups);
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

    fn request_identity(&self, req: &Request) -> SubjectIdentity {
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
            return Err(EINVAL);
        }
        let version = u32::from_le_bytes(value[0..4].try_into().map_err(|_| EINVAL)?);
        if version != 0x0002 {
            return Err(EINVAL);
        }
        let mut entries = Vec::new();
        let mut idx = 4usize;
        while idx + 8 <= value.len() {
            let tag = u16::from_le_bytes(value[idx..idx + 2].try_into().map_err(|_| EINVAL)?);
            let perm = u16::from_le_bytes(
                value[idx + 2..idx + 4]
                    .try_into()
                    .map_err(|_| EINVAL)?,
            );
            let mut id = i32::from_le_bytes(
                value[idx + 4..idx + 8]
                    .try_into()
                    .map_err(|_| EINVAL)?,
            );
            if tag == 0x0001 || tag == 0x0004 || tag == 0x0020 || tag == 0x0010 {
                id = -1;
            }
            entries.push(PosixAclEntry { tag, perm, id });
            idx += 8;
        }
        if idx != value.len() {
            return Err(EINVAL);
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

    fn mode_from_acl_entries(
        entries: &[PosixAclEntry],
        current_perm: u16,
    ) -> Result<u16, libc::c_int> {
        let mut user_obj_perm = None;
        let mut group_obj_perm = None;
        let mut mask_perm = None;
        let mut other_perm = None;
        let mut has_named_entry = false;
        for entry in entries {
            let perm = entry.perm & 0o7;
            match entry.tag {
                0x0001 => user_obj_perm = Some(perm),
                0x0002 | 0x0008 => has_named_entry = true,
                0x0004 => group_obj_perm = Some(perm),
                0x0010 => mask_perm = Some(perm),
                0x0020 => other_perm = Some(perm),
                _ => {}
            }
        }
        if has_named_entry && mask_perm.is_none() {
            return Err(EINVAL);
        }
        let user_bits = user_obj_perm.ok_or(EINVAL)?;
        let group_bits = mask_perm.or(group_obj_perm).ok_or(EINVAL)?;
        let other_bits = other_perm.ok_or(EINVAL)?;
        Ok((current_perm & !0o777) | (user_bits << 6) | (group_bits << 3) | other_bits)
    }

    fn acl_access_mode_bits(value: &[u8]) -> Result<u16, libc::c_int> {
        let entries = Self::parse_posix_acl_xattr(value)?;
        Ok(Self::mode_from_acl_entries(&entries, 0)? & 0o777)
    }

    fn acl_allows(
        &self,
        path: &str,
        attrs: &FileAttr,
        mode: i32,
        subject: &SubjectIdentity,
    ) -> Result<bool, libc::c_int> {
        let authority = self.repo.authoritative_view();
        let current_attrs = self
            .attrs_for_path_from_repo(&authority, path)?
            .ok_or(ENOENT)?;
        if current_attrs.file_attr.ino != attrs.ino {
            return Ok(false);
        }
        let attrs = &current_attrs.file_attr;
        if !self.acl_enabled {
            return Ok(self.can_access(subject, attrs, mode));
        }
        let acl_value = authority
            .fetch_xattr_value(path, "system.posix_acl_access")
            .map_err(|_| EIO)?;
        if let Some(value) = acl_value {
            let entries = Self::parse_posix_acl_xattr(&value)?;
            Ok(self.acl_permission_from_entries(subject, &entries, attrs, mode))
        } else {
            Ok(self.can_access(subject, attrs, mode))
        }
    }

    fn acl_allows_for_attrs(
        &self,
        attrs: &ParsedAttrs,
        mode: i32,
        subject: &SubjectIdentity,
    ) -> Result<bool, libc::c_int> {
        if !self.acl_enabled {
            return Ok(self.can_access(subject, &attrs.file_attr, mode));
        }
        let authority = self.repo.authoritative_view();
        let acl_value = authority
            .fetch_xattr_value_for_owner(
                &attrs.xattr_owner_kind,
                attrs.xattr_owner_id,
                "system.posix_acl_access",
            )
            .map_err(|_| EIO)?;
        if let Some(value) = acl_value {
            let entries = Self::parse_posix_acl_xattr(&value)?;
            Ok(self.acl_permission_from_entries(subject, &entries, &attrs.file_attr, mode))
        } else {
            Ok(self.can_access(subject, &attrs.file_attr, mode))
        }
    }

    fn check_path_search_permissions(
        &self,
        path: &str,
        subject: &SubjectIdentity,
    ) -> Result<(), libc::c_int> {
        let normalized = Self::normalize_path(path);
        let components: Vec<&str> = normalized
            .split('/')
            .filter(|component| !component.is_empty())
            .collect();
        if components.is_empty() {
            return Ok(());
        }
        let parent_component_count = components.len().saturating_sub(1);
        let mut directory = "/".to_string();

        for index in 0..=parent_component_count {
            let attrs = self.attrs_for_path(&directory)?.ok_or(ENOENT)?;
            if !self.acl_allows(&directory, &attrs.file_attr, libc::X_OK, subject)? {
                return Err(EACCES);
            }
            if index < parent_component_count {
                directory = if directory == "/" {
                    format!("/{}", components[index])
                } else {
                    format!("{}/{}", directory, components[index])
                };
            }
        }
        Ok(())
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
        start_a <= end_b && start_b <= end_a
    }

    fn update_owner_lock_range(
        records: &[PosixLockRecord],
        owner: u64,
        start: u64,
        end: u64,
        replacement: Option<PosixLockRecord>,
    ) -> Vec<PosixLockRecord> {
        let mut updated = Vec::with_capacity(records.len() + 1);
        for record in records {
            if record.owner != owner
                || !Self::range_overlaps(start, Some(end), record.start, record.end)
            {
                updated.push(record.clone());
                continue;
            }

            if record.start < start {
                updated.push(PosixLockRecord {
                    end: Some(start - 1),
                    ..record.clone()
                });
            }

            let record_end = record.end.unwrap_or(u64::MAX);
            if end < record_end {
                updated.push(PosixLockRecord {
                    start: end + 1,
                    ..record.clone()
                });
            }
        }
        if let Some(replacement) = replacement {
            updated.push(replacement);
        }
        updated
    }

    fn resource_key_for_lock(
        &self,
        fh: u64,
        ino: u64,
    ) -> Result<(String, String), libc::c_int> {
        let handle = self.file_handle_state_for_handle(fh).ok_or(EBADF)?;
        let file_id = handle.file_id.ok_or(ENOENT)?;
        let attrs = self
            .attrs_for_open_file_id(file_id)?
            .ok_or(ENOENT)?;
        if attrs.file_attr.ino.0 != ino {
            return Err(ENOENT);
        }
        Ok((format!("file:{file_id}"), handle.path))
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
                    "{}\t{}\t{}\t{}\t{}",
                    record.owner,
                    record.typ,
                    record.start,
                    record
                        .end
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    record.pid,
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
            let pid = match parts.next() {
                Some(value) => value.parse::<u32>().map_err(|_| EIO)?,
                None => 0,
            };
            if parts.next().is_some() {
                return Err(EIO);
            }
            if !matches!(typ, libc::F_RDLCK | libc::F_WRLCK)
                || start > i64::MAX as u64
                || end.is_some_and(|value| value > i64::MAX as u64 || value < start)
            {
                return Err(EIO);
            }
            records.push(PosixLockRecord {
                owner,
                typ,
                start,
                end,
                pid,
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
            local_lock_records_for_key(&self.posix_locks, resource_key)
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

    fn update_pg_lock_range(
        &self,
        resource_key: &str,
        owner: u64,
        start: u64,
        end: u64,
        typ: i32,
        pid: u32,
    ) -> Result<bool, libc::c_int> {
        let (resource_kind, resource_id) = Self::lock_resource_kind_id(resource_key)?;
        let lease_ttl_seconds = self.lock_lease_ttl.as_secs_f64().ceil().max(1.0) as u64;
        let updated = self
            .repo
            .update_lock_range_state_blob_for_owner(
                &resource_kind,
                resource_id,
                owner,
                lease_ttl_seconds,
                |state| {
                    let records = Self::lock_records_from_blob(state)
                        .map_err(|errno| format!("invalid lock state errno={errno}"))?;
                    if typ != libc::F_UNLCK
                        && Self::lock_conflict(&records, owner, typ, start, end).is_some()
                    {
                        return Ok(None);
                    }
                    let records = Self::update_owner_lock_range(
                        &records,
                        owner,
                        start,
                        end,
                        (typ != libc::F_UNLCK).then_some(PosixLockRecord {
                            owner,
                            typ,
                            start,
                            end: Some(end),
                            pid,
                        }),
                    );
                    let owner_records = records
                        .iter()
                        .filter(|record| record.owner == owner)
                        .cloned()
                        .collect::<Vec<_>>();
                    Ok(Some(Self::lock_records_to_blob(&owner_records)))
                },
            )
            .map_err(|err| {
                warn!(
                    "FOD atomic lock range update failed resource_key={} owner={} err={}",
                    resource_key, owner, err
                );
                EIO
            })?;

        if updated {
            self.load_lock_records_for_key(resource_key)?;
        }
        Ok(updated)
    }

    fn register_local_lock_owner(&self, owner: u64) {
        if let Ok(mut guard) = self.local_lock_owners.lock() {
            guard.insert(owner);
        }
        if self.lock_backend_is_pg() {
            if let Some(session_id) = self.session_id {
                if let Err(err) = self.repo.touch_client_session_owner_key(session_id, owner) {
                    warn!(
                        "FOD session owner tracking failed session_id={} owner={} err={}",
                        session_id, owner, err
                    );
                }
            }
        }
    }

    fn refresh_local_lock_owner_state(&self, owner: u64) {
        refresh_local_lock_owner_state(&self.posix_locks, &self.local_lock_owners, owner);
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

    fn file_handle_state_for_handle(&self, fh: u64) -> Option<FileHandleState> {
        self.fh_table
            .lock()
            .ok()
            .and_then(|guard| guard.get(&fh).cloned())
    }

    fn file_id_for_handle(&self, fh: u64, _ino: u64) -> Result<Option<u64>, libc::c_int> {
        let state = self.file_handle_state_for_handle(fh).ok_or(EBADF)?;
        Ok(state.file_id)
    }

    fn file_id_for_handle_or_errno(&self, fh: u64, ino: u64) -> Result<u64, libc::c_int> {
        self.file_id_for_handle(fh, ino)?.ok_or(ENOENT)
    }


    fn should_cache_file_read_metadata(
        read_only: bool,
        fopen_direct_io: bool,
        atime_policy: AtimePolicy,
        handle_flags: i32,
        metadata_cache_ttl: Duration,
    ) -> bool {
        if !fopen_direct_io {
            return false;
        }
        if read_only {
            // Preserve existing read-only/replica direct-I/O handle caching.
            return true;
        }
        atime_policy == AtimePolicy::NoAtime
            && (handle_flags & libc::O_ACCMODE) == libc::O_RDONLY
            && !metadata_cache_ttl.is_zero()
    }

    pub(crate) fn invalidate_read_metadata_for_file(&self, file_id: u64) {
        if let Ok(mut guard) = self.fh_table.lock() {
            for state in guard.values_mut() {
                if state.file_id == Some(file_id) {
                    state.read_metadata = None;
                    state.read_metadata_loaded_at = None;
                }
            }
        }
    }

    fn file_read_metadata_for_handle(
        &self,
        fh: u64,
        file_id: u64,
    ) -> Result<Option<FileReadMetadata>, String> {
        // Replica/read-only direct-I/O keeps the pre-3.4.16 handle-cache
        // behavior and does not need to consult reloadable TTL on every read.
        let metadata_cache_ttl = if self.read_only {
            Duration::ZERO
        } else {
            self.metadata_cache_ttl_live()
        };

        if let Some(metadata) = self.fh_table.lock().ok().and_then(|mut guard| {
            guard.get_mut(&fh).and_then(|state| {
                if state.file_id != Some(file_id) {
                    return None;
                }

                if !Self::should_cache_file_read_metadata(
                    self.read_only,
                    self.fopen_direct_io,
                    self.atime_policy,
                    state.flags,
                    metadata_cache_ttl,
                ) {
                    // Drop a previously cached snapshot before consulting the
                    // repository so a failed refresh cannot resurrect it if
                    // runtime policy later enables caching again.
                    state.read_metadata = None;
                    state.read_metadata_loaded_at = None;
                    return None;
                }

                let fresh = self.read_only
                    || state
                        .read_metadata_loaded_at
                        .is_some_and(|loaded_at| loaded_at.elapsed() < metadata_cache_ttl);

                if !fresh {
                    // An expired snapshot must not survive a failed refresh.
                    state.read_metadata = None;
                    state.read_metadata_loaded_at = None;
                    return None;
                }

                state.read_metadata.clone()
            })
        }) {
            return Ok(Some(metadata));
        }

        let metadata = self.repo.file_read_metadata(file_id)?;
        if let Ok(mut guard) = self.fh_table.lock() {
            if let Some(state) = guard.get_mut(&fh) {
                if state.file_id == Some(file_id) {
                    if Self::should_cache_file_read_metadata(
                        self.read_only,
                        self.fopen_direct_io,
                        self.atime_policy,
                        state.flags,
                        metadata_cache_ttl,
                    ) {
                        if let Some(metadata) = metadata.as_ref() {
                            state.read_metadata = Some(metadata.clone());
                            state.read_metadata_loaded_at = Some(Instant::now());
                        } else {
                            state.read_metadata = None;
                            state.read_metadata_loaded_at = None;
                        }
                    } else {
                        // Do not leave a snapshot behind when runtime policy
                        // disables primary handle-metadata caching.
                        state.read_metadata = None;
                        state.read_metadata_loaded_at = None;
                    }
                }
            }
        }
        Ok(metadata)
    }

    fn file_size_for_file_id_or_errno(&self, file_id: u64) -> Result<u64, libc::c_int> {
        match self.repo.file_size(file_id) {
            Ok(Some(size)) => Ok(size),
            Ok(None) => Err(ENOENT),
            Err(_) => Err(EIO),
        }
    }

    fn file_size_for_update_file_id_or_errno(&self, file_id: u64) -> Result<u64, libc::c_int> {
        match self.repo.file_size_authoritative(file_id) {
            Ok(Some(size)) => Ok(size),
            Ok(None) => Err(ENOENT),
            Err(_) => Err(EIO),
        }
    }

    fn ioctl_flags_value(in_data: &[u8]) -> Result<u32, libc::c_int> {
        match in_data.len() {
            4 => {
                let flags_bytes: [u8; 4] = in_data.try_into().map_err(|_| EINVAL)?;
                Ok(u32::from_ne_bytes(flags_bytes))
            }
            8 => {
                let flags_bytes: [u8; 8] = in_data.try_into().map_err(|_| EINVAL)?;
                let raw_flags = u64::from_ne_bytes(flags_bytes);
                if raw_flags > u32::MAX as u64 {
                    return Err(EINVAL);
                }
                Ok(raw_flags as u32)
            }
            _ => Err(EINVAL),
        }
    }


    fn remove_primary_file_or_promote_hardlink(&self, file_id: u64) -> Result<(), String> {
        match self.repo.promote_hardlink_to_primary(file_id) {
            Ok(true) => Ok(()),
            Ok(false) => self.repo.purge_primary_file(file_id).map_err(|err| {
                warn!(
                    "FOD purge_primary_file failed after no hardlink promotion file_id={} err={}",
                    file_id, err
                );
                err
            }),
            Err(err) => {
                warn!(
                    "FOD promote_hardlink_to_primary failed file_id={} err={}",
                    file_id, err
                );
                Err(err)
            }
        }
    }

    fn ioctl_fsxattr_values(in_data: &[u8]) -> Result<(u32, u32, u32, u32, u32), libc::c_int> {
        if in_data.len() != IOCTL_FSXATTR_BYTES {
            return Err(EINVAL);
        }
        let xflags = u32::from_ne_bytes(in_data[0..4].try_into().map_err(|_| EINVAL)?);
        let extsize = u32::from_ne_bytes(in_data[4..8].try_into().map_err(|_| EINVAL)?);
        let nextents = u32::from_ne_bytes(in_data[8..12].try_into().map_err(|_| EINVAL)?);
        let projid = u32::from_ne_bytes(in_data[12..16].try_into().map_err(|_| EINVAL)?);
        let cowextsize = u32::from_ne_bytes(in_data[16..20].try_into().map_err(|_| EINVAL)?);
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

        // Sciezka zapamietana lokalnie moze zostac zdalnie usunieta i odtworzona
        // przez inny mount z nowa generacja inode. W takim przypadku nie wolno
        // zwracac atrybutow nowego obiektu dla starego inode przekazanego przez
        // kernel. ENOENT pozwala getattr przejsc do fallbacku po file_id dla
        // nadal otwartego starego inode, a pozostale operacje nie trafiaja
        // przypadkiem w replacement.
        if attrs.file_attr.ino.0 != ino {
            debug!(
                "FOD stale inode path mapping ino={} path={} resolved_ino={}",
                ino, path, attrs.file_attr.ino.0
            );
            return Err(ENOENT);
        }

        Ok((path, attrs))
    }

    fn authoritative_entry_attrs_for_ino(
        &self,
        ino: u64,
    ) -> Result<(String, ParsedAttrs), libc::c_int> {
        let path = self.entry_path_for_ino(ino)?;
        let authority = self.repo.authoritative_view();
        if let Some(attrs) = self.attrs_for_path_from_repo(&authority, &path)? {
            if attrs.file_attr.ino.0 == ino {
                return Ok((path, attrs));
            }
        }

        // fgetxattr/fsetxattr may target an open file after unlink or a remote
        // replacement of its former pathname. In that case, bind the request
        // to the live handle's file_id instead of following the stale path.
        let attrs = self
            .attrs_for_open_inode(ino)?
            .filter(|attrs| attrs.xattr_owner_kind == "file")
            .ok_or(ENOENT)?;
        Ok((path, attrs))
    }

    fn authoritative_path_attrs_for_ino(
        &self,
        ino: u64,
    ) -> Result<(String, ParsedAttrs), libc::c_int> {
        let path = self.entry_path_for_ino(ino)?;
        let attrs = self
            .lookup_path_from_authority(&path)?
            .ok_or(ENOENT)?;
        if attrs.file_attr.ino.0 != ino {
            return Err(ENOENT);
        }
        Ok((path, attrs))
    }

    fn parent_entry_id_for_inode(&self, ino: u64) -> Result<Option<u64>, libc::c_int> {
        if ino == ROOT_INO {
            return Ok(None);
        }
        let path = self.entry_path_for_ino(ino)?;
        let authority = self.repo.authoritative_view();
        let attrs = self
            .attrs_for_path_from_repo(&authority, &path)?
            .ok_or(ENOENT)?;
        if attrs.file_attr.ino.0 != ino {
            return Err(ENOENT);
        }
        let resolved = authority.resolve_path(&path).map_err(|_| EIO)?;
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
        self.attrs_for_path_from_repo(&self.repo, path)
    }

    fn attrs_for_path_from_repo(
        &self,
        repo: &DbRepo,
        path: &str,
    ) -> Result<Option<ParsedAttrs>, libc::c_int> {
        let blob = repo.fetch_path_attrs_blob(path).map_err(|err| {
            warn!("FOD path attributes query failed path={} err={}", path, err);
            EIO
        })?;
        let Some(blob) = blob else {
            return Ok(None);
        };
        let fields = Self::decode_nul_fields(&blob);
        if fields.is_empty() {
            return Ok(None);
        }
        let obj_type = fields[0].clone();
        let mut file_attr = if obj_type == "symlink" {
            if fields.len() < 9 {
                return Err(EIO);
            }
            let raw_inode = fields[1].parse::<u64>().map_err(|_| EIO)?;
            let target = fields[2].clone();
            let mod_date = fields[3].clone();
            let acc_date = fields[4].clone();
            let chg_date = fields[5].clone();
            let uid = fields[6].parse::<u32>().map_err(|_| EIO)?;
            let gid = fields[7].parse::<u32>().map_err(|_| EIO)?;
            let inode_seed = fields[8].clone();
            let inode = self.stable_inode(&obj_type, &inode_seed, raw_inode)?;
            FileAttr {
                ino: INodeNo(inode),
                size: target.len() as u64,
                blocks: self.block_count(target.len() as u64, "symlink"),
                atime: Self::parse_time(&acc_date)?,
                mtime: Self::parse_time(&mod_date)?,
                ctime: Self::parse_time(&chg_date)?,
                crtime: Self::parse_time(&chg_date)?,
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
            if fields.len() < 11 {
                return Err(EIO);
            }
            let raw_inode = fields[1].parse::<u64>().map_err(|_| EIO)?;
            let size = fields[2].parse::<u64>().map_err(|_| EIO)?;
            let mode = fields[3].clone();
            let mod_date = fields[4].clone();
            let acc_date = fields[5].clone();
            let chg_date = fields[6].clone();
            let uid = fields[7].parse::<u32>().map_err(|_| EIO)?;
            let gid = fields[8].parse::<u32>().map_err(|_| EIO)?;
            let inode_seed = fields[9].clone();
            let allocated_bytes = fields[10].parse::<u64>().map_err(|_| EIO)?;
            let mut kind = if obj_type == "dir" {
                FileType::Directory
            } else {
                FileType::RegularFile
            };
            let perm = parse_file_mode(&mode)?;
            let mut rdev = 0;
            if obj_type == "hardlink" {
                let file_id = repo
                    .get_hardlink_file_id(raw_inode)
                    .map_err(|_| EIO)?
                    .ok_or(EIO)?;
                if let Some((special_type, major, minor)) =
                    repo.get_special_file_metadata(file_id).map_err(|_| EIO)?
                {
                    kind = Self::file_type_from_special(&special_type).ok_or(EIO)?;
                    rdev = Self::encode_fuse_rdev(major, minor)?;
                }
            } else if obj_type == "file" {
                if let Some((special_type, major, minor)) =
                    repo.get_special_file_metadata(raw_inode).map_err(|_| EIO)?
                {
                    kind = Self::file_type_from_special(&special_type).ok_or(EIO)?;
                    rdev = Self::encode_fuse_rdev(major, minor)?;
                }
            }
            let inode = self.stable_inode(&obj_type, &inode_seed, raw_inode)?;
            let nlink = match obj_type.as_str() {
                "hardlink" => {
                    let file_id = repo
                        .get_hardlink_file_id(raw_inode)
                        .map_err(|_| EIO)?
                        .ok_or(EIO)?;
                    repo.count_file_links(file_id).map_err(|_| EIO)?
                }
                "file" => repo.count_file_links(raw_inode).map_err(|_| EIO)?,
                "dir" => 2u64
                    .checked_add(repo.count_directory_subdirs(raw_inode).map_err(|_| EIO)?)
                    .ok_or(EOVERFLOW)?,
                _ => 1,
            };
            FileAttr {
                ino: INodeNo(inode),
                size,
                blocks: self.block_count(allocated_bytes, &obj_type),
                atime: Self::parse_time(&acc_date)?,
                mtime: Self::parse_time(&mod_date)?,
                ctime: Self::parse_time(&chg_date)?,
                crtime: Self::parse_time(&chg_date)?,
                kind,
                perm,
                nlink: Self::fuse_nlink(nlink)?,
                uid,
                gid,
                rdev,
                flags: 0,
                blksize: self.block_size as u32,
            }
        };
        if Self::normalize_path(path) == "/" {
            file_attr.ino = INodeNo::ROOT;
        }
        let raw_inode = fields[1].parse::<u64>().map_err(|_| EIO)?;
        let (xattr_owner_kind, xattr_owner_id) = match obj_type.as_str() {
            "file" => ("file".to_string(), raw_inode),
            "hardlink" => (
                "file".to_string(),
                repo.get_hardlink_file_id(raw_inode)
                    .map_err(|_| EIO)?
                    .ok_or(ENOENT)?,
            ),
            "dir" => (
                "dir".to_string(),
                if Self::normalize_path(path) == "/" {
                    0
                } else {
                    raw_inode
                },
            ),
            "symlink" => ("symlink".to_string(), raw_inode),
            _ => return Err(EIO),
        };
        Ok(Some(ParsedAttrs {
            file_attr,
            inode_seed: fields[if obj_type == "symlink" { 8 } else { 9 }].clone(),
            xattr_owner_kind,
            xattr_owner_id,
        }))
    }

    fn attrs_for_open_file_id(&self, file_id: u64) -> Result<Option<ParsedAttrs>, libc::c_int> {
        let authority = self.repo.authoritative_view();
        let blob = authority.fetch_file_attrs_blob(file_id).map_err(|_| EIO)?;
        let Some(blob) = blob else {
            return Ok(None);
        };
        let fields = Self::decode_nul_fields(&blob);
        if fields.len() < 10 {
            return Err(EIO);
        }

        let raw_inode = fields[0].parse::<u64>().map_err(|_| EIO)?;
        let size = fields[1].parse::<u64>().map_err(|_| EIO)?;
        let mode = fields[2].clone();
        let mod_date = fields[3].clone();
        let acc_date = fields[4].clone();
        let chg_date = fields[5].clone();
        let uid = fields[6].parse::<u32>().map_err(|_| EIO)?;
        let gid = fields[7].parse::<u32>().map_err(|_| EIO)?;
        let inode_seed = fields[8].clone();
        let allocated_bytes = fields[9].parse::<u64>().map_err(|_| EIO)?;

        let mut kind = FileType::RegularFile;
        let perm = parse_file_mode(&mode)?;
        let mut rdev = 0;
        if let Some((special_type, major, minor)) = authority
            .get_special_file_metadata(raw_inode)
            .map_err(|_| EIO)?
        {
            kind = Self::file_type_from_special(&special_type).ok_or(EIO)?;
            rdev = Self::encode_fuse_rdev(major, minor)?;
        }

        let nlink = authority
            .file_link_count_for_stat(raw_inode)
            .map_err(|_| EIO)?
            .ok_or(EIO)?;
        let inode = self.stable_inode("file", &inode_seed, raw_inode)?;
        Ok(Some(ParsedAttrs {
            inode_seed,
            xattr_owner_kind: "file".to_string(),
            xattr_owner_id: raw_inode,
            file_attr: FileAttr {
                ino: INodeNo(inode),
                size,
                blocks: self.block_count(allocated_bytes, "file"),
                atime: Self::parse_time(&acc_date)?,
                mtime: Self::parse_time(&mod_date)?,
                ctime: Self::parse_time(&chg_date)?,
                crtime: Self::parse_time(&chg_date)?,
                kind,
                perm,
                nlink: Self::fuse_nlink(nlink)?,
                uid,
                gid,
                rdev,
                flags: 0,
                blksize: self.block_size as u32,
            },
        }))
    }

    fn pending_write_state_for_path(&self, path: &str) -> Option<WriteState> {
        let handles = self
            .fh_table
            .lock()
            .ok()?
            .iter()
            .filter_map(|(fh, handle)| {
                (handle.path == path && handle.file_id.is_some()).then_some(*fh)
            })
            .collect::<Vec<_>>();
        let states = self.write_states.lock().ok()?;
        let pending = handles
            .iter()
            .filter_map(|fh| states.get(fh))
            .filter(|state| Self::write_state_has_pending_changes(state))
            .collect::<Vec<_>>();
        pending
            .iter()
            .filter(|state| state.truncate_pending)
            .max_by_key(|state| state.file_size)
            .or_else(|| pending.iter().max_by_key(|state| state.file_size))
            .map(|state| (*state).clone())
    }

    fn overlay_pending_write_size(&self, attrs: &mut ParsedAttrs, state: Option<&WriteState>) {
        let Some(state) = state.filter(|state| Self::write_state_has_pending_changes(state)) else {
            return;
        };
        if attrs.file_attr.kind == FileType::RegularFile {
            if attrs.file_attr.blocks == 0 {
                let pending_allocated_bytes = state
                    .payload
                    .as_blocks()
                    .map(|blocks| {
                        blocks
                            .values()
                            .filter(|block| block.iter().any(|byte| *byte != 0))
                            .count() as u64
                            * self.block_size
                    })
                    .unwrap_or(0);
                attrs.file_attr.blocks = self.block_count(pending_allocated_bytes, "file");
            }
            attrs.file_attr.size = state.file_size;
        }
    }

    fn attrs_for_open_inode(&self, ino: u64) -> Result<Option<ParsedAttrs>, libc::c_int> {
        let file_ids = self
            .fh_table
            .lock()
            .map_err(|_| EIO)?
            .values()
            .filter_map(|state| state.file_id)
            .collect::<HashSet<_>>();

        for file_id in file_ids {
            if let Some(attrs) = self.attrs_for_open_file_id(file_id)? {
                if attrs.file_attr.ino.0 == ino {
                    return Ok(Some(attrs));
                }
            }
        }
        Ok(None)
    }

    fn block_count(&self, allocated_bytes: u64, kind: &str) -> u64 {
        if kind == "dir" {
            return 1;
        }
        if allocated_bytes == 0 {
            return 0;
        }
        // POSIX st_blocks is always expressed in 512-byte units. It is
        // independent of the FOD storage block size exposed as st_blksize.
        const STAT_BLOCK_BYTES: u64 = 512;
        1 + allocated_bytes.saturating_sub(1) / STAT_BLOCK_BYTES
    }

    fn fuse_nlink(nlink: u64) -> Result<u32, libc::c_int> {
        u32::try_from(nlink).map_err(|_| EOVERFLOW)
    }

    fn lookup_path(&self, path: &str) -> Result<Option<ParsedAttrs>, libc::c_int> {
        let path = Self::normalize_path(path);
        if path == "/" {
            return self.root_attr().map(Some);
        }
        self.attrs_for_path(&path)
    }

    fn lookup_path_from_authority(
        &self,
        path: &str,
    ) -> Result<Option<ParsedAttrs>, libc::c_int> {
        let path = Self::normalize_path(path);
        if path == "/" {
            return self.root_attr().map(Some);
        }
        let authority = self.repo.authoritative_view();
        self.attrs_for_path_from_repo(&authority, &path)
    }

    fn root_attr(&self) -> Result<ParsedAttrs, libc::c_int> {
        let authority = self.repo.authoritative_view();
        let mut attrs = self
            .attrs_for_path_from_repo(&authority, "/")?
            .ok_or(ENOENT)?;
        let child_dirs = authority.count_root_directory_children().map_err(|_| EIO)?;
        attrs.file_attr.ino = INodeNo::ROOT;
        let root_nlink = 2u64.checked_add(child_dirs).ok_or(EOVERFLOW)?;
        attrs.file_attr.nlink = Self::fuse_nlink(root_nlink)?;
        Ok(attrs)
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

    fn register_lookup_path(&self, path: &str, ino: u64) {
        if let Ok(mut refs) = self.lookup_refs.lock() {
            self.register_path(path, ino);
            let count = refs.entry(ino).or_insert(0);
            *count = count.saturating_add(1);
        } else {
            self.register_path(path, ino);
        }
    }

    fn inode_has_active_handle(&self, ino: u64) -> bool {
        let paths = self
            .path_to_inode
            .read()
            .map(|guard| {
                guard
                    .iter()
                    .filter_map(|(path, cached_ino)| (*cached_ino == ino).then_some(path.clone()))
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        if paths.is_empty() {
            return false;
        }
        self.fh_table
            .lock()
            .map(|guard| guard.values().any(|state| paths.contains(&state.path)))
            .unwrap_or(true)
    }

    fn remove_cached_inode_paths(&self, ino: u64) {
        if ino == ROOT_INO {
            return;
        }
        if let Ok(mut path_guard) = self.path_to_inode.write() {
            path_guard.retain(|_, cached_ino| *cached_ino != ino);
        }
        if let Ok(mut inode_guard) = self.inode_to_path.write() {
            inode_guard.remove(&ino);
        }
    }

    fn evict_inode_if_unreferenced(&self, ino: u64) -> bool {
        if ino == ROOT_INO {
            return false;
        }
        let refs = match self.lookup_refs.lock() {
            Ok(refs) => refs,
            Err(_) => return false,
        };
        if refs.get(&ino).copied().unwrap_or(0) != 0 {
            return false;
        }
        if self.inode_has_active_handle(ino) {
            return false;
        }
        self.remove_cached_inode_paths(ino);
        true
    }

    fn apply_forget_ref_delta(refs: &mut HashMap<u64, u64>, ino: u64, nlookup: u64) -> (u64, bool) {
        if ino == ROOT_INO {
            return (0, false);
        }
        let Some(current) = refs.get(&ino).copied() else {
            return (0, false);
        };
        let remaining = current.saturating_sub(nlookup);
        if nlookup > current {
            warn!(
                "FOD forget lookup underflow ino={} tracked={} nlookup={}",
                ino, current, nlookup
            );
        }
        if remaining != 0 {
            refs.insert(ino, remaining);
            return (remaining, false);
        }
        refs.remove(&ino);
        (0, true)
    }

    fn forget_lookup_refs(&self, ino: u64, nlookup: u64) -> (u64, bool) {
        let (remaining, retire_candidate) = {
            let mut refs = match self.lookup_refs.lock() {
                Ok(refs) => refs,
                Err(_) => return (0, false),
            };
            Self::apply_forget_ref_delta(&mut refs, ino, nlookup)
        };
        let evicted = retire_candidate && self.evict_inode_if_unreferenced(ino);
        (remaining, evicted)
    }

    fn profile_forget_event(&self, ino: u64, nlookup: u64, remaining: u64, evicted: bool) {
        if !fod_fuse_profile_metadata_cache_enabled() {
            return;
        }

        self.metadata_profile_forget_calls
            .fetch_add(1, Ordering::Relaxed);
        self.metadata_profile_forget_nlookup
            .fetch_add(nlookup, Ordering::Relaxed);

        let (lookup_ref_inodes, lookup_ref_total) = self
            .lookup_refs
            .lock()
            .map(|guard| {
                (
                    guard.len(),
                    guard.values().copied().fold(0u64, u64::saturating_add),
                )
            })
            .unwrap_or((0, 0));
        let inode_to_path = self
            .inode_to_path
            .read()
            .map(|guard| guard.len())
            .unwrap_or(0);
        let path_to_inode = self
            .path_to_inode
            .read()
            .map(|guard| guard.len())
            .unwrap_or(0);
        let process_rss_bytes = current_process_rss_bytes().unwrap_or(0);

        info!(
            "FOD forget profile: ino={} nlookup={} remaining={} evicted={} lookup_ref_inodes={} lookup_ref_total={} inode_to_path={} path_to_inode={} process_rss_bytes={}",
            ino,
            nlookup,
            remaining,
            evicted,
            lookup_ref_inodes,
            lookup_ref_total,
            inode_to_path,
            path_to_inode,
            process_rss_bytes
        );
    }

    fn touch_directory_access_time_by_id(
        &self,
        directory_id: u64,
        file_attr: &FileAttr,
    ) -> Option<SystemTime> {
        if self.read_only || !should_update_atime(self.atime_policy, true, file_attr) {
            return None;
        }
        if self.atime_policy == AtimePolicy::Default {
            if let Ok(age) = Self::current_time().duration_since(file_attr.atime) {
                if age < ATIME_TOUCH_INTERVAL {
                    return None;
                }
            }
        }
        let now = Self::current_time();
        let atime = Self::system_time_to_db_string(now);
        match self
            .repo
            .update_directory_access_date(directory_id, &atime)
        {
            Ok(()) => Some(now),
            Err(err) => {
                warn!(
                    "FOD directory atime touch failed directory_id={} err={}",
                    directory_id, err
                );
                None
            }
        }
    }

    fn touch_symlink_access_time_by_id(&self, symlink_id: u64, file_attr: &FileAttr) {
        if self.read_only || !should_update_atime(self.atime_policy, false, file_attr) {
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
        if let Err(err) = self.repo.update_symlink_access_date(symlink_id, &atime) {
            warn!(
                "FOD symlink atime touch failed symlink_id={} err={}",
                symlink_id, err
            );
        }
    }

    fn touch_access_time(&self, path: &str, file_attr: &FileAttr) {
        // A read-only mount is observation-only. Never emit automatic
        // metadata DML to a PostgreSQL physical replica.
        if self.read_only {
            return;
        }
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
                    _ => Ok(()),
                }
            }
            FileType::Symlink => match self.repo.authoritative_view().resolve_path(path) {
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

    fn touch_file_access_time_from_read_metadata(&self, file_id: u64, metadata: &FileReadMetadata) {
        // Reads on a replica must not turn into an UPDATE access_date.
        if self.read_only {
            return;
        }
        let (atime, mtime) = match (
            Self::parse_time(&metadata.access_date),
            Self::parse_time(&metadata.modification_date),
        ) {
            (Ok(atime), Ok(mtime)) => (atime, mtime),
            _ => {
                warn!("FOD read metadata contains an invalid timestamp file_id={}", file_id);
                return;
            }
        };
        if !self
            .atime_policy
            .should_update(false, &FileReadMetadataTime { atime, mtime })
        {
            return;
        }
        if self.atime_policy == AtimePolicy::Default {
            if let Ok(age) = Self::current_time().duration_since(atime) {
                if age < ATIME_TOUCH_INTERVAL {
                    return;
                }
            }
        }
        let atime = Self::system_time_to_db_string(Self::current_time());
        if let Err(err) = self.repo.update_file_access_date(file_id, &atime) {
            warn!(
                "FOD atime touch failed file_id={} change_date={} err={}",
                file_id, metadata.change_date, err
            );
        }
    }

    fn noatime_direct_uncached_read_fast_path_enabled(&self) -> bool {
        if self.atime_policy != AtimePolicy::NoAtime || !self.fopen_direct_io {
            return false;
        }
        let live = self.reloadable_runtime();
        live.read_cache_blocks == 0
            && live.read_ahead_blocks == 0
            && live.sequential_read_ahead_blocks == 0
            && live.direct_io_read_prefetch_blocks == 0
            && live.small_file_read_threshold_blocks == 0
    }

    fn read_noatime_direct_uncached(
        &self,
        file_id: u64,
        offset: u64,
        size: u64,
    ) -> Result<Vec<u8>, libc::c_int> {
        if size == 0 {
            return Ok(Vec::new());
        }
        let requested_end = offset.checked_add(size).ok_or(EINVAL)?;
        let first_block = offset / self.block_size;
        let requested_last_block = requested_end.saturating_sub(1) / self.block_size;
        let started = Instant::now();
        let snapshot = self
            .repo
            .fetch_block_range_with_size_shared(
                file_id,
                first_block,
                requested_last_block,
                self.block_size,
            )
            .map_err(|_| EIO)?;
        self.record_read_block_map_elapsed(started.elapsed());
        let Some((file_size, mut blocks)) = snapshot else {
            return Err(ENOENT);
        };
        if offset >= file_size {
            return Ok(Vec::new());
        }
        let end_offset = requested_end.min(file_size);
        let last_block = end_offset.saturating_sub(1) / self.block_size;
        blocks.retain(|(idx, _)| *idx >= first_block && *idx <= last_block);
        let started = Instant::now();
        let data = assemble_read_slice(
            first_block,
            last_block,
            offset,
            end_offset,
            self.block_size,
            &blocks,
        );
        self.record_assemble_read_slice_elapsed(started.elapsed());
        debug!(
            "FOD read fast path fused_block_range_with_size file_id={} offset={} size={} file_size={} first_block={} last_block={}",
            file_id, offset, size, file_size, first_block, last_block
        );
        Ok(data)
    }

    fn read_degraded_snapshot(
        &self,
        file_id: u64,
        offset: u64,
        size: u64,
    ) -> Result<Vec<u8>, libc::c_int> {
        if size == 0 {
            return Ok(Vec::new());
        }
        let requested_end = offset.saturating_add(size);
        let first_block = offset / self.block_size;
        let requested_last_block = requested_end.saturating_sub(1) / self.block_size;
        let started = Instant::now();
        let snapshot = self
            .repo
            .fetch_block_range_with_generation_shared(
                file_id,
                first_block,
                requested_last_block,
                self.block_size,
            )
            .map_err(|err| {
                warn!(
                    "FOD degraded read snapshot failed file_id={} offset={} size={} err={}",
                    file_id, offset, size, err
                );
                EIO
            })?;
        self.record_read_block_map_elapsed(started.elapsed());
        let started = Instant::now();
        let (data, content_generation) =
            assemble_degraded_snapshot(snapshot, offset, size, self.block_size)?;
        self.record_assemble_read_slice_elapsed(started.elapsed());
        debug!(
            "FOD degraded snapshot read file_id={} content_generation={} offset={} size={}",
            file_id, content_generation, offset, size
        );
        Ok(data)
    }

    fn strict_cross_mount_reads_enabled() -> bool {
        std::env::var("FOD_CROSS_MOUNT_STRICT_READS")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
    }

    fn fopen_flags(&self) -> FopenFlags {
        // Domyslnie nie wymuszamy direct_io.
        // direct_io jest trybem diagnostycznym/zgodnosciowym, bo potrafi mocno spowolnic
        // sekwencyjny zapis malymi blokami przez ominiecie cache/writeback kernela.
        if self.fopen_direct_io || Self::strict_cross_mount_reads_enabled() {
            FopenFlags::FOPEN_DIRECT_IO
        } else {
            FopenFlags::empty()
        }
    }

    fn next_handle(&self) -> u64 {
        let mut guard = self.next_fh.lock().unwrap();
        let fh = *guard;
        *guard += 1;
        fh
    }

    fn new_inode_seed(&self, object_type: &str) -> Result<String, libc::c_int> {
        let session_id = self.session_id.ok_or(EIO)?;
        Ok(format!(
            "fod:{object_type}:{session_id}:{}",
            self.next_handle()
        ))
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

    fn insert_handle_for_file(
        &self,
        fh: u64,
        path: String,
        file_id: Option<u64>,
        flags: i32,
        write_ownership: Option<FileWriteOwnership>,
    ) {
        if let Ok(mut guard) = self.fh_table.lock() {
            guard.insert(
                fh,
                FileHandleState {
                    path,
                    file_id,
                    flags,
                    atime_touched: false,
                    read_metadata: None,
                    read_metadata_loaded_at: None,
                    write_ownership,
                },
            );
        }
    }

    fn register_file_open_lease_for_handle(
        &self,
        fh: u64,
        file_id: u64,
    ) -> Result<(), libc::c_int> {
        if self.degraded_read_only {
            return Ok(());
        }
        let Some(session_id) = self.session_id else {
            warn!(
                "FOD file open lease unavailable fh={} file_id={} reason=no_client_session",
                fh, file_id
            );
            return Err(EIO);
        };
        let Some(repo) = self.client_session_repo() else {
            return Err(EIO);
        };
        repo.register_file_open_lease(
            file_id,
            session_id,
            fh,
            self.client_session_lease_ttl_seconds,
        )
        .map_err(|err| {
            warn!(
                "FOD file open lease registration failed fh={} file_id={} session_id={} err={}",
                fh, file_id, session_id, err
            );
            // Only a genuinely unavailable file maps to ENOENT.
            // An expired/missing PostgreSQL client session must fail closed.
            crate::errno::open_lease_errno(&err)
        })
    }

    fn create_handle_for_file(
        &self,
        path: String,
        file_id: Option<u64>,
        flags: i32,
    ) -> Result<u64, libc::c_int> {
        let fh = self.next_handle();
        if let Some(file_id) = file_id {
            self.register_file_open_lease_for_handle(fh, file_id)?;
        }
        self.insert_handle_for_file(fh, path, file_id, flags, None);
        Ok(fh)
    }

    fn write_ownership_resource_for_path(
        &self,
        path: &str,
    ) -> Result<(Option<u64>, String), libc::c_int> {
        let normalized = Self::normalize_path(path);
        let path = Path::new(&normalized);
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
            .ok_or(EIO)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("/"));
        let parent_text = parent.to_string_lossy();
        let parent_id = if parent_text.is_empty() || parent_text == "/" {
            None
        } else {
            let resolved = self
                .repo
                .authoritative_view()
                .resolve_path(parent_text.as_ref())
                .map_err(|err| {
                    warn!(
                        "FOD write ownership parent resolution failed path={} parent={} err={}",
                        normalized, parent_text, err
                    );
                    EIO
                })?;
            if resolved.kind.as_deref() != Some("dir") {
                return Err(ENOENT);
            }
            Some(resolved.entry_id.ok_or(EIO)?)
        };
        Ok((parent_id, name))
    }

    fn local_write_ownership_for_file(&self, file_id: u64) -> Option<FileWriteOwnership> {
        self.fh_table.lock().ok().and_then(|guard| {
            guard.values().find_map(|state| {
                (state.file_id == Some(file_id))
                    .then(|| state.write_ownership.clone())
                    .flatten()
            })
        })
    }

    fn write_persistence_fence_for_handle(&self, fh: u64) -> Option<(u64, WritePersistenceFence)> {
        let session_id = self.session_id?;
        self.fh_table.lock().ok().and_then(|guard| {
            let state = guard.get(&fh)?;
            let ownership = state.write_ownership.as_ref()?;
            let file_fencing_token = ownership.lease.file_fencing_token?;
            Some((
                ownership.file_id,
                WritePersistenceFence {
                    session_id,
                    owner_key: ownership.owner_key,
                    file_fencing_token,
                },
            ))
        })
    }

    fn adopt_source_data_object_for_handle(
        &self,
        fh_out: u64,
        src_file_id: u64,
        dst_file_id: u64,
    ) -> Result<bool, libc::c_int> {
        let Some((ownership_file_id, write_fence)) =
            self.write_persistence_fence_for_handle(fh_out)
        else {
            warn!(
                "FOD data-object adoption refused fh_out={} dst_file_id={} reason=no_write_fence",
                fh_out, dst_file_id
            );
            return Err(EIO);
        };
        if ownership_file_id != dst_file_id {
            warn!(
                "FOD data-object adoption refused fh_out={} dst_file_id={} owned_file_id={}",
                fh_out, dst_file_id, ownership_file_id
            );
            return Err(EIO);
        }
        self.repo
            .adopt_source_data_object_with_fence(src_file_id, dst_file_id, write_fence)
            .map_err(|err| {
                warn!(
                    "FOD data-object adoption fenced transaction failed src_file_id={} dst_file_id={} err={}",
                    src_file_id, dst_file_id, err
                );
                EIO
            })
    }

    fn attach_write_persistence_fence_for_handle(
        &self,
        fh: u64,
        state: &mut WriteState,
    ) -> Result<(), libc::c_int> {
        if state.persistence_fence.is_some() {
            return Ok(());
        }
        let Some((ownership_file_id, fence)) = self.write_persistence_fence_for_handle(fh) else {
            // P1 jest fail-closed: stan zapisu bez aktywnego ownership/fence
            // nie moze zostac utrwalony w PostgreSQL.
            warn!(
                "FOD write persistence fence missing fh={} state_file_id={}",
                fh, state.file_id
            );
            return Err(EIO);
        };
        if ownership_file_id != state.file_id {
            warn!(
                "FOD write persistence fence file mismatch fh={} state_file_id={} ownership_file_id={}",
                fh, state.file_id, ownership_file_id
            );
            return Err(EIO);
        }
        state.persistence_fence = Some(fence);
        Ok(())
    }

    // Integration-test-only deterministic race hook.
    // This code is absent from normal production builds.
    #[cfg(feature = "integration-test-hooks")]
    fn test_create_before_ownership_barrier(path: &str) -> Result<(), libc::c_int> {
        let Ok(barrier_dir) = std::env::var("FOD_TEST_CREATE_BEFORE_OWNERSHIP_BARRIER_DIR") else {
            return Ok(());
        };

        if barrier_dir.trim().is_empty() {
            return Ok(());
        }

        let barrier_dir = PathBuf::from(barrier_dir);
        let target_path = barrier_dir.join("target");

        let target = match fs::read_to_string(&target_path) {
            Ok(value) => value,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                warn!(
                    "FOD test create barrier target read failed path={} err={}",
                    path, err
                );
                return Err(EIO);
            }
        };

        if target != path {
            return Ok(());
        }

        let ready_path = barrier_dir.join("ready");
        if let Err(err) = fs::write(&ready_path, path.as_bytes()) {
            warn!(
                "FOD test create barrier ready write failed path={} err={}",
                path, err
            );
            return Err(EIO);
        }

        let release_path = barrier_dir.join("release");
        let deadline = Instant::now() + Duration::from_secs(10);

        while !release_path.exists() {
            if Instant::now() >= deadline {
                warn!("FOD test create barrier timeout path={}", path);
                return Err(EIO);
            }
            thread::sleep(Duration::from_millis(5));
        }

        let _ = fs::remove_file(&target_path);
        Ok(())
    }

    // Integration-test-only deterministic rename race hook.
    // This code is absent from normal production builds.
    #[cfg(feature = "integration-test-hooks")]
    fn test_rename_before_ownership_barrier(path: &str) -> Result<(), libc::c_int> {
        let Ok(barrier_dir) = std::env::var("FOD_TEST_RENAME_BEFORE_OWNERSHIP_BARRIER_DIR") else {
            return Ok(());
        };

        if barrier_dir.trim().is_empty() {
            return Ok(());
        }

        let barrier_dir = PathBuf::from(barrier_dir);
        let target_path = barrier_dir.join("target");

        let target = match fs::read_to_string(&target_path) {
            Ok(value) => value,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                warn!(
                    "FOD test rename barrier target read failed path={} err={}",
                    path, err
                );
                return Err(EIO);
            }
        };

        if target != path {
            return Ok(());
        }

        let ready_path = barrier_dir.join(format!("ready.{}", std::process::id()));
        if let Err(err) = fs::write(&ready_path, path.as_bytes()) {
            warn!(
                "FOD test rename barrier ready write failed path={} err={}",
                path, err
            );
            return Err(EIO);
        }

        let release_path = barrier_dir.join("release");
        let deadline = Instant::now() + Duration::from_secs(10);

        while !release_path.exists() {
            if Instant::now() >= deadline {
                warn!("FOD test rename barrier timeout path={}", path);
                return Err(EIO);
            }
            thread::sleep(Duration::from_millis(5));
        }

        Ok(())
    }

    fn create_file_for_subject(
        &self,
        parent_id: Option<u64>,
        name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
        inode_seed: &str,
        supplementary_groups: &[u32],
    ) -> Result<u64, String> {
        if self.acl_enabled {
            // PostgreSQL checks parent access and applies inherited ACL data
            // in the same transaction as the new inode.
            self.repo.create_file_with_default_acl(
                parent_id,
                name,
                mode,
                uid,
                gid,
                inode_seed,
                supplementary_groups,
            )
        } else {
            self.repo.create_file_for_subject(
                parent_id,
                name,
                mode,
                uid,
                gid,
                inode_seed,
                supplementary_groups,
            )
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn create_writable_file_with_destination_ownership(
        &self,
        path: String,
        parent_id: Option<u64>,
        name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
        supplementary_groups: &[u32],
        flags: i32,
    ) -> Result<(u64, u64, bool), libc::c_int> {
        let _gate = self.write_ownership_gate.lock().map_err(|_| EIO)?;
        let fh = self.next_handle();

        if self.session_id.is_none() {
            warn!(
                "FOD create destination ownership unavailable path={} reason=no_client_session",
                path
            );
            return Err(EIO);
        }
        let inode_seed = self.new_inode_seed("file")?;

        let ownership_repo = self.client_session_repo().ok_or(EIO)?;
        let owner_key = fh;

        #[cfg(feature = "integration-test-hooks")]
        Self::test_create_before_ownership_barrier(&path)?;

        let destination_lease = match ownership_repo.acquire_write_ownership(
            parent_id,
            name,
            None,
            owner_key,
            self.client_session_lease_ttl_seconds,
        ) {
            Ok(Some(lease)) => lease,
            Ok(None) => {
                debug!(
                    "FOD create destination ownership busy path={} owner_key={}",
                    path, owner_key
                );
                return Err(EBUSY);
            }
            Err(err) => {
                warn!(
                    "FOD create destination ownership acquire failed path={} owner_key={} err={}",
                    path, owner_key, err
                );
                return Err(EIO);
            }
        };

        let release_destination = |reason: &str| {
            if let Err(err) = ownership_repo.release_write_ownership(owner_key, destination_lease) {
                warn!(
                    "FOD create destination ownership release failed path={} owner_key={} reason={} err={}",
                    path, owner_key, reason, err
                );
            }
        };

        // Re-check the namespace only after destination ownership is ours.
        // This closes the cross-mount negative-lookup/create race.
        let authority = self.repo.authoritative_view();
        let resolved = match authority.resolve_path(&path) {
            Ok(value) => value,
            Err(err) => {
                warn!(
                    "FOD create destination recheck failed path={} owner_key={} err={}",
                    path, owner_key, err
                );
                release_destination("resolve_path");
                return Err(EIO);
            }
        };

        let (file_id, created_new) = match resolved.kind.as_deref() {
            Some("file") => {
                if flags & libc::O_EXCL != 0 {
                    release_destination("existing_file_excl");
                    return Err(EEXIST);
                }
                match resolved.entry_id {
                    Some(file_id) => (file_id, false),
                    None => {
                        release_destination("existing_file_missing_id");
                        return Err(EIO);
                    }
                }
            }
            Some("hardlink") => {
                if flags & libc::O_EXCL != 0 {
                    release_destination("existing_hardlink_excl");
                    return Err(EEXIST);
                }
                let Some(hardlink_id) = resolved.entry_id else {
                    release_destination("existing_hardlink_missing_id");
                    return Err(EIO);
                };
                match authority.get_hardlink_file_id(hardlink_id) {
                    Ok(Some(file_id)) => (file_id, false),
                    Ok(None) => {
                        release_destination("existing_hardlink_missing_file");
                        return Err(EIO);
                    }
                    Err(err) => {
                        warn!(
                            "FOD create hardlink file lookup failed path={} hardlink_id={} err={}",
                            path, hardlink_id, err
                        );
                        release_destination("existing_hardlink_lookup");
                        return Err(EIO);
                    }
                }
            }
            Some(_) => {
                release_destination("existing_non_file");
                return Err(EEXIST);
            }
            None => match self.create_file_for_subject(
                parent_id,
                name,
                mode,
                uid,
                gid,
                &inode_seed,
                supplementary_groups,
            ) {
                Ok(file_id) => (file_id, true),
                Err(err) => {
                    warn!(
                        "FOD create pre-owned destination create_file failed path={} owner_key={} err={}",
                        path, owner_key, err
                    );
                    release_destination("create_file");
                    return Err(crate::errno::namespace_mutation_errno(&err));
                }
            },
        };

        let lease = match ownership_repo.acquire_write_ownership(
            parent_id,
            name,
            Some(file_id),
            owner_key,
            self.client_session_lease_ttl_seconds,
        ) {
            Ok(Some(lease)) => lease,
            Ok(None) => {
                let mut rollback_failed = false;
                if created_new {
                    if let Err(err) = self.remove_primary_file_or_promote_hardlink(file_id) {
                        rollback_failed = true;
                        warn!(
                            "FOD create file-ownership rollback failed path={} file_id={} owner_key={} err={}",
                            path, file_id, owner_key, err
                        );
                    }
                }
                release_destination("file_ownership_busy");
                return Err(if rollback_failed { EIO } else { EBUSY });
            }
            Err(err) => {
                if created_new {
                    if let Err(rollback_err) = self.remove_primary_file_or_promote_hardlink(file_id)
                    {
                        warn!(
                            "FOD create file-ownership error rollback failed path={} file_id={} owner_key={} err={}",
                            path, file_id, owner_key, rollback_err
                        );
                    }
                }
                warn!(
                    "FOD create file ownership acquire failed path={} file_id={} owner_key={} err={}",
                    path, file_id, owner_key, err
                );
                release_destination("file_ownership_error");
                return Err(EIO);
            }
        };

        if let Err(errno) = self.register_file_open_lease_for_handle(fh, file_id) {
            if let Err(err) = ownership_repo.release_write_ownership(owner_key, lease) {
                warn!(
                    "FOD create open-lease ownership rollback failed file_id={} owner_key={} err={}",
                    file_id, owner_key, err
                );
            }
            if created_new {
                if let Err(err) = self.remove_primary_file_or_promote_hardlink(file_id) {
                    warn!(
                        "FOD create open-lease file rollback failed file_id={} err={}",
                        file_id, err
                    );
                }
            }
            return Err(errno);
        }
        let ownership = FileWriteOwnership {
            parent_id,
            name: name.to_string(),
            file_id,
            owner_key,
            lease,
            destination_released: false,
        };
        self.insert_handle_for_file(fh, path, Some(file_id), flags, Some(ownership));
        Ok((file_id, fh, created_new))
    }

    fn create_writable_handle_with_ownership(
        &self,
        path: String,
        file_id: u64,
        flags: i32,
    ) -> Result<u64, libc::c_int> {
        let _gate = self.write_ownership_gate.lock().map_err(|_| EIO)?;
        let fh = self.next_handle();

        if let Some(ownership) = self.local_write_ownership_for_file(file_id) {
            self.register_file_open_lease_for_handle(fh, file_id)?;
            self.insert_handle_for_file(fh, path, Some(file_id), flags, Some(ownership));
            return Ok(fh);
        }

        if self.session_id.is_none() {
            warn!(
                "FOD write ownership unavailable path={} file_id={} reason=no_client_session",
                path, file_id
            );
            return Err(EIO);
        }
        let (parent_id, name) = self.write_ownership_resource_for_path(&path)?;
        let ownership_repo = self.client_session_repo().ok_or(EIO)?;
        let owner_key = fh;
        let lease = match ownership_repo.acquire_write_ownership(
            parent_id,
            &name,
            Some(file_id),
            owner_key,
            self.client_session_lease_ttl_seconds,
        ) {
            Ok(Some(lease)) => lease,
            Ok(None) => {
                debug!(
                    "FOD write ownership busy path={} file_id={} owner_key={}",
                    path, file_id, owner_key
                );
                return Err(EBUSY);
            }
            Err(err) => {
                warn!(
                    "FOD write ownership acquire failed path={} file_id={} owner_key={} err={}",
                    path, file_id, owner_key, err
                );
                return Err(EIO);
            }
        };
        if let Err(errno) = self.register_file_open_lease_for_handle(fh, file_id) {
            if let Err(err) = ownership_repo.release_write_ownership(owner_key, lease) {
                warn!(
                    "FOD open-lease ownership rollback failed file_id={} owner_key={} err={}",
                    file_id, owner_key, err
                );
            }
            return Err(errno);
        }
        let ownership = FileWriteOwnership {
            parent_id,
            name,
            file_id,
            owner_key,
            lease,
            destination_released: false,
        };
        self.insert_handle_for_file(fh, path, Some(file_id), flags, Some(ownership));
        Ok(fh)
    }

    fn release_write_ownership_for_handle(&self, fh: u64) -> Result<(), libc::c_int> {
        let _gate = self.write_ownership_gate.lock().map_err(|_| EIO)?;
        let ownership = self.fh_table.lock().ok().and_then(|guard| {
            guard
                .get(&fh)
                .and_then(|state| state.write_ownership.clone())
        });
        let Some(ownership) = ownership else {
            return Ok(());
        };

        let shared_locally = self
            .fh_table
            .lock()
            .map(|guard| {
                guard.iter().any(|(other_fh, state)| {
                    *other_fh != fh
                        && state
                            .write_ownership
                            .as_ref()
                            .map(|other| other.owner_key == ownership.owner_key)
                            .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if shared_locally {
            return Ok(());
        }

        let ownership_repo = self.client_session_repo().ok_or(EIO)?;
        ownership_repo
            .release_write_ownership(ownership.owner_key, ownership.lease)
            .map_err(|err| {
                warn!(
                    "FOD write ownership release failed fh={} file_id={} owner_key={} err={}",
                    fh, ownership.file_id, ownership.owner_key, err
                );
                EIO
            })
    }

    fn rollback_open_handle(&self, fh: u64) {
        if let Err(errno) = self.release_write_ownership_for_handle(fh) {
            warn!(
                "FOD write ownership rollback release failed fh={} errno={}",
                fh, errno
            );
        }
        self.remove_handle_state(fh);
    }

    fn open_file_id_for_path(&self, path: &str) -> Option<u64> {
        self.fh_table.lock().ok().and_then(|guard| {
            guard
                .values()
                .find_map(|state| (state.path == path).then_some(state.file_id).flatten())
        })
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

    fn mark_unlink_destination_released(&self, file_id: u64, parent_id: Option<u64>, name: &str) {
        if let Ok(mut guard) = self.fh_table.lock() {
            for state in guard.values_mut() {
                if state.file_id == Some(file_id) {
                    if let Some(ownership) = state.write_ownership.as_mut() {
                        if ownership.parent_id == parent_id && ownership.name == name {
                            ownership.destination_released = true;
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn unlink_file_with_primary_authority(
        &self,
        file_id: u64,
        parent_id: Option<u64>,
        parent_path: &str,
        old_path: &str,
        old_name: &str,
        req_id: u64,
        subject: &SubjectIdentity,
    ) -> Result<(), String> {
        let session_id = self.session_id.unwrap_or(0);
        let hidden_name = format!("{DEFERRED_UNLINK_NAME_PREFIX}{session_id}-{file_id}-{req_id}");
        let outcome = self.repo.unlink_file_entry_for_subject_acl(
            file_id,
            &hidden_name,
            parent_id,
            old_name,
            subject.uid,
            subject.gid,
            &subject.groups.iter().copied().collect::<Vec<_>>(),
            self.acl_enabled,
        )?;
        if outcome == FileUnlinkOutcome::SourceChanged {
            return Err("FOD_UNLINK_SOURCE_CHANGED".to_string());
        }
        // This mirrors the committed primary result; it never decides whether
        // another host may reclaim a file or keep writing an open inode.
        self.mark_unlink_destination_released(file_id, parent_id, old_name);
        self.remove_cached_path(old_path);
        if outcome == FileUnlinkOutcome::Deferred {
            let hidden_path = Self::join_path(parent_path, OsStr::new(&hidden_name));
            if let Ok(mut guard) = self.fh_table.lock() {
                for state in guard.values_mut() {
                    if state.file_id == Some(file_id) && state.path == old_path {
                        state.path = hidden_path.clone();
                    }
                }
            }
        }
        self.invalidate_statfs_cache();
        info!(
            "FOD authoritative unlink file_id={} path={} outcome={:?}",
            file_id, old_path, outcome
        );
        Ok(())
    }

    fn finalize_deferred_unlink_after_handle_removal(
        &self,
        removed_state: Option<&FileHandleState>,
    ) -> Result<(), libc::c_int> {
        let Some(file_id) = removed_state.and_then(|state| state.file_id) else {
            return Ok(());
        };
        if self.repo.file_is_unlinked(file_id).map_err(|_| EIO)? != Some(true) {
            return Ok(());
        }
        if self
            .repo
            .has_active_file_open_leases(file_id)
            .map_err(|_| EIO)?
        {
            return Ok(());
        }

        self.repo.purge_primary_file(file_id).map_err(|err| {
            warn!(
                "FOD deferred unlink final purge failed file_id={} err={}",
                file_id, err
            );
            EIO
        })?;
        self.invalidate_statfs_cache();
        info!("FOD deferred unlink finalized file_id={}", file_id);
        Ok(())
    }

    fn take_handle_state(&self, fh: u64) -> Option<FileHandleState> {
        let removed_state = self
            .fh_table
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(&fh));
        if let Ok(mut guard) = self.write_states.lock() {
            guard.remove(&fh);
        }
        if let Ok(mut guard) = self.handle_operation_locks.lock() {
            guard.remove(&fh);
        }
        if let Some(file_id) = removed_state.as_ref().and_then(|state| state.file_id) {
            if let (Some(session_id), Some(repo)) = (self.session_id, self.client_session_repo()) {
                if let Err(err) = repo.release_file_open_lease(session_id, fh) {
                    warn!(
                        "FOD file open lease release failed fh={} file_id={} session_id={} err={}",
                        fh, file_id, session_id, err
                    );
                }
            }
        }
        if let Some(path) = removed_state.as_ref().map(|state| state.path.as_str()) {
            if let Some(ino) = self.inode_for_path(path) {
                let evicted = self.evict_inode_if_unreferenced(ino);
                if fod_fuse_profile_metadata_cache_enabled() {
                    let lookup_remaining = self
                        .lookup_refs
                        .lock()
                        .map(|guard| guard.get(&ino).copied().unwrap_or(0))
                        .unwrap_or(u64::MAX);
                    let inode_cached = self
                        .inode_to_path
                        .read()
                        .map(|guard| guard.contains_key(&ino))
                        .unwrap_or(true);
                    let path_cached = self
                        .path_to_inode
                        .read()
                        .map(|guard| guard.values().any(|cached_ino| *cached_ino == ino))
                        .unwrap_or(true);
                    info!(
                        "FOD release cache profile: fh={} ino={} evicted={} lookup_remaining={} inode_cached={} path_cached={}",
                        fh,
                        ino,
                        evicted,
                        lookup_remaining,
                        inode_cached,
                        path_cached
                    );
                }
            }
        }
        removed_state
    }

    fn remove_handle_state(&self, fh: u64) {
        let _ = self.take_handle_state(fh);
    }

    fn handle_operation_lock(&self, fh: u64) -> Arc<Mutex<()>> {
        let mut locks = self
            .handle_operation_locks
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        Arc::clone(locks.entry(fh).or_default())
    }

    fn flush_write_state_for_handle(&self, fh: u64) -> Result<bool, libc::c_int> {
        let Some(mut state) = self.write_state_for_handle(fh) else {
            return Ok(false);
        };
        if !Self::write_state_has_pending_changes(&state) {
            return Ok(false);
        }
        self.attach_write_persistence_fence_for_handle(fh, &mut state)?;
        self.flush_write_state(&mut state)?;
        if Self::write_state_has_pending_changes(&state) {
            self.update_write_state(fh, state);
        } else {
            self.remove_write_state(fh);
        }
        Ok(true)
    }

    fn clear_locks_for_owner_and_sync(&self, owner: u64, context: &str, fh: u64) {
        self.register_local_lock_owner(owner);
        if self.lock_backend_is_pg() {
            if self.posix_locks.is_poisoned() {
                match self.repo.delete_lock_range_leases_for_owner(owner) {
                    Ok(()) => {
                        if let Err(err) = self.repo.clear_lock_wait_edges_for_owner(owner) {
                            warn!(
                                "FOD {} recovered lock owner but could not clear its wait edges fh={} owner={} err={}",
                                context, fh, owner, err
                            );
                        }
                        if let Ok(mut guard) = self.local_lock_owners.lock() {
                            guard.remove(&owner);
                        }
                        warn!(
                            "FOD {} recovered poisoned lock map by deleting PostgreSQL owner leases fh={} owner={}",
                            context, fh, owner
                        );
                    }
                    Err(err) => warn!(
                        "FOD {} could not clear PostgreSQL lock owner after lock-map poison fh={} owner={} err={}",
                        context, fh, owner, err
                    ),
                }
                return;
            }
            if let Err(err) = self.repo.clear_lock_wait_edges_for_owner(owner) {
                warn!(
                    "FOD {} clear lock wait edges failed fh={} owner={} err={}",
                    context, fh, owner, err
                );
            }
        }
        let changed = self.clear_locks_for_owner(owner);
        for resource_key in changed {
            let result = if self.lock_backend_is_pg() {
                self.update_pg_lock_range(&resource_key, owner, 0, u64::MAX, libc::F_UNLCK, 0)
                    .map(|_| ())
            } else {
                let records = self
                    .posix_locks
                    .lock()
                    .ok()
                    .and_then(|guard| guard.get(&resource_key).cloned())
                    .unwrap_or_default();
                self.persist_lock_records_for_key(&resource_key, owner, &records)
            };
            if let Err(errno) = result {
                warn!(
                    "FOD {} lock sync failed fh={} resource_key={} errno={}",
                    context, fh, resource_key, errno
                );
            }
        }
        self.refresh_local_lock_owner_state(owner);
    }

    pub fn register_client_session(
        &mut self,
        mountpoint: &Path,
        mount_mode: &str,
    ) -> Result<(), String> {
        if self.session_id.is_some() {
            return Ok(());
        }
        let Some(repo) = self.client_session_repo() else {
            return Ok(());
        };
        let host_name = client_session_hostname(std::env::var("HOSTNAME").ok(), current_hostname());
        let mountpoint = mountpoint.to_string_lossy().to_string();
        let pid = u64::from(std::process::id());
        let session_id = repo.register_client_session(
            &host_name,
            &mountpoint,
            mount_mode,
            self.lock_backend.as_str(),
            pid,
            self.client_session_lease_ttl_seconds,
        )?;
        self.session_id = Some(session_id);
        Ok(())
    }

    pub fn start_client_session_heartbeat(&mut self) -> Result<(), String> {
        if self.client_session_heartbeat.is_some() {
            return Ok(());
        }
        let Some(session_id) = self.session_id else {
            return Ok(());
        };
        let Some(repo) = self.client_session_repo() else {
            return Ok(());
        };
        let handle = ClientSessionHeartbeatHandle::spawn(
            repo.clone(),
            session_id,
            Arc::clone(&self.fh_table),
            self.client_session_heartbeat_interval,
            self.client_session_lease_ttl_seconds,
        )?;
        self.client_session_heartbeat = Some(handle);
        Ok(())
    }

    pub fn start_client_session_maintenance(&mut self) -> Result<(), String> {
        if self.client_session_maintenance.is_some() {
            return Ok(());
        }
        if self.session_id.is_none() {
            return Ok(());
        }
        let Some(repo) = self.client_session_repo() else {
            return Ok(());
        };
        let handle = ClientSessionMaintenanceHandle::spawn(repo.clone())?;
        self.client_session_maintenance = Some(handle);
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

    pub fn start_shared_monitor_publisher(&mut self) -> Result<(), String> {
        if self.shared_monitor_publisher.is_some() {
            return Ok(());
        }
        let Some(session_id) = self.session_id else {
            return Ok(());
        };
        let Some(publish_repo) = self.client_session_repo() else {
            return Ok(());
        };
        let publisher = SharedMonitorPublisherHandle::spawn(SharedMonitorPublisherInput {
            observability_repo: self.repo.clone(),
            publish_repo: publish_repo.clone(),
            session_id,
            interval: self.client_session_heartbeat_interval,
            read: Arc::clone(&self.logical_read_tasks),
            write: Arc::clone(&self.logical_write_tasks),
            copy: Arc::clone(&self.logical_copy_tasks),
            profile: Arc::clone(&self.profile),
            fuse_compatibility: Arc::clone(&self.fuse_compatibility),
        })?;
        self.shared_monitor_publisher = Some(publisher);
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

    pub fn start_logical_task_observability(&mut self) -> Result<(), String> {
        if self.logical_task_observability.is_some() {
            return Ok(());
        }
        let sampler = LogicalTaskObservabilitySampler::spawn(
            self.logical_task_trackers(),
            LOGICAL_TASK_OBSERVABILITY_INTERVAL,
        )?;
        self.logical_task_observability = Some(sampler);
        Ok(())
    }
}

impl Drop for FodFuse {
    fn drop(&mut self) {
        if let Some(mut publisher) = self.shared_monitor_publisher.take() {
            publisher.stop();
        }
        if let Some(mut heartbeat) = self.client_session_heartbeat.take() {
            heartbeat.stop();
        }
        if let Some(mut maintenance) = self.client_session_maintenance.take() {
            maintenance.stop();
        }
        if let Some(mut sampler) = self.logical_task_observability.take() {
            sampler.stop();
        }
        if fod_fuse_profile_metadata_cache_enabled() {
            let process_rss_bytes = current_process_rss_bytes().ok();
            info!(
                "FOD metadata cache profile: process_rss_bytes={:?} {}",
                process_rss_bytes,
                self.debug_snapshot()
            );
        }
        if fod_fuse_profile_io_enabled() && self.profile.has_activity() {
            info!("FOD boundary profile:");
            for line in self.profile.snapshot_lines() {
                info!("  {}", line);
            }
        }
        self.log_logical_task_observability("shutdown");
    }
}

impl Filesystem for FodFuse {
    fn init(&mut self, _req: &Request, config: &mut KernelConfig) -> std::io::Result<()> {
        let snapshot = FuseCompatibilitySnapshot::configure(config, self.acl_enabled)?;
        snapshot.log();
        if !self.read_only
            && snapshot
                .unsupported_capabilities
                .contains(InitFlags::FUSE_ATOMIC_O_TRUNC)
        {
            return Err(std::io::Error::other(
                "writable FOD mount requires FUSE_ATOMIC_O_TRUNC for write ownership safety",
            ));
        }
        let mut compatibility = self
            .fuse_compatibility
            .write()
            .unwrap_or_else(|err| err.into_inner());
        *compatibility = Some(snapshot.shared_monitor_stats());
        Ok(())
    }

    fn lookup(&self, req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let parent = parent.0;
        let req_id = self.next_request_id();
        if let Err(errno) = Self::validate_component_name(name) {
            fuse_reply_error!(reply, errno);
            return;
        }
        let parent_path = match self.entry_path_for_ino(parent) {
            Ok(path) => path,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "lookup",
                    errno,
                    format!("parent={} name={}", parent, name.to_string_lossy()),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        if let Err(errno) = self.parent_entry_id_for_inode(parent) {
            fuse_reply_error!(reply, errno);
            return;
        }
        let subject = self.request_identity(req);
        let parent_attrs = match self.attrs_for_path(&parent_path) {
            Ok(Some(attrs)) => attrs,
            Ok(None) => {
                fuse_reply_error!(reply, ENOENT);
                return;
            }
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        match self.acl_allows(&parent_path, &parent_attrs.file_attr, libc::X_OK, &subject) {
            Ok(true) => {}
            Ok(false) => {
                fuse_reply_error!(reply, EACCES);
                return;
            }
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        }
        let child_path = Self::join_path(&parent_path, name);
        self.log_request_start(
            req_id,
            "lookup",
            format!("parent={} child={}", parent_path, child_path),
        );
        match self.lookup_path(&child_path) {
            Ok(Some(attrs)) => {
                self.register_lookup_path(&child_path, attrs.file_attr.ino.0);
                reply.entry_with_ttls(
                    &self.attr_cache_ttl_live(),
                    &self.entry_cache_ttl_live(),
                    &attrs.file_attr,
                    Generation(0),
                );
            }
            Ok(None) => {
                let ttl = self.negative_cache_ttl_live();
                if ttl.is_zero() {
                    fuse_reply_error!(reply, ENOENT);
                } else {
                    reply.negative(&ttl);
                }
            }
            Err(errno) => fuse_reply_error!(reply, errno),
        }
    }

    fn forget(&self, _req: &Request, ino: INodeNo, nlookup: u64) {
        let (remaining, evicted) = self.forget_lookup_refs(ino.0, nlookup);
        self.profile_forget_event(ino.0, nlookup, remaining, evicted);
    }

    fn getattr(&self, req: &Request, ino: INodeNo, fh: Option<FileHandle>, reply: ReplyAttr) {
        let ino = ino.0;
        let req_id = self.next_request_id();
        self.log_request_start(req_id, "getattr", format!("ino={} fh={:?}", ino, fh));
        if let Some(fh) = fh {
            let fh = fh.0;
            let file_id = self
                .fh_table
                .lock()
                .ok()
                .and_then(|guard| guard.get(&fh).and_then(|state| state.file_id));
            if let Some(file_id) = file_id {
                match self.attrs_for_open_file_id(file_id) {
                    Ok(Some(mut attrs)) => {
                        let write_state = self.write_state_for_handle(fh);
                        self.overlay_pending_write_size(&mut attrs, write_state.as_ref());
                        reply.attr(&self.attr_cache_ttl_live(), &attrs.file_attr);
                        return;
                    }
                    Ok(None) => {
                        fuse_reply_error!(reply, ENOENT);
                        return;
                    }
                    Err(errno) => {
                        self.log_request_error(
                            req_id,
                            "getattr",
                            errno,
                            format!("ino={} fh={} file_id={}", ino, fh, file_id),
                        );
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                }
            }
        }
        // Linux uses this reply for local `default_permissions` checks when
        // cached attributes expire. Do not repopulate its mode/owner cache
        // from a lagging replica on a replica-read-routed mount.
        match self.authoritative_path_attrs_for_ino(ino) {
            Ok((path, mut attrs)) => {
                if fh.is_none() {
                    // A path lookup may still resolve the detached database
                    // row for an open, unlinked inode. `fstat()` must remain
                    // bound to that live file_id and must not reauthorize its
                    // former pathname after the parent has been removed.
                    if attrs.file_attr.kind == FileType::RegularFile {
                        match self.attrs_for_open_inode(ino) {
                            Ok(Some(open_attrs)) if open_attrs.file_attr.nlink == 0 => {
                                reply.attr(&self.attr_cache_ttl_live(), &open_attrs.file_attr);
                                return;
                            }
                            Ok(_) => {}
                            Err(errno) => {
                                fuse_reply_error!(reply, errno);
                                return;
                            }
                        }
                    }
                    let subject = self.request_identity(req);
                    if let Err(errno) = self.check_path_search_permissions(&path, &subject) {
                        self.log_request_error(
                            req_id,
                            "getattr",
                            errno,
                            format!("path={} search_permission", path),
                        );
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                    let write_state = self.pending_write_state_for_path(&path);
                    self.overlay_pending_write_size(&mut attrs, write_state.as_ref());
                }
                self.register_path(&path, attrs.file_attr.ino.0);
                reply.attr(&self.attr_cache_ttl_live(), &attrs.file_attr);
            }
            Err(ENOENT) => match self.attrs_for_open_inode(ino) {
                Ok(Some(attrs)) => {
                    debug!(
                        "FOD req={} op=getattr open_inode_fallback ino={} nlink={}",
                        req_id, ino, attrs.file_attr.nlink
                    );
                    reply.attr(&self.attr_cache_ttl_live(), &attrs.file_attr);
                }
                Ok(None) => {
                    self.log_request_error(req_id, "getattr", ENOENT, format!("ino={}", ino));
                    fuse_reply_error!(reply, ENOENT);
                }
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "getattr",
                        errno,
                        format!("ino={} open_inode_fallback", ino),
                    );
                    fuse_reply_error!(reply, errno);
                }
            },
            Err(errno) => {
                self.log_request_error(req_id, "getattr", errno, format!("ino={}", ino));
                fuse_reply_error!(reply, errno);
            }
        }
    }

    fn opendir(&self, req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        let ino = ino.0;
        let (path, parsed_attrs) = match self.authoritative_entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let directory_id = if ino == ROOT_INO {
            None
        } else {
            Some(parsed_attrs.xattr_owner_id)
        };
        let attrs = parsed_attrs.file_attr;
        if attrs.kind != FileType::Directory {
            fuse_reply_error!(reply, ENOTDIR);
            return;
        }
        let subject = self.request_identity(req);
        match self.acl_allows(&path, &attrs, libc::R_OK, &subject) {
            Ok(true) => {}
            Ok(false) => {
                fuse_reply_error!(reply, EACCES);
                return;
            }
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        }
        let metadata_profile_enabled = fod_fuse_profile_metadata_cache_enabled();
        let snapshot_started = metadata_profile_enabled.then(Instant::now);
        let entries = if fod_fuse_readdir_batch_metadata_enabled() {
            match self
                .repo
                .list_directory_entry_metadata_for_id(directory_id)
            {
                Ok(entries) => DirectoryEntrySnapshot::Metadata(entries),
                Err(err) => {
                    warn!("FOD opendir metadata snapshot failed path={} err={}", path, err);
                    fuse_reply_error!(reply, EIO);
                    return;
                }
            }
        } else {
            match self.repo.list_directory_entries_blob_for_id(directory_id) {
                Ok(Some(blob)) => DirectoryEntrySnapshot::Names(Self::decode_nul_fields(&blob)),
                Ok(None) => DirectoryEntrySnapshot::Names(Vec::new()),
                Err(err) => {
                    warn!("FOD opendir name snapshot failed path={} err={}", path, err);
                    fuse_reply_error!(reply, EIO);
                    return;
                }
            }
        };
        if let Some(started) = snapshot_started {
            let elapsed = duration_to_micros(started.elapsed());
            self.metadata_profile_readdir_list_us
                .fetch_add(elapsed, Ordering::Relaxed);
            self.metadata_profile_readdir_total_us
                .fetch_add(elapsed, Ordering::Relaxed);
        }
        let parent_path = path
            .rsplit_once('/')
            .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
            .unwrap_or("/");
        let parent_ino = if ino == ROOT_INO {
            ROOT_INO
        } else {
            self.inode_for_path(parent_path).unwrap_or(ROOT_INO)
        };
        let fh = self.next_handle();
        let state = DirectoryHandleState {
            ino,
            path,
            directory_id,
            attrs,
            parent_ino,
            entries,
            highest_requested_offset: 0,
        };
        self.directory_fh_table
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(fh, Arc::new(Mutex::new(state)));
        reply.opened(FileHandle(fh), FopenFlags::empty());
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let ino = ino.0;
        let req_id = self.next_request_id();
        let Some(state) = self
            .directory_fh_table
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(&fh.0)
            .cloned()
        else {
            fuse_reply_error!(reply, EBADF);
            return;
        };
        let (path, directory_id, parent_ino, entries) = {
            let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
            if state.ino != ino {
                fuse_reply_error!(reply, EBADF);
                return;
            }
            // Cookie zero after any stream progress is a rewind. Refresh the
            // per-open snapshot once so a restarted stream sees current names.
            if offset == 0 && state.highest_requested_offset > 0 {
                state.entries = if fod_fuse_readdir_batch_metadata_enabled() {
                    match self
                        .repo
                        .list_directory_entry_metadata_for_id(state.directory_id)
                    {
                        Ok(entries) => DirectoryEntrySnapshot::Metadata(entries),
                        Err(err) => {
                            warn!(
                                "FOD readdir rewind metadata snapshot failed path={} err={}",
                                state.path, err
                            );
                            fuse_reply_error!(reply, EIO);
                            return;
                        }
                    }
                } else {
                    match self
                        .repo
                        .list_directory_entries_blob_for_id(state.directory_id)
                    {
                        Ok(Some(blob)) => {
                            DirectoryEntrySnapshot::Names(Self::decode_nul_fields(&blob))
                        }
                        Ok(None) => DirectoryEntrySnapshot::Names(Vec::new()),
                        Err(err) => {
                            warn!(
                                "FOD readdir rewind name snapshot failed path={} err={}",
                                state.path, err
                            );
                            fuse_reply_error!(reply, EIO);
                            return;
                        }
                    }
                };
                state.highest_requested_offset = 0;
            }
            state.highest_requested_offset = state.highest_requested_offset.max(offset);
            (
                state.path.clone(),
                state.directory_id,
                state.parent_ino,
                state.entries.clone(),
            )
        };
        self.log_request_start(
            req_id,
            "readdir",
            format!("path={} ino={} fh={} offset={}", path, ino, fh.0, offset),
        );
        let started = Instant::now();
        let mut emitted = 0u64;
        if offset < 1 && reply.add(INodeNo(ino), 1, FileType::Directory, ".") {
            reply.ok();
            return;
        }
        if offset < 2 && reply.add(INodeNo(parent_ino), 2, FileType::Directory, "..") {
            reply.ok();
            return;
        }
        match entries {
            DirectoryEntrySnapshot::Metadata(entries) => {
                for (index, entry) in entries
                    .into_iter()
                    .enumerate()
                    .skip(readdir_child_entries_to_skip(offset))
                {
                    let kind = match Self::directory_entry_file_type(
                        &entry.kind,
                        entry.special_type.as_deref(),
                    ) {
                        Ok(kind) => kind,
                        Err(errno) => {
                            warn!(
                                "FOD readdir invalid child type path={} name={} type={} special_type={:?}",
                                path, entry.name, entry.kind, entry.special_type
                            );
                            fuse_reply_error!(reply, errno);
                            return;
                        }
                    };
                    let inode = match self.stable_inode(
                        &entry.kind,
                        &entry.inode_seed,
                        entry.entry_id,
                    ) {
                        Ok(inode) => inode,
                        Err(errno) => {
                            fuse_reply_error!(reply, errno);
                            return;
                        }
                    };
                    if fod_fuse_readdir_register_paths_enabled() {
                        let child_path =
                            Self::join_path(&path, OsStr::from_bytes(entry.name.as_bytes()));
                        self.register_path(&child_path, inode);
                    }
                    emitted = emitted.saturating_add(1);
                    if reply.add(INodeNo(inode), (index + 3) as u64, kind, entry.name) {
                        break;
                    }
                }
            }
            DirectoryEntrySnapshot::Names(names) => {
                for (index, name) in names
                    .into_iter()
                    .enumerate()
                    .skip(readdir_child_entries_to_skip(offset))
                {
                    let entry = match self
                        .repo
                        .directory_entry_metadata_for_id_and_name(directory_id, &name)
                    {
                        Ok(Some(entry)) => entry,
                        Ok(None) => continue,
                        Err(err) => {
                            warn!(
                                "FOD readdir child metadata lookup failed path={} name={} directory_id={:?} err={}",
                                path, name, directory_id, err
                            );
                            fuse_reply_error!(reply, EIO);
                            return;
                        }
                    };
                    let kind = match Self::directory_entry_file_type(
                        &entry.kind,
                        entry.special_type.as_deref(),
                    ) {
                        Ok(kind) => kind,
                        Err(errno) => {
                            warn!(
                                "FOD readdir invalid child type path={} name={} type={} special_type={:?}",
                                path, entry.name, entry.kind, entry.special_type
                            );
                            fuse_reply_error!(reply, errno);
                            return;
                        }
                    };
                    let inode = match self.stable_inode(
                        &entry.kind,
                        &entry.inode_seed,
                        entry.entry_id,
                    ) {
                        Ok(inode) => inode,
                        Err(errno) => {
                            fuse_reply_error!(reply, errno);
                            return;
                        }
                    };
                    if fod_fuse_readdir_register_paths_enabled() {
                        let child_path =
                            Self::join_path(&path, OsStr::from_bytes(entry.name.as_bytes()));
                        self.register_path(&child_path, inode);
                    }
                    emitted = emitted.saturating_add(1);
                    if reply.add(INodeNo(inode), (index + 3) as u64, kind, name) {
                        break;
                    }
                }
            }
        }
        if fod_fuse_profile_metadata_cache_enabled() {
            self.metadata_profile_readdir_calls
                .fetch_add(1, Ordering::Relaxed);
            self.metadata_profile_readdir_entries
                .fetch_add(emitted, Ordering::Relaxed);
            self.metadata_profile_readdir_total_us
                .fetch_add(duration_to_micros(started.elapsed()), Ordering::Relaxed);
        }
        if !matches!(
            self.atime_policy,
            AtimePolicy::NoAtime | AtimePolicy::Nodiratime
        ) {
            if let Some(directory_id) = directory_id {
                let state = self
                    .directory_fh_table
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .get(&fh.0)
                    .cloned();
                if let Some(state) = state {
                    let mut state = state.lock().unwrap_or_else(|err| err.into_inner());
                    if let Some(atime) =
                        self.touch_directory_access_time_by_id(directory_id, &state.attrs)
                    {
                        state.attrs.atime = atime;
                    }
                }
            } else if let Ok(Some(current_attrs)) = self.attrs_for_path(&path) {
                if current_attrs.file_attr.ino.0 == ino {
                    self.touch_access_time(&path, &current_attrs.file_attr);
                }
            }
        }
        reply.ok();
    }

    fn releasedir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        reply: ReplyEmpty,
    ) {
        let removed = self
            .directory_fh_table
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&fh.0);
        if removed.is_some() {
            reply.ok();
        } else {
            fuse_reply_error!(reply, EBADF);
        }
    }

    fn readlink(&self, req: &Request, ino: INodeNo, reply: ReplyData) {
        let ino = ino.0;
        let _read_profile = self.start_fuse_read_profile();
        let (path, attrs) = match self.authoritative_entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        if attrs.file_attr.kind != FileType::Symlink || attrs.xattr_owner_kind != "symlink" {
            fuse_reply_error!(reply, ENOENT);
            return;
        }
        let subject = self.request_identity(req);
        if let Err(errno) = self.check_path_search_permissions(&path, &subject) {
            fuse_reply_error!(reply, errno);
            return;
        }
        let authority = self.repo.authoritative_view();
        match authority.load_symlink_target(attrs.xattr_owner_id) {
            Ok(Some(target)) => {
                self.touch_symlink_access_time_by_id(attrs.xattr_owner_id, &attrs.file_attr);
                self.reply_data_profiled(reply, target.as_bytes())
            }
            Ok(None) => fuse_reply_error!(reply, ENOENT),
            Err(_) => fuse_reply_error!(reply, EIO),
        }
    }

    fn statfs(&self, req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        const STATFS_FREE_INODE_HEADROOM: u64 = 1_000_000;
        const STATFS_NAME_MAX: u32 = 255;
        let request_id = req.unique();
        self.register_statfs_reply(request_id, reply);

        if std::env::var_os("FOD_TEST_STATFS_FORCE_EIO").is_some() {
            warn!("FOD statfs snapshot failed err=test fault injection");
            self.finish_statfs_reply(request_id, Err(EIO));
            return;
        }

        let snapshot = match self.cached_statfs_snapshot() {
            Ok(snapshot) => snapshot,
            Err(err) => {
                warn!("FOD statfs snapshot failed err={}", err);
                self.finish_statfs_reply(request_id, Err(EIO));
                return;
            }
        };
        let capacity = match self.statfs_capacity(snapshot.max_fs_size_bytes) {
            Ok(capacity) => capacity,
            Err(err) => {
                warn!("FOD statfs capacity failed err={}", err);
                self.finish_statfs_reply(request_id, Err(EIO));
                return;
            }
        };
        let total_blocks = capacity
            .map(|capacity| capacity.total_bytes / self.block_size)
            .unwrap_or(snapshot.blocks);
        let used_blocks = snapshot.blocks.min(total_blocks);
        let capacity_free_blocks = total_blocks.saturating_sub(used_blocks);
        let available_blocks = capacity
            .map(|capacity| capacity.available_bytes / self.block_size)
            .unwrap_or(capacity_free_blocks);
        let free_blocks = capacity_free_blocks.min(available_blocks);
        let used_inodes = snapshot
            .files
            .saturating_add(snapshot.dirs)
            .saturating_add(snapshot.symlinks);
        // PostgreSQL-backed FOD has no fixed inode pool. Report stable virtual
        // headroom so callers do not interpret f_ffree=0 as inode exhaustion.
        let free_inodes = STATFS_FREE_INODE_HEADROOM;
        let total_inodes = used_inodes.saturating_add(free_inodes);
        debug!(
            "FOD statfs blocks={} used_blocks={} files={} dirs={} symlinks={} total_inodes={} free_inodes={} total_data_size={} reserved_data_size={} block_size={} max_fs_size_bytes={:?} pg_visible_path={:?}",
            total_blocks,
            used_blocks,
            snapshot.files,
            snapshot.dirs,
            snapshot.symlinks,
            total_inodes,
            free_inodes,
            snapshot.total_data_size,
            snapshot.reserved_data_size,
            self.block_size,
            snapshot.max_fs_size_bytes,
            self.pg_visible_path.as_deref().map(|path| path.display().to_string())
        );
        self.finish_statfs_reply(
            request_id,
            Ok(StatfsReplyData {
                blocks: total_blocks,
                free_blocks,
                total_inodes,
                free_inodes,
                block_size: self.block_size as u32,
                name_max: STATFS_NAME_MAX,
            }),
        );
    }

    fn setxattr(
        &self,
        req: &Request,
        ino: INodeNo,
        name: &OsStr,
        value: &[u8],
        flags: i32,
        position: u32,
        reply: ReplyEmpty,
    ) {
        let ino = ino.0;
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        if position != 0 {
            fuse_reply_error!(reply, EOPNOTSUPP);
            return;
        }
        if flags & !(libc::XATTR_CREATE | libc::XATTR_REPLACE) != 0
            || flags & libc::XATTR_CREATE != 0 && flags & libc::XATTR_REPLACE != 0
        {
            fuse_reply_error!(reply, EINVAL);
            return;
        }
        let name = match Self::os_str_as_utf8(name) {
            Ok(name) => name.to_owned(),
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let (path, attrs) = match self.authoritative_entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let owner = (attrs.xattr_owner_kind, attrs.xattr_owner_id);
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
            fuse_reply_error!(reply, EOPNOTSUPP);
            return;
        }
        if !self.acl_enabled && name == "system.posix_acl_access" {
            warn!(
                "FOD system.posix_acl_access xattr rejected because ACL support is disabled path={}",
                path
            );
            fuse_reply_error!(reply, EOPNOTSUPP);
            return;
        }
        let posix_acl_transaction = self.acl_enabled
            && ((name == "system.posix_acl_access" && matches!(owner.0.as_str(), "file" | "dir"))
                || (name == "system.posix_acl_default" && owner.0 == "dir"));
        if name.starts_with("system.posix_acl_") && !posix_acl_transaction {
            fuse_reply_error!(
                reply,
                if self.acl_enabled {
                    EINVAL
                } else {
                    EOPNOTSUPP
                }
            );
            return;
        }
        let acl_mode_bits = if posix_acl_transaction {
            if name == "system.posix_acl_access" {
                match Self::acl_access_mode_bits(value) {
                    Ok(value) => Some(value),
                    Err(errno) => {
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                }
            } else {
                match Self::parse_posix_acl_xattr(value)
                    .and_then(|entries| Self::mode_from_acl_entries(&entries, 0))
                {
                    Ok(_) => None,
                    Err(errno) => {
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                }
            }
        } else {
            None
        };

        if posix_acl_transaction {
            let subject = self.request_identity(req);
            match self.repo.store_posix_acl_for_owner(
                &owner.0,
                owner.1,
                &name,
                value,
                acl_mode_bits,
                flags & libc::XATTR_CREATE != 0,
                flags & libc::XATTR_REPLACE != 0,
                subject.uid,
            ) {
                Ok(()) => {
                    debug!(
                        "FOD setxattr stored ACL and synchronized mode path={}",
                        path
                    );
                    reply.ok();
                }
                Err(err) => {
                    warn!(
                        "FOD setxattr ACL transaction failed path={} err={}",
                        path, err
                    );
                    fuse_reply_error!(reply, crate::errno::xattr_errno(&err));
                }
            }
            return;
        }

        let subject = self.request_identity(req);
        let result = self.repo.store_xattr_value_for_owner_as(
            &owner.0,
            owner.1,
            &name,
            value,
            flags & libc::XATTR_CREATE != 0,
            flags & libc::XATTR_REPLACE != 0,
            subject.uid,
            subject.gid,
            &subject.groups.iter().copied().collect::<Vec<_>>(),
            self.acl_enabled,
        );
        match result {
            Ok(_) => {
                debug!("FOD setxattr stored path={} name={}", path, name);
                reply.ok()
            }
            Err(err) => {
                warn!("FOD setxattr failed path={} name={}", path, name);
                fuse_reply_error!(reply, crate::errno::xattr_errno(&err))
            }
        }
    }

    fn getxattr(&self, req: &Request, ino: INodeNo, name: &OsStr, size: u32, reply: ReplyXattr) {
        let ino = ino.0;
        let _read_profile = self.start_fuse_read_profile();
        self.profile.record_getxattr();
        let name = match Self::os_str_as_utf8(name) {
            Ok(name) => name.to_owned(),
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let (path, attrs) = match self.authoritative_entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        debug!("FOD getxattr path={} name={} size={}", path, name, size);
        let subject = self.request_identity(req);
        if name.starts_with("trusted.") && subject.uid != 0 {
            fuse_reply_error!(reply, ENODATA);
            return;
        }
        if name.starts_with("user.") {
            if !matches!(attrs.file_attr.kind, FileType::RegularFile | FileType::Directory) {
                fuse_reply_error!(reply, ENODATA);
                return;
            }
            match self.acl_allows_for_attrs(&attrs, libc::R_OK, &subject) {
                Ok(true) => {}
                Ok(false) => {
                    fuse_reply_error!(reply, ENODATA);
                    return;
                }
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            }
        }

        // Linux probes security.capability repeatedly while writing. Use the
        // file-handle fast-path counter only when its file_id matches the
        // owner resolved from the verified inode; both cases fetch the value
        // directly by that PostgreSQL owner ID.
        let open_file_id = self.open_file_id_for_path(&path);
        if attrs.xattr_owner_kind == "file" && open_file_id == Some(attrs.xattr_owner_id) {
            self.profile.record_getxattr_open_file_owner_fast_path();
        } else {
            self.profile.record_getxattr_path_resolution_fallback();
        }
        let result = self.repo.fetch_xattr_value_for_owner(
            &attrs.xattr_owner_kind,
            attrs.xattr_owner_id,
            &name,
        );

        match result {
            Ok(Some(value)) => {
                if size == 0 {
                    reply.size(value.len() as u32);
                } else if size < value.len() as u32 {
                    fuse_reply_error!(reply, ERANGE);
                } else {
                    self.reply_xattr_profiled(reply, &value);
                }
            }
            Ok(None) => {
                debug!("FOD getxattr missing path={} name={}", path, name);
                fuse_reply_error!(reply, ENODATA)
            }
            Err(_) => {
                warn!("FOD getxattr failed path={} name={}", path, name);
                fuse_reply_error!(reply, EIO)
            }
        }
    }

    fn listxattr(&self, req: &Request, ino: INodeNo, size: u32, reply: ReplyXattr) {
        let ino = ino.0;
        let _read_profile = self.start_fuse_read_profile();
        let (path, attrs) = match self.authoritative_entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let owner = (attrs.xattr_owner_kind, attrs.xattr_owner_id);
        let actor_uid = self.request_identity(req).uid;
        debug!(
            "FOD listxattr path={} owner_kind={} owner_id={}",
            path, owner.0, owner.1
        );
        match self.repo.list_xattr_names_for_owner(&owner.0, owner.1) {
            Ok(names) => {
                let mut payload = Vec::new();
                for name in names
                    .into_iter()
                    .filter(|name| actor_uid == 0 || !name.starts_with("trusted."))
                {
                    payload.extend_from_slice(name.as_bytes());
                    payload.push(0);
                }
                if size == 0 {
                    reply.size(payload.len() as u32);
                } else if size < payload.len() as u32 {
                    fuse_reply_error!(reply, ERANGE);
                } else {
                    self.reply_xattr_profiled(reply, &payload);
                }
            }
            Err(_) => {
                warn!("FOD listxattr failed path={}", path);
                fuse_reply_error!(reply, EIO)
            }
        }
    }

    fn removexattr(&self, req: &Request, ino: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let ino = ino.0;
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        let name = match Self::os_str_as_utf8(name) {
            Ok(name) => name.to_owned(),
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let (path, attrs) = match self.authoritative_entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let owner = (attrs.xattr_owner_kind, attrs.xattr_owner_id);
        if name.starts_with("system.posix_acl_")
            && (!self.acl_enabled
                || (name == "system.posix_acl_default" && owner.0 != "dir")
                || (name == "system.posix_acl_access"
                    && !matches!(owner.0.as_str(), "file" | "dir")))
        {
            fuse_reply_error!(
                reply,
                if self.acl_enabled {
                    EINVAL
                } else {
                    EOPNOTSUPP
                }
            );
            return;
        }
        debug!("FOD removexattr path={} name={}", path, name);
        let subject = self.request_identity(req);
        match self
            .repo
            .remove_xattr_for_owner_as(
                &owner.0,
                owner.1,
                &name,
                subject.uid,
                subject.gid,
                &subject.groups.iter().copied().collect::<Vec<_>>(),
                self.acl_enabled,
            )
        {
            Ok(deleted) if deleted > 0 => {
                debug!("FOD removexattr removed path={} name={}", path, name);
                reply.ok()
            }
            Ok(_) => {
                debug!("FOD removexattr missing path={} name={}", path, name);
                fuse_reply_error!(reply, ENODATA)
            }
            Err(err) => {
                warn!("FOD removexattr failed path={} name={}", path, name);
                fuse_reply_error!(reply, crate::errno::xattr_errno(&err))
            }
        }
    }

    fn access(&self, req: &Request, ino: INodeNo, mask: AccessFlags, reply: ReplyEmpty) {
        let ino = ino.0;
        let mask = mask.bits();
        let subject = self.request_identity(req);
        let (path, attrs) = match self.authoritative_entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        if let Err(errno) = self.check_path_search_permissions(&path, &subject) {
            fuse_reply_error!(reply, errno);
            return;
        }
        debug!("FOD access path={} mask={:#x}", path, mask);
        // Linux permits root to bypass read/write mode checks, but X_OK on a
        // regular file still requires at least one execute bit to be set.
        // Keep this access(2) rule local: other root permission checks use
        // the ordinary privileged bypass in `acl_allows`.
        if subject.is_root()
            && attrs.file_attr.kind == FileType::RegularFile
            && mask & libc::X_OK != 0
            && attrs.file_attr.perm & 0o111 == 0
        {
            fuse_reply_error!(reply, EACCES);
            return;
        }
        match self.acl_allows(&path, &attrs.file_attr, mask, &subject) {
            Ok(true) => {
                debug!("FOD access granted path={} mask={:#x}", path, mask);
                reply.ok()
            }
            Ok(false) => {
                debug!("FOD access denied path={} mask={:#x}", path, mask);
                fuse_reply_error!(reply, EACCES)
            }
            Err(errno) => {
                warn!(
                    "FOD access check failed path={} mask={:#x} errno={}",
                    path, mask, errno
                );
                fuse_reply_error!(reply, errno)
            }
        }
    }

    fn ioctl(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        _flags: IoctlFlags,
        cmd: u32,
        in_data: &[u8],
        out_size: u32,
        reply: ReplyIoctl,
    ) {
        let ino = ino.0;
        let fh = fh.0;
        if self.read_only && ioctl_is_mutating(cmd) {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        let handle = match self.file_handle_state_for_handle(fh) {
            Some(handle) => handle,
            None => {
                fuse_reply_error!(reply, EBADF);
                return;
            }
        };
        let (path, attrs) = match handle.file_id {
            Some(file_id) => match self.attrs_for_open_file_id(file_id) {
                Ok(Some(attrs)) if attrs.file_attr.ino.0 == ino => (handle.path, attrs),
                Ok(_) => {
                    fuse_reply_error!(reply, ENOENT);
                    return;
                }
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            },
            None => match self.entry_attrs_for_ino(ino) {
                Ok(value) => value,
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            },
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
                        fuse_reply_error!(reply, errno);
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
                    fuse_reply_error!(reply, EOPNOTSUPP);
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
                            fuse_reply_error!(reply, errno);
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
                    fuse_reply_error!(reply, EOPNOTSUPP);
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
                    fuse_reply_error!(reply, ENOTTY);
                    return;
                }
                let available = size.min(u32::MAX as u64) as u32;
                reply.ioctl(0, &available.to_ne_bytes());
            }
            _ => fuse_reply_error!(reply, ENOTTY),
        }
    }

    fn poll(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        _ph: PollNotifier,
        _events: PollEvents,
        _flags: PollFlags,
        reply: ReplyPoll,
    ) {
        let ino = ino.0;
        let fh = fh.0;
        let (path, attrs) = match self.entry_attrs_for_ino(ino) {
            Ok(path) => path,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        if attrs.file_attr.kind != FileType::RegularFile {
            fuse_reply_error!(reply, ENOTTY);
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
        reply.poll(PollEvents::from_bits_retain(revents));
    }

    fn open(&self, req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let ino = ino.0;
        let flags = flags.0;
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
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        if let Err(errno) = self.check_path_search_permissions(&path, &subject) {
            self.log_request_error(
                req_id,
                "open",
                errno,
                format!("path={} search_permission", path),
            );
            fuse_reply_error!(reply, errno);
            return;
        }
        self.log_request_start(
            req_id,
            "open",
            format!("path={} ino={} flags={:#x}", path, ino, flags),
        );
        let file_id = match self.file_id_for_path(&path) {
            Ok(Some(value)) => value,
            Ok(None) => {
                fuse_reply_error!(reply, ENOENT);
                return;
            }
            Err(errno) => {
                fuse_reply_error!(reply, errno);
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
                EACCES,
                format!("path={} denied_by_acl flags={:#x}", path, flags),
            );
            fuse_reply_error!(reply, EACCES);
            return;
        }
        if self.read_only
            && ((flags & libc::O_ACCMODE) != libc::O_RDONLY || (flags & libc::O_TRUNC) != 0)
        {
            self.log_request_error(
                req_id,
                "open",
                EROFS,
                format!("path={} read_only flags={:#x}", path, flags),
            );
            fuse_reply_error!(reply, EROFS);
            return;
        }
        let writable = (flags & libc::O_ACCMODE) != libc::O_RDONLY;
        let fh = if writable {
            match self.create_writable_handle_with_ownership(path.clone(), file_id, flags) {
                Ok(fh) => fh,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "open",
                        errno,
                        format!(
                            "path={} file_id={} write ownership flags={:#x}",
                            path, file_id, flags
                        ),
                    );
                    fuse_reply_error!(reply, errno);
                    return;
                }
            }
        } else {
            0
        };

        // Przy drugim lub kolejnym fh pending state starszego uchwytu musi
        // byc widoczny juz dla fstat/stat/SEEK_END nowego uchwytu. Dla writable
        // handle ownership jest juz zdobyty przed jakimkolwiek flush/truncate.
        let existing_handle_count = self.open_handle_count_for_file(file_id);
        let publish_pending = if writable {
            existing_handle_count > 1
        } else {
            existing_handle_count > 0
        };
        if !self.read_only && publish_pending {
            if let Err(errno) = self.flush_pending_write_states_for_file_except(file_id, u64::MAX) {
                if writable {
                    self.rollback_open_handle(fh);
                }
                self.log_request_error(
                    req_id,
                    "open",
                    errno,
                    format!(
                        "path={} file_id={} publish pending write state",
                        path, file_id
                    ),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
            debug!(
                "FOD req={} op=open pending_write_visibility file_id={} published_before_new_handle=true",
                req_id, file_id
            );
        }

        let fh = if writable {
            fh
        } else {
            match self.create_handle_for_file(path.clone(), Some(file_id), flags) {
                Ok(fh) => fh,
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            }
        };

        if writable && (flags & libc::O_TRUNC) != 0 {
            let mut state = Self::new_write_state(file_id, 0, true);
            if let Err(errno) = self.attach_write_persistence_fence_for_handle(fh, &mut state) {
                self.rollback_open_handle(fh);
                self.log_request_error(
                    req_id,
                    "open",
                    errno,
                    format!("path={} file_id={} O_TRUNC fence", path, file_id),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
            if let Err(errno) = self.flush_write_state(&mut state) {
                self.rollback_open_handle(fh);
                self.log_request_error(
                    req_id,
                    "open",
                    errno,
                    format!("path={} file_id={} O_TRUNC flush", path, file_id),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
            self.remove_write_state(fh);
            debug!(
                "FOD req={} op=open atomic_truncate path={} file_id={} fh={}",
                req_id, path, file_id, fh
            );
        }

        debug!("FOD open granted fh={} writable={}", fh, writable);
        reply.opened(FileHandle(fh), self.fopen_flags());
    }

    fn getlk(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        lock_owner: LockOwner,
        start: u64,
        end: u64,
        typ: i32,
        _pid: u32,
        reply: ReplyLock,
    ) {
        if self.degraded_read_only {
            fuse_reply_error!(reply, EOPNOTSUPP);
            return;
        }
        let ino = ino.0;
        let fh = fh.0;
        let lock_owner = lock_owner.0;
        let (resource_key, path) = match self.resource_key_for_lock(fh, ino) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let owner = lock_owner;
        debug!(
            "FOD getlk path={} owner={} start={} end={} typ={}",
            path, owner, start, end, typ
        );
        let records = match self.load_lock_records_for_key(&resource_key) {
            Ok(records) => records,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        if let Some(conflict) = Self::lock_conflict(&records, owner, typ, start, end) {
            let conflict_end = conflict.end.unwrap_or(i64::MAX as u64);
            debug!(
                "FOD getlk conflict path={} owner={} conflict_owner={} start={} end={} typ={}",
                path,
                owner,
                conflict.owner,
                conflict.start,
                conflict_end,
                conflict.typ
            );
            reply.locked(
                conflict.start,
                conflict_end,
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
        &self,
        req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        lock_owner: LockOwner,
        start: u64,
        end: u64,
        typ: i32,
        pid: u32,
        sleep_flag: bool,
        reply: ReplyEmpty,
    ) {
        let (pending_wait, _pending_wait_guard) = if sleep_flag {
            let pending = Arc::new(PendingLockWait {
                cancelled: AtomicBool::new(false),
                waiter: thread::current(),
            });
            match self.pending_lock_waits.lock() {
                Ok(mut waits) => {
                    waits.insert(req.unique(), pending.clone());
                }
                Err(_) => {
                    fuse_reply_error!(reply, EIO);
                    return;
                }
            }
            let guard = PendingLockWaitGuard {
                waits: &self.pending_lock_waits,
                request_id: req.unique(),
            };
            (Some(pending), Some(guard))
        } else {
            (None, None)
        };

        if self.degraded_read_only {
            fuse_reply_error!(reply, EOPNOTSUPP);
            return;
        }
        let ino = ino.0;
        let fh = fh.0;
        let lock_owner = lock_owner.0;
        let (resource_key, path) = match self.resource_key_for_lock(fh, ino) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let wait_resource = if self.lock_backend_is_pg() && sleep_flag {
            match Self::lock_resource_kind_id(&resource_key) {
                Ok(value) => Some(value),
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            }
        } else {
            None
        };
        let owner = lock_owner;
        self.register_local_lock_owner(owner);
        debug!(
            "FOD setlk path={} owner={} start={} end={} typ={} pid={} sleep_flag={}",
            path, owner, start, end, typ, pid, sleep_flag
        );
        if typ == libc::F_UNLCK {
            if self.lock_backend_is_pg() {
                if let Err(errno) =
                    self.update_pg_lock_range(&resource_key, owner, start, end, typ, pid)
                {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            } else {
                let records = match self.load_lock_records_for_key(&resource_key) {
                    Ok(records) => records,
                    Err(errno) => {
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                };
                let records = Self::update_owner_lock_range(&records, owner, start, end, None);
                if let Err(errno) =
                    self.persist_lock_records_for_key(&resource_key, owner, &records)
                {
                    fuse_reply_error!(reply, errno);
                    return;
                }
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
            fuse_reply_error!(reply, EROFS);
            return;
        }

        let mut wait_edge_registered = false;
        loop {
            if wait_edge_registered {
                if let Err(err) = self.repo.clear_lock_wait_edges(owner, req.unique().0) {
                    warn!(
                        "FOD setlk clear wait edges failed owner={} wait_id={} err={}",
                        owner,
                        req.unique().0,
                        err
                    );
                    fuse_reply_error!(reply, EIO);
                    return;
                }
                wait_edge_registered = false;
            }
            if pending_wait
                .as_ref()
                .is_some_and(|pending| pending.cancelled.load(Ordering::Acquire))
            {
                fuse_reply_error!(reply, EINTR);
                return;
            }
            let acquired = if self.lock_backend_is_pg() {
                match self.update_pg_lock_range(&resource_key, owner, start, end, typ, pid) {
                    Ok(acquired) => acquired,
                    Err(errno) => {
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                }
            } else {
                let records = match self.load_lock_records_for_key(&resource_key) {
                    Ok(records) => records,
                    Err(errno) => {
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                };
                if Self::lock_conflict(&records, owner, typ, start, end).is_some() {
                    false
                } else {
                    let records = Self::update_owner_lock_range(
                        &records,
                        owner,
                        start,
                        end,
                        (typ != libc::F_UNLCK).then_some(PosixLockRecord {
                            owner,
                            typ,
                            start,
                            end: Some(end),
                            pid,
                        }),
                    );
                    if let Err(errno) =
                        self.persist_lock_records_for_key(&resource_key, owner, &records)
                    {
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                    true
                }
            };
            if acquired {
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
                fuse_reply_error!(reply, EWOULDBLOCK);
                return;
            }
            if self.lock_backend_is_pg() {
                let (resource_kind, resource_id) = wait_resource
                    .as_ref()
                    .expect("blocking PostgreSQL lock has a resource identity");
                match self.repo.register_lock_wait_and_detect_deadlock(
                    resource_kind,
                    *resource_id,
                    owner,
                    pid,
                    req.unique().0,
                    typ,
                    start,
                    end,
                    self.lock_lease_ttl.as_secs_f64().ceil().max(1.0) as u64,
                ) {
                    Ok(true) => {
                        debug!(
                            "FOD setlk deadlock path={} owner={} wait_id={}",
                            path,
                            owner,
                            req.unique().0
                        );
                        fuse_reply_error!(reply, EDEADLK);
                        return;
                    }
                    Ok(false) => wait_edge_registered = true,
                    Err(err) => {
                        warn!(
                            "FOD setlk wait graph update failed path={} owner={} err={}",
                            path, owner, err
                        );
                        fuse_reply_error!(reply, EIO);
                        return;
                    }
                }
            }
            if pending_wait
                .as_ref()
                .is_some_and(|pending| pending.cancelled.load(Ordering::Acquire))
            {
                if self.lock_backend_is_pg() {
                    if let Err(err) = self.repo.clear_lock_wait_edges(owner, req.unique().0) {
                        warn!(
                            "FOD setlk cancelled wait cleanup failed owner={} wait_id={} err={}",
                            owner,
                            req.unique().0,
                            err
                        );
                    }
                }
                fuse_reply_error!(reply, EINTR);
                return;
            }
            debug!(
                "FOD setlk waiting path={} owner={} typ={}",
                path, owner, typ
            );
            thread::park_timeout(self.lock_poll_interval);
        }
    }

    fn interrupt(&self, _req: &Request, interrupted_request_id: RequestId) -> bool {
        if self.interrupt_statfs(interrupted_request_id) {
            return true;
        }
        let pending = self
            .pending_lock_waits
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(&interrupted_request_id)
            .cloned();
        let Some(pending) = pending else {
            return false;
        };
        pending.cancelled.store(true, Ordering::Release);
        pending.waiter.unpark();
        true
    }

    fn bmap(&self, _req: &Request, ino: INodeNo, blocksize: u32, idx: u64, reply: ReplyBmap) {
        let ino = ino.0;
        if blocksize == 0 {
            fuse_reply_error!(reply, EINVAL);
            return;
        }
        let (path, attrs) = match self.entry_attrs_for_ino(ino) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let size = attrs.file_attr.size;
        let logical_blocks = if size == 0 {
            0
        } else {
            size.div_ceil(u64::from(blocksize))
        }
        .max(1);
        debug!(
            "FOD bmap path={} ino={} blocksize={} idx={} logical_blocks={}",
            path, ino, blocksize, idx, logical_blocks
        );
        if idx >= logical_blocks {
            fuse_reply_error!(reply, EINVAL);
            return;
        }
        reply.bmap(idx);
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        let fh = fh.0;
        let operation_lock = self.handle_operation_lock(fh);
        let _operation_guard = operation_lock
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let lock_owner = lock_owner.0;
        let _write_profile = self.start_fuse_write_profile();
        if self.file_handle_state_for_handle(fh).is_none() {
            fuse_reply_error!(reply, EBADF);
            return;
        }
        debug!(
            "FOD flush fh={} lock_owner={} read_only={}",
            fh, lock_owner, self.read_only
        );
        if self.read_only {
            // flush(2) jest wywolywany przez kernel takze podczas close()
            // czystego O_RDONLY. Read-only mount nie ma wtedy danych do
            // utrwalenia, wiec EROFS byloby bledem close(), a nie ochrona
            // przed modyfikacja filesystemu.
            self.clear_locks_for_owner_and_sync(lock_owner, "flush", fh);
            debug!(
                "FOD flush completed read-only fh={} lock_owner={}",
                fh, lock_owner
            );
            reply.ok();
            return;
        }
        self.register_local_lock_owner(lock_owner);
        match self.flush_write_state_for_handle(fh) {
            Ok(true) => {
                debug!("FOD flush persisted dirty state fh={}", fh);
            }
            Ok(false) => {
                debug!("FOD flush clean or missing state fh={}", fh);
            }
            Err(errno) => {
                warn!("FOD flush failed fh={} errno={}", fh, errno);
                fuse_reply_error!(reply, errno);
                return;
            }
        }
        self.clear_locks_for_owner_and_sync(lock_owner, "flush", fh);
        debug!("FOD flush completed fh={} lock_owner={}", fh, lock_owner);
        reply.ok();
    }

    fn fsync(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        datasync: bool,
        reply: ReplyEmpty,
    ) {
        let fh = fh.0;
        let operation_lock = self.handle_operation_lock(fh);
        let _operation_guard = operation_lock
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _write_profile = self.start_fuse_write_profile();
        if self.file_handle_state_for_handle(fh).is_none() {
            fuse_reply_error!(reply, EBADF);
            return;
        }
        debug!(
            "FOD fsync fh={} datasync={} read_only={}",
            fh, datasync, self.read_only
        );
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        match self.flush_write_state_for_handle(fh) {
            Ok(true) => debug!("FOD fsync persisted dirty state fh={}", fh),
            Ok(false) => debug!("FOD fsync clean or missing state fh={}", fh),
            Err(errno) => {
                warn!("FOD fsync failed fh={} errno={}", fh, errno);
                fuse_reply_error!(reply, errno);
                return;
            }
        }
        reply.ok();
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let ino = ino.0;
        let fh = fh.0;
        let req_id = self.next_request_id();
        let _read_profile = self.start_fuse_read_profile();
        let (_read_permit, read_task) =
            self.logical_read_gate
                .observe_task(&self.logical_read_tasks, u64::from(size), false);
        let handle = match self.file_handle_state_for_handle(fh) {
            Some(handle) => handle,
            None => {
                fuse_reply_error!(reply, EBADF);
                return;
            }
        };
        if handle.flags & libc::O_ACCMODE == libc::O_WRONLY {
            fuse_reply_error!(reply, EBADF);
            return;
        }
        let file_id = match self.file_id_for_handle_or_errno(fh, ino) {
            Ok(value) => value,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "read",
                    errno,
                    format!("ino={} fh={} offset={} size={}", ino, fh, offset, size),
                );
                fuse_reply_error!(reply, errno);
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
        if self.degraded_read_only {
            match self.read_degraded_snapshot(file_id, offset, u64::from(size)) {
                Ok(data) => {
                    read_task.complete(0, data.len() as u64);
                    self.reply_data_profiled(reply, &data);
                }
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "read",
                        errno,
                        format!("file_id={} degraded_snapshot", file_id),
                    );
                    fuse_reply_error!(reply, errno);
                }
            }
            return;
        }
        if let Err(errno) = self.flush_pending_write_states_for_file_except(file_id, fh) {
            self.log_request_error(
                req_id,
                "read",
                errno,
                format!("file_id={} flush pending sibling fh states", file_id),
            );
            fuse_reply_error!(reply, errno);
            return;
        }
        let size = size as u64;
        // Experimental fail-closed read coherence path. The repository query
        // resolves the file's CURRENT data_object_id and blocks together,
        // without trusting this mount's previously read block maps.
        // Preserve read-your-own-writes when this handle has pending changes.
        if Self::strict_cross_mount_reads_enabled() && self.write_state_for_handle(fh).is_none() {
            match self.read_noatime_direct_uncached(file_id, offset, size) {
                Ok(data) => {
                    read_task.complete(0, data.len() as u64);
                    self.reply_data_profiled(reply, &data);
                }
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "read",
                        errno,
                        format!("file_id={} strict_cross_mount_read", file_id),
                    );
                    fuse_reply_error!(reply, errno);
                }
            }
            return;
        }
        if self.atime_policy == AtimePolicy::NoAtime {
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
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                };
                read_task.complete(0, data.len() as u64);
                self.reply_data_profiled(reply, &data);
                return;
            }
            if self.noatime_direct_uncached_read_fast_path_enabled() {
                match self.read_noatime_direct_uncached(file_id, offset, size) {
                    Ok(data) => {
                        read_task.complete(0, data.len() as u64);
                        self.reply_data_profiled(reply, &data);
                    }
                    Err(errno) => {
                        self.log_request_error(
                            req_id,
                            "read",
                            errno,
                            format!("file_id={} fused_block_range_with_size", file_id),
                        );
                        fuse_reply_error!(reply, errno);
                    }
                }
                return;
            }
        }
        let read_metadata = match self.file_read_metadata_for_handle(fh, file_id) {
            Ok(value) => value,
            Err(err) => {
                self.log_request_error(
                    req_id,
                    "read",
                    EIO,
                    format!("file_id={} read metadata skipped err={}", file_id, err),
                );
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
                    fuse_reply_error!(reply, errno);
                    return;
                }
            };
            if let Some(metadata) = read_metadata.as_ref() {
                self.touch_file_access_time_from_read_metadata(file_id, metadata);
            }
            read_task.complete(0, data.len() as u64);
            self.reply_data_profiled(reply, &data);
            return;
        }
        let file_size = match read_metadata.as_ref() {
            Some(metadata) => metadata.size,
            None => match self.file_size_for_file_id_or_errno(file_id) {
                Ok(value) => value,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "read",
                        errno,
                        format!("file_id={} file_size", file_id),
                    );
                    fuse_reply_error!(reply, errno);
                    return;
                }
            },
        };
        if offset >= file_size {
            read_task.complete(0, 0);
            self.reply_data_profiled(reply, &[]);
            return;
        }
        let end_offset = offset.saturating_add(size).min(file_size);
        let live = self.reloadable_runtime();
        let first_block = offset / self.block_size;
        let last_block = (end_offset.saturating_sub(1)) / self.block_size;
        let total_blocks = 1 + (file_size - 1) / self.block_size.max(1);
        let (sequential, streak) = self.read_sequence_state_for_file(file_id, offset, end_offset);
        if first_block == last_block {
            if let Some(block) = self.cached_read_block(file_id, first_block) {
                let block_offset = first_block.saturating_mul(self.block_size);
                let slice_start = (offset.saturating_sub(block_offset)) as usize;
                let slice_end = (end_offset.saturating_sub(block_offset)) as usize;
                let data = block.as_ref();
                if slice_end <= data.len() && slice_start <= slice_end {
                    read_task.complete(0, slice_end.saturating_sub(slice_start) as u64);
                    self.reply_data_profiled(reply, &data[slice_start..slice_end]);
                    if let Some(metadata) = read_metadata.as_ref() {
                        self.touch_file_access_time_from_read_metadata(file_id, metadata);
                    }
                    return;
                }
            }
        } else if let Some(blocks) = self.cached_read_block_map(file_id, first_block, last_block) {
            let started = Instant::now();
            let data = assemble_read_slice(
                first_block,
                last_block,
                offset,
                end_offset,
                self.block_size,
                &blocks,
            );
            self.record_assemble_read_slice_elapsed(started.elapsed());
            if let Some(metadata) = read_metadata.as_ref() {
                self.touch_file_access_time_from_read_metadata(file_id, metadata);
            }
            read_task.complete(0, data.len() as u64);
            self.reply_data_profiled(reply, &data);
            return;
        }
        let (fetch_first, fetch_last) = if total_blocks <= live.small_file_read_threshold_blocks {
            (0, total_blocks.saturating_sub(1))
        } else {
            let requested_blocks = last_block.saturating_sub(first_block).saturating_add(1);
            let mut read_ahead_blocks = live.read_ahead_blocks;
            if sequential {
                let dynamic_ahead = live
                    .sequential_read_ahead_blocks
                    .saturating_mul(streak.max(1));
                read_ahead_blocks = read_ahead_blocks.max(dynamic_ahead);
                let direct_io_ahead = live
                    .direct_io_read_prefetch_blocks
                    .saturating_sub(requested_blocks);
                if self.fopen_direct_io && direct_io_ahead > 0 {
                    read_ahead_blocks = read_ahead_blocks.max(direct_io_ahead);
                }
            }
            let cache_cap = self.read_cache_limit_blocks().saturating_sub(1) as u64;
            read_ahead_blocks = read_ahead_blocks.min(cache_cap);
            (
                first_block,
                last_block
                    .saturating_add(read_ahead_blocks)
                    .min(total_blocks.saturating_sub(1)),
            )
        };
        if first_block == last_block {
            match self.read_block_map_target_block(file_id, fetch_first, fetch_last, first_block) {
                Ok(Some(block)) => {
                    let block_offset = first_block.saturating_mul(self.block_size);
                    let slice_start = (offset.saturating_sub(block_offset)) as usize;
                    let slice_end = (end_offset.saturating_sub(block_offset)) as usize;
                    let data = block.as_ref();
                    if slice_end <= data.len() && slice_start <= slice_end {
                        read_task.complete(0, slice_end.saturating_sub(slice_start) as u64);
                        self.reply_data_profiled(reply, &data[slice_start..slice_end]);
                        if let Some(metadata) = read_metadata.as_ref() {
                            self.touch_file_access_time_from_read_metadata(file_id, metadata);
                        }
                        return;
                    }
                    self.log_request_error(
                        req_id,
                        "read",
                        EIO,
                        format!(
                            "file_id={} block_index={} target block slice out of bounds",
                            file_id, first_block
                        ),
                    );
                    fuse_reply_error!(reply, EIO);
                    return;
                }
                Ok(None) => {
                    let data_len = end_offset.saturating_sub(offset) as usize;
                    let data = vec![0_u8; data_len];
                    if let Some(metadata) = read_metadata.as_ref() {
                        self.touch_file_access_time_from_read_metadata(file_id, metadata);
                    }
                    read_task.complete(0, data.len() as u64);
                    self.reply_data_profiled(reply, &data);
                    return;
                }
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "read",
                        errno,
                        format!("file_id={} read_block_map_target_block", file_id),
                    );
                    fuse_reply_error!(reply, errno);
                    return;
                }
            }
        }
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
                                read_task.complete(0, slice_end.saturating_sub(slice_start) as u64);
                                self.reply_data_profiled(reply, &data[slice_start..slice_end]);
                                if let Some(metadata) = read_metadata.as_ref() {
                                    self.touch_file_access_time_from_read_metadata(
                                        file_id, metadata,
                                    );
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
                if let Some(metadata) = read_metadata.as_ref() {
                    self.touch_file_access_time_from_read_metadata(file_id, metadata);
                }
                read_task.complete(0, data.len() as u64);
                self.reply_data_profiled(reply, &data)
            }
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "read",
                    errno,
                    format!("file_id={} read_block_map", file_id),
                );
                fuse_reply_error!(reply, errno)
            }
        }
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let fh = fh.0;
        let operation_lock = self.handle_operation_lock(fh);
        let _operation_guard = operation_lock
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let lock_owner = lock_owner.map(|owner| owner.0);
        let _write_profile = self.start_fuse_write_profile();
        debug!(
            "FOD release fh={} flags={:#x} lock_owner={:?} flush={} read_only={}",
            fh, _flags, lock_owner, _flush, self.read_only
        );
        if self.read_only {
            if let Some(owner) = lock_owner {
                self.clear_locks_for_owner_and_sync(owner, "release", fh);
            }
            self.remove_handle_state(fh);
            reply.ok();
            return;
        }
        if let Err(errno) = self.flush_write_state_for_handle(fh) {
            warn!("FOD release flush failed fh={} errno={}", fh, errno);
            if let Some(owner) = lock_owner {
                self.clear_locks_for_owner_and_sync(owner, "release", fh);
            }
            // Nie zwalniaj ownership po nieudanym finalnym flushu.
            self.remove_handle_state(fh);
            fuse_reply_error!(reply, errno);
            return;
        }
        if let Some(owner) = lock_owner {
            self.clear_locks_for_owner_and_sync(owner, "release", fh);
        }
        if let Err(errno) = self.release_write_ownership_for_handle(fh) {
            self.remove_handle_state(fh);
            fuse_reply_error!(reply, errno);
            return;
        }
        let removed_state = self.take_handle_state(fh);
        if let Err(errno) =
            self.finalize_deferred_unlink_after_handle_removal(removed_state.as_ref())
        {
            fuse_reply_error!(reply, errno);
            return;
        }
        debug!("FOD release completed fh={}", fh);
        reply.ok();
    }

    fn setattr(
        &self,
        req: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let ino = ino.0;
        let fh = fh.map(|handle| handle.0);
        let operation_lock = size
            .is_some()
            .then(|| fh.map(|fh| self.handle_operation_lock(fh)))
            .flatten();
        let _operation_guard = operation_lock.as_ref().map(|lock| {
            lock.lock().unwrap_or_else(|err| err.into_inner())
        });
        let subject = self.request_identity(req);
        let _write_profile = size.is_some().then(|| self.start_fuse_write_profile());
        if self.read_only
            && (SetattrMutationFlags {
                mode: mode.is_some(),
                uid: uid.is_some(),
                gid: gid.is_some(),
                size: size.is_some(),
                atime: atime.is_some(),
                mtime: mtime.is_some(),
                ctime: _ctime.is_some(),
                crtime: _crtime.is_some(),
                chgtime: _chgtime.is_some(),
                bkuptime: _bkuptime.is_some(),
                flags: _flags.is_some(),
            })
            .any()
        {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        let authority = self.repo.authoritative_view();
        let handle_state = fh.and_then(|handle| self.file_handle_state_for_handle(handle));
        let handle_file_id = handle_state.as_ref().and_then(|state| state.file_id);
        let path = match handle_state.as_ref() {
            Some(state) if handle_file_id.is_some() => state.path.clone(),
            _ => match self.entry_path_for_ino(ino) {
                Ok(path) => path,
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            },
        };
        let (current_attrs, kind, entry_id) = if let Some(file_id) = handle_file_id {
            match self.attrs_for_open_file_id(file_id) {
                Ok(Some(attrs)) if attrs.file_attr.ino.0 == ino => {
                    (attrs, "file".to_string(), Some(file_id))
                }
                Ok(_) => {
                    fuse_reply_error!(reply, ENOENT);
                    return;
                }
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            }
        } else {
            let current_attrs = match if ino == ROOT_INO {
                self.root_attr().map(Some)
            } else {
                self.attrs_for_path_from_repo(&authority, &path)
            } {
                Ok(Some(attrs)) if attrs.file_attr.ino.0 == ino => attrs,
                Ok(_) => {
                    fuse_reply_error!(reply, ENOENT);
                    return;
                }
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            };
            let resolved = match authority.resolve_path(&path) {
                Ok(value) => value,
                Err(_) => {
                    fuse_reply_error!(reply, EIO);
                    return;
                }
            };
            (current_attrs, resolved.kind.unwrap_or_default(), resolved.entry_id)
        };
        let expected_seed = current_attrs.inode_seed.clone();
        let current_attrs = current_attrs.file_attr;

        if let Some(new_size) = size {
            if ino == ROOT_INO || kind == "dir" {
                fuse_reply_error!(reply, EISDIR);
                return;
            }
            let file_id = match kind.as_str() {
                "file" => entry_id,
                "hardlink" => match entry_id
                    .map(|id| authority.get_hardlink_file_id(id))
                    .transpose()
                {
                    Ok(Some(Some(id))) => Some(id),
                    Ok(_) => None,
                    Err(_) => {
                        fuse_reply_error!(reply, EIO);
                        return;
                    }
                },
                _ => None,
            };
            let Some(file_id) = file_id else {
                fuse_reply_error!(reply, ENOENT);
                return;
            };
            if let Some(fh) = fh {
                let Some(handle_state) = self.file_handle_state_for_handle(fh) else {
                    fuse_reply_error!(reply, EBADF);
                    return;
                };
                if handle_state.file_id != Some(file_id)
                    || (handle_state.flags & libc::O_ACCMODE) == libc::O_RDONLY
                {
                    fuse_reply_error!(reply, EBADF);
                    return;
                }
                let mut state = self
                    .write_state_for_handle(fh)
                    .unwrap_or_else(|| Self::new_write_state(file_id, new_size, true));
                state.file_id = file_id;
                state.file_size = new_size;
                state.truncate_pending = true;
                if let Err(errno) = self.attach_write_persistence_fence_for_handle(fh, &mut state) {
                    fuse_reply_error!(reply, errno);
                    return;
                }
                if let Err(errno) = self.flush_write_state(&mut state) {
                    fuse_reply_error!(reply, errno);
                    return;
                }
                if Self::write_state_has_pending_changes(&state) {
                    self.update_write_state(fh, state);
                } else {
                    self.remove_write_state(fh);
                }
            } else {
                match self.acl_allows(&path, &current_attrs, libc::W_OK, &subject) {
                    Ok(true) => {}
                    Ok(false) => {
                        fuse_reply_error!(reply, EACCES);
                        return;
                    }
                    Err(errno) => {
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                }
                // Path-based truncate (setattr size bez fh) musi wejsc w ten sam
                // first-writer-wins co zapis przez otwarty uchwyt. Wczesniej
                // flush_write_state() byl wywolywany bez write ownership i bez
                // persistence fence, przez co drugi mount mogl zmienic rozmiar
                // pliku nalezacego do aktywnego writera.
                let ownership_fh = match self.create_writable_handle_with_ownership(
                    path.clone(),
                    file_id,
                    libc::O_WRONLY,
                ) {
                    Ok(fh) => fh,
                    Err(errno) => {
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                };

                let mut state = Self::new_write_state(file_id, new_size, true);
                state.write_authorization = Some(FileWriteAuthorization {
                    actor_uid: subject.uid,
                    actor_groups: subject.groups.iter().copied().collect(),
                    expected_inode_seed: expected_seed.clone(),
                    use_posix_acl: self.acl_enabled,
                });

                // Tymczasowy writable handle przenosi fencing token do WriteState.
                // Bez tego sam lease blokowalby konkurenta przy acquire, ale stale
                // writer nadal nie bylby chroniony podczas persist/flush.
                let persist_result = self
                    .attach_write_persistence_fence_for_handle(ownership_fh, &mut state)
                    .and_then(|_| self.flush_write_state(&mut state));

                // Ten fh istnieje tylko na czas path-based truncate. Musimy go
                // zawsze posprzatac, rowniez po bledzie persist.
                let release_result = self.release_write_ownership_for_handle(ownership_fh);
                self.remove_handle_state(ownership_fh);

                if let Err(errno) = persist_result {
                    if let Err(release_errno) = release_result {
                        warn!(
                            "FOD path truncate ownership release failed path={} file_id={} fh={} persist_errno={} release_errno={}",
                            path, file_id, ownership_fh, errno, release_errno
                        );
                    }
                    fuse_reply_error!(reply, errno);
                    return;
                }

                if let Err(errno) = release_result {
                    warn!(
                        "FOD path truncate ownership release failed path={} file_id={} fh={} errno={}",
                        path, file_id, ownership_fh, errno
                    );
                    fuse_reply_error!(reply, errno);
                    return;
                }
            }
        }

        if mode.is_some() || uid.is_some() || gid.is_some() {
            let permission_owner = if kind == "hardlink" {
                match entry_id
                    .map(|id| authority.get_hardlink_file_id(id))
                    .transpose()
                {
                    Ok(Some(Some(id))) => Some(("file", id)),
                    Ok(_) => None,
                    Err(_) => {
                        fuse_reply_error!(reply, EIO);
                        return;
                    }
                }
            } else {
                entry_id.map(|id| (kind.as_str(), id))
            };
            let Some((owner_kind, owner_id)) = permission_owner else {
                fuse_reply_error!(reply, ENOENT);
                return;
            };
            let groups: Vec<u32> = subject.groups.iter().copied().collect();
            let change = InodePermissionChange {
                actor_uid: subject.uid,
                actor_groups: &groups,
                use_posix_acl: self.acl_enabled,
                mode,
                uid,
                gid,
            };
            match authority.change_inode_permissions(owner_kind, owner_id, &expected_seed, &change)
            {
                Ok(InodePermissionOutcome::Changed) => {}
                Ok(InodePermissionOutcome::Denied) => {
                    fuse_reply_error!(reply, EPERM);
                    return;
                }
                Ok(InodePermissionOutcome::Missing) => {
                    fuse_reply_error!(reply, ENOENT);
                    return;
                }
                Err(_) => {
                    fuse_reply_error!(reply, EIO);
                    return;
                }
            }
        }

        if atime.is_some() || mtime.is_some() {
            let atime_text = Self::time_or_now_to_db_string(atime);
            let mtime_text = Self::time_or_now_to_db_string(mtime);
            let atime_value = match atime {
                Some(TimeOrNow::Now) => Some(InodeTimeValue::Now),
                Some(TimeOrNow::SpecificTime(_)) => {
                    atime_text.as_deref().map(InodeTimeValue::Specific)
                }
                None => None,
            };
            let mtime_value = match mtime {
                Some(TimeOrNow::Now) => Some(InodeTimeValue::Now),
                Some(TimeOrNow::SpecificTime(_)) => {
                    mtime_text.as_deref().map(InodeTimeValue::Specific)
                }
                None => None,
            };
            let (owner_kind, owner_id) = match kind.as_str() {
                "file" => match entry_id {
                    Some(id) => ("file", id),
                    None => {
                        fuse_reply_error!(reply, ENOENT);
                        return;
                    }
                },
                "hardlink" => match entry_id
                    .map(|id| authority.get_hardlink_file_id(id))
                    .transpose()
                {
                    Ok(Some(Some(id))) => ("file", id),
                    Ok(_) => {
                        fuse_reply_error!(reply, ENOENT);
                        return;
                    }
                    Err(_) => {
                        fuse_reply_error!(reply, EIO);
                        return;
                    }
                },
                "dir" => match entry_id {
                    Some(id) => ("dir", id),
                    None => {
                        fuse_reply_error!(reply, ENOENT);
                        return;
                    }
                },
                "symlink" => match entry_id {
                    Some(id) => ("symlink", id),
                    None => {
                        fuse_reply_error!(reply, ENOENT);
                        return;
                    }
                },
                _ => {
                    fuse_reply_error!(reply, EIO);
                    return;
                }
            };
            let groups: Vec<u32> = subject.groups.iter().copied().collect();
            let change = InodeTimesChange {
                actor_uid: subject.uid,
                actor_groups: &groups,
                use_posix_acl: self.acl_enabled,
                atime: atime_value,
                mtime: mtime_value,
            };
            match authority.change_inode_times(owner_kind, owner_id, &expected_seed, &change) {
                Ok(InodeTimesOutcome::Changed) => {}
                Ok(InodeTimesOutcome::Denied) => {
                    fuse_reply_error!(reply, EPERM);
                    return;
                }
                Ok(InodeTimesOutcome::WriteDenied) => {
                    fuse_reply_error!(reply, EACCES);
                    return;
                }
                Ok(InodeTimesOutcome::Missing) => {
                    fuse_reply_error!(reply, ENOENT);
                    return;
                }
                Err(_) => {
                    fuse_reply_error!(reply, EIO);
                    return;
                }
            }
        }

        match if let Some(file_id) = handle_file_id {
            self.attrs_for_open_file_id(file_id)
        } else if ino == ROOT_INO {
            self.root_attr().map(Some)
        } else {
            self.attrs_for_path_from_repo(&authority, &path)
        } {
            Ok(Some(attrs)) if attrs.file_attr.ino.0 == ino => {
                reply.attr(&self.attr_cache_ttl_live(), &attrs.file_attr)
            }
            Ok(Some(_)) => fuse_reply_error!(reply, ENOENT),
            Ok(None) => fuse_reply_error!(reply, ENOENT),
            Err(errno) => fuse_reply_error!(reply, errno),
        }
    }

    fn mkdir(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        let parent = parent.0;
        let req_id = self.next_request_id();
        let subject = self.request_identity(req);
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        if let Err(errno) = Self::validate_component_name(name) {
            fuse_reply_error!(reply, errno);
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
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let child_path = Self::join_path(&parent_path, name);
        let parent_id = match self.parent_entry_id_for_inode(parent) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let mut mode = mode & !umask;
        if !subject.is_root() {
            mode &= !(libc::S_ISUID | libc::S_ISGID);
        }
        self.log_request_start(
            req_id,
            "mkdir",
            format!(
                "parent={} child={} mode={:#o} uid={} gid={}",
                parent_path, child_path, mode, subject.uid, subject.gid
            ),
        );
        let target_name = name.to_string_lossy();
        let supplementary_groups = subject.groups.iter().copied().collect::<Vec<_>>();
        // A path-derived seed would give a remove/recreate at the same name
        // the same FUSE inode. Pair the mount's PostgreSQL-assigned session ID
        // with its local monotonic counter to preserve object generations.
        let inode_seed = match self.new_inode_seed("directory") {
            Ok(seed) => seed,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let create_result = if self.acl_enabled {
            self.repo.create_directory_with_default_acl(
                parent_id,
                target_name.as_ref(),
                mode,
                subject.uid,
                subject.gid,
                &inode_seed,
                &supplementary_groups,
            )
        } else {
            self.repo.create_directory_for_subject(
                parent_id,
                target_name.as_ref(),
                mode,
                subject.uid,
                subject.gid,
                &inode_seed,
                &supplementary_groups,
            )
        };
        match create_result {
            Ok(directory_id) => {
                let _ = self.append_journal_event(
                    subject.uid,
                    "mkdir",
                    &child_path,
                    None,
                    Some(directory_id),
                );
                self.invalidate_statfs_cache();
                match self.lookup_path_from_authority(&child_path) {
                    Ok(Some(attrs)) => {
                        self.register_lookup_path(&child_path, attrs.file_attr.ino.0);
                        debug!(
                            "FOD req={} op=mkdir created path={} directory_id={}",
                            req_id, child_path, directory_id
                        );
                        reply.entry_with_ttls(
                            &self.attr_cache_ttl_live(),
                            &self.entry_cache_ttl_live(),
                            &attrs.file_attr,
                            Generation(0),
                        );
                    }
                    Ok(None) => fuse_reply_error!(reply, EIO),
                    Err(errno) => fuse_reply_error!(reply, errno),
                }
            }
            Err(err) => fuse_reply_error!(reply, crate::errno::namespace_mutation_errno(&err)),
        }
    }

    fn unlink(&self, req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let parent = parent.0;
        let req_id = self.next_request_id();
        let subject = self.request_identity(req);
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        if let Err(errno) = Self::validate_component_name(name) {
            fuse_reply_error!(reply, errno);
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
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let expected_parent_id = match self.parent_entry_id_for_inode(parent) {
            Ok(parent_id) => parent_id,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let child_path = Self::join_path(&parent_path, name);
        self.log_request_start(
            req_id,
            "unlink",
            format!("parent={} child={}", parent_path, child_path),
        );
        let authority = self.repo.authoritative_view();
        let resolved = match authority.resolve_path(&child_path).map_err(|_| EIO) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let kind = resolved.kind;
        let entry_id = resolved.entry_id;
        let parent_id = resolved.parent_id;
        if parent_id != expected_parent_id {
            fuse_reply_error!(reply, ENOENT);
            return;
        }
        if parent_path != "/" && parent_id.is_none() {
            fuse_reply_error!(reply, ENOENT);
            return;
        }
        let result = match kind.as_deref() {
            Some("file") | Some("hardlink") => {
                let file_id_result = authority.get_file_id(&child_path).map_err(|_| EIO);
                let file_id = match file_id_result {
                    Ok(Some(value)) => value,
                    Ok(None) => {
                        self.log_request_error(
                            req_id,
                            "unlink",
                            ENOENT,
                            format!("child={} missing file id", child_path),
                        );
                        fuse_reply_error!(reply, ENOENT);
                        return;
                    }
                    Err(errno) => {
                        self.log_request_error(
                            req_id,
                            "unlink",
                            errno,
                            format!("child={}", child_path),
                        );
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                };
                self.unlink_file_with_primary_authority(
                    file_id,
                    parent_id,
                    &parent_path,
                    &child_path,
                    name.to_string_lossy().as_ref(),
                    req_id,
                    &subject,
                )
            }
            Some("symlink") => match entry_id {
                Some(symlink_id) => self.repo.unlink_symlink_entry_for_subject_acl(
                    symlink_id,
                    parent_id,
                    name.to_string_lossy().as_ref(),
                    subject.uid,
                    subject.gid,
                    &subject.groups.iter().copied().collect::<Vec<_>>(),
                    self.acl_enabled,
                ),
                None => Err("missing symlink id".to_string()),
            },
            Some("dir") => {
                self.log_request_error(
                    req_id,
                    "unlink",
                    EISDIR,
                    format!("child={} dir", child_path),
                );
                fuse_reply_error!(reply, EISDIR);
                return;
            }
            _ => {
                self.log_request_error(
                    req_id,
                    "unlink",
                    ENOENT,
                    format!("child={} missing", child_path),
                );
                fuse_reply_error!(reply, ENOENT);
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
                self.invalidate_statfs_cache();
                debug!("FOD req={} op=unlink completed path={}", req_id, child_path);
                reply.ok();
            }
            Err(err) => {
                let errno = crate::errno::namespace_mutation_errno(&err);
                self.log_request_error(
                    req_id,
                    "unlink",
                    errno,
                    format!("child={} delete err={}", child_path, err),
                );
                fuse_reply_error!(reply, errno)
            }
        }
    }

    fn rmdir(&self, req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let parent = parent.0;
        let req_id = self.next_request_id();
        let subject = self.request_identity(req);
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        if let Err(errno) = Self::validate_component_name(name) {
            fuse_reply_error!(reply, errno);
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
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let expected_parent_id = match self.parent_entry_id_for_inode(parent) {
            Ok(parent_id) => parent_id,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let child_path = Self::join_path(&parent_path, name);
        self.log_request_start(
            req_id,
            "rmdir",
            format!("parent={} child={}", parent_path, child_path),
        );
        let authority = self.repo.authoritative_view();
        let resolved = match authority.resolve_path(&child_path) {
            Ok(value) => value,
            Err(err) => {
                warn!("FOD rmdir resolve failed path={} err={}", child_path, err);
                fuse_reply_error!(reply, EIO);
                return;
            }
        };
        if resolved.parent_id != expected_parent_id {
            fuse_reply_error!(reply, ENOENT);
            return;
        }
        let directory_id = match (resolved.kind.as_deref(), resolved.entry_id) {
            (Some("dir"), Some(id)) => id,
            (None, _) => {
                fuse_reply_error!(reply, ENOENT);
                return;
            }
            _ => {
                fuse_reply_error!(reply, ENOTDIR);
                return;
            }
        };
        match self.repo.unlink_directory_entry_for_subject_acl(
            directory_id,
            resolved.parent_id,
            name.to_string_lossy().as_ref(),
            subject.uid,
            subject.gid,
            &subject.groups.iter().copied().collect::<Vec<_>>(),
            self.acl_enabled,
        ) {
            Ok(DirectoryUnlinkOutcome::Removed) => {
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
            Ok(DirectoryUnlinkOutcome::NotEmpty) => fuse_reply_error!(reply, ENOTEMPTY),
            Ok(DirectoryUnlinkOutcome::SourceChanged) => fuse_reply_error!(reply, ENOENT),
            Err(err) => {
                warn!(
                    "FOD rmdir failed path={} directory_id={} err={}",
                    child_path, directory_id, err
                );
                fuse_reply_error!(reply, crate::errno::namespace_mutation_errno(&err));
            }
        }
    }

    fn rename(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        let parent = parent.0;
        let newparent = newparent.0;
        let req_id = self.next_request_id();
        let subject = self.request_identity(req);
        if flags.bits() & !RenameFlags::RENAME_NOREPLACE.bits() != 0 {
            fuse_reply_error!(reply, EINVAL);
            return;
        }
        let no_replace = flags.contains(RenameFlags::RENAME_NOREPLACE);
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        if let Err(errno) = Self::validate_component_name(name) {
            fuse_reply_error!(reply, errno);
            return;
        }
        if let Err(errno) = Self::validate_component_name(newname) {
            fuse_reply_error!(reply, errno);
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
                fuse_reply_error!(reply, errno);
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
                fuse_reply_error!(reply, errno);
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
        let authority = self.repo.authoritative_view();
        let resolve = |path: &str| {
            authority
                .resolve_path(path)
                .map(|value| (value.kind, value.entry_id))
                .map_err(|_| EIO)
        };
        let (kind, entry_id) = match resolve(&old_path) {
            Ok(value) => value,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "rename",
                    errno,
                    format!("old_path={} resolve", old_path),
                );
                fuse_reply_error!(reply, errno);
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
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let old_name = name.to_string_lossy().to_string();
        let new_name = newname.to_string_lossy().to_string();
        let source_attrs = self
            .attrs_for_path_from_repo(&authority, &old_path)
            .ok()
            .flatten();
        let old_ino = source_attrs
            .as_ref()
            .map(|attrs| attrs.file_attr.ino.0)
            .unwrap_or(ROOT_INO);
        if old_path != new_path {
            let existing = match resolve(&new_path) {
                Ok(value) => value,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "rename",
                        errno,
                        format!("new_path={} resolve", new_path),
                    );
                    fuse_reply_error!(reply, errno);
                    return;
                }
            };

            let source_is_directory = matches!(kind.as_deref(), Some("dir"));
            let target_is_directory = matches!(existing.0.as_deref(), Some("dir"));

            if !no_replace || existing.0.is_none() {
                if source_is_directory {
                    let old_prefix = format!("{}/", old_path.trim_end_matches('/'));
                    if new_parent_path == old_path || new_parent_path.starts_with(&old_prefix) {
                        self.log_request_error(
                            req_id,
                            "rename",
                            EINVAL,
                            format!(
                                "old_path={} new_parent_path={} directory cycle",
                                old_path, new_parent_path
                            ),
                        );
                        fuse_reply_error!(reply, EINVAL);
                        return;
                    }
                    if existing.0.is_some() && !target_is_directory {
                        self.log_request_error(
                            req_id,
                            "rename",
                            ENOTDIR,
                            format!("new_path={} non-directory target", new_path),
                        );
                        fuse_reply_error!(reply, ENOTDIR);
                        return;
                    }
                } else if target_is_directory {
                    self.log_request_error(
                        req_id,
                        "rename",
                        EISDIR,
                        format!("new_path={} directory target", new_path),
                    );
                    fuse_reply_error!(reply, EISDIR);
                    return;
                }
            }

            if matches!(
                kind.as_deref(),
                Some("file" | "hardlink" | "symlink" | "dir")
            ) && matches!(
                existing.0.as_deref(),
                None | Some("file" | "hardlink" | "symlink" | "dir")
            ) {
                let old_parent_id = match self.parent_entry_id_for_inode(parent) {
                    Ok(value) => value,
                    Err(errno) => {
                        self.log_request_error(
                            req_id,
                            "rename",
                            errno,
                            format!("parent={} parent_id", parent),
                        );
                        fuse_reply_error!(reply, errno);
                        return;
                    }
                };

                let source = match (kind.as_deref(), entry_id) {
                    (Some("file"), Some(file_id)) => RenameNamespaceSource::File { file_id },
                    (Some("hardlink"), Some(hardlink_id)) => {
                        let file_id = match authority.get_hardlink_file_id(hardlink_id) {
                            Ok(Some(file_id)) => file_id,
                            Ok(None) => {
                                self.log_request_error(
                                    req_id,
                                    "rename",
                                    EIO,
                                    format!(
                                        "old_path={} hardlink_id={} missing backing file",
                                        old_path, hardlink_id
                                    ),
                                );
                                fuse_reply_error!(reply, EIO);
                                return;
                            }
                            Err(err) => {
                                warn!(
                                    "FOD req={} op=rename hardlink source lookup failed old_path={} hardlink_id={} err={}",
                                    req_id, old_path, hardlink_id, err
                                );
                                fuse_reply_error!(reply, EIO);
                                return;
                            }
                        };
                        RenameNamespaceSource::Hardlink {
                            hardlink_id,
                            file_id,
                        }
                    }
                    (Some("symlink"), Some(symlink_id)) => {
                        RenameNamespaceSource::Symlink { symlink_id }
                    }
                    (Some("dir"), Some(directory_id)) => {
                        RenameNamespaceSource::Directory { directory_id }
                    }
                    _ => {
                        self.log_request_error(
                            req_id,
                            "rename",
                            EIO,
                            format!("old_path={} missing namespace source id", old_path),
                        );
                        fuse_reply_error!(reply, EIO);
                        return;
                    }
                };

                let expected_target = match (existing.0.as_deref(), existing.1) {
                    (None, _) => None,
                    (Some("file"), Some(file_id)) => Some(RenameNamespaceTarget::File { file_id }),
                    (Some("hardlink"), Some(hardlink_id)) => {
                        let file_id = match authority.get_hardlink_file_id(hardlink_id) {
                            Ok(Some(file_id)) => file_id,
                            Ok(None) => {
                                self.log_request_error(
                                    req_id,
                                    "rename",
                                    EIO,
                                    format!(
                                        "new_path={} hardlink_id={} missing backing file",
                                        new_path, hardlink_id
                                    ),
                                );
                                fuse_reply_error!(reply, EIO);
                                return;
                            }
                            Err(err) => {
                                warn!(
                                    "FOD req={} op=rename hardlink target lookup failed new_path={} hardlink_id={} err={}",
                                    req_id, new_path, hardlink_id, err
                                );
                                fuse_reply_error!(reply, EIO);
                                return;
                            }
                        };
                        Some(RenameNamespaceTarget::Hardlink {
                            hardlink_id,
                            file_id,
                        })
                    }
                    (Some("symlink"), Some(symlink_id)) => {
                        Some(RenameNamespaceTarget::Symlink { symlink_id })
                    }
                    (Some("dir"), Some(directory_id)) => {
                        Some(RenameNamespaceTarget::Directory { directory_id })
                    }
                    _ => {
                        self.log_request_error(
                            req_id,
                            "rename",
                            EIO,
                            format!("new_path={} invalid protected namespace target", new_path),
                        );
                        fuse_reply_error!(reply, EIO);
                        return;
                    }
                };

                #[cfg(feature = "integration-test-hooks")]
                if let Err(errno) = Self::test_rename_before_ownership_barrier(&new_path) {
                    self.log_request_error(
                        req_id,
                        "rename",
                        errno,
                        format!("new_path={} test barrier", new_path),
                    );
                    fuse_reply_error!(reply, errno);
                    return;
                }

                let rename_result = if no_replace {
                    self.repo
                        .rename_namespace_noreplace_with_ownership_guard_for_subject_acl(
                            source,
                            old_parent_id,
                            &old_name,
                            new_parent_id,
                            &new_name,
                            subject.uid,
                            subject.gid,
                            &subject.groups.iter().copied().collect::<Vec<_>>(),
                            self.acl_enabled,
                        )
                } else {
                    self.repo.rename_namespace_replace_with_ownership_guard_for_subject_acl(
                        source,
                        old_parent_id,
                        &old_name,
                        new_parent_id,
                        &new_name,
                        expected_target,
                        subject.uid,
                        subject.gid,
                        &subject.groups.iter().copied().collect::<Vec<_>>(),
                        self.acl_enabled,
                    )
                };
                match rename_result {
                    Ok(RenameFileReplaceOutcome::Applied) => {
                        if expected_target.is_some() {
                            self.remove_cached_path(&new_path);
                            self.remove_cached_handle_paths(&new_path);
                        }
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
                            "FOD req={} op=rename completed protected_namespace_replace old_path={} new_path={}",
                            req_id, old_path, new_path
                        );
                        reply.ok();
                        return;
                    }
                    Ok(RenameFileReplaceOutcome::NoOpSameFile) => {
                        debug!(
                            "FOD req={} op=rename completed no-op same backing file old_path={} new_path={}",
                            req_id, old_path, new_path
                        );
                        reply.ok();
                        return;
                    }
                    Ok(RenameFileReplaceOutcome::Busy) => {
                        self.log_request_error(
                            req_id,
                            "rename",
                            EBUSY,
                            format!(
                                "old_path={} new_path={} ownership conflict",
                                old_path, new_path
                            ),
                        );
                        fuse_reply_error!(reply, EBUSY);
                        return;
                    }
                    Ok(RenameFileReplaceOutcome::SourceMissing) => {
                        self.log_request_error(
                            req_id,
                            "rename",
                            ENOENT,
                            format!("old_path={} source changed", old_path),
                        );
                        fuse_reply_error!(reply, ENOENT);
                        return;
                    }
                    Ok(RenameFileReplaceOutcome::TargetExists) => {
                        self.log_request_error(
                            req_id,
                            "rename",
                            EEXIST,
                            format!("new_path={} no-replace target exists", new_path),
                        );
                        fuse_reply_error!(reply, EEXIST);
                        return;
                    }
                    Ok(RenameFileReplaceOutcome::InvalidDirectoryCycle) => {
                        fuse_reply_error!(reply, EINVAL);
                        return;
                    }
                    Ok(RenameFileReplaceOutcome::TargetNotEmpty) => {
                        self.log_request_error(
                            req_id,
                            "rename",
                            ENOTEMPTY,
                            format!("new_path={} target directory not empty", new_path),
                        );
                        fuse_reply_error!(reply, ENOTEMPTY);
                        return;
                    }
                    Err(err) => {
                        warn!(
                            "FOD req={} op=rename protected namespace replace failed old_path={} new_path={} err={}",
                            req_id, old_path, new_path, err
                        );
                        fuse_reply_error!(reply, crate::errno::namespace_mutation_errno(&err));
                        return;
                    }
                }
            }

            self.log_request_error(
                req_id,
                "rename",
                EIO,
                format!(
                    "old_path={} new_path={} unsupported namespace combination source_kind={:?} target_kind={:?}",
                    old_path,
                    new_path,
                    kind.as_deref(),
                    existing.0.as_deref()
                ),
            );
            fuse_reply_error!(reply, EIO);
            return;
        }

        if no_replace {
            fuse_reply_error!(reply, EEXIST);
            return;
        }

        debug!(
            "FOD req={} op=rename completed no-op same path old_path={} new_path={}",
            req_id, old_path, new_path
        );
        reply.ok();
    }

    fn create(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let parent = parent.0;
        let req_id = self.next_request_id();
        let subject = self.request_identity(req);
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        if let Err(errno) = Self::validate_component_name(name) {
            fuse_reply_error!(reply, errno);
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
                fuse_reply_error!(reply, errno);
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
        let parent_id = match self.parent_entry_id_for_inode(parent) {
            Ok(value) => value,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "create",
                    errno,
                    format!("path={} phase=resolve_parent_inode", child_path),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        // FUSE CREATE passes the full file mode, including S_IFREG. Store only
        // POSIX permission and special bits in the database mode column.
        let mut mode = (mode & 0o7777) & !umask;
        if !subject.is_root() {
            mode &= !(libc::S_ISUID | libc::S_ISGID);
        }
        let writable = matches!(flags & libc::O_ACCMODE, libc::O_WRONLY | libc::O_RDWR);
        let supplementary_groups = subject.groups.iter().copied().collect::<Vec<_>>();

        let (file_id, fh, created_new) = if writable {
            match self.create_writable_file_with_destination_ownership(
                child_path.clone(),
                parent_id,
                name.to_string_lossy().as_ref(),
                mode,
                subject.uid,
                subject.gid,
                &supplementary_groups,
                flags,
            ) {
                Ok(value) => value,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "create",
                        errno,
                        format!("path={} phase=writable_create_setup", child_path),
                    );
                    self.remove_cached_path(&child_path);
                    self.invalidate_statfs_cache();
                    fuse_reply_error!(reply, errno);
                    return;
                }
            }
        } else {
            let inode_seed = match self.new_inode_seed("file") {
                Ok(seed) => seed,
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            };
            let file_id = match self.create_file_for_subject(
                parent_id,
                name.to_string_lossy().as_ref(),
                mode,
                subject.uid,
                subject.gid,
                &inode_seed,
                &supplementary_groups,
            ) {
                Ok(file_id) => file_id,
                Err(err) => {
                    fuse_reply_error!(reply, crate::errno::namespace_mutation_errno(&err));
                    return;
                }
            };
            let fh = match self.create_handle_for_file(child_path.clone(), Some(file_id), flags) {
                Ok(fh) => fh,
                Err(errno) => {
                    let _ = self.remove_primary_file_or_promote_hardlink(file_id);
                    fuse_reply_error!(reply, errno);
                    return;
                }
            };
            (file_id, fh, true)
        };

        // create() can recover an entry that appeared after the initial negative
        // lookup but before destination ownership was acquired. In that case the
        // kernel still expects O_TRUNC semantics exactly like open(O_TRUNC).
        if writable && !created_new && (flags & libc::O_TRUNC) != 0 {
            let mut state = Self::new_write_state(file_id, 0, true);
            if let Err(errno) = self.attach_write_persistence_fence_for_handle(fh, &mut state) {
                self.rollback_open_handle(fh);
                self.log_request_error(
                    req_id,
                    "create",
                    errno,
                    format!(
                        "path={} file_id={} recovered O_TRUNC fence",
                        child_path, file_id
                    ),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
            if let Err(errno) = self.flush_write_state(&mut state) {
                self.rollback_open_handle(fh);
                self.log_request_error(
                    req_id,
                    "create",
                    errno,
                    format!(
                        "path={} file_id={} recovered O_TRUNC flush",
                        child_path, file_id
                    ),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
            self.remove_write_state(fh);
            debug!(
                "FOD req={} op=create recovered_existing_atomic_truncate path={} file_id={} fh={}",
                req_id, child_path, file_id, fh
            );
        }

        match self.lookup_path_from_authority(&child_path) {
            Ok(Some(attrs)) => {
                self.register_lookup_path(&child_path, attrs.file_attr.ino.0);
                if created_new {
                    let _ = self.append_journal_event(
                        subject.uid,
                        "create",
                        &child_path,
                        Some(file_id),
                        None,
                    );
                }
                self.invalidate_statfs_cache();
                debug!(
                    "FOD req={} op=create completed path={} file_id={} fh={} created_new={}",
                    req_id, child_path, file_id, fh, created_new
                );
                reply.created_with_ttls(
                    &self.attr_cache_ttl_live(),
                    &self.entry_cache_ttl_live(),
                    &attrs.file_attr,
                    Generation(0),
                    FileHandle(fh),
                    self.fopen_flags(),
                );
            }
            Ok(None) => {
                self.log_request_error(
                    req_id,
                    "create",
                    EIO,
                    format!("path={} phase=post_create_authoritative_lookup_missing", child_path),
                );
                // Kernel nie dostanie fh po bledzie create(), wiec musimy sami
                // zwolnic ownership i usunac lokalny stan uchwytu.
                if writable {
                    if let Err(release_errno) = self.release_write_ownership_for_handle(fh) {
                        warn!(
                            "FOD create cleanup ownership release failed path={} file_id={} fh={} errno={}",
                            child_path, file_id, fh, release_errno
                        );
                    }
                }
                self.remove_handle_state(fh);

                if created_new {
                    if let Err(err) = self.remove_primary_file_or_promote_hardlink(file_id) {
                        warn!(
                            "FOD create rollback after lookup miss failed path={} file_id={} err={}",
                            child_path, file_id, err
                        );
                    }
                }
                self.remove_cached_path(&child_path);
                self.invalidate_statfs_cache();
                fuse_reply_error!(reply, EIO);
            }
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "create",
                    errno,
                    format!("path={} phase=post_create_authoritative_lookup", child_path),
                );
                // Jak wyzej: create() zwraca blad, wiec nie mozemy pozostawic
                // niewidocznego dla kernela fh z aktywnym write ownership.
                if writable {
                    if let Err(release_errno) = self.release_write_ownership_for_handle(fh) {
                        warn!(
                            "FOD create cleanup ownership release failed path={} file_id={} fh={} errno={}",
                            child_path, file_id, fh, release_errno
                        );
                    }
                }
                self.remove_handle_state(fh);

                let rollback_failed = created_new
                    && self
                        .remove_primary_file_or_promote_hardlink(file_id)
                        .is_err();
                self.remove_cached_path(&child_path);
                self.invalidate_statfs_cache();

                if rollback_failed {
                    warn!(
                        "FOD create rollback after lookup error failed path={} file_id={} errno={}",
                        child_path, file_id, errno
                    );
                    fuse_reply_error!(reply, EIO);
                } else {
                    fuse_reply_error!(reply, errno);
                }
            }
        }
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let ino = ino.0;
        let fh = fh.0;
        let req_id = self.next_request_id();
        let _write_profile = self.start_fuse_write_profile();
        let admission_started = Instant::now();
        let (_write_permit, write_task) = self.logical_write_gate.observe_task(
            &self.logical_write_tasks,
            data.len() as u64,
            false,
        );
        self.record_write_admission_elapsed(admission_started.elapsed());
        let operation_lock = self.handle_operation_lock(fh);
        let _operation_guard = operation_lock
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let preflight_started = Instant::now();
        if self.read_only {
            self.record_write_preflight_elapsed(preflight_started.elapsed());
            fuse_reply_error!(reply, EROFS);
            return;
        }
        let handle = match self.file_handle_state_for_handle(fh) {
            Some(handle) => handle,
            None => {
                self.record_write_preflight_elapsed(preflight_started.elapsed());
                fuse_reply_error!(reply, EBADF);
                return;
            }
        };
        if handle.flags & libc::O_ACCMODE == libc::O_RDONLY {
            self.record_write_preflight_elapsed(preflight_started.elapsed());
            fuse_reply_error!(reply, EBADF);
            return;
        }
        let handle_lookup_started = Instant::now();
        let file_id_result = self.file_id_for_handle_or_errno(fh, ino);
        self.record_write_handle_lookup_elapsed(handle_lookup_started.elapsed());
        let file_id = match file_id_result {
            Ok(value) => value,
            Err(errno) => {
                self.record_write_preflight_elapsed(preflight_started.elapsed());
                self.log_request_error(
                    req_id,
                    "write",
                    errno,
                    format!("ino={} fh={} offset={} len={}", ino, fh, offset, data.len()),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let start_log_started = Instant::now();
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
        self.record_write_start_log_elapsed(start_log_started.elapsed());
        if data.is_empty() {
            self.record_write_preflight_elapsed(preflight_started.elapsed());
            let complete_started = Instant::now();
            write_task.complete(0, 0);
            self.record_write_task_complete_elapsed(complete_started.elapsed());
            self.reply_written_profiled(reply, 0);
            return;
        }
        self.record_write_preflight_elapsed(preflight_started.elapsed());

        let drain_started = Instant::now();
        let sibling_states_result = self.drain_pending_write_states_for_file_except(file_id, fh);
        self.record_write_drain_sibling_states_elapsed(drain_started.elapsed());
        let sibling_states = match sibling_states_result {
            Ok(states) => states,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "write",
                    errno,
                    format!("file_id={} take pending sibling fh states", file_id),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let take_state_started = Instant::now();
        let existing_state = self.take_write_state_for_handle(fh);
        self.record_write_take_state_elapsed(take_state_started.elapsed());
        let existing_size = if let Some(state) = existing_state.as_ref() {
            state.file_size
        } else {
            let size_started = Instant::now();
            let size_result = self.file_size_for_update_file_id_or_errno(file_id);
            self.record_write_existing_size_lookup_elapsed(size_started.elapsed());
            match size_result {
                Ok(size) => size,
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
                        format!("ino={} fh={} file_id={} size", ino, fh, file_id),
                    );
                    fuse_reply_error!(reply, errno);
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
            let probe_started = Instant::now();
            // This comparison can suppress a real mutation. Never compare the
            // request with replica-routed bytes: a lagging replica could make
            // an old value look unchanged after another writer committed.
            let existing = self.assemble_file_slice_for_update_profiled(
                file_id,
                first_block,
                last_block,
                offset,
                end_offset,
                self.block_size,
            );
            self.record_write_unchanged_probe_elapsed(probe_started.elapsed());
            if let Ok(existing) = existing {
                if existing == data {
                    debug!(
                        "FOD req={} op=write skipped unchanged slice ino={} fh={} offset={} len={}",
                        req_id,
                        ino,
                        fh,
                        offset,
                        data.len()
                    );
                    let complete_started = Instant::now();
                    write_task.complete(0, data.len() as u64);
                    self.record_write_task_complete_elapsed(complete_started.elapsed());
                    self.reply_written_profiled(reply, data.len() as u32);
                    return;
                }
            }
        }
        let mut state =
            existing_state.unwrap_or_else(|| Self::new_write_state(file_id, existing_size, false));
        state.file_id = file_id;
        if let Err(errno) = self.attach_write_persistence_fence_for_handle(fh, &mut state) {
            self.update_write_state(fh, state);
            for (sibling_fh, sibling_state) in sibling_states {
                self.update_write_state(sibling_fh, sibling_state);
            }
            self.log_request_error(req_id, "write", errno, format!("fh={} fence", fh));
            fuse_reply_error!(reply, errno);
            return;
        }
        let merge_started = Instant::now();
        for (_sibling_fh, sibling_state) in sibling_states.iter() {
            let sibling_state = self.clone_write_state_profiled(sibling_state);
            self.merge_write_state_into(&mut state, sibling_state, self.block_size);
        }
        self.record_write_merge_sibling_states_elapsed(merge_started.elapsed());
        if let Err(errno) = self.update_write_buffer(&mut state, offset, data) {
            self.log_request_error(req_id, "write", errno, format!("fh={} update", fh));
            fuse_reply_error!(reply, errno);
            return;
        }
        state.buffered_bytes = state.buffered_bytes.saturating_add(data.len() as u64);

        let flush_decision_started = Instant::now();
        let shared_open_handles = self.open_handle_count_for_file(file_id);
        let block_size = self.block_size.max(1);
        let write_len = data.len() as u64;
        let partial_block_visibility_write = write_len > 0
            && (offset % block_size != 0 || write_len < block_size || write_len % block_size != 0);
        // Przy wielu aktywnych fh publikujemy zmiany od razu.
        // Sam czesciowy callback nie jest jednak granica logicznego write(2):
        // kernel moze technicznie podzielic jeden duzy zapis direct_io na kilka
        // niewyrownanych callbackow. Sibling state jest scalany w write() i
        // publikowany przed read() drugiego fh, wiec nie trzeba flushowac per fragment.
        let should_flush_now = self.should_flush_write_state(
            state.buffered_bytes,
            shared_open_handles,
            partial_block_visibility_write,
        );
        debug!(
            "FOD req={} op=write flush_decision buffered_bytes={} shared_open_handles={} partial_block_visibility_write={} should_flush={}",
            req_id,
            state.buffered_bytes,
            shared_open_handles,
            partial_block_visibility_write,
            should_flush_now
        );
        self.record_write_flush_decision_elapsed(flush_decision_started.elapsed());
        let mut flushed_now = false;

        if should_flush_now {
            if let Err(errno) = self.flush_write_state(&mut state) {
                let state_update_started = Instant::now();
                self.update_write_state(fh, state);
                for (sibling_fh, sibling_state) in sibling_states {
                    self.update_write_state(sibling_fh, sibling_state);
                }
                self.record_write_state_update_elapsed(state_update_started.elapsed());
                self.log_request_error(req_id, "write", errno, format!("fh={} flush", fh));
                fuse_reply_error!(reply, errno);
                return;
            }

            flushed_now = true;
        }

        let state_update_started = Instant::now();
        if has_sibling_states {
            let lock_started = Instant::now();
            let result = self.write_states.lock();
            self.record_write_state_lock_elapsed(lock_started.elapsed());
            if let Ok(mut guard) = result {
                for (sibling_fh, _) in sibling_states.iter() {
                    guard.remove(sibling_fh);
                }
            }
        }

        // Po auto-flushu nie zostawiamy pustego WriteState pod tym fh.
        // Inaczej read() moze wejsc w read_from_write_state() zamiast czytac normalnie z repo.
        if flushed_now && !Self::write_state_has_pending_changes(&state) {
            let lock_started = Instant::now();
            let result = self.write_states.lock();
            self.record_write_state_lock_elapsed(lock_started.elapsed());
            if let Ok(mut guard) = result {
                guard.remove(&fh);
            }
        } else {
            self.update_write_state(fh, state);
        }
        self.record_write_state_update_elapsed(state_update_started.elapsed());

        debug!(
            "FOD req={} op=write buffered ino={} fh={} bytes={}",
            req_id,
            ino,
            fh,
            data.len()
        );
        let complete_started = Instant::now();
        write_task.complete(0, data.len() as u64);
        self.record_write_task_complete_elapsed(complete_started.elapsed());
        self.reply_written_profiled(reply, data.len() as u32);
    }

    fn lseek(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: i64,
        whence: i32,
        reply: ReplyLseek,
    ) {
        let req_id = self.next_request_id();

        if offset < 0 {
            self.log_request_error(
                req_id,
                "lseek",
                EINVAL,
                format!("ino={} fh={} offset={} whence={}", ino, fh, offset, whence),
            );
            fuse_reply_error!(reply, EINVAL);
            return;
        }

        let seek_data = if whence == libc::SEEK_DATA {
            true
        } else if whence == libc::SEEK_HOLE {
            false
        } else {
            self.log_request_error(
                req_id,
                "lseek",
                EINVAL,
                format!(
                    "ino={} fh={} offset={} unsupported whence={}",
                    ino, fh, offset, whence
                ),
            );
            fuse_reply_error!(reply, EINVAL);
            return;
        };

        let file_id = match self.file_id_for_handle_or_errno(fh.into(), ino.into()) {
            Ok(value) => value,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "lseek",
                    errno,
                    format!("ino={} fh={} handle lookup", ino, fh),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
        };

        if let Err(errno) = self.flush_pending_write_states_for_file_except(file_id, u64::MAX) {
            self.log_request_error(
                req_id,
                "lseek",
                errno,
                format!("file_id={} flush pending write states", file_id),
            );
            fuse_reply_error!(reply, errno);
            return;
        }

        let file_size = match self.file_size_for_file_id_or_errno(file_id) {
            Ok(value) => value,
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "lseek",
                    errno,
                    format!("file_id={} load file size", file_id),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
        };

        let offset_u64 = offset as u64;
        if offset_u64 >= file_size {
            fuse_reply_error!(reply, ENXIO);
            return;
        }

        match self.repo.sparse_seek_blockwise(
            file_id,
            file_size,
            offset_u64,
            self.block_size,
            seek_data,
        ) {
            Ok(Some(result)) => match i64::try_from(result) {
                Ok(result) => reply.offset(result),
                Err(_) => fuse_reply_error!(reply, EOVERFLOW),
            },
            Ok(None) => fuse_reply_error!(reply, ENXIO),
            Err(err) => {
                self.log_request_error(
                    req_id,
                    "lseek",
                    EIO,
                    format!(
                        "file_id={} offset={} whence={} err={}",
                        file_id, offset, whence, err
                    ),
                );
                fuse_reply_error!(reply, EIO);
            }
        }
    }

    fn fallocate(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        length: u64,
        mode: i32,
        reply: ReplyEmpty,
    ) {
        let ino = ino.0;
        let fh = fh.0;
        let req_id = self.next_request_id();
        let _write_profile = self.start_fuse_write_profile();
        let supported_mode = libc::FALLOC_FL_PUNCH_HOLE | libc::FALLOC_FL_KEEP_SIZE;

        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        if mode != supported_mode {
            self.log_request_error(
                req_id,
                "fallocate",
                EOPNOTSUPP,
                format!(
                    "ino={} fh={} offset={} length={} mode={:#x} supported_mode={:#x}",
                    ino, fh, offset, length, mode, supported_mode
                ),
            );
            fuse_reply_error!(reply, EOPNOTSUPP);
            return;
        }
        if length == 0 {
            fuse_reply_error!(reply, EINVAL);
            return;
        }
        if offset.checked_add(length).is_none() {
            fuse_reply_error!(reply, EFBIG);
            return;
        }

        let handle = match self.file_handle_state_for_handle(fh) {
            Some(handle) => handle,
            None => {
                fuse_reply_error!(reply, EBADF);
                return;
            }
        };
        if (handle.flags & libc::O_ACCMODE) == libc::O_RDONLY {
            fuse_reply_error!(reply, EBADF);
            return;
        }

        let file_id = match self.file_id_for_handle_or_errno(fh, ino) {
            Ok(file_id) => file_id,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        self.log_request_start(
            req_id,
            "fallocate",
            format!(
                "ino={} fh={} file_id={} offset={} length={} mode={:#x}",
                ino, fh, file_id, offset, length, mode
            ),
        );

        if let Err(errno) = self.flush_pending_write_states_for_file_except(file_id, u64::MAX) {
            self.log_request_error(
                req_id,
                "fallocate",
                errno,
                format!("file_id={} flush pending write states", file_id),
            );
            fuse_reply_error!(reply, errno);
            return;
        }

        let Some((ownership_file_id, write_fence)) = self.write_persistence_fence_for_handle(fh)
        else {
            self.log_request_error(
                req_id,
                "fallocate",
                EIO,
                format!("file_id={} missing write persistence fence", file_id),
            );
            fuse_reply_error!(reply, EIO);
            return;
        };
        if ownership_file_id != file_id {
            self.log_request_error(
                req_id,
                "fallocate",
                EIO,
                format!(
                    "file_id={} ownership_file_id={} fence mismatch",
                    file_id, ownership_file_id
                ),
            );
            fuse_reply_error!(reply, EIO);
            return;
        }

        let live = self.reloadable_runtime();
        match self.repo.punch_hole_keep_size_with_fence(
            file_id,
            self.block_size,
            offset,
            length,
            live.copy_dedupe_crc_table,
            write_fence,
        ) {
            Ok(()) => {
                self.clear_read_cache_for_file(file_id);
                self.clear_recent_write_blocks_for_file(file_id);
                self.invalidate_read_metadata_for_file(file_id);
                self.invalidate_statfs_cache();
                self.maybe_touch_client_session_write();
                debug!(
                    "FOD req={} op=fallocate punch_hole_keep_size completed file_id={} offset={} length={}",
                    req_id, file_id, offset, length
                );
                reply.ok();
            }
            Err(err) => {
                let errno = persist_error_errno(&err);
                self.log_request_error(
                    req_id,
                    "fallocate",
                    errno,
                    format!(
                        "file_id={} offset={} length={} err={}",
                        file_id, offset, length, err
                    ),
                );
                fuse_reply_error!(reply, errno);
            }
        }
    }

    fn copy_file_range(
        &self,
        _req: &Request,
        ino_in: INodeNo,
        fh_in: FileHandle,
        offset_in: u64,
        ino_out: INodeNo,
        fh_out: FileHandle,
        offset_out: u64,
        len: u64,
        _flags: CopyFileRangeFlags,
        reply: ReplyWrite,
    ) {
        let ino_in = ino_in.0;
        let fh_in = fh_in.0;
        let ino_out = ino_out.0;
        let fh_out = fh_out.0;
        let req_id = self.next_request_id();
        let _write_profile = self.start_fuse_write_profile();
        let (_copy_permit, copy_task) =
            self.logical_write_gate
                .observe_task(&self.logical_copy_tasks, len, false);
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        let src_handle = match self.file_handle_state_for_handle(fh_in) {
            Some(handle) => handle,
            None => {
                fuse_reply_error!(reply, EBADF);
                return;
            }
        };
        let dst_handle = match self.file_handle_state_for_handle(fh_out) {
            Some(handle) => handle,
            None => {
                fuse_reply_error!(reply, EBADF);
                return;
            }
        };
        if src_handle.flags & libc::O_ACCMODE == libc::O_WRONLY
            || dst_handle.flags & libc::O_ACCMODE == libc::O_RDONLY
        {
            fuse_reply_error!(reply, EBADF);
            return;
        }
        if len == 0 {
            copy_task.complete(0, 0);
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
                fuse_reply_error!(reply, ENOENT);
                return;
            }
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "copy_file_range",
                    errno,
                    format!("src ino={} fh={}", ino_in, fh_in),
                );
                fuse_reply_error!(reply, errno);
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
                fuse_reply_error!(reply, ENOENT);
                return;
            }
            Err(errno) => {
                self.log_request_error(
                    req_id,
                    "copy_file_range",
                    errno,
                    format!("dst ino={} fh={}", ino_out, fh_out),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let src_state = self.write_state_for_handle(fh_in);
        let dst_state = self.write_state_for_handle(fh_out);
        let dst_size = if let Some(state) = dst_state.as_ref() {
            state.file_size
        } else {
            match self.file_size_for_update_file_id_or_errno(dst_file_id) {
                Ok(value) => value,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "copy_file_range",
                        errno,
                        format!("dst_file_id={} size", dst_file_id),
                    );
                    fuse_reply_error!(reply, errno);
                    return;
                }
            }
        };
        let src_size = if let Some(state) = src_state.as_ref() {
            state.file_size
        } else {
            match self.file_size_for_update_file_id_or_errno(src_file_id) {
                Ok(value) => value,
                Err(errno) => {
                    self.log_request_error(
                        req_id,
                        "copy_file_range",
                        errno,
                        format!("src_file_id={} size", src_file_id),
                    );
                    fuse_reply_error!(reply, errno);
                    return;
                }
            }
        };
        let src_offset = offset_in;
        let dst_offset = offset_out;
        let Some(bounds) =
            copy_range_bounds(self.block_size, src_offset, dst_offset, len, src_size)
        else {
            copy_task.complete(0, 0);
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
        debug!(
            "FOD req={} op=copy_file_range whole_object_decision src_file_id={} dst_file_id={} src_size={} dst_size={} src_state_present={} dst_state_present={} src_dirty={} dst_dirty={} src_offset={} dst_offset={} requested_len={} copy_len={} whole_object_adoption={}",
            req_id,
            src_file_id,
            dst_file_id,
            src_size,
            dst_size,
            src_state.is_some(),
            dst_state.is_some(),
            src_dirty,
            dst_dirty,
            offset_in,
            offset_out,
            len,
            bounds.copy_len,
            adopt_whole_object
        );
        let CopyRangeBounds {
            src_offset,
            dst_offset,
            copy_len,
            src_end_offset,
            dst_end_offset,
            src_first_block,
            src_last_block,
            dst_first_block,
            dst_last_block,
        } = bounds;
        let dedupe_enabled = self.copy_dedupe_enabled_for_len(copy_len);

        if adopt_whole_object {
            match self.adopt_source_data_object_for_handle(fh_out, src_file_id, dst_file_id) {
                Ok(true) => {
                    if let Some(state) = dst_state.as_ref() {
                        let mut state = self.clone_write_state_profiled(state);
                        state.file_size = src_size;
                        self.update_write_state(fh_out, state);
                    }
                    self.clear_read_cache_for_file(dst_file_id);
                    self.clear_recent_write_blocks_for_file(dst_file_id);
                    self.invalidate_statfs_cache();
                    debug!(
                        "FOD req={} op=copy_file_range adopted source data object src_file_id={} dst_file_id={} len={}",
                        req_id, src_file_id, dst_file_id, copy_len
                    );
                    copy_task.complete(0, copy_len);
                    self.reply_written_profiled(reply, copy_len as u32);
                    return;
                }
                Ok(false) => debug!(
                    "FOD req={} op=copy_file_range whole_object_adoption declined src_file_id={} dst_file_id={} src_size={} dst_size={}",
                    req_id, src_file_id, dst_file_id, src_size, dst_size
                ),
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
                    fuse_reply_error!(reply, EIO);
                    return;
                }
            }
        }

        let target_file_size = dst_size.max(dst_end_offset);
        let reservation_bytes = target_file_size
            .saturating_add(self.block_size.saturating_sub(1))
            .checked_div(self.block_size.max(1))
            .unwrap_or(0)
            .saturating_mul(self.block_size.max(1));
        let capacity_reservation =
            match PayloadCapacityReservation::reserve(&self.repo, reservation_bytes) {
                Ok(reservation) => reservation,
                Err(err) => {
                    let errno = persist_error_errno(&err);
                    self.log_request_error(
                        req_id,
                        "copy_file_range",
                        errno,
                        format!(
                            "dst_file_id={} reserve_bytes={} err={}",
                            dst_file_id, reservation_bytes, err
                        ),
                    );
                    fuse_reply_error!(reply, errno);
                    return;
                }
            };

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
                    fuse_reply_error!(reply, errno);
                    return;
                }
            }
        } else {
            match self.assemble_file_slice_for_update_profiled(
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
                    fuse_reply_error!(reply, EIO);
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
        if let Err(errno) = self.attach_write_persistence_fence_for_handle(fh_out, &mut state) {
            self.log_request_error(
                req_id,
                "copy_file_range",
                errno,
                format!("fh_out={} fence", fh_out),
            );
            fuse_reply_error!(reply, errno);
            return;
        }
        let current_size = state.file_size;
        let target_end = dst_offset.saturating_add(copy_len);

        if dedupe_enabled && dst_offset < current_size {
            let current = if had_dst_state {
                let mut compare_state = self.clone_write_state_profiled(&state);
                if target_end > compare_state.file_size {
                    compare_state.file_size = target_end;
                }
                self.read_copy_destination_slice(ReadCopyDestinationSlice {
                    dst_file_id,
                    state: Some(&mut compare_state),
                    dst_first_block,
                    dst_last_block,
                    dst_offset,
                    size: copy_len,
                    current_size,
                })
            } else {
                self.read_copy_destination_slice(ReadCopyDestinationSlice {
                    dst_file_id,
                    state: None,
                    dst_first_block,
                    dst_last_block,
                    dst_offset,
                    size: copy_len,
                    current_size,
                })
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
                    fuse_reply_error!(reply, EIO);
                    return;
                }
            };
            let runs = pack_copy_skip_unchanged_runs(dst_offset, self.block_size, &data, &current);
            if runs.is_empty() {
                if target_end > current_size {
                    state.file_size = target_end;
                    if let Err(errno) = self.flush_write_state_with_capacity_reservation(
                        &mut state,
                        capacity_reservation.token(),
                    ) {
                        self.log_request_error(
                            req_id,
                            "copy_file_range",
                            errno,
                            format!("fh_out={} flush", fh_out),
                        );
                        fuse_reply_error!(reply, errno);
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
                copy_task.complete(0, copy_len);
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
                    fuse_reply_error!(reply, errno);
                    return;
                }
                state.buffered_bytes = state
                    .buffered_bytes
                    .saturating_add(run_payload.len() as u64);
            }
            if let Err(errno) = self.flush_write_state_with_capacity_reservation(
                &mut state,
                capacity_reservation.token(),
            ) {
                self.log_request_error(
                    req_id,
                    "copy_file_range",
                    errno,
                    format!("fh_out={} flush", fh_out),
                );
                fuse_reply_error!(reply, errno);
                return;
            }
            self.update_write_state(fh_out, state);
            debug!(
                "FOD req={} op=copy_file_range dedupe wrote changed blocks src_file_id={} dst_file_id={} len={}",
                req_id, src_file_id, dst_file_id, copy_len
            );
            copy_task.complete(0, copy_len);
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
            fuse_reply_error!(reply, errno);
            return;
        }
        state.buffered_bytes = state.buffered_bytes.saturating_add(data.len() as u64);
        if let Err(errno) = self
            .flush_write_state_with_capacity_reservation(&mut state, capacity_reservation.token())
        {
            self.log_request_error(
                req_id,
                "copy_file_range",
                errno,
                format!("fh_out={} flush", fh_out),
            );
            fuse_reply_error!(reply, errno);
            return;
        }
        self.update_write_state(fh_out, state);
        debug!(
            "FOD req={} op=copy_file_range completed src_file_id={} dst_file_id={} len={}",
            req_id, src_file_id, dst_file_id, copy_len
        );
        copy_task.complete(0, copy_len);
        self.reply_written_profiled(reply, copy_len as u32);
    }

    fn mknod(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        rdev: u32,
        reply: ReplyEntry,
    ) {
        let parent = parent.0;
        let subject = self.request_identity(req);
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        if let Err(errno) = Self::validate_component_name(name) {
            fuse_reply_error!(reply, errno);
            return;
        }
        let file_type = mode & libc::S_IFMT;
        if file_type == libc::S_IFREG {
            let parent_path = match self.entry_path_for_ino(parent) {
                Ok(path) => path,
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            };
            let child_path = Self::join_path(&parent_path, name);
            let parent_id = match self.parent_entry_id_for_inode(parent) {
                Ok(value) => value,
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            };
            let mut mode = (mode & !umask) & !libc::S_IFMT;
            if !subject.is_root() {
                mode &= !(libc::S_ISUID | libc::S_ISGID);
            }
            let supplementary_groups = subject.groups.iter().copied().collect::<Vec<_>>();
            let inode_seed = match self.new_inode_seed("file") {
                Ok(seed) => seed,
                Err(errno) => {
                    fuse_reply_error!(reply, errno);
                    return;
                }
            };
            match self.create_file_for_subject(
                parent_id,
                name.to_string_lossy().as_ref(),
                mode,
                subject.uid,
                subject.gid,
                &inode_seed,
                &supplementary_groups,
            ) {
                Ok(_) => match self.lookup_path_from_authority(&child_path) {
                    Ok(Some(attrs)) => {
                        self.register_lookup_path(&child_path, attrs.file_attr.ino.0);
                        self.invalidate_statfs_cache();
                        reply.entry_with_ttls(
                            &self.attr_cache_ttl_live(),
                            &self.entry_cache_ttl_live(),
                            &attrs.file_attr,
                            Generation(0),
                        );
                    }
                    Ok(None) => fuse_reply_error!(reply, EIO),
                    Err(errno) => fuse_reply_error!(reply, errno),
                },
                Err(err) => fuse_reply_error!(reply, crate::errno::namespace_mutation_errno(&err)),
            }
            return;
        }
        let parent_path = match self.entry_path_for_ino(parent) {
            Ok(path) => path,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let child_path = Self::join_path(&parent_path, name);
        let parent_id = match self.parent_entry_id_for_inode(parent) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let mut mode = (mode & !umask) & !libc::S_IFMT;
        if !subject.is_root() {
            mode &= !(libc::S_ISUID | libc::S_ISGID);
        }
        let file_kind = if file_type == libc::S_IFIFO {
            "fifo"
        } else if file_type == libc::S_IFCHR {
            "char"
        } else if file_type == libc::S_IFBLK {
            "block"
        } else {
            fuse_reply_error!(reply, EINVAL);
            return;
        };
        let (rdev_major, rdev_minor) = Self::decode_fuse_rdev(rdev);
        let supplementary_groups = subject.groups.iter().copied().collect::<Vec<_>>();
        let inode_seed = match self.new_inode_seed(file_kind) {
            Ok(seed) => seed,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let create_result = if self.acl_enabled {
            self.repo.create_special_file_with_default_acl(
                parent_id,
                name.to_string_lossy().as_ref(),
                mode,
                subject.uid,
                subject.gid,
                &inode_seed,
                file_kind,
                rdev_major,
                rdev_minor,
                &supplementary_groups,
            )
        } else {
            self.repo.create_special_file_for_subject(
                parent_id,
                name.to_string_lossy().as_ref(),
                mode,
                subject.uid,
                subject.gid,
                &inode_seed,
                file_kind,
                rdev_major,
                rdev_minor,
                &supplementary_groups,
            )
        };
        match create_result {
            Ok(_) => {}
            Err(err) => {
                fuse_reply_error!(reply, crate::errno::namespace_mutation_errno(&err));
                return;
            }
        };
        match self.lookup_path_from_authority(&child_path) {
            Ok(Some(attrs)) => {
                self.register_lookup_path(&child_path, attrs.file_attr.ino.0);
                self.invalidate_statfs_cache();
                reply.entry_with_ttls(
                    &self.attr_cache_ttl_live(),
                    &self.entry_cache_ttl_live(),
                    &attrs.file_attr,
                    Generation(0),
                );
            }
            Ok(None) => fuse_reply_error!(reply, EIO),
            Err(errno) => fuse_reply_error!(reply, errno),
        }
    }

    fn symlink(
        &self,
        req: &Request,
        parent: INodeNo,
        link_name: &OsStr,
        target: &Path,
        reply: ReplyEntry,
    ) {
        let parent = parent.0;
        let subject = self.request_identity(req);
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        if let Err(errno) = Self::validate_component_name(link_name) {
            fuse_reply_error!(reply, errno);
            return;
        }
        let target = match Self::os_str_as_utf8(target.as_os_str()) {
            Ok(target) => target.to_owned(),
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let parent_path = match self.entry_path_for_ino(parent) {
            Ok(path) => path,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let child_path = Self::join_path(&parent_path, link_name);
        let parent_id = match self.parent_entry_id_for_inode(parent) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let inode_seed = match self.new_inode_seed("symlink") {
            Ok(seed) => seed,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let supplementary_groups = subject.groups.iter().copied().collect::<Vec<_>>();
        let create_result = if self.acl_enabled {
            self.repo.create_symlink_for_subject_with_acl(
                parent_id,
                link_name.to_string_lossy().as_ref(),
                &target,
                subject.uid,
                subject.gid,
                &inode_seed,
                &supplementary_groups,
            )
        } else {
            self.repo.create_symlink_for_subject(
                parent_id,
                link_name.to_string_lossy().as_ref(),
                &target,
                subject.uid,
                subject.gid,
                &inode_seed,
                &supplementary_groups,
            )
        };
        match create_result {
            Ok(_) => match self.lookup_path_from_authority(&child_path) {
                Ok(Some(attrs)) => {
                    self.register_lookup_path(&child_path, attrs.file_attr.ino.0);
                    self.invalidate_statfs_cache();
                    reply.entry_with_ttls(
                        &self.attr_cache_ttl_live(),
                        &self.entry_cache_ttl_live(),
                        &attrs.file_attr,
                        Generation(0),
                    );
                }
                Ok(None) => fuse_reply_error!(reply, EIO),
                Err(errno) => fuse_reply_error!(reply, errno),
            },
            Err(err) => fuse_reply_error!(reply, crate::errno::namespace_mutation_errno(&err)),
        }
    }

    fn link(
        &self,
        req: &Request,
        ino: INodeNo,
        newparent: INodeNo,
        newname: &OsStr,
        reply: ReplyEntry,
    ) {
        let ino = ino.0;
        let newparent = newparent.0;
        let subject = self.request_identity(req);
        if self.read_only {
            fuse_reply_error!(reply, EROFS);
            return;
        }
        if let Err(errno) = Self::validate_component_name(newname) {
            fuse_reply_error!(reply, errno);
            return;
        }
        let source_path = match self.entry_path_for_ino(ino) {
            Ok(path) => path,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let authority = self.repo.authoritative_view();
        let source_entry = match authority.resolve_path(&source_path) {
            Ok(resolved) => resolved,
            Err(_) => {
                fuse_reply_error!(reply, EIO);
                return;
            }
        };
        let source_file_id = match (source_entry.kind.as_deref(), source_entry.entry_id) {
            (Some("file"), Some(file_id)) => Some(file_id),
            (Some("hardlink"), Some(hardlink_id)) => {
                match authority.get_hardlink_file_id(hardlink_id) {
                    Ok(file_id) => file_id,
                    Err(_) => {
                        fuse_reply_error!(reply, EIO);
                        return;
                    }
                }
            }
            _ => None,
        };
        let Some(source_file_id) = source_file_id else {
            fuse_reply_error!(reply, ENOENT);
            return;
        };
        match self.attrs_for_open_file_id(source_file_id) {
            Ok(Some(attrs)) if attrs.file_attr.ino.0 == ino => {}
            Ok(_) => {
                fuse_reply_error!(reply, ENOENT);
                return;
            }
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        }
        let new_parent_path = match self.entry_path_for_ino(newparent) {
            Ok(path) => path,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let child_path = Self::join_path(&new_parent_path, newname);
        let new_parent_id = match self.parent_entry_id_for_inode(newparent) {
            Ok(value) => value,
            Err(errno) => {
                fuse_reply_error!(reply, errno);
                return;
            }
        };
        let supplementary_groups = subject.groups.iter().copied().collect::<Vec<_>>();
        let create_result = if self.acl_enabled {
            self.repo.create_hardlink_for_subject_with_acl(
                source_file_id,
                new_parent_id,
                newname.to_string_lossy().as_ref(),
                subject.uid,
                subject.gid,
                &supplementary_groups,
            )
        } else {
            self.repo.create_hardlink_for_subject(
                source_file_id,
                new_parent_id,
                newname.to_string_lossy().as_ref(),
                subject.uid,
                subject.gid,
                &supplementary_groups,
            )
        };
        match create_result {
            Ok(_) => match self.lookup_path_from_authority(&child_path) {
                Ok(Some(attrs)) => {
                    self.register_lookup_path(&child_path, attrs.file_attr.ino.0);
                    self.invalidate_statfs_cache();
                    reply.entry_with_ttls(
                        &self.attr_cache_ttl_live(),
                        &self.entry_cache_ttl_live(),
                        &attrs.file_attr,
                        Generation(0),
                    );
                }
                Ok(None) => fuse_reply_error!(reply, EIO),
                Err(errno) => fuse_reply_error!(reply, errno),
            },
            Err(err) => {
                fuse_reply_error!(reply, crate::errno::namespace_mutation_errno(&err))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        assemble_degraded_snapshot, client_session_hostname, ioctl_is_mutating, parse_file_mode,
        readdir_child_entries_to_skip, should_update_atime, AtimePolicy, FodFuse,
        SetattrMutationFlags, WriteState,
    };
    use crate::write_payload::{BlockWriteState, WritePayloadState};
    use fuser::{FileAttr, FileType, INodeNo};
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime};

    #[test]
    fn persisted_file_mode_rejects_type_bits_and_malformed_values() {
        assert_eq!(parse_file_mode("640"), Ok(0o640));
        assert_eq!(parse_file_mode("0o4755"), Ok(0o4755));
        assert_eq!(parse_file_mode("10000"), Err(crate::errno::EIO));
        assert_eq!(parse_file_mode("broken"), Err(crate::errno::EIO));
    }

    #[test]
    fn fuse_nlink_reports_unrepresentable_link_counts() {
        assert_eq!(FodFuse::fuse_nlink(0), Ok(0));
        assert_eq!(FodFuse::fuse_nlink(u32::MAX as u64), Ok(u32::MAX));
        assert_eq!(
            FodFuse::fuse_nlink(u32::MAX as u64 + 1),
            Err(crate::errno::EOVERFLOW)
        );
    }

    #[test]
    fn readdir_offsets_resume_after_the_last_cookie() {
        assert_eq!(readdir_child_entries_to_skip(0), 0);
        assert_eq!(readdir_child_entries_to_skip(1), 0);
        assert_eq!(readdir_child_entries_to_skip(2), 0);
        assert_eq!(readdir_child_entries_to_skip(3), 1);
        assert_eq!(readdir_child_entries_to_skip(4), 2);
        assert_eq!(
            readdir_child_entries_to_skip(u64::MAX),
            (u64::MAX - 2) as usize
        );
    }

    #[test]
    fn parent_access_denial_maps_to_eacces() {
        assert_eq!(
            crate::errno::namespace_mutation_errno(rust_hotpath::pg::PARENT_ACCESS_DENIED),
            crate::errno::EACCES
        );
    }

    #[test]
    fn unknown_special_file_type_fails_closed() {
        assert_eq!(FodFuse::file_type_from_special("socket"), None);
        assert_eq!(
            FodFuse::directory_entry_file_type("file", Some("socket")),
            Err(crate::errno::EIO)
        );
        assert_eq!(
            FodFuse::directory_entry_file_type("unknown", None),
            Err(crate::errno::EIO)
        );
    }

    #[test]
    fn lock_record_blob_round_trip_preserves_pid_and_reads_legacy_rows() {
        let record = super::PosixLockRecord {
            owner: 41,
            typ: libc::F_WRLCK,
            start: 7,
            end: Some(19),
            pid: 1234,
        };
        let encoded = FodFuse::lock_records_to_blob(std::slice::from_ref(&record));
        let decoded = FodFuse::lock_records_from_blob(encoded.as_bytes()).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].owner, record.owner);
        assert_eq!(decoded[0].typ, record.typ);
        assert_eq!(decoded[0].start, record.start);
        assert_eq!(decoded[0].end, record.end);
        assert_eq!(decoded[0].pid, record.pid);

        let legacy = FodFuse::lock_records_from_blob(b"41\t1\t7\t19").unwrap();
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].pid, 0);
    }

    #[test]
    fn malformed_lock_records_fail_closed() {
        for blob in [
            b"41\t3\t7\t19\t1234".as_slice(),
            b"41\t1\t9223372036854775808\t\t1234",
            b"41\t2\t7\t9223372036854775808\t1234",
            b"41\t1\t19\t7\t1234",
            b"41\t1\t7\t19\t1234\textra",
            b"41\t1\t7",
        ] {
            assert!(matches!(
                FodFuse::lock_records_from_blob(blob),
                Err(errno) if errno == crate::errno::EIO
            ));
        }
    }

    #[test]
    fn poisoned_local_lock_map_fails_closed_and_keeps_owner_heartbeat() {
        let posix_locks = Mutex::new(HashMap::from([(
            "file:7".to_string(),
            vec![super::PosixLockRecord {
                owner: 41,
                typ: libc::F_WRLCK,
                start: 0,
                end: Some(9),
                pid: 123,
            }],
        )]));
        let panic_result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = posix_locks.lock().unwrap();
            panic!("poison lock map for regression test");
        }));
        assert!(panic_result.is_err());

        assert!(matches!(
            super::local_lock_records_for_key(&posix_locks, "file:7"),
            Err(errno) if errno == crate::errno::EIO
        ));
        let (snapshot, poisoned) = super::snapshot_posix_locks(&posix_locks);
        assert!(poisoned);
        assert_eq!(snapshot[0].0, "file:7");
        assert_eq!(snapshot[0].1[0].owner, 41);

        let local_lock_owners = Mutex::new(HashSet::from([41]));
        super::refresh_local_lock_owner_state(&posix_locks, &local_lock_owners, 41);
        assert!(local_lock_owners.lock().unwrap().contains(&41));
    }

    fn file_attr(kind: FileType, atime_age_secs: u64, mtime_age_secs: u64) -> FileAttr {
        let now = SystemTime::now();
        FileAttr {
            ino: INodeNo::ROOT,
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
    fn degraded_snapshot_returns_estale_when_open_file_disappears() {
        assert_eq!(assemble_degraded_snapshot(None, 0, 4, 4), Err(crate::errno::ESTALE));
    }

    #[test]
    fn degraded_snapshot_zero_fills_holes_only_for_existing_file() {
        let snapshot = Some((8, 7, vec![(0, Arc::<[u8]>::from([1, 2, 3, 4]))]));
        assert_eq!(
            assemble_degraded_snapshot(snapshot, 4, 4, 4),
            Ok((vec![0; 4], 7))
        );
    }

    #[test]
    fn degraded_snapshot_rejects_overflowing_read_range() {
        let snapshot = Some((8, 3, Vec::new()));
        assert_eq!(
            assemble_degraded_snapshot(snapshot, u64::MAX - 1, 4, 4),
            Err(crate::errno::EINVAL)
        );
    }

    #[test]
    fn degraded_snapshot_empty_read_does_not_require_file_snapshot() {
        assert_eq!(
            assemble_degraded_snapshot(None, u64::MAX, 0, 4),
            Ok((Vec::new(), 0))
        );
    }

    #[test]
    fn degraded_snapshot_returns_eof_for_offset_at_or_after_file_size() {
        let snapshot = Some((8, 3, Vec::new()));
        assert_eq!(
            assemble_degraded_snapshot(snapshot.clone(), 8, 4, 4),
            Ok((Vec::new(), 3))
        );
        assert_eq!(
            assemble_degraded_snapshot(snapshot, 12, 4, 4),
            Ok((Vec::new(), 3))
        );
    }

    #[test]
    fn forget_ref_delta_preserves_partial_lookup_count() {
        let mut refs = HashMap::from([(11_u64, 3_u64)]);

        assert_eq!(
            super::FodFuse::apply_forget_ref_delta(&mut refs, 11, 1),
            (2, false)
        );
        assert_eq!(refs.get(&11), Some(&2));
    }

    #[test]
    fn forget_ref_delta_retires_exact_lookup_count() {
        let mut refs = HashMap::from([(12_u64, 2_u64)]);

        assert_eq!(
            super::FodFuse::apply_forget_ref_delta(&mut refs, 12, 2),
            (0, true)
        );
        assert!(!refs.contains_key(&12));
    }

    #[test]
    fn forget_ref_delta_duplicate_inode_matches_sequential_forget() {
        let mut refs = HashMap::from([(13_u64, 3_u64)]);

        let first = super::FodFuse::apply_forget_ref_delta(&mut refs, 13, 1);
        let second = super::FodFuse::apply_forget_ref_delta(&mut refs, 13, 2);
        let third = super::FodFuse::apply_forget_ref_delta(&mut refs, 13, 1);

        assert_eq!(first, (2, false));
        assert_eq!(second, (0, true));
        assert_eq!(third, (0, false));
        assert!(!refs.contains_key(&13));
    }

    #[test]
    fn forget_ref_delta_underflow_saturates_and_retires() {
        let mut refs = HashMap::from([(14_u64, 1_u64)]);

        assert_eq!(
            super::FodFuse::apply_forget_ref_delta(&mut refs, 14, 3),
            (0, true)
        );
        assert!(!refs.contains_key(&14));
    }

    #[test]
    fn client_session_hostname_prefers_env_and_falls_back_to_system() {
        assert_eq!(
            client_session_hostname(
                Some(" env-host ".to_string()),
                Ok("system-host".to_string())
            ),
            "env-host"
        );
        assert_eq!(
            client_session_hostname(Some("  ".to_string()), Ok(" system-host ".to_string())),
            "system-host"
        );
        assert_eq!(
            client_session_hostname(None, Ok("system-host".to_string())),
            "system-host"
        );
        assert_eq!(
            client_session_hostname(None, Err("hostname unavailable".to_string())),
            "unknown"
        );
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
    fn file_read_metadata_cache_policy_preserves_replica_and_bounds_primary() {
        let ttl = Duration::from_secs(1);

        assert!(super::FodFuse::should_cache_file_read_metadata(
            true,
            true,
            AtimePolicy::Default,
            libc::O_RDONLY,
            Duration::ZERO,
        ));
        assert!(super::FodFuse::should_cache_file_read_metadata(
            false,
            true,
            AtimePolicy::NoAtime,
            libc::O_RDONLY,
            ttl,
        ));
        assert!(!super::FodFuse::should_cache_file_read_metadata(
            false,
            true,
            AtimePolicy::NoAtime,
            libc::O_RDONLY,
            Duration::ZERO,
        ));
        assert!(!super::FodFuse::should_cache_file_read_metadata(
            false,
            true,
            AtimePolicy::NoAtime,
            libc::O_RDWR,
            ttl,
        ));
        assert!(!super::FodFuse::should_cache_file_read_metadata(
            false,
            true,
            AtimePolicy::Relatime,
            libc::O_RDONLY,
            ttl,
        ));
        assert!(!super::FodFuse::should_cache_file_read_metadata(
            false,
            false,
            AtimePolicy::NoAtime,
            libc::O_RDONLY,
            ttl,
        ));
    }

    #[test]
    fn acl_access_mode_uses_mask_for_group_class_and_preserves_special_bits() {
        let entries = [
            super::PosixAclEntry {
                tag: 0x0001,
                perm: 0o6,
                id: -1,
            },
            super::PosixAclEntry {
                tag: 0x0004,
                perm: 0o7,
                id: -1,
            },
            super::PosixAclEntry {
                tag: 0x0010,
                perm: 0o4,
                id: -1,
            },
            super::PosixAclEntry {
                tag: 0x0020,
                perm: 0o1,
                id: -1,
            },
        ];

        assert_eq!(
            super::FodFuse::mode_from_acl_entries(&entries, 0o4000 | 0o777).unwrap(),
            0o4000 | 0o641
        );
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
            persistence_fence: None,
            write_authorization: None,
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
                    blocks.into()
                },
            }),
            ..clean.clone()
        };

        assert!(!super::FodFuse::write_state_has_pending_changes(&clean));
        assert!(super::FodFuse::write_state_has_pending_changes(&buffered));
        assert!(super::FodFuse::write_state_has_pending_changes(&truncated));
        assert!(super::FodFuse::write_state_has_pending_changes(&blocked));
    }
    #[test]
    fn read_only_setattr_detects_every_mutating_field() {
        assert!(!SetattrMutationFlags::default().any());

        for index in 0..11 {
            let mut fields = SetattrMutationFlags::default();
            match index {
                0 => fields.mode = true,
                1 => fields.uid = true,
                2 => fields.gid = true,
                3 => fields.size = true,
                4 => fields.atime = true,
                5 => fields.mtime = true,
                6 => fields.ctime = true,
                7 => fields.crtime = true,
                8 => fields.chgtime = true,
                9 => fields.bkuptime = true,
                10 => fields.flags = true,
                _ => unreachable!(),
            }
            assert!(
                fields.any(),
                "setattr field index {index} must be treated as a mutation"
            );
        }
    }

    #[test]
    fn read_only_ioctl_classifies_mutating_commands() {
        assert!(ioctl_is_mutating(libc::FS_IOC_SETFLAGS as u32));
        assert!(ioctl_is_mutating(libc::FS_IOC32_SETFLAGS as u32));
        assert!(ioctl_is_mutating(super::IOCTL_FS_IOC_FSSETXATTR));

        assert!(!ioctl_is_mutating(super::IOCTL_FIGETBSZ));
        assert!(!ioctl_is_mutating(libc::FIONREAD as u32));
    }
}
