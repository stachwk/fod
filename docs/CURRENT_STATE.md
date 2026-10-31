# FOD current state

This document is the compact canonical description of the **current** FOD architecture and operational defaults. Historical versioned documents remain useful evidence but may describe defaults that have since changed.

The authoritative release version is stored in `../fod_version.txt`.

Current POSIX/Linux/FUSE support status is summarized in
[`compatibility-contracts.md`](compatibility-contracts.md). The POSIX audit is
still in progress and does not claim general POSIX conformance. The enforced
Linux/libfuse minimums and their validation boundary are in
[`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md).

FOD 3.4.462 introduced schema version 38 and `symlink_hardlinks`,
directory-entry aliases for shared symlink inodes. The current FOD 3.4.465
continues to require schema version 38. Link,
lookup/readdir, shared metadata and xattrs, `st_nlink`, unlink promotion and
rename promotion are implemented in progress. A mounted regression is now
available as `make test-symlink-hardlink`. It was attempted with local libfuse
3.18.3 and Linux 6.17.0-41, but the environment denied the mount with `EPERM`
(`CapEff=0`); cross-mount race verification remains open. See
[`SYMLINK_HARDLINK_DESIGN.md`](SYMLINK_HARDLINK_DESIGN.md).

FOD 3.4.463 also makes shared PostgreSQL scalar decoders reject missing, NULL,
or malformed result rows rather than silently producing empty values; optional
scalar reads preserve the distinct zero-row case. Focused database regression
coverage is in `scalar_text_decoders_distinguish_absent_null_and_malformed_results`.

FOD 3.4.464 closes cached libpq connections when a repository's connection
pools are dropped and reports a missing data-object reference count through the
owning operation instead of losing that context in the scalar decoder. The
`pg_query` suite now verifies pool cleanup and passed all 66 tests against a
fresh disposable PostgreSQL 16.15 database built with `BLCKSZ=32KiB`.

FOD 3.4.465 rejects namespace lookup results with duplicate rows or malformed
column counts. `resolve_path` fetches up to two candidates so it can report a
cross-table name collision instead of silently selecting one. The
`duplicate_symlink_namespace_rows_are_rejected` regression reproduces this
corruption and verifies both direct symlink lookup and path resolution fail
closed; all 67 `pg_query` tests passed on the disposable 32KiB database.

FOD 3.4.466 closes and releases PostgreSQL connections if a database callback
panics while the connection is checked out. The original panic continues to
unwind; the connection is not returned to the pool in an open transaction.
`panicking_transaction_releases_its_postgres_connection` reproduced the leaked
session before the fix and verifies that the session count returns to baseline
and the repository remains usable afterward. The full 68-test `pg_query`
suite passed on a fresh disposable PostgreSQL 16.15 database with
`BLCKSZ=32KiB`.

FOD 3.4.467 preserves the no-conflict/no-longer-owned cases after strict scalar
decoding: lock acquisition now represents an absent conflict row as an empty
value, and stale write-lease heartbeats treat zero `RETURNING` rows as lost
ownership. Lock-range state regressions now assert the persisted `lock_pid`
field. The complete `fod-rust-hotpath` test suite passed on a fresh disposable
PostgreSQL 16.15 database with verified `BLCKSZ=32KiB`.

FOD 3.4.430 adds passing mounted `rename()` coverage for file/symlink
replacement pairings, including `EISDIR`, `ENOTDIR` and rollback preservation;
FOD 3.4.431 extends these checks to verify committed namespace changes and
rejected-operation preservation from both mounts. FOD 3.4.432 adds FIFO/file
replacement cases and verifies the resulting type and cross-mount visibility.
FOD 3.4.433 adds a sticky-directory rename denial check: unprivileged rename
returns `EPERM` and preserves both entries. FOD 3.4.434 verifies that owning
only the source or only the destination still returns `EPERM`, while owning
both permits the rename. FOD 3.4.435 fixes umask handling with POSIX ACLs: ACL
mounts negotiate `FUSE_DONT_MASK` with `FUSE_POSIX_ACL`, because Linux otherwise
applies umask before the FOD callback. A parent default ACL filters the original
requested creation mode; without one, FOD applies umask inside the primary
transaction. Mounted coverage checks capability negotiation, ACL filtering,
directory inheritance, nested files, and umask application for files,
directories and FIFOs when no default ACL exists. P07 remains open for other
POSIX permission and identity requirements.

FOD 3.4.453 keeps `open()` bound to the inode validated for the FUSE request.
A two-mount barrier regression replaces the pathname after inode validation;
the stale open now returns `ENOENT` instead of opening the replacement. The
full create/write ownership test passed against PostgreSQL 16.15 built with
`BLCKSZ=32KiB`, Linux 6.17.0-41 and libfuse3 3.18.3.

FOD 3.4.455 derives a parent directory's database ID from the same PostgreSQL
metadata row that validates its FUSE inode. A two-mount regression replaces
the directory while `mkdir()` or `lookup()` is paused after validation; both
stale requests now return `ENOENT`. `lookup()` resolves a child from the
verified PostgreSQL directory ID and name, then fetches its attributes by
object ID. The full ownership test passed on PostgreSQL 16.15 with
`BLCKSZ=32KiB`, Linux 6.17.0-41 and libfuse3 3.18.3.

FOD 3.4.456 records `unlink` journal identity from the committed operation
result instead of resolving the pathname after commit. A two-mount barrier
test recreates the name before journal insertion and verifies the single
`unlink` event has no file ID rather than referencing the replacement. The
full create/write ownership test passed against PostgreSQL 16.15
`BLCKSZ=32KiB`, Linux 6.17.0-41 and libfuse3 3.18.3.

FOD 3.4.457 binds `rename()`'s source inode to the entry ID already resolved by
PostgreSQL. It now propagates source-attribute read errors before committing
the rename instead of proceeding with `ROOT_INO` as a fallback that could
poison local inode-to-path state.

FOD 3.4.458 derives the `..` entry's inode in `opendir()` from the directory's
parent ID in PostgreSQL and reads the parent's inode by ID from the primary.
A mounted regression evicts the local parent-path cache immediately before
opening a nested directory and verifies the `getdents64()` `d_ino` for `..`.
The full readdir snapshot/descriptor-limit test passed on PostgreSQL 16.15 with
`BLCKSZ=32KiB`, Linux 6.17.0-41-generic and libfuse3 3.18.3.

FOD 3.4.454 binds `poll()` metadata to the open handle's `file_id`, preserving
read readiness after a remote unlink. Regular files also report read readiness
at EOF. Mounted coverage for an empty file, an open-unlinked descriptor, and a
read-only mount passed against PostgreSQL 16.15 with `BLCKSZ=32KiB` and
libfuse3 3.18.3.

FOD 3.4.452 binds `mkdir`, `mknod`, `symlink` and `link` replies to the
PostgreSQL-committed entry or source file ID. A two-mount regression replaces
the path between commit and reply; deleted originals return `EIO`, and hardlink
replies keep the source inode when the alias is replaced. It also covers the
root-inode metadata parsing and local reverse-path defects found while adding
these cases. The full `test_create_write_ownership.py` passed against
PostgreSQL 16.15 built with `BLCKSZ=32KiB`, Linux 6.17.0-41 and libfuse3 3.18.3.

## Runtime architecture

FOD is Rust-backed end to end:

- `rust_fuse` owns the FUSE frontend and filesystem callbacks,
- `rust_runtime` owns PostgreSQL connectivity, configuration and shared runtime services,
- `rust_hotpath` contains storage/read/write hot-path helpers,
- `rust_mkfs` owns schema/bootstrap/config tooling,
- `rust_monitor` owns runtime and cluster diagnostics,
- `rust_indexer` owns external-source indexing/import tooling.

PostgreSQL owns durable filesystem metadata, payload, shared lock/session state and replication.

`FOD_WORKERS_READ` parallelizes reads when uncached stored blocks fall into
multiple disjoint ranges and meet `FOD_WORKERS_READ_MIN_BLOCKS`. The mounted
`concurrent_read_tasks_on_sparse_file_preserve_parallel_block_ranges` test
uses 4 workers, 16 stored blocks separated by holes and 8 simultaneous readers;
all readers verify both payload and zero-filled holes. It passed on FOD 3.4.350
against a disposable initialized database.
The mounted `concurrent_threads_run_multiple_copy_file_range_tasks` regression
starts 32 copy tasks together across 8 FUSE event threads and checks every
destination byte-for-byte. On FOD 3.4.366, writable `CREATE` was fixed to strip
the file-type bits included in FUSE's mode argument before persisting POSIX
permission bits. The same stress run exposed and fixed missing whitespace in
the recursive directory-rename cycle query. Both the concurrent
create/write/read and 32-task copy tests pass against a disposable schema-32
database on Linux 6.17/libfuse 3.17.4. `FOD_TEST_CONFIG` isolates Rust FUSE
tests from a developer's shared database. Older failed test attempts left
test-owned mount artifacts documented in [`TODO.md`](../TODO.md).
Whole-object `copy_file_range` adoption now serializes source and destination
writes using PostgreSQL advisory locks in sorted resource-key order. The mounted
C9 regression confirms that a source write after the copy detaches its data
object and leaves destination bytes unchanged; details and environment are in
the [primary-authority audit](FOD_PRIMARY_AUTHORITY_AUDIT.md).
FOD 3.4.451 also binds the FUSE `create()` reply attributes to the file ID held
by its open handle. A two-mount test replaces the created path after commit but
before reply, verifying the descriptor retains the original inode and payload
instead of receiving metadata from the replacement.
The eight-thread `concurrent_threads_unlink_and_replace_many_open_file_tasks`
regression runs 32 unlink/recreate tasks together and verifies each old open
descriptor still reads its original inode while each pathname reads a distinct
replacement inode.
The multithreaded namespace suite also races parent-directory creation against
child-file and child-directory creation for 24 fresh names of each type. Each
child create must either succeed after the parent is available or return
`ENOENT`; the final path state must match the syscall result. The case is part
of `make test-fuse-multithreaded-tasks`. On FOD 3.4.373, this regression and
the eight other `mount_smoke` tests matching `concurrent_` passed in
`release-lto` against a fresh disposable PostgreSQL database on 2026-10-30.
The permissions regression invokes libc `opendir()` as `nobody` on a mode-000
directory and checks exact `EACCES`; the mounted ACL and permissions tests pass.
The FUSE `open()` callback keeps ACL denial (`EACCES`) distinct from failed
ACL metadata reads (`EIO`) and malformed stored ACL data (`EINVAL`). A unit
regression covers the mapping, and `test_acl_mount_option.py` confirms mounted
`EINVAL` plus the callback log after corrupting only the file's stored ACL.
On FOD 3.4.428, `test_readdir_snapshot_multimount.py` also passed on Linux
6.17.0-41/libfuse3 3.18.2 against a disposable PostgreSQL 16.15 database. It
verifies cross-mount snapshot continuation and rewind, plus exact `EMFILE` from
both `open()` and libc `opendir()` at the process descriptor limit.

Logical-task admission has multithreaded unit coverage for FIFO order, capacity
limits, queue progress as tasks finish, and balanced completion/failure metrics.
An additional regression confirms that a running file write and a queued
`copy_file_range` task share the same write admission limit while their
operation-class counters remain separate.
Additional regressions verify that unwinding a worker releases its admission
slot for a queued task and that dropping an unfinished observation records a
failure while balancing active-transaction and payload counters.

The per-handle write-state map treats a poisoned mutex as `EIO` across read,
write, flush, truncate, `getattr`, `FIONREAD`, `poll` and `copy_file_range` paths.
It no longer reports a missing buffer or silently drops state updates when the
lock is poisoned. `write()` takes its target and sibling buffers atomically and
restores their prior merged state if block preparation fails. A deterministic
unit test covers the `EIO` mapping. This is a process-local error boundary;
PostgreSQL remains the durable authority.

The mounted timestamp regression checks invalid nanoseconds in both `utimensat`
fields and verifies that each `UTIME_NOW`/`UTIME_OMIT` order changes only the
requested timestamp. `test_symlink.sh` also submits `.123456789` through
`utimens()` and verifies that `stat()` reports `.123457000`, documenting the
PostgreSQL `TIMESTAMP` microsecond precision used for persisted filesystem
times. The test passed on PostgreSQL 16.15 `BLCKSZ=32KiB`, Linux 6.17.0-41 and
libfuse3 3.18.3. The broader metadata-and-timestamps POSIX audit remains open.

Writable mounts use a zero FUSE attribute-cache TTL even if
`FOD_ATTR_TIMEOUT_SECONDS` requests a positive value. Linux computes an
`O_APPEND` write offset from its local inode size before sending the request to
FUSE; refreshing attributes before each writable path lookup prevents a stale
size on one mount from overwriting an append committed through another mount.
Read-only mounts keep the configured attribute TTL. This correctness rule can
increase metadata requests on writable mounts; the entry-name TTL remains a
separate setting.

## Correctness boundaries and unfinished work

[Architecture invariants](FOD_ARCHITECTURE_INVARIANTS.md) define the required
PostgreSQL authority for namespace, ownership, leases, fencing and inode
lifecycle. The [primary authority audit](FOD_PRIMARY_AUTHORITY_AUDIT.md)
records verified unlink/rmdir decisions and cross-type namespace creation
protection, transactional inode chmod/chown and sticky-bit authorization,
passing mounted regressions and further mutation-precondition review.
Its findings must be resolved and validated before claiming complete
primary-authoritative mutation coverage.
`readlink()` also validates the requested symlink inode against the current
primary-resolved path before returning a target; a stale inode cannot follow a
replacement symlink at the same name. The two-mount regression is
`test_stale_inode_readlink.py` (`make test-stale-inode-readlink`).
Pathname bytes that are valid on Linux, including trailing spaces and tabs, are
preserved by FUSE path normalization and PostgreSQL namespace lookup. The
mounted `test_path_trailing_slash.py` regression distinguishes names with and
without those characters, then checks reads, rename, unlink and children under
parent directories ending in a space or tab.

POSIX record locks use the database `file_id` verified from the open FUSE
handle. They remain attached to an unlinked inode if another mount creates a
replacement at the old path. The two-mount regression is
`file_locks_remain_bound_to_open_inode_after_path_replacement` in
`rust_fuse/tests/lock_backend_smoke.rs`.
`posix_record_locks_are_not_inherited_across_fork` checks that a child process
inherits the descriptor but `F_GETLK` still reports the parent's process-owned
record lock. `posix_record_locks_survive_exec_with_open_fd` checks that a
separate process still sees the lock after its owner execs while preserving the
open descriptor.

`test_lseek.py` also checks the `RLIMIT_FSIZE` write boundary on a mounted FOD
filesystem. An oversized write stops at the configured limit and raises
`SIGXFSZ`. Later `write()`/`pwrite()` and over-limit `truncate()`/`ftruncate()`
fail with `EFBIG`; the written payload remains intact and failed truncations
leave their files empty. POSIX interruption and I/O-error cases remain under
review in the conformance audit.

PostgreSQL schema 30 includes transactional per-file content generations and
the cross-mount POSIX lock wait graph.
These primitives and the local cache publication gate do not establish default
cross-mount cache coherence. The [TX03J generation audit](tx03j-generation-audit.md)
tracks missing integration; strict opt-in reads are diagnostic coverage, not
proof that ordinary cached reads satisfy the same contract.

Explicit `FOD_DEGRADED_READONLY_ENABLED=1` mounts can read a configured physical
standby without registering primary-backed session/open leases. They force
read-only operation, bypass content caches and use one generation-bearing SQL
snapshot per read. They may expose stale replayed data and return `ESTALE`
after remote reclamation. Normal read-only mounts retain their authoritative
lease requirement. Background role monitoring reports promotion candidates;
live operator transition and source switching are unimplemented, so recovery
requires controlled unmount/remount. See the
[degraded contract review](FOD_DEGRADED_READONLY_CONTRACT_REVIEW.md).

The single ordered queue is [plans/CURRENT.md](plans/CURRENT.md); the
[HA/cache workstream](plans/HA_READONLY_RECOVERY_AND_CACHE.md) contains its
recovery acceptance matrix and diagnostic benchmark evidence.

## Production Docker topology

The supported reference topology is:

```text
MASTERS=1
SLAVES=0..32
1 persistent FOD/FUSE client
```

`MASTERS>1` is rejected.

Default production PostgreSQL image:

```text
ghcr.io/stachwk/postgres-16-fod-32k:16.15
```

Additional supported 32K image families:

```text
ghcr.io/stachwk/postgres-17-fod-32k:17.11
ghcr.io/stachwk/postgres-18-fod-32k:18.6
```

PG16 remains the reference deployment default.

By default the FOD client uses the exact release from `fod_version.txt`. `FOD_CLIENT_VERSION=X.Y.Z` may select another already-published client build without changing the repository/source version. A fully-qualified `FOD_DOCKER_DEPLOY_CLIENT_IMAGE=registry/path:tag` override has the highest deployment priority. The mutable `:3.4` tag remains only a convenience alias.

Image build/publish remains source-version based by default. Selecting `FOD_CLIENT_VERSION` for deployment does not retag binaries built from the current checkout as an older release.

## Storage payload model

The production payload model is **block-only**. Durable file payload is canonical in `fod.data_blocks`; the former extent runtime path was retired after migration of legacy extent data back to blocks.

The default logical FOD storage block for **newly initialized** filesystems is 32 KiB. The storage block size is persisted per filesystem in `fod.config` as `block_size`; existing filesystems keep the value with which they were initialized, including historical 4 KiB filesystems. Changing an existing filesystem to another block size requires an explicit data-format migration/rewrite and is not a runtime tuning operation.

Historical extent-engine plans, measurements and migration records are retained under [`history/`](history/) as evidence, but they are not alternate current runtime paths and must not be used to infer present storage behavior.

A future payload-format change requires an explicit storage-format/compatibility decision and migration plan; it must not be introduced as an implicit performance shortcut.

## I/O size layers

These values describe different layers and must not be conflated:

| Layer | Current default/reference | Meaning |
| --- | ---: | --- |
| FOD storage block | 32 KiB for newly initialized filesystems | logical block persisted per filesystem in `fod.config`; existing filesystems retain their recorded value |
| FUSE max write | 1 MiB | request ceiling exposed/configured for FUSE writes |
| FUSE max readahead | 512 KiB | readahead ceiling |
| base persist chunk | 128 FOD blocks = 4 MiB at the 32 KiB default | normal PostgreSQL payload batching unit; byte size scales with the filesystem's persisted storage block size |
| PostgreSQL server block | 32 KiB | `BLCKSZ` of the reference PostgreSQL Docker image |

The reference image is built from the pinned PostgreSQL source in
[`docker/postgres-blocksize/Dockerfile`](../docker/postgres-blocksize/Dockerfile)
with `--with-blocksize=32`. The publisher
[`scripts/publish_postgres_fod_32k.sh`](../scripts/publish_postgres_fod_32k.sh)
verifies both compiled `pg_config --configure` and runtime `SHOW block_size`;
the live test cluster confirmed `32768` bytes. Use this build and verify its
actual page size when reviewing PostgreSQL index or tuple limits. The standard
8 KiB image is only the compatibility/comparison variant. More build details:
[`FOD_POSTGRES_32K_CONTAINER.md`](FOD_POSTGRES_32K_CONTAINER.md).

Increasing a FUSE request size does **not** change the on-database FOD storage format. A 32 KiB default applies only when a new filesystem is initialized without an explicit `--block-size`; an existing filesystem continues to use its persisted `block_size`.

The new-filesystem storage default is owned by `fod-rust-mkfs`; current runtime/FUSE defaults are defined by `../fod_config.ini` and `../fod_config.example.ini`.

## Machine-readable compatibility/status diagnostics

`fod-rust-mkfs status --json` is the versioned machine-readable source for
PostgreSQL/libpq runtime compatibility, PostgreSQL runtime requirements, FOD
schema readiness and persisted storage-format settings. Its top-level
`schema_version` is independent from the FOD database schema version.

The JSON source intentionally does not infer negotiated FUSE state. FUSE
capabilities and effective request limits exist only after a mount negotiates
with the kernel.

Mounted clients publish that negotiated state through
`SharedMonitorSessionStats` schema version 2 in the optional
`fuse_compatibility` object. The shared publisher starts before FUSE
negotiation, so an early sample may correctly contain
`"fuse_compatibility": null`; after `Filesystem::init` succeeds, subsequent
samples expose the negotiated kernel/userspace protocol versions, requested,
enabled and unsupported capabilities, effective I/O request limits and the
kernel-derived request ceiling. Older schema-version-1 payloads remain readable
with the field absent/defaulted.

`fod-monitor report --json` schema version 3 aggregates these two versioned
sources and adds a concise `compatibility_summary` without replacing the raw
source data:

- `mkfs_status` contains the raw machine-readable `fod-rust-mkfs status --json`
  payload;
- `cluster.sessions[].stats` retains the shared monitor payload, including
  optional negotiated FUSE compatibility;
- `compatibility_summary.postgresql` exposes only directly observed PostgreSQL,
  schema and persisted-block-size facts;
- `compatibility_summary.fuse` reports active/telemetry/negotiated session
  counts plus observed shared-monitor schema versions, fuser versions,
  kernel/negotiated protocols and the union of unsupported requested
  capabilities;
- `compatibility_summary.coverage` is `complete`, `partial` or `unavailable`
  and describes only diagnostic-data coverage, not a compatibility pass/fail
  verdict;
- `mkfs_status_error` and `cluster_error` remain independent source errors.

Missing source data is never synthesized. If the mkfs/status source is
unavailable its summary fields remain `null`; if cluster telemetry is
unavailable the FUSE source status is `unavailable` with no fabricated
protocol/capability values. The raw report remains usable when one source
fails. `cluster --json` retains schema version 1, shared monitor telemetry
retains schema version 2, and `fod-rust-mkfs status --json` retains schema
version 1.

## PostgreSQL deployment behavior

The reference Docker deployment uses one writable primary and zero or more streaming replicas. Smoke validation requires:

- primary `SHOW block_size = 32768`,
- primary not in recovery,
- every requested replica in recovery,
- at least the requested number of streaming replication connections.

### Repository QNAP PostgreSQL preset

`QNAP=1` selects a persistent server-tuning profile intended for the current
reference QNAP host:

```text
8 GB RAM
2 CPU
HDD
PostgreSQL 16.15
server BLCKSZ = 32 KiB
```

The repository preset is:

```text
shared_buffers                  = 1GB
effective_cache_size            = 4GB
work_mem                        = 4MB
maintenance_work_mem            = 256MB
max_connections                 = 64
random_page_cost                = 4
effective_io_concurrency        = 1
max_wal_size                    = 2GB
checkpoint_timeout              = 15min
checkpoint_completion_target    = 0.9
wal_compression                 = off
autovacuum_max_workers          = 2
autovacuum_work_mem             = 128MB
max_parallel_workers            = 2
max_parallel_workers_per_gather = 1
```

`pg_stat_statements` remains the default shared preload library.

The resolved preset can be inspected without changing the server:

```bash
QNAP=1 make postgres-qnap-config-show
```

The QNAP profile is supplied to PostgreSQL through the container command line,
so it survives recreation of the PostgreSQL volume. Explicit `POSTGRES_*=...`
values supplied on the `make` command line may override the preset for
controlled tests/benchmarks. Ambient shell `POSTGRES_*` values are not treated
as QNAP defaults.

`QNAP=0` keeps the normal local PostgreSQL defaults unless tuning is supplied
explicitly.

The 2026-09-08 live validation confirmed all 15 profile parameters with
`source = command line`; the earlier `ALTER SYSTEM` copies were removed and
`postgresql.auto.conf` contained no remaining profile entries.

## FOD/FUSE deployment behavior

The FOD client container requires `/dev/fuse`, `CAP_SYS_ADMIN`, `rshared` bind propagation and compatible host AppArmor policy.

A host can legitimately show more than one propagated FUSE row for the same mount. Health is determined by one unique FUSE `MAJ:MIN` identity shared by the host and the FOD container, not by requiring exactly one `findmnt` row.

### External unmount/session teardown

On the current `fuser 0.18.0` / libfuse3 stack an external
`fusermount3 -u` cleanly removes the mount and lets the FOD process exit with
status 0. The historical `fuser 0.17` post-unmount
`Failed to umount filesystem: Invalid argument` warning is not reproduced by
the current validation.

PostgreSQL client-session rows remain lease based. A process exit does not
require immediate deletion of its `fod.client_sessions` row; after heartbeat
stops, `lease_expires_at` makes the row stale and normal active-session
maintenance prunes expired rows. This preserves crash/failure semantics and is
separate from the FUSE unmount lifecycle.

## Mounted fallocate semantics

FOD 3.4.28 implements one explicit Linux `fallocate` mutation contract:
`FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE`.

The operation preserves logical file size, maps fully punched canonical blocks
to sparse/missing `fod.data_blocks`, zeroes only the punched bytes in partial
boundary blocks, keeps hardlink/data-object semantics, and runs under the same
PostgreSQL write-ownership/fencing and quota/accounting boundaries as ordinary
persisted writes. Read/recent-write/metadata/statfs caches are invalidated after
success.

Plain `mode=0`, `FALLOC_FL_KEEP_SIZE`, `FALLOC_FL_ZERO_RANGE` and unsupported
flag combinations remain intentionally unsupported. The block-only format keeps
all-zero blocks sparse, so it has no durable state distinguishing an allocated
zero/unwritten range from a hole. A libc `posix_fallocate()` fallback success is
therefore not a FOD preallocation guarantee.

## Mounted sparse seek status

FOD 3.4.28 does not yet implement an explicit sparse-aware
`lseek(SEEK_DATA/SEEK_HOLE)` callback.

The mounted S1.1 baseline confirmed that fuser's default callback returns
`ENOSYS`, after which Linux uses a generic fallback for the mount. Under that
fallback all offsets below EOF are effectively treated as data and the only
reported hole is the implicit one at EOF. Consequently canonical missing
`fod.data_blocks` created by sparse writes or `PUNCH_HOLE|KEEP_SIZE` are not
currently discoverable through `SEEK_DATA`/`SEEK_HOLE`.

Offsets at or beyond EOF return `ENXIO`, including empty-file offset 0. The
baseline probes are non-mutating.

FOD 3.4.29 implements the selected S1.2 contract explicitly. Sparse seek is
block-granular: a missing canonical storage block is a hole and a present
canonical block is data. Zero bytes inside a present partial block are not a
hole. The callback flushes pending same-mount writes before consulting
PostgreSQL and uses indexed block-order lookups without reading payload bytes.
Offsets at or beyond EOF return `ENXIO`.

The FOD 3.4.29 implementation is validated across dense and sparse layouts,
partial-block punching, multi-digit block orders, pending writes, remount, and
F1.2 punch-hole regression. PostgreSQL plans use
`idx_data_blocks_object_order` for both sparse-seek queries without a text
sort. Release-lto ASM review confirmed the numeric SQL literals and absence of
`_order::text`; release reproducibility and package payload integrity gates
also pass.

## Database schema and primary-authoritative mutations

Current PostgreSQL schema version is 34; the migration manifest in
`rust_mkfs/src/main.rs` is authoritative. Earlier schema numbers below identify
the migrations that introduced particular contracts.

The `schema_version.minimum_client_version` value is a monotonic FOD release
floor written by `mkfs.fod init` and `mkfs.fod upgrade`. Primary mounts read it
from the primary. Replica mounts check the current floor and upgrade marker on
the writable primary at startup while keeping file reads on the standby; they
fail closed if the primary is unavailable. Explicit degraded read-only mounts
may run without primary and use the standby's last replayed floor until WAL
catches up. Replica mounts require role-aware primary/replica endpoints. Their
local schema version is checked separately to reject a standby that has not
replayed the required migration.
`test_client_version_gate_replica.py` verifies rejection from both endpoints,
including while standby WAL replay is paused and its local client floor remains
old; it also covers primary unavailability in normal and degraded read-only
modes.
The migration also renames the schema-number column, so pre-gate clients that
query its old name fail startup instead of silently skipping the check. A later
release may mount without raising the floor; the floor moves only when an
operator runs the schema command with that release.
See the [versioning ADR](adr/storage-format-versioning.md).

The schema-admission gate first shipped in FOD 3.4.443 with schema version 34;
FOD 3.4.465 currently requires schema version 38. Migrations 36 and 37
normalize legacy POSIX modes and widen the entity-ID graph; migration 38 adds
the shared-symlink alias table. Migration 37 changes column and index types,
so run production upgrades only in a maintenance window.
Before applying migrations, `mkfs.fod upgrade` raises the database client floor
and sets `schema_version.upgrade_in_progress` transactionally. Mount admission checks
that marker; client-session registration uses a shared PostgreSQL advisory
lock, while upgrades hold its exclusive counterpart and refuse to proceed when
unexpired FOD sessions exist. A PostgreSQL trigger also rejects legacy client
session inserts while the gate is set. The final schema number, release floor,
and cleared marker publish in one transaction. A physical standby below schema
38 rejects the new client until WAL replay catches up. Migration 38 adds the
PostgreSQL table for additional directory entries that share a symlink inode.
Replica startup reads the client floor and upgrade marker from the writable
primary, so WAL lag cannot
admit an outdated client. Client-session registration also rechecks both gates
under the shared advisory lock, closing the race between the startup snapshot
and session creation. Interrupted upgrades leave the marker set so operators can
rerun `mkfs.fod upgrade`; they must unmount all FOD hosts before starting an
upgrade.

PostgreSQL file-mode lookups return `None` only when the path has no matching
row. NULL, empty, non-octal, malformed, or out-of-range modes (above `0o7777`)
fail as metadata errors instead of being presented as a missing path or
projected with file-type bits as permissions. FUSE validates path and open-file
attribute modes against the same bound. PostgreSQL uses one POSIX-mode decoder
for namespace lookups, mutation authorization, ACL/xattr checks and directory
permission checks, so these paths reject corrupt mode rows consistently.
The FUSE `CREATE` callback removes file-type bits from the mode supplied by the
kernel before persisting permissions; the filesystem object kind is stored
separately. This keeps regular-file mode values within the POSIX `0o7777`
permission range.
Broader fault-injection coverage for malformed persisted rows remains in the
[unresolved findings](../TODO.md#unresolved-findings).
Namespace lookup and directory-entry metadata decoders reject entity ID zero;
FOD represents the logical root separately and does not assign it an entity
row ID of zero. Single-entry child lookup also rejects parent `Some(0)` so it
cannot be interpreted as the root's SQL NULL-parent sentinel.

Primary authority requirements are defined by
[FOD_ARCHITECTURE_INVARIANTS.md](FOD_ARCHITECTURE_INVARIANTS.md). A dated audit
reported one reproduced hardlink/unlink data-loss case and additional mutation
review items; the implementation queue tracks their revalidation and closure.
Do not treat an older audit's test result as proof of current behavior.

Mutation input is read from primary: partial writes and truncate boundaries,
copy planning and copy source/destination blocks, and the buffered-write
unchanged-slice comparison. A stale replica match must never suppress a real
write or supply old bytes to a copy. The mounted primary/replica regression
pauses replay, changes the source on primary, then verifies both an overwrite
and a partial `copy_file_range` through another mount directly in PostgreSQL.
Truncate-boundary planning also obtains the previous EOF from primary. A
separate regression pauses standby replay, extends a file on primary, then
extends it through a replica-routed mount and checks both visible contents and
the primary payload row. It passed on 2026-10-30. The broader
mutation-authority audit remains active.
Create/open rollback cleanup now attempts hardlink promotion on the primary
before purging. A no-promotion result falls through to primary purge, which
rechecks surviving links and active leases under the file-lifecycle lock; a
replica-routed link count no longer selects the cleanup operation.

## POSIX byte-range locks

Writable primary mounts store record-lock ranges and blocking wait-for edges in
PostgreSQL. The wait graph uses `(host_name, lock_pid)` process identities,
because FUSE `lock_owner` values are local to one mount connection. `F_SETLKW`
registers active conflicting processes and checks for a path back to the
waiting process while holding the shared wait-graph transaction lock. A
detected cycle returns `EDEADLK`; lock-range updates use
the same graph-before-resource lock order and invalidate incoming wait edges so
waiters rebuild them as ranges change. This gives deadlock detection one
ordering authority across processes and mounts. The graph lock serializes
PostgreSQL-backed range-lock changes and needs workload measurement before any
optimization. FIFO lock operations remain local Linux FUSE behavior and are
outside FOD's cross-mount lock guarantee. See the [POSIX audit](POSIX_CONFORMANCE_AUDIT.md#pending-requirement-ledger)
for remaining `fork()`/`exec()` and lock-limit checks.

## Cross-mount write ownership and FUSE hang safety

Writable FOD mounts use PostgreSQL-authoritative write ownership introduced with
schema version 24. The schema contains `fod.destination_write_leases` and
`fod.file_write_leases`.

Schema version 25 extends the same authority rule to open-unlink lifetime.
`fod.files.unlinked` records a namespace-removed primary generation and
`fod.file_open_leases` records open handles by client session. An unlinked file
is reclaimed only when PostgreSQL sees no active open lease joined to an active
client session. A recreated pathname uses a distinct file/inode generation.

Crash convergence has mounted coverage for a single crashed holder with forced
and natural TTL expiry, a surviving second holder, staggered crashes of two
holders, and near-simultaneous crashes of both holders. In every validated
case, another live mount's maintenance prunes expired sessions and purges the
old generation only after the last authoritative lease is inactive; the
replacement remains isolated.

The runtime contract is:

- destination/file ownership is first-writer-wins and acquired with
  non-blocking PostgreSQL try-lock coordination;
- writable `create` reserves destination ownership before namespace mutation,
  then rechecks the namespace while that reservation is held;
- concurrent writable creates of the same initially absent destination
  therefore produce exactly one namespace creator; the competing creator fails
  promptly with `EBUSY` instead of mutating and rolling back the winner;
- the creator upgrades the same owner key from destination-only ownership to
  destination+file ownership, and namespace rollback is allowed only when that
  request actually created the file;
- if the post-reservation recheck finds an existing file, non-`O_EXCL` create
  attaches to that file identity and preserves `O_TRUNC` atomic-truncate
  semantics; `O_EXCL` still returns `EEXIST`;
- a competing writer fails promptly with `EBUSY` instead of waiting;
- active ownership is renewed together with client-session heartbeat activity;
- lease expiry and heartbeat decisions use PostgreSQL `clock_timestamp()` as the
  authoritative clock;
- monotonic fencing tokens protect persistence after lease loss or takeover;
- stale writers are rejected before PostgreSQL payload persistence;
- a competing pathname `truncate()` on mount B returns `EBUSY` while mount A
  writes under its valid lease; the size and payload match mount A's completed
  4096-byte write, and lease owner/token remain unchanged. Verified on FOD
  3.4.354 by racing `pwrite()` and path `truncate()` across two mounts
  against an isolated PostgreSQL database on 2026-10-30;
- path-based `truncate()` checks write/search permissions through FOD's mode and POSIX ACL authorization path; mounted tests verify exact `EACCES` for a non-writable file, a non-searchable parent and a denying ACL, while preserving payload and size;
- writable `open`, `O_TRUNC`, `create` and path-based `truncate`/`setattr`
  participate in the same ownership/fencing contract;
- writable mounts require `FUSE_ATOMIC_O_TRUNC`, `FUSE_HANDLE_KILLPRIV_V2` and
  `FUSE_SETXATTR_EXT`; startup rejects a kernel missing any of these semantics.
  Write, truncate and `O_TRUNC` pass Linux killpriv flags; the PostgreSQL
  content transaction clears set-ID bits and any remaining
  `security.capability`. Linux can first issue an internal
  `REMOVEXATTR security.capability` before the content request. FOD permits this
  exact privilege-removal side effect from the writer UID; explicit
  unprivileged `removexattr()` remains rejected by Linux before FUSE. When this
  path occurs, attribute removal and content persistence are separate,
  PostgreSQL-serialized transactions in Linux VFS order. See
  [`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md).

