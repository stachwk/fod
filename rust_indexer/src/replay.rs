// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_REQUEST_TOKEN: AtomicU64 = AtomicU64::new(1);

pub(crate) fn request_token(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let seq = NEXT_REQUEST_TOKEN.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!("{prefix}:{pid}:{nanos}:{seq}")
}
