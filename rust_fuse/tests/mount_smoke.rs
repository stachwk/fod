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

#[test]
fn reports_negotiated_fuse_compatibility() -> Result<(), String> {
    let mounted = MountedFs::start("fuse-compatibility")?;
    let log = mounted.log_tail(200);
    let compatibility_line = log
        .lines()
        .find(|line| line.contains("FOD FUSE compatibility:"))
        .ok_or_else(|| format!("missing FUSE compatibility log line\n{log}"))?;
    println!("{compatibility_line}");
    let required_fields = [
        "FOD FUSE compatibility:",
        "fuser=0.17.0",
        "userspace_protocol_max=7.40",
        "kernel_protocol=",
        "negotiated_protocol=",
        "available_capabilities=",
        "fod_requested_capabilities=[POSIX_LOCKS,FLOCK_LOCKS]",
        "fod_enabled_capabilities=",
        "max_write=unavailable",
        "max_readahead=unavailable",
        "max_background=unavailable",
        "congestion_threshold=unavailable",
    ];
    for field in required_fields {
        if !log.contains(field) {
            return Err(format!("missing FUSE compatibility field {field:?}\n{log}"));
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

    if fs::read(&renamed_path).map_err(|err| err.to_string())? != &payload[..4] {
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
