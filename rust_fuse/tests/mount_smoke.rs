// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::ffi::CString;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

use support::{block_size_from_config, unique_suffix, MountedFs};

fn renameat2_with_flags(old: &Path, new: &Path, flags: u32) -> io::Result<()> {
    let old = CString::new(old.as_os_str().as_bytes())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let new = CString::new(new.as_os_str().as_bytes())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            old.as_ptr(),
            libc::AT_FDCWD,
            new.as_ptr(),
            flags,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn metadata_times(path: &Path) -> Result<(i64, i64, i64, i64), String> {
    let meta = fs::metadata(path).map_err(|err| err.to_string())?;
    Ok((
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec(),
    ))
}

fn negotiated_u64(line: &str, key: &str) -> Result<Option<u64>, String> {
    let prefix = format!("{key}=");
    let value = line
        .split_whitespace()
        .find_map(|part| part.strip_prefix(&prefix))
        .ok_or_else(|| format!("missing FUSE negotiated field {key:?}\n{line}"))?;
    if value == "unavailable" {
        return Ok(None);
    }
    value
        .parse::<u64>()
        .map(Some)
        .map_err(|err| format!("invalid FUSE negotiated field {key:?}={value:?}: {err}\n{line}"))
}

#[test]
fn reports_negotiated_fuse_compatibility() -> Result<(), String> {
    let mounted = MountedFs::start("fuse-compatibility")?;
    let log = mounted.log_tail(200);
    let host_requirements_line = log
        .lines()
        .find(|line| line.contains("FOD host requirements:"))
        .ok_or_else(|| format!("missing FOD host requirements log line\n{log}"))?;
    let compatibility_line = log
        .lines()
        .find(|line| line.contains("FOD FUSE compatibility:"))
        .ok_or_else(|| format!("missing FUSE compatibility log line\n{log}"))?;
    let negotiated_line = log
        .lines()
        .find(|line| line.contains("FOD FUSE negotiated:"))
        .ok_or_else(|| format!("missing FUSE negotiated log line\n{log}"))?;

    println!("{host_requirements_line}");
    println!("{compatibility_line}");
    println!("{negotiated_line}");

    for field in [
        "FOD host requirements:",
        "linux_kernel=",
        "minimum_linux_kernel=6.17.0",
        "libfuse=",
        "minimum_libfuse=3.0.0",
    ] {
        if !host_requirements_line.contains(field) {
            return Err(format!(
                "missing host requirement field {field:?}\n{host_requirements_line}"
            ));
        }
    }

    for field in [
        "FOD FUSE compatibility:",
        "fuser=0.18.0",
        "userspace_protocol_max=7.40",
        "kernel_protocol=",
        "negotiated_protocol=",
        "available_capabilities=",
        "fod_requested_capabilities=[POSIX_LOCKS,ATOMIC_O_TRUNC,FLOCK_LOCKS,HANDLE_KILLPRIV_V2,SETXATTR_EXT]",
        "fod_enabled_capabilities=",
    ] {
        if !compatibility_line.contains(field) {
            return Err(format!(
                "missing FUSE compatibility field {field:?}\n{compatibility_line}"
            ));
        }
    }

    for field in [
        "FOD FUSE negotiated:",
        "requested_max_write=",
        "effective_max_write=",
        "requested_max_readahead=",
        "effective_max_readahead=",
        "kernel_page_size_bytes=",
        "kernel_max_pages_limit=",
        "kernel_max_request_bytes=",
        "estimated_request_ceiling_bytes=",
        "max_background=",
        "congestion_threshold=",
    ] {
        if !negotiated_line.contains(field) {
            return Err(format!(
                "missing FUSE negotiated field {field:?}\n{negotiated_line}"
            ));
        }
    }

    let requested_write = negotiated_u64(negotiated_line, "requested_max_write")?
        .ok_or_else(|| format!("requested_max_write unavailable\n{negotiated_line}"))?;
    let effective_write = negotiated_u64(negotiated_line, "effective_max_write")?
        .ok_or_else(|| format!("effective_max_write unavailable\n{negotiated_line}"))?;
    let requested_readahead = negotiated_u64(negotiated_line, "requested_max_readahead")?
        .ok_or_else(|| format!("requested_max_readahead unavailable\n{negotiated_line}"))?;
    let effective_readahead = negotiated_u64(negotiated_line, "effective_max_readahead")?
        .ok_or_else(|| format!("effective_max_readahead unavailable\n{negotiated_line}"))?;

    if requested_write == 0 || effective_write == 0 || effective_write > requested_write {
        return Err(format!(
            "invalid FUSE max_write negotiation requested={requested_write} effective={effective_write}\n{negotiated_line}"
        ));
    }
    if requested_readahead == 0
        || effective_readahead == 0
        || effective_readahead > requested_readahead
    {
        return Err(format!(
            "invalid FUSE max_readahead negotiation requested={requested_readahead} effective={effective_readahead}\n{negotiated_line}"
        ));
    }

    let page_size = negotiated_u64(negotiated_line, "kernel_page_size_bytes")?;
    let max_pages = negotiated_u64(negotiated_line, "kernel_max_pages_limit")?;
    let kernel_max_request = negotiated_u64(negotiated_line, "kernel_max_request_bytes")?;
    let estimated_request = negotiated_u64(negotiated_line, "estimated_request_ceiling_bytes")?
        .ok_or_else(|| format!("estimated_request_ceiling_bytes unavailable\n{negotiated_line}"))?;

    let configured_request = effective_write.max(effective_readahead);
    if estimated_request == 0 || estimated_request > configured_request {
        return Err(format!(
            "invalid FUSE estimated request ceiling estimated={estimated_request} configured={configured_request}\n{negotiated_line}"
        ));
    }

    if compatibility_line.contains("MAX_PAGES") {
        if let (Some(page_size), Some(max_pages), Some(kernel_max_request)) =
            (page_size, max_pages, kernel_max_request)
        {
            let expected = page_size
                .checked_mul(max_pages)
                .ok_or_else(|| "kernel FUSE request-byte calculation overflowed".to_string())?;
            if kernel_max_request != expected {
                return Err(format!(
                    "invalid FUSE kernel request limit page_size={page_size} max_pages={max_pages} bytes={kernel_max_request} expected={expected}\n{negotiated_line}"
                ));
            }
            if estimated_request > kernel_max_request {
                return Err(format!(
                    "estimated FUSE request ceiling exceeds kernel limit estimated={estimated_request} kernel={kernel_max_request}\n{negotiated_line}"
                ));
            }
        }
    }

    Ok(())
}

#[test]
fn write_noop() -> Result<(), String> {
    // Ten test porownuje timestampy bezposrednio przed i po identycznym write.
    // Wylacz attr cache tylko tutaj, aby stary wpis kernela po pierwszym write
    // nie wygladal jak zmiana metadanych spowodowana drugim, no-op write.
    let mounted = MountedFs::start_with_env(
        "write-noop",
        &[("FOD_METADATA_CACHE_TTL_SECONDS", "0".to_string())],
    )?;
    let suffix = unique_suffix();
    let file_path = mounted.mountpoint.join(format!("write_noop_{suffix}.txt"));
    let payload = b"payload\n";

    fs::write(&file_path, payload).map_err(|err| err.to_string())?;
    let before = metadata_times(&file_path)?;
    OpenOptions::new()
        .write(true)
        .open(&file_path)
        .map_err(|err| err.to_string())?
        .write_all(payload)
        .map_err(|err| err.to_string())?;
    let after = metadata_times(&file_path)?;

    let size = fs::metadata(&file_path)
        .map_err(|err| err.to_string())?
        .len();
    if size != payload.len() as u64 {
        return Err(format!("expected size {}, got {}", payload.len(), size));
    }
    if before != after {
        return Err(format!(
            "write noop changed metadata: before={before:?} after={after:?}"
        ));
    }
    Ok(())
}

#[test]
fn zero_length_write_is_noop() -> Result<(), String> {
    let mounted = MountedFs::start("zero-length-write")?;
    let suffix = unique_suffix();
    let dir_path = mounted
        .mountpoint
        .join(format!("zero_length_write_{suffix}"));
    let file_path = dir_path.join("payload.bin");
    let payload = b"payload";

    fs::create_dir(&dir_path).map_err(|err| err.to_string())?;
    fs::write(&file_path, payload).map_err(|err| err.to_string())?;

    let before_len = fs::metadata(&file_path)
        .map_err(|err| err.to_string())?
        .len();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| err.to_string())?;
    let fd = file.as_raw_fd();
    let offset = (before_len + 4096) as libc::off_t;
    let rc = unsafe { libc::pwrite(fd, payload.as_ptr() as *const libc::c_void, 0, offset) };
    if rc != 0 {
        return Err(format!("zero-length pwrite returned {rc}"));
    }
    drop(file);

    let after_len = fs::metadata(&file_path)
        .map_err(|err| err.to_string())?
        .len();
    if after_len != before_len {
        return Err(format!(
            "zero-length write changed file size: before={before_len} after={after_len}"
        ));
    }

    let read_back = fs::read(&file_path).map_err(|err| err.to_string())?;
    if read_back != payload {
        return Err(format!(
            "zero-length write changed file contents: {:?}",
            read_back
        ));
    }

    Ok(())
}