All FOD hosts and PostgreSQL nodes should keep system time synchronized with
`chrony` or an equivalent NTP implementation. Correct lease ordering does not
depend on client wall-clock timestamps because PostgreSQL server time is
authoritative.

`fod-bootstrap` also protects mounted FUSE instances from a stuck `statfs`
request. By default it probes every 5 seconds and treats a probe lasting 15
seconds as a hung frontend. When possible it aborts the matching fusectl
connection, terminates the frontend and lazily detaches the aborted mount.
`FOD_FUSE_HANG_GUARD_SECONDS=0` disables this guard; the interval can be
overridden with `FOD_FUSE_HANG_GUARD_INTERVAL_SECONDS`.

## systemd lifecycle

### First start / inactive service

```text
systemctl start
  -> host-prepare
  -> docker_deploy.sh up
  -> FOD start/reconcile
  -> PostgreSQL smoke
  -> FOD/FUSE smoke
```

### Reinstall / FOD upgrade while service is active

```text
systemctl reload
  -> boot.sh start
  -> reconcile existing PostgreSQL topology
  -> reconcile exact FOD image
  -> smoke checks
```

This path does not execute `ExecStop`. Healthy PostgreSQL primary/replica containers are therefore preserved during an FOD client reinstall/upgrade.

The image reconciliation guard compares the running FOD container image ID with the required exact image ID. If they differ, only the FOD container/mount is recreated.

