// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_hotpath::ffi::{fod_copy_dedupe, fod_free_ranges, DbfsRange};
use std::env;
use std::time::Instant;

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
        payload.extend_from_slice(b"fod-copy-dedupe-");
    }
    payload.truncate(total);
    payload
}

fn run_case(label: &str, payload: &[u8], current: &[u8], block_size: usize) -> Result<(), String> {
    let mut out_ptr: *mut DbfsRange = std::ptr::null_mut();
    let mut out_len: usize = 0;

    let start = Instant::now();
    let status = fod_copy_dedupe(
        0,
        payload.as_ptr(),
        payload.len(),
        current.as_ptr(),
        current.len(),
        block_size,
        &mut out_ptr,
        &mut out_len,
    );
    if status != 0 {
        return Err(format!("fod_copy_dedupe returned status {status}"));
    }
    let elapsed = start.elapsed().as_secs_f64();
    let ranges = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
    let changed_bytes: u64 = ranges
        .iter()
        .map(|range| range.end.saturating_sub(range.start))
        .sum();
    let throughput_mib_s = if elapsed > 0.0 {
        (payload.len() as f64 / 1024.0 / 1024.0) / elapsed
    } else {
        0.0
    };
    println!(
        "OK copy-dedupe/{label} bytes={} elapsed_s={elapsed:.6} throughput_mib_s={throughput_mib_s:.2} ranges={} changed_bytes={}",
        payload.len(),
        ranges.len(),
        changed_bytes,
    );
    fod_free_ranges(out_ptr, out_len);
    Ok(())
}

#[test]
fn copy_dedupe_benchmark() -> Result<(), String> {
    let block_size =
        parse_bytes(&env::var("COPY_DEDUPE_BLOCK_SIZE").unwrap_or_else(|_| "512K".to_string()))?;
    let block_count = env::var("COPY_DEDUPE_BLOCK_COUNT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(8);
    if block_size == 0 || block_count == 0 {
        return Err(
            "COPY_DEDUPE_BLOCK_SIZE and COPY_DEDUPE_BLOCK_COUNT must be greater than zero"
                .to_string(),
        );
    }

    let payload = build_payload(block_size, block_count);
    let current_off = payload.iter().map(|byte| byte ^ 0xFF).collect::<Vec<u8>>();
    let current_on = payload.clone();

    run_case("off", &payload, &current_off, block_size)?;
    run_case("on", &payload, &current_on, block_size)?;
    Ok(())
}
