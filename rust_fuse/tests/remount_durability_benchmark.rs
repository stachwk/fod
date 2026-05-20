// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::fs;
use std::time::Instant;

use support::{unique_suffix, MountedFs};

#[test]
fn remount_durability_benchmark() -> Result<(), String> {
    let suffix = unique_suffix();
    let mut payload = Vec::with_capacity(64 * 1024);
    while payload.len() < 64 * 1024 {
        payload.extend_from_slice(b"fod-remount-durability-");
    }
    payload.truncate(64 * 1024);
    let file_name = format!("durability-{suffix}.bin");

    let mounted1 = MountedFs::start("remount-durability-1")?;
    let mount1_path = mounted1.mountpoint.display().to_string();
    let file_path = mounted1.mountpoint.join(&file_name);
    fs::write(&file_path, &payload).map_err(|err| format!("write failed: {err}"))?;
    drop(mounted1);

    let start = Instant::now();
    let mounted2 = MountedFs::start("remount-durability-2")?;
    let mount2_path = mounted2.mountpoint.display().to_string();
    let remount_path = mounted2.mountpoint.join(&file_name);
    let read_back = fs::read(&remount_path).map_err(|err| format!("read_back failed: {err}"))?;
    let elapsed = start.elapsed().as_secs_f64();

    if read_back != payload {
        return Err("remount durability payload mismatch".to_string());
    }

    println!(
        "OK remount-durability bytes={} elapsed_s={elapsed:.6} mount1={} mount2={}",
        payload.len(),
        mount1_path,
        mount2_path,
    );
    Ok(())
}
