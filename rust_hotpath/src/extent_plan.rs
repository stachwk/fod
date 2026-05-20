// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use crate::extent::{coalesce_sorted_blocks, Extent};

/// Minimal planning input for the extent-engine PoC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtentPlanInput<'a> {
    pub blocks: &'a [u64],
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
        .plan(ExtentPlanInput { blocks })
        .expect("coalescing planner always produces an output")
}

pub fn plan_extent_poc(settings: ExtentPoCSettings, blocks: &[u64]) -> Option<ExtentPlanOutput> {
    if !settings.enabled {
        return None;
    }

    match settings.mode {
        ExtentPoCMode::SequentialOnly => {
            SequentialOnlyExtentPlanner.plan(ExtentPlanInput { blocks })
        }
    }
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
            plan_extent_poc(settings, &[3, 4, 5]).map(|plan| plan.into_ranges()),
            Some(vec![(3, 5)])
        );
        assert_eq!(plan_extent_poc(settings, &[3, 5]), None);
        assert_eq!(
            plan_extent_poc(ExtentPoCSettings::default(), &[3, 4, 5]),
            None
        );
    }

    #[test]
    fn coalescing_planner_still_returns_all_contiguous_ranges() {
        let plan = plan_coalesced_extents(&[7, 3, 4, 10, 11, 11, 8]);
        assert_eq!(plan.into_ranges(), vec![(3, 4), (7, 8), (10, 11)]);
    }
}
