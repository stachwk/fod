// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use support::{db_repo, mountpoint_ready, resolve_file_id, try_unmount, unique_suffix, MountedFs};

struct DeadlockChildren {
    children: Vec<(Child, ChildStdin)>,
    ready_paths: Vec<PathBuf>,
}

impl Drop for DeadlockChildren {
    fn drop(&mut self) {
        for ready_path in &self.ready_paths {
            let _ = fs::remove_file(ready_path);
        }
        for (child, _) in &mut self.children {
            let _ = child.kill();
        }
        for (child, _) in &mut self.children {
            let _ = child.wait();
        }
    }
}

extern "C" fn ignore_signal(_signal: libc::c_int) {}

fn install_signal_handler(signal: libc::c_int) -> Result<libc::sigaction, String> {
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = ignore_signal as *const () as usize;
    action.sa_flags = 0;
    unsafe { libc::sigemptyset(&mut action.sa_mask) };
    let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::sigaction(signal, &action, &mut previous) };
    if rc == -1 {
        return Err(format!(
            "install signal handler failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(previous)
}

fn flock(lock_type: i16, start: i64, len: i64) -> libc::flock {
    flock_with_whence(lock_type, libc::SEEK_SET as i16, start, len)
}

fn flock_with_whence(lock_type: i16, whence: i16, start: i64, len: i64) -> libc::flock {
    libc::flock {
        l_type: lock_type,
        l_whence: whence,
        l_start: start,
        l_len: len,
        l_pid: 0,
    }
}

fn fcntl_lock(fd: i32, cmd: i32, lock: &mut libc::flock) -> Result<(), String> {
    let rc = unsafe { libc::fcntl(fd, cmd, lock) };
    if rc == -1 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

fn query_lock(fd: i32, lock_type: i16) -> Result<libc::flock, String> {
    let mut lock = flock(lock_type, 0, 0);
    fcntl_lock(fd, libc::F_GETLK, &mut lock)?;
    Ok(lock)
}

fn query_lock_range(fd: i32, lock_type: i16, start: i64, len: i64) -> Result<libc::flock, String> {
    let mut lock = flock(lock_type, start, len);
    fcntl_lock(fd, libc::F_GETLK, &mut lock)?;
    Ok(lock)
}

fn running_as_root() -> bool {
    (unsafe { libc::geteuid() }) == 0
}

fn skip_without_root() -> bool {
    if running_as_root() {
        false
    } else {
        eprintln!("skipping lock backend smoke: run via sudo for root/FUSE coverage");
        true
    }
}

#[test]
fn memory_backend_mount_prunes_expired_shared_sessions_without_lock_heartbeat() -> Result<(), String>
{
    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }

    let memory_mount = MountedFs::start_with_role(
        "lock-backend-memory-session-maintenance",
        "primary",
        &[
            ("FOD_LOCK_BACKEND", "memory".to_string()),
            ("FOD_SESSION_MAINTENANCE_INTERVAL_MS", "500".to_string()),
        ],
    )?;
    let startup_log = memory_mount.log_tail(200);
    if !startup_log.contains("lock backend=Memory") {
        return Err(format!(
            "test mount did not use memory lock backend\n{}",
            startup_log
        ));
    }

    let repo = db_repo()?;
    let token = format!("expired-memory-session-{}", unique_suffix());
    let escaped_token = token.replace('\'', "''");
    repo.exec(&format!(
        "
        WITH inserted AS (
            INSERT INTO client_sessions (
                host_name,
                mountpoint,
                mount_mode,
                lock_backend,
                pid,
                request_token,
                lease_expires_at,
                heartbeat_at
            )
            VALUES (
                'expired-memory-test',
                '/tmp/fod-expired-memory-test',
                'primary',
                'memory',
                0,
                '{escaped_token}',
                NOW() - INTERVAL '1 second',
                NOW() - INTERVAL '1 second'
            )
            RETURNING session_id
        )
        INSERT INTO monitor_session_stats (
            session_id,
            fod_version,
            sample_seq,
            sampled_at,
            payload_json,
            updated_at
        )
        SELECT
            session_id,
            '3.3.3-test',
            1,
            NOW(),
            jsonb_build_object('test_token', '{escaped_token}'),
            NOW()
        FROM inserted
        "
    ))?;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let remaining = repo.query_scalar_text(&format!(
            "SELECT COUNT(*)::text FROM client_sessions WHERE request_token = '{escaped_token}'"
        ))?;
        if remaining.trim() == "0" {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "expired memory-backend session was not pruned; remaining={}\n{}",
                remaining.trim(),
                memory_mount.log_tail(200)
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let monitor_remaining = repo.query_scalar_text(&format!(
        "
        SELECT COUNT(*)::text
        FROM monitor_session_stats
        WHERE payload_json ->> 'test_token' = '{escaped_token}'
        "
    ))?;
    if monitor_remaining.trim() != "0" {
        return Err(format!(
            "monitor_session_stats row did not cascade with expired client session: {}",
            monitor_remaining.trim()
        ));
    }

    drop(memory_mount);
    Ok(())
}

#[test]
fn primary_lease_expiry_allows_second_mount_reacquire() -> Result<(), String> {
    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }
    let lease_ttl = "120".to_string();
    let primary_a = MountedFs::start_with_role(
        "lock-backend-expiry-primary-a",
        "primary",
        &[("FOD_LOCK_LEASE_TTL_SECONDS", lease_ttl.clone())],
    )?;
    let suffix = unique_suffix();
    let dir_path = primary_a.mountpoint.join(format!("lock-expiry-{suffix}"));
    let file_path = dir_path.join("payload.txt");
    fs::create_dir(&dir_path).map_err(|err| format!("create_dir failed: {err}"))?;
    fs::write(&file_path, b"lock-expiry").map_err(|err| format!("write failed: {err}"))?;

    let fd_a = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("open primary A failed: {err}"))?;
    let mut write_lock = flock(libc::F_WRLCK as i16, 0, 0);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut write_lock)
        .map_err(|err| format!("primary A setlk failed: {err}\n{}", primary_a.log_tail(200)))?;

    let primary_b = MountedFs::start_with_role(
        "lock-backend-expiry-primary-b",
        "primary",
        &[("FOD_LOCK_LEASE_TTL_SECONDS", lease_ttl)],
    )?;
    let fd_b = OpenOptions::new()
        .read(true)
        .open(
            primary_b
                .mountpoint
                .join(format!("lock-expiry-{suffix}/payload.txt")),
        )
        .map_err(|err| format!("open primary B failed: {err}\n{}", primary_b.log_tail(200)))?;

    let primary_b_before_expiry = query_lock(fd_b.as_raw_fd(), libc::F_WRLCK as i16)?;
    if primary_b_before_expiry.l_type != libc::F_WRLCK as i16 {
        return Err(format!(
            "primary B should see PG-backed write lock before expiry, got l_type={}",
            primary_b_before_expiry.l_type
        ));
    }

    let repo = db_repo()?;
    let file_id = resolve_file_id(&repo, &primary_a.mountpoint, &file_path)?;
    repo.exec(&format!(
        "UPDATE lock_range_leases SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE resource_kind = 'file' AND resource_id = {file_id}"
    ))?;

    let primary_b_after_expiry = query_lock(fd_b.as_raw_fd(), libc::F_WRLCK as i16)?;
    if primary_b_after_expiry.l_type != libc::F_UNLCK as i16 {
        return Err(format!(
            "primary B should see the expired lease as unlocked, got l_type={}",
            primary_b_after_expiry.l_type
        ));
    }

    // Primary B jest celowo O_RDONLY: write ownership pozostaje pojedyncze,
    // natomiast POSIX lock backend nadal musi pozwolic na read-lock po
    // wygaśnięciu centralnego write-lock lease.
    let mut reacquire = flock(libc::F_RDLCK as i16, 0, 0);
    fcntl_lock(fd_b.as_raw_fd(), libc::F_SETLK, &mut reacquire).map_err(|err| {
        format!(
            "primary B read-lock reacquire after expiry failed: {err}\n{}",
            primary_b.log_tail(200)
        )
    })?;

    let primary_a_after_b_reacquire = query_lock(fd_a.as_raw_fd(), libc::F_WRLCK as i16)?;
    if primary_a_after_b_reacquire.l_type != libc::F_RDLCK as i16 {
        return Err(format!(
            "primary A should see the reacquired read lock as conflicting, got l_type={}",
            primary_a_after_b_reacquire.l_type
        ));
    }

    let mut unlock_b = flock(libc::F_UNLCK as i16, 0, 0);
    fcntl_lock(fd_b.as_raw_fd(), libc::F_SETLK, &mut unlock_b).map_err(|err| {
        format!(
            "primary B unlock failed: {err}\n{}",
            primary_b.log_tail(200)
        )
    })?;
    let mut unlock_a = flock(libc::F_UNLCK as i16, 0, 0);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut unlock_a).map_err(|err| {
        format!(
            "primary A unlock failed: {err}\n{}",
            primary_a.log_tail(200)
        )
    })?;

    drop(fd_b);
    drop(fd_a);
    drop(primary_b);
    drop(primary_a);
    Ok(())
}

