# FOD Roadmap

## Current Status

- FOD `3.2.16` has a working PostgreSQL-backed FUSE core, schema version `18`, documented runtime profiles, a shared Rust indexing core, and a broad local integration suite.
- The repository currently has no active GitHub Actions workflow. `make test-all` is the main local regression gate, while `make test-all-full` adds wider mounted and indexer coverage.
- Benchmark baselines are tracked in [`BENCHMARKS.md`](BENCHMARKS.md), while [`TODO.md`](TODO.md) records open follow-ups, accepted decisions, completed work, and regression notes.
- SELinux mount-label policy is a deliberate non-goal; xattr-backed metadata and runtime gating are the supported path.
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
- safe schema init, repair, status, and migration handling through schema version `18`
- Rust-backed repository and query layers
- split attribute and directory-entry caches
- shared Rust `fod-indexer` core with capability-driven source kinds
- bounded transactional replay with durable outcome confirmation
- transactional block/extent payload quota under a shared PostgreSQL advisory lock
- crash-recoverable copy-capacity reservations with renewal before persistence
- canonical `statfs` accounting for payload, reservations, capacity, and inode headroom
- mounted `df`/`du`/sparse/shared-object regression before and after remount
- forced concurrent two-mount quota regression
- explicit ADR for storage-format versioning

## Near Term

- keep `make test-all`, `make test-all-full`, `cargo fmt --all -- --check`, `cargo check --workspace --locked`, and `make test-version` as the documented local quality gates
- introduce an active automated workflow only as an explicit project decision, with Rust 1.85 minimum-version coverage and locked dependency checks
- create a machine-readable FUSE callback and capability matrix covering: kernel protocol availability, public `fuser` API availability, FOD implementation status, semantic readiness, and assigned regression tests
- implement an explicit `fsync` durability contract by reusing the existing write-state persistence path and propagating PostgreSQL errors truthfully
- instrument inode/path cache lifetime and implement `forget` plus `batch_forget` if large-tree measurements confirm retained entries
- connect the existing resize and sparse-storage machinery to explicitly supported `fallocate` modes; reject unsupported mode combinations with `EOPNOTSUPP`
- benchmark `readdirplus` against `readdir` for large directories and keep it only when it measurably reduces callbacks or PostgreSQL work
- implement sparse-aware `lseek(SEEK_DATA/SEEK_HOLE)` over block and extent maps after edge-case tests define the contract
- keep post-ABI-7.31 features disabled until both the public `fuser` API and truthful FOD semantics exist; protocol negotiation alone is not an enablement decision
- validate supported `libfuse3` versions, especially external unmount/session teardown and `copy_file_range`, without assuming a libfuse upgrade adds missing high-level `fuser` callbacks
- keep benchmark baselines, decision notes, schema status, compatibility contracts, and runtime profiles synchronized with code changes

## Medium Term

- design role-aware multi-endpoint PostgreSQL routing with explicit primary and replica roles, operation-aware selection, and read-after-write consistency
- add endpoint health, latency, pool, replay-lag, and circuit-breaker diagnostics before enabling automatic replica routing
- strengthen production-style fault tests for reconnect, promotion, lag, lock/session safety, and replay confirmation without presenting independent primaries as a safe multi-primary filesystem
- continue performance work only from measured SQL, WAL, connection, memory, or FUSE callback evidence
- keep backup and restore aligned with PostgreSQL operational practices rather than creating a parallel FOD-specific backup format
- split oversized Rust modules incrementally when behavior-preserving moves improve reviewability and test isolation
- add dependency and security monitoring when it can be introduced without weakening reproducible builds or the one-version-per-commit rule

## Non-Goals for Now

- full SELinux mount-label policy
- general-purpose execution semantics for special device nodes beyond stored metadata
- replacing PostgreSQL backup and restore with a custom FOD backup subsystem
- enabling FUSE passthrough without a real backing file descriptor and a coherent PostgreSQL/storage model
- claiming native `SYNCFS`, `TMPFILE`, or `STATX` support before the public `fuser` API and the corresponding FOD contracts exist
