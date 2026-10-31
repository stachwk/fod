# FOD current implementation plan

Updated: 2026-10-31. This is the single implementation queue. Work is performed
on `main`. Implemented behavior belongs in [CURRENT_STATE.md](../CURRENT_STATE.md);
long-term proposals belong in [ROADMAP.md](../../ROADMAP.md).

## Implementation order

| Priority | Work | Current boundary and completion evidence |
| --- | --- | --- |
| 1 | Primary-authoritative mutation decisions | The [authority audit](../FOD_PRIMARY_AUTHORITY_AUDIT.md) records verified unlink lifetime, atomic rmdir, cross-type creation, transactional chmod/chown and sticky-bit checks. Default ACL inheritance, create authorization and parent ACL checks for unlink/rmdir/rename now share the relevant PostgreSQL mutation transaction. `lookup` gets the directory ID from the same primary metadata row that validates the parent inode, then resolves children by `(directory_id, name)`; namespace mutations also use the validated parent ID. `opendir` checks directory inode identity on primary before snapshot creation. Its `..` inode comes from the opened directory parent ID in PostgreSQL and is fetched by ID from primary; a mounted `getdents64()` regression passes after deliberate parent-path cache eviction. The open handle retains its PostgreSQL directory ID for initial/rewind snapshots, names-mode child metadata and atime updates, so readdir cannot switch an old handle to a replacement directory at the same path. Two-mount regressions cover stale-inode `opendir`, `readlink` and remove/recreate during an open directory stream; `readlink` rejects a stale symlink inode with `ENOENT` and reads replacement targets only after a fresh lookup. `unlink` and `rmdir` compare the entry's resolved parent ID. Create callbacks are verified with replica replay paused: `mkdir`, `create`, `mknod`, `symlink` and `link` return primary-committed metadata before replica visibility, while conflicting creates return `EEXIST`. Generic xattr set/remove authorization and `CREATE`/`REPLACE` semantics now share the locked inode transaction; all xattr callbacks validate the request inode against primary-resolved metadata (or a live handle file ID after unlink) and use its database owner ID, with a passing two-mount regression for stale cached paths and open-unlinked handles. `user.*` reads check ACL/mode from the verified inode owner, including open-unlinked files; unprivileged `listxattr` hides `trusted.*`. FUSE access/open permissions, inode identity for open, lock resource identity/type checks, and write-ownership parent IDs use the current primary namespace. Atime updates resolve file, directory and symlink target IDs on primary. `getattr`, `setattr` and ioctl attributes for open/unlinked file IDs use current metadata from primary and remain bound to the handle's `file_id`. `link` verifies its source inode against primary metadata before selecting the source file ID. `read`/`write` enforce descriptor modes; unknown handles fail closed rather than falling back to a pathname. `flush`/`fsync` reject unknown handles. `copy_file_range` validates descriptor access modes before mutation. Mounted ACL access/inheritance and named-user parent-ACL denial for every create type, `unlink`/`rmdir` and both rename parents (exact `EACCES`), generic xattr and `FIONREAD` syscall checks pass; Linux v6.17 handles regular-file `FIONREAD` in VFS using the descriptor offset before FUSE ioctl dispatch. `FICLONE` is rejected by Linux FUSE before userspace on the tested kernel and is not implemented by FOD. Fallback-backed namespaces, SELinux MAC, clone range and broader ioctl cases remain outside that evidence. Root `getattr` projects its PostgreSQL row and migration 0028 updates parent/root timestamps transactionally for namespace changes. Timestamp `setattr` now authorizes and preserves fields in one primary transaction; path `truncate` repeats authorization in its content transaction under the file row lock. Partial-write/truncate-boundary inputs, write/copy planning sizes and copy source/dedupe blocks now load from primary. Full-object copy adoption serializes source and destination against fenced writes with sorted resource-key locks; the C9 two-mount regression passed on FOD 3.4.397, and C7/C8 ownership/fencing regressions passed on the same stack. Explicit telemetry/control DSNs are checked against the data cluster and database before read-only session/lease registration. Paused-replica regressions verify root `access(X_OK)` callback behavior, kernel `default_permissions` attribute refresh through primary-backed `getattr()`, and denial of `open()` after parent search permission is revoked on primary. The refresh regression uses a read-only mount with configured positive attribute TTL. Indexer cleanup refuses `materialize_running` plans; FOD 3.4.515 routes fallible root/block-size setup failures through rollback so ordinary errors do not strand a partial tree. FOD 3.4.519 implements schema-39 materialization leases and per-transaction fencing; crash takeover and cleanup now use PostgreSQL row locking and a monotonic token, while runtime regressions remain pending. FOD 3.4.514 replaced stale shared-object count assignment with an atomic decrement and guarded exclusive-object deletion. FOD 3.4.513 refuses cleanup when imported files have external hardlink aliases. The external-hardlink cleanup regression is now added but not yet run; the shared-reference concurrency regression is added but not yet run, and the new schema-39 stale-plan recovery implementation awaits runtime regressions in `TODO.md`; continue with remaining mutation preconditions. |
| 2 | POSIX/Linux/FUSE compatibility audit | The [overview](../compatibility-contracts.md) is the compact status page; the [POSIX ledger](../POSIX_CONFORMANCE_AUDIT.md) owns P01-P12, and the [FUSE guide](../FUSE_REQUIREMENTS.md) owns the runtime floor and callback inventory. Only two `lseek()` case groups are Pass; 12 POSIX areas remain Pending, with no general POSIX claim. Linux 6.17.0/libfuse3 3.18.2 are enforced minimums; focused mounted tests passed on Linux 6.17.0-41 with libfuse3 3.18.2 and 3.18.3, but broad distro coverage remains open. FOD 3.4.437 preserves an open rename target inode across mounts until the final lease is released. `test_path_trailing_slash.py` now checks exact `EILSEQ` for invalid-UTF-8 rename source and destination and protects valid U+FFFD aliases. The write/truncate killpriv gap is implemented and mounted regressions pass on Linux 6.17.0-41/libfuse3 3.18.2 and 3.18.3; details and the kernel's separate capability-xattr removal transaction are recorded in [`TODO.md`](../../TODO.md). FOD 3.4.428 covers both mixed timestamp sentinel orders in the mounted ACL/timestamp test and maps poisoned per-handle write-state locks to `EIO`; FOD 3.4.435 fixes umask handling with default ACLs and tests nested inheritance; FOD 3.4.436 checks umask application to files, directories and FIFOs without a default ACL. These focused cases do not close a POSIX matrix area. The readdir snapshot/descriptor-limit test passed on 3.4.428 with `open()` and `opendir()` both returning `EMFILE`. FOD 3.4.458 also verifies nested `..` `d_ino` after local parent-cache eviction on PostgreSQL 16.15 `BLCKSZ=32KiB`/Linux 6.17.0-41/libfuse3 3.18.3; P02 remains Pending for the other listed cases. Symlink hard links are in implementation (FOD 3.4.461/schema 38): PostgreSQL schema, namespace lookup/readdir, shared inode/xattr metadata, link count, link/unlink promotion and alias rename are present; the focused database test passes on the default PostgreSQL 16.15 `BLCKSZ=32KiB`. The mounted regression is added as `make test-symlink-hardlink`; its first run with Linux 6.17.0-41/libfuse 3.18.3 was denied by the environment (`EPERM`, `CapEff=0`) before assertions, and two-mount race coverage remains open; follow [`SYMLINK_HARDLINK_DESIGN.md`](../SYMLINK_HARDLINK_DESIGN.md) and [`TODO.md`](../../TODO.md). Then close or explicitly scope each pending POSIX area and validate supported distribution packages and continue mapping the remaining Linux/FUSE callback boundaries. |
| 3 | TX03J cross-mount read coherence | The [coherence specification](../TX03J_CROSS_MOUNT_CACHE_COHERENCE.md) defines the contract; the [generation audit](../tx03j-generation-audit.md) distinguishes implemented PostgreSQL generation primitives from missing cross-host integration. Local writes now invalidate the Rust block cache under its per-file publication gate; the source review confirms there is no durable invalidation outbox, production FUSE notifier worker or `fsync()` ACK barrier yet. Before implementing the barrier, define what a kernel ACK proves and how in-flight reads are drained or rejected. The opt-in strict path and local cache gate do not close default multi-host coherence; see the open TX03J item in `TODO.md`. |
| 4 | Degraded read-only recovery | The [HA implementation plan](HA_READONLY_RECOVERY_AND_CACHE.md) owns the detailed phases and acceptance matrix. Explicit standby reads and role monitoring exist; live source switching, fencing proof validation and rejoin barriers remain open. Use controlled remount until those barriers are complete. |
| 5 | Controlled cache performance comparison | Follow the performance workstream in the [HA/cache plan](HA_READONLY_RECOVERY_AND_CACHE.md#workstream-b-cache-and-historical-performance-comparison), after the relevant correctness gates. Existing five-run pilot measurements are diagnostic evidence, not a release baseline. |
| 6 | Quantify clock-skew and queue-order evidence | The [time requirements](../POSTGRESQL_REQUIREMENTS.md#3-synchronizacja-czasu-hostów) require monitored NTP synchronization. The shortest effective lock lease is 1 s; validate 100 ms inter-node offset as a candidate against PostgreSQL failover and fencing before adopting a numeric limit. FIFO tickets order waiters within each process, but production telemetry does not correlate tickets with task identities. If historical task order must be diagnosed, add ticket/task correlation locally and authoritative transaction-order evidence across hosts; wall-clock log sorting is insufficient. |
| 7 | Signed `BIGINT` range decision | Closed: retain PostgreSQL `BIGINT`; no current FOD contract requires values above `i64::MAX`. File sizes follow 64-bit `off_t`, PostgreSQL generates IDs, block indices are bounded by file size, and FUSE link-count overflow is reported. Reopen only for an explicit per-field wider-range requirement. Evidence: [SQL schema limit audit](../SQL_SCHEMA_LIMIT_AUDIT.md), [TODO](../../TODO.md). |

Recent database-decoder hardening is recorded in the [primary-authority audit](../FOD_PRIMARY_AUTHORITY_AUDIT.md): FOD 3.4.490 rejects malformed namespace metadata rows, FOD 3.4.491 rejects invalid IDs in create-confirmation queries, FOD 3.4.492 enforces exact hardlink and endpoint-probe result shapes, FOD 3.4.493 validates copy and expired-session results, FOD 3.4.494 runs schema-corruption and session-pruning operation tests against disposable schema-38 databases, FOD 3.4.495 rejects malformed boolean results at lease, fencing, schema-admission and namespace decision boundaries, FOD 3.4.496 extends that strict decoding to indexer reads, snapshots and cleanup, FOD 3.4.497 applies exact scalar-boolean result validation to mkfs and FUSE runtime-override discovery, FOD 3.4.498 tests the mkfs boolean result shape and NULL contract without a database, FOD 3.4.499 rejects non-positive data-object IDs in both file metadata readers, FOD 3.4.500 validates positive data-object IDs returned by object creation, copy and request-token resolution, FOD 3.4.501 rejects malformed boolean state in file purge and rename-replacement lease checks, FOD 3.4.502 rejects zero backing-symlink IDs from symlink-hardlink lookup, FOD 3.4.503 rejects zero entity IDs across indexer result models and cleanup selections, FOD 3.4.504 finishes routing PostgreSQL booleans in unlink, rmdir, payload-empty and xattr/ACL checks through strict parsing, and FOD 3.4.505 validates positive lifecycle/session/namespace IDs before use. FOD 3.4.506 rejects malformed startup recovery, schema/config readiness and upgrade-state booleans. FOD 3.4.507 validates positive matching tokens returned by write-lease heartbeats. FOD 3.4.508 requires positive fencing tokens when acquiring write leases. FOD 3.4.509 rejects zero tokens in caller-provided leases and persistence fences. Indexer cleanup follow-ups: FOD 3.4.513 protects external hardlink aliases, 3.4.514 applies atomic reference-count deltas, and 3.4.515 rolls back ordinary materialization setup errors; direct cleanup regressions remain pending. The shared database remains schema 31; temporary database administration connects to `postgres`, outside the FOD schema gate. Verify the actual FOD PostgreSQL build and runtime page size before each DB test.

FOD 3.4.475 closes one `link()` mutation-precondition gap: PostgreSQL now
revalidates the exact source entry inside the transaction. The focused
schema-38 regression passed with verified PostgreSQL 16.15 `BLCKSZ=32KiB`.
The mounted two-host race remains open because the local host has libfuse 3.17.4;
see the authority audit and `TODO.md`.

FOD 3.4.477 hardens PostgreSQL single-row and first-column result decoding:
missing rows are distinct from malformed shapes, NULL fields fail closed, and
unexpected multiple rows are rejected. The focused decoder regression passed
on the verified PostgreSQL 16.15 `BLCKSZ=32KiB` runtime. FOD 3.4.478 extends
strict NULL and shape validation to general multirow results; its focused
regression passed after checking the image, custom `pg_config` build flags and
live 32768-byte page size. Continue the broader database fault-injection audit.
FOD 3.4.479 extends exact result-shape checks to specialized xattr and block
read decoders. All 72 `pg_query` tests passed serially against a disposable
schema-38 admin database on the verified PostgreSQL 16.15 `BLCKSZ=32KiB` build;
the configured schema-31 admin database was left unchanged and the disposable
database was dropped after the run.
FOD 3.4.480 applies exact result-column contracts to the remaining optional
row readers and adds a zero-row/mismatched-column regression. Continue the
broader database fault-injection audit. The focused decoder regression and all
72 `pg_query` tests passed against the verified PostgreSQL 16.15
`BLCKSZ=32KiB` server.
FOD 3.4.481 applies exact result-column contracts to every remaining production
optional-row reader; the generic helper is now used only by decoder tests. The
focused decoder regression and all 72 `pg_query` tests passed against an
isolated PostgreSQL 16.15 database with `BLCKSZ=32KiB`. Continue the broader
database fault-injection audit.
FOD 3.4.482 hardens the PostgreSQL lock-range snapshot: it requires five
columns and rejects malformed NULLs while preserving nullable `range_end` as
the empty-field representation. All 10 lock-manager tests passed against an
isolated, initialized database on the verified 32 KiB PostgreSQL build.
Continue the broader database fault-injection audit.
FOD 3.4.483 makes all namespace insert `RETURNING` readers require a single
positive ID and rejects malformed result cardinality or shape. The focused
decoder regression and all 72 `pg_query` tests passed on the verified isolated
32 KiB PostgreSQL database. Continue the broader database fault-injection
audit.
FOD 3.4.484 adds exact cardinality and positivity checks to staged index ID
upserts. The isolated pure decoder regression passes; continue reviewing
database-backed staging behavior and the broader fault-injection audit.
FOD 3.4.485 makes namespace create-confirmation and startup queries validate
exact row cardinality and column shape. The focused decoder regression and all
72 `pg_query` tests passed against an isolated PostgreSQL 16.15 database with
verified 32 KiB pages. Continue the broader database fault-injection audit.
FOD 3.4.486 extends strict row-shape validation to transactional chmod and
timestamp authorization metadata. All 72 `pg_query` tests passed on an
isolated current-schema database on the verified 32 KiB build. Continue the
broader fault-injection audit.
FOD 3.4.487 makes directory-entry metadata lookup reject multiple rows for
one parent/name instead of selecting the first namespace kind. All 72
`pg_query` tests passed on the verified 32 KiB PostgreSQL build. Continue the
broader fault-injection audit.
FOD 3.4.488 makes hash and import-plan result readers fail on malformed SQL
row widths instead of omitting rows; the pure shape-validator regression is
in place. Continue the broader indexer and database fault-injection audit.
FOD 3.4.489 applies exact result-shape checks throughout indexer row decoders,
including source, snapshot, catalogue, cleanup and aggregate reads. All 35
indexer unit tests passed; continue broader database fault-injection testing.

The normal SELinux/ACL and HA-chain Docker test stacks now use the FOD 32 KiB
PostgreSQL image and verify the live page size in their healthchecks; both
stacks were run successfully on 2026-10-31. The explicit 8 KiB variant remains
limited to compatibility/comparison work.

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

FOD 3.4.451 fixes a separate `create()` reply race: after the create transaction
committed, a second mount could replace the path before FUSE loaded reply
attributes, pairing the replacement inode with the original file handle. The
callback now builds attributes from the handle's committed `file_id`. The
two-mount regression failed with the old path lookup and passed with the fix;
the full create/write ownership test passed on Linux 6.17.0-41/libfuse3 3.18.3
against a disposable PostgreSQL 16.15 database. Continue auditing remaining
mutation preconditions.

FOD 3.4.452 extends ID-bound replies to `mkdir`, `mknod`, `symlink` and `link`.
The same mounted regression now replaces each destination after commit and
before reply; the original create returns `EIO` if its object was deleted, and
hardlink replies retain the source inode after an alias replacement. Fixes also
cover root-inode parsing for ID-based metadata and reverse path selection for
hardlinked inodes. The full test passed against PostgreSQL 16.15 with
`BLCKSZ=32KiB` on Linux 6.17.0-41/libfuse3 3.18.3. The active primary-authority
audit remains open for other mutation preconditions.

FOD 3.4.453 binds `open()` to the file ID from the inode attributes already
validated for that request. A deterministic two-mount regression confirms a
concurrent unlink/recreate returns `ENOENT` for the stale inode instead of
opening the replacement. The full create/write ownership test passed on Linux
6.17.0-41/libfuse3 3.18.3 against PostgreSQL 16.15 with `BLCKSZ=32KiB`; see the
[primary-authority audit](../FOD_PRIMARY_AUTHORITY_AUDIT.md).

FOD 3.4.454 makes `poll()` use the open handle's file ID and keeps regular-file
read readiness true at EOF. The mounted `test_poll.py` reproduced `POLLERR`
after remote unlink before the fix and now passes for empty files, open-unlinked
descriptors, and a read-only mount on PostgreSQL 16.15 `BLCKSZ=32KiB`/libfuse3
3.18.3.

FOD 3.4.455 makes parent inode validation and parent `directory_id` selection
use the same PostgreSQL metadata result. `lookup()` also resolves the child by
the verified directory ID and name, then reads attributes by object ID. The
two-mount barrier regressions replace the parent during `mkdir()` and
`lookup()`; stale requests return `ENOENT` and leave the replacement
unchanged. The full ownership test passed on PostgreSQL 16.15 `BLCKSZ=32KiB`,
Linux 6.17.0-41/libfuse3 3.18.3.
Create/open rollback cleanup also stopped choosing promotion versus purge from
replica-routed link counts: primary promotion is attempted first, then primary
purge rechecks links and leases under the lifecycle lock.

FOD 3.4.457 fixes rename's source inode cache handling: source attributes are
loaded by the entry ID from authoritative resolution, and a failed read returns
`EIO` before mutation instead of substituting root inode 1. The deterministic
fault-injection regression and full two-mount create/write ownership test pass
on the PostgreSQL 16.15 `BLCKSZ=32KiB` runtime.

Schema 35 replaces unbounded indexer text B-tree keys with digest candidate
indexes, full-value comparisons and PostgreSQL advisory serialization. The
long-path integration regression passed with a 3000-byte source name and
3041-byte path, including scan and snapshot readback; the schema-upgrade test
also passed. The compatibility PG8 build reproduced a 3216-byte B-tree key
failure at the 2704-byte limit, while the digest-index test passed on both PG8
and the default PG32 build. See [`SQL_SCHEMA_LIMIT_AUDIT.md`](../SQL_SCHEMA_LIMIT_AUDIT.md)
and [`TODO.md`](../../TODO.md) for the implementation and evidence.

Schema 36 normalizes legacy full `st_mode` values after verifying the inode
type bits, then constrains file and directory modes to octal permission and
special bits. A read-only scan of the active schema 31 database found 800
affected file rows; the live database was not changed. The isolated schema
upgrade regression seeds legacy file/directory values before upgrading. The
schema 37 widened all 45 entity ID/reference columns and 17 `SERIAL` sequences
to `BIGINT`, preserving 29 foreign keys and 7 triggers; fresh schema and
runtime lock-table DDL agree. Its schema-upgrade regression passes on an
isolated database. No production migration was run. Before upgrading a real
database, schedule a maintenance window for the column/index locks and
rewrites. The remaining SQL-domain question is whether any counter truly needs
unsigned values above signed `BIGINT`.
