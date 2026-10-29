// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! Central Linux errno mapping for FOD errors returned by the FUSE filesystem.

use libc::c_int;
use rust_hotpath::pg::{
    FILE_WRITE_ACCESS_DENIED, FILE_WRITE_TARGET_MISSING, NAMESPACE_ENTRY_EXISTS_PREFIX,
    NAMESPACE_PARENT_MISSING_PREFIX, NAMESPACE_SOURCE_CHANGED, PARENT_ACCESS_DENIED,
    STICKY_PERMISSION_DENIED, STORAGE_QUOTA_EXCEEDED_PREFIX, WRITE_OWNERSHIP_FENCE_REJECTED_PREFIX,
};

/// Converts a persisted-write domain error into the Linux errno exposed by FOD.
pub(crate) fn persist_error_errno(error: &str) -> c_int {
    if error.starts_with(STORAGE_QUOTA_EXCEEDED_PREFIX) {
        libc::ENOSPC
    } else if error.starts_with(FILE_WRITE_ACCESS_DENIED) {
        libc::EACCES
    } else if error.starts_with(FILE_WRITE_TARGET_MISSING) {
        libc::ENOENT
    } else if error.starts_with(WRITE_OWNERSHIP_FENCE_REJECTED_PREFIX) {
        libc::EBUSY
    } else {
        libc::EIO
    }
}

/// Converts a namespace mutation domain error into the Linux errno exposed by FOD.
pub(crate) fn namespace_mutation_errno(error: &str) -> c_int {
    if error == PARENT_ACCESS_DENIED {
        libc::EACCES
    } else if error.starts_with(NAMESPACE_ENTRY_EXISTS_PREFIX) {
        libc::EEXIST
    } else if error == STICKY_PERMISSION_DENIED {
        libc::EPERM
    } else if error.starts_with(NAMESPACE_PARENT_MISSING_PREFIX)
        || error == NAMESPACE_SOURCE_CHANGED
    {
        libc::ENOENT
    } else {
        libc::EIO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_write_errors_map_to_linux_errno() {
        assert_eq!(
            persist_error_errno(STORAGE_QUOTA_EXCEEDED_PREFIX),
            libc::ENOSPC
        );
        assert_eq!(persist_error_errno(FILE_WRITE_ACCESS_DENIED), libc::EACCES);
        assert_eq!(persist_error_errno(FILE_WRITE_TARGET_MISSING), libc::ENOENT);
        assert_eq!(
            persist_error_errno(WRITE_OWNERSHIP_FENCE_REJECTED_PREFIX),
            libc::EBUSY
        );
        assert_eq!(persist_error_errno("unclassified"), libc::EIO);
    }

    #[test]
    fn namespace_errors_map_to_linux_errno() {
        assert_eq!(namespace_mutation_errno(PARENT_ACCESS_DENIED), libc::EACCES);
        assert_eq!(
            namespace_mutation_errno(NAMESPACE_ENTRY_EXISTS_PREFIX),
            libc::EEXIST
        );
        assert_eq!(
            namespace_mutation_errno(STICKY_PERMISSION_DENIED),
            libc::EPERM
        );
        assert_eq!(
            namespace_mutation_errno(NAMESPACE_PARENT_MISSING_PREFIX),
            libc::ENOENT
        );
        assert_eq!(
            namespace_mutation_errno(NAMESPACE_SOURCE_CHANGED),
            libc::ENOENT
        );
        assert_eq!(namespace_mutation_errno("unclassified"), libc::EIO);
    }
}
