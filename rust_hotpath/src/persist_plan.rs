// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

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
    let ordered_dirty_ranges = sorted_contiguous_ranges(dirty_blocks);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistWriteClass {
    NewObjectSequential,
    ExistingObjectPatch,
    TruncateOnly,
}

impl PersistWriteClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewObjectSequential => "new_object_sequential",
            Self::ExistingObjectPatch => "existing_object_patch",
            Self::TruncateOnly => "truncate_only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistWriteClassInput {
    pub new_object_sequential: bool,
    pub truncate_pending: bool,
    pub has_payload: bool,
}

pub fn classify_persist_write(input: PersistWriteClassInput) -> PersistWriteClass {
    if input.new_object_sequential && input.has_payload {
        PersistWriteClass::NewObjectSequential
    } else if input.truncate_pending && !input.has_payload {
        PersistWriteClass::TruncateOnly
    } else {
        PersistWriteClass::ExistingObjectPatch
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistExecutionPlan {
    pub total_blocks: u64,
    pub write_class: PersistWriteClass,
    pub payload: PersistPayloadPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistPayloadPlan {
    Blocks(Vec<PersistBlockPlanEntry>),
}

#[derive(Debug, Clone, Copy)]
pub struct PersistPlanInput<'a> {
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
        for (start_block, end_block) in sorted_contiguous_ranges(dirty_blocks) {
            let start_block = start_block.min(total_blocks);
            let end_block = end_block.min(total_blocks.saturating_sub(1));
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

pub fn choose_persist_plan(input: PersistPlanInput<'_>) -> PersistBlockPlan {
    persist_block_plan(input)
}

pub fn choose_persist_execution_plan(input: PersistPlanInput<'_>) -> PersistExecutionPlan {
    let plan = choose_persist_plan(input);
    let write_class = classify_persist_write(PersistWriteClassInput {
        new_object_sequential: false,
        truncate_pending: input.truncate_pending,
        has_payload: !plan.blocks.is_empty(),
    });
    PersistExecutionPlan {
        total_blocks: plan.total_blocks,
        write_class,
        payload: PersistPayloadPlan::Blocks(plan.blocks),
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
    if values.is_empty() {
        return Vec::new();
    }

    let mut sorted_values = values.to_vec();
    sorted_values.sort_unstable();
    sorted_values.dedup();
    contiguous_ranges(&sorted_values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_persist_plan_always_uses_blocks() {
        let input = PersistPlanInput {
            file_size: 16_384,
            block_size: 4_096,
            truncate_pending: true,
            dirty_blocks: &[0, 1, 2, 3],
        };

        assert_eq!(choose_persist_plan(input), persist_block_plan(input));

        let fallback_input = PersistPlanInput {
            dirty_blocks: &[0, 2, 3],
            ..input
        };

        assert_eq!(
            choose_persist_plan(fallback_input),
            persist_block_plan(fallback_input)
        );
    }

    #[test]
    fn choose_persist_execution_plan_preserves_plan_shape() {
        let input = PersistPlanInput {
            file_size: 16_384,
            block_size: 4_096,
            truncate_pending: true,
            dirty_blocks: &[0, 1, 2, 3],
        };

        let PersistExecutionPlan {
            total_blocks,
            write_class,
            payload: PersistPayloadPlan::Blocks(blocks),
        } = choose_persist_execution_plan(input);
        assert_eq!(total_blocks, 4);
        assert_eq!(write_class, PersistWriteClass::ExistingObjectPatch);
        assert_eq!(blocks, persist_block_plan(input).blocks);

        let truncate_only_input = PersistPlanInput {
            truncate_pending: true,
            dirty_blocks: &[],
            ..input
        };

        let PersistExecutionPlan {
            total_blocks,
            write_class,
            payload: PersistPayloadPlan::Blocks(blocks),
        } = choose_persist_execution_plan(truncate_only_input);
        assert_eq!(total_blocks, 4);
        assert_eq!(write_class, PersistWriteClass::TruncateOnly);
        assert!(blocks.is_empty());
    }

    #[test]
    fn persist_write_classification_separates_storage_semantics() {
        assert_eq!(
            classify_persist_write(PersistWriteClassInput {
                new_object_sequential: true,
                truncate_pending: true,
                has_payload: true,
            }),
            PersistWriteClass::NewObjectSequential
        );
        assert_eq!(
            classify_persist_write(PersistWriteClassInput {
                new_object_sequential: false,
                truncate_pending: false,
                has_payload: true,
            }),
            PersistWriteClass::ExistingObjectPatch
        );
        assert_eq!(
            classify_persist_write(PersistWriteClassInput {
                new_object_sequential: false,
                truncate_pending: true,
                has_payload: false,
            }),
            PersistWriteClass::TruncateOnly
        );
    }
}
