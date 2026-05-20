// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use crate::extent_plan::{
    plan_coalesced_extents, plan_extent_poc, ExtentPlanOutput, ExtentPoCMode, ExtentPoCSettings,
};

pub fn block_count_for_length(length: u64, block_size: u64, minimum_one: bool) -> u64 {
    if length == 0 {
        return if minimum_one { 1 } else { 0 };
    }
    let block_size = block_size.max(1);
    let count = 1 + (length - 1) / block_size;
    if minimum_one {
        count.max(1)
    } else {
        count
    }
}

pub fn dirty_block_size(file_size: u64, block_index: u64, block_size: u64) -> u64 {
    let block_size = block_size.max(1);
    let block_start = block_index.saturating_mul(block_size);
    let block_end = file_size.min(block_start.saturating_add(block_size));
    block_end.saturating_sub(block_start)
}

#[derive(Debug, PartialEq, Eq)]
pub struct LogicalResizePlan {
    pub old_size: u64,
    pub new_size: u64,
    pub block_size: u64,
    pub old_total_blocks: u64,
    pub new_total_blocks: u64,
    pub shrinking: bool,
    pub has_valid_blocks: bool,
    pub delete_from_block: u64,
    pub max_valid_block: u64,
    pub has_partial_tail: bool,
    pub tail_block_index: u64,
    pub tail_valid_len: u64,
}

