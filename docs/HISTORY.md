# FOD documentation history and evidence map

This document separates **current task documentation** from version-specific
implementation notes, benchmark evidence and future plans.

Use [`CURRENT_STATE.md`](CURRENT_STATE.md), current configuration files and the
current task guides to determine present behavior. Use the documents listed
here when you need the evidence or reasoning behind a particular change.

## Current canonical/task documents

These are intended to describe the current project rather than one historical
release:

- [`README.md`](README.md) - task index for all documentation,
- [`CURRENT_STATE.md`](CURRENT_STATE.md) - architecture/defaults/lifecycle,
- [`OPERATIONS.md`](OPERATIONS.md) - deployment operations and upgrade runbook,
- [`SECURITY.md`](SECURITY.md) - privilege, permissions and host security,
- [`DEVELOPMENT.md`](DEVELOPMENT.md) - development/versioning/testing entry point,
- [`runtime-configuration.md`](runtime-configuration.md) - runtime controls and
  configuration history,
- [`POSTGRESQL_REQUIREMENTS.md`](POSTGRESQL_REQUIREMENTS.md) - PostgreSQL
  requirements,
- [`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md) - FUSE/kernel requirements,
- [`performance.md`](performance.md) - profiling workflow,
- [`fod-indexer.md`](fod-indexer.md) - indexer guide.

When a long-lived document contains historical version notes, the current
configuration/tests and `CURRENT_STATE.md` take precedence for current defaults.

## FUSE and I/O sizing history

- `FOD_3_2_85_FUSE_512K_IO_PLAN.md`
- `FOD_3_2_87_FUSE_SPLIT_WRITE_BUFFERING.md`
- `FOD_3_2_88_MULTI_HANDLE_WRITE_VISIBILITY.md`
- `FOD_3_2_89_DEFAULT_WRITE_256K.md`
- `FOD_3_2_90_DEFAULT_WRITE_512K.md`
- `FOD_3_2_90_IO_BASELINE_4K_1M.md`
- `FOD_3_3_19_FUSE_MAX_WRITE_CONFIG_DEFAULT.md`

These files record how request/buffer defaults evolved. They do not override
the current `fod_config*.ini` values.

## Monitoring, PostgreSQL and runtime history

- `FOD_3_3_1_SHARED_MONITORING.md`
- `FOD_3_3_18_POSTGRESQL_PLANNER_STABILITY.md`
- `FOD_3_3_20_SHM_CARGO_TARGET.md`
- `FOD_3_3_21_CONTROLLED_TARGET_CLEANUP.md`
- `FOD_3_3_22_AUX_TARGET_CLEANUP.md`
- `FOD_3_3_31_INSTANCE_LOGGING.md`
- `FOD_3_4_1_PRIMARY_REPLICA_BENCHMARK.md`
- `FOD_3_4_1_PRIMARY_WRITE_POSTGRES_PROFILE.md`

For current deployment and operational behavior prefer `CURRENT_STATE.md`,
`OPERATIONS.md`, `POSTGRESQL_REQUIREMENTS.md` and `runtime-configuration.md`.

## Host security and toolchain history

- `FOD_3_3_22_ROCKY_SELINUX.md` - verified Rocky Linux SELinux behavior; it
  remains the detailed evidence behind the current security summary,
- `FOD_3_3_23_RUST_TOOLCHAIN_BENCHMARK.md`,
- `FOD_3_3_24_RUST_1_98_RELEASE_LTO.md`.

The current pinned Rust toolchain is defined by `../rust-toolchain.toml`, not by
copying a version from an older benchmark note.

## Packaging and PostgreSQL image decisions

These documents describe decisions/infrastructure that may span releases but
also contain version-specific examples:

- [`FOD_NATIVE_INSTALLATION_PACKAGES.md`](FOD_NATIVE_INSTALLATION_PACKAGES.md),
- [`FOD_POSTGRES_32K_CONTAINER.md`](FOD_POSTGRES_32K_CONTAINER.md),
- [`FOD_POSTGRES_BLCKSZ_32K_DEFAULT_DECISION.md`](FOD_POSTGRES_BLCKSZ_32K_DEFAULT_DECISION.md),
- [`FOD_STORAGE_BLOCK_SIZE_SELECTION.md`](FOD_STORAGE_BLOCK_SIZE_SELECTION.md),
- [`FOD_RANDOM_IO_POSTGRESQL_TUNING.md`](FOD_RANDOM_IO_POSTGRESQL_TUNING.md),
- [`FOD_CROSS_PLATFORM_FILESYSTEM_BACKEND_CONSIDERATIONS.md`](FOD_CROSS_PLATFORM_FILESYSTEM_BACKEND_CONSIDERATIONS.md).

## Performance evidence

The repository contains dated or workload-specific performance documents such
as `performance-data-blocks-*` and `performance-profile-*`. Treat these as
measurement evidence tied to the recorded environment, commit and workload.

For a new optimization:

1. start from [`performance.md`](performance.md),
2. capture a current baseline,
3. compare like-for-like environment/workload runs,
4. update accepted baseline summaries only after the result is repeatable.

## Plans and proposals

The following are planning documents rather than statements that the feature is
already implemented:

- [`FOD_CURRENT_ACTION_PLAN.md`](FOD_CURRENT_ACTION_PLAN.md),
- [`FOD_FUTURE_IDEAS.md`](FOD_FUTURE_IDEAS.md),
- [`fod-roadmap-3.2.62-plus.md`](fod-roadmap-3.2.62-plus.md),
- [`storage-engine-v2-plan.md`](storage-engine-v2-plan.md),
- [`transactional-replay-project.md`](transactional-replay-project.md),
- [`quota-lock-concurrency-plan.md`](quota-lock-concurrency-plan.md),
- [`indexer-catalog-snapshot-regression-plan.md`](indexer-catalog-snapshot-regression-plan.md),
- [`block-only-performance-plan.md`](block-only-performance-plan.md),
- [`mounted-fuse-write-profile-plan.md`](mounted-fuse-write-profile-plan.md).

A plan should be linked from current documentation only when it helps explain
future work; it should not be phrased as current capability until implementation
and validation have landed.

## ADRs and durable design decisions

Architecture Decision Records are different from both task guides and benchmark
history. They capture durable design decisions:

- [`adr/storage-format-versioning.md`](adr/storage-format-versioning.md),
- [`adr/storage-object-segment-manifest.md`](adr/storage-object-segment-manifest.md).

When an ADR conflicts with an old experiment note, prefer the accepted ADR plus
current code/tests.

## Maintenance rule

When behavior changes:

- update the current task/canonical document,
- add a version-specific evidence document only when the measurement or
  implementation history is useful on its own,
- do not rewrite old evidence to pretend it described the new behavior,
- link historical evidence from here instead of expanding root README files
  with release-by-release chronology.
