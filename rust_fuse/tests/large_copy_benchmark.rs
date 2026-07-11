// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::env;
use std::fs::{self, File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::time::Instant;

use support::{
    checked_payload_len, db_repo, parse_size_bytes, repeating_payload, resolve_file_id,
    unique_suffix, MountedFs,
};

fn copy_file_range_all(
    src: &File,
    dst: &File,
    mut len: usize,
    request_size: usize,
) -> Result<usize, String> {
    let mut copied = 0usize;
    while len > 0 {
        let chunk = len.min(request_size.max(1));
        let ret = unsafe {
            libc::copy_file_range(
                src.as_raw_fd(),
                std::ptr::null_mut(),
                dst.as_raw_fd(),
                std::ptr::null_mut(),
                chunk,
                0,
            )
        };
        if ret < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if ret == 0 {
            break;
        }
        let moved = usize::try_from(ret).map_err(|err| err.to_string())?;
        copied += moved;
        len = len.saturating_sub(moved);
    }
    Ok(copied)
}

#[test]
fn large_copy_benchmark() -> Result<(), String> {
    let mounted = MountedFs::start("large-copy-benchmark")?;
    let suffix = unique_suffix();
    let block_size =
        parse_size_bytes(&env::var("LARGE_COPY_BLOCK_SIZE").unwrap_or_else(|_| "4M".to_string()))?;
    let block_count = env::var("LARGE_COPY_BLOCK_COUNT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(16);
    let sync_mode = env::var("LARGE_COPY_SYNC")
        .ok()
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(false);
    let payload = repeating_payload(
        b"fod-large-copy-",
        checked_payload_len(block_size, block_count)?,
    );
    let request_size = match env::var("LARGE_COPY_REQUEST_SIZE") {
        Ok(value) if value.trim().eq_ignore_ascii_case("full") => payload.len(),
        Ok(value) => parse_size_bytes(&value)?,
        Err(_) => 4 * 1024 * 1024,
    };
    let expect_shared_object = env::var("LARGE_COPY_EXPECT_SHARED_OBJECT")
        .ok()
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(false);
    let dir_path = mounted.mountpoint.join(format!("large-copy-{suffix}"));
    let src_path = dir_path.join("src.bin");
    let dst_path = dir_path.join("dst.bin");

    fs::create_dir(&dir_path).map_err(|err| format!("create_dir failed: {err}"))?;
    fs::write(&src_path, &payload).map_err(|err| format!("write src failed: {err}"))?;

    let src_fh = File::open(&src_path).map_err(|err| format!("open src failed: {err}"))?;
    let dst_fh = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&dst_path)
        .map_err(|err| format!("open dst failed: {err}"))?;

    let start = Instant::now();
    let copied = copy_file_range_all(&src_fh, &dst_fh, payload.len(), request_size)?;
    if copied != payload.len() {
        return Err(format!(
            "copy_file_range copied {copied} of {}",
            payload.len()
        ));
    }
    if sync_mode {
        dst_fh
            .sync_all()
            .map_err(|err| format!("sync_all failed: {err}"))?;
    }
    drop(dst_fh);
    let elapsed = start.elapsed().as_secs_f64();

    let read_back = fs::read(&dst_path).map_err(|err| format!("read_back failed: {err}"))?;
    if read_back != payload {
        return Err("large copy payload mismatch".to_string());
    }
    if expect_shared_object {
        let repo = db_repo()?;
        let src_file_id = resolve_file_id(&repo, &mounted.mountpoint, &src_path)?;
        let dst_file_id = resolve_file_id(&repo, &mounted.mountpoint, &dst_path)?;
        let src_object_id = repo
            .file_data_object_id(src_file_id)?
            .ok_or_else(|| "source data object is missing".to_string())?;
        let dst_object_id = repo
            .file_data_object_id(dst_file_id)?
            .ok_or_else(|| "destination data object is missing".to_string())?;
        if src_object_id != dst_object_id {
            let copy_log = fs::read_to_string(&mounted.log_path)
                .unwrap_or_default()
                .lines()
                .filter(|line| line.to_ascii_lowercase().contains("copy_file_range"))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(format!(
                "whole-file copy did not adopt source data object: src={src_object_id} dst={dst_object_id}\n{copy_log}"
            ));
        }
    }

    let throughput_mib_s = if elapsed > 0.0 {
        (payload.len() as f64 / 1024.0 / 1024.0) / elapsed
    } else {
        0.0
    };
    println!(
        "OK large-copy-benchmark bytes={} request_size={} shared_object={} elapsed_s={elapsed:.6} throughput_mib_s={throughput_mib_s:.2}",
        payload.len(),
        request_size,
        expect_shared_object,
    );

    Ok(())
}
