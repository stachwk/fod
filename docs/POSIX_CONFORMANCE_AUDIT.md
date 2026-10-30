# POSIX conformance audit

Status as of FOD 3.4.380 (`68d23b7`): **requirement audit in progress; no
general POSIX conformance claim**.

The initial matrix marks only the tested standard `lseek()` and sparse
`SEEK_DATA`/`SEEK_HOLE` case groups Pass. The remaining groups stay Pending
while any applicable requirement or required error is unmapped. The P01-P12
ledger below records the bounded closure evidence; tests collected across
multiple FOD releases are evidence for individual cases, not a claim that the
complete current tree passes every row. The latest broad mounted evidence
recorded by this audit uses Linux 6.17 and libfuse 3.17.4.

For the compact three-layer status, read
[`compatibility-contracts.md`](compatibility-contracts.md). Linux/FUSE runtime
minimums and the unverified minimum-stack test gap are summarized in
[`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md).

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
- [POSIX.1-2024 `readdir()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/readdir.html)
- [POSIX.1-2024 `truncate()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/truncate.html)
- [POSIX.1-2024 `fcntl()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/fcntl.html)
- [POSIX.1-2024 `mkfifo()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/mkfifo.html)
- [POSIX.1-2024 `statvfs()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/statvfs.html)

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

## Requirement matrix