#[test]
fn unlink_after_write() -> Result<(), String> {
    let mounted = MountedFs::start("unlink-after-write")?;
    let suffix = unique_suffix();
    let dir_path = mounted
        .mountpoint
        .join(format!("unlink_after_write_{suffix}"));
    let file_path = dir_path.join("payload.bin");

    fs::create_dir(&dir_path).map_err(|err| err.to_string())?;
    fs::write(&file_path, b"payload").map_err(|err| err.to_string())?;
    fs::remove_file(&file_path).map_err(|err| err.to_string())?;

    if file_path.exists() {
        return Err("file still exists after unlink".to_string());
    }
    Ok(())
}

#[test]
fn unlink_promotes_remaining_hardlink() -> Result<(), String> {
    let mounted = MountedFs::start("unlink-promotes-remaining-hardlink")?;
    let suffix = unique_suffix();
    let dir_path = mounted
        .mountpoint
        .join(format!("unlink_promotes_remaining_hardlink_{suffix}"));
    let primary_path = dir_path.join("primary.bin");
    let hardlink_path = dir_path.join("hardlink.bin");
    let payload = b"hardlink payload";

    fs::create_dir(&dir_path).map_err(|err| err.to_string())?;
    fs::write(&primary_path, payload).map_err(|err| err.to_string())?;
    fs::hard_link(&primary_path, &hardlink_path).map_err(|err| err.to_string())?;
    fs::remove_file(&primary_path).map_err(|err| err.to_string())?;

    if primary_path.exists() {
        return Err("primary path still exists after unlink".to_string());
    }
    let read_back = fs::read(&hardlink_path).map_err(|err| err.to_string())?;
    if read_back != payload {
        return Err(format!(
            "hardlink lost data after primary unlink: {:?}",
            read_back
        ));
    }

    Ok(())
}

#[test]
fn multi_open_unique_handles() -> Result<(), String> {
    let mounted = MountedFs::start("multi-open-unique-handles")?;
    let suffix = unique_suffix();
    let dir_path = mounted
        .mountpoint
        .join(format!("multi_open_unique_handles_{suffix}"));
    let file_path = dir_path.join("payload.bin");

    fs::create_dir(&dir_path).map_err(|err| format!("create_dir failed: {err}"))?;
    fs::write(&file_path, b"").map_err(|err| format!("create empty file failed: {err}"))?;

    let mut fh_plain = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("open fh_plain failed: {err}"))?;
    let fh_probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("open fh_probe failed: {err}"))?;

    if fh_plain.as_raw_fd() == fh_probe.as_raw_fd() {
        return Err("handles should be independent".to_string());
    }

    fh_plain
        .write_all(b"AA")
        .map_err(|err| format!("write fh_plain failed: {err}"))?;
    fh_plain
        .flush()
        .map_err(|err| format!("flush fh_plain before append failed: {err}"))?;
    drop(fh_probe);
    drop(fh_plain);

    let mut fh_append = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("reopen fh_append failed: {err}"))?;
    fh_append
        .seek(SeekFrom::Start(2))
        .map_err(|err| format!("append seek failed: {err}"))?;
    fh_append
        .write_all(b"BB")
        .map_err(|err| format!("write fh_append failed: {err}"))?;
    fh_append
        .flush()
        .map_err(|err| format!("flush fh_append failed: {err}"))?;
    drop(fh_append);

    let mut data = Vec::new();
    fs::File::open(&file_path)
        .map_err(|err| format!("reopen failed: {err}"))?
        .read_to_end(&mut data)
        .map_err(|err| format!("read back failed: {err}"))?;
    if data != b"AABB" {
        return Err(format!(
            "unexpected data after concurrent opens: {:?}",
            data
        ));
    }
    Ok(())
}

