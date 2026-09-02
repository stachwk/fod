# FOD documentation

Documentation is organized primarily by **task**. Start from the section that
matches what you want to do. Versioned `FOD_3_*`, dated performance reports and
plan documents are evidence/history, not the first source for current defaults.

## 1. I want to understand the current system

- [`CURRENT_STATE.md`](CURRENT_STATE.md) - compact current architecture,
  deployment model, I/O size layers and lifecycle guarantees.
- [`../README.md`](../README.md) / [`../README.pl`](../README.pl) - project entry
  pages.
- [`../ROADMAP.md`](../ROADMAP.md) - current direction.
- [`../TODO.md`](../TODO.md) - decisions, follow-ups and regression notes.

## 2. I want to install or deploy FOD

### Docker reference deployment

- [`DOCKER_DEPLOYMENT.md`](DOCKER_DEPLOYMENT.md) - complete PostgreSQL + FOD
  Docker deployment.
- [`DOCKER_FOD_INSTALL.md`](DOCKER_FOD_INSTALL.md) - FOD/FUSE client layer.
- [`DOCKER_SYSTEMD.md`](DOCKER_SYSTEMD.md) - persistent host startup and
  systemd integration.
- [`OPERATIONS.md`](OPERATIONS.md) - day-2 status, upgrade, restart and reboot
  procedures.

### Native packages

- [`FOD_NATIVE_INSTALLATION_PACKAGES.md`](FOD_NATIVE_INSTALLATION_PACKAGES.md)
  - Ubuntu/Debian and RHEL/Rocky package build/install model.

## 3. I want to operate or upgrade a deployment

- [`OPERATIONS.md`](OPERATIONS.md) - main operations runbook.
- [`DOCKER_SYSTEMD.md`](DOCKER_SYSTEMD.md) - exact systemd lifecycle and
  non-disruptive reinstall behavior.
- [`DOCKER_FOD_INSTALL.md`](DOCKER_FOD_INSTALL.md) - FUSE identity and FOD-only
  lifecycle.
- [`CURRENT_STATE.md`](CURRENT_STATE.md) - current lifecycle guarantees.

## 4. I want to configure a mount or runtime behavior

- [`runtime-configuration.md`](runtime-configuration.md) - configuration
  precedence, runtime controls, PostgreSQL routing/admission and monitoring
  controls.
- [`runtime-env-ini-audit.md`](runtime-env-ini-audit.md) - generated audit of
  INI-backed and environment-only controls.
- `../fod_config.ini` - local development/test configuration.
- `../fod_config.example.ini` - shareable/package configuration template.

For current high-level defaults, especially I/O sizes, use
[`CURRENT_STATE.md`](CURRENT_STATE.md) together with the current INI files.
Historical version notes inside long-lived configuration documentation explain
how defaults evolved; they do not override current configuration/tests.

## 5. I want to configure PostgreSQL, replicas or failover

- [`POSTGRESQL_REQUIREMENTS.md`](POSTGRESQL_REQUIREMENTS.md) - hard PostgreSQL
  requirements, connection budget and tuning boundaries.
- [`postgresql-runtime-requirements.md`](postgresql-runtime-requirements.md) -
  runtime session contract.
- [`runtime-configuration.md`](runtime-configuration.md) - endpoint routing,
  replica reads, failover, admission and telemetry settings.
- [`FOD_POSTGRES_32K_CONTAINER.md`](FOD_POSTGRES_32K_CONTAINER.md) - reference
  PostgreSQL 32 KiB image details.
- [`FOD_POSTGRES_BLCKSZ_32K_DEFAULT_DECISION.md`](FOD_POSTGRES_BLCKSZ_32K_DEFAULT_DECISION.md)
  - decision record for the reference server block size.

## 6. I want to configure FUSE, storage blocks or I/O sizing