#[test]
fn primary_uses_pg_leases_and_replica_stays_memory_backed() -> Result<(), String> {
    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }
    let primary_a = MountedFs::start_with_role("lock-backend-primary-a", "primary", &[])?;
    let suffix = unique_suffix();
    let dir_path = primary_a.mountpoint.join(format!("lock-backend-{suffix}"));
    let file_path = dir_path.join("payload.txt");
    fs::create_dir(&dir_path).map_err(|err| format!("create_dir failed: {err}"))?;
    fs::write(&file_path, b"lock-backend").map_err(|err| format!("write failed: {err}"))?;

    let fd_a = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("open primary A failed: {err}"))?;
    let mut write_lock = flock(libc::F_WRLCK as i16, 0, 0);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut write_lock)
        .map_err(|err| format!("primary A setlk failed: {err}\n{}", primary_a.log_tail(200)))?;

    let primary_b = MountedFs::start_with_role("lock-backend-primary-b", "primary", &[])?;
    let fd_b = OpenOptions::new()
        .read(true)
        .open(
            primary_b
                .mountpoint
                .join(format!("lock-backend-{suffix}/payload.txt")),
        )
        .map_err(|err| format!("open primary B failed: {err}\n{}", primary_b.log_tail(200)))?;
    let primary_b_query = query_lock(fd_b.as_raw_fd(), libc::F_WRLCK as i16)?;
    if primary_b_query.l_type != libc::F_WRLCK as i16 {
        return Err(format!(
            "primary B should see PG-backed write lock, got l_type={}",
            primary_b_query.l_type
        ));
    }
    if primary_b_query.l_pid != unsafe { libc::getpid() } {
        return Err(format!(
            "F_GETLK should report the conflicting process pid {}, got {}",
            unsafe { libc::getpid() },
            primary_b_query.l_pid
        ));
    }

    // O_RDONLY primary B nie konkuruje o write ownership. F_RDLCK jest
    // jednak prawidlowym POSIX probe i musi konfliktowac z cudzym F_WRLCK.
    let mut blocked_read_lock = flock(libc::F_RDLCK as i16, 0, 0);
    let blocked = unsafe { libc::fcntl(fd_b.as_raw_fd(), libc::F_SETLK, &mut blocked_read_lock) };
    if blocked != -1 {
        return Err("primary B unexpectedly acquired conflicting read lock".to_string());
    }
    let blocked_errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    if blocked_errno != libc::EWOULDBLOCK && blocked_errno != libc::EACCES {
        return Err(format!("primary B wrong lock error: {blocked_errno}"));
    }

    let replica = MountedFs::start_with_role("lock-backend-replica", "replica", &[])?;
    let fd_replica = OpenOptions::new()
        .read(true)
        .open(
            replica
                .mountpoint
                .join(format!("lock-backend-{suffix}/payload.txt")),
        )
        .map_err(|err| format!("open replica failed: {err}\n{}", replica.log_tail(200)))?;
    let replica_query = query_lock(fd_replica.as_raw_fd(), libc::F_WRLCK as i16)?;
    if replica_query.l_type != libc::F_UNLCK as i16 {
        return Err(format!(
            "replica should stay memory-backed and not see PG lease state, got l_type={}",
            replica_query.l_type
        ));
    }

    let mut unlock = flock(libc::F_UNLCK as i16, 0, 0);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut unlock).map_err(|err| {
        format!(
            "primary A unlock failed: {err}\n{}",
            primary_a.log_tail(200)
        )
    })?;
    let primary_b_after_unlock = query_lock(fd_b.as_raw_fd(), libc::F_WRLCK as i16)?;
    if primary_b_after_unlock.l_type != libc::F_UNLCK as i16 {
        return Err(format!(
            "primary B should observe PG refresh after unlock, got l_type={}",
            primary_b_after_unlock.l_type
        ));
    }

    drop(fd_replica);
    drop(fd_b);
    drop(fd_a);
    drop(replica);
    drop(primary_b);
    drop(primary_a);
    Ok(())
}

