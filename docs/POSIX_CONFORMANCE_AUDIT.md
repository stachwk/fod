# POSIX conformance audit

Status: **scoping started; no general POSIX conformance claim**.

## Scope and reference edition

The audit evaluates the behavior visible to applications using the Linux FUSE
mount, for the filesystem operations FOD implements or exposes through its
declared mount contract. The normative baseline is **POSIX.1-2024 (IEEE Std
1003.1-2024), The Open Group Base Specifications Issue 8**. When a requirement
is materially changed from POSIX.1-2017, record that difference rather than
silently mixing editions. POSIX.1-2024 is the currently published edition; the
technical corrigendum project is tracked separately and is not treated as
published normative text.

Primary references:

- [POSIX.1-2024 / Issue 8](https://pubs.opengroup.org/onlinepubs/9799919799/)
- [IEEE active standard record](https://standards.ieee.org/ieee/9945/12489/)
- [Austin Group Issue 415: accepted `SEEK_HOLE`/`SEEK_DATA` text](https://austingroupbugs.net/view.php?id=415#c862)
- [POSIX.1-2024 `open()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/open.html)
- [POSIX.1-2024 `read()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/read.html)
- [POSIX.1-2024 `write()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/write.html)
- [POSIX.1-2024 `lseek()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/lseek.html)
- [POSIX.1-2024 `stat()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/stat.html)
- [POSIX.1-2024 `utimensat()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/utimensat.html)
- [Linux kernel `fs/utimes.c`](https://github.com/torvalds/linux/blob/master/fs/utimes.c)
- [POSIX.1-2024 `rename()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html)

This is a filesystem compatibility audit, not a claim that FOD implements a
complete POSIX operating system. Shell and utility requirements, process and
terminal interfaces, device-driver behavior, and APIs absent from FOD's declared
filesystem contract are out of scope unless a concrete FOD mount behavior
depends on them. FUSE protocol behavior and Linux VFS behavior are recorded as
the delivery layer, not substituted for POSIX requirements.

POSIX ACL xattrs, Linux generic xattrs, Linux ioctl commands, SELinux, FUSE
capabilities, and FOD-specific extensions are tracked as separate Linux or FOD
compatibility contracts. They must not be presented as requirements from the
POSIX base specification. Multi-host ordering and PostgreSQL authority are
additional FOD correctness requirements and remain governed by
[architecture invariants](FOD_ARCHITECTURE_INVARIANTS.md).

## Status meanings

- **Pending** — requirement scope or evidence has not been reviewed against the
  normative text.
- **Pass** — every in-scope requirement in the row has implementation evidence
  and a passing targeted regression.
- **Fail** — a reproducible deviation from an in-scope requirement exists.
- **Not implemented** — the interface or required behavior is absent from FOD's
  declared support boundary; document whether that is an intentional exclusion.
- **Not applicable** — the normative requirement does not apply to the declared
  FOD interface; include the reason.

An existing test name is a starting point for evidence, not proof of compliance.
Every row must identify the normative section and the specific assertion before
its status can move from Pending.

## Initial requirement matrix

| Area | Normative interfaces / clauses to review | Existing FOD evidence to inspect | Status |
| --- | --- | --- | --- |
| Path resolution and name syntax | XSH `open`, `stat`, `mkdir`, `link`, `symlink`, `rename`, `unlink`; XBD pathname resolution | `rust_fuse/src/fs.rs`; `tests/integration/test_mount_suite.py`, `test_symlink.sh`, `test_rename_directory_transactional.py` | Pending |
| Directory operations and iteration | XSH `opendir`, `readdir`, `closedir`, `mkdir`, `rmdir`; XBD directory stream rules | FUSE callbacks in `rust_fuse/src/fs.rs`; `test_mount_suite.py`, `test_directories.sh`, `test_tree_scale.py`, `test_readdir_small_buffers.py`, `test_path_search_permissions.py` | Pending |
| Open and descriptor semantics | XSH `open`, `close`, `fcntl`, `dup`; access modes, append and create flags | `open`, `create`, `release`, `flush` callbacks; `test_mount_suite.py`, `test_create_write_ownership.py`, append-after-seek case in `test_lseek.py` | Pending |
| Read and write | XSH `read`, `write`, `pread`, `pwrite`; short I/O, EOF and offset rules | `test_lseek.py`, `test_mount_suite.py` | Pending |
| Standard file-offset positioning | XSH `lseek`, `SEEK_SET`, `SEEK_CUR`, `SEEK_END`; negative result, offset beyond EOF and no size change from seeking | Mounted `test_lseek.py` | Pass (listed cases) |
| Sparse file-offset positioning | Optional POSIX.1-2024 XSH `lseek` `SEEK_DATA` and `SEEK_HOLE` behavior; distinguish Issue 8 requirements from their pre-Issue-8 Linux/BSD history and Linux FUSE callback details | `test_lseek_sparse_runtime.py`, `test_lseek_sparse_contract.py`; callback `FodFuse::lseek` | Pass |
| Truncation and file length | XSH `truncate`, `ftruncate`, `open(O_TRUNC)` | `test_truncate_shrink_block_boundary.py`, `test_mount_suite.py`, Rust `mount_smoke` | Pending |
| Metadata and timestamps | XSH `stat`, `fstat`, `chmod`, `fchmod`, `chown`, `utimensat`; XBD file status fields | `test_mount_suite.py`, `test_permissions.py`, `test_timestamp_touch_once.py`, `test_utimens_noop` | Pending |
| Permissions and identity | XSH `access`, `umask`, `chmod`, `chown`; XBD file access permission rules | `test_permissions.py`, `test_access_groups.py`, `test_ownership_inheritance.py`, `test_root_owned_permissions.py` | Pending |
| Links and namespace mutation | XSH `link`, `unlink`, `rename`, `symlink`, `readlink`, `mkdir`, `rmdir`; atomicity and error precedence | `test_hardlink.sh`, `test_symlink.sh`, `test_rename_directory_transactional.py`, `test_chmod_rmdir.py`, primary-authority regressions | Pending |
| Special files and FIFOs | XSH FIFO creation/open/read/write behavior; classify Linux `mknod` and device-node provisioning separately | `test_mknod.py`, `test_mount_suite.py` | Pending |
| File locks | XSH `fcntl` record locking and applicable advisory-lock requirements | `getlk`/`setlk`; `test_mount_suite.py`, Rust lock smoke tests | Pending |
| Filesystem capacity and status | XSH `statvfs`; XBD `statvfs` fields and units | `statfs`; `test_df.sh`, two-mount quota tests, `docs/space-accounting.md` | Pending |
| Error behavior | XSH per-interface `ERRORS` and `RATIONALE`; errno precedence for supported operations | Targeted integration tests and `pjdfstest` subsets; map each error assertion to clause | Pending |
| Linux/FUSE extensions | Not POSIX base requirements; use Linux and FUSE references | `test_xattr.py`, `test_acl_mount_option.py`, `test_ioctl.py`, path-search xattr cases in `test_path_search_permissions.py`, compatibility contracts | Separate extension audit |

### Sparse `lseek` finding

Austin Group Issue 415's accepted text makes `SEEK_DATA` and `SEEK_HOLE` an
optional Issue 8 feature. It requires `ENXIO` when `offset >= EOF`; `SEEK_DATA`
also returns `ENXIO` when no non-hole byte remains. A hole is a contiguous run
of zero bytes, but an implementation need not classify every zero byte as a
hole. Every seekable file has a virtual hole beginning at EOF. The accepted
proposal did not add `_PC_MIN_HOLE_SIZE` to `fpathconf()`.

FOD implements the feature at storage-block granularity: absent payload blocks
read as zeroes and are reported as holes; persisted blocks are reported as
data, including blocks whose bytes may all be zero. This is permitted because
zero bytes need not be reported as holes. The mounted runtime matrix covers
dense files, punched and sparse gaps, trailing and empty files, EOF and
beyond-EOF errors, pending writes and remount. It also verifies `ENXIO` leaves
the open-file offset unchanged for both whence values and that the calls do not
mutate visible or PostgreSQL file state. `make test-lseek-sparse-runtime`
passed on FOD 3.4.104.

On Linux, the FUSE `lseek` callback is used for `SEEK_DATA` and `SEEK_HOLE`;
the kernel handles `SEEK_SET`, `SEEK_CUR` and `SEEK_END`. That division is
verified by the mounted standard-offset and sparse-offset tests. The finding
closes these listed sparse-seek requirements; it does not audit every
`lseek()` error case for every supported file type.

### Timestamp authorization finding

Issue 8 `utimensat()` permits a non-owner with write access to set both times to
the current time (or to omit both); setting specific times requires ownership or
appropriate privilege. The mounted ACL regression grants a named user write
access, changes the file owner to `nobody`, verifies the named user can set
current times, and verifies a specific-time update fails with `EPERM` without
changing mtime. The regression also calls `utimensat()` as
`nobody` with both times set to `UTIME_OMIT` on a file whose ACL denies access;
the call succeeds and leaves atime and mtime unchanged. It also succeeds for a
nonexistent path on the mounted FOD directory. Linux `fs/utimes.c` returns from
the `utimensat` syscall before pathname lookup when both fields are
`UTIME_OMIT`, so those two checks verify Linux VFS behavior and do not exercise a
FOD callback. A separate owner test sends `UTIME_OMIT` for atime and `UTIME_NOW`
for mtime; mtime advances while atime stays unchanged, exercising the mounted
filesystem path. All cases passed in `make test-acl-mount-option` on FOD
3.4.110 and kernel `6.17.0-41-generic`. The broader metadata and timestamp row
remains Pending.

### Logical root directory metadata finding

The mounted root-permissions regression records root `st_mtime_ns` and
`st_ctime_ns` before and after creating and removing a direct child. Both values
must advance after each namespace mutation. `make test-mount-root-permissions`
passed on FOD 3.4.111. A second regression mutates the root through one mount
and observes root metadata through another with the metadata TTL set to zero.
It verifies that `st_nlink` increases and decreases with direct child
directories, root inode, mode, UID and GID stay stable, and root `atime`
advances after `readdir` under `strictatime`. The earlier test passed on FOD
3.4.120 for namespace timestamps, link count and identity. The atime extension
reproduced a bug: `readdir` skipped root attributes, so the final atime update
was skipped too. The fix fetches root attributes without its synthesized link
count and updates the stored PostgreSQL directory record. The expanded test
passed on FOD 3.4.122. This still does not cover every root metadata field, so
the metadata row remains Pending.

### Directory iteration cookie finding

The FUSE `readdir` callback assigns cookies 1 and 2 to `.` and `..`, then
`index + 3` to each child. Linux resumes iteration using the cookie returned
for the last entry. Earlier code treated that cookie as a count of child rows
to skip, which omitted entries when `getdents64()` continued after filling a
small userspace buffer. It also failed to return `..` if iteration resumed at
cookie 1. Both batch-metadata and blob-backed paths now resume children after
`offset - 2`, and emit dot entries only when their cookies are beyond the
incoming offset. The mounted small-buffer regression uses 64-byte
`getdents64()` buffers and checks for missing or duplicate names. The broader
directory-iteration row remains Pending until its normative requirements and
other error and mutation cases are reviewed.

The permission review found that `readdir` relied on Linux's
`default_permissions` mount option. When that option was disabled, the callback
returned names without checking directory read permission. A path-resolution
check found the same gap in `lookup` and cached-inode operations: a user could
stat or open a child through a directory without search permission. The
callbacks now check directory `R_OK` before listing and `X_OK` on each parent
before lookup, path-based `getattr` or `open`, using FOD's mode and POSIX ACL
authorization path. These extra checks are skipped when kernel
`default_permissions` checks are active. The regression mounts with
`default_permissions` disabled and `allow_other` enabled, sets a directory to
mode `000`, and verifies the unrelated `nobody` user receives `EACCES` both
when listing it, statting a child through it and reading a child file. The
listing and stat failures were reproduced before their fixes; the read case is
covered by the targeted regression. The full targeted test passes after the
path-search checks. The same test checks that PostgreSQL-authoritative parent
write/search authorization rejects `create`, `mkdir`, `unlink`, `rename` and
`rmdir` in the mode-000 directory. It also creates a file owned by `nobody`
before removing directory permissions, then verifies `chmod` and `utimens`
cannot reach that file by path, and path-based `truncate` is denied. On a
searchable parent it also verifies that `truncate` and `open(O_TRUNC)` reject
a file without write permission. Linux xattr calls (`getxattr`, `listxattr`,
`setxattr` and `removexattr`) fail through the inaccessible parent. This is
extension coverage, not a POSIX base claim. Other path-resolution requirements
remain under review.

## Audit procedure

1. For each row, list exact Issue 8 section and option-group applicability.
2. Compare its normative conditions, results, errors, and atomicity rules with
   the callback and storage transaction that implements them.
3. Run or add the narrowest deterministic mounted regression; use `pjdfstest`
   where a relevant case exists and record skipped cases with their reason.
4. Mark each requirement individually Pass, Fail, Not implemented, or Not
   applicable, then summarize the row. Keep unreviewed requirements Pending.
5. Fix reproducible in-scope failures in focused changes. Document intentional
   exclusions and the verified support boundary in
   [compatibility contracts](compatibility-contracts.md).

The audit is complete only when every in-scope requirement has evidence and
every excluded interface has a stated reason. Do not infer conformance from
successful common-command smoke tests or from FUSE capability negotiation.
