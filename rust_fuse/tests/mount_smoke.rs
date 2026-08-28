// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use support::{block_size_from_config, unique_suffix, MountedFs};

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
    let compatibility_line = log
        .lines()
        .find(|line| line.contains("FOD FUSE compatibility:"))
        .ok_or_else(|| format!("missing FUSE compatibility log line\n{log}"))?;
    let negotiated_line = log
        .lines()
        .find(|line| line.contains("FOD FUSE negotiated:"))
        .ok_or_else(|| format!("missing FUSE negotiated log line\n{log}"))?;

    println!("{compatibility_line}");
    println!("{negotiated_line}");

    for field in [
        "FOD FUSE compatibility:",
        "fuser=0.18.0",
        "userspace_protocol_max=7.40",
        "kernel_protocol=",
        "negotiated_protocol=",
        "available_capabilities=",
        "fod_requested_capabilities=[POSIX_LOCKS,FLOCK_LOCKS]",
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
    let mounted = MountedFs::start("write-noop")?;
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
fn mkdir_parent_missing() -> Result<(), String> {
    let mounted = MountedFs::start_without_init("mkdir-parent-missing")?;
    let suffix = unique_suffix();
    let missing_parent = mounted.mountpoint.join(format!("missing-parent-{suffix}"));
    let nested_dir = missing_parent.join("child");

    let err = fs::create_dir(&nested_dir).expect_err("mkdir unexpectedly created missing parents");
    if err.kind() != std::io::ErrorKind::NotFound {
        return Err(format!("expected ENOENT/NotFound, got {err}"));
    }

    if missing_parent.exists() {
        return Err("missing parent should not have been created".to_string());
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