#[test]
fn getlk_reports_zero_length_lock_through_eof() -> Result<(), String> {
    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }

    let primary_a = MountedFs::start_with_role("lock-eof-primary-a", "primary", &[])?;
    let suffix = unique_suffix();
    let dir_path = primary_a.mountpoint.join(format!("lock-eof-{suffix}"));
    let file_path = dir_path.join("payload.txt");
    fs::create_dir(&dir_path).map_err(|err| format!("create_dir failed: {err}"))?;
    fs::write(&file_path, b"lock-through-eof").map_err(|err| format!("write failed: {err}"))?;

    let fd_a = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("open primary A failed: {err}"))?;
    let lock_start = 7;
    let mut eof_lock = flock(libc::F_WRLCK as i16, lock_start, 0);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut eof_lock).map_err(|err| {
        format!(
            "primary A setlk through EOF failed: {err}\n{}",
            primary_a.log_tail(200)
        )
    })?;

    let primary_b = MountedFs::start_with_role("lock-eof-primary-b", "primary", &[])?;
    let fd_b = OpenOptions::new()
        .read(true)
        .open(
            primary_b
                .mountpoint
                .join(format!("lock-eof-{suffix}/payload.txt")),
        )
        .map_err(|err| format!("open primary B failed: {err}\n{}", primary_b.log_tail(200)))?;
    let observed = query_lock_range(fd_b.as_raw_fd(), libc::F_WRLCK as i16, lock_start + 1, 0)?;
    if observed.l_type != libc::F_WRLCK as i16 {
        return Err(format!(
            "F_GETLK should report the conflicting EOF lock, got l_type={}",
            observed.l_type
        ));
    }
    if observed.l_start != lock_start || observed.l_len != 0 {
        return Err(format!(
            "F_GETLK should preserve EOF range as l_start={lock_start}, l_len=0; got l_start={}, l_len={}",
            observed.l_start, observed.l_len
        ));
    }

    let mut unlock = flock(libc::F_UNLCK as i16, lock_start, 0);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut unlock)
        .map_err(|err| format!("primary A EOF unlock failed: {err}"))?;
    drop(fd_b);
    drop(fd_a);
    drop(primary_b);
    drop(primary_a);
    Ok(())
}

#[test]
fn malformed_active_range_lease_returns_eio_from_getlk() -> Result<(), String> {
    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }

    let mount = MountedFs::start_with_role("lock-malformed-range-lease", "primary", &[])?;
    let suffix = unique_suffix();
    let dir_path = mount.mountpoint.join(format!("lock-malformed-{suffix}"));
    let file_path = dir_path.join("payload.txt");
    fs::create_dir(&dir_path).map_err(|err| format!("create_dir failed: {err}"))?;
    fs::write(&file_path, b"malformed-lock-lease").map_err(|err| format!("write failed: {err}"))?;

    let fd = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("open malformed-lease file failed: {err}"))?;
    let repo = db_repo()?;
    let file_id = resolve_file_id(&repo, &mount.mountpoint, &file_path)?;
    let owner = file_id.saturating_add(1);
    repo.exec(&format!(
        "INSERT INTO lock_range_leases (resource_kind, resource_id, session_id, owner_key, lock_type, range_start, range_end, lock_pid, lease_expires_at, heartbeat_at, created_at, updated_at) VALUES ('file', {file_id}, 0, {owner}, 3, 0, 8, 1234, NOW() + INTERVAL '1 minute', NOW(), NOW(), NOW())"
    ))
    .map_err(|err| format!("seed malformed active range lease failed: {err}"))?;

    // F_GETLK must surface corrupt authoritative state as EIO. Treating the
    // row as absent would incorrectly report F_UNLCK and permit new locks.
    let mut probe = flock(libc::F_WRLCK as i16, 0, 1);
    let rc = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETLK, &mut probe) };
    let observed_errno = std::io::Error::last_os_error().raw_os_error();

    let cleanup = repo.exec(&format!(
        "DELETE FROM lock_range_leases WHERE resource_kind = 'file' AND resource_id = {file_id} AND owner_key = {owner}"
    ));
    drop(fd);
    let _ = fs::remove_file(&file_path);
    let _ = fs::remove_dir(&dir_path);
    drop(mount);
    cleanup.map_err(|err| format!("remove malformed range lease fixture failed: {err}"))?;

    if rc != -1 || observed_errno != Some(libc::EIO) {
        return Err(format!(
            "F_GETLK on malformed active range lease should fail with EIO, got rc={rc} errno={observed_errno:?}"
        ));
    }
    Ok(())
}