| Area | Normative interfaces / clauses to review | Existing FOD evidence to inspect | Status |
| --- | --- | --- | --- |
| Path resolution and name syntax | XSH `open`, `stat`, `mkdir`, `link`, `symlink`, `rename`, `unlink`; XBD pathname resolution | `rust_fuse/src/fs.rs`; `test_path_trailing_slash.py` checks `ENOTDIR` for regular files and symlinks to files, plus directory and symlink-to-directory resolution; `test_directories.sh` checks `O_CREAT|O_EXCL` file creation below a regular file returns `ENOTDIR` without changing that file; `test_symlink.sh` checks `stat` follows a valid link while `lstat` reports the link inode, `EINVAL` for `readlink` on a regular file, short-buffer behavior without NUL termination, `lstat` target-byte `st_size`, unlink preserving the target, dangling-link errors, `O_CREAT|O_EXCL` returning `EEXIST`, and `ELOOP`; `test_mount_suite.py`, `test_rename_directory_transactional.py` | Pending |
| Directory operations and iteration | XSH `opendir`, `readdir`, `closedir`, `mkdir`, `rmdir`; XBD directory stream rules | FUSE callbacks in `rust_fuse/src/fs.rs`; `test_directories.sh` checks `EEXIST` for `mkdir` on an existing file or directory, `EISDIR`/`ENOENT` for invalid `unlink` and `ENOENT`/`ENOTDIR` (including `rmdir` on a symlink to a directory)/`ENOTEMPTY` for invalid `rmdir`, and `ENOENT`/`ENOTDIR` when file creation, `mkdir` or `rmdir` traverses a missing or non-directory parent; failed child creation preserves the parent file contents; `test_mount_suite.py`, `test_tree_scale.py`, `test_readdir_small_buffers.py`; `test_path_search_permissions.py` checks exact libc `opendir()` `EACCES` on a mode-000 directory and passed on FOD 3.4.374 with a disposable database | Pending |
| Open and descriptor semantics | XSH `open`, `close`, `fcntl`, `dup`; access modes, append and create flags | `open`, `create`, `release`, `flush` callbacks; `test_mount_suite.py`, `test_create_write_ownership.py` (`O_EXCL` conflict/recovery and exclusive cross-mount write lease), `test_symlink.sh` verifies `O_TRUNC` follows a valid symlink and `O_CREAT` creates the target through a dangling symlink while retaining the link; append-after-seek, shared-`dup`-offset and descriptor access-mode cases in `test_lseek.py`; `mount_smoke::cross_mount_append_uses_fresh_kernel_file_size` verifies append offsets after another mount changes EOF | Pending |
| Read and write | XSH `read`, `write`, `pread`, `pwrite`; short I/O, EOF, access-mode errors and offset rules | `test_lseek.py` (including EOF short reads, zero-byte writes, `EBADF` for incompatible descriptor modes, and `pread`/`pwrite` offset preservation), `test_mount_suite.py`; `mount_smoke::cross_mount_append_uses_fresh_kernel_file_size` verifies the second mount writes at the PostgreSQL-current EOF after acquiring the lease | Pending |
| Standard file-offset positioning | XSH `lseek`, `SEEK_SET`, `SEEK_CUR`, `SEEK_END`; negative result, offset beyond EOF and no size change from seeking | Mounted `test_lseek.py` | Pass (listed cases) |
| Sparse file-offset positioning | Optional POSIX.1-2024 XSH `lseek` `SEEK_DATA` and `SEEK_HOLE` behavior; distinguish Issue 8 requirements from their pre-Issue-8 Linux/BSD history and Linux FUSE callback details | `test_lseek_sparse_runtime.py`, `test_lseek_sparse_contract.py`; callback `FodFuse::lseek` | Pass |
| Truncation and file length | XSH `truncate`, `ftruncate`, `open(O_TRUNC)` | `test_truncate_shrink_block_boundary.py` covers block-boundary and zero-fill behavior, descriptor-offset preservation, and POSIX-permitted `EINVAL`/`EBADF` on a read-only descriptor; `test_mount_suite.py`, Rust `mount_smoke` | Pending |
| Metadata and timestamps | XSH `stat`, `fstat`, `chmod`, `fchmod`, `chown`, `utimensat`; XBD file status fields | `test_mount_suite.py`, `test_permissions.py`, `test_timestamp_touch_once.py`, `test_utimens_noop`; `test_symlink.sh` checks `utimens` through a symlink at PostgreSQL's microsecond resolution and verifies the symlink mtime is preserved | Pending |
| Permissions and identity | XSH `access`, `umask`, `chmod`, `chown`; XBD file access permission rules | `test_permissions.py`, `test_access_groups.py`, `test_ownership_inheritance.py` checks umask masking and setgid parent GID inheritance for files, directories, symlinks and FIFOs, including recursive setgid propagation to child directories; `test_symlink.sh` verifies `chmod` through a symlink changes target mode; `test_root_owned_permissions.py`. Linux v6.17 `inode_init_owner()` assigns a new inode the setgid parent's GID and always propagates `S_ISGID` to child directories ([`fs/inode.c`](https://github.com/torvalds/linux/blob/v6.17/fs/inode.c#L2540-L2567)). Linux v6.17 [`do_faccessat()`](https://github.com/torvalds/linux/blob/v6.17/fs/open.c#L2765-L2772) switches fsuid/fsgid to real uid/gid for `access()`; FUSE request headers carry fsuid/fsgid ([kernel source](https://github.com/torvalds/linux/blob/v6.17/fs/fuse/dev.c#L2552-L2566)), so `req.uid()`/`req.gid()` already represent those IDs for ordinary `access()` (unlike `AT_EACCESS`). Mounted `test_path_search_permissions.py` distinguishes real UID `nobody` from effective UID 0 and checks root `X_OK` with and without an execute bit. For ownership changes, `test_permissions.py` checks regular-file set-ID clearing, exact preservation of directory mode `06755` after `chgrp`, and removal of Linux `security.capability` on regular-file `chown` (tracked separately as a Linux extension); Linux v6.17 `chown_common()` requests set-ID clearing only for non-directories [source](https://github.com/torvalds/linux/blob/v6.17/fs/open.c#L3434-L3438) | Pending |
| Links and namespace mutation | XSH `link`, `unlink`, `rename`, `symlink`, `readlink`, `mkdir`, `rmdir`; atomicity and error precedence | `test_hardlink.sh` checks `EEXIST` for an occupied target, `ENOENT` for a missing source and `EPERM` for a directory source; `test_directories.sh` checks `ENOTDIR` for hard-link and symbolic-link destinations and rename targets beneath a regular file, with source and parent-file preservation; `test_rename_directory_transactional.py` checks `ENOENT` for a missing rename source while preserving the destination on both mounts; `test_symlink.sh`, `test_chmod_rmdir.py`, primary-authority regressions | Pending |
| Special files and FIFOs | XSH FIFO creation/open/read/write behavior; classify Linux `mknod` and device-node provisioning separately | `test_mknod.py` passed on FOD 3.4.380 against a fresh disposable PostgreSQL 16.15 database on Linux 6.17/libfuse 3.17.4: regular `mknod`, FIFO type/mode, nonblocking writer `ENXIO`, `EPIPE` after the last reader closes, `ESPIPE` for `pread()`/`pwrite()`, nonblocking empty-read `EAGAIN`, blocking writer handoff, transfer/EOF and concurrent `_PC_PIPE_BUF` record atomicity. Character/block device creation is skipped by FOD with `EPERM`; these Linux device nodes are outside the POSIX FIFO claim. The mounted setgid inheritance regression also verifies FIFO GID inheritance | Map remaining FIFO open/read/write interruption and error requirements; keep Linux character/block devices outside the POSIX FIFO claim |
| File locks | XSH `fcntl()` `F_GETLK`, `F_SETLK`, and `F_SETLKW`; POSIX requires record locks for regular files and permits implementations to support other types | `rust_fuse/tests/lock_backend_smoke.rs` checks regular-file read/write conflict queries and nonblocking `EACCES`/`EWOULDBLOCK`, blocking `F_SETLKW` handoff after a cross-mount unlock, signal interruption returning `EINTR` with no later lock acquisition, and a cross-range cycle between two processes on separate mounts returning `EDEADLK`; it also checks conflicts across primary mounts and visibility from a replica, byte-range boundaries, partial unlock and overlapping-range replacement, lease expiry, and POSIX close-triggered process unlock. PostgreSQL stores the wait-for edges keyed by host and caller PID and serializes cycle checks with lock-range updates. FIFO locks are outside FOD cross-mount guarantees because Linux FUSE handles them locally without invoking FOD callbacks | Pending |
| Filesystem capacity and status | POSIX.1-2024 XSH `statvfs()`/`fstatvfs()` and XBD `<sys/statvfs.h>`: `f_blocks`/free counts use `f_frsize` units; `f_bavail`/`f_favail` are available to non-privileged processes; `f_flag` defines `ST_RDONLY` and `ST_NOSUID`; `f_namemax` is the component-name limit | Linux FUSE `statfs` callback in `rust_fuse/src/fs.rs` supplies block counts, inode counts, block size and name limit; Linux derives mount flags. `test_df.sh` checks `f_bsize`/`f_frsize` positive and equal, block and inode count ordering, path/descriptor parity, `ST_RDONLY`/`ST_NOSUID`, name limit 255, backing-filesystem capacity cap, stable virtual inode headroom of 1,000,000 and symlink inode-count updates. It also checks `statvfs()` `EACCES` (when unprivileged), `ELOOP`, `ENAMETOOLONG`, `ENOENT`, `ENOTDIR`, and Linux empty-path `ENOENT`, plus `fstatvfs()` `EBADF`. `make test-df` passed twice on FOD 3.4.171, including the unprivileged `EACCES` case. Accounting checks compare used blocks with PostgreSQL payload plus active reservations before and after remount. `test_two_mount_quota.py` covers shared quota admission. FOD reports synthetic inode headroom because PostgreSQL-backed FOD has no finite inode pool; it sets `f_favail == f_ffree` because it has no separate non-privileged inode quota. Linux FUSE protocol has no `f_fsid` reply field, and Linux v6.17 explicitly leaves `kstatfs.f_fsid` zero, so FOD cannot supply a filesystem ID through this callback. Source review confirms FOD maps statfs snapshot or backing-capacity failures to `EIO`; `test_statfs_eio.py` injects an `EIO` callback reply and checks that `statvfs()` returns it. This does not simulate a real PostgreSQL failure. POSIX also requires `EINTR` for a caught signal and `EOVERFLOW` when a result cannot fit the caller's structure. Linux can produce `EOVERFLOW` in a 32-bit compatibility `statfs` syscall while copying the kernel result; this syscall path is outside FOD's supported architecture matrix. Remaining gaps: determine whether signal-interrupted `statvfs()` needs a mounted regression and validate applicable `EOVERFLOW` cases on supported 64-bit-and-wider ABIs. See [POSIX header definition](https://pubs.opengroup.org/onlinepubs/9799919799/basedefs/sys_statvfs.h.html), [POSIX `statvfs()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/statvfs.html), [POSIX Programmer's Manual `statvfs()` errors](https://man7.org/linux/man-pages/man3/fstatvfs.3p.html#errors), [Linux FUSE v6.17 statfs conversion](https://github.com/torvalds/linux/blob/v6.17/fs/fuse/inode.c#L3196-L3219), [Linux v6.17 `statfs` overflow checks](https://github.com/torvalds/linux/blob/v6.17/fs/statfs.c#L1423-L1450), and [Linux `statvfs(3)` errors](https://man7.org/linux/man-pages/man3/statvfs.3.html#errors) | Pending |
| Error behavior | XSH per-interface `ERRORS` and `RATIONALE`; errno precedence for supported operations | [`ERRNO_REFERENCE.md`](ERRNO_REFERENCE.md) summarizes common codes and test evidence; targeted integration tests and `pjdfstest` subsets | Pending |
| Linux/FUSE extensions | Not POSIX base requirements; use Linux and FUSE references | `test_xattr.py`, `test_acl_mount_option.py`, `test_ioctl.py`, path-search xattr cases in `test_path_search_permissions.py`, compatibility contracts | Separate extension audit |

## Pending requirement ledger

The matrix remains deliberately conservative. The following ledger gives every
Pending row a bounded next step; completing one item closes only the stated gap,
not the whole row unless all of its normative conditions and errors are mapped.

| ID | Area | Verified baseline | Remaining closure evidence |
| --- | --- | --- | --- |
| P01 | Path resolution and name syntax | Trailing slash, symlink following, dangling links and loops have mounted regressions; `test_path_trailing_slash.py` checks Linux `ENOENT` for empty pathnames in `stat`, `lstat`, `open`, `mkdir`, `unlink`, `rmdir`, `readlink`, `access`, `chmod`, `chown`, `utime`, `truncate`, `statvfs`, `link`, `symlink` and `rename`; libc `access()` is checked for `-1`/`ENOENT` because Python `os.access()` reports failure as `False`. A path to a regular file ending in `/` returns `ENOTDIR` for `stat`, `open`, `lstat`, `unlink`, `rmdir`, `truncate`, `chmod`, `chown`, `utime`, `readlink`, `statvfs`, hard-link source, and rename source/destination; `mkdir`, hard-link destination and symlink destination return `EEXIST`; `access` returns `-1`/`ENOTDIR`. The file and rename source remain unchanged. A trailing-slash symlink to a regular file returns `ENOTDIR` for `stat`, `open`, `lstat`, `unlink`, `rmdir`, `truncate`, `chmod`, `chown`, `utime`, `readlink`, `statvfs`, hard-link source and rename source/destination; `mkdir`, hard-link destination and symlink destination return `EEXIST`, and `access` returns `-1`/`ENOTDIR`. The link and target remain unchanged. A trailing-slash symlink to a dangling target returns `ENOENT` for `stat`, `lstat`, `open`, `truncate`, `chmod`, `chown`, `utime`, `readlink`, `statvfs`, hard-link source and `access`; `mkdir`, hard-link destination and symlink destination return `EEXIST`; `unlink`, `rmdir` and rename source/destination return `ENOTDIR`. The dangling symlink and rename source are preserved. A trailing-slash symlink to a directory resolves for `stat`, `lstat`, `open`, `chmod`, `utime`, `chown`, `statvfs` and `access` (metadata operations apply to the target); `mkdir` and link/symlink destinations return `EEXIST`, hard-linking the directory returns `EPERM`, `unlink`/`rmdir` and rename source/destination return `ENOTDIR`, `truncate` returns `EISDIR`, and `readlink` returns `EINVAL`, preserving the symlink, directory and rename source. The test also verifies that a path longer than the mount's `PC_PATH_MAX`, composed of single-byte components below `NAME_MAX`, returns `ENAMETOOLONG` for 17 path operations and `-1`/`ENAMETOOLONG` through libc `access()`; Linux rejects it before FUSE dispatch. It covers `.`/`..`, repeated separators, Linux equivalence of paths beginning `//` and `///` to a single leading slash, and `NAME_MAX` boundary behavior | Map remaining XBD pathname-resolution rules for empty components and final-component behavior of other operations; classify residual `PATH_MAX` behavior against conditional POSIX requirements |
| P02 | Directory operations and iteration | Create/remove errors, PostgreSQL SQLSTATE `53100` mapped to exact `ENOSPC` for `mkdir()`, SQLSTATE `58030` mapped to exact `EIO` for `mkdir()`/`rmdir()` with namespace rollback, exact `EACCES` from libc `opendir()` on a mode-000 directory and denied `mkdir()`/`rmdir()` (the mounted regression passed on FOD 3.4.374 in `release-lto` against disposable PostgreSQL 16.15), and ACL-enabled `EACCES` for regular-file, directory, `mknod`, symlink and hardlink creation plus `unlink()`, `rmdir()` and both rename parents when a named-user ACL denies parent write access despite mode `0777`; exact `EROFS` for those operations on a read-only replica, `EBUSY` when `rmdir()` targets the mounted FUSE root, `ELOOP` for `mkdir()`/`rmdir()` through a symlink-loop parent, `ENAMETOOLONG` for components beyond `NAME_MAX`, `EINVAL` for `rmdir()` of final `.`, `ENOTEMPTY` for final `..`, `ENOTDIR` for `rmdir()` on a symlink to a directory, link counts, small-buffer cookies, a 24-round-per-type threaded parent-directory versus child file/directory create race (ENOENT before parent creation or a matching child entry afterward; passed in release-lto with the complete concurrent mount_smoke subset on 2026-10-30), the 16-round threaded file-versus-directory create race (exactly one winner, `EEXIST` for the loser, and winner type/content validation), and two-mount remove/recreate with old-handle rewind are covered; the replacement receives a distinct inode and is visible after fresh lookup. `test_directories.sh` checks `ENOTDIR`/`EBADF` descriptor errors. `test_readdir_snapshot_multimount.py` traverses 512 initial children, changes membership through a second mount, verifies stable unaffected entries, duplicate-free continuation, and refreshed membership after rewind; direct libc calls cover successful `opendir()`/`readdir()`/`closedir()`, `ENOENT` for a missing path and `EMFILE` at the process descriptor limit. POSIX leaves pre-rewind visibility of changed names unspecified. FOD pins ordered entries to each `opendir()` handle and refreshes after rewind. See [`POSIX errno matrix`](POSIX_ERRNO_MATRIX.md) | 32-bit narrow-`ino_t` `EOVERFLOW` conversion is outside FOD's supported architecture matrix; artifacts target 64-bit and wider userspace only. Exact sticky-directory `EPERM` for `unlink()` and `rmdir()` is covered by `test_permissions.py`; system-wide `ENFILE`, failure-path `closedir()` behavior and remaining `mkdir()`/`rmdir()` errors remain to be checked on supported systems. Current mounted tests exercise `getdents64()` and libc on the host ABI |
| P03 | Open and descriptor semantics | Access modes, append-after-seek, shared duplicated offset, create conflicts, descriptor `O_APPEND`/`FD_CLOEXEC` flags and symlink create/truncate are covered; `test_create_write_ownership.py` verifies a second mount gets fail-fast `EBUSY` for `O_APPEND` while the first holds the write lease; `mount_smoke::cross_mount_append_uses_fresh_kernel_file_size` primes a stale size on mount B, appends through A, then proves B receives offset 3 and preserves `A-A-B` even when a positive attribute TTL is requested. Writable mounts now send zero attribute TTL so Linux revalidates inode size before `O_APPEND` offset selection | Map all supported `open()` flags and errors and test the interrupted close boundary; lowest-descriptor selection remains a kernel descriptor-table property rather than a FOD callback |
| P04 | Read and write | EOF short reads, zero-length read/write, access-mode errors, positioned-I/O offset preservation, write/pwrite beyond-EOF zero-fill, PostgreSQL SQLSTATE `53100` reaching `fsync()` as `ENOSPC`, cross-mount append at the current EOF, signed `off_t` addition overflow, and `RLIMIT_FSIZE` are covered. `test_lseek.py` confirms Linux returns `EINVAL` for `pwrite(OFF_T_MAX, 2 bytes)` and preserves the file; Linux v6.17 rejects it in `rw_verify_area()` before FUSE dispatch. A subprocess limits file size to 4096 bytes, requests an 8192-byte write, verifies a 4096-byte short write and `SIGXFSZ`, then checks `EFBIG` from `write()` and `pwrite()` beyond the limit with unchanged size and payload | Verify signal interruption before any data is transferred, backend `EIO`, and the platform-specific maximum-offset behavior; the signed-overflow case is intercepted by Linux with `EINVAL` before FOD can map it to `EFBIG` |
| P05 | Truncation and file length | Shrink/extend zero-fill, block boundaries, descriptor offset preservation, `mtime`/`ctime` updates after path `truncate()` and descriptor `ftruncate()`, symlink following, `EISDIR`/`ENOENT`/`EINVAL` path errors and POSIX-permitted `EBADF`/`EINVAL` for a read-only descriptor are covered by `test_truncate_shrink_block_boundary.py`, and `test_lseek.py` verifies exact `EBADF` for `ftruncate()` on a closed descriptor without changing size, timestamps or payload; exact `EACCES` for a non-writable file and an unsearchable path component is covered by `test_path_search_permissions.py` and `test_acl_mount_option.py`. `test_lseek.py` also sets `RLIMIT_FSIZE=4096` in a subprocess and verifies `EFBIG` plus unchanged content for over-limit path `truncate()` and descriptor `ftruncate()`. `mount_smoke::concurrent_write_and_truncate_leave_a_serializable_file` races positioned writes with `ftruncate()` on cloned descriptors for 24 rounds, requires a serializable final value, and caught/fixed false `EBUSY` from same-lease flush validation plus stale per-handle buffered state overwriting truncate. `test_stale_writer_fencing.py` races mount A `pwrite()` against mount B pathname `truncate()` after a barrier; the write completes, truncate returns `EBUSY`, and PostgreSQL confirms the writer's 4096-byte payload plus unchanged lease owner/token. It then revokes mount A's lease, lets B take over, and verifies stale `ftruncate()` returns `EBUSY` without changing B's payload | Verify `EINTR` and I/O-error cases |
| P06 | Metadata and timestamps | Root metadata, common stat fields, chmod/chown, timestamp authorization, symlink-follow behavior, descriptor-based `fstat()`/`fchmod()`/timestamp updates, and exact `EBADF` from `fstat()`, `fchmod()`, `fchown()` and `futimens()` on a closed descriptor are covered | Verify remaining file-status fields and type-specific values, timestamp precision contract, and required errors |
| P07 | Permissions and identity | Mode checks, supplementary groups, umask, ownership inheritance, ACL authorization, inaccessible-parent search, real-UID `access()` with effective UID 0, root `X_OK` with/without execute bits, and named-user ACL denial for file/directory/special-file/symlink/hardlink creation, `unlink()`, `rmdir()` and both rename parents despite mode `0777` are covered; `test_ownership_inheritance.py` checks setgid parent GID inheritance for files, directories, symlinks and FIFOs plus setgid propagation to nested directories; `test_acl_mount_option.py` checks exact `EACCES`, preservation, and that `chmod()` updates both access-ACL owner/mask entries and mode; `pg_query::chmod_updates_access_acl_and_mode_in_one_transaction` covers files/directories and ACLs with and without named entries. Linux v6.17 `inode_init_owner()` assigns a new inode the setgid parent's GID and always propagates `S_ISGID` to child directories ([`fs/inode.c`](https://github.com/torvalds/linux/blob/v6.17/fs/inode.c#L2540-L2567)). Linux v6.17 FUSE documents that the userspace daemon must update inode access permissions when ACLs change ([`fs/fuse/acl.c`](https://github.com/torvalds/linux/blob/v6.17/fs/fuse/acl.c#L116-L124)); Linux chmod ACL semantics update `ACL_USER_OBJ`, `ACL_MASK` or (without a mask) `ACL_GROUP_OBJ`, and `ACL_OTHER` ([`fs/posix_acl.c`](https://github.com/torvalds/linux/blob/v6.17/fs/posix_acl.c#L506-L545)) | Map privilege rules for ownership/mode changes and set-ID clearing separately from default ACL inheritance; check remaining operation-specific permission errors |
| P08 | Links and namespace mutation | Hardlink identity/counts, common link errors, transactional rename cases, same-inode rename no-op, symlink behavior and unlink lifecycle are covered. `test_rename_directory_transactional.py` disables FUSE attribute caching for its assertions and verifies cross-parent directory moves and empty-directory replacement: source/destination parent `st_nlink` changes 3/2 to 2/3 on a move, then 3/4 to 2/4 on replacement, observed through both mounts. Rust `mount_smoke::concurrent_threads_rename_one_source_to_distinct_destinations` races two threads renaming the same source to separate targets for 16 rounds and checks one success, one surviving target, and intact contents. The eight-thread `concurrent_threads_create_write_and_read_independent_files` test also covers parallel directory creation, rename and removal; it exposed and now verifies the whitespace-correct recursive cycle-check query for directory rename. `concurrent_threads_unlink_and_replace_many_open_file_tasks` runs 32 unlink/recreate tasks across eight threads and verifies open descriptors retain old contents and replacements have distinct inode identities | Map atomic replacement and error precedence for every file-type pairing, and cross-mount observation after committed namespace mutations |
| P09 | Special files and FIFOs | `test_mknod.py` requires regular `mknod()` and `mkfifo()` creation, FIFO type/mode, nonblocking writer `ENXIO`, `EPIPE` after the last reader closes, `ESPIPE` for `pread()`/`pwrite()`, nonblocking empty-read `EAGAIN`, blocking writer handoff, data transfer, EOF after writer close and non-interleaving concurrent records up to `_PC_PIPE_BUF`; Linux device-node probes remain separate | Map remaining FIFO open/read/write interruption and error requirements; keep Linux character/block devices outside the POSIX FIFO claim |
| P10 | File locks | PostgreSQL-backed conflicts, ranges, blocking handoff, signal interruption returning `EINTR` without a late lock acquisition, cross-mount `EDEADLK` cycle detection between two processes, expiry, close-triggered release, cross-mount `F_GETLK` conflicting PID, relative/negative-length range normalization and descriptor access-mode `EBADF` are covered. A mounted regression also seeds an active range lease with an unsupported lock type and verifies `F_GETLK` fails with `EIO` instead of treating corrupt authoritative state as unlocked. `file_locks_remain_bound_to_open_inode_after_path_replacement` verifies record locks use the open handle's authoritative `file_id` after remote unlink/recreate; it passed on FOD 3.4.368. `posix_record_locks_are_not_inherited_across_fork` forks while the parent holds `F_WRLCK`; the child inherits the descriptor but `F_GETLK` still reports the parent's PID and lock, passing on FOD 3.4.370. `posix_record_locks_survive_exec_with_open_fd` verifies an `F_WRLCK` and its non-close-on-exec descriptor survive `exec()` and remain visible to a separate process, also passing on FOD 3.4.370 | Retain the documented FIFO exclusion from FOD cross-mount lock guarantees |
| P11 | Filesystem capacity and status | Mapped POSIX block units, availability fields, required flags and name limit; tested block sizes, capacity cap, virtual inode headroom of 1,000,000, PostgreSQL accounting, reservations, shared quota admission, path/descriptor parity, field ordering, path errors `EACCES`, `ELOOP`, `ENAMETOOLONG`, `ENOENT`, `ENOTDIR`; `fstatvfs()` invalid-fd `EBADF`. `test_df.sh` asserts FOD's `f_bavail == f_bfree` and `f_favail == f_ffree` and covers the synthetic inode-headroom contract. Linux FUSE does not expose `f_fsid` to userspace and v6.17 sets it to zero. One earlier `test-df` run copied the expected bytes but found distinct source/copy PostgreSQL `data_object_id`s. Four traceable runs on 3.4.207 and ten new `release-lto` debug runs on 3.4.322 all passed through `whole_object_adoption=true` and recorded successful source-object adoption; all ten recent traces show no pending source/destination write state and matching PostgreSQL object IDs. The intermittent mismatch remains unresolved because it has not been reproduced. See [`TODO.md`](../TODO.md) for archived first-mount traces. | Determine whether a signal-interrupted mounted `statvfs()` test is needed; test applicable `EOVERFLOW` cases on supported 64-bit-and-wider ABIs (Linux 32-bit compatibility syscall coverage is outside support); diagnose the intermittent whole-object sharing failure before treating it as closed |
| P12 | Error behavior | Targeted regressions assert exact errno values including `EACCES` for denied `mkdir()`/`rmdir()`, `EROFS` for read-only replica `mkdir()`/`rmdir()` and metadata operations, `EPERM` for non-owner `chmod`/`chown`, `EINVAL` for invalid `utimensat` nanoseconds, `EILSEQ`, `ENAMETOOLONG`, directory-fd `ENOTDIR`/`EBADF`, FIFO `ENXIO`, xattr-buffer `ERANGE`, degraded-read `ESTALE`, `truncate()` `EACCES`/`EISDIR`/`ENOENT`/`EINVAL`, `statvfs()` `ENOENT`/`ENOTDIR`, `fstatvfs()` `EBADF`, common namespace errors and access-mode failures. [`POSIX_ERRNO_MATRIX.md`](POSIX_ERRNO_MATRIX.md) maps required, conditional, and XSI-only errors for file I/O, namespace, metadata, locks and capacity against Issue 8, and records confirmed Linux 6.17 VFS short-circuits | Add `stat`/`statvfs` overflow assertions and signal-interrupted `statvfs`. Linux VFS masks `chmod` mode bits and POSIX marks `EINVAL` as `may fail`; see the confirmed probe in [`POSIX_ERRNO_MATRIX.md`](POSIX_ERRNO_MATRIX.md). Keep the optional POSIX FIFO lock behavior and Linux FUSE mount-local limitation documented in [`TODO.md`](../TODO.md); resolve remaining Linux VFS/libfuse/FOD errno attribution; `unlink(directory)` retains documented Linux `EISDIR` despite POSIX Issue 8 prescribing `EPERM`; run applicable `pjdfstest` subsets with explicit skips |

### Cross-mount append finding

Resolved in FOD 3.4.310.

The PostgreSQL exclusive file write-ownership lease remains in place:
`test_create_write_ownership.py` verifies that a second mount gets `EBUSY`
while the first holds the lease and can acquire a descriptor after release.
The separate sequential offset defect reproduced on FOD 3.4.309 when a writable
mount had a positive attribute TTL: mount B first observed size 1, mount A
appended `-A`, then B's FUSE write callback still received offset 1 and
overwrote the file as `A-B` instead of `A-A-B`. This reproduced at attribute
TTLs 1 and 120 seconds; separate probes reproduced it in buffered and direct-I/O
modes.
With attribute TTL zero, B received offset 3 and the contents remained
`A-A-B`.

Linux v6.17 runs `generic_write_checks()` before the FUSE write callback and
chooses the append position from that mount's local inode size; the FUSE write
reply contains no server-assigned offset. FOD now sends an effective attribute
TTL of zero on writable mounts, even when a positive TTL is requested. This
forces Linux to refresh the inode size before resolving `O_APPEND`; read-only
mounts retain their configured TTL and the entry-name TTL remains independent.
The mounted `mount_smoke::cross_mount_append_uses_fresh_kernel_file_size` test
requests a 120-second TTL in buffered and direct-I/O modes, checks the effective
logged value is zero, primes mount B's cached size at 1, appends through A and
then B, and verifies both callback offset 3 and final bytes `A-A-B` in each
mode. The test passed on FOD 3.4.310;
the full Rust `mount_smoke` suite and the exclusive ownership integration test
also passed. The tradeoff is an additional attribute revalidation on writable
mount metadata accesses; effective attribute TTL is logged at mount startup.
The earlier anomalous output came from an older 3.4.304 runtime artifact and is
not evidence against the current-version regression.

Sources:
[Linux FUSE write paths](https://github.com/torvalds/linux/blob/v6.17/fs/fuse/file.c),
[Linux write checks](https://github.com/torvalds/linux/blob/v6.17/fs/read_write.c),
[FUSE write reply](https://github.com/torvalds/linux/blob/v6.17/include/uapi/linux/fuse.h).

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
`getdents64()` buffers and checks for missing or duplicate names.

POSIX.1-2024 `readdir()` explicitly leaves unspecified whether an entry added
or removed after the latest `opendir()` or `rewinddir()` is returned by a later
call. FOD now pins an ordered entry snapshot to each `opendir()` handle and the
PostgreSQL directory ID captured for that handle. Continuation and rewind use
that ID; names-mode child metadata and directory atime updates do too. Rewind
refreshes the snapshot when cookie zero restarts an iterated stream, and
`releasedir()` frees the state. FUSE-created directories persist a generation
seed made from the PostgreSQL-assigned mount session ID and a mount-local
monotonic counter, preventing a replacement from reusing the old directory's
`st_ino`; a two-mount regression confirms the old handle stays bound to its
original directory ID through rewind. No regression should
assert whether a newly added or removed name appears before rewind. Mounted
continuation across 512 initial entries, cross-mount membership changes,
rewind, libc EOF and `ENOENT` handling, and process-limit `EMFILE` are covered
by `test_readdir_snapshot_multimount.py`. A 32-bit ABI check for `EOVERFLOW`
from converting the 64-bit FUSE inode to a narrower `ino_t` is outside FOD's
supported artifact matrix. System-wide `ENFILE`, failure-path `closedir()`
behavior and remaining libc/kernel/FUSE error attribution remain open, so P02
stays Pending until the remaining in-scope regressions and mappings are
complete. See
[POSIX.1-2024 `readdir()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/readdir.html).

The POSIX `readdir()` errors cross several layers. POSIX requires `EOVERFLOW`
when a value cannot fit the returned `struct dirent`; for FOD, the inode number
is a 64-bit FUSE value, so the relevant conversion overflow occurs on a narrower
ABI. Linux v6.17 `getdents64` writes the 64-bit inode directly, while legacy
compat `getdents` checks whether the inode fits its narrower field and returns
`EOVERFLOW`. glibc's compatibility `getdents64` converter also checks a
narrower `ino_t`. FOD does not build or publish 32-bit artifacts, so this
32-bit-only conversion case is outside supported-platform verification; it is
not an errno FOD can select reliably in its FUSE `readdir()` callback.

The Linux kernel rejects a closed directory fd with `EBADF` before calling the
FUSE filesystem; its directory iteration also returns `ENOENT` for a dead
directory inode. FOD's callback-level `EBADF` is for an unknown per-open FUSE
handle, which should not be sent by the kernel for a valid open stream. POSIX
classifies invalid-stream `EBADF` and invalid-position `ENOENT` as `may fail`;
FOD currently treats an out-of-range cookie as end-of-stream. The existing
`test_directories.sh` check is a direct `getdents64()` call after closing the
descriptor, not a libc `closedir()` error check. Remaining verification is a
direct libc stream error checks remain in scope. Sources:
[POSIX `readdir()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/readdir.html), [Linux v6.17 directory syscalls](https://github.com/torvalds/linux/blob/v6.17/fs/readdir.c), [Linux v6.17 FUSE readdir](https://github.com/torvalds/linux/blob/v6.17/fs/fuse/readdir.c), and [glibc `getdents64` compatibility conversion](https://codebrowser.dev/glibc/glibc/sysdeps/unix/sysv/linux/getdents64.c.html).

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
