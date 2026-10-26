# FOD Roadmap

The single current implementation queue is [docs/plans/CURRENT.md](docs/plans/CURRENT.md).
This roadmap describes direction; it does not define a second execution order.
Current guarantees and limitations are recorded in [docs/CURRENT_STATE.md](docs/CURRENT_STATE.md).

## Current Status

- The current FOD release is sourced from `fod_version.txt`; this roadmap intentionally avoids duplicating the latest patch number as an authoritative version source.
- FOD has a working PostgreSQL-backed Rust FUSE core, a versioned PostgreSQL schema managed by the Rust mkfs migration manifest, documented runtime profiles, a shared Rust indexing core, and a broad local integration suite.
- The current database schema is version `27`. Machine-readable compatibility diagnostics use independent schema versions: `fod-rust-mkfs status --json` schema 1, `fod-monitor cluster --json` schema 1, `SharedMonitorSessionStats` schema 2, and `fod-monitor report --json` schema 3.
- The repository currently has no active GitHub Actions workflow. `make test-all` is the main local regression gate, while `make test-all-full` adds wider mounted and indexer coverage.
- Benchmark baselines are tracked in [`BENCHMARKS.md`](BENCHMARKS.md). [`TODO.md`](TODO.md) is a historical decision/follow-up archive; the only active implementation sequence is [`docs/plans/CURRENT.md`](docs/plans/CURRENT.md).
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
- safe schema init, repair, status, and migration handling through schema version `27`
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
- transactional PostgreSQL-authoritative rename/replace across file, hardlink, symlink and directory namespaces with active-writer fencing and rollback
- explicit writable-mount requirement for `FUSE_ATOMIC_O_TRUNC`
- role-aware PostgreSQL endpoint selection, failover, WAL-gated replica reads, scoring, and promotion validation
- validated external `fusermount3 -u` teardown on the current fuser/libfuse3 stack without the historical benign `EINVAL`
- versioned PostgreSQL/libpq/schema/storage diagnostics through `fod-rust-mkfs status --json`
- versioned negotiated FUSE telemetry through shared monitor schema 2
- aggregated `fod-monitor report --json` schema 3 with coverage-only compatibility summary and explicit missing-source states
- explicit ADR for storage-format versioning

## Completed milestones

Fallocate (`PUNCH_HOLE|KEEP_SIZE`), sparse `SEEK_DATA`/`SEEK_HOLE`,
open/unlink crash convergence, inode/path retention, batched readdir metadata,
same-destination create/rename ownership, external-unmount validation and P1–P4
are completed milestones. Their acceptance evidence and dates belong in
[`docs/CURRENT_STATE.md`](docs/CURRENT_STATE.md) and
[`docs/HISTORY.md`](docs/HISTORY.md), not in a second active checklist.

## Long-term direction

These themes guide future proposals; they are not scheduled work. Promote a
theme into the current queue only when a concrete correctness gap, deployment
need or reproducible measurement justifies it.

- Harden role-aware PostgreSQL runtime behavior only for explicit measured or
  correctness gaps. Delivered startup routing, WAL-gated replica reads,
  scoring, failover and promotion checks are summarized in the current state.
- Consider external or cross-process fencing only with a concrete ownership
  model, failure scenario and acceptance test.
- Keep multiple writable endpoints limited to entrypoints for one authoritative
  PostgreSQL cluster unless a database technology provides a proven global
  conflict order.
- Extend monitoring from a specific operational need; avoid growing database
  hot-path modules into general diagnostics frameworks.
- Continue performance work from measured SQL, WAL, connection, memory or FUSE
  callback evidence.
- Keep backup and restore aligned with PostgreSQL operational practices.
- Split oversized Rust modules incrementally when the moves improve review and
  test isolation without changing behavior.
- Add dependency and security monitoring only when it preserves reproducible
  builds and the one-version-per-commit rule.

## Non-Goals for Now

- full SELinux mount-label policy or host-side FUSE promotion to `fs_use_xattr`
- general-purpose execution semantics for special device nodes beyond stored metadata
- replacing PostgreSQL backup and restore with a custom FOD backup subsystem
- enabling FUSE passthrough without a real backing file descriptor and a coherent PostgreSQL/storage model
- claiming native `SYNCFS`, `TMPFILE`, or `STATX` support before the public `fuser` API and the corresponding FOD contracts exist
- presenting independent writable PostgreSQL primaries as one safe multi-primary FOD filesystem
