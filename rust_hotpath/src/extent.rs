// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

/// Inclusive block span used as a small PoC for extent-based storage planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub start_block: u64,
    pub end_block: u64,
}

impl Extent {
    pub fn new_checked(start_block: u64, end_block: u64) -> Option<Self> {
        if start_block > end_block {
            return None;
        }
        Some(Self {
            start_block,
            end_block,
        })
    }

    pub fn len(self) -> u64 {
        self.end_block
            .saturating_sub(self.start_block)
            .saturating_add(1)
    }

    pub fn to_range(self) -> (u64, u64) {
        (self.start_block, self.end_block)
    }
}

/// Merge sorted block indices into contiguous extents.
pub fn coalesce_sorted_blocks(blocks: &[u64]) -> Vec<Extent> {
    if blocks.is_empty() {
        return Vec::new();
    }

    let mut sorted_blocks = blocks.to_vec();
    sorted_blocks.sort_unstable();
    sorted_blocks.dedup();

    let mut extents = Vec::new();
    let mut start = sorted_blocks[0];
    let mut end = start;

    for block in sorted_blocks.into_iter().skip(1) {
        if block == end.saturating_add(1) {
            end = block;
            continue;
        }
        extents.push(Extent::new_checked(start, end).expect("coalesced extent must be ordered"));
        start = block;
        end = block;
    }

    extents.push(Extent::new_checked(start, end).expect("coalesced extent must be ordered"));
    extents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_reversed_extent() {
        assert_eq!(Extent::new_checked(5, 3), None);
    }
}
