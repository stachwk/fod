// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use support::{checked_payload_len, parse_size_bytes, repeating_payload, MountedFs};

fn conflict_id() -> String {
    env::var("DATA_BLOCKS_CONFLICT_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "default".to_string())
        .replace(['/', '\\'], "_")
}

fn payload_shape() -> Result<(usize, usize, usize), String> {
    let block_size = parse_size_bytes(
        &env::var("DATA_BLOCKS_CONFLICT_BLOCK_SIZE").unwrap_or_else(|_| "4M".to_string()),
    )?;
    let block_count = env::var("DATA_BLOCKS_CONFLICT_BLOCK_COUNT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(16);
    let total = checked_payload_len(block_size, block_count)?;
    Ok((block_size, block_count, total))
}

fn payload(marker: &[u8]) -> Result<Vec<u8>, String> {
    let (_, _, total) = payload_shape()?;
    Ok(repeating_payload(marker, total))
}

fn workload_paths(mountpoint: &Path) -> (PathBuf, PathBuf) {
    let dir_path = mountpoint.join(format!("data-blocks-conflict-{}", conflict_id()));
    let file_path = dir_path.join("payload.bin");
    (dir_path, file_path)
}

fn mounted_conflict_fs(name: &str) -> Result<MountedFs, String> {
    MountedFs::start_with_env(
        name,
        &[
            ("FOD_WRITE_FLUSH_THRESHOLD_BYTES", (1_u64 << 60).to_string()),
            ("FOD_PROFILE_IO", "1".to_string()),
        ],
    )
}

fn write_full_payload(file_path: &Path, payload: &[u8], truncate: bool) -> Result<f64, String> {
    let mut file = OpenOptions::new()
        .create(truncate)
        .read(true)
        .write(true)
        .truncate(truncate)
        .open(file_path)
        .map_err(|err| format!("open {} failed: {err}", file_path.display()))?;

    file.seek(SeekFrom::Start(0))
        .map_err(|err| format!("seek failed: {err}"))?;
    let start = Instant::now();
    file.write_all(payload)
        .map_err(|err| format!("write_all failed: {err}"))?;
    drop(file);
    Ok(start.elapsed().as_secs_f64())
}

fn verify_payload(file_path: &Path, expected: &[u8]) -> Result<(), String> {
    let read_back = fs::read(file_path)
        .map_err(|err| format!("read_back {} failed: {err}", file_path.display()))?;
    if read_back != expected {
        return Err("data_blocks conflict payload mismatch".to_string());
    }
    Ok(())
}

#[test]
fn data_blocks_conflict_seed() -> Result<(), String> {
    let mounted = mounted_conflict_fs("data-blocks-conflict-seed")?;
    let (block_size, block_count, total) = payload_shape()?;
    let seed_payload = payload(b"fod-data-blocks-conflict-seed-")?;
    let (dir_path, file_path) = workload_paths(&mounted.mountpoint);

    let _ = fs::remove_file(&file_path);
    let _ = fs::remove_dir(&dir_path);
    fs::create_dir_all(&dir_path).map_err(|err| format!("create_dir_all failed: {err}"))?;

    let elapsed = write_full_payload(&file_path, &seed_payload, true)?;
    verify_payload(&file_path, &seed_payload)?;
    let throughput_mib_s = if elapsed > 0.0 {
        (total as f64 / 1024.0 / 1024.0) / elapsed
    } else {
        0.0
    };

    println!(
        "OK data-blocks-conflict-seed id={} block_size={} block_count={} bytes={} elapsed_s={elapsed:.6} throughput_mib_s={throughput_mib_s:.2}",
        conflict_id(),
        block_size,
        block_count,
        total,
    );
    Ok(())
}

#[test]
fn data_blocks_conflict_overwrite_benchmark() -> Result<(), String> {
    let mounted = mounted_conflict_fs("data-blocks-conflict-overwrite")?;
    let (block_size, block_count, total) = payload_shape()?;
    let overwrite_payload = payload(b"fod-data-blocks-conflict-overwrite-")?;
    let (_, file_path) = workload_paths(&mounted.mountpoint);

    if !file_path.exists() {
        return Err(format!(
            "missing seeded conflict file {}; run data_blocks_conflict_seed first with DATA_BLOCKS_CONFLICT_ID={}",
            file_path.display(),
            conflict_id()
        ));
    }

    let elapsed = write_full_payload(&file_path, &overwrite_payload, false)?;
    verify_payload(&file_path, &overwrite_payload)?;
    let throughput_mib_s = if elapsed > 0.0 {
        (total as f64 / 1024.0 / 1024.0) / elapsed
    } else {
        0.0
    };

    println!(
        "OK data-blocks-conflict-overwrite bytes={} id={} block_size={} block_count={} elapsed_s={elapsed:.6} throughput_mib_s={throughput_mib_s:.2}",
        total,
        conflict_id(),
        block_size,
        block_count,
    );
    Ok(())
}
