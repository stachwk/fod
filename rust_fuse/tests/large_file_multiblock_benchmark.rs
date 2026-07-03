// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use support::{checked_payload_len, parse_size_bytes, repeating_payload, unique_suffix, MountedFs};

struct Cleanup {
    file_path: PathBuf,
    dir_path: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file_path);
        let _ = fs::remove_dir(&self.dir_path);
    }
}

#[test]
fn large_file_multiblock_benchmark() -> Result<(), String> {
    let threshold = (1_u64 << 60).to_string();
    let mounted = MountedFs::start_with_env(
        "large-file-multiblock-benchmark",
        &[
            ("FOD_WRITE_FLUSH_THRESHOLD_BYTES", threshold),
            ("FOD_PROFILE_IO", "1".to_string()),
        ],
    )?;

    let suffix = unique_suffix();
    let dir_path = mounted.mountpoint.join(format!("large-file-{suffix}"));
    let file_path = dir_path.join("payload.bin");
    let chunk_size =
        parse_size_bytes(&env::var("LARGE_FILE_CHUNK_SIZE").unwrap_or_else(|_| "4M".to_string()))?;
    let chunk_count = env::var("LARGE_FILE_CHUNK_COUNT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(16);
    let payload = repeating_payload(
        b"fod-large-file-",
        checked_payload_len(chunk_size, chunk_count)?,
    );
    let _cleanup = Cleanup {
        file_path: file_path.clone(),
        dir_path: dir_path.clone(),
    };

    fs::create_dir(&dir_path).map_err(|err| format!("create_dir failed: {err}"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&file_path)
        .map_err(|err| format!("open failed: {err}"))?;

    let start = Instant::now();
    for chunk in payload.chunks(chunk_size) {
        file.write_all(chunk)
            .map_err(|err| format!("write_all failed: {err}"))?;
    }
    file.flush().map_err(|err| format!("flush failed: {err}"))?;
    drop(file);
    let elapsed = start.elapsed().as_secs_f64();

    let read_back = fs::read(&file_path).map_err(|err| format!("read_back failed: {err}"))?;
    if read_back != payload {
        return Err("large multi-block payload mismatch".to_string());
    }

    let throughput_mib_s = if elapsed > 0.0 {
        (payload.len() as f64 / 1024.0 / 1024.0) / elapsed
    } else {
        0.0
    };
    println!(
        "OK large-file-multiblock bytes={} elapsed_s={elapsed:.6} throughput_mib_s={throughput_mib_s:.2}",
        payload.len(),
    );
    Ok(())
}
