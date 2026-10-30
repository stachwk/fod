// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! Central Linux errno mapping for FOD errors returned by the FUSE filesystem.

use libc::c_int;
pub(crate) use libc::{
    EACCES, EBADF, EBUSY, EEXIST, EFBIG, EILSEQ, EINTR, EINVAL, EIO, EISDIR, ENAMETOOLONG, ENODATA,
    ENOENT, ENOSPC, ENOTDIR, ENOTEMPTY, ENOTTY, ENXIO, EOPNOTSUPP, EOVERFLOW, EPERM, ERANGE,
    EROFS, ESTALE, EWOULDBLOCK,
};
use rust_hotpath::pg::{
    FILE_WRITE_ACCESS_DENIED, FILE_WRITE_TARGET_MISSING, NAMESPACE_ENTRY_EXISTS_PREFIX,
    NAMESPACE_PARENT_MISSING_PREFIX, NAMESPACE_SOURCE_CHANGED, PARENT_ACCESS_DENIED,
    STICKY_PERMISSION_DENIED, STORAGE_QUOTA_EXCEEDED_PREFIX, WRITE_OWNERSHIP_FENCE_REJECTED_PREFIX,
};

const POSTGRES_DISK_FULL_SQLSTATE: &str = "SQLSTATE 53100:";

/// Converts a persisted-write domain error into the Linux errno exposed by FOD.
pub(crate) fn persist_error_errno(error: &str) -> c_int {
    if error.starts_with(STORAGE_QUOTA_EXCEEDED_PREFIX)
        || error.starts_with(POSTGRES_DISK_FULL_SQLSTATE)
    {
        ENOSPC
    } else if error.starts_with(FILE_WRITE_ACCESS_DENIED) {
        EACCES
    } else if error.starts_with(FILE_WRITE_TARGET_MISSING) {
        ENOENT
    } else if error.starts_with(WRITE_OWNERSHIP_FENCE_REJECTED_PREFIX) {
        EBUSY
    } else {
        EIO
    }
}

/// Converts a namespace mutation domain error into the Linux errno exposed by FOD.
pub(crate) fn namespace_mutation_errno(error: &str) -> c_int {
    if error.starts_with(POSTGRES_DISK_FULL_SQLSTATE) {
        ENOSPC
    } else if error == PARENT_ACCESS_DENIED {
        EACCES
    } else if error.starts_with(NAMESPACE_ENTRY_EXISTS_PREFIX) {
        EEXIST
    } else if error == STICKY_PERMISSION_DENIED {
        EPERM
    } else if error.starts_with(NAMESPACE_PARENT_MISSING_PREFIX)
        || error == NAMESPACE_SOURCE_CHANGED
    {
        ENOENT
    } else {
        EIO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_errno_aliases_match() {
        assert_eq!(libc::EAGAIN, EWOULDBLOCK);
        assert_eq!(libc::ENOTSUP, EOPNOTSUPP);
    }

    #[test]
    fn persisted_write_errors_map_to_linux_errno() {
        assert_eq!(
            persist_error_errno(STORAGE_QUOTA_EXCEEDED_PREFIX),
            ENOSPC
        );
        assert_eq!(persist_error_errno("SQLSTATE 53100: disk full"), ENOSPC);
        assert_eq!(persist_error_errno(FILE_WRITE_ACCESS_DENIED), EACCES);
        assert_eq!(persist_error_errno(FILE_WRITE_TARGET_MISSING), ENOENT);
        assert_eq!(
            persist_error_errno(WRITE_OWNERSHIP_FENCE_REJECTED_PREFIX),
            EBUSY
        );
        assert_eq!(persist_error_errno("unclassified"), EIO);
    }

    #[test]
    fn namespace_errors_map_to_linux_errno() {
        assert_eq!(
            namespace_mutation_errno("SQLSTATE 53100: disk full"),
            ENOSPC
        );
        assert_eq!(namespace_mutation_errno(PARENT_ACCESS_DENIED), EACCES);
        assert_eq!(
            namespace_mutation_errno(NAMESPACE_ENTRY_EXISTS_PREFIX),
            EEXIST
        );
        assert_eq!(
            namespace_mutation_errno(STICKY_PERMISSION_DENIED),
            EPERM
        );
        assert_eq!(
            namespace_mutation_errno(NAMESPACE_PARENT_MISSING_PREFIX),
            ENOENT
        );
        assert_eq!(
            namespace_mutation_errno(NAMESPACE_SOURCE_CHANGED),
            ENOENT
        );
        assert_eq!(namespace_mutation_errno("unclassified"), EIO);
    }
}
