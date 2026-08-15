// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub(crate) struct BlockWriteState {
    pub(crate) blocks: BTreeMap<u64, Vec<u8>>,
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
            Self::BlockOverlay(state) => Some(&state.blocks),
        }
    }

    pub(crate) fn as_blocks_mut(&mut self) -> Option<&mut BTreeMap<u64, Vec<u8>>> {
        match self {
            Self::BlockOverlay(state) => Some(&mut state.blocks),
        }
    }

    pub(crate) fn ensure_block_overlay(&mut self, block_size: u64) {
        let _ = block_size;
    }

    pub(crate) fn take_blocks(&mut self) -> BTreeMap<u64, Vec<u8>> {
        match std::mem::take(self) {
            Self::BlockOverlay(state) => state.blocks,
        }
    }
}
