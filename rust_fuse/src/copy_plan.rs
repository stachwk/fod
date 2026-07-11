// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CopyRangeBounds {
    pub(crate) src_offset: u64,
    pub(crate) dst_offset: u64,
    pub(crate) copy_len: u64,
    pub(crate) src_end_offset: u64,
    pub(crate) dst_end_offset: u64,
    pub(crate) src_first_block: u64,
    pub(crate) src_last_block: u64,
    pub(crate) dst_first_block: u64,
    pub(crate) dst_last_block: u64,
}

impl CopyRangeBounds {
    pub(crate) fn can_adopt_whole_object(
        self,
        src_size: u64,
        dst_size: u64,
        src_dirty: bool,
        dst_dirty: bool,
    ) -> bool {
        self.src_offset == 0
            && self.dst_offset == 0
            && self.copy_len == src_size
            && dst_size == 0
            && !src_dirty
            && !dst_dirty
    }
}

pub(crate) fn copy_range_bounds(
    block_size: u64,
    src_offset: u64,
    dst_offset: u64,
    len: u64,
    src_size: u64,
) -> Option<CopyRangeBounds> {
    if src_offset >= src_size {
        return None;
    }
    let copy_len = len.min(src_size - src_offset);
    if copy_len == 0 {
        return None;
    }
    let block_size = block_size.max(1);
    let src_end_offset = src_offset.saturating_add(copy_len);
    let dst_end_offset = dst_offset.saturating_add(copy_len);
    let src_first_block = src_offset / block_size;
    let src_last_block = (src_end_offset.saturating_sub(1)) / block_size;
    let dst_first_block = dst_offset / block_size;
    let dst_last_block = (dst_end_offset.saturating_sub(1)) / block_size;
    Some(CopyRangeBounds {
        src_offset,
        dst_offset,
        copy_len,
        src_end_offset,
        dst_end_offset,
        src_first_block,
        src_last_block,
        dst_first_block,
        dst_last_block,
    })
}

pub(crate) fn pack_copy_skip_unchanged_runs(
    dst_offset: u64,
    block_size: u64,
    payload: &[u8],
    current: &[u8],
) -> Vec<(u64, Vec<u8>)> {
    let block_size = block_size.max(1) as usize;
    let mut runs = Vec::new();
    let mut run_start = None;
    let mut run_payload = Vec::new();
    let mut rel = 0usize;

    while rel < payload.len() {
        let chunk_len = (payload.len() - rel).min(block_size);
        let chunk = &payload[rel..rel + chunk_len];
        let same = if rel + chunk_len <= current.len() {
            let current_chunk = &current[rel..rel + chunk_len];
            current_chunk == chunk
        } else {
            let mut padded = vec![0u8; chunk_len];
            if rel < current.len() {
                let available = current.len() - rel;
                padded[..available].copy_from_slice(&current[rel..]);
            }
            padded == chunk
        };

        if same {
            if let Some(start) = run_start.take() {
                runs.push((dst_offset + start as u64, std::mem::take(&mut run_payload)));
            }
        } else {
            if run_start.is_none() {
                run_start = Some(rel);
            }
            run_payload.extend_from_slice(chunk);
        }

        rel += chunk_len;
    }

    if let Some(start) = run_start.take() {
        runs.push((dst_offset + start as u64, run_payload));
    }

    runs
}

#[cfg(test)]
mod tests {
    use super::{copy_range_bounds, pack_copy_skip_unchanged_runs, CopyRangeBounds};

    #[test]
    fn copy_range_bounds_keep_source_and_destination_offsets_separate() {
        let bounds = copy_range_bounds(8, 8, 24, 16, 128).expect("expected bounds");
        assert_eq!(
            bounds,
            CopyRangeBounds {
                src_offset: 8,
                dst_offset: 24,
                copy_len: 16,
                src_end_offset: 24,
                dst_end_offset: 40,
                src_first_block: 1,
                src_last_block: 2,
                dst_first_block: 3,
                dst_last_block: 4,
            }
        );
    }

    #[test]
    fn copy_range_bounds_clamp_to_source_size() {
        let bounds = copy_range_bounds(8, 120, 24, 32, 128).expect("expected bounds");
        assert_eq!(bounds.copy_len, 8);
        assert_eq!(bounds.src_end_offset, 128);
        assert_eq!(bounds.dst_end_offset, 32);
        assert_eq!(bounds.src_first_block, 15);
        assert_eq!(bounds.src_last_block, 15);
        assert_eq!(bounds.dst_first_block, 3);
        assert_eq!(bounds.dst_last_block, 3);
    }

    #[test]
    fn whole_object_adoption_requires_clean_full_copy_to_empty_destination() {
        let bounds = copy_range_bounds(8, 0, 0, 128, 128).expect("expected bounds");
        assert!(bounds.can_adopt_whole_object(128, 0, false, false));
        assert!(!bounds.can_adopt_whole_object(128, 1, false, false));
        assert!(!bounds.can_adopt_whole_object(128, 0, true, false));
        assert!(!bounds.can_adopt_whole_object(128, 0, false, true));

        let partial = copy_range_bounds(8, 0, 0, 64, 128).expect("expected bounds");
        assert!(!partial.can_adopt_whole_object(128, 0, false, false));
        let shifted = copy_range_bounds(8, 8, 0, 120, 128).expect("expected bounds");
        assert!(!shifted.can_adopt_whole_object(128, 0, false, false));
    }

    #[test]
    fn pack_copy_skip_unchanged_runs_groups_changed_blocks() {
        let payload = b"AAAA1111BBBB2222";
        let current = b"AAAA0000BBBB";

        assert_eq!(
            pack_copy_skip_unchanged_runs(100, 4, payload, current),
            vec![(104, b"1111".to_vec()), (112, b"2222".to_vec())]
        );
    }

    #[test]
    fn pack_copy_skip_unchanged_runs_skips_zero_padded_tail() {
        let payload = b"ABCD\x00\x00";
        let current = b"ABCD";

        assert!(pack_copy_skip_unchanged_runs(24, 4, payload, current).is_empty());
    }
}