#[test]
fn busy_mount_drop_preserves_shared_namespace_entries() -> Result<(), String> {
    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }

    let mount = MountedFs::start_with_role("lock-busy-drop-primary", "primary", &[])?;
    let suffix = unique_suffix();
    let sentinel_name = format!("lock-busy-drop-{suffix}");
    let sentinel_path = mount.mountpoint.join(&sentinel_name);
    fs::create_dir(&sentinel_path).map_err(|err| format!("create sentinel failed: {err}"))?;
    let workspace = mount.workspace.clone();
    let mountpoint = mount.mountpoint.clone();
    let mut cwd_holder = std::process::Command::new("sleep")
        .arg("60")
        .current_dir(&mountpoint)
        .spawn()
        .map_err(|err| format!("start mount cwd holder failed: {err}"))?;

    // A process cwd inside the mount makes normal unmount return EBUSY. Drop
    // must preserve the workspace instead of recursively deleting through the
    // still-mounted shared namespace.
    drop(mount);
    let workspace_preserved = workspace.exists();
    let mount_still_attached = mountpoint_ready(&mountpoint);

    let _ = cwd_holder.kill();
    let _ = cwd_holder.wait();
    let detached_after_cwd_exit = try_unmount(&mountpoint);
    if detached_after_cwd_exit {
        let _ = fs::remove_dir_all(&workspace);
    }
    if !workspace_preserved || !mount_still_attached || !detached_after_cwd_exit {
        return Err(format!(
            "busy mount cleanup did not preserve and later detach the workspace: preserved={workspace_preserved} attached_after_drop={mount_still_attached} detached_after_cwd_exit={detached_after_cwd_exit}"
        ));
    }

    let observer = MountedFs::start_with_role("lock-busy-drop-observer", "primary", &[])?;
    let observed_path = observer.mountpoint.join(&sentinel_name);
    let sentinel_preserved = observed_path.is_dir();
    if sentinel_preserved {
        fs::remove_dir(&observed_path)
            .map_err(|err| format!("remove verified sentinel failed: {err}"))?;
    }
    drop(observer);
    if !sentinel_preserved {
        return Err(format!(
            "test mount cleanup removed shared namespace sentinel {sentinel_name}"
        ));
    }
    Ok(())
}

#[test]
fn primary_mounts_conflict_on_range_lock_across_hosts() -> Result<(), String> {
    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }
    let primary_a = MountedFs::start_with_role("lock-backend-range-primary-a", "primary", &[])?;
    let suffix = unique_suffix();
    let dir_path = primary_a.mountpoint.join(format!("lock-range-{suffix}"));
    let file_path = dir_path.join("payload.txt");
    fs::create_dir(&dir_path).map_err(|err| format!("create_dir failed: {err}"))?;
    fs::write(&file_path, b"lock-range-backend").map_err(|err| format!("write failed: {err}"))?;

    let fd_a = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("open primary A failed: {err}"))?;
    let write_only = OpenOptions::new()
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("open primary A write-only failed: {err}"))?;
    let mut invalid_read_lock = flock(libc::F_RDLCK as i16, 0, 1);
    let invalid_read_result = unsafe {
        libc::fcntl(
            write_only.as_raw_fd(),
            libc::F_SETLK,
            &mut invalid_read_lock,
        )
    };
    if invalid_read_result != -1
        || std::io::Error::last_os_error().raw_os_error() != Some(libc::EBADF)
    {
        return Err("F_RDLCK on O_WRONLY descriptor should fail with EBADF".to_string());
    }
    drop(write_only);
    let range_start = 2;
    let range_len = 8;
    let mut write_lock = flock(libc::F_WRLCK as i16, range_start, range_len);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut write_lock).map_err(|err| {
        format!(
            "primary A range setlk failed: {err}\n{}",
            primary_a.log_tail(200)
        )
    })?;

    let primary_b = MountedFs::start_with_role("lock-backend-range-primary-b", "primary", &[])?;
    let fd_b = OpenOptions::new()
        .read(true)
        .open(
            primary_b
                .mountpoint
                .join(format!("lock-range-{suffix}/payload.txt")),
        )
        .map_err(|err| format!("open primary B failed: {err}\n{}", primary_b.log_tail(200)))?;
    let mut invalid_write_lock = flock(libc::F_WRLCK as i16, 0, 1);
    let invalid_write_result =
        unsafe { libc::fcntl(fd_b.as_raw_fd(), libc::F_SETLK, &mut invalid_write_lock) };
    if invalid_write_result != -1
        || std::io::Error::last_os_error().raw_os_error() != Some(libc::EBADF)
    {
        return Err("F_WRLCK on O_RDONLY descriptor should fail with EBADF".to_string());
    }
    let primary_b_query = query_lock_range(
        fd_b.as_raw_fd(),
        libc::F_WRLCK as i16,
        range_start,
        range_len,
    )?;
    if primary_b_query.l_type != libc::F_WRLCK as i16 {
        return Err(format!(
            "primary B should see PG-backed range write lock, got l_type={}",
            primary_b_query.l_type
        ));
    }
    let adjacent_query = query_lock_range(
        fd_b.as_raw_fd(),
        libc::F_WRLCK as i16,
        range_start + range_len,
        1,
    )?;
    if adjacent_query.l_type != libc::F_UNLCK as i16 {
        return Err(format!(
            "lock must not include the byte immediately after its range, got l_type={}",
            adjacent_query.l_type
        ));
    }

    // Zakresowy read lock jest legalny na O_RDONLY i nadal musi widziec
    // centralny write-lock lease pierwszego primary.
    let mut blocked_read_lock = flock(libc::F_RDLCK as i16, range_start, range_len);
    let blocked = unsafe { libc::fcntl(fd_b.as_raw_fd(), libc::F_SETLK, &mut blocked_read_lock) };
    if blocked != -1 {
        return Err("primary B unexpectedly acquired conflicting range read lock".to_string());
    }
    let blocked_errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    if blocked_errno != libc::EWOULDBLOCK && blocked_errno != libc::EACCES {
        return Err(format!("primary B wrong range lock error: {blocked_errno}"));
    }

    let mut partial_unlock = flock(libc::F_UNLCK as i16, 4, 2);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut partial_unlock)
        .map_err(|err| format!("primary A partial range unlock failed: {err}"))?;
    for (offset, expected_type) in [
        (2, libc::F_WRLCK as i16),
        (4, libc::F_UNLCK as i16),
        (8, libc::F_WRLCK as i16),
    ] {
        let observed = query_lock_range(fd_b.as_raw_fd(), libc::F_WRLCK as i16, offset, 1)?;
        if observed.l_type != expected_type {
            let repo = db_repo()?;
            let file_id = resolve_file_id(&repo, &primary_a.mountpoint, &file_path)?;
            let ranges = repo.load_lock_range_state_blob("file", file_id)?;
            return Err(format!(
                "partial unlock at offset 4 length 2: offset {offset} expected lock type {expected_type}, got {}; stored ranges: {}",
                observed.l_type,
                String::from_utf8_lossy(&ranges)
            ));
        }
    }

    let mut middle_read_lock = flock(libc::F_RDLCK as i16, 4, 2);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut middle_read_lock)
        .map_err(|err| format!("primary A middle range read lock failed: {err}"))?;
    for (offset, expected_type) in [
        (2, libc::F_WRLCK as i16),
        (4, libc::F_RDLCK as i16),
        (8, libc::F_WRLCK as i16),
    ] {
        let observed = query_lock_range(fd_b.as_raw_fd(), libc::F_WRLCK as i16, offset, 1)?;
        if observed.l_type != expected_type {
            return Err(format!(
                "replacing middle range with read lock: offset {offset} expected lock type {expected_type}, got {}",
                observed.l_type
            ));
        }
    }

    let mut unlock = flock(libc::F_UNLCK as i16, range_start, range_len);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut unlock).map_err(|err| {
        format!(
            "primary A range unlock failed: {err}\n{}",
            primary_a.log_tail(200)
        )
    })?;
    let primary_b_after_unlock = query_lock_range(
        fd_b.as_raw_fd(),
        libc::F_WRLCK as i16,
        range_start,
        range_len,
    )?;
    if primary_b_after_unlock.l_type != libc::F_UNLCK as i16 {
        return Err(format!(
            "primary B should observe PG refresh for range lock after unlock, got l_type={}",
            primary_b_after_unlock.l_type
        ));
    }

    let file_len = fs::metadata(&file_path)
        .map_err(|err| format!("stat lock file failed: {err}"))?
        .len() as i64;
    let relative_start = file_len - 4;
    let mut relative_lock = flock_with_whence(libc::F_WRLCK as i16, libc::SEEK_END as i16, -4, 2);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut relative_lock)
        .map_err(|err| format!("SEEK_END relative range lock failed: {err}"))?;
    let relative_observed =
        query_lock_range(fd_b.as_raw_fd(), libc::F_WRLCK as i16, relative_start, 2)?;
    if relative_observed.l_type != libc::F_WRLCK as i16 {
        return Err(format!(
            "SEEK_END relative lock was not normalized to absolute range, got l_type={}",
            relative_observed.l_type
        ));
    }
    relative_lock.l_type = libc::F_UNLCK as i16;
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut relative_lock)
        .map_err(|err| format!("SEEK_END relative range unlock failed: {err}"))?;

    let mut negative_length_lock = flock(libc::F_WRLCK as i16, 10, -4);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut negative_length_lock)
        .map_err(|err| format!("negative-length range lock failed: {err}"))?;
    let negative_length_observed = query_lock_range(fd_b.as_raw_fd(), libc::F_WRLCK as i16, 6, 4)?;
    if negative_length_observed.l_type != libc::F_WRLCK as i16 {
        return Err(format!(
            "negative-length lock was not normalized to bytes 6..9, got l_type={}",
            negative_length_observed.l_type
        ));
    }
    negative_length_lock.l_type = libc::F_UNLCK as i16;
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut negative_length_lock)
        .map_err(|err| format!("negative-length range unlock failed: {err}"))?;

    drop(fd_b);
    drop(fd_a);
    drop(primary_b);
    drop(primary_a);
    Ok(())
}