#[test]
fn concurrent_threads_create_write_and_read_independent_files() -> Result<(), String> {
    const WORKERS: usize = 8;
    const PAYLOAD_LEN: usize = 128 * 1024;

    let mounted = MountedFs::start_with_env(
        "concurrent-threaded-file-tasks",
        &[("FOD_FUSE_EVENT_THREADS", WORKERS.to_string())],
    )?;
    let mount_log = mounted.log_tail(200);
    let event_loop = mount_log
        .lines()
        .find(|line| line.contains("FOD FUSE event loop:"))
        .ok_or_else(|| format!("missing FUSE event-loop configuration\n{}", mount_log))?;
    if !event_loop.contains(&format!("threads={WORKERS}")) {
        return Err(format!(
            "expected {WORKERS} FUSE event threads, got {event_loop}"
        ));
    }

    let suffix = unique_suffix();
    let dir_path = mounted
        .mountpoint
        .join(format!("concurrent_thread_tasks_{suffix}"));
    fs::create_dir(&dir_path).map_err(|err| format!("create task directory failed: {err}"))?;

    let barrier = Arc::new(Barrier::new(WORKERS));
    let workers = (0..WORKERS)
        .map(|worker_id| {
            let path = dir_path.join(format!("worker-{worker_id}.bin"));
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || -> Result<(std::path::PathBuf, Vec<u8>), String> {
                let marker = u8::try_from(worker_id + 1)
                    .map_err(|err| format!("invalid worker marker: {err}"))?;
                let payload = vec![marker; PAYLOAD_LEN];
                barrier.wait();

                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|err| format!("worker {worker_id} create failed: {err}"))?;
                file.write_all(&payload)
                    .map_err(|err| format!("worker {worker_id} write failed: {err}"))?;
                file.sync_all()
                    .map_err(|err| format!("worker {worker_id} sync failed: {err}"))?;
                Ok((path, payload))
            })
        })
        .collect::<Vec<_>>();

    for worker in workers {
        let (path, expected) = worker
            .join()
            .map_err(|_| "concurrent file worker panicked".to_string())??;
        let actual =
            fs::read(&path).map_err(|err| format!("read back {} failed: {err}", path.display()))?;
        if actual != expected {
            return Err(format!(
                "concurrent worker wrote incorrect data to {}: expected {} bytes, got {}",
                path.display(),
                expected.len(),
                actual.len()
            ));
        }
    }

    let shared_file_path = dir_path.join("parallel-same-file-writes.bin");
    let shared_file_size = WORKERS * PAYLOAD_LEN;
    fs::write(&shared_file_path, vec![0; shared_file_size])
        .map_err(|err| format!("initialize shared write target failed: {err}"))?;
    let mut expected_shared_file = vec![0; shared_file_size];
    let barrier = Arc::new(Barrier::new(WORKERS));
    let writers = (0..WORKERS)
        .map(|worker_id| {
            let path = shared_file_path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || -> Result<(usize, Vec<u8>), String> {
                let marker = u8::try_from(worker_id + 1)
                    .map_err(|err| format!("invalid shared-file marker: {err}"))?;
                let payload = vec![marker; PAYLOAD_LEN];
                let offset = worker_id * PAYLOAD_LEN;
                barrier.wait();
                let file = OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .map_err(|err| format!("shared-file writer {worker_id} open failed: {err}"))?;
                file.write_all_at(&payload, offset as u64).map_err(|err| {
                    format!("shared-file writer {worker_id} write_at failed: {err}")
                })?;
                file.sync_all()
                    .map_err(|err| format!("shared-file writer {worker_id} sync failed: {err}"))?;
                Ok((offset, payload))
            })
        })
        .collect::<Vec<_>>();

    for writer in writers {
        let (offset, payload) = writer
            .join()
            .map_err(|_| "shared-file writer thread panicked".to_string())??;
        expected_shared_file[offset..offset + payload.len()].copy_from_slice(&payload);
    }
    let actual_shared_file = fs::read(&shared_file_path)
        .map_err(|err| format!("read shared-file write result failed: {err}"))?;
    if actual_shared_file != expected_shared_file {
        return Err(format!(
            "parallel writes to one file produced incorrect data: expected {} bytes, got {}",
            expected_shared_file.len(),
            actual_shared_file.len()
        ));
    }

    let barrier = Arc::new(Barrier::new(WORKERS));
    let readers = (0..WORKERS)
        .map(|worker_id| {
            let path = shared_file_path.clone();
            let expected = expected_shared_file.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || -> Result<(), String> {
                barrier.wait();
                let file = fs::File::open(&path)
                    .map_err(|err| format!("shared-file reader {worker_id} open failed: {err}"))?;
                let mut actual = Vec::new();
                file.take((expected.len() + 1) as u64)
                    .read_to_end(&mut actual)
                    .map_err(|err| format!("shared-file reader {worker_id} read failed: {err}"))?;
                if actual != expected {
                    return Err(format!(
                        "shared-file reader {worker_id} observed incorrect data"
                    ));
                }
                Ok(())
            })
        })
        .collect::<Vec<_>>();
    for reader in readers {
        reader
            .join()
            .map_err(|_| "shared-file reader thread panicked".to_string())??;
    }

    let shared_path = dir_path.join("same-name-race.bin");
    let barrier = Arc::new(Barrier::new(WORKERS));
    let creators = (0..WORKERS)
        .map(|worker_id| {
            let path = shared_path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || -> Result<bool, String> {
                barrier.wait();
                match OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(mut file) => {
                        file.write_all(&[u8::try_from(worker_id + 1).unwrap()])
                            .map_err(|err| format!("creator {worker_id} write failed: {err}"))?;
                        file.sync_all()
                            .map_err(|err| format!("creator {worker_id} sync failed: {err}"))?;
                        Ok(true)
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
                    Err(err) => Err(format!("creator {worker_id} failed: {err}")),
                }
            })
        })
        .collect::<Vec<_>>();

    let mut successful_creates = 0;
    for creator in creators {
        successful_creates += usize::from(
            creator
                .join()
                .map_err(|_| "same-name create worker panicked".to_string())??,
        );
    }
    if successful_creates != 1 {
        return Err(format!(
            "expected exactly one successful concurrent create, got {successful_creates}"
        ));
    }
    let winner_payload = fs::read(&shared_path)
        .map_err(|err| format!("read same-name race result failed: {err}"))?;
    if winner_payload.len() != 1 || !(1..=WORKERS as u8).contains(&winner_payload[0]) {
        return Err(format!(
            "same-name race produced unexpected winner contents: {winner_payload:?}"
        ));
    }

    let barrier = Arc::new(Barrier::new(WORKERS));
    let namespace_workers = (0..WORKERS)
        .map(|worker_id| {
            let parent = dir_path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || -> Result<(), String> {
                let worker_dir = parent.join(format!("namespace-worker-{worker_id}"));
                let renamed_dir = parent.join(format!("renamed-worker-{worker_id}"));

                barrier.wait();
                fs::create_dir(&worker_dir)
                    .map_err(|err| format!("worker {worker_id} create directory failed: {err}"))?;
                fs::rename(&worker_dir, &renamed_dir)
                    .map_err(|err| format!("worker {worker_id} rename directory failed: {err}"))?;
                fs::remove_dir(&renamed_dir)
                    .map_err(|err| format!("worker {worker_id} rmdir failed: {err}"))?;
                Ok(())
            })
        })
        .collect::<Vec<_>>();

    for worker in namespace_workers {
        let result = worker
            .join()
            .map_err(|_| "concurrent namespace worker panicked".to_string())?;
        result.map_err(|err| format!("{err}\n{}", mounted.log_tail(200)))?;
    }

    const TASK_ITERATIONS: usize = 12;
    let barrier = Arc::new(Barrier::new(WORKERS));
    let task_workers = (0..WORKERS)
        .map(|worker_id| {
            let parent = dir_path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || -> Result<(), String> {
                barrier.wait();
                for task_id in 0..TASK_ITERATIONS {
                    let source = parent.join(format!("task-{worker_id}-{task_id}.tmp"));
                    let destination = parent.join(format!("task-{worker_id}-{task_id}.bin"));
                    let marker = u8::try_from(worker_id + 1)
                        .map_err(|err| format!("invalid task worker marker: {err}"))?;
                    let payload = vec![marker; PAYLOAD_LEN + task_id];

                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&source)
                        .map_err(|err| {
                            format!("worker {worker_id} task {task_id} create failed: {err}")
                        })?;
                    file.write_all(&payload).map_err(|err| {
                        format!("worker {worker_id} task {task_id} write failed: {err}")
                    })?;
                    file.sync_all().map_err(|err| {
                        format!("worker {worker_id} task {task_id} sync failed: {err}")
                    })?;
                    drop(file);

                    fs::rename(&source, &destination).map_err(|err| {
                        format!("worker {worker_id} task {task_id} rename failed: {err}")
                    })?;
                    let actual = fs::read(&destination).map_err(|err| {
                        format!("worker {worker_id} task {task_id} read failed: {err}")
                    })?;
                    if actual != payload {
                        return Err(format!(
                            "worker {worker_id} task {task_id} payload mismatch: expected {} bytes, got {}",
                            payload.len(),
                            actual.len()
                        ));
                    }
                    fs::remove_file(&destination).map_err(|err| {
                        format!("worker {worker_id} task {task_id} unlink failed: {err}")
                    })?;
                }
                Ok(())
            })
        })
        .collect::<Vec<_>>();

    for worker in task_workers {
        worker
            .join()
            .map_err(|_| "repeated concurrent file-task worker panicked".to_string())??;
    }

    let leftover_task_files = fs::read_dir(&dir_path)
        .map_err(|err| format!("read task directory after file tasks failed: {err}"))?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("task-")
        })
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    if !leftover_task_files.is_empty() {
        return Err(format!(
            "concurrent file tasks left entries behind: {leftover_task_files:?}"
        ));
    }

    let leftover_namespace_dirs = fs::read_dir(&dir_path)
        .map_err(|err| format!("read task directory after namespace workers failed: {err}"))?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("namespace-worker-") || name.starts_with("renamed-worker-")
        })
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    if !leftover_namespace_dirs.is_empty() {
        return Err(format!(
            "concurrent namespace workers left entries behind: {leftover_namespace_dirs:?}"
        ));
    }

    Ok(())
}

