// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub(crate) struct BlockWriteState {
    // WriteState snapshots are cloned by read-after-write paths. Keep the
    // whole overlay behind Arc so cloning a snapshot is O(1) instead of
    // copying every dirty block. Writers use Arc::make_mut below, which
    // preserves snapshot isolation with copy-on-write when a read is active.
    pub(crate) blocks: Arc<BTreeMap<u64, Vec<u8>>>,
}

#[derive(Debug, Clone)]
pub(crate) enum WritePayloadState {
    BlockOverlay(BlockWriteState),
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
        }
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn as_blocks(&self) -> Option<&BTreeMap<u64, Vec<u8>>> {
        match self {
            Self::BlockOverlay(state) => Some(state.blocks.as_ref()),
        }
    }

    pub(crate) fn as_blocks_mut(&mut self) -> Option<&mut BTreeMap<u64, Vec<u8>>> {
        match self {
            Self::BlockOverlay(state) => Some(Arc::make_mut(&mut state.blocks)),
        }
    }

    pub(crate) fn ensure_block_overlay(&mut self, block_size: u64) {
        let _ = block_size;
    }

    pub(crate) fn take_blocks(&mut self) -> BTreeMap<u64, Vec<u8>> {
        match std::mem::take(self) {
            Self::BlockOverlay(state) => match Arc::try_unwrap(state.blocks) {
                Ok(blocks) => blocks,
                Err(shared) => shared.as_ref().clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_shares_block_map_until_mutation() {
        let mut payload = WritePayloadState::default();
        payload
            .as_blocks_mut()
            .expect("block overlay")
            .insert(7, vec![1, 2, 3, 4]);

        let mut snapshot = payload.clone();

        match (&payload, &snapshot) {
            (WritePayloadState::BlockOverlay(left), WritePayloadState::BlockOverlay(right)) => {
                assert!(Arc::ptr_eq(&left.blocks, &right.blocks))
            }
        }

        snapshot
            .as_blocks_mut()
            .expect("block overlay")
            .insert(8, vec![5, 6, 7, 8]);

        assert!(payload
            .as_blocks()
            .expect("block overlay")
            .get(&8)
            .is_none());
        assert_eq!(
            snapshot
                .as_blocks()
                .expect("block overlay")
                .get(&8)
                .map(Vec::as_slice),
            Some(&[5, 6, 7, 8][..])
        );

        match (&payload, &snapshot) {
            (WritePayloadState::BlockOverlay(left), WritePayloadState::BlockOverlay(right)) => {
                assert!(!Arc::ptr_eq(&left.blocks, &right.blocks))
            }
        }
    }

    #[test]
    fn take_blocks_preserves_shared_snapshot() {
        let mut payload = WritePayloadState::default();
        payload
            .as_blocks_mut()
            .expect("block overlay")
            .insert(11, vec![9, 8, 7, 6]);

        let snapshot = payload.clone();
        let blocks = payload.take_blocks();

        assert!(payload.is_empty());
        assert_eq!(blocks.get(&11).map(Vec::as_slice), Some(&[9, 8, 7, 6][..]));
        assert_eq!(
            snapshot
                .as_blocks()
                .expect("block overlay")
                .get(&11)
                .map(Vec::as_slice),
            Some(&[9, 8, 7, 6][..])
        );
    }
}
