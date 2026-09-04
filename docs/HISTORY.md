# FOD historical evidence map

This file indexes **historical evidence**: release-specific implementation
notes, measurements, compatibility/toolchain baselines, completed design plans
and work records that help explain how current FOD behavior was reached.

Historical documents are stored physically under [`history/`](history/). For
current behavior start from [`README.md`](README.md) and
[`CURRENT_STATE.md`](CURRENT_STATE.md). For intended future work use
[`plans/CURRENT.md`](plans/CURRENT.md), `../ROADMAP.md`, `../TODO.md` and the
remaining documents under [`plans/`](plans/).

## FUSE, storage and I/O evolution

Release-specific sizing/buffering work:

- [`FOD_3_2_85_FUSE_512K_IO_PLAN.md`](history/FOD_3_2_85_FUSE_512K_IO_PLAN.md)
- [`FOD_3_2_87_FUSE_SPLIT_WRITE_BUFFERING.md`](history/FOD_3_2_87_FUSE_SPLIT_WRITE_BUFFERING.md)
- [`FOD_3_2_88_MULTI_HANDLE_WRITE_VISIBILITY.md`](history/FOD_3_2_88_MULTI_HANDLE_WRITE_VISIBILITY.md)
- [`FOD_3_2_89_DEFAULT_WRITE_256K.md`](history/FOD_3_2_89_DEFAULT_WRITE_256K.md)
- [`FOD_3_2_90_DEFAULT_WRITE_512K.md`](history/FOD_3_2_90_DEFAULT_WRITE_512K.md)
- [`FOD_3_2_90_IO_BASELINE_4K_1M.md`](history/FOD_3_2_90_IO_BASELINE_4K_1M.md)
- [`FOD_3_3_19_FUSE_MAX_WRITE_CONFIG_DEFAULT.md`](history/FOD_3_3_19_FUSE_MAX_WRITE_CONFIG_DEFAULT.md)

FUSE/library compatibility baselines:

- [`fuse-abi-7-31-current-baseline.md`](history/fuse-abi-7-31-current-baseline.md)
- [`fuse-protocol-7-32-7-40-capabilities.md`](history/fuse-protocol-7-32-7-40-capabilities.md)
- [`fuser-0-17-migration-baseline.md`](history/fuser-0-17-migration-baseline.md)

