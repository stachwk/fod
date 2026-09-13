# FOD Roadmap

## Current Status

- The current FOD release is sourced from `fod_version.txt`; this roadmap intentionally avoids duplicating the latest patch number as an authoritative version source.
- FOD has a working PostgreSQL-backed Rust FUSE core, a versioned PostgreSQL schema managed by the Rust mkfs migration manifest, documented runtime profiles, a shared Rust indexing core, and a broad local integration suite.
- The current database schema is version `24`. Machine-readable compatibility diagnostics use independent schema versions: `fod-rust-mkfs status --json` schema 1, `fod-monitor cluster --json` schema 1, `SharedMonitorSessionStats` schema 2, and `fod-monitor report --json` schema 3.
- The repository currently has no active GitHub Actions workflow. `make test-all` is the main local regression gate, while `make test-all-full` adds wider mounted and indexer coverage.
- Benchmark baselines are tracked in [`BENCHMARKS.md`](BENCHMARKS.md). [`TODO.md`](TODO.md) is a mixed archive/follow-up record; the maintained implementation sequence is [`docs/plans/CURRENT.md`](docs/plans/CURRENT.md).
- SELinux mount-label policy is a deliberate non-goal. Rocky Linux 10.2 support is defined as operational SELinux enforcement through the host FUSE `fusefs_t` label and normal domain policy; per-inode `security.selinux` labeling depends on host/mount-stack support.
- Schema init, upgrade, and clean operations are non-destructive by default on existing databases and are protected by the schema-admin secret flow.
- The runtime is Rust-backed end to end: mount entrypoints, namespace operations, metadata, permissions, locking, payload storage, schema tooling, monitoring, and indexing no longer depend on the removed Python runtime.
- PostgreSQL-backed advisory locking is the production path for `flock`, `fcntl` range locks, payload-quota admission, copy-capacity reservations, cross-mount destination/write ownership, and stale-writer fencing.
- `max_fs_size_bytes` is the canonical transactional payload quota and the `statfs` capacity ceiling. Persisted payload and active reservations determine filesystem-wide used and available space.
- The broad role-aware PostgreSQL routing project is complete: startup role selection, runtime primary failover, WAL-gated replica reads, adaptive replica scoring, promotion validation, and process-local generation fencing are implemented.
- P1-P4 are closed: cross-mount write ownership/fencing, QNAP COPY-buffer repeatability, external-unmount teardown validation, and compatibility-diagnostics aggregation all have archived evidence under `docs/history/`.

## Completed Foundation

- PostgreSQL-backed Rust FUSE filesystem core
- block-range reads with cache and read-ahead
- buffered writes with dirty tracking and chunked persistence
- xattr and ACL support
- PostgreSQL-backed advisory locking and session leases
- runtime tunables in `fod_config.ini`
- safe schema init, repair, status, and migration handling through schema version `24`
- Rust-backed repository and query layers
- split attribute and directory-entry caches
- shared Rust `fod-indexer` core with capability-driven source kinds
- bounded transactional replay with durable outcome confirmation
- transactional block payload quota under a shared PostgreSQL advisory lock
- crash-recoverable copy-capacity reservations with renewal before persistence
- canonical `statfs` accounting for payload, reservations, capacity, and inode headroom
- mounted `df`/`du`/sparse/shared-object regression before and after remount
- forced concurrent two-mount quota regression
- PostgreSQL-authoritative cross-mount destination/write ownership with fencing tokens and stale-writer rejection
- explicit writable-mount requirement for `FUSE_ATOMIC_O_TRUNC`
- role-aware PostgreSQL endpoint selection, failover, WAL-gated replica reads, scoring, and promotion validation
- validated external `fusermount3 -u` teardown on the current fuser/libfuse3 stack without the historical benign `EINVAL`
- versioned PostgreSQL/libpq/schema/storage diagnostics through `fod-rust-mkfs status --json`
- versioned negotiated FUSE telemetry through shared monitor schema 2
- aggregated `fod-monitor report --json` schema 3 with coverage-only compatibility summary and explicit missing-source states
- explicit ADR for storage-format versioning

