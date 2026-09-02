# FOD documentation

This directory is organized primarily by **task**. Versioned `FOD_3_*` documents remain as historical implementation/benchmark records and should not be used as the first source for current defaults.

## 1. I want to understand the current system

- [`CURRENT_STATE.md`](CURRENT_STATE.md) - current architecture, deployment model, I/O size layers and lifecycle guarantees.
- [`../ROADMAP.md`](../ROADMAP.md) - current direction and planned work.
- [`../TODO.md`](../TODO.md) - decisions, follow-ups and regression notes.

## 2. I want to install or deploy FOD

- [`DOCKER_DEPLOYMENT.md`](DOCKER_DEPLOYMENT.md) - complete PostgreSQL + FOD Docker deployment.
- [`DOCKER_FOD_INSTALL.md`](DOCKER_FOD_INSTALL.md) - FOD/FUSE client layer.
- [`DOCKER_SYSTEMD.md`](DOCKER_SYSTEMD.md) - persistent host startup and systemd integration.

## 3. I want to operate or upgrade a deployment

- [`OPERATIONS.md`](OPERATIONS.md) - status, smoke, upgrade, restart and reboot verification.
- [`DOCKER_SYSTEMD.md`](DOCKER_SYSTEMD.md) - exact systemd lifecycle.
- [`DOCKER_FOD_INSTALL.md`](DOCKER_FOD_INSTALL.md) - FUSE identity and FOD-only lifecycle.

## 4. I want to configure PostgreSQL

- [`POSTGRESQL_REQUIREMENTS.md`](POSTGRESQL_REQUIREMENTS.md) - hard requirements, connection budget and server settings.
- [`postgresql-runtime-requirements.md`](postgresql-runtime-requirements.md) - runtime session contract.
- `../fod_config.ini` and `../fod_config.example.ini` - current runtime defaults/examples.

## 5. I want to configure FUSE or I/O sizing

- [`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md) - kernel/FUSE capabilities and request-size constraints.
- [`CURRENT_STATE.md`](CURRENT_STATE.md) - canonical current distinction between 4 KiB FOD storage blocks, 1 MiB FUSE max write, 512 KiB readahead, 512 KiB base persist chunks and the 32 KiB PostgreSQL server block size.
- Historical measurement records include `FOD_3_2_85_FUSE_512K_IO_PLAN.md`, `FOD_3_2_90_DEFAULT_WRITE_512K.md`, `FOD_3_2_90_IO_BASELINE_4K_1M.md` and `FOD_3_3_19_FUSE_MAX_WRITE_CONFIG_DEFAULT.md`.

## 6. I want to test a change

- [`../zasady_sprawdzen.md`](../zasady_sprawdzen.md) - step-by-step validation profiles.
- [`../README.md`](../README.md#quality-gates) - minimum release/policy gates.
- [`../BENCHMARKS.md`](../BENCHMARKS.md) - benchmark baselines and methodology.

The repository uses local Make/Cargo validation. It intentionally has no active GitHub Actions workflow.

## 7. I want to work on schema or runtime development

- [`versioning.md`](versioning.md) - one-version-per-commit rule.
- [`FOD_CURRENT_ACTION_PLAN.md`](FOD_CURRENT_ACTION_PLAN.md) - ordered current implementation sequence.
- `../migrations/` - fresh schema and numbered upgrade path.
- `../rust_runtime/`, `../rust_fuse/`, `../rust_hotpath/`, `../rust_mkfs/`, `../rust_monitor/`, `../rust_indexer/` - active Rust runtime/tooling crates.

## 8. I want historical implementation evidence

Files named `FOD_<version>_*.md` document specific implementation decisions, experiments and benchmark results from the version named in the file. Keep them for traceability, but prefer `CURRENT_STATE.md`, current configuration files and the task documents above when determining present behavior.
