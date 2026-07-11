// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub(crate) struct BlockWriteState {
    pub(crate) blocks: BTreeMap<u64, Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingSegment {
    pub(crate) start_offset: u64,
    pub(crate) payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SequentialSegmentState {
    pub(crate) start_offset: u64,
    pub(crate) next_offset: u64,
    pub(crate) segments: Vec<PendingSegment>,
    pub(crate) current: Vec<u8>,
}

impl SequentialSegmentState {
    pub(crate) fn new(start_offset: u64) -> Self {
        Self {
            start_offset,
            next_offset: start_offset,
            segments: Vec::new(),
            current: Vec::new(),
        }
    }

    pub(crate) fn append(&mut self, offset: u64, data: &[u8], target_bytes: u64) -> bool {
        if offset != self.next_offset || target_bytes == 0 {
            return false;
        }
        let Ok(target_bytes) = usize::try_from(target_bytes) else {
            return false;
        };
        if target_bytes == 0 {
            return false;
        }

        let mut cursor = 0usize;
        while cursor < data.len() {
            let available = target_bytes.saturating_sub(self.current.len());
            if available == 0 {
                self.finish_current();
                continue;
            }
            let take = available.min(data.len() - cursor);
            self.current.extend_from_slice(&data[cursor..cursor + take]);
            self.next_offset = self.next_offset.saturating_add(take as u64);
            cursor += take;
            if self.current.len() == target_bytes {
                self.finish_current();
            }
        }
        true
    }

    fn finish_current(&mut self) {
        if self.current.is_empty() {
            return;
        }
        let start_offset = self.next_offset.saturating_sub(self.current.len() as u64);
        self.segments.push(PendingSegment {
            start_offset,
            payload: std::mem::take(&mut self.current),
        });
    }

    fn current_start_offset(&self) -> u64 {
        self.next_offset.saturating_sub(self.current.len() as u64)
    }

    pub(crate) fn payload_bytes(&self) -> u64 {
        self.next_offset.saturating_sub(self.start_offset)
    }

    pub(crate) fn segment_count(&self) -> usize {
        self.segments.len() + usize::from(!self.current.is_empty())
    }

    pub(crate) fn segment_descriptors(&self) -> Vec<(u64, u64)> {
        let mut descriptors = self
            .segments
            .iter()
            .map(|segment| (segment.start_offset, segment.payload.len() as u64))
            .collect::<Vec<_>>();
        if !self.current.is_empty() {
            descriptors.push((self.current_start_offset(), self.current.len() as u64));
        }
        descriptors
    }

    pub(crate) fn into_segments(mut self) -> Vec<PendingSegment> {
        self.finish_current();
        self.segments
    }

    pub(crate) fn from_segments(segments: Vec<PendingSegment>) -> Self {
        let start_offset = segments
            .first()
            .map(|segment| segment.start_offset)
            .unwrap_or(0);
        let next_offset = segments
            .last()
            .map(|segment| {
                segment
                    .start_offset
                    .saturating_add(segment.payload.len() as u64)
            })
            .unwrap_or(start_offset);
        Self {
            start_offset,
            next_offset,
            segments,
            current: Vec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.payload_bytes() == 0
    }

    pub(crate) fn read_range(&self, offset: u64, end_offset: u64) -> Option<Vec<u8>> {
        if offset < self.start_offset || end_offset < offset || end_offset > self.next_offset {
            return None;
        }
        let output_len = usize::try_from(end_offset - offset).ok()?;
        let mut output = vec![0u8; output_len];
        let mut copied = 0usize;

        for segment in &self.segments {
            copied += copy_segment_overlap(
                segment.start_offset,
                &segment.payload,
                offset,
                end_offset,
                &mut output,
            );
        }
        copied += copy_segment_overlap(
            self.current_start_offset(),
            &self.current,
            offset,
            end_offset,
            &mut output,
        );

        (copied == output_len).then_some(output)
    }

    pub(crate) fn into_block_overlay(self, block_size: u64) -> BlockWriteState {
        let current_start_offset = self.current_start_offset();
        let Self {
            segments, current, ..
        } = self;
        let mut block_state = BlockWriteState::default();
        for segment in segments {
            overlay_bytes(
                &mut block_state.blocks,
                block_size,
                segment.start_offset,
                &segment.payload,
            );
        }
        overlay_bytes(
            &mut block_state.blocks,
            block_size,
            current_start_offset,
            &current,
        );
        block_state
    }
}

fn copy_segment_overlap(
    segment_offset: u64,
    payload: &[u8],
    read_offset: u64,
    read_end: u64,
    output: &mut [u8],
) -> usize {
    if payload.is_empty() {
        return 0;
    }
    let segment_end = segment_offset.saturating_add(payload.len() as u64);
    let copy_start = segment_offset.max(read_offset);
    let copy_end = segment_end.min(read_end);
    if copy_end <= copy_start {
        return 0;
    }

    let payload_start = (copy_start - segment_offset) as usize;
    let payload_end = (copy_end - segment_offset) as usize;
    let output_start = (copy_start - read_offset) as usize;
    let output_end = output_start + payload_end - payload_start;
    output[output_start..output_end].copy_from_slice(&payload[payload_start..payload_end]);
    output_end - output_start
}

fn overlay_bytes(blocks: &mut BTreeMap<u64, Vec<u8>>, block_size: u64, offset: u64, data: &[u8]) {
    if data.is_empty() {
        return;
    }
    let block_size = block_size.max(1);
    let block_size_usize = block_size.min(usize::MAX as u64) as usize;
    let mut cursor = 0usize;
    while cursor < data.len() {
        let absolute_offset = offset.saturating_add(cursor as u64);
        let block_index = absolute_offset / block_size;
        let block_offset = (absolute_offset % block_size) as usize;
        let copy_len = (block_size_usize - block_offset).min(data.len() - cursor);
        let block = blocks
            .entry(block_index)
            .or_insert_with(|| vec![0u8; block_size_usize]);
        block[block_offset..block_offset + copy_len]
            .copy_from_slice(&data[cursor..cursor + copy_len]);
        cursor += copy_len;
    }
}

#[derive(Debug, Clone)]
pub(crate) enum WritePayloadState {
    BlockOverlay(BlockWriteState),
    SequentialSegments(SequentialSegmentState),
}

impl Default for WritePayloadState {
    fn default() -> Self {
        Self::BlockOverlay(BlockWriteState::default())
    }
}

impl WritePayloadState {
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Self::BlockOverlay(state) => state.blocks.is_empty(),
            Self::SequentialSegments(state) => state.is_empty(),
        }
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn as_blocks(&self) -> Option<&BTreeMap<u64, Vec<u8>>> {
        match self {
            Self::BlockOverlay(state) => Some(&state.blocks),
            Self::SequentialSegments(_) => None,
        }
    }

    pub(crate) fn as_blocks_mut(&mut self) -> Option<&mut BTreeMap<u64, Vec<u8>>> {
        match self {
            Self::BlockOverlay(state) => Some(&mut state.blocks),
            Self::SequentialSegments(_) => None,
        }
    }

    pub(crate) fn as_sequential(&self) -> Option<&SequentialSegmentState> {
        match self {
            Self::BlockOverlay(_) => None,
            Self::SequentialSegments(state) => Some(state),
        }
    }

    pub(crate) fn as_sequential_mut(&mut self) -> Option<&mut SequentialSegmentState> {
        match self {
            Self::BlockOverlay(_) => None,
            Self::SequentialSegments(state) => Some(state),
        }
    }

    pub(crate) fn ensure_block_overlay(&mut self, block_size: u64) {
        if matches!(self, Self::BlockOverlay(_)) {
            return;
        }
        let previous = std::mem::take(self);
        let Self::SequentialSegments(state) = previous else {
            unreachable!();
        };
        *self = Self::BlockOverlay(state.into_block_overlay(block_size));
    }

    pub(crate) fn take_blocks(&mut self) -> BTreeMap<u64, Vec<u8>> {
        match std::mem::take(self) {
            Self::BlockOverlay(state) => state.blocks,
            Self::SequentialSegments(_) => unreachable!("payload must be downgraded first"),
        }
    }

    pub(crate) fn take_sequential(&mut self) -> Option<SequentialSegmentState> {
        if !matches!(self, Self::SequentialSegments(_)) {
            return None;
        }
        match std::mem::take(self) {
            Self::SequentialSegments(state) => Some(state),
            Self::BlockOverlay(_) => unreachable!(),
        }
    }

    pub(crate) fn restore_sequential(&mut self, state: SequentialSegmentState) {
        *self = Self::SequentialSegments(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_state_builds_bounded_segments() {
        let mut state = SequentialSegmentState::new(0);
        assert!(state.append(0, &[1; 10], 4));

        assert_eq!(state.start_offset, 0);
        assert_eq!(state.next_offset, 10);
        assert_eq!(state.segment_count(), 3);
        assert_eq!(state.segments.len(), 2);
        assert_eq!(state.segments[0].payload, vec![1; 4]);
        assert_eq!(state.segments[1].payload, vec![1; 4]);
        assert_eq!(state.current, vec![1; 2]);
    }

    #[test]
    fn sequential_state_rejects_gap_and_backward_write() {
        let mut state = SequentialSegmentState::new(0);
        assert!(state.append(0, b"abcd", 8));
        assert!(!state.append(6, b"gap", 8));
        assert!(!state.append(2, b"back", 8));
        assert_eq!(state.next_offset, 4);
        assert_eq!(state.current, b"abcd");
    }

    #[test]
    fn sequential_state_reads_across_segment_boundaries() {
        let mut state = SequentialSegmentState::new(0);
        assert!(state.append(0, b"abcdefghij", 4));

        assert_eq!(state.read_range(2, 9), Some(b"cdefghi".to_vec()));
        assert_eq!(state.read_range(0, 10), Some(b"abcdefghij".to_vec()));
        assert_eq!(state.read_range(0, 11), None);
    }

    #[test]
    fn sequential_state_downgrades_without_losing_partial_blocks() {
        let mut state = SequentialSegmentState::new(0);
        assert!(state.append(0, b"abcdefghij", 6));

        let block_state = state.into_block_overlay(4);
        assert_eq!(block_state.blocks.len(), 3);
        assert_eq!(block_state.blocks[&0], b"abcd");
        assert_eq!(block_state.blocks[&1], b"efgh");
        assert_eq!(block_state.blocks[&2], b"ij\0\0");
    }

    #[test]
    fn sequential_state_roundtrips_owned_segments_after_failure() {
        let mut state = SequentialSegmentState::new(0);
        assert!(state.append(0, b"abcdefghij", 4));

        let restored = SequentialSegmentState::from_segments(state.into_segments());
        assert_eq!(restored.segment_descriptors(), vec![(0, 4), (4, 4), (8, 2)]);
        assert_eq!(restored.read_range(0, 10), Some(b"abcdefghij".to_vec()));
    }
}