These documents explain how request/buffer and protocol decisions evolved.
They do not override current `fod_config*.ini`, tests or
[`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md).

## PostgreSQL, routing and failover evolution

- [`FOD_3_3_18_POSTGRESQL_PLANNER_STABILITY.md`](history/FOD_3_3_18_POSTGRESQL_PLANNER_STABILITY.md)
- [`FOD_3_4_1_PRIMARY_REPLICA_BENCHMARK.md`](history/FOD_3_4_1_PRIMARY_REPLICA_BENCHMARK.md)
- [`FOD_3_4_1_PRIMARY_WRITE_POSTGRES_PROFILE.md`](history/FOD_3_4_1_PRIMARY_WRITE_POSTGRES_PROFILE.md)
- [`postgresql-multi-endpoint-phase-1.md`](history/postgresql-multi-endpoint-phase-1.md)
- [`postgresql-multi-endpoint-phase-2.md`](history/postgresql-multi-endpoint-phase-2.md)
- [`postgresql-multi-endpoint-phase-3.md`](history/postgresql-multi-endpoint-phase-3.md)
- [`postgresql-multi-endpoint-phase-4.md`](history/postgresql-multi-endpoint-phase-4.md)

For the current runtime contract use
[`POSTGRESQL_REQUIREMENTS.md`](POSTGRESQL_REQUIREMENTS.md),
[`postgresql-runtime-requirements.md`](postgresql-runtime-requirements.md),
[`runtime-configuration.md`](runtime-configuration.md) and
[`CURRENT_STATE.md`](CURRENT_STATE.md).

## Runtime, observability and build/tooling history

- [`FOD_3_3_1_SHARED_MONITORING.md`](history/FOD_3_3_1_SHARED_MONITORING.md)
- [`FOD_3_3_20_SHM_CARGO_TARGET.md`](history/FOD_3_3_20_SHM_CARGO_TARGET.md)
- [`FOD_3_3_21_CONTROLLED_TARGET_CLEANUP.md`](history/FOD_3_3_21_CONTROLLED_TARGET_CLEANUP.md)
- [`FOD_3_3_22_AUX_TARGET_CLEANUP.md`](history/FOD_3_3_22_AUX_TARGET_CLEANUP.md)
- [`FOD_TEST_FUSE_CLEANUP_HARDENING.md`](history/FOD_TEST_FUSE_CLEANUP_HARDENING.md)
- [`FOD_3_3_31_INSTANCE_LOGGING.md`](history/FOD_3_3_31_INSTANCE_LOGGING.md)
- [`FOD_3_3_23_RUST_TOOLCHAIN_BENCHMARK.md`](history/FOD_3_3_23_RUST_TOOLCHAIN_BENCHMARK.md)
- [`FOD_3_3_24_RUST_1_98_RELEASE_LTO.md`](history/FOD_3_3_24_RUST_1_98_RELEASE_LTO.md)

The current Rust toolchain comes from `../rust-toolchain.toml`; old toolchain
benchmarks are evidence, not a version pin.

## Host-security evidence

- [`FOD_3_3_22_ROCKY_SELINUX.md`](history/FOD_3_3_22_ROCKY_SELINUX.md) -
  verified Rocky Linux SELinux behavior behind the current host-security
  guidance.

For current policy use [`SECURITY.md`](SECURITY.md) and
[`FOD_RUNTIME_PRIVILEGE_POLICY.md`](FOD_RUNTIME_PRIVILEGE_POLICY.md).

## Dated performance evidence

These reports are tied to their recorded commit, environment and workload:

- [`performance-data-blocks-profile-2026-07-01.md`](history/performance-data-blocks-profile-2026-07-01.md)
- [`performance-profile-io-visibility-2026-07-01.md`](history/performance-profile-io-visibility-2026-07-01.md)
- [`performance-data-blocks-conflict-noop-profile-2026-07-03.md`](history/performance-data-blocks-conflict-noop-profile-2026-07-03.md)
- [`performance-data-blocks-conflict-profile-2026-07-03.md`](history/performance-data-blocks-conflict-profile-2026-07-03.md)
- [`performance-data-blocks-dml-profile-2026-07-03.md`](history/performance-data-blocks-dml-profile-2026-07-03.md)
- [`performance-data-blocks-swap-profile-2026-07-03.md`](history/performance-data-blocks-swap-profile-2026-07-03.md)
- [`performance-data-blocks-swap-repeat-profile-2026-07-04.md`](history/performance-data-blocks-swap-repeat-profile-2026-07-04.md)

For a new optimization start from [`performance.md`](performance.md), capture a
current baseline and compare like-for-like runs. Accepted baseline summaries
belong in `../BENCHMARKS.md` and
[`performance-baselines.md`](performance-baselines.md), not in an old dated
report.

## Completed project and planning records

These documents have plan/roadmap-style names but now primarily record completed
implementation, design or measurement work:

- [`FOD_CURRENT_ACTION_PLAN_2026-08-26.md`](history/FOD_CURRENT_ACTION_PLAN_2026-08-26.md)
  - former ordered implementation plan; its completed sequence is preserved
  without making it the current backlog,
- [`block-only-performance-plan.md`](history/block-only-performance-plan.md) -
  completed block-only write/performance execution record,
- [`mounted-fuse-write-profile-plan.md`](history/mounted-fuse-write-profile-plan.md)
  - completed mounted-FUSE write profiling/optimization record,
- [`quota-lock-concurrency-plan.md`](history/quota-lock-concurrency-plan.md) -
  completed quota-lock concurrency redesign/validation record,
- [`transactional-replay-project.md`](history/transactional-replay-project.md) -
  stabilized bounded transactional replay project and smoke-coverage record,
- [`storage-engine-v2-plan.md`](history/storage-engine-v2-plan.md) - retired
  Storage Engine v2/extent experiment record,
- [`indexer-catalog-snapshot-regression-plan.md`](history/indexer-catalog-snapshot-regression-plan.md)
  - implemented catalogue-snapshot regression plan,
- [`fod-roadmap-3.2.62-plus.md`](history/fod-roadmap-3.2.62-plus.md) - execution
  roadmap tied to the historical 3.2.62+ sequence.

The maintained next-step plan is [`plans/CURRENT.md`](plans/CURRENT.md). Longer
term proposals stay under [`plans/`](plans/) or `../ROADMAP.md`.

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
- add a release-specific, dated or completed-project evidence document under
  `history/` only when its measurement, compatibility result or implementation
  reasoning is useful on its own,
- add new historical evidence to this index,
- do not rewrite old evidence to make it appear to describe later defaults,
- keep active/future plans under `plans/`, `ROADMAP.md` or `TODO.md` rather than
  mixing them into this historical index.
