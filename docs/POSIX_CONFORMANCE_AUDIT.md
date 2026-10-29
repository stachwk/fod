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

## Initial requirement matrix

| Area | Normative interfaces / clauses to review | Existing FOD evidence to inspect | Status |
| --- | --- | --- | --- |
| Path resolution and name syntax | XSH `open`, `stat`, `mkdir`, `link`, `symlink`, `rename`, `unlink`; XBD pathname resolution | `rust_fuse/src/fs.rs`; `test_path_trailing_slash.py` checks `ENOTDIR` for regular files and symlinks to files, plus directory and symlink-to-directory resolution; `test_directories.sh` checks `O_CREAT|O_EXCL` file creation below a regular file returns `ENOTDIR` without changing that file; `test_symlink.sh` checks `stat` follows a valid link while `lstat` reports the link inode, `EINVAL` for `readlink` on a regular file, short-buffer behavior without NUL termination, `lstat` target-byte `st_size`, unlink preserving the target, dangling-link errors, `O_CREAT|O_EXCL` returning `EEXIST`, and `ELOOP`; `test_mount_suite.py`, `test_rename_directory_transactional.py` | Pending |
| Directory operations and iteration | XSH `opendir`, `readdir`, `closedir`, `mkdir`, `rmdir`; XBD directory stream rules | FUSE callbacks in `rust_fuse/src/fs.rs`; `test_directories.sh` checks `EEXIST` for `mkdir` on an existing file or directory, `EISDIR`/`ENOENT` for invalid `unlink` and `ENOENT`/`ENOTDIR`/`ENOTEMPTY` for invalid `rmdir`, and `ENOENT`/`ENOTDIR` when file creation, `mkdir` or `rmdir` traverses a missing or non-directory parent; failed child creation preserves the parent file contents; `test_mount_suite.py`, `test_tree_scale.py`, `test_readdir_small_buffers.py`, `test_path_search_permissions.py` | Pending |
| Open and descriptor semantics | XSH `open`, `close`, `fcntl`, `dup`; access modes, append and create flags | `open`, `create`, `release`, `flush` callbacks; `test_mount_suite.py`, `test_create_write_ownership.py` (`O_EXCL` conflict/recovery), `test_symlink.sh` verifies `O_TRUNC` follows a valid symlink and `O_CREAT` creates the target through a dangling symlink while retaining the link; append-after-seek, shared-`dup`-offset and descriptor access-mode cases in `test_lseek.py`; see the cross-mount append finding below | Pending |
| Read and write | XSH `read`, `write`, `pread`, `pwrite`; short I/O, EOF, access-mode errors and offset rules | `test_lseek.py` (including EOF short reads, zero-byte writes, `EBADF` for incompatible descriptor modes, and `pread`/`pwrite` offset preservation), `test_mount_suite.py`; cross-mount append remains blocked at writable open by the exclusive write-ownership lease | Pending |
| Standard file-offset positioning | XSH `lseek`, `SEEK_SET`, `SEEK_CUR`, `SEEK_END`; negative result, offset beyond EOF and no size change from seeking | Mounted `test_lseek.py` | Pass (listed cases) |
| Sparse file-offset positioning | Optional POSIX.1-2024 XSH `lseek` `SEEK_DATA` and `SEEK_HOLE` behavior; distinguish Issue 8 requirements from their pre-Issue-8 Linux/BSD history and Linux FUSE callback details | `test_lseek_sparse_runtime.py`, `test_lseek_sparse_contract.py`; callback `FodFuse::lseek` | Pass |
| Truncation and file length | XSH `truncate`, `ftruncate`, `open(O_TRUNC)` | `test_truncate_shrink_block_boundary.py` covers block-boundary and zero-fill behavior, descriptor-offset preservation, and POSIX-permitted `EINVAL`/`EBADF` on a read-only descriptor; `test_mount_suite.py`, Rust `mount_smoke` | Pending |
| Metadata and timestamps | XSH `stat`, `fstat`, `chmod`, `fchmod`, `chown`, `utimensat`; XBD file status fields | `test_mount_suite.py`, `test_permissions.py`, `test_timestamp_touch_once.py`, `test_utimens_noop`; `test_symlink.sh` checks `utimens` through a symlink at PostgreSQL's microsecond resolution and verifies the symlink mtime is preserved | Pending |
| Permissions and identity | XSH `access`, `umask`, `chmod`, `chown`; XBD file access permission rules | `test_permissions.py`, `test_access_groups.py`, `test_ownership_inheritance.py` checks umask masking on created files and directories; `test_symlink.sh` verifies `chmod` through a symlink changes target mode; `test_root_owned_permissions.py` | Pending |
| Links and namespace mutation | XSH `link`, `unlink`, `rename`, `symlink`, `readlink`, `mkdir`, `rmdir`; atomicity and error precedence | `test_hardlink.sh` checks `EEXIST` for an occupied target, `ENOENT` for a missing source and `EPERM` for a directory source; `test_directories.sh` checks `ENOTDIR` for hard-link and symbolic-link destinations and rename targets beneath a regular file, with source and parent-file preservation; `test_rename_directory_transactional.py` checks `ENOENT` for a missing rename source while preserving the destination on both mounts; `test_symlink.sh`, `test_chmod_rmdir.py`, primary-authority regressions | Pending |
| Special files and FIFOs | XSH FIFO creation/open/read/write behavior; classify Linux `mknod` and device-node provisioning separately | `test_mknod.py`, `test_mount_suite.py` | Pending |
| File locks | XSH `fcntl()` `F_GETLK`, `F_SETLK`, and `F_SETLKW`; process-associated advisory record-lock behavior | `rust_fuse/tests/lock_backend_smoke.rs` checks read/write conflict queries and nonblocking `EACCES`/`EWOULDBLOCK`, blocking `F_SETLKW` handoff after a cross-mount unlock, conflicts across primary mounts and visibility from a replica, byte-range boundaries, partial unlock and overlapping-range replacement, lease expiry, and POSIX close-triggered process unlock; PostgreSQL serializes cross-host conflict checks with range updates. Signal interruption, deadlock detection, and returned conflicting-lock PID semantics are not covered by this evidence | Pending |
| Filesystem capacity and status | POSIX.1-2024 XSH `statvfs()`/`fstatvfs()` and XBD `<sys/statvfs.h>`: `f_blocks`/free counts use `f_frsize` units; `f_bavail`/`f_favail` are available to non-privileged processes; `f_flag` defines `ST_RDONLY` and `ST_NOSUID`; `f_namemax` is the component-name limit | Linux FUSE `statfs` callback in `rust_fuse/src/fs.rs` supplies block counts, inode counts, block size and name limit; Linux derives mount flags. `test_df.sh` checks `f_bsize`/`f_frsize` positive and equal, block and inode count ordering, path/descriptor parity, `ST_RDONLY`/`ST_NOSUID`, name limit 255, backing-filesystem capacity cap, inode headroom and symlink inode-count updates. It also checks `statvfs()` `EACCES` (when unprivileged), `ELOOP`, `ENAMETOOLONG`, `ENOENT`, `ENOTDIR`, and Linux empty-path `ENOENT`, plus `fstatvfs()` `EBADF`. `make test-df` passed twice on FOD 3.4.171, including the unprivileged `EACCES` case. Accounting checks compare used blocks with PostgreSQL payload plus active reservations before and after remount. `test_two_mount_quota.py` covers shared quota admission. Linux reports `f_favail == f_ffree`; FOD reports virtual inode headroom, not a finite PostgreSQL inode quota. Linux FUSE protocol has no `f_fsid` reply field, and Linux v6.17 explicitly leaves `kstatfs.f_fsid` zero, so FOD cannot supply a filesystem ID through this callback. Source review confirms FOD maps statfs snapshot or backing-capacity failures to `EIO`; this path has no targeted fault-injection regression yet. POSIX also requires `EINTR` for a caught signal and `EOVERFLOW` when a result cannot fit the caller's structure. On Linux, a 32-bit compatibility `statfs` syscall can produce `EOVERFLOW` while copying the kernel result, outside FOD's callback. Remaining gaps: exercise FOD's `EIO` path; determine whether signal-interrupted `statvfs()` needs a mounted regression; validate `EOVERFLOW` on every supported ABI; and document/test why FOD's free and non-privileged-available counts are equal. See [POSIX header definition](https://pubs.opengroup.org/onlinepubs/9799919799/basedefs/sys_statvfs.h.html), [POSIX `statvfs()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/statvfs.html), [POSIX Programmer's Manual `statvfs()` errors](https://man7.org/linux/man-pages/man3/fstatvfs.3p.html#errors), [Linux FUSE v6.17 statfs conversion](https://github.com/torvalds/linux/blob/v6.17/fs/fuse/inode.c#L3196-L3219), [Linux v6.17 `statfs` overflow checks](https://github.com/torvalds/linux/blob/v6.17/fs/statfs.c#L1423-L1450), and [Linux `statvfs(3)` errors](https://man7.org/linux/man-pages/man3/statvfs.3.html#errors) | Pending |
| Error behavior | XSH per-interface `ERRORS` and `RATIONALE`; errno precedence for supported operations | [`ERRNO_REFERENCE.md`](ERRNO_REFERENCE.md) summarizes common codes and test evidence; targeted integration tests and `pjdfstest` subsets | Pending |
| Linux/FUSE extensions | Not POSIX base requirements; use Linux and FUSE references | `test_xattr.py`, `test_acl_mount_option.py`, `test_ioctl.py`, path-search xattr cases in `test_path_search_permissions.py`, compatibility contracts | Separate extension audit |

## Pending requirement ledger

The matrix remains deliberately conservative. The following ledger gives every
Pending row a bounded next step; completing one item closes only the stated gap,
not the whole row unless all of its normative conditions and errors are mapped.

| ID | Area | Verified baseline | Remaining closure evidence |
| --- | --- | --- | --- |
| P01 | Path resolution and name syntax | Trailing slash, symlink following, dangling links and loops have mounted regressions; `test_path_trailing_slash.py` also covers `.`/`..`, repeated separators, the reported `NAME_MAX` boundary and `ENAMETOOLONG` | Map remaining XBD pathname-resolution rules for empty components, `PATH_MAX`, and per-operation final-component following; add mounted boundary and errno cases |
| P02 | Directory operations and iteration | Create/remove errors, permissions, link counts and small-buffer cookies are covered; `test_directories.sh` repeats directory-fd listing and checks `ENOTDIR`/`EBADF` descriptor errors | Verify `readdir()` mutation rules and all remaining required `mkdir()`/`rmdir()` errors against Issue 8 |
| P03 | Open and descriptor semantics | Access modes, append-after-seek, shared duplicated offset, create conflicts, descriptor `O_APPEND`/`FD_CLOEXEC` flags and symlink create/truncate are covered; a two-mount concurrency probe found a second writable open returns `EBUSY` while the first mount owns the file write lease | Design PostgreSQL-authoritative shared or serialized append admission that permits concurrent writable opens without losing cross-host write fencing; then assert complete, non-interleaved `O_APPEND` records across mounts. Also map all supported `open()` flags and errors and test interrupted close boundary; lowest-descriptor selection remains a kernel descriptor-table property rather than a FOD callback |
| P04 | Read and write | EOF short reads, zero-length read/write, access-mode errors, positioned-I/O offset preservation, and write/pwrite beyond-EOF zero-fill are covered | Add partial-I/O/error boundaries; cross-mount append serialization cannot be validated until concurrent writable opens are admitted under a PostgreSQL-authoritative ordering contract |
| P05 | Truncation and file length | Shrink/extend zero-fill, block boundaries, descriptor offset preservation, mtime update, symlink following, `EISDIR`/`ENOENT`/`EINVAL` path errors and read-only descriptor rejection are covered | Map `truncate()`/`ftruncate()` authorization and remaining required errors; verify remaining timestamp effects and concurrent truncate/write serialization across mounts |
| P06 | Metadata and timestamps | Root metadata, common stat fields, chmod/chown, timestamp authorization, symlink-follow behavior, and descriptor-based `fstat()`/`fchmod()`/timestamp updates are covered | Verify remaining file-status fields and type-specific values, timestamp precision contract, and required errors |
| P07 | Permissions and identity | Mode checks, supplementary groups, umask, ownership inheritance, ACL authorization and inaccessible-parent search are covered | Map effective/real-ID semantics for `access()`, privilege rules for ownership/mode changes, set-ID bit clearing, and default ACL inheritance separately from base POSIX modes |
| P08 | Links and namespace mutation | Hardlink identity/counts, common link errors, transactional rename cases, same-inode rename no-op, symlink behavior and unlink lifecycle are covered | Map atomic replacement and error precedence for every file-type pairing, cross-directory parent link counts, and cross-mount observation after committed namespace mutations |
| P09 | Special files and FIFOs | `test_mknod.py` requires regular `mknod()` and `mkfifo()` creation, FIFO type/mode, nonblocking writer `ENXIO`, blocking writer handoff, data transfer, EOF after writer close and non-interleaving concurrent records up to `_PC_PIPE_BUF`; Linux device-node probes remain separate | Map remaining FIFO open/read/write interruption and error requirements; keep Linux character/block devices outside the POSIX FIFO claim |
| P10 | File locks | PostgreSQL-backed conflicts, ranges, blocking handoff, expiry, close-triggered release, cross-mount `F_GETLK` conflicting PID, relative/negative-length range normalization and descriptor access-mode `EBADF` are covered | Verify `F_SETLKW` signal interruption and deadlock behavior plus fork/exec lifecycle |
| P11 | Filesystem capacity and status | Mapped POSIX block units, availability fields, required flags and name limit; tested block sizes, capacity cap, inode headroom, PostgreSQL accounting, reservations, shared quota admission, path/descriptor parity, field ordering, and path errors `EACCES`, `ELOOP`, `ENAMETOOLONG`, `ENOENT`, `ENOTDIR`; `fstatvfs()` invalid-fd `EBADF`. Linux FUSE does not expose `f_fsid` to userspace and v6.17 sets it to zero. | Add a deterministic FOD statfs `EIO` regression; determine whether a signal-interrupted mounted `statvfs()` test is needed; test `EOVERFLOW` on supported 32-bit ABIs; document and assert FOD's equal `f_bfree`/`f_bavail` behavior and virtual inode headroom |
| P12 | Error behavior | Targeted regressions assert exact errno values including `ENAMETOOLONG`, directory-fd `ENOTDIR`/`EBADF`, FIFO `ENXIO`, `truncate()` `EISDIR`/`ENOENT`/`EINVAL`, `statvfs()` `ENOENT`/`ENOTDIR`, `fstatvfs()` `EBADF`, common namespace errors and access-mode failures | Build an interface-to-`ERRORS` table from Issue 8, distinguish mandatory and optional errors, record Linux VFS short-circuits, and run the applicable `pjdfstest` subset with explicit skips |

### Cross-mount append finding

A mounted probe opened the same file for `O_APPEND` from two FOD mounts backed
by the same PostgreSQL database and synchronized concurrent writers. The second
writable `open()` returned `EBUSY` before either thread reached its write
barrier. Source inspection traces this to
`FodFuse::create_writable_handle_with_ownership()`: PostgreSQL grants one
exclusive file write-ownership lease, and a competing mount maps an unavailable
lease to `EBUSY`. This is a concrete POSIX open/append limitation, not evidence
of record interleaving. Keep the P03/P04 items open until writable opens can
participate in a PostgreSQL-authoritative serialization design and a mounted
test proves complete append records from both mounts.

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