#[test]
fn blocking_setlkw_waits_for_cross_mount_unlock() -> Result<(), String> {
    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }

    let primary_a = MountedFs::start_with_role("lock-setlkw-primary-a", "primary", &[])?;
    let suffix = unique_suffix();
    let relative = format!("lock-setlkw-{suffix}/payload.txt");
    let file_path = primary_a.mountpoint.join(&relative);
    fs::create_dir(file_path.parent().expect("file parent"))
        .map_err(|err| format!("create lock directory failed: {err}"))?;
    fs::write(&file_path, b"blocking lock handoff")
        .map_err(|err| format!("write lock file failed: {err}"))?;

    let fd_a = OpenOptions::new()
        .read(true)
        .open(&file_path)
        .map_err(|err| format!("open primary A failed: {err}"))?;
    let mut lock_a = flock(libc::F_RDLCK as i16, 0, 0);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut lock_a)
        .map_err(|err| format!("primary A setlk failed: {err}"))?;

    let primary_b = MountedFs::start_with_role(
        "lock-setlkw-primary-b",
        "primary",
        &[("FOD_LOG_LEVEL", "debug".to_string())],
    )?;
    let fd_b = OpenOptions::new()
        .read(true)
        .write(true)
        .open(primary_b.mountpoint.join(&relative))
        .map_err(|err| format!("open primary B failed: {err}"))?;

    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let waiter = thread::spawn(move || {
        let mut lock_b = flock(libc::F_WRLCK as i16, 0, 0);
        let result = fcntl_lock(fd_b.as_raw_fd(), libc::F_SETLKW, &mut lock_b);
        let _ = result_tx.send((result, fd_b));
    });

    let wait_marker = format!("FOD setlk waiting path=/{relative}");
    let wait_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if primary_b.log_tail(400).contains(&wait_marker) {
            break;
        }
        if let Ok((result, returned_fd)) = result_rx.try_recv() {
            let _ = waiter.join();
            drop(returned_fd);
            return Err(format!(
                "F_SETLKW returned before the conflicting lock was released: {result:?}"
            ));
        }
        if Instant::now() >= wait_deadline {
            return Err(format!(
                "F_SETLKW did not enter the blocking wait path\n{}",
                primary_b.log_tail(400)
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }

    let mut unlock_a = flock(libc::F_UNLCK as i16, 0, 0);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut unlock_a)
        .map_err(|err| format!("primary A unlock failed: {err}"))?;

    let (wait_result, fd_b) = result_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|err| format!("F_SETLKW did not complete after unlock: {err}"))?;
    wait_result.map_err(|err| format!("F_SETLKW failed after unlock: {err}"))?;
    waiter
        .join()
        .map_err(|_| "F_SETLKW waiter thread panicked".to_string())?;

    let observed = query_lock(fd_a.as_raw_fd(), libc::F_WRLCK as i16)?;
    if observed.l_type != libc::F_WRLCK as i16 {
        return Err(format!(
            "primary A should observe primary B's acquired write lock, got l_type={}",
            observed.l_type
        ));
    }

    let mut unlock_b = flock(libc::F_UNLCK as i16, 0, 0);
    fcntl_lock(fd_b.as_raw_fd(), libc::F_SETLK, &mut unlock_b)
        .map_err(|err| format!("primary B unlock failed: {err}"))?;

    drop(fd_b);
    drop(fd_a);
    drop(primary_b);
    drop(primary_a);
    Ok(())
}