#[test]
fn concurrent_threads_unlink_and_replace_many_open_file_tasks() -> Result<(), String> {
    const WORKERS: usize = 8;
    const TASKS_PER_WORKER: usize = 4;
    const PAYLOAD_LEN: usize = 16 * 1024;

    let mounted = MountedFs::start_with_env(
        "concurrent-threaded-open-unlink-replace",
        &[("FOD_FUSE_EVENT_THREADS", WORKERS.to_string())],
    )?;
    let mount_log = mounted.log_tail(200);
    let event_loop = mount_log
        .lines()
        .find(|line| line.contains("FOD FUSE event loop:"))
        .ok_or_else(|| format!("missing FUSE event-loop configuration\n{mount_log}"))?;
    if !event_loop.contains(&format!("threads={WORKERS}")) {
        return Err(format!(
            "expected {WORKERS} FUSE event threads, got {event_loop}"
        ));
    }
    let root = mounted
        .mountpoint
        .join(format!("open-unlink-replace-{}", unique_suffix()));
    fs::create_dir(&root).map_err(|err| format!("create task directory failed: {err}"))?;

    let mut worker_tasks = (0..WORKERS)
        .map(|_| Vec::with_capacity(TASKS_PER_WORKER))
        .collect::<Vec<_>>();
    for worker_id in 0..WORKERS {
        for task_id in 0..TASKS_PER_WORKER {
            let path = root.join(format!("worker-{worker_id}-task-{task_id}.bin"));
            let old_marker = u8::try_from(worker_id + 1)
                .map_err(|err| format!("invalid old-file marker: {err}"))?;
            let replacement_marker = u8::try_from(WORKERS + worker_id + 1)
                .map_err(|err| format!("invalid replacement marker: {err}"))?;
            let old_payload = vec![old_marker; PAYLOAD_LEN + task_id];
            let replacement_payload = vec![replacement_marker; PAYLOAD_LEN + task_id];
            fs::write(&path, &old_payload)
                .map_err(|err| format!("worker {worker_id} task {task_id} seed failed: {err}"))?;
            let old_file = fs::File::open(&path)
                .map_err(|err| format!("worker {worker_id} task {task_id} open failed: {err}"))?;
            let old_inode = old_file
                .metadata()
                .map_err(|err| format!("worker {worker_id} task {task_id} fstat failed: {err}"))?
                .ino();
            worker_tasks[worker_id].push((
                path,
                old_file,
                old_inode,
                old_payload,
                replacement_payload,
            ));
        }
    }

    let barrier = Arc::new(Barrier::new(WORKERS));
    let workers = worker_tasks
        .into_iter()
        .enumerate()
        .map(|(worker_id, tasks)| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || -> Result<(), String> {
                barrier.wait();
                for (task_id, (path, old_file, old_inode, old_payload, replacement_payload)) in
                    tasks.into_iter().enumerate()
                {
                    fs::remove_file(&path).map_err(|err| {
                        format!("worker {worker_id} task {task_id} unlink failed: {err}")
                    })?;
                    fs::write(&path, &replacement_payload).map_err(|err| {
                        format!("worker {worker_id} task {task_id} replacement failed: {err}")
                    })?;

                    let mut old_actual = Vec::new();
                    old_file
                        .take((old_payload.len() + 1) as u64)
                        .read_to_end(&mut old_actual)
                        .map_err(|err| {
                            format!("worker {worker_id} task {task_id} old read failed: {err}")
                        })?;
                    if old_actual != old_payload {
                        return Err(format!(
                            "worker {worker_id} task {task_id} old descriptor read replacement data"
                        ));
                    }

                    let replacement_file = fs::File::open(&path).map_err(|err| {
                        format!("worker {worker_id} task {task_id} replacement open failed: {err}")
                    })?;
                    let replacement_inode = replacement_file
                        .metadata()
                        .map_err(|err| {
                            format!(
                                "worker {worker_id} task {task_id} replacement fstat failed: {err}"
                            )
                        })?
                        .ino();
                    if replacement_inode == old_inode {
                        return Err(format!(
                            "worker {worker_id} task {task_id} reused inode {old_inode}"
                        ));
                    }
                    let mut replacement_actual = Vec::new();
                    replacement_file
                        .take((replacement_payload.len() + 1) as u64)
                        .read_to_end(&mut replacement_actual)
                        .map_err(|err| {
                            format!(
                                "worker {worker_id} task {task_id} replacement read failed: {err}"
                            )
                        })?;
                    if replacement_actual != replacement_payload {
                        return Err(format!(
                            "worker {worker_id} task {task_id} replacement payload mismatch"
                        ));
                    }
                }
                Ok(())
            })
        })
        .collect::<Vec<_>>();

    for worker in workers {
        worker
            .join()
            .map_err(|_| "open/unlink replacement worker panicked".to_string())??;
    }

    fs::remove_dir_all(&root).map_err(|err| format!("remove task directory failed: {err}"))?;
    Ok(())
}

#[test]
fn concurrent_read_tasks_on_sparse_file_preserve_parallel_block_ranges() -> Result<(), String> {
    const READERS: usize = 8;
    const STORED_BLOCKS: usize = 16;

    let mounted = MountedFs::start_with_env(
        "concurrent-sparse-parallel-reads",
        &[
            ("FOD_FUSE_EVENT_THREADS", READERS.to_string()),
            ("FOD_WORKERS_READ", "4".to_string()),
            ("FOD_WORKERS_READ_MIN_BLOCKS", "1".to_string()),
        ],
    )?;
    let block_size = block_size_from_config()?;
    let path = mounted
        .mountpoint
        .join(format!("concurrent-sparse-read-{}.bin", unique_suffix()));
    let file_len = (STORED_BLOCKS * 2 - 1)
        .checked_mul(block_size)
        .ok_or_else(|| "sparse parallel-read file size overflow".to_string())?;
    let mut expected = vec![0; file_len];
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|err| format!("create sparse parallel-read file failed: {err}"))?;
    file.set_len(file_len as u64)
        .map_err(|err| format!("extend sparse parallel-read file failed: {err}"))?;

    for block_index in 0..STORED_BLOCKS {
        let marker = u8::try_from(block_index + 1)
            .map_err(|err| format!("invalid sparse block marker: {err}"))?;
        let block = vec![marker; block_size];
        let offset = (block_index * 2 * block_size) as u64;
        file.write_all_at(&block, offset)
            .map_err(|err| format!("write sparse block {block_index} failed: {err}"))?;
        let start = offset as usize;
        expected[start..start + block_size].copy_from_slice(&block);
    }
    file.sync_all()
        .map_err(|err| format!("sync sparse parallel-read file failed: {err}"))?;
    drop(file);

    let barrier = Arc::new(Barrier::new(READERS));
    let readers = (0..READERS)
        .map(|reader_id| {
            let path = path.clone();
            let expected = expected.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || -> Result<(), String> {
                let mut file = fs::File::open(&path)
                    .map_err(|err| format!("reader {reader_id} open failed: {err}"))?;
                barrier.wait();
                let mut actual = Vec::new();
                file.read_to_end(&mut actual)
                    .map_err(|err| format!("reader {reader_id} read failed: {err}"))?;
                if actual != expected {
                    let first_mismatch = actual
                        .iter()
                        .zip(&expected)
                        .position(|(actual, expected)| actual != expected);
                    return Err(format!(
                        "reader {reader_id} got {} bytes instead of {} with first mismatch at {first_mismatch:?}",
                        actual.len(),
                        expected.len()
                    ));
                }
                Ok(())
            })
        })
        .collect::<Vec<_>>();

    for (reader_id, reader) in readers.into_iter().enumerate() {
        reader
            .join()
            .map_err(|_| format!("sparse parallel-read worker {reader_id} panicked"))??;
    }

    Ok(())
}

#[test]
fn concurrent_file_and_directory_create_at_same_name_have_one_winner() -> Result<(), String> {
    const ROUNDS: usize = 16;
    const WORKERS: usize = 2;

    let mounted = MountedFs::start_with_env(
        "concurrent-file-directory-create-race",
        &[("FOD_FUSE_EVENT_THREADS", WORKERS.to_string())],
    )?;
    let root = mounted
        .mountpoint
        .join(format!("file-directory-races-{}", unique_suffix()));
    fs::create_dir(&root).map_err(|err| format!("create race directory failed: {err}"))?;

    for round in 0..ROUNDS {
        let name = format!("same-name-{round}");
        let file_path = root.join(&name);
        let directory_path = root.join(&name);
        let file_create_path = file_path.clone();
        let barrier = Arc::new(Barrier::new(WORKERS));

        let file_barrier = Arc::clone(&barrier);
        let file_creator = thread::spawn(move || {
            file_barrier.wait();
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file_create_path)
            {
                Ok(mut file) => {
                    file.write_all(b"file-winner")?;
                    file.sync_all()?;
                    Ok(true)
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
                Err(error) => Err(error),
            }
        });

        let directory_barrier = Arc::clone(&barrier);
        let directory_creator = thread::spawn(move || {
            directory_barrier.wait();
            match fs::create_dir(&directory_path) {
                Ok(()) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
                Err(error) => Err(error),
            }
        });

        let file_won = file_creator
            .join()
            .map_err(|_| format!("file creator panicked in round {round}"))?
            .map_err(|err| format!("file creator failed in round {round}: {err}"))?;
        let directory_won = directory_creator
            .join()
            .map_err(|_| format!("directory creator panicked in round {round}"))?
            .map_err(|err| format!("directory creator failed in round {round}: {err}"))?;

        if usize::from(file_won) + usize::from(directory_won) != 1 {
            return Err(format!(
                "round {round} expected exactly one create winner, file={file_won} directory={directory_won}"
            ));
        }

        let metadata = fs::symlink_metadata(&file_path)
            .map_err(|err| format!("inspect create-race winner in round {round}: {err}"))?;
        if file_won {
            if !metadata.file_type().is_file() {
                return Err(format!(
                    "file create winner has wrong type in round {round}"
                ));
            }
            let payload = fs::read(&file_path)
                .map_err(|err| format!("read create-race file winner in round {round}: {err}"))?;
            if payload != b"file-winner" {
                return Err(format!(
                    "file create winner has wrong contents in round {round}"
                ));
            }
        } else if !metadata.file_type().is_dir() {
            return Err(format!(
                "directory create winner has wrong type in round {round}"
            ));
        }
    }

    Ok(())
}

