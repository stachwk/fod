// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use crate::extent::{coalesce_sorted_blocks, Extent};

/// Minimal planning input for the extent-engine PoC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtentPlanInput<'a> {
    pub blocks: &'a [u64],
    pub block_size: u64,
}

/// Minimal planning output for the extent-engine PoC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtentPlanOutput {
    pub extents: Vec<Extent>,
}

impl ExtentPlanOutput {
    pub fn into_ranges(self) -> Vec<(u64, u64)> {
        self.extents
            .into_iter()
            .map(|extent| extent.to_range())
            .collect()
    }
}

/// The first PoC mode only accepts a single contiguous extent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtentPoCMode {
    SequentialOnly,
}

/// Minimal PoC gate that keeps the extent path opt-in and narrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtentPoCSettings {
    pub enabled: bool,
    pub mode: ExtentPoCMode,
}

impl Default for ExtentPoCSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ExtentPoCMode::SequentialOnly,
        }
    }
}

pub trait ExtentPlanner {
    fn plan(&self, input: ExtentPlanInput<'_>) -> Option<ExtentPlanOutput>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CoalescingExtentPlanner;

impl ExtentPlanner for CoalescingExtentPlanner {
    fn plan(&self, input: ExtentPlanInput<'_>) -> Option<ExtentPlanOutput> {
        Some(ExtentPlanOutput {
            extents: coalesce_sorted_blocks(input.blocks),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundedExtentPlanner {
    pub target_bytes: u64,
    pub max_bytes: u64,
}

impl BoundedExtentPlanner {
    fn blocks_per_extent(self, block_size: u64) -> Option<u64> {
        if block_size == 0 || self.target_bytes == 0 || self.max_bytes < block_size {
            return None;
        }

        let target_blocks = (self.target_bytes / block_size).max(1);
        let max_blocks = self.max_bytes / block_size;
        Some(target_blocks.min(max_blocks))
    }
}

impl ExtentPlanner for BoundedExtentPlanner {
    fn plan(&self, input: ExtentPlanInput<'_>) -> Option<ExtentPlanOutput> {
        let blocks_per_extent = self.blocks_per_extent(input.block_size)?;
        let mut extents = Vec::new();

        for contiguous in coalesce_sorted_blocks(input.blocks) {
            let mut start_block = contiguous.start_block;
            loop {
                let remaining_blocks = contiguous
                    .end_block
                    .saturating_sub(start_block)
                    .saturating_add(1);
                let block_count = remaining_blocks.min(blocks_per_extent);
                let end_block = start_block.saturating_add(block_count.saturating_sub(1));
                extents.push(
                    Extent::new_checked(start_block, end_block)
                        .expect("bounded extent must be ordered"),
                );
                if end_block >= contiguous.end_block {
                    break;
                }
                start_block = end_block.saturating_add(1);
            }
        }

        Some(ExtentPlanOutput { extents })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SequentialOnlyExtentPlanner;

impl ExtentPlanner for SequentialOnlyExtentPlanner {
    fn plan(&self, input: ExtentPlanInput<'_>) -> Option<ExtentPlanOutput> {
        let extents = coalesce_sorted_blocks(input.blocks);
        if extents.len() != 1 {
            return None;
        }
        Some(ExtentPlanOutput { extents })
    }
}

pub fn plan_coalesced_extents(blocks: &[u64]) -> ExtentPlanOutput {
    CoalescingExtentPlanner
        .plan(ExtentPlanInput {
            blocks,
            block_size: 1,
        })
        .expect("coalescing planner always produces an output")
}

pub fn plan_extent_poc(
    settings: ExtentPoCSettings,
    blocks: &[u64],
    block_size: u64,
    target_bytes: u64,
    max_bytes: u64,
) -> Option<ExtentPlanOutput> {
    if !settings.enabled {
        return None;
    }

    match settings.mode {
        ExtentPoCMode::SequentialOnly => {
            SequentialOnlyExtentPlanner.plan(ExtentPlanInput { blocks, block_size })?;
        }
    }

    BoundedExtentPlanner {
        target_bytes,
        max_bytes,
    }
    .plan(ExtentPlanInput { blocks, block_size })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_only_poc_requires_single_contiguous_extent() {
        let settings = ExtentPoCSettings {
            enabled: true,
            mode: ExtentPoCMode::SequentialOnly,
        };
        assert_eq!(
            plan_extent_poc(settings, &[3, 4, 5], 4096, 1024 * 1024, 1024 * 1024)
                .map(|plan| plan.into_ranges()),
            Some(vec![(3, 5)])
        );
        assert_eq!(
            plan_extent_poc(settings, &[3, 5], 4096, 1024 * 1024, 1024 * 1024),
            None
        );
        assert_eq!(
            plan_extent_poc(
                ExtentPoCSettings::default(),
                &[3, 4, 5],
                4096,
                1024 * 1024,
                1024 * 1024,
            ),
            None
        );
    }

    #[test]
    fn coalescing_planner_still_returns_all_contiguous_ranges() {
        let plan = plan_coalesced_extents(&[7, 3, 4, 10, 11, 11, 8]);
        assert_eq!(plan.into_ranges(), vec![(3, 4), (7, 8), (10, 11)]);
    }

    fn bounded_ranges(blocks: &[u64], target_bytes: u64, max_bytes: u64) -> Vec<(u64, u64)> {
        BoundedExtentPlanner {
            target_bytes,
            max_bytes,
        }
        .plan(ExtentPlanInput {
            blocks,
            block_size: 4096,
        })
        .expect("bounded extent plan")
        .into_ranges()
    }

    #[test]
    fn bounded_planner_handles_empty_and_single_block_inputs() {
        assert!(bounded_ranges(&[], 1024 * 1024, 1024 * 1024).is_empty());
        assert_eq!(bounded_ranges(&[7], 1024 * 1024, 1024 * 1024), vec![(7, 7)]);
    }

    #[test]
    fn bounded_planner_preserves_exact_target_and_final_fragment() {
        let exact_target = (0..256).collect::<Vec<_>>();
        assert_eq!(
            bounded_ranges(&exact_target, 1024 * 1024, 1024 * 1024),
            vec![(0, 255)]
        );

        let target_plus_one = (0..257).collect::<Vec<_>>();
        assert_eq!(
            bounded_ranges(&target_plus_one, 1024 * 1024, 1024 * 1024),
            vec![(0, 255), (256, 256)]
        );

        let partial_final = (10..610).collect::<Vec<_>>();
        assert_eq!(
            bounded_ranges(&partial_final, 1024 * 1024, 1024 * 1024),
            vec![(10, 265), (266, 521), (522, 609)]
        );
    }

    #[test]
    fn bounded_planner_splits_multiple_extents_without_joining_gaps() {
        let blocks = [0, 1, 2, 3, 8, 9, 10, 11, 12];
        assert_eq!(
            bounded_ranges(&blocks, 3 * 4096, 3 * 4096),
            vec![(0, 2), (3, 3), (8, 10), (11, 12)]
        );
    }

    #[test]
    fn bounded_planner_keeps_unsorted_and_duplicate_input_contract() {
        let blocks = [258, 0, 257, 1, 256, 1, 2, 258];
        assert_eq!(
            bounded_ranges(&blocks, 2 * 4096, 2 * 4096),
            vec![(0, 1), (2, 2), (256, 257), (258, 258)]
        );
    }

    #[test]
    fn bounded_planner_caps_target_at_maximum_size() {
        let blocks = (0..1024).collect::<Vec<_>>();
        let ranges = bounded_ranges(&blocks, 4 * 1024 * 1024, 1024 * 1024);
        assert_eq!(ranges.len(), 4);
        assert_eq!(ranges[0], (0, 255));
        assert_eq!(ranges[3], (768, 1023));
    }

    #[test]
    fn bounded_planner_rejects_a_maximum_smaller_than_one_block() {
        assert_eq!(
            BoundedExtentPlanner {
                target_bytes: 4096,
                max_bytes: 4095,
            }
            .plan(ExtentPlanInput {
                blocks: &[0],
                block_size: 4096,
            }),
            None
        );
    }
}