#[test]
fn blocking_setlkw_reports_cross_mount_deadlock() -> Result<(), String> {
    const CHILD_MODE: &str = "FOD_LOCK_DEADLOCK_CHILD";
    if std::env::var_os(CHILD_MODE).is_some() {
        let initial_path = std::env::var("FOD_LOCK_DEADLOCK_INITIAL_PATH")
            .map_err(|err| format!("missing deadlock initial path: {err}"))?;
        let target_path = std::env::var("FOD_LOCK_DEADLOCK_TARGET_PATH")
            .map_err(|err| format!("missing deadlock target path: {err}"))?;
        let ready_path = std::env::var("FOD_LOCK_DEADLOCK_READY")
            .map_err(|err| format!("missing deadlock ready path: {err}"))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&initial_path)
            .map_err(|err| format!("deadlock child open failed: {err}"))?;
        let target_file = OpenOptions::new()
            .read(true)
            .open(&target_path)
            .map_err(|err| format!("deadlock child target open failed: {err}"))?;
        let mut initial_lock = flock(libc::F_WRLCK as i16, 0, 1);
        fcntl_lock(file.as_raw_fd(), libc::F_SETLK, &mut initial_lock)
            .map_err(|err| format!("deadlock child initial lock failed: {err}"))?;
        fs::write(&ready_path, b"ready")
            .map_err(|err| format!("deadlock child ready marker failed: {err}"))?;

        let mut go = [0u8; 1];
        std::io::stdin()
            .read_exact(&mut go)
            .map_err(|err| format!("deadlock child start signal failed: {err}"))?;
        if go[0] != b'G' {
            return Err("deadlock child received an invalid start signal".to_string());
        }

        let mut requested_lock = flock(libc::F_RDLCK as i16, 0, 1);
        let rc = unsafe {
            libc::fcntl(
                target_file.as_raw_fd(),
                libc::F_SETLKW,
                &mut requested_lock as *mut libc::flock,
            )
        };
        let errno = if rc == -1 {
            std::io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or(libc::EIO)
        } else {
            0
        };
        if errno != libc::EDEADLK {
            return Err(format!(
                "F_SETLKW should detect the inter-process cycle as EDEADLK, got rc={rc} errno={errno}"
            ));
        }
        return Ok(());
    }

    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }

    let debug_options = [("FOD_LOG_LEVEL", "debug".to_string())];
    let mount_a = MountedFs::start_with_role("lock-setlkw-deadlock-a", "primary", &debug_options)?;
    let mount_b = MountedFs::start_with_role("lock-setlkw-deadlock-b", "primary", &debug_options)?;
    let suffix = unique_suffix();
    let file_a = mount_a
        .mountpoint
        .join(format!("lock-setlkw-deadlock-{suffix}-a.bin"));
    let file_b = mount_a
        .mountpoint
        .join(format!("lock-setlkw-deadlock-{suffix}-b.bin"));
    let file_a_on_b = mount_b.mountpoint.join(file_a.file_name().unwrap());
    let file_b_on_b = mount_b.mountpoint.join(file_b.file_name().unwrap());
    fs::write(&file_a, b"deadlock detection a")
        .map_err(|err| format!("seed first deadlock lock file failed: {err}"))?;
    fs::write(&file_b, b"deadlock detection b")
        .map_err(|err| format!("seed second deadlock lock file failed: {err}"))?;

    let mut children = DeadlockChildren {
        children: Vec::new(),
        ready_paths: Vec::new(),
    };
    for worker in 0..2 {
        let (initial_path, target_path) = if worker == 0 {
            (file_a.clone(), file_b_on_b.clone())
        } else {
            (file_b_on_b.clone(), file_a_on_b.clone())
        };
        let ready_path = std::env::temp_dir().join(format!(
            "fod-lock-deadlock-{}-{worker}-{suffix}.ready",
            std::process::id()
        ));
        let child = Command::new(std::env::current_exe().map_err(|err| err.to_string())?)
            .arg("--exact")
            .arg("blocking_setlkw_reports_cross_mount_deadlock")
            .arg("--nocapture")
            .env(CHILD_MODE, "1")
            .env("FOD_LOCK_DEADLOCK_INITIAL_PATH", &initial_path)
            .env("FOD_LOCK_DEADLOCK_TARGET_PATH", &target_path)
            .env("FOD_LOCK_DEADLOCK_READY", &ready_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|err| format!("spawn deadlock lock owner failed: {err}"))?;
        let mut child = child;
        let child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| "deadlock lock owner has no stdin pipe".to_string())?;
        children.ready_paths.push(ready_path.clone());
        children.children.push((child, child_stdin));

        let ready_deadline = Instant::now() + Duration::from_secs(5);
        while !ready_path.exists() {
            if let Some(status) = children
                .children
                .last_mut()
                .expect("child was stored")
                .0
                .try_wait()
                .map_err(|err| format!("poll deadlock lock owner failed: {err}"))?
            {
                return Err(format!(
                    "deadlock lock owner exited before taking its first lock: {status}"
                ));
            }
            if Instant::now() >= ready_deadline {
                return Err(format!(
                    "deadlock lock owner did not acquire its first range: {}",
                    ready_path.display()
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    for (_, stdin) in &mut children.children {
        stdin
            .write_all(b"G")
            .map_err(|err| format!("release deadlock lock owner: {err}"))?;
        stdin
            .flush()
            .map_err(|err| format!("flush deadlock lock owner signal: {err}"))?;
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut detected = false;
    let mut child_failures = Vec::new();
    while Instant::now() < deadline {
        for (child, _) in &mut children.children {
            if let Some(status) = child
                .try_wait()
                .map_err(|err| format!("poll deadlock result failed: {err}"))?
            {
                if status.success() {
                    detected = true;
                } else {
                    child_failures.push(status.to_string());
                }
            }
        }
        if detected || !child_failures.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    if !child_failures.is_empty() {
        return Err(format!(
            "deadlock child rejected F_SETLKW result: {}",
            child_failures.join(", ")
        ));
    }
    if !detected {
        return Err(
            format!(
                "F_SETLKW deadlock cycle did not produce EDEADLK within 10 seconds\nmount A:\n{}\nmount B:\n{}",
                mount_a.log_tail(300),
                mount_b.log_tail(300)
            ),
        );
    }
    drop(children);
    drop(mount_b);
    drop(mount_a);
    Ok(())
}

#[test]
fn blocking_setlkw_returns_eintr_without_acquiring_lock_later() -> Result<(), String> {
    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }

    let signal = libc::SIGUSR1;
    let primary_a = MountedFs::start_with_role("lock-setlkw-eintr-primary-a", "primary", &[])?;
    let suffix = unique_suffix();
    let relative = format!("lock-setlkw-eintr-{suffix}/payload.txt");
    let file_path = primary_a.mountpoint.join(&relative);
    fs::create_dir(file_path.parent().expect("file parent"))
        .map_err(|err| format!("create lock directory failed: {err}"))?;
    fs::write(&file_path, b"interrupt blocking lock")
        .map_err(|err| format!("write lock file failed: {err}"))?;

    let fd_a = OpenOptions::new()
        .read(true)
        .open(&file_path)
        .map_err(|err| format!("open primary A failed: {err}"))?;
    let mut lock_a = flock(libc::F_RDLCK as i16, 0, 0);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut lock_a)
        .map_err(|err| format!("primary A setlk failed: {err}"))?;

    let primary_b = MountedFs::start_with_role(
        "lock-setlkw-eintr-primary-b",
        "primary",
        &[("FOD_LOG_LEVEL", "debug".to_string())],
    )?;
    let fd_b = OpenOptions::new()
        .read(true)
        .write(true)
        .open(primary_b.mountpoint.join(&relative))
        .map_err(|err| format!("open primary B failed: {err}\n{}", primary_b.log_tail(200)))?;

    let previous_handler = install_signal_handler(signal)?;
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let (thread_tx, thread_rx) = mpsc::sync_channel(1);
    let waiter = thread::spawn(move || {
        let pthread = unsafe { libc::pthread_self() };
        let _ = thread_tx.send(pthread);
        let mut lock_b = flock(libc::F_WRLCK as i16, 0, 0);
        let rc = unsafe { libc::fcntl(fd_b.as_raw_fd(), libc::F_SETLKW, &mut lock_b) };
        let result = if rc == -1 {
            Err(std::io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or(libc::EIO))
        } else {
            Ok(())
        };
        let _ = result_tx.send((result, fd_b));
    });

    let wait_marker = format!("FOD setlk waiting path=/{relative}");
    let wait_deadline = Instant::now() + Duration::from_secs(5);
    let mut entered_wait = false;
    while Instant::now() < wait_deadline {
        if primary_b.log_tail(400).contains(&wait_marker) {
            entered_wait = true;
            break;
        }
        if result_rx.try_recv().is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let mut failure = if entered_wait {
        match thread_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(pthread) => {
                let rc = unsafe { libc::pthread_kill(pthread, signal) };
                if rc != 0 {
                    Some(format!(
                        "pthread_kill failed: {}",
                        std::io::Error::from_raw_os_error(rc)
                    ))
                } else {
                    None
                }
            }
            Err(err) => Some(format!("waiting pthread id was not reported: {err}")),
        }
    } else {
        Some(format!(
            "F_SETLKW did not enter the blocking wait path\n{}",
            primary_b.log_tail(400)
        ))
    };

    let mut result = result_rx.recv_timeout(Duration::from_secs(3)).ok();
    if result.is_none() {
        if failure.is_none() {
            failure = Some("F_SETLKW did not return after SIGUSR1".to_string());
        }
        // Release the conflicting lock so a broken implementation can still
        // finish and clean up its mount instead of leaving the test hung.
        let mut unlock_a = flock(libc::F_UNLCK as i16, 0, 0);
        if let Err(err) = fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut unlock_a) {
            failure.get_or_insert_with(|| format!("primary A unlock failed: {err}"));
        }
        result = result_rx.recv_timeout(Duration::from_secs(5)).ok();
    } else {
        let mut unlock_a = flock(libc::F_UNLCK as i16, 0, 0);
        if let Err(err) = fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut unlock_a) {
            failure.get_or_insert_with(|| format!("primary A unlock failed: {err}"));
        }
    }

    if let Some((wait_result, fd_b)) = result {
        let was_interrupted = wait_result == Err(libc::EINTR);
        if !was_interrupted {
            failure.get_or_insert_with(|| {
                format!("F_SETLKW should return EINTR after SIGUSR1, got {wait_result:?}")
            });
        }

        match query_lock(fd_a.as_raw_fd(), libc::F_WRLCK as i16) {
            Ok(observed) if observed.l_type != libc::F_UNLCK as i16 => {
                failure.get_or_insert_with(|| {
                    format!(
                        "interrupted waiter left or later acquired a write lock, observed l_type={}",
                        observed.l_type
                    )
                });
                let mut unlock_b = flock(libc::F_UNLCK as i16, 0, 0);
                let _ = fcntl_lock(fd_b.as_raw_fd(), libc::F_SETLK, &mut unlock_b);
            }
            Err(err) => {
                failure.get_or_insert_with(|| format!("query lock after EINTR failed: {err}"));
            }
            _ => {}
        }
        drop(fd_b);
    } else {
        failure.get_or_insert_with(|| "F_SETLKW waiter did not finish during cleanup".to_string());
    }

    if waiter.join().is_err() {
        failure.get_or_insert_with(|| "F_SETLKW waiter thread panicked".to_string());
    }
    unsafe { libc::sigaction(signal, &previous_handler, std::ptr::null_mut()) };
    drop(fd_a);
    drop(primary_b);
    drop(primary_a);
    if let Some(failure) = failure {
        return Err(failure);
    }
    Ok(())
}

#[test]
fn closing_another_descriptor_releases_process_record_locks() -> Result<(), String> {
    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }

    let primary_a = MountedFs::start_with_role("lock-close-primary-a", "primary", &[])?;
    let suffix = unique_suffix();
    let relative = format!("lock-close-{suffix}/payload.txt");
    let file_path = primary_a.mountpoint.join(&relative);
    fs::create_dir(file_path.parent().expect("file parent"))
        .map_err(|err| format!("create lock directory failed: {err}"))?;
    fs::write(&file_path, b"close releases process locks")
        .map_err(|err| format!("write lock file failed: {err}"))?;

    let lock_fd = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("open lock descriptor failed: {err}"))?;
    let close_fd = OpenOptions::new()
        .read(true)
        .open(&file_path)
        .map_err(|err| format!("open descriptor to close failed: {err}"))?;
    let mut write_lock = flock(libc::F_WRLCK as i16, 0, 0);
    fcntl_lock(lock_fd.as_raw_fd(), libc::F_SETLK, &mut write_lock)
        .map_err(|err| format!("set process write lock failed: {err}"))?;

    let primary_b = MountedFs::start_with_role("lock-close-primary-b", "primary", &[])?;
    let observer = OpenOptions::new()
        .read(true)
        .open(primary_b.mountpoint.join(&relative))
        .map_err(|err| format!("open observer descriptor failed: {err}"))?;
    let before_close = query_lock(observer.as_raw_fd(), libc::F_WRLCK as i16)?;
    if before_close.l_type != libc::F_WRLCK as i16 {
        return Err(format!(
            "observer should see process write lock before close, got l_type={}",
            before_close.l_type
        ));
    }

    drop(close_fd);
    let after_close = query_lock(observer.as_raw_fd(), libc::F_WRLCK as i16)?;
    if after_close.l_type != libc::F_UNLCK as i16 {
        return Err(format!(
            "closing another descriptor should release process locks, got l_type={}",
            after_close.l_type
        ));
    }

    drop(observer);
    drop(lock_fd);
    drop(primary_b);
    drop(primary_a);
    Ok(())
}

