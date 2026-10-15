#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX03H contract: absent file is ENOENT, invalid session remains EIO."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
hotpath = (ROOT / "rust_hotpath/src/pg.rs").read_text()
fuse = (ROOT / "rust_fuse/src/fs.rs").read_text()

start = hotpath.index("pub fn register_file_open_lease(")
end = hotpath.index("pub fn release_file_open_lease(", start)
registration = hotpath[start:end]

start = fuse.index("fn register_file_open_lease_for_handle(")
end = fuse.index("fn create_handle_for_file(", start)
mapping = fuse[start:end]

for marker in (
    "Self::advisory_xact_lock_text_on_conn(conn, &resource_key)?",
    "WHEN NOT EXISTS (",
    "FROM client_sessions",
    "lease_expires_at > clock_timestamp()",
    "FROM files",
    "AND NOT unlinked",
    'Err("FOD_OPEN_FILE_UNAVAILABLE".to_string())',
    'Err("FOD_OPEN_SESSION_INVALID".to_string())',
    'Err("FOD_OPEN_LEASE_INCONSISTENT".to_string())',
):
    assert marker in registration, f"TX03H missing classification contract: {marker}"

assert 'if err == "FOD_OPEN_FILE_UNAVAILABLE" {' in mapping
assert 'ENOENT\n            } else {\n                EIO' in mapping
assert 'if err == "file open lease requires an active client session"' not in mapping
print("TX03H_CLASSIFICATION_CONTRACT=PASS file_unavailable=ENOENT session_invalid=EIO")
