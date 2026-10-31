# FOD current implementation plan

Updated: 2026-10-31. This is the single implementation queue. Work is performed
on `main`. Implemented behavior belongs in [CURRENT_STATE.md](../CURRENT_STATE.md);
long-term proposals belong in [ROADMAP.md](../../ROADMAP.md).

## Implementation order

| Priority | Work | Current boundary and completion evidence |
| --- | --- | --- |
| 1 | Primary-authoritative mutation decisions | The [authority audit](../FOD_PRIMARY_AUTHORITY_AUDIT.md) records verified unlink lifetime, atomic rmdir, cross-type creation, transactional chmod/chown and sticky-bit checks. Default ACL inheritance, create authorization and parent ACL checks for unlink/rmdir/rename now share the relevant PostgreSQL mutation transaction. `lookup` and namespace mutations verify that cached parent paths still resolve to the requested directory inode on primary; `opendir` checks directory inode identity on primary before snapshot creation. The open handle retains its PostgreSQL directory ID for initial/rewind snapshots, names-mode child metadata and atime updates, so readdir cannot switch an old handle to a replacement directory at the same path. Two-mount regressions cover stale-inode `opendir`, `readlink` and remove/recreate during an open directory stream; `readlink` rejects a stale symlink inode with `ENOENT` and reads replacement targets only after a fresh lookup. `unlink` and `rmdir` compare the entry's resolved parent ID. Create callbacks are verified with replica replay paused: `mkdir`, `create`, `mknod`, `symlink` and `link` return primary-committed metadata before replica visibility, while conflicting creates return `EEXIST`. Generic xattr set/remove authorization and `CREATE`/`REPLACE` semantics now share the locked inode transaction; all xattr callbacks validate the request inode against primary-resolved metadata (or a live handle file ID after unlink) and use its database owner ID, with a passing two-mount regression for stale cached paths and open-unlinked handles. `user.*` reads check ACL/mode from the verified inode owner, including open-unlinked files; unprivileged `listxattr` hides `trusted.*`. FUSE access/open permissions, inode identity for open, lock resource identity/type checks, and write-ownership parent IDs use the current primary namespace. Atime updates resolve file, directory and symlink target IDs on primary. `getattr`, `setattr` and ioctl attributes for open/unlinked file IDs use current metadata from primary and remain bound to the handle's `file_id`. `link` verifies its source inode against primary metadata before selecting the source file ID. `read`/`write` enforce descriptor modes; unknown handles fail closed rather than falling back to a pathname. `flush`/`fsync` reject unknown handles. `copy_file_range` validates descriptor access modes before mutation. Mounted ACL access/inheritance and named-user parent-ACL denial for every create type, `unlink`/`rmdir` and both rename parents (exact `EACCES`), generic xattr and `FIONREAD` syscall checks pass; Linux v6.17 handles regular-file `FIONREAD` in VFS using the descriptor offset before FUSE ioctl dispatch. `FICLONE` is rejected by Linux FUSE before userspace on the tested kernel and is not implemented by FOD. Fallback-backed namespaces, SELinux MAC, clone range and broader ioctl cases remain outside that evidence. Root `getattr` projects its PostgreSQL row and migration 0028 updates parent/root timestamps transactionally for namespace changes. Timestamp `setattr` now authorizes and preserves fields in one primary transaction; path `truncate` repeats authorization in its content transaction under the file row lock. Partial-write/truncate-boundary inputs, write/copy planning sizes and copy source/dedupe blocks now load from primary. Full-object copy adoption serializes source and destination against fenced writes with sorted resource-key locks; the C9 two-mount regression passed on FOD 3.4.397, and C7/C8 ownership/fencing regressions passed on the same stack. Explicit telemetry/control DSNs are checked against the data cluster and database before read-only session/lease registration. Paused-replica regressions verify root `access(X_OK)` callback behavior, kernel `default_permissions` attribute refresh through primary-backed `getattr()`, and denial of `open()` after parent search permission is revoked on primary. The refresh regression uses a read-only mount with configured positive attribute TTL. Next: continue auditing remaining mutation preconditions. |
| 2 | POSIX/Linux/FUSE compatibility audit | The [overview](../compatibility-contracts.md) is the compact status page; the [POSIX ledger](../POSIX_CONFORMANCE_AUDIT.md) owns P01-P12, and the [FUSE guide](../FUSE_REQUIREMENTS.md) owns the runtime floor and callback inventory. Only two `lseek()` case groups are Pass; 12 POSIX areas remain Pending, with no general POSIX claim. Linux 6.17.0/libfuse3 3.18.2 are enforced minimums; focused mounted tests passed on Linux 6.17.0-41 with libfuse3 3.18.2 and 3.18.3, but broad distro coverage remains open. The write/truncate killpriv gap is implemented and mounted regressions pass on Linux 6.17.0-41/libfuse3 3.18.2 and 3.18.3; details and the kernel's separate capability-xattr removal transaction are recorded in [`TODO.md`](../../TODO.md). FOD 3.4.427 also maps poisoned per-handle write-state locks to `EIO`; this local failure handling does not close a POSIX matrix area. Remaining: close or explicitly scope each pending POSIX area and validate supported distribution packages and continue mapping the remaining Linux/FUSE callback boundaries. |
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
