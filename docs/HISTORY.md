# FOD historical evidence map

This file indexes **historical evidence**: release-specific implementation
notes, measurements, compatibility/toolchain baselines and work records that
help explain how current FOD behavior was reached.

For current behavior start from [`README.md`](README.md) and
[`CURRENT_STATE.md`](CURRENT_STATE.md). For intended future work use
`../ROADMAP.md`, `../TODO.md`, `FOD_CURRENT_ACTION_PLAN.md` and
`FOD_FUTURE_IDEAS.md`. Plans are deliberately not indexed here as implemented
history.

## FUSE, storage and I/O evolution

Release-specific sizing/buffering work:

- `FOD_3_2_85_FUSE_512K_IO_PLAN.md`
- `FOD_3_2_87_FUSE_SPLIT_WRITE_BUFFERING.md`
- `FOD_3_2_88_MULTI_HANDLE_WRITE_VISIBILITY.md`
- `FOD_3_2_89_DEFAULT_WRITE_256K.md`
- `FOD_3_2_90_DEFAULT_WRITE_512K.md`
- `FOD_3_2_90_IO_BASELINE_4K_1M.md`
- `FOD_3_3_19_FUSE_MAX_WRITE_CONFIG_DEFAULT.md`

FUSE/library compatibility baselines:

- `fuse-abi-7-31-current-baseline.md`
- `fuse-protocol-7-32-7-40-capabilities.md`
- `fuser-0-17-migration-baseline.md`

These documents explain how request/buffer and protocol decisions evolved.
They do not override current `fod_config*.ini`, tests or
[`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md).

## PostgreSQL, routing and failover evolution

- `FOD_3_3_18_POSTGRESQL_PLANNER_STABILITY.md`
- `FOD_3_4_1_PRIMARY_REPLICA_BENCHMARK.md`
- `FOD_3_4_1_PRIMARY_WRITE_POSTGRES_PROFILE.md`
- `postgresql-multi-endpoint-phase-1.md`
- `postgresql-multi-endpoint-phase-2.md`
- `postgresql-multi-endpoint-phase-3.md`
- `postgresql-multi-endpoint-phase-4.md`

For the current runtime contract use
[`POSTGRESQL_REQUIREMENTS.md`](POSTGRESQL_REQUIREMENTS.md),
[`postgresql-runtime-requirements.md`](postgresql-runtime-requirements.md),
[`runtime-configuration.md`](runtime-configuration.md) and
[`CURRENT_STATE.md`](CURRENT_STATE.md).

## Runtime, observability and build/tooling history

- `FOD_3_3_1_SHARED_MONITORING.md`
- `FOD_3_3_20_SHM_CARGO_TARGET.md`
- `FOD_3_3_21_CONTROLLED_TARGET_CLEANUP.md`
- `FOD_3_3_22_AUX_TARGET_CLEANUP.md`
- `FOD_TEST_FUSE_CLEANUP_HARDENING.md`
- `FOD_3_3_31_INSTANCE_LOGGING.md`
- `FOD_3_3_23_RUST_TOOLCHAIN_BENCHMARK.md`
- `FOD_3_3_24_RUST_1_98_RELEASE_LTO.md`

The current Rust toolchain comes from `../rust-toolchain.toml`; old toolchain
benchmarks are evidence, not a version pin.

## Host-security evidence

- `FOD_3_3_22_ROCKY_SELINUX.md` - verified Rocky Linux SELinux behavior behind
  the current host-security guidance.

For current policy use [`SECURITY.md`](SECURITY.md) and
[`FOD_RUNTIME_PRIVILEGE_POLICY.md`](FOD_RUNTIME_PRIVILEGE_POLICY.md).

## Dated performance evidence

These reports are tied to their recorded commit, environment and workload:

- `performance-data-blocks-profile-2026-07-01.md`
- `performance-profile-io-visibility-2026-07-01.md`
- `performance-data-blocks-conflict-noop-profile-2026-07-03.md`
- `performance-data-blocks-conflict-profile-2026-07-03.md`
- `performance-data-blocks-dml-profile-2026-07-03.md`
- `performance-data-blocks-swap-profile-2026-07-03.md`
- `performance-data-blocks-swap-repeat-profile-2026-07-04.md`

For a new optimization start from [`performance.md`](performance.md), capture a
current baseline and compare like-for-like runs. Accepted baseline summaries
belong in `../BENCHMARKS.md` and
[`performance-baselines.md`](performance-baselines.md), not in an old dated
report.

## Long-running work journals

Two root files are retained as chronological working evidence:

- `../commands.md` - commands and execution context recorded during development,
- `../conclusions.md` - accumulated implementation and benchmark conclusions.

They are useful for reconstructing earlier work, but they are not current
operations/configuration documentation and may contain statements tied to older
releases.

## Durable background and decision records

The following documents are long-lived background/decision records rather than
release chronology. They are listed here only to distinguish them from dated
historical evidence:

- [`FOD_NATIVE_INSTALLATION_PACKAGES.md`](FOD_NATIVE_INSTALLATION_PACKAGES.md)
- [`FOD_POSTGRES_32K_CONTAINER.md`](FOD_POSTGRES_32K_CONTAINER.md)
- [`FOD_POSTGRES_BLCKSZ_32K_DEFAULT_DECISION.md`](FOD_POSTGRES_BLCKSZ_32K_DEFAULT_DECISION.md)
- [`FOD_STORAGE_BLOCK_SIZE_SELECTION.md`](FOD_STORAGE_BLOCK_SIZE_SELECTION.md)
- [`FOD_RANDOM_IO_POSTGRESQL_TUNING.md`](FOD_RANDOM_IO_POSTGRESQL_TUNING.md)
- [`FOD_CROSS_PLATFORM_FILESYSTEM_BACKEND_CONSIDERATIONS.md`](FOD_CROSS_PLATFORM_FILESYSTEM_BACKEND_CONSIDERATIONS.md)
- [`adr/storage-format-versioning.md`](adr/storage-format-versioning.md)
- [`adr/storage-object-segment-manifest.md`](adr/storage-object-segment-manifest.md)

Current task documentation may rely on these records, but current behavior must
still be confirmed against current code/tests/configuration and the relevant
task guide.

## Evidence maintenance rule

When behavior changes:

- update the current task/canonical document,
- add a release-specific or dated evidence document only when its measurement,
  compatibility result or implementation reasoning is useful on its own,
- add new historical evidence to this index,
- do not rewrite old evidence to make it appear to describe later defaults,
- keep future plans in roadmap/TODO/plan documents rather than mixing them into
  this historical index.