- [`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md) - kernel/FUSE capabilities and
  request-size constraints.
- [`CURRENT_STATE.md`](CURRENT_STATE.md) - current distinction between 4 KiB FOD
  storage blocks, 1 MiB FUSE max write, 512 KiB readahead, 512 KiB base persist
  chunks and the 32 KiB PostgreSQL server block size.
- [`FOD_STORAGE_BLOCK_SIZE_SELECTION.md`](FOD_STORAGE_BLOCK_SIZE_SELECTION.md)
  - storage-block design/selection background.
- [`HISTORY.md`](HISTORY.md) - map to older FUSE/I/O experiments.

## 7. I want to understand security, permissions or host policy

- [`SECURITY.md`](SECURITY.md) - task-oriented security/permissions entry point.
- [`FOD_RUNTIME_PRIVILEGE_POLICY.md`](FOD_RUNTIME_PRIVILEGE_POLICY.md) -
  authoritative root-operated runtime policy.
- [`FOD_3_3_22_ROCKY_SELINUX.md`](FOD_3_3_22_ROCKY_SELINUX.md) - verified
  Rocky Linux 10.2 SELinux model.
- [`LOCAL_TEST_DATA_PRIVACY.md`](LOCAL_TEST_DATA_PRIVACY.md) - local/private test
  data handling.

Docker AppArmor requirements are also documented in
[`DOCKER_FOD_INSTALL.md`](DOCKER_FOD_INSTALL.md).

## 8. I want to profile or optimize performance

- [`performance.md`](performance.md) - canonical profiling workflow and Make
  targets.
- [`../BENCHMARKS.md`](../BENCHMARKS.md) - accepted benchmark baselines.
- [`performance-baselines.md`](performance-baselines.md) - baseline details.
- [`FOD_RANDOM_IO_POSTGRESQL_TUNING.md`](FOD_RANDOM_IO_POSTGRESQL_TUNING.md) -
  PostgreSQL random-I/O tuning background.
- [`HISTORY.md`](HISTORY.md) - dated/version-specific performance evidence.

Performance changes should be supported by before/after measurements from the
relevant workload.

## 9. I want to index or import external sources

- [`fod-indexer.md`](fod-indexer.md) - complete indexer guide: sources,
  scan/hash/dedupe/plan/materialize/cleanup and snapshots.
- [`fod-indexer-read-api.md`](fod-indexer-read-api.md) - read-only integration
  contract.

The shared indexer stays outside the FUSE hot path and keeps source-specific
logic at the adapter boundary.

## 10. I want to test or validate a change

- [`../zasady_sprawdzen.md`](../zasady_sprawdzen.md) - step-by-step validation
  profiles and ordering rules.
- [`../README.md`](../README.md#quality-gates) - minimum release/policy gates.
- [`performance.md`](performance.md) - profiling gates for performance work.

The repository intentionally has no active GitHub Actions workflow. Validation
uses local Make/Cargo gates.

## 11. I want to develop FOD, change schema or compatibility contracts

- [`DEVELOPMENT.md`](DEVELOPMENT.md) - development entry point.
- [`versioning.md`](versioning.md) - one-version-per-commit rule.
- [`compatibility-contracts.md`](compatibility-contracts.md) - compatibility
  contracts.
- [`space-accounting.md`](space-accounting.md) - capacity/accounting contract.
- [`MAKE_TARGET_NAMING.md`](MAKE_TARGET_NAMING.md) - Make target naming policy.
- `../migrations/` - fresh schema and numbered upgrade path.
- [`adr/storage-format-versioning.md`](adr/storage-format-versioning.md) -
  storage-format versioning ADR.
- [`adr/storage-object-segment-manifest.md`](adr/storage-object-segment-manifest.md)
  - storage object/segment manifest ADR.

## 12. I want plans, future ideas or historical evidence

- [`HISTORY.md`](HISTORY.md) - categorized map of versioned implementation and
  benchmark evidence.
- [`FOD_CURRENT_ACTION_PLAN.md`](FOD_CURRENT_ACTION_PLAN.md) - ordered current
  implementation sequence.
- [`FOD_FUTURE_IDEAS.md`](FOD_FUTURE_IDEAS.md) - future ideas.
- [`../ROADMAP.md`](../ROADMAP.md) - roadmap.
- [`../TODO.md`](../TODO.md) - follow-ups/decisions.

Files named `FOD_<version>_*.md`, dated performance documents and `*-plan.md`
files are intentionally retained for traceability. Do not use them as the
canonical source for a current default unless a current task document points to
that result explicitly.