#[test]
fn concurrent_threads_append_complete_records_to_one_file() -> Result<(), String> {
    const WORKERS: usize = 8;
    const APPENDS_PER_WORKER: usize = 32;
    const RECORD_LEN: usize = 16;

    let mounted = MountedFs::start_with_env(
        "concurrent-threaded-append-tasks",
        &[("FOD_FUSE_EVENT_THREADS", WORKERS.to_string())],
    )?;
    let path = mounted
        .mountpoint
        .join(format!("concurrent-append-{}.bin", unique_suffix()));
    fs::write(&path, []).map_err(|err| format!("create append target failed: {err}"))?;

    let barrier = Arc::new(Barrier::new(WORKERS));
    let writers = (0..WORKERS)
        .map(|worker_id| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || -> Result<(), String> {
                let mut file = OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .map_err(|err| format!("append worker {worker_id} open failed: {err}"))?;
                barrier.wait();

                for task_id in 0..APPENDS_PER_WORKER {
                    let mut record = [0u8; RECORD_LEN];
                    record[..8].copy_from_slice(&(worker_id as u64).to_le_bytes());
                    record[8..].copy_from_slice(&(task_id as u64).to_le_bytes());
                    let written = file.write(&record).map_err(|err| {
                        format!("worker {worker_id} task {task_id} append failed: {err}")
                    })?;
                    if written != RECORD_LEN {
                        return Err(format!(
                            "worker {worker_id} task {task_id} wrote a partial record: {written}/{RECORD_LEN} bytes"
                        ));
                    }
                }

                file.sync_all()
                    .map_err(|err| format!("append worker {worker_id} sync failed: {err}"))
            })
        })
        .collect::<Vec<_>>();

    for writer in writers {
        writer
            .join()
            .map_err(|_| "concurrent append worker panicked".to_string())??;
    }

    let actual = fs::read(&path).map_err(|err| format!("read append result failed: {err}"))?;
    let expected_len = WORKERS * APPENDS_PER_WORKER * RECORD_LEN;
    if actual.len() != expected_len {
        return Err(format!(
            "concurrent append produced {} bytes; expected {expected_len}",
            actual.len()
        ));
    }

    let mut observed = vec![vec![false; APPENDS_PER_WORKER]; WORKERS];
    for (record_index, record) in actual.chunks_exact(RECORD_LEN).enumerate() {
        let worker_id = u64::from_le_bytes(record[..8].try_into().unwrap()) as usize;
        let task_id = u64::from_le_bytes(record[8..].try_into().unwrap()) as usize;
        if worker_id >= WORKERS || task_id >= APPENDS_PER_WORKER {
            return Err(format!(
                "append record {record_index} has unexpected identity ({worker_id}, {task_id})"
            ));
        }
        if std::mem::replace(&mut observed[worker_id][task_id], true) {
            return Err(format!(
                "append record ({worker_id}, {task_id}) appeared more than once"
            ));
        }
    }

    if observed.iter().flatten().any(|present| !present) {
        return Err("concurrent append omitted one or more task records".to_string());
    }

    Ok(())
}

#[test]
fn concurrent_threads_write_through_cloned_shared_offset_handles() -> Result<(), String> {
    const WORKERS: usize = 8;
    const WRITES_PER_WORKER: usize = 64;
    const RECORD_LEN: usize = 16;

    let mounted = MountedFs::start_with_env(
        "concurrent-shared-offset-writes",
        &[("FOD_FUSE_EVENT_THREADS", WORKERS.to_string())],
    )?;
    let path = mounted
        .mountpoint
        .join(format!("shared-offset-{}.bin", unique_suffix()));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|err| format!("create shared-offset file failed: {err}"))?;

    let barrier = Arc::new(Barrier::new(WORKERS));
    let writers = (0..WORKERS)
        .map(|worker_id| {
            let file = file
                .try_clone()
                .map_err(|err| format!("clone shared-offset handle failed: {err}"))?;
            let barrier = Arc::clone(&barrier);
            Ok(thread::spawn(move || -> Result<(), String> {
                let mut file = file;
                barrier.wait();
                for task_id in 0..WRITES_PER_WORKER {
                    let mut record = [0u8; RECORD_LEN];
                    record[..8].copy_from_slice(&(worker_id as u64).to_le_bytes());
                    record[8..].copy_from_slice(&(task_id as u64).to_le_bytes());
                    let written = file.write(&record).map_err(|err| {
                        format!("worker {worker_id} task {task_id} write failed: {err}")
                    })?;
                    if written != RECORD_LEN {
                        return Err(format!(
                            "worker {worker_id} task {task_id} wrote a partial record: {written}/{RECORD_LEN} bytes"
                        ));
                    }
                }
                Ok(())
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;

    for writer in writers {
        writer
            .join()
            .map_err(|_| "shared-offset writer thread panicked".to_string())??;
    }
    file.sync_all()
        .map_err(|err| format!("sync shared-offset writes failed: {err}"))?;

    let actual =
        fs::read(&path).map_err(|err| format!("read shared-offset result failed: {err}"))?;
    let expected_len = WORKERS * WRITES_PER_WORKER * RECORD_LEN;
    if actual.len() != expected_len {
        return Err(format!(
            "shared-offset writes produced {} bytes; expected {expected_len}",
            actual.len()
        ));
    }

    let mut observed = vec![vec![false; WRITES_PER_WORKER]; WORKERS];
    for (record_index, record) in actual.chunks_exact(RECORD_LEN).enumerate() {
        let worker_id = u64::from_le_bytes(record[..8].try_into().unwrap()) as usize;
        let task_id = u64::from_le_bytes(record[8..].try_into().unwrap()) as usize;
        if worker_id >= WORKERS || task_id >= WRITES_PER_WORKER {
            return Err(format!(
                "shared-offset record {record_index} has unexpected identity ({worker_id}, {task_id})"
            ));
        }
        if std::mem::replace(&mut observed[worker_id][task_id], true) {
            return Err(format!(
                "shared-offset record ({worker_id}, {task_id}) appeared more than once"
            ));
        }
    }
    if observed.iter().flatten().any(|present| !present) {
        return Err("shared-offset writes omitted one or more task records".to_string());
    }

    Ok(())
}

#[test]
fn concurrent_threads_run_multiple_copy_file_range_tasks() -> Result<(), String> {
    const WORKERS: usize = 8;
    const TASKS_PER_WORKER: usize = 4;
    const PAYLOAD_LEN: usize = 64 * 1024;

    let mounted = MountedFs::start_with_env(
        "concurrent-threaded-copy-file-range-tasks",
        &[("FOD_FUSE_EVENT_THREADS", WORKERS.to_string())],
    )?;
    let mount_log = mounted.log_tail(200);
    let event_loop = mount_log
        .lines()
        .find(|line| line.contains("FOD FUSE event loop:"))
        .ok_or_else(|| format!("missing FUSE event-loop configuration\n{}", mount_log))?;
    if !event_loop.contains(&format!("threads={WORKERS}")) {
        return Err(format!(
            "expected {WORKERS} FUSE event threads, got {event_loop}"
        ));
    }
    let root = mounted
        .mountpoint
        .join(format!("copy-tasks-{}", unique_suffix()));
    fs::create_dir(&root).map_err(|err| format!("create copy task directory failed: {err}"))?;

    let mut tasks = Vec::with_capacity(WORKERS * TASKS_PER_WORKER);
    for worker_id in 0..WORKERS {
        for task_id in 0..TASKS_PER_WORKER {
            let source = root.join(format!("source-{worker_id}-{task_id}.bin"));
            let destination = root.join(format!("destination-{worker_id}-{task_id}.bin"));
            let marker = u8::try_from(worker_id * TASKS_PER_WORKER + task_id + 1)
                .map_err(|err| format!("invalid copy task marker: {err}"))?;
            let payload = vec![marker; PAYLOAD_LEN + task_id];
            fs::write(&source, &payload).map_err(|err| {
                format!(
                    "seed copy task {worker_id}/{task_id} failed: {err}\n{}",
                    mounted.log_tail(200)
                )
            })?;
            tasks.push((source, destination, payload, worker_id, task_id));
        }
    }

    let barrier = Arc::new(Barrier::new(tasks.len()));
    let workers = tasks
        .into_iter()
        .map(|(source, destination, expected, worker_id, task_id)| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || -> Result<(std::path::PathBuf, Vec<u8>), String> {
                barrier.wait();
                let input = fs::File::open(&source).map_err(|err| {
                    format!("copy worker {worker_id} task {task_id} open source failed: {err}")
                })?;
                let output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination)
                    .map_err(|err| {
                        format!("copy worker {worker_id} task {task_id} create destination failed: {err}")
                    })?;
                let mut input_offset = 0i64;
                let mut output_offset = 0i64;

                loop {
                    if output_offset < 0 || output_offset as usize > expected.len() {
                        return Err(format!(
                            "copy worker {worker_id} task {task_id} produced an invalid offset: {output_offset}"
                        ));
                    }
                    let remaining = expected.len() - output_offset as usize;
                    if remaining == 0 {
                        break;
                    }
                    let copied = unsafe {
                        libc::copy_file_range(
                            input.as_raw_fd(),
                            &mut input_offset,
                            output.as_raw_fd(),
                            &mut output_offset,
                            remaining,
                            0,
                        )
                    };
                    if copied < 0 {
                        return Err(format!(
                            "copy worker {worker_id} task {task_id} copy_file_range failed: {}",
                            io::Error::last_os_error()
                        ));
                    }
                    if copied == 0 {
                        break;
                    }
                }

                if output_offset as usize != expected.len() {
                    return Err(format!(
                        "copy worker {worker_id} task {task_id} copied {} bytes, expected {}",
                        output_offset,
                        expected.len()
                    ));
                }
                output.sync_all().map_err(|err| {
                    format!("copy worker {worker_id} task {task_id} sync failed: {err}")
                })?;
                Ok((destination, expected))
            })
        })
        .collect::<Vec<_>>();

    for worker in workers {
        let (destination, expected) = worker
            .join()
            .map_err(|_| "concurrent copy task worker panicked".to_string())??;
        let actual = fs::read(&destination)
            .map_err(|err| format!("read copied task {} failed: {err}", destination.display()))?;
        if actual != expected {
            return Err(format!(
                "copy task {} payload mismatch: expected {} bytes, got {}",
                destination.display(),
                expected.len(),
                actual.len()
            ));
        }
    }

    Ok(())
}

#[test]
fn concurrent_threads_rename_one_source_to_distinct_destinations() -> Result<(), String> {
    const ROUNDS: usize = 16;
    const WORKERS: usize = 2;

    let mounted = MountedFs::start_with_env(
        "concurrent-threaded-rename-race",
        &[("FOD_FUSE_EVENT_THREADS", WORKERS.to_string())],
    )?;
    let root = mounted
        .mountpoint
        .join(format!("rename-races-{}", unique_suffix()));
    fs::create_dir(&root).map_err(|err| format!("create rename race directory failed: {err}"))?;

    for round in 0..ROUNDS {
        let source = root.join(format!("source-{round}.bin"));
        let destinations = [
            root.join(format!("destination-{round}-a.bin")),
            root.join(format!("destination-{round}-b.bin")),
        ];
        let payload = format!("rename-race-round-{round}").into_bytes();
        fs::write(&source, &payload)
            .map_err(|err| format!("seed rename race source failed: {err}"))?;

        let barrier = Arc::new(Barrier::new(WORKERS));
        let renamers = destinations
            .iter()
            .cloned()
            .map(|destination| {
                let source = source.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    fs::rename(&source, &destination)
                })
            })
            .collect::<Vec<_>>();

        let mut successful_renames = 0;
        for renamer in renamers {
            match renamer
                .join()
                .map_err(|_| format!("rename race worker panicked in round {round}"))?
            {
                Ok(()) => successful_renames += 1,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(format!(
                        "rename race failed unexpectedly in round {round}: {err}"
                    ));
                }
            }
        }

        if successful_renames != 1 {
            return Err(format!(
                "expected exactly one successful rename in round {round}, got {successful_renames}"
            ));
        }

        if source.exists() {
            return Err(format!("rename race source still exists in round {round}"));
        }
        let mut present_destinations = 0;
        for destination in &destinations {
            match fs::read(destination) {
                Ok(actual) => {
                    present_destinations += 1;
                    if actual != payload {
                        return Err(format!(
                            "rename race destination {} has incorrect contents in round {round}",
                            destination.display()
                        ));
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(format!(
                        "read rename race destination {} failed: {err}",
                        destination.display()
                    ));
                }
            }
        }
        if present_destinations != 1 {
            return Err(format!(
                "expected exactly one destination in round {round}, found {present_destinations}"
            ));
        }
    }

    Ok(())
}

