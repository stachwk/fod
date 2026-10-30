# FOD current implementation plan

Updated: 2026-10-30. This is the single implementation queue. Work is performed
on `main`. Implemented behavior belongs in [CURRENT_STATE.md](../CURRENT_STATE.md);
long-term proposals belong in [ROADMAP.md](../../ROADMAP.md).

## Implementation order

| Priority | Work | Current boundary and completion evidence |
| --- | --- | --- |
| 1 | Primary-authoritative mutation decisions | The [authority audit](../FOD_PRIMARY_AUTHORITY_AUDIT.md) records verified unlink lifetime, atomic rmdir, cross-type creation, transactional chmod/chown and sticky-bit checks. Default ACL inheritance, create authorization and parent ACL checks for unlink/rmdir/rename now share the relevant PostgreSQL mutation transaction. `lookup` and namespace mutations verify that cached parent paths still resolve to the requested directory inode on primary; `opendir` checks directory inode identity on primary before snapshot creation. The open handle retains its PostgreSQL directory ID for initial/rewind snapshots, names-mode child metadata and atime updates, so readdir cannot switch an old handle to a replacement directory at the same path. Two-mount regressions cover stale-inode `opendir`, `readlink` and remove/recreate during an open directory stream; `readlink` rejects a stale symlink inode with `ENOENT` and reads replacement targets only after a fresh lookup. `unlink` and `rmdir` compare the entry's resolved parent ID. Create callbacks are verified with replica replay paused: `mkdir`, `create`, `mknod`, `symlink` and `link` return primary-committed metadata before replica visibility, while conflicting creates return `EEXIST`. Generic xattr set/remove authorization and `CREATE`/`REPLACE` semantics now share the locked inode transaction; all xattr callbacks validate the request inode against primary-resolved metadata (or a live handle file ID after unlink) and use its database owner ID, with a passing two-mount regression for stale cached paths and open-unlinked handles. `user.*` reads check ACL/mode from the verified inode owner, including open-unlinked files; unprivileged `listxattr` hides `trusted.*`. FUSE access/open permissions, inode identity for open, lock resource identity/type checks, and write-ownership parent IDs use the current primary namespace. Atime updates resolve file, directory and symlink target IDs on primary. `getattr`, `setattr` and ioctl attributes for open/unlinked file IDs use current metadata from primary and remain bound to the handle's `file_id`. `link` verifies its source inode against primary metadata before selecting the source file ID. `read`/`write` enforce descriptor modes; unknown handles fail closed rather than falling back to a pathname. `flush`/`fsync` reject unknown handles. `copy_file_range` validates descriptor access modes before mutation. Mounted ACL access/inheritance and named-user parent-ACL denial for every create type, `unlink`/`rmdir` and both rename parents (exact `EACCES`), generic xattr and `FIONREAD` syscall checks pass; Linux v6.17 handles regular-file `FIONREAD` in VFS using the descriptor offset before FUSE ioctl dispatch. `FICLONE` is rejected by Linux FUSE before userspace on the tested kernel and is not implemented by FOD. Fallback-backed namespaces, SELinux MAC, clone range and broader ioctl cases remain outside that evidence. Root `getattr` projects its PostgreSQL row and migration 0028 updates parent/root timestamps transactionally for namespace changes. Timestamp `setattr` now authorizes and preserves fields in one primary transaction; path `truncate` repeats authorization in its content transaction under the file row lock. Partial-write/truncate-boundary inputs, write/copy planning sizes and copy source/dedupe blocks now load from primary. Explicit telemetry/control DSNs are checked against the data cluster and database before read-only session/lease registration. Next: audit remaining mutation preconditions. |
| 2 | POSIX conformance audit | The [audit matrix](../POSIX_CONFORMANCE_AUDIT.md) defines POSIX.1-2024 Issue 8 as the baseline and separates Linux/FUSE extensions. P02 now has mounted evidence for 512-entry directory iteration, cross-mount membership changes, rewind refresh, libc stream `ENOENT`/`EMFILE` behavior, and a 16-round threaded file-versus-directory create race with exactly one winner and preserved winner type/content. FOD 3.4.366 isolated schema-32 mounted regressions found and fixed two defects: `CREATE` had persisted the FUSE file-type bits and then rejected mode `0o100664` as outside `0o7777`, while directory-rename cycle SQL lost whitespace and reached PostgreSQL as `$1UNIONSELECT`. The eight-thread create/write/read stress test and 32-task `copy_file_range` test now pass on Linux 6.17/libfuse 3.17.4. On FOD 3.4.373, all nine `mount_smoke` tests matching `concurrent_` passed in `release-lto` against a fresh disposable PostgreSQL database on 2026-10-30; this includes 24 barrier-started parent/child file and directory creation races per type. The test helper accepts `FOD_TEST_CONFIG` for a disposable database and cleans its complete launcher process group. P10 now detects `F_SETLKW` cycles as `EDEADLK` between two processes on separate mounts using a PostgreSQL wait-for graph. Mounted fork/exec regressions confirm record locks are not inherited by a child and survive `exec()` when the locked descriptor remains open. It also has a mounted `EIO` regression for malformed active PostgreSQL range leases. P04 now tests `pwrite()` signed `off_t` addition overflow: Linux returns `EINVAL` in `rw_verify_area()` before FUSE, so this case cannot be mapped by FOD to POSIX `EFBIG`. `test_lseek.py` also sets `RLIMIT_FSIZE=4096` in a child process on a mounted FOD filesystem, verifies an 8192-byte request returns a 4096-byte partial write with `SIGXFSZ`, then checks `EFBIG` from later `write()` and `pwrite()` calls and unchanged payload. Signal interruption before any data transfer and backend `EIO` remain. P06 now has a mounted closed-descriptor regression proving `fstat()`, `fchmod()`, `fchown()` and `futimens()` return exact `EBADF` without modifying the file; Linux rejects them before FUSE. P05 now has `mtime`/`ctime` checks for path and descriptor truncation, exact `EBADF` for `ftruncate()` on a closed descriptor with unchanged size/timestamps/payload, a 24-round same-handle threaded `pwrite()`/`ftruncate()` serializability test and a two-mount stale-fence test proving old-handle `ftruncate()` returns `EBUSY` without changing the new writer payload. A two-mount P05 regression in `test_stale_writer_fencing.py` now starts mount A `pwrite()` and mount B pathname `truncate()` concurrently behind a barrier. The write completes, `truncate()` returns `EBUSY`, and primary PostgreSQL confirms the 4096-byte size, the writer's intended payload, and unchanged lease owner/token (isolated PostgreSQL run on 3.4.354, 2026-10-30). The test deliberately uses PostgreSQL for payload verification because mount B may retain a local stale read cache; cross-mount read coherence remains tracked separately under TX03J. P05 path-truncate authorization now has exact `EACCES` evidence for no file write permission, no parent search permission and denying POSIX ACL, with payload preservation checked through PostgreSQL where ACL blocks FUSE reads. Remaining P05 work is `EINTR` and I/O-error cases. P01 checks Linux `ENOENT` for empty pathnames across `stat()`, `lstat()`, `open()`, `mkdir()`, `unlink()`, `rmdir()`, `readlink()`, `access()`, `chmod()`, `chown()`, `utime()`, `truncate()`, `statvfs()`, `link()`, `symlink()` and `rename()`; libc `access()` is checked for `-1`/`ENOENT` because Python `os.access()` returns a boolean. A regular-file path ending in `/` returns `ENOTDIR` for metadata, removal, attribute, statvfs, hard-link source and rename operations, and `EEXIST` for mkdir and link/symlink destinations; the file remains intact. A trailing-slash symlink to a regular file returns `ENOTDIR` for metadata, removal, attribute, readlink, statvfs, hard-link source and rename operations; `mkdir()` and link/symlink destinations return `EEXIST`, while `access()` returns `-1`/`ENOTDIR`; the symlink and target remain intact. A trailing-slash symlink to a dangling target returns `ENOENT` for `stat()`, `lstat()`, `open()`, `truncate()`, `chmod()`, `chown()`, `utime()`, `readlink()`, `statvfs()`, hard-link source and `access()`; `mkdir()` and link/symlink destinations return `EEXIST`, and `unlink()`/`rmdir()` plus rename source/destination return `ENOTDIR`. A trailing-slash symlink to a directory resolves for `stat()`, `lstat()`, `open()`, `chmod()`, `utime()`, `chown()`, `statvfs()` and `access()`; `mkdir()` and link/symlink destinations give `EEXIST`, hard-linking the directory gives `EPERM`, `unlink()`/`rmdir()` and rename source/destination give `ENOTDIR`, `truncate()` gives `EISDIR`, and `readlink()` gives `EINVAL`. A path longer than the mount's `PC_PATH_MAX`, composed of single-byte components, returns exact `ENAMETOOLONG` for 17 path operations and libc `access()`; Linux rejects it before FUSE dispatch. `test_path_trailing_slash.py` also verifies Linux treats leading `//` and `///` like a single leading slash. POSIX leaves exactly two leading slashes implementation-defined; classify conditional `PATH_MAX` requirements, and continue review of empty components and other final-component rules. Continue requirement-level review; FOD build artifacts support 64-bit and wider userspace architectures only; 32-bit ABI `EOVERFLOW` checks are outside that platform matrix. P02 asserts exact sticky-directory `EPERM` and `rmdir(path/..)` `ENOTEMPTY`; system-wide `ENFILE`, `closedir()` failure behavior and remaining namespace errno mapping are still open. No general POSIX compliance claim is made. |
| 3 | TX03J cross-mount read coherence | The [coherence specification](../TX03J_CROSS_MOUNT_CACHE_COHERENCE.md) defines the contract; the [generation audit](../tx03j-generation-audit.md) distinguishes implemented PostgreSQL generation primitives from missing cross-host integration. A strict opt-in read path and local cache gate do not close default multi-host coherence. |
| 4 | Degraded read-only recovery | The [HA implementation plan](HA_READONLY_RECOVERY_AND_CACHE.md) owns the detailed phases and acceptance matrix. Explicit standby reads and role monitoring exist; live source switching, fencing proof validation and rejoin barriers remain open. Use controlled remount until those barriers are complete. |
| 5 | Controlled cache performance comparison | Follow the performance workstream in the [HA/cache plan](HA_READONLY_RECOVERY_AND_CACHE.md#workstream-b-cache-and-historical-performance-comparison), after the relevant correctness gates. Existing five-run pilot measurements are diagnostic evidence, not a release baseline. |
| 6 | Quantify clock-skew and queue-order evidence | The [time requirements](../POSTGRESQL_REQUIREMENTS.md#3-synchronizacja-czasu-hostów) require monitored NTP synchronization. The shortest effective lock lease is 1 s; validate 100 ms inter-node offset as a candidate against PostgreSQL failover and fencing before adopting a numeric limit. FIFO tickets order waiters within each process, but production telemetry does not correlate tickets with task identities. If historical task order must be diagnosed, add ticket/task correlation locally and authoritative transaction-order evidence across hosts; wall-clock log sorting is insufficient. |

These priorities do not imply a diagnosed root cause beyond the evidence in
the linked reviews. Recheck source and existing regressions before choosing a
fix. Source-review suspicions must stay distinct from reproduced defects.

## Shared acceptance rules

- Namespace mutations, leases, fencing, inode generations and cross-host
  ordering remain PostgreSQL-authoritative under the
  [architecture invariants](../FOD_ARCHITECTURE_INVARIANTS.md).
- HA role observation alone cannot admit writes. Recovery requires trusted
  identity, timeline and deployment fencing evidence, plus request draining,
  cache invalidation and fresh authoritative leases.
- Retain deterministic reproductions and the exact build profile/toolchain.
  Unexplained intermittent defects require ASM/ELF analysis as specified in
  [AGENTS.md](../../AGENTS.md).
- Follow [development](../DEVELOPMENT.md),
  [versioning](../versioning.md) and
  [validation procedures](../../zasady_sprawdzen.md). After every commit,
  inspect the complete parent-to-commit diff.

## Deferred work

Reopen `readdirplus`, QNAP COPY-buffer tuning, external-unmount teardown or
broad read-path optimization only for a concrete new correctness gap or
measurement. Longer-term proposals live in
[FOD_FUTURE_IDEAS.md](FOD_FUTURE_IDEAS.md), not in this queue.

## Completed work and supporting reviews

Schema upgrades now hold a PostgreSQL advisory lock, refuse active FOD
sessions, publish a durable mount-admission marker before migrations, reject
legacy session inserts through a PostgreSQL trigger, and atomically clear the
marker when publishing the final schema version and client floor. New runtime
builds also reject replicas below their required schema version. See
[`TODO.md`](../../TODO.md) for test evidence and the all-host unmount
requirement.

The former plan, including completed fallocate, sparse seek, create/rename,
open/unlink convergence and inode/path cache retirement, is preserved in
[the 2026-10-06 plan archive](../history/FOD_CURRENT_PLAN_2026-10-06.md).
[HISTORY.md](../HISTORY.md) indexes older completed plans and measurements.
The [documentation index](../README.md#correctness-reviews-and-evidence)
connects the transaction, privilege, write-fencing and authority reviews.

Update this queue when a task closes or its priority changes. Keep detailed
experiments and acceptance matrices in the linked topic document rather than
duplicating them here.

Recent authority-audit increment: identical-write suppression and the
non-adoption `copy_file_range` path now read source bytes from primary. A
paused-replica regression verifies both behaviors against PostgreSQL payload
rows. Truncate-boundary planning now reads its previous EOF from primary too;
the two-mount regression pauses replica replay, extends on primary, then
extends through the replica-routed mount and verifies preserved bytes via FUSE
and in the primary payload row. Continue the remaining mutation-precondition
audit; this closes one stale-read path, not the entire authority audit.
Create/open rollback cleanup also stopped choosing promotion versus purge from
replica-routed link counts: primary promotion is attempted first, then primary
purge rechecks links and leases under the lifecycle lock.
