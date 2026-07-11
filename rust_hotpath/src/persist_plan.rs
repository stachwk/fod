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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistSegmentInput {
    pub start_offset: u64,
    pub payload_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistSegmentPlanEntry {
    pub start_block: u64,
    pub block_count: u64,
    pub used_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistSegmentPlan {
    pub total_blocks: u64,
    pub entries: Vec<PersistSegmentPlanEntry>,
}

pub fn plan_sequential_segment_persist(
    file_size: u64,
    block_size: u64,
    max_segment_bytes: u64,
    segments: &[PersistSegmentInput],
) -> Result<PersistSegmentPlan, String> {
    if block_size == 0 {
        return Err("segment persistence requires a non-zero block size".to_string());
    }
    if max_segment_bytes < block_size {
        return Err("segment persistence maximum is smaller than one block".to_string());
    }
    if file_size == 0 {
        return if segments.is_empty() {
            Ok(PersistSegmentPlan {
                total_blocks: 0,
                entries: Vec::new(),
            })
        } else {
            Err("empty file cannot have segment payloads".to_string())
        };
    }
    if segments.is_empty() {
        return Err("non-empty file requires segment payloads".to_string());
    }

    let total_blocks = block_count_for_length(file_size, block_size, false);
    let mut expected_offset = 0u64;
    let mut entries = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        if segment.payload_bytes == 0 {
            return Err(format!("segment {index} has an empty payload"));
        }
        if segment.payload_bytes > max_segment_bytes {
            return Err(format!("segment {index} exceeds the configured maximum"));
        }
        if segment.start_offset != expected_offset {
            return Err(format!(
                "segment {index} starts at {} instead of {}",
                segment.start_offset, expected_offset
            ));
        }
        if segment.start_offset % block_size != 0 {
            return Err(format!("segment {index} start is not block-aligned"));
        }
        let segment_end = segment
            .start_offset
            .checked_add(segment.payload_bytes)
            .ok_or_else(|| format!("segment {index} end offset overflows"))?;
        if segment_end > file_size {
            return Err(format!("segment {index} exceeds file size"));
        }
        if index + 1 < segments.len() && segment.payload_bytes % block_size != 0 {
            return Err(format!(
                "segment {index} has a non-aligned non-final payload"
            ));
        }
        let block_count = block_count_for_length(segment.payload_bytes, block_size, false);
        entries.push(PersistSegmentPlanEntry {
            start_block: segment.start_offset / block_size,
            block_count,
            used_bytes: segment.payload_bytes,
        });
        expected_offset = segment_end;
    }

    if expected_offset != file_size {
        return Err(format!(
            "segment coverage ends at {expected_offset} instead of file size {file_size}"
        ));
    }

    Ok(PersistSegmentPlan {
        total_blocks,
        entries,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistPayloadPlan {
    Blocks(Vec<PersistBlockPlanEntry>),
    Extents(ExtentPlanOutput),
}

#[derive(Debug, Clone, Copy)]
pub struct PersistPlanInput<'a> {
    pub enable_extents: bool,
    pub extent_target_bytes: u64,
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
        if let Some(plan) = plan_extent_poc(
            settings,
            input.dirty_blocks,
            input.block_size,
            input.extent_target_bytes,
            input.extent_target_bytes,
        ) {
            let total_blocks = block_count_for_length(input.file_size, input.block_size, false);
            if let (Some(first), Some(last)) = (plan.extents.first(), plan.extents.last()) {
                if first.start_block == 0 && last.end_block.saturating_add(1) == total_blocks {
                    return PersistPlan::Extents(plan);
                }
            }
        }
    }

    PersistPlan::Blocks(persist_block_plan(input))
}

pub fn choose_persist_execution_plan(input: PersistPlanInput<'_>) -> PersistExecutionPlan {
    match choose_persist_plan(input) {
        PersistPlan::Blocks(plan) => {
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
        PersistPlan::Extents(plan) => {
            let write_class = classify_persist_write(PersistWriteClassInput {
                new_object_sequential: false,
                truncate_pending: input.truncate_pending,
                has_payload: !plan.extents.is_empty(),
            });
            PersistExecutionPlan {
                total_blocks: block_count_for_length(input.file_size, input.block_size, false),
                write_class,
                payload: PersistPayloadPlan::Extents(plan),
            }
        }
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
            extent_target_bytes: 1024 * 1024,
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
            extent_target_bytes: 1024 * 1024,
            file_size: 16_384,
            block_size: 4_096,
            truncate_pending: true,
            dirty_blocks: &[0, 1, 2, 3],
        };

        match choose_persist_execution_plan(input) {
            PersistExecutionPlan {
                total_blocks,
                write_class,
                payload: PersistPayloadPlan::Extents(plan),
            } => {
                assert_eq!(total_blocks, 4);
                assert_eq!(write_class, PersistWriteClass::ExistingObjectPatch);
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
                write_class,
                payload: PersistPayloadPlan::Blocks(blocks),
            } => {
                assert_eq!(total_blocks, 4);
                assert_eq!(write_class, PersistWriteClass::ExistingObjectPatch);
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
                write_class,
                payload: PersistPayloadPlan::Blocks(blocks),
            } => {
                assert_eq!(total_blocks, 4);
                assert_eq!(write_class, PersistWriteClass::TruncateOnly);
                assert!(blocks.is_empty());
            }
            _ => panic!("expected truncate-only execution plan"),
        }
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

    #[test]
    fn choose_persist_plan_bounds_full_file_extents() {
        let dirty_blocks = (0..1024).collect::<Vec<_>>();
        let input = PersistPlanInput {
            enable_extents: true,
            extent_target_bytes: 1024 * 1024,
            file_size: 4 * 1024 * 1024,
            block_size: 4096,
            truncate_pending: true,
            dirty_blocks: &dirty_blocks,
        };

        match choose_persist_plan(input) {
            PersistPlan::Extents(plan) => assert_eq!(
                plan.into_ranges(),
                vec![(0, 255), (256, 511), (512, 767), (768, 1023)]
            ),
            PersistPlan::Blocks(_) => panic!("expected bounded extent plan"),
        }
    }

    #[test]
    fn disabled_extent_path_does_not_read_extent_target() {
        let input = PersistPlanInput {
            enable_extents: false,
            extent_target_bytes: 0,
            file_size: 8192,
            block_size: 4096,
            truncate_pending: true,
            dirty_blocks: &[0, 1],
        };

        match choose_persist_plan(input) {
            PersistPlan::Blocks(plan) => assert_eq!(plan, persist_block_plan(input)),
            PersistPlan::Extents(_) => panic!("disabled extent path must use blocks"),
        }
    }

    #[test]
    fn sequential_segment_plan_accepts_bounded_full_coverage() {
        let plan = plan_sequential_segment_persist(
            10_000,
            4096,
            8192,
            &[
                PersistSegmentInput {
                    start_offset: 0,
                    payload_bytes: 8192,
                },
                PersistSegmentInput {
                    start_offset: 8192,
                    payload_bytes: 1808,
                },
            ],
        )
        .expect("valid sequential segment plan");

        assert_eq!(plan.total_blocks, 3);
        assert_eq!(
            plan.entries,
            vec![
                PersistSegmentPlanEntry {
                    start_block: 0,
                    block_count: 2,
                    used_bytes: 8192,
                },
                PersistSegmentPlanEntry {
                    start_block: 2,
                    block_count: 1,
                    used_bytes: 1808,
                },
            ]
        );
    }

    #[test]
    fn sequential_segment_plan_rejects_gaps_and_unaligned_segments() {
        let gap = plan_sequential_segment_persist(
            8192,
            4096,
            4096,
            &[
                PersistSegmentInput {
                    start_offset: 0,
                    payload_bytes: 4096,
                },
                PersistSegmentInput {
                    start_offset: 5000,
                    payload_bytes: 3192,
                },
            ],
        )
        .expect_err("gap must be rejected");
        assert!(gap.contains("starts at"));

        let unaligned = plan_sequential_segment_persist(
            8192,
            4096,
            8192,
            &[
                PersistSegmentInput {
                    start_offset: 0,
                    payload_bytes: 4000,
                },
                PersistSegmentInput {
                    start_offset: 4000,
                    payload_bytes: 4192,
                },
            ],
        )
        .expect_err("unaligned non-final segment must be rejected");
        assert!(unaligned.contains("non-aligned non-final"));
    }

    #[test]
    fn sequential_segment_plan_rejects_oversized_or_incomplete_payloads() {
        let oversized = plan_sequential_segment_persist(
            8192,
            4096,
            4096,
            &[PersistSegmentInput {
                start_offset: 0,
                payload_bytes: 8192,
            }],
        )
        .expect_err("oversized segment must be rejected");
        assert!(oversized.contains("exceeds the configured maximum"));

        let incomplete = plan_sequential_segment_persist(
            8192,
            4096,
            8192,
            &[PersistSegmentInput {
                start_offset: 0,
                payload_bytes: 4096,
            }],
        )
        .expect_err("incomplete coverage must be rejected");
        assert!(incomplete.contains("coverage ends"));
    }
}