#[test]
fn rename_noreplace_is_atomic_and_unsupported_flags_fail() -> Result<(), String> {
    const WORKERS: usize = 2;
    let mounted = MountedFs::start_with_env(
        "rename-noreplace-atomicity",
        &[("FOD_FUSE_EVENT_THREADS", WORKERS.to_string())],
    )?;
    let root = mounted
        .mountpoint
        .join(format!("rename-noreplace-{}", unique_suffix()));
    fs::create_dir(&root).map_err(|err| format!("create rename test directory failed: {err}"))?;

    let existing_source = root.join("existing-source");
    let existing_target = root.join("existing-target");
    fs::write(&existing_source, b"source")
        .map_err(|err| format!("seed no-replace source failed: {err}"))?;
    fs::write(&existing_target, b"target")
        .map_err(|err| format!("seed no-replace target failed: {err}"))?;
    let error = renameat2_with_flags(&existing_source, &existing_target, libc::RENAME_NOREPLACE)
        .expect_err("RENAME_NOREPLACE replaced an existing target");
    if error.raw_os_error() != Some(libc::EEXIST) {
        return Err(format!(
            "RENAME_NOREPLACE returned the wrong errno: {error}"
        ));
    }
    if fs::read(&existing_source).map_err(|err| err.to_string())? != b"source"
        || fs::read(&existing_target).map_err(|err| err.to_string())? != b"target"
    {
        return Err("failed RENAME_NOREPLACE changed source or target contents".to_string());
    }

    let same_path_error =
        renameat2_with_flags(&existing_source, &existing_source, libc::RENAME_NOREPLACE)
            .expect_err("same-path RENAME_NOREPLACE unexpectedly succeeded");
    if same_path_error.raw_os_error() != Some(libc::EEXIST) {
        return Err(format!(
            "same-path RENAME_NOREPLACE returned the wrong errno: {same_path_error}"
        ));
    }

    let exchange_error =
        renameat2_with_flags(&existing_source, &existing_target, libc::RENAME_EXCHANGE)
            .expect_err("unsupported RENAME_EXCHANGE unexpectedly succeeded");
    if exchange_error.raw_os_error() != Some(libc::EINVAL) {
        return Err(format!(
            "unsupported RENAME_EXCHANGE returned the wrong errno: {exchange_error}"
        ));
    }

    let movable_source = root.join("movable-source");
    let missing_target = root.join("missing-target");
    fs::write(&movable_source, b"move-me")
        .map_err(|err| format!("seed missing-target source failed: {err}"))?;
    renameat2_with_flags(&movable_source, &missing_target, libc::RENAME_NOREPLACE)
        .map_err(|err| format!("RENAME_NOREPLACE of missing target failed: {err}"))?;
    if !missing_target.exists() || movable_source.exists() {
        return Err("RENAME_NOREPLACE did not move source to absent target".to_string());
    }

    let directory_source = root.join("cycle-source");
    let existing_child = directory_source.join("existing-child");
    fs::create_dir_all(&existing_child)
        .map_err(|err| format!("seed existing directory-cycle target failed: {err}"))?;
    let existing_child_error =
        renameat2_with_flags(&directory_source, &existing_child, libc::RENAME_NOREPLACE)
            .expect_err("RENAME_NOREPLACE replaced an existing directory-cycle target");
    if existing_child_error.raw_os_error() != Some(libc::EEXIST) {
        return Err(format!(
            "directory-cycle RENAME_NOREPLACE returned the wrong errno: {existing_child_error}"
        ));
    }
    fs::remove_dir(&existing_child)
        .map_err(|err| format!("remove existing cycle target failed: {err}"))?;
    let missing_child_error =
        renameat2_with_flags(&directory_source, &existing_child, libc::RENAME_NOREPLACE)
            .expect_err("RENAME_NOREPLACE moved a directory inside itself");
    if missing_child_error.raw_os_error() != Some(libc::EINVAL) {
        return Err(format!(
            "directory-cycle RENAME_NOREPLACE returned the wrong errno: {missing_child_error}"
        ));
    }
    if !directory_source.is_dir() || existing_child.exists() {
        return Err("directory-cycle RENAME_NOREPLACE changed the namespace".to_string());
    }

    let sources = [root.join("racer-a"), root.join("racer-b")];
    let target = root.join("racer-target");
    fs::write(&sources[0], b"A").map_err(|err| format!("seed racer A failed: {err}"))?;
    fs::write(&sources[1], b"B").map_err(|err| format!("seed racer B failed: {err}"))?;
    let barrier = Arc::new(Barrier::new(WORKERS));
    let racers = sources
        .iter()
        .cloned()
        .map(|source| {
            let target = target.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                renameat2_with_flags(&source, &target, libc::RENAME_NOREPLACE)
            })
        })
        .collect::<Vec<_>>();

    let outcomes = racers
        .into_iter()
        .map(|racer| {
            racer
                .join()
                .map_err(|_| "RENAME_NOREPLACE racer panicked".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let winners = outcomes.iter().filter(|result| result.is_ok()).count();
    let exists_failures = outcomes
        .iter()
        .filter(|result| {
            result.as_ref().err().and_then(io::Error::raw_os_error) == Some(libc::EEXIST)
        })
        .count();
    if winners != 1 || exists_failures != 1 {
        return Err(format!(
            "concurrent RENAME_NOREPLACE expected one winner and one EEXIST, got {outcomes:?}"
        ));
    }
    let winner = fs::read(&target).map_err(|err| format!("read race winner failed: {err}"))?;
    if winner != b"A" && winner != b"B" {
        return Err(format!(
            "concurrent RENAME_NOREPLACE corrupted winner: {winner:?}"
        ));
    }
    let remaining_sources = sources.iter().filter(|source| source.exists()).count();
    if remaining_sources != 1 {
        return Err(format!(
            "concurrent RENAME_NOREPLACE left {remaining_sources} source files"
        ));
    }

    Ok(())
}

#[test]
fn concurrent_write_and_truncate_leave_a_serializable_file() -> Result<(), String> {
    const ROUNDS: usize = 24;
    const WORKERS: usize = 2;

    let mounted = MountedFs::start_with_env(
        "concurrent-write-truncate-race",
        &[("FOD_FUSE_EVENT_THREADS", WORKERS.to_string())],
    )?;

    for round in 0..ROUNDS {
        let path = mounted.mountpoint.join(format!(
            "write-truncate-race-{}-{round}.bin",
            unique_suffix()
        ));
        fs::write(&path, b"seed")
            .map_err(|err| format!("seed write/truncate race file failed: {err}"))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|err| format!("open write/truncate race file failed: {err}"))?;
        let writer_file = file
            .try_clone()
            .map_err(|err| format!("clone race writer handle failed: {err}"))?;
        let truncate_file = file
            .try_clone()
            .map_err(|err| format!("clone race truncate handle failed: {err}"))?;
        let barrier = Arc::new(Barrier::new(WORKERS));
        let writer_barrier = Arc::clone(&barrier);
        let writer = thread::spawn(move || -> Result<(), String> {
            writer_barrier.wait();
            writer_file
                .write_at(b"AB", 0)
                .map_err(|err| format!("concurrent pwrite failed: {err}"))
                .and_then(|written| {
                    if written == 2 {
                        Ok(())
                    } else {
                        Err(format!("concurrent pwrite was short: {written}/2"))
                    }
                })
        });
        let truncate_barrier = Arc::clone(&barrier);
        let truncater = thread::spawn(move || -> Result<(), String> {
            truncate_barrier.wait();
            truncate_file
                .set_len(1)
                .map_err(|err| format!("concurrent ftruncate failed: {err}"))
        });

        let write_result = writer
            .join()
            .map_err(|_| format!("write worker panicked in round {round}"))?;
        if let Err(err) = write_result {
            return Err(format!("{err}\n{}", mounted.log_tail(120)));
        }
        let truncate_result = truncater
            .join()
            .map_err(|_| format!("truncate worker panicked in round {round}"))?;
        if let Err(err) = truncate_result {
            return Err(format!("{err}\n{}", mounted.log_tail(120)));
        }
        file.sync_all()
            .map_err(|err| format!("sync race file failed in round {round}: {err}"))?;

        let actual =
            fs::read(&path).map_err(|err| format!("read write/truncate result failed: {err}"))?;
        if actual != b"A" && actual != b"AB" {
            return Err(format!(
                "round {round} produced non-serializable write/truncate result: {actual:?}"
            ));
        }
    }

    Ok(())
}

#[test]
fn cross_mount_append_uses_fresh_kernel_file_size() -> Result<(), String> {
    for (mode, direct_io) in [("buffered", "0"), ("direct-io", "1")] {
        verify_cross_mount_append_mode(mode, direct_io)?;
    }
    Ok(())
}

fn verify_cross_mount_append_mode(mode: &str, direct_io: &str) -> Result<(), String> {
    const REQUESTED_ATTR_TTL_SECONDS: &str = "120";
    const INITIAL: &[u8] = b"A";

    let extra_env = [
        (
            "FOD_ATTR_TIMEOUT_SECONDS",
            REQUESTED_ATTR_TTL_SECONDS.to_string(),
        ),
        ("FOD_FOPEN_DIRECT_IO", direct_io.to_string()),
    ];
    let mount_a = MountedFs::start_with_env(&format!("cross-mount-append-a-{mode}"), &extra_env)?;
    let mount_b = MountedFs::start_with_env(&format!("cross-mount-append-b-{mode}"), &extra_env)?;
    let path_a = mount_a
        .mountpoint
        .join(format!("cross-mount-append-{}.bin", unique_suffix()));
    let path_b = mount_b.mountpoint.join(
        path_a
            .file_name()
            .ok_or_else(|| "append test path has no file name".to_string())?,
    );

    for (label, mounted) in [("A", &mount_a), ("B", &mount_b)] {
        let log = mounted.log_tail(100);
        let cache_line = log
            .lines()
            .find(|line| line.contains("FOD FUSE reply cache TTLs:"))
            .ok_or_else(|| format!("mount {label} has no FUSE cache TTL log line\n{log}"))?;
        if !cache_line.contains("attr=0s") {
            return Err(format!(
                "writable mount {label} kept a positive attribute TTL despite cross-mount append requirements: {cache_line}"
            ));
        }
    }

    fs::write(&path_a, INITIAL).map_err(|err| format!("create append target failed: {err}"))?;
    let cached_size = fs::metadata(&path_b)
        .map_err(|err| format!("prime mount B file attributes failed: {err}"))?
        .len();
    if cached_size != INITIAL.len() as u64 {
        return Err(format!("mount B observed initial size {cached_size}"));
    }

    let mut writer_a = OpenOptions::new()
        .append(true)
        .open(&path_a)
        .map_err(|err| format!("mount A append open failed: {err}"))?;
    writer_a
        .write_all(b"-A")
        .map_err(|err| format!("mount A append failed: {err}"))?;
    writer_a
        .sync_all()
        .map_err(|err| format!("mount A append sync failed: {err}"))?;
    drop(writer_a);

    let mut writer_b = OpenOptions::new()
        .append(true)
        .open(&path_b)
        .map_err(|err| format!("mount B append open failed: {err}"))?;
    writer_b
        .write_all(b"-B")
        .map_err(|err| format!("mount B append failed: {err}"))?;
    writer_b
        .sync_all()
        .map_err(|err| format!("mount B append sync failed: {err}"))?;
    drop(writer_b);

    let actual = fs::read(&path_b).map_err(|err| format!("read append result failed: {err}"))?;
    if actual != b"A-A-B" {
        return Err(format!(
            "cross-mount append corrupted file: expected {:?}, got {:?}",
            b"A-A-B", actual
        ));
    }

    let mount_b_log = mount_b.log_tail(200);
    let write_line = mount_b_log
        .lines()
        .find(|line| line.contains("op=write ino=") && line.contains("len=2"))
        .ok_or_else(|| format!("mount B did not log its FUSE write\n{mount_b_log}"))?;
    if !write_line.contains("offset=3") {
        return Err(format!(
            "mount B sent a stale O_APPEND offset instead of current EOF: {write_line}"
        ));
    }

    Ok(())
}

#[test]
fn create_file_and_directory_parent_missing() -> Result<(), String> {
    let mounted = MountedFs::start_without_init("create-parent-missing")?;
    let suffix = unique_suffix();
    let missing_parent = mounted.mountpoint.join(format!("missing-parent-{suffix}"));
    let nested_file = missing_parent.join("file");
    let nested_dir = missing_parent.join("child");

    let err = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&nested_file)
        .expect_err("file create unexpectedly created a missing parent");
    if err.kind() != std::io::ErrorKind::NotFound {
        return Err(format!("expected file create ENOENT/NotFound, got {err}"));
    }

    let err = fs::create_dir(&nested_dir).expect_err("mkdir unexpectedly created missing parents");
    if err.kind() != std::io::ErrorKind::NotFound {
        return Err(format!("expected mkdir ENOENT/NotFound, got {err}"));
    }

    if missing_parent.exists() {
        return Err("missing parent should not have been created".to_string());
    }

    Ok(())
}

