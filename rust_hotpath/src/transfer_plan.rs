// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use crate::persist_plan::block_count_for_length;

pub fn block_transfer_plan(
    length: u64,
    block_size: u64,
    requested_workers: u64,
    workers_min_blocks: u64,
    minimum_one: bool,
) -> (u64, bool, u64) {
    let total_blocks = block_count_for_length(length, block_size, minimum_one);
    let (parallel, workers) = parallel_worker_plan(
        requested_workers,
        workers_min_blocks,
        total_blocks,
        total_blocks,
    );
    (total_blocks, parallel, workers)
}

pub fn write_copy_worker_count(
    total_blocks: u64,
    workers_write: u64,
    workers_write_min_blocks: u64,
) -> u64 {
    parallel_worker_count(
        workers_write,
        workers_write_min_blocks,
        total_blocks,
        total_blocks,
    )
}

pub fn write_copy_plan(
    length: u64,
    block_size: u64,
    workers_write: u64,
    workers_write_min_blocks: u64,
    copy_dedupe_enabled: bool,
    copy_dedupe_min_blocks: u64,
    copy_dedupe_max_blocks: u64,
) -> (u64, bool, bool, u64) {
    let (total_blocks, parallel, workers) = block_transfer_plan(
        length,
        block_size,
        workers_write,
        workers_write_min_blocks,
        true,
    );
    let dedupe_enabled = write_copy_dedupe_plan(
        length,
        block_size,
        copy_dedupe_enabled,
        copy_dedupe_min_blocks,
        copy_dedupe_max_blocks,
    )
    .1;
    (total_blocks, dedupe_enabled, parallel, workers)
}

pub fn write_copy_dedupe_plan(
    length: u64,
    block_size: u64,
    copy_dedupe_enabled: bool,
    copy_dedupe_min_blocks: u64,
    copy_dedupe_max_blocks: u64,
) -> (u64, bool) {
    let total_blocks = block_count_for_length(length, block_size, true);
    let dedupe_enabled = copy_dedupe_enabled
        && total_blocks >= copy_dedupe_min_blocks.max(1)
        && (copy_dedupe_max_blocks == 0 || total_blocks <= copy_dedupe_max_blocks);
    (total_blocks, dedupe_enabled)
}

pub fn parallel_worker_count(
    requested_workers: u64,
    minimum_items_for_parallel: u64,
    total_items: u64,
    parallel_groups: u64,
) -> u64 {
    if requested_workers <= 1 || total_items < minimum_items_for_parallel || parallel_groups <= 1 {
        return 1;
    }

    requested_workers.min(parallel_groups).max(1)
}

pub fn parallel_worker_plan(
    requested_workers: u64,
    minimum_items_for_parallel: u64,
    total_items: u64,
    parallel_groups: u64,
) -> (bool, u64) {
    let workers = parallel_worker_count(
        requested_workers,
        minimum_items_for_parallel,
        total_items,
        parallel_groups,
    );
    (workers > 1, workers)
}
