# FOD Roadmap

## Current Status

- The current FOD release is sourced from `fod_version.txt`; this roadmap
  intentionally avoids duplicating the latest patch number as an authoritative
  version source.
- FOD has a working PostgreSQL-backed FUSE core, a versioned PostgreSQL schema
  managed by the Rust mkfs migration manifest, documented runtime profiles,
  a shared Rust indexing core, and a broad local integration suite.
- The repository currently has no active GitHub Actions workflow. `make test-all` is the main local regression gate, while `make test-all-full` adds wider mounted and indexer coverage.
- Benchmark baselines are tracked in [`BENCHMARKS.md`](BENCHMARKS.md), while [`TODO.md`](TODO.md) records open follow-ups, accepted decisions, completed work, and regression notes.
- The maintained next implementation sequence is tracked in [`docs/plans/CURRENT.md`](docs/plans/CURRENT.md).
- SELinux mount-label policy is a deliberate non-goal. Rocky Linux 10.2 support is defined as operational SELinux enforcement through the host FUSE `fusefs_t` label and normal domain policy; per-inode `security.selinux` labeling depends on host/mount-stack support.
- Schema init, upgrade, and clean operations are non-destructive by default on existing databases and are protected by the schema-admin secret flow.
- The runtime is Rust-backed end to end: mount entrypoints, namespace operations, metadata, permissions, locking, payload storage, schema tooling, and indexing no longer depend on the removed Python runtime.
- PostgreSQL-backed advisory locking is the production path for `flock`, `fcntl` range locks, payload-quota admission, and copy-capacity reservations across independent mounts.
- `max_fs_size_bytes` is the canonical transactional payload quota and the `statfs` capacity ceiling. Persisted payload and active reservations determine filesystem-wide used and available space.
- The mounted accounting regression now verifies logical size, attributed per-file allocation, sparse ranges, shared data objects, persisted payload, active reservations, and remount stability against PostgreSQL.
- The dedicated two-mount quota regression forces both writers behind the shared advisory lock and verifies one commit, one `ENOSPC`, no rejected payload, and no leaked reservation state.
- The long-term architecture remains explicit: Rust owns the runtime and hot-path engine, PostgreSQL owns durable shared state, and documentation/test harnesses remain outside the runtime surface.

## Completed Foundation

- PostgreSQL-backed FUSE filesystem core
- block-range reads with cache and read-ahead
- buffered writes with dirty tracking and chunked persistence
- xattr and ACL support
- PostgreSQL-backed advisory locking and session leases
- runtime tunables in `fod_config.ini`
- safe schema init, repair, status, and migration handling through schema version `22`
- Rust-backed repository and query layers
- split attribute and directory-entry caches
- shared Rust `fod-indexer` core with capability-driven source kinds
- bounded transactional replay with durable outcome confirmation
- transactional block payload quota under a shared PostgreSQL advisory lock
- crash-recoverable copy-capacity reservations with renewal before persistence
- canonical `statfs` accounting for payload, reservations, capacity, and inode headroom
- mounted `df`/`du`/sparse/shared-object regression before and after remount
- forced concurrent two-mount quota regression
- explicit ADR for storage-format versioning
- role-aware PostgreSQL endpoint selection without list-position semantics
- runtime primary failover for HA/proxy entrypoints representing one authoritative cluster
- WAL-gated replica reads with primary fallback
- replica health/latency scoring and circuit-breaker cooldown
- fail-closed primary promotion validation and process-local primary-generation fencing

## Near Term

- make PostgreSQL-authoritative destination-path serialization across
  independent mounts the next correctness priority; concurrent writers must
  never interleave one logical destination
- treat the FOD 3.4.16-3.4.20 read-path optimization sequence as closed; do not
  reopen metadata/range-cache tuning without a new measured regression
- keep the repository QNAP PostgreSQL preset stable for the current
  8 GB / 2 CPU / HDD reference host; repeat the COPY send-buffer matrix only
  before considering a default change
- keep multi-endpoint work limited to explicit hardening gaps; startup routing,
  primary failover, WAL-gated replica reads, scoring and promotion validation
  are already delivered
- aggregate compatibility diagnostics only when the underlying FUSE,
  PostgreSQL, libpq, runtime and storage-format signals remain trustworthy
- reproduce the external-unmount/session-teardown warning on the current
  `fuser 0.18` / libfuse3 stack before changing teardown behavior
- instrument inode/path cache lifetime and implement `forget` plus
  `batch_forget` only if large-tree measurements justify it
- connect the existing resize and sparse-storage machinery to explicitly
  supported `fallocate` modes and reject unsupported combinations with
  `EOPNOTSUPP`
- benchmark `readdirplus` against `readdir` for large directories and keep it
  only when it measurably reduces callbacks or PostgreSQL work
- implement sparse-aware `lseek(SEEK_DATA/SEEK_HOLE)` only after edge-case tests
  define the contract
- keep local quality gates, benchmark baselines, current documentation and
  authoritative version/schema metadata synchronized with code changes

## Medium Term

- harden the existing role-aware multi-endpoint runtime only through explicit measured gaps; do not reopen the already delivered startup routing, WAL-gated replica-read, scoring, failover or promotion-guard stages as if they were unimplemented
- keep external/cross-process fencing and fairness separate from process-local generation fencing; add either only with a concrete ownership model, failure scenario and acceptance test
- keep multiple writable endpoints limited to HA/proxy entrypoints for the same authoritative PostgreSQL cluster unless an explicitly selected database technology provides real multi-primary conflict handling
- keep expanding `fod-rust-monitor` as the shared home for process, PostgreSQL, lane, queue, and throughput diagnostics instead of growing `pg_lanes.rs` or `pg.rs` further
- strengthen production-style fault tests for reconnect, promotion, lag, lock/session safety, and replay confirmation without presenting independent primaries as a safe multi-primary filesystem
- continue performance work only from measured SQL, WAL, connection, memory, or FUSE callback evidence
- keep backup and restore aligned with PostgreSQL operational practices rather than creating a parallel FOD-specific backup format
- split oversized Rust modules incrementally when behavior-preserving moves improve reviewability and test isolation
- add dependency and security monitoring when it can be introduced without weakening reproducible builds or the one-version-per-commit rule

## Non-Goals for Now

- full SELinux mount-label policy or host-side FUSE promotion to `fs_use_xattr`
- general-purpose execution semantics for special device nodes beyond stored metadata
- replacing PostgreSQL backup and restore with a custom FOD backup subsystem
- enabling FUSE passthrough without a real backing file descriptor and a coherent PostgreSQL/storage model
- claiming native `SYNCFS`, `TMPFILE`, or `STATX` support before the public `fuser` API and the corresponding FOD contracts exist
