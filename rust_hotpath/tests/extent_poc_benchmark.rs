// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::env;
use std::time::Instant;

use fod_rust_hotpath::{plan_extent_poc, ExtentPoCMode, ExtentPoCSettings};

fn parse_iterations() -> usize {
    env::var("EXTENT_POC_BENCH_ITERATIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(10_000)
}

fn build_contiguous_blocks(block_count: usize) -> Vec<u64> {
    (0..block_count as u64).collect()
}

#[test]
fn extent_poc_benchmark() -> Result<(), String> {
    let iterations = parse_iterations();
    let settings = ExtentPoCSettings {
        enabled: true,
        mode: ExtentPoCMode::SequentialOnly,
    };
    let scenarios = [
        ("4 KiB", 4 * 1024usize, 1usize),
        ("64 KiB", 64 * 1024usize, 16usize),
        ("1 MiB", 1024 * 1024usize, 256usize),
        ("4 MiB", 4 * 1024 * 1024usize, 1024usize),
    ];

    for (label, bytes, blocks) in scenarios {
        let blocks = build_contiguous_blocks(blocks);
        let mut checksum = 0usize;
        let start = Instant::now();

        for _ in 0..iterations {
            let plan = plan_extent_poc(settings, &blocks)
                .ok_or_else(|| format!("extent PoC unexpectedly rejected {label}"))?;
            if plan.extents.len() != 1 {
                return Err(format!(
                    "expected one contiguous extent for {label}, got {}",
                    plan.extents.len()
                ));
            }
            checksum += plan.extents.len();
        }

        let elapsed = start.elapsed().as_secs_f64();
        let elapsed_per_op_ns = if iterations > 0 {
            (elapsed * 1_000_000_000.0) / iterations as f64
        } else {
            0.0
        };
        println!(
            "OK extent-poc-benchmark size={} bytes={} blocks={} iterations={} elapsed_s={elapsed:.6} per_op_ns={elapsed_per_op_ns:.2} checksum={checksum}",
            label,
            bytes,
            blocks.len(),
            iterations,
        );
    }

    Ok(())
}