pub fn logical_resize_plan(old_size: u64, new_size: u64, block_size: u64) -> LogicalResizePlan {
    let block_size = block_size.max(1);
    let shrinking = new_size < old_size;
    let has_valid_blocks = new_size > 0;
    let old_total_blocks = block_count_for_length(old_size, block_size, false);
    let new_total_blocks = block_count_for_length(new_size, block_size, false);
    let max_valid_block = if has_valid_blocks {
        (new_size - 1) / block_size
    } else {
        0
    };
    let tail_valid_len = if has_valid_blocks {
        new_size % block_size
    } else {
        0
    };
    let has_partial_tail = has_valid_blocks && tail_valid_len != 0;
    let tail_block_index = if has_partial_tail {
        new_size / block_size
    } else {
        0
    };
    let delete_from_block = if shrinking {
        new_total_blocks
    } else {
        old_total_blocks
    };

    LogicalResizePlan {
        old_size,
        new_size,
        block_size,
        old_total_blocks,
        new_total_blocks,
        shrinking,
        has_valid_blocks,
        delete_from_block,
        max_valid_block,
        has_partial_tail,
        tail_block_index,
        tail_valid_len,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PersistLayoutPlan {
    pub total_blocks: u64,
    pub truncate_only: bool,
    pub ordered_dirty_ranges: Vec<(u64, u64)>,
}

pub fn persist_layout_plan(
    file_size: u64,
    block_size: u64,
    truncate_pending: bool,
    dirty_blocks: &[u64],
) -> PersistLayoutPlan {
    let block_size = block_size.max(1);
    let total_blocks = block_count_for_length(file_size, block_size, false);
    let ordered_dirty_ranges = plan_coalesced_extents(dirty_blocks).into_ranges();
    let truncate_only = truncate_pending && dirty_blocks.is_empty();

    PersistLayoutPlan {
        total_blocks,
        truncate_only,
        ordered_dirty_ranges,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistBlockPlanEntry {
    pub block_index: u64,
    pub used_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistBlockPlan {
    pub total_blocks: u64,
    pub truncate_only: bool,
    pub blocks: Vec<PersistBlockPlanEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistPlan {
    Blocks(PersistBlockPlan),
    Extents(ExtentPlanOutput),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistExecutionPlan {
    pub total_blocks: u64,
    pub truncate_only: bool,
    pub payload: PersistPayloadPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistPayloadPlan {
    Blocks(Vec<PersistBlockPlanEntry>),
    Extents(ExtentPlanOutput),
}

#[derive(Debug, Clone, Copy)]
pub struct PersistPlanInput<'a> {
    pub enable_extents: bool,
    pub file_size: u64,
    pub block_size: u64,
    pub truncate_pending: bool,
    pub dirty_blocks: &'a [u64],
}

pub fn persist_block_plan(input: PersistPlanInput<'_>) -> PersistBlockPlan {
    let file_size = input.file_size;
    let block_size = input.block_size;
    let truncate_pending = input.truncate_pending;
    let dirty_blocks = input.dirty_blocks;
    let block_size = block_size.max(1);
    let total_blocks = block_count_for_length(file_size, block_size, false);
    let truncate_only = truncate_pending && dirty_blocks.is_empty();

    let mut blocks = Vec::new();
    if !truncate_only {
        for extent in plan_coalesced_extents(dirty_blocks).extents {
            let start_block = extent.start_block.min(total_blocks);
            let end_block = extent.end_block.min(total_blocks.saturating_sub(1));
            if start_block > end_block {
                continue;
            }
            for block_index in start_block..=end_block {
                let used_len = dirty_block_size(file_size, block_index, block_size);
                if used_len == 0 {
                    continue;
                }
                blocks.push(PersistBlockPlanEntry {
                    block_index,
                    used_len,
                });
            }
        }
    }

    PersistBlockPlan {
        total_blocks,
        truncate_only,
        blocks,
    }
}

pub fn choose_persist_plan(input: PersistPlanInput<'_>) -> PersistPlan {
    if input.enable_extents {
        let settings = ExtentPoCSettings {
            enabled: true,
            mode: ExtentPoCMode::SequentialOnly,
        };
        if let Some(plan) = plan_extent_poc(settings, input.dirty_blocks) {
            let total_blocks = block_count_for_length(input.file_size, input.block_size, false);
            if let Some(extent) = plan.extents.first() {
                if plan.extents.len() == 1
                    && extent.start_block == 0
                    && extent.end_block.saturating_add(1) == total_blocks
                {
                    return PersistPlan::Extents(plan);
                }
            }
        }
    }

    PersistPlan::Blocks(persist_block_plan(input))
}

pub fn choose_persist_execution_plan(input: PersistPlanInput<'_>) -> PersistExecutionPlan {
    match choose_persist_plan(input) {
        PersistPlan::Blocks(plan) => PersistExecutionPlan {
            total_blocks: plan.total_blocks,
            truncate_only: plan.truncate_only,
            payload: PersistPayloadPlan::Blocks(plan.blocks),
        },
        PersistPlan::Extents(plan) => PersistExecutionPlan {
            total_blocks: block_count_for_length(input.file_size, input.block_size, false),
            truncate_only: input.truncate_pending && input.dirty_blocks.is_empty(),
            payload: PersistPayloadPlan::Extents(plan),
        },
    }
}

pub fn contiguous_ranges(values: &[u64]) -> Vec<(u64, u64)> {
    if values.is_empty() {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    let mut start = values[0];
    let mut end = values[0];
    for &value in &values[1..] {
        if value == end.saturating_add(1) {
            end = value;
            continue;
        }
        ranges.push((start, end));
        start = value;
        end = value;
    }
    ranges.push((start, end));
    ranges
}

pub fn sorted_contiguous_ranges(values: &[u64]) -> Vec<(u64, u64)> {
    plan_coalesced_extents(values).into_ranges()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_persist_plan_uses_extent_poc_only_when_enabled_and_sequential() {
        let input = PersistPlanInput {
            enable_extents: true,
            file_size: 16_384,
            block_size: 4_096,
            truncate_pending: true,
            dirty_blocks: &[0, 1, 2, 3],
        };

        match choose_persist_plan(input) {
            PersistPlan::Extents(plan) => {
                assert_eq!(plan.into_ranges(), vec![(0, 3)]);
            }
            PersistPlan::Blocks(_) => panic!("expected extent PoC plan"),
        }

        let block_input = PersistPlanInput {
            enable_extents: false,
            ..input
        };

        match choose_persist_plan(block_input) {
            PersistPlan::Blocks(plan) => {
                assert_eq!(plan, persist_block_plan(block_input));
            }
            PersistPlan::Extents(_) => panic!("expected block storage plan"),
        }

        let fallback_input = PersistPlanInput {
            dirty_blocks: &[0, 2, 3],
            ..input
        };

        match choose_persist_plan(fallback_input) {
            PersistPlan::Blocks(plan) => {
                assert_eq!(plan, persist_block_plan(fallback_input));
            }
            PersistPlan::Extents(_) => panic!("expected block storage fallback"),
        }
    }

    #[test]
    fn choose_persist_execution_plan_preserves_plan_shape() {
        let input = PersistPlanInput {
            enable_extents: true,
            file_size: 16_384,
            block_size: 4_096,
            truncate_pending: true,
            dirty_blocks: &[0, 1, 2, 3],
        };

        match choose_persist_execution_plan(input) {
            PersistExecutionPlan {
                total_blocks,
                truncate_only,
                payload: PersistPayloadPlan::Extents(plan),
            } => {
                assert_eq!(total_blocks, 4);
                assert!(!truncate_only);
                assert_eq!(plan.into_ranges(), vec![(0, 3)]);
            }
            _ => panic!("expected extent execution plan"),
        }

        let block_input = PersistPlanInput {
            enable_extents: false,
            ..input
        };

        match choose_persist_execution_plan(block_input) {
            PersistExecutionPlan {
                total_blocks,
                truncate_only,
                payload: PersistPayloadPlan::Blocks(blocks),
            } => {
                assert_eq!(total_blocks, 4);
                assert!(!truncate_only);
                assert_eq!(blocks, persist_block_plan(block_input).blocks);
            }
            _ => panic!("expected block execution plan"),
        }

        let truncate_only_input = PersistPlanInput {
            truncate_pending: true,
            dirty_blocks: &[],
            ..input
        };

        match choose_persist_execution_plan(truncate_only_input) {
            PersistExecutionPlan {
                total_blocks,
                truncate_only,
                payload: PersistPayloadPlan::Blocks(blocks),
            } => {
                assert_eq!(total_blocks, 4);
                assert!(truncate_only);
                assert!(blocks.is_empty());
            }
            _ => panic!("expected truncate-only execution plan"),
        }
    }
}