#[test]
fn concurrent_parent_directory_create_and_child_entry_create_are_serializable() -> Result<(), String>
{
    const ROUNDS: usize = 24;

    let mounted = MountedFs::start_with_env(
        "concurrent-parent-child-create",
        &[("FOD_FUSE_EVENT_THREADS", "2".to_string())],
    )?;
    let root = mounted
        .mountpoint
        .join(format!("parent-child-create-{}", unique_suffix()));
    fs::create_dir(&root).map_err(|err| format!("create test root failed: {err}"))?;

    for child_kind in ["file", "directory"] {
        for round in 0..ROUNDS {
            let parent = root.join(format!("parent-{child_kind}-{round}"));
            let child = parent.join("child-entry");
            let barrier = Arc::new(Barrier::new(3));

            let mkdir_parent = parent.clone();
            let mkdir_barrier = Arc::clone(&barrier);
            let mkdir_worker = thread::spawn(move || {
                mkdir_barrier.wait();
                fs::create_dir(&mkdir_parent)
            });

            let create_child = child.clone();
            let create_barrier = Arc::clone(&barrier);
            let child_is_directory = child_kind == "directory";
            let child_worker = thread::spawn(move || {
                create_barrier.wait();
                if child_is_directory {
                    fs::create_dir(&create_child)
                } else {
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&create_child)
                        .map(drop)
                }
            });

            barrier.wait();
            mkdir_worker
                .join()
                .map_err(|_| format!("mkdir worker panicked for {child_kind} round {round}"))?
                .map_err(|err| {
                    format!("mkdir parent failed for {child_kind} round {round}: {err}")
                })?;
            let child_result = child_worker.join().map_err(|_| {
                format!("child-create worker panicked for {child_kind} round {round}")
            })?;

            let child_created = match child_result {
                Ok(()) => true,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
                Err(err) => {
                    return Err(format!(
                        "{child_kind} create returned unexpected error in round {round}: {err}"
                    ));
                }
            };

            if !parent.is_dir() {
                return Err(format!(
                    "parent directory missing for {child_kind} round {round}"
                ));
            }
            let child_metadata = fs::symlink_metadata(&child);
            if child_created {
                let metadata = child_metadata.map_err(|err| {
                    format!("created {child_kind} missing after round {round}: {err}")
                })?;
                if metadata.is_dir() != (child_kind == "directory") {
                    return Err(format!(
                        "created child has wrong type for {child_kind} round {round}"
                    ));
                }
            } else if !matches!(child_metadata, Err(ref err) if err.kind() == std::io::ErrorKind::NotFound)
            {
                return Err(format!(
                    "child path should be absent after ENOENT for {child_kind} round {round}: {child_metadata:?}"
                ));
            }
        }
    }

    Ok(())
}