## Near Term

- Audit the mounted Rust `fallocate` contract before adding behavior. The current Rust FUSE frontend does not expose a repository-visible `fallocate` implementation, while older TODO/archive text still contains legacy claims from earlier runtime stages. First establish the actual kernel/fuser/runtime syscall behavior and define exactly which modes FOD can support safely.
- If the `fuser 0.18.0` callback surface supports the required operation, implement only explicitly defined `fallocate` modes. Preserve write ownership/fencing, payload quota, sparse accounting, cache/statfs invalidation, timestamps, hardlink/data-object semantics, and remount durability; reject unsupported mode combinations with `EOPNOTSUPP`.
- Keep the repository QNAP PostgreSQL preset stable for the current 8 GB / 2 CPU / HDD reference host. P2 did not justify changing the current `FOD_PERSIST_COPY_SEND_BUFFER_BYTES` default; repeat the matrix only after a new measured regression or materially changed environment.
- Treat the FOD 3.4.16-3.4.20 read-path optimization sequence as closed. Reopen metadata/range-cache tuning only for a new measured regression.
- Treat external-unmount/session teardown as closed on the validated current stack. Reopen only if a future fuser/libfuse3 version reproduces a correctness or warning regression.
- Treat compatibility-diagnostics aggregation as closed. Extend source fields only when a concrete consumer needs additional trustworthy machine-readable data; do not invent a new compatibility subsystem.
- Instrument inode/path cache lifetime and implement `forget` plus `batch_forget` only if large-tree measurements show retained-state pressure that the current cache/path model does not bound adequately.
- Benchmark `readdirplus` against `readdir` for large directories and keep it only when it measurably reduces callbacks or PostgreSQL work without weakening cache correctness.
- Define sparse-file edge cases before adding mounted `lseek(SEEK_DATA/SEEK_HOLE)` semantics; existing generic seek/helper behavior must not be mistaken for sparse-aware mounted support.
- Keep local quality gates, benchmark baselines, current documentation, and authoritative version/schema metadata synchronized with code changes.

## Medium Term

- Harden the existing role-aware multi-endpoint runtime only through explicit measured gaps; do not reopen already delivered startup routing, WAL-gated replica-read, scoring, failover, or promotion-guard stages as if they were unimplemented.
- Keep external/cross-process fencing and fairness separate from process-local generation fencing; add either only with a concrete ownership model, failure scenario, and acceptance test.
- Keep multiple writable endpoints limited to HA/proxy entrypoints for the same authoritative PostgreSQL cluster unless an explicitly selected database technology provides real multi-primary conflict handling.
- Keep expanding `fod-rust-monitor` as the shared home for process, PostgreSQL, lane, queue, throughput, and compatibility diagnostics instead of growing `pg_lanes.rs` or `pg.rs` further.
- Strengthen production-style fault tests for reconnect, promotion, lag, lock/session safety, write-ownership fencing, and replay confirmation without presenting independent primaries as a safe multi-primary filesystem.
- Continue performance work only from measured SQL, WAL, connection, memory, or FUSE callback evidence.
- Keep backup and restore aligned with PostgreSQL operational practices rather than creating a parallel FOD-specific backup format.
- Split oversized Rust modules incrementally when behavior-preserving moves improve reviewability and test isolation.
- Add dependency and security monitoring when it can be introduced without weakening reproducible builds or the one-version-per-commit rule.

## Non-Goals for Now

- full SELinux mount-label policy or host-side FUSE promotion to `fs_use_xattr`
- general-purpose execution semantics for special device nodes beyond stored metadata
- replacing PostgreSQL backup and restore with a custom FOD backup subsystem
- enabling FUSE passthrough without a real backing file descriptor and a coherent PostgreSQL/storage model
- claiming native `SYNCFS`, `TMPFILE`, or `STATX` support before the public `fuser` API and the corresponding FOD contracts exist
- presenting independent writable PostgreSQL primaries as one safe multi-primary FOD filesystem
