// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::env;
use std::fs::{self, File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::time::Instant;

use support::{unique_suffix, MountedFs};

fn parse_bytes(value: &str) -> Result<usize, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("empty size value".to_string());
    }
    let (number, multiplier) = match value.chars().last().map(|ch| ch.to_ascii_lowercase()) {
        Some('k') => (&value[..value.len() - 1], 1024usize),
        Some('m') => (&value[..value.len() - 1], 1024usize * 1024),
        Some('g') => (&value[..value.len() - 1], 1024usize * 1024 * 1024),
        _ => (value, 1usize),
    };
    let base = number
        .parse::<usize>()
        .map_err(|err| format!("failed to parse size {value:?}: {err}"))?;
    base.checked_mul(multiplier)
        .ok_or_else(|| format!("size overflow for {value:?}"))
}

fn build_payload(block_size: usize, block_count: usize) -> Vec<u8> {
    let total = block_size.saturating_mul(block_count);
    let mut payload = Vec::with_capacity(total);
    while payload.len() < total {
        payload.extend_from_slice(b"fod-large-copy-");
    }
    payload.truncate(total);
    payload
}

fn copy_file_range_all(src: &File, dst: &File, mut len: usize) -> Result<usize, String> {
    let mut copied = 0usize;
    while len > 0 {
        let chunk = len.min(4 * 1024 * 1024);
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
        parse_bytes(&env::var("LARGE_COPY_BLOCK_SIZE").unwrap_or_else(|_| "4M".to_string()))?;
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
    let payload = build_payload(block_size, block_count);
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
    let copied = copy_file_range_all(&src_fh, &dst_fh, payload.len())?;
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

    let throughput_mib_s = if elapsed > 0.0 {
        (payload.len() as f64 / 1024.0 / 1024.0) / elapsed
    } else {
        0.0
    };
    println!(
        "OK large-copy-benchmark bytes={} elapsed_s={elapsed:.6} throughput_mib_s={throughput_mib_s:.2}",
        payload.len(),
    );

    Ok(())
}
