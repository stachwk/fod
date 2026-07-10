// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fuser::{mount2, MountOption};
use log::info;
use rust_hotpath::pg::{DbRepo, StartupSnapshot};
use std::path::{Path, PathBuf};
use std::time::Duration;

use fod_rust_runtime::{
    env_var_truthy_with_legacy_alias, env_var_with_legacy_alias, RuntimeCacheSettings,
    RuntimeConfig, RuntimeCoreSettings, RuntimeLockSettings, RuntimeMountSettings,
    RuntimeSecuritySettings, RuntimeStorageSettings,
};
pub use fod_rust_runtime::{AtimePolicy, LockBackend};

use crate::fs::FodFuse;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FuseStorageSettings {
    pub block_size: u64,
    pub write_flush_threshold_bytes: u64,
    pub max_fs_size_bytes: Option<u64>,
    pub pg_visible_path: Option<PathBuf>,
    pub copy_dedupe_enabled: bool,
    pub copy_dedupe_min_blocks: u64,
    pub copy_dedupe_max_blocks: u64,
    pub copy_dedupe_crc_table: bool,
    pub enable_extents: bool,
    pub extent_target_bytes: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FuseCacheSettings {
    pub metadata_cache_ttl: Duration,
    pub statfs_cache_ttl: Duration,
    pub read_cache_blocks: u64,
    pub read_cache_eviction_policy: String,
    pub read_ahead_blocks: u64,
    pub sequential_read_ahead_blocks: u64,
    pub small_file_read_threshold_blocks: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FuseWorkerSettings {
    pub workers_read: u64,
    pub workers_read_min_blocks: u64,
    pub workers_write: u64,
    pub workers_write_min_blocks: u64,
}

#[derive(Debug, Clone)]
pub struct FuseLockSettings {
    pub lock_backend: LockBackend,
    pub lock_lease_ttl: Duration,
    pub lock_heartbeat_interval: Duration,
    pub lock_poll_interval: Duration,
}

#[derive(Debug, Clone)]
pub struct FuseSecuritySettings {
    pub selinux_enabled: bool,
    pub acl_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct FodFuseSettings {
    pub storage: FuseStorageSettings,
    pub cache: FuseCacheSettings,
    pub workers: FuseWorkerSettings,
    pub locks: FuseLockSettings,
    pub security: FuseSecuritySettings,
    pub atime_policy: AtimePolicy,
    pub read_only: bool,
    pub use_fuse_context: bool,
    pub fopen_direct_io: bool,
}

impl FodFuseSettings {
    pub fn from_runtime(
        runtime: &RuntimeConfig,
        snapshot: &StartupSnapshot,
        requested_readonly: bool,
    ) -> Self {
        let read_only = runtime.effective_read_only(requested_readonly, snapshot.is_in_recovery);
        let mount = runtime.mount_settings(read_only);
        let security = runtime.security_settings();
        let lock = runtime.lock_settings(read_only);
        let cache = runtime.cache_settings();
        let storage = runtime.storage_settings();
        Self {
            storage: FuseStorageSettings {
                block_size: snapshot.block_size.unwrap_or(4096) as u64,
                write_flush_threshold_bytes: storage.write_flush_threshold_bytes,
                max_fs_size_bytes: storage.max_fs_size_bytes,
                pg_visible_path: storage.pg_visible_path.clone(),
                copy_dedupe_enabled: storage.copy_dedupe_enabled,
                copy_dedupe_min_blocks: storage.copy_dedupe_min_blocks,
                copy_dedupe_max_blocks: storage.copy_dedupe_max_blocks,
                copy_dedupe_crc_table: storage.copy_dedupe_crc_table,
                enable_extents: storage.enable_extents,
                extent_target_bytes: storage.extent_target_bytes,
            },
            cache: FuseCacheSettings {
                metadata_cache_ttl: cache.metadata_cache_ttl,
                statfs_cache_ttl: cache.statfs_cache_ttl,
                read_cache_blocks: cache.read_cache_blocks,
                read_cache_eviction_policy: cache.read_cache_eviction_policy.clone(),
                read_ahead_blocks: cache.read_ahead_blocks,
                sequential_read_ahead_blocks: cache.sequential_read_ahead_blocks,
                small_file_read_threshold_blocks: cache.small_file_read_threshold_blocks,
            },
            workers: FuseWorkerSettings {
                workers_read: storage.workers_read,
                workers_read_min_blocks: storage.workers_read_min_blocks,
                workers_write: storage.workers_write,
                workers_write_min_blocks: storage.workers_write_min_blocks,
            },
            locks: FuseLockSettings {
                lock_backend: lock.lock_backend,
                lock_lease_ttl: lock.lock_lease_ttl,
                lock_heartbeat_interval: lock.lock_heartbeat_interval,
                lock_poll_interval: lock.lock_poll_interval,
            },
            security: FuseSecuritySettings {
                selinux_enabled: security.selinux_enabled,
                acl_enabled: security.acl_enabled,
            },
            atime_policy: mount.atime_policy,
            read_only: mount.read_only,
            use_fuse_context: mount.use_fuse_context,
            fopen_direct_io: mount.fopen_direct_io,
        }
    }
}

fn mount_options(
    mount: &RuntimeMountSettings,
    security: &RuntimeSecuritySettings,
) -> Vec<MountOption> {
    let mut options = vec![
        MountOption::FSName("fod".to_string()),
        MountOption::AutoUnmount,
    ];
    if mount.default_permissions {
        options.push(MountOption::DefaultPermissions);
    }
    if env_var_truthy_with_legacy_alias("FOD_ALLOW_OTHER", false) {
        options.push(MountOption::AllowOther);
    }
    if mount.lazytime {
        options.push(MountOption::CUSTOM("lazytime".to_string()));
    }
    if mount.sync {
        options.push(MountOption::Sync);
    }
    if mount.dirsync {
        options.push(MountOption::DirSync);
    }
    if mount.read_only {
        options.push(MountOption::RO);
    }
    if let Some(value) = security.selinux_context.as_ref() {
        options.push(MountOption::CUSTOM(format!("context={value}")));
    }
    if let Some(value) = security.selinux_fscontext.as_ref() {
        options.push(MountOption::CUSTOM(format!("fscontext={value}")));
    }
    if let Some(value) = security.selinux_defcontext.as_ref() {
        options.push(MountOption::CUSTOM(format!("defcontext={value}")));
    }
    if let Some(value) = security.selinux_rootcontext.as_ref() {
        options.push(MountOption::CUSTOM(format!("rootcontext={value}")));
    }
    if let Some(value) = env_var_with_legacy_alias("FOD_ENTRY_TIMEOUT_SECONDS")
        .filter(|value| !value.trim().is_empty())
    {
        options.push(MountOption::CUSTOM(format!("entry_timeout={value}")));
    }
    if let Some(value) = env_var_with_legacy_alias("FOD_ATTR_TIMEOUT_SECONDS")
        .filter(|value| !value.trim().is_empty())
    {
        options.push(MountOption::CUSTOM(format!("attr_timeout={value}")));
    }
    if let Some(value) = env_var_with_legacy_alias("FOD_NEGATIVE_TIMEOUT_SECONDS")
        .filter(|value| !value.trim().is_empty())
    {
        options.push(MountOption::CUSTOM(format!("negative_timeout={value}")));
    }
    if mount.fuse_writeback_cache {
        options.push(MountOption::CUSTOM("writeback_cache".to_string()));
    }
    options
}

fn log_mount_status(
    mountpoint: &Path,
    core: &RuntimeCoreSettings,
    mount: &RuntimeMountSettings,
    security: &RuntimeSecuritySettings,
    lock: &RuntimeLockSettings,
    cache: &RuntimeCacheSettings,
    storage: &RuntimeStorageSettings,
    fs: &FodFuse,
    snapshot: &StartupSnapshot,
    options: &[MountOption],
) {
    info!("FOD mount startup status");
    info!(
        "FOD version={} FOD schema name={} FOD schema version={:?} initialized={}",
        fod_rust_runtime::FOD_VERSION_LABEL,
        fod_rust_runtime::FOD_SCHEMA_NAME,
        snapshot.schema_version,
        snapshot.schema_is_initialized
    );
    info!("FOD mountpoint={}", mountpoint.display());
    info!(
        "FOD core role={} profile={:?} force_read_only={} log_level={} use_rust_fuse={} pool_max_connections={}",
        core.role.as_str(),
        core.profile.as_ref(),
        core.force_read_only,
        core.log_level,
        core.use_rust_fuse,
        core.pool_max_connections
    );
    info!(
        "FOD mount read_only={} default_permissions={} lazytime={} sync={} dirsync={} atime_policy={:?} use_fuse_context={} fopen_direct_io={} fuse_writeback_cache={}",
        mount.read_only,
        mount.default_permissions,
        mount.lazytime,
        mount.sync,
        mount.dirsync,
        mount.atime_policy,
        mount.use_fuse_context,
        mount.fopen_direct_io,
        mount.fuse_writeback_cache
    );
    info!("FOD recovery_mode={}", snapshot.is_in_recovery);
    info!(
        "FOD cache metadata_cache_ttl={}s statfs_cache_ttl={}s read_cache_blocks={} read_cache_eviction_policy={} read_ahead_blocks={} sequential_read_ahead_blocks={} small_file_read_threshold_blocks={}",
        cache.metadata_cache_ttl.as_secs(),
        cache.statfs_cache_ttl.as_secs(),
        cache.read_cache_blocks,
        cache.read_cache_eviction_policy,
        cache.read_ahead_blocks,
        cache.sequential_read_ahead_blocks,
        cache.small_file_read_threshold_blocks
    );
    info!(
        "FOD lock backend={:?} lock_lease_ttl={}s lock_heartbeat_interval={}s lock_poll_interval={}s",
        lock.lock_backend,
        lock.lock_lease_ttl.as_secs_f64(),
        lock.lock_heartbeat_interval.as_secs_f64(),
        lock.lock_poll_interval.as_secs_f64()
    );
    info!(
        "FOD security selinux_enabled={} acl_enabled={} context={:?} fscontext={:?} defcontext={:?} rootcontext={:?}",
        security.selinux_enabled,
        security.acl_enabled,
        security.selinux_context,
        security.selinux_fscontext,
        security.selinux_defcontext,
        security.selinux_rootcontext
    );
    info!(
        "FOD storage block_size={} write_flush_threshold={} bytes max_fs_size_bytes={:?} pg_visible_path={:?} workers_read={} workers_read_min_blocks={} workers_write={} workers_write_min_blocks={} persist_buffer_chunk_blocks={} persist_block_transport={} data_object_swap_cleanup={} synchronous_commit={} copy_dedupe_enabled={} copy_dedupe_min_blocks={} copy_dedupe_max_blocks={} copy_dedupe_crc_table={} enable_extents={} extent_target_bytes={}",
        fs.block_size,
        storage.write_flush_threshold_bytes,
        storage.max_fs_size_bytes,
        storage
            .pg_visible_path
            .as_ref()
            .map(|path| path.display().to_string()),
        storage.workers_read,
        storage.workers_read_min_blocks,
        storage.workers_write,
        storage.workers_write_min_blocks,
        storage.persist_buffer_chunk_blocks,
        storage.persist_block_transport.as_str(),
        storage.data_object_swap_cleanup.as_str(),
        storage.synchronous_commit.as_str(),
        storage.copy_dedupe_enabled,
        storage.copy_dedupe_min_blocks,
        storage.copy_dedupe_max_blocks,
        storage.copy_dedupe_crc_table,
        storage.enable_extents,
        storage.extent_target_bytes
    );
    info!("FOD mount options: {:?}", options);
    if env_var_truthy_with_legacy_alias("FOD_DEBUG_SNAPSHOT", false) {
        info!("FOD debug snapshot: {}", fs.debug_snapshot());
    }
}

pub fn mount_fuse(
    repo: DbRepo,
    runtime: &RuntimeConfig,
    settings: FodFuseSettings,
    mountpoint: &Path,
    snapshot: &StartupSnapshot,
) -> Result<(), String> {
    if !snapshot.schema_is_initialized {
        return Err(
            "fod schema is not initialized; run `make init` or `mkfs.fod init --schema-admin-password YOUR_SECRET` first"
                .to_string(),
        );
    }

    let mut fs = FodFuse::new(repo, settings, runtime);
    let read_only = fs.read_only;
    let core = runtime.core_settings();
    let mount = runtime.mount_settings(read_only);
    let security = runtime.security_settings();
    let lock = runtime.lock_settings(read_only);
    let cache = runtime.cache_settings();
    let storage = runtime.storage_settings();

    if !read_only {
        fs.repo
            .ensure_lock_schema()
            .map_err(|err| format!("failed to ensure lock schema: {err}"))?;
        fs.repo
            .ensure_client_session_schema()
            .map_err(|err| format!("failed to ensure client session schema: {err}"))?;
        fs.register_client_session(mountpoint, "primary")
            .map_err(|err| format!("failed to register client session: {err}"))?;
        fs.start_lock_heartbeat()
            .map_err(|err| format!("failed to start lock heartbeat: {err}"))?;
    }
    fs.start_runtime_reload(runtime)
        .map_err(|err| format!("failed to start runtime reload: {err}"))?;

    let options = mount_options(&mount, &security);
    log_mount_status(
        mountpoint, &core, &mount, &security, &lock, &cache, &storage, &fs, snapshot, &options,
    );
    info!("FOD mounting filesystem at {}", mountpoint.display());
    println!(
        "fod-rust-fuse: mounting filesystem at {}",
        mountpoint.display()
    );

    mount2(fs, mountpoint, &options).map_err(|err| format!("mount failed: {:?}", err))
}