#[test]
fn truncate_rename() -> Result<(), String> {
    let mounted = MountedFs::start_without_init("truncate-rename")?;
    let suffix = unique_suffix();
    let dir_path = mounted.mountpoint.join(format!("truncate_{suffix}"));
    let file_path = dir_path.join("data.txt");
    let renamed_path = dir_path.join("data-renamed.txt");
    let payload = b"abcdef123456";

    fs::create_dir(&dir_path).map_err(|err| err.to_string())?;
    fs::write(&file_path, payload).map_err(|err| err.to_string())?;
    fs::rename(&file_path, &renamed_path).map_err(|err| err.to_string())?;

    if fs::read(&renamed_path).map_err(|err| err.to_string())? != payload {
        return Err("rename/read mismatch".to_string());
    }

    let fh = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&renamed_path)
        .map_err(|err| err.to_string())?;
    fh.set_len(4).map_err(|err| err.to_string())?;
    drop(fh);

    if fs::read(&renamed_path).map_err(|err| err.to_string())? != payload[..4] {
        return Err("truncate/read mismatch".to_string());
    }

    if file_path.exists() {
        return Err("old path still opens after rename".to_string());
    }

    Ok(())
}

#[test]
fn block_read_range() -> Result<(), String> {
    let mounted = MountedFs::start_without_init("block-read")?;
    let suffix = unique_suffix();
    let dir_path = mounted.mountpoint.join(format!("block_read_{suffix}"));
    let file_path = dir_path.join("payload.bin");
    let block_size = block_size_from_config()?;
    let payload_size = (block_size * 3) + 321;
    let mut pattern = Vec::with_capacity(payload_size);
    while pattern.len() < payload_size {
        pattern.extend_from_slice(b"0123456789abcdef");
    }
    pattern.truncate(payload_size);

    fs::create_dir(&dir_path).map_err(|err| err.to_string())?;
    fs::write(&file_path, &pattern).map_err(|err| err.to_string())?;

    let mut fh = OpenOptions::new()
        .read(true)
        .open(&file_path)
        .map_err(|err| err.to_string())?;

    let offset = block_size - 7;
    let size = block_size + 33;
    fh.seek(SeekFrom::Start(offset as u64))
        .map_err(|err| err.to_string())?;
    let mut chunk = vec![0_u8; size];
    let read = fh.read(&mut chunk).map_err(|err| err.to_string())?;
    chunk.truncate(read);
    if chunk != pattern[offset..offset + read] {
        return Err("partial read mismatch".to_string());
    }

    let tail_offset = pattern.len().saturating_sub(17);
    fh.seek(SeekFrom::Start(tail_offset as u64))
        .map_err(|err| err.to_string())?;
    let mut tail = Vec::new();
    fh.read_to_end(&mut tail).map_err(|err| err.to_string())?;
    if tail != pattern[tail_offset..] {
        return Err("tail read mismatch".to_string());
    }

    Ok(())
}

#[test]
fn noatime_direct_uncached_read_uses_fused_postgres_query() -> Result<(), String> {
    let mounted = MountedFs::start_with_env(
        "noatime-fused-primary-read",
        &[
            ("FOD_ATIME_POLICY", "noatime".to_string()),
            ("FOD_FOPEN_DIRECT_IO", "1".to_string()),
            ("FOD_READ_CACHE_BLOCKS", "0".to_string()),
            ("FOD_READ_AHEAD_BLOCKS", "0".to_string()),
            ("FOD_SEQUENTIAL_READ_AHEAD_BLOCKS", "0".to_string()),
            ("FOD_DIRECT_IO_READ_PREFETCH_BLOCKS", "0".to_string()),
            ("FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS", "0".to_string()),
        ],
    )?;
    let suffix = unique_suffix();
    let file_path = mounted
        .mountpoint
        .join(format!("fused-primary-read-{suffix}.bin"));
    let block_size = block_size_from_config()?.max(1);
    let target_len = block_size * 3 + 137;
    let mut payload = Vec::with_capacity(target_len);
    while payload.len() < target_len {
        payload.extend_from_slice(b"FOD-3.3.12-fused-primary-read-");
    }
    payload.truncate(target_len);
    fs::write(&file_path, &payload).map_err(|err| err.to_string())?;
    let mut file = OpenOptions::new()
        .read(true)
        .open(&file_path)
        .map_err(|err| err.to_string())?;
    let mut actual = Vec::new();
    file.read_to_end(&mut actual)
        .map_err(|err| err.to_string())?;
    if actual != payload {
        return Err(format!(
            "fused read payload mismatch got={} expected={}",
            actual.len(),
            payload.len()
        ));
    }
    let log = mounted.log_tail(1000);
    if !log.contains("atime_policy=NoAtime") {
        return Err(format!(
            "noatime was not applied by test bootstrap
{log}"
        ));
    }
    if !log.contains("read fast path fused_block_range_with_size") {
        return Err(format!(
            "fused primary read fast path missing from log
{log}"
        ));
    }
    Ok(())
}
