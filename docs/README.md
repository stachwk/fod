# FOD documentation

Documentation is organized by task first, then by lifecycle class. Current task guides stay directly under `docs/`; historical evidence lives under [`history/`](history/); active and future plans live under [`plans/`](plans/).

## Documentation classes and source-of-truth order

| Class | Use it for | Primary locations |
| --- | --- | --- |
| Current behavior | What FOD does now and how to operate it | [`CURRENT_STATE.md`](CURRENT_STATE.md) and the task guides below |
| Durable contracts | Compatibility, security, schema/storage and architecture decisions expected to outlive one release | requirement/contract documents and [`adr/`](adr/) |
| Historical evidence | Release-specific implementation notes, measurements, compatibility/toolchain baselines and completed project records | [`HISTORY.md`](HISTORY.md) and [`history/`](history/) |
| Plans | Intended work that is not yet a statement of current capability | [`plans/`](plans/), `../ROADMAP.md`, `../TODO.md` |
| Work journals | Execution logs and accumulated conclusions retained for traceability | `../commands.md`, `../conclusions.md` |

When documents disagree about **current behavior**, use this order:

1. current code, tests, configuration and authoritative version metadata,
2. [`CURRENT_STATE.md`](CURRENT_STATE.md) and the current task guide for the area,
3. durable requirement/contract/ADR documents,
4. historical evidence and work journals,
5. plans and proposals.

Historical evidence is intentionally not rewritten to match later defaults. Update the current task documentation instead.

## 1. Understand the current system

- [`CURRENT_STATE.md`](CURRENT_STATE.md) - compact current architecture, deployment model, I/O size layers and lifecycle guarantees.
- [`../README.md`](../README.md) / [`../README.pl`](../README.pl) - project entry pages.
- [`../ROADMAP.md`](../ROADMAP.md) - current direction.
- [`../TODO.md`](../TODO.md) - decisions, follow-ups and regression notes.

## 2. Install or deploy FOD

- [`DOCKER_DEPLOYMENT.md`](DOCKER_DEPLOYMENT.md) - complete PostgreSQL + FOD Docker deployment.
- [`DOCKER_FOD_INSTALL.md`](DOCKER_FOD_INSTALL.md) - FOD/FUSE client layer.
- [`DOCKER_SYSTEMD.md`](DOCKER_SYSTEMD.md) - persistent host startup and systemd integration.
- [`OPERATIONS.md`](OPERATIONS.md) - day-2 status, upgrade, restart and reboot procedures.
- [`FOD_NATIVE_INSTALLATION_PACKAGES.md`](FOD_NATIVE_INSTALLATION_PACKAGES.md) - native Ubuntu/Debian and RHEL/Rocky package model.

## 3. Configure runtime, PostgreSQL and FUSE

- [`runtime-configuration.md`](runtime-configuration.md) - configuration precedence and runtime controls.
- [`runtime-env-ini-audit.md`](runtime-env-ini-audit.md) - generated environment/INI audit.
- [`POSTGRESQL_REQUIREMENTS.md`](POSTGRESQL_REQUIREMENTS.md) - PostgreSQL requirements and tuning boundaries.
- [`postgresql-runtime-requirements.md`](postgresql-runtime-requirements.md) - PostgreSQL session contract.
- [`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md) - FUSE/kernel capabilities and request-size constraints.
- [`FOD_STORAGE_BLOCK_SIZE_SELECTION.md`](FOD_STORAGE_BLOCK_SIZE_SELECTION.md) - storage-block design background.

Current high-level defaults are summarized in [`CURRENT_STATE.md`](CURRENT_STATE.md). Historical sizing experiments are indexed by [`HISTORY.md`](HISTORY.md).

## 4. Security, permissions and host policy

- [`SECURITY.md`](SECURITY.md) - security/permissions entry point.
- [`FOD_RUNTIME_PRIVILEGE_POLICY.md`](FOD_RUNTIME_PRIVILEGE_POLICY.md) - authoritative root-operated runtime policy.
- [`history/FOD_3_3_22_ROCKY_SELINUX.md`](history/FOD_3_3_22_ROCKY_SELINUX.md) - verified Rocky Linux 10.2 SELinux evidence.
- [`LOCAL_TEST_DATA_PRIVACY.md`](LOCAL_TEST_DATA_PRIVACY.md) - local/private test-data handling.

## 5. Profile or optimize performance

- [`performance.md`](performance.md) - canonical profiling workflow and Make targets.
- [`../BENCHMARKS.md`](../BENCHMARKS.md) - accepted benchmark baselines.
- [`performance-baselines.md`](performance-baselines.md) - baseline details.
- [`FOD_RANDOM_IO_POSTGRESQL_TUNING.md`](FOD_RANDOM_IO_POSTGRESQL_TUNING.md) - PostgreSQL random-I/O tuning background.
- [`plans/CURRENT.md`](plans/CURRENT.md) - maintained next optimization target and acceptance boundary.
- [`HISTORY.md`](HISTORY.md) - dated, release-specific and completed performance-plan evidence.

## 6. Index or import external sources

- [`fod-indexer.md`](fod-indexer.md) - complete indexer guide.
- [`fod-indexer-read-api.md`](fod-indexer-read-api.md) - read-only integration contract.

## 7. Test or validate a change

- [`../zasady_sprawdzen.md`](../zasady_sprawdzen.md) - step-by-step validation profiles and ordering rules.
- [`../README.md`](../README.md#quality-gates) - minimum release/policy gates.
- [`performance.md`](performance.md) - profiling gates for performance work.

The repository intentionally has no active GitHub Actions workflow. Validation uses local Make/Cargo gates.

## 8. Develop FOD, change schema or compatibility contracts

- [`DEVELOPMENT.md`](DEVELOPMENT.md) - development entry point.
- [`versioning.md`](versioning.md) - one-version-per-commit rule.
- [`compatibility-contracts.md`](compatibility-contracts.md) - compatibility contracts.
- [`space-accounting.md`](space-accounting.md) - capacity/accounting contract.
- [`MAKE_TARGET_NAMING.md`](MAKE_TARGET_NAMING.md) - Make target naming policy.
- [`adr/storage-format-versioning.md`](adr/storage-format-versioning.md) - storage-format versioning ADR.
- [`adr/storage-object-segment-manifest.md`](adr/storage-object-segment-manifest.md) - storage object/segment manifest ADR.

## 9. Historical evidence

- [`HISTORY.md`](HISTORY.md) - categorized evidence map.
- [`history/README.md`](history/README.md) - physical history-directory policy.
- `../commands.md` - chronological execution journal.
- `../conclusions.md` - accumulated implementation/benchmark conclusions.

Historical files remain as recorded and are not canonical sources for later defaults.

## 10. Plans and future work

- [`plans/README.md`](plans/README.md) - plans index and lifecycle rule.
- [`plans/CURRENT.md`](plans/CURRENT.md) - compact maintained current implementation order.
- [`plans/FOD_FUTURE_IDEAS.md`](plans/FOD_FUTURE_IDEAS.md) - longer-term ideas and proposals.
- [`../ROADMAP.md`](../ROADMAP.md) - long-term roadmap.
- [`../TODO.md`](../TODO.md) - mixed follow-up/archive record.

Completed plans and old execution roadmaps are retained under [`history/`](history/) rather than mixed with active/future plans.