For systemd installs the selected client version is resolved to a full image name and persisted as `FOD_DOCKER_DEPLOY_CLIENT_IMAGE` in `/etc/fod/docker-deploy.env`. Later reboots therefore keep using that selected image even if the Git checkout advances to another FOD release.

The behavior was live-validated during the `3.4.4 -> 3.4.5` upgrade: the FOD container changed to the new exact image while primary and both replica container IDs remained unchanged.

### Explicit full restart

`make docker-deploy-systemd-restart` intentionally performs a full service restart. FOD is stopped first and the PostgreSQL Compose containers are then stopped/recreated while persistent volumes are retained.

## Data safety boundaries

Normal `down`, reinstall and systemd lifecycle operations preserve PostgreSQL volumes and generated deployment state.

Destructive volume removal is a separate guarded operation:

```bash
make docker-deploy-destroy MASTERS=1 SLAVES=2 DESTROY=YES
```

## Validation entry points

```bash
make test-version
make test-cargo-lock-integrity
make test-docker-deploy-policy
make test-docker-fod-install-policy
make test-docker-deploy-systemd-policy
make test-docker-fod-client-policy
```

Operational smoke:

```bash
sudo make docker-deploy-systemd-smoke MASTERS=1 SLAVES=2
```

For task-oriented navigation see [`README.md`](README.md).
