// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::fs::{self, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::Path;

use support::{db_repo, resolve_file_id, unique_suffix, MountedFs};

fn flock(lock_type: i16, start: i64, len: i64) -> libc::flock {
    libc::flock {
        l_type: lock_type,
        l_whence: libc::SEEK_SET as i16,
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

fn require_root() -> Result<(), String> {
    if unsafe { libc::geteuid() } != 0 {
        Err("lock backend smoke must be run via sudo".to_string())
    } else {
        Ok(())
    }
}

#[test]
fn primary_lease_expiry_allows_second_mount_reacquire() -> Result<(), String> {
    require_root()?;
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
        .write(true)
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

    let mut reacquire = flock(libc::F_WRLCK as i16, 0, 0);
    fcntl_lock(fd_b.as_raw_fd(), libc::F_SETLK, &mut reacquire).map_err(|err| {
        format!(
            "primary B reacquire after expiry failed: {err}\n{}",
            primary_b.log_tail(200)
        )
    })?;

    let primary_a_after_b_reacquire = query_lock(fd_a.as_raw_fd(), libc::F_WRLCK as i16)?;
    if primary_a_after_b_reacquire.l_type != libc::F_WRLCK as i16 {
        return Err(format!(
            "primary A should see the reacquired lock as conflicting, got l_type={}",
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
    require_root()?;
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
        .write(true)
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

    let mut blocked_write_lock = flock(libc::F_WRLCK as i16, 0, 0);
    let blocked = unsafe { libc::fcntl(fd_b.as_raw_fd(), libc::F_SETLK, &mut blocked_write_lock) };
    if blocked != -1 {
        return Err("primary B unexpectedly acquired conflicting write lock".to_string());
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
fn primary_mounts_conflict_on_range_lock_across_hosts() -> Result<(), String> {
    require_root()?;
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
        .write(true)
        .open(
            primary_b
                .mountpoint
                .join(format!("lock-range-{suffix}/payload.txt")),
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

    let mut blocked_write_lock = flock(libc::F_WRLCK as i16, range_start, range_len);
    let blocked = unsafe { libc::fcntl(fd_b.as_raw_fd(), libc::F_SETLK, &mut blocked_write_lock) };
    if blocked != -1 {
        return Err("primary B unexpectedly acquired conflicting range write lock".to_string());
    }
    let blocked_errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    if blocked_errno != libc::EWOULDBLOCK && blocked_errno != libc::EACCES {
        return Err(format!("primary B wrong range lock error: {blocked_errno}"));
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

    drop(fd_b);
    drop(fd_a);
    drop(primary_b);
    drop(primary_a);
    Ok(())
}

#[test]
fn primary_uses_pg_range_leases_and_replica_stays_memory_backed() -> Result<(), String> {
    require_root()?;
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
        .write(true)
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