#[test]
fn primary_uses_pg_range_leases_and_replica_stays_memory_backed() -> Result<(), String> {
    if skip_without_root() {
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping lock backend smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }
    let primary_a = MountedFs::start_with_role("lock-backend-range-replica-a", "primary", &[])?;
    let suffix = unique_suffix();
    let dir_path = primary_a
        .mountpoint
        .join(format!("lock-range-replica-{suffix}"));
    let file_path = dir_path.join("payload.txt");
    fs::create_dir(&dir_path).map_err(|err| format!("create_dir failed: {err}"))?;
    fs::write(&file_path, b"lock-range-replica").map_err(|err| format!("write failed: {err}"))?;

    let fd_a = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("open primary A failed: {err}"))?;
    let range_start = 4;
    let range_len = 6;
    let mut write_lock = flock(libc::F_WRLCK as i16, range_start, range_len);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut write_lock).map_err(|err| {
        format!(
            "primary A range setlk failed: {err}\n{}",
            primary_a.log_tail(200)
        )
    })?;

    let primary_b = MountedFs::start_with_role("lock-backend-range-replica-b", "primary", &[])?;
    let fd_b = OpenOptions::new()
        .read(true)
        .open(
            primary_b
                .mountpoint
                .join(format!("lock-range-replica-{suffix}/payload.txt")),
        )
        .map_err(|err| format!("open primary B failed: {err}\n{}", primary_b.log_tail(200)))?;
    let primary_b_query = query_lock_range(
        fd_b.as_raw_fd(),
        libc::F_WRLCK as i16,
        range_start,
        range_len,
    )?;
    if primary_b_query.l_type != libc::F_WRLCK as i16 {
        return Err(format!(
            "primary B should see PG-backed range write lock, got l_type={}",
            primary_b_query.l_type
        ));
    }

    let replica = MountedFs::start_with_role("lock-backend-range-replica", "replica", &[])?;
    let fd_replica = OpenOptions::new()
        .read(true)
        .open(
            replica
                .mountpoint
                .join(format!("lock-range-replica-{suffix}/payload.txt")),
        )
        .map_err(|err| format!("open replica failed: {err}\n{}", replica.log_tail(200)))?;
    let replica_query = query_lock_range(
        fd_replica.as_raw_fd(),
        libc::F_WRLCK as i16,
        range_start,
        range_len,
    )?;
    if replica_query.l_type != libc::F_UNLCK as i16 {
        return Err(format!(
            "replica should stay memory-backed and not see PG range lease state, got l_type={}",
            replica_query.l_type
        ));
    }

    let mut unlock = flock(libc::F_UNLCK as i16, range_start, range_len);
    fcntl_lock(fd_a.as_raw_fd(), libc::F_SETLK, &mut unlock).map_err(|err| {
        format!(
            "primary A range unlock failed: {err}\n{}",
            primary_a.log_tail(200)
        )
    })?;
    let primary_b_after_unlock = query_lock_range(
        fd_b.as_raw_fd(),
        libc::F_WRLCK as i16,
        range_start,
        range_len,
    )?;
    if primary_b_after_unlock.l_type != libc::F_UNLCK as i16 {
        return Err(format!(
            "primary B should observe PG refresh for range lock after unlock, got l_type={}",
            primary_b_after_unlock.l_type
        ));
    }

    drop(fd_replica);
    drop(fd_b);
    drop(fd_a);
    drop(replica);
    drop(primary_b);
    drop(primary_a);
    Ok(())
}
