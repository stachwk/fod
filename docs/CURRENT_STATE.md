# FOD current state

This document is the compact canonical description of the **current** FOD architecture and operational defaults. Historical versioned documents remain useful evidence but may describe defaults that have since changed.

The authoritative release version is stored in `../fod_version.txt`.

## Runtime architecture

FOD is Rust-backed end to end:

- `rust_fuse` owns the FUSE frontend and filesystem callbacks,
- `rust_runtime` owns PostgreSQL connectivity, configuration and shared runtime services,
- `rust_hotpath` contains storage/read/write hot-path helpers,
- `rust_mkfs` owns schema/bootstrap/config tooling,
- `rust_monitor` owns runtime and cluster diagnostics,
- `rust_indexer` owns external-source indexing/import tooling.

PostgreSQL owns durable filesystem metadata, payload, shared lock/session state and replication.

## Production Docker topology

The supported reference topology is:

```text
MASTERS=1
SLAVES=0..32
1 persistent FOD/FUSE client
```

`MASTERS>1` is rejected.

Current production PostgreSQL image:

```text
ghcr.io/stachwk/postgres-16-fod-32k:16.15
```

By default the FOD client uses the exact release from `fod_version.txt`. `FOD_CLIENT_VERSION=X.Y.Z` may select another already-published client build without changing the repository/source version. A fully-qualified `FOD_DOCKER_DEPLOY_CLIENT_IMAGE=registry/path:tag` override has the highest deployment priority. The mutable `:3.4` tag remains only a convenience alias.

Image build/publish remains source-version based by default. Selecting `FOD_CLIENT_VERSION` for deployment does not retag binaries built from the current checkout as an older release.

## Storage payload model

The production payload model is **block-only**. Durable file payload is canonical in `fod.data_blocks`; the former extent runtime path was retired after migration of legacy extent data back to blocks.

The default logical FOD storage block for **newly initialized** filesystems is 32 KiB. The storage block size is persisted per filesystem in `fod.config` as `block_size`; existing filesystems keep the value with which they were initialized, including historical 4 KiB filesystems. Changing an existing filesystem to another block size requires an explicit data-format migration/rewrite and is not a runtime tuning operation.

Historical extent-engine plans, measurements and migration records are retained under [`history/`](history/) as evidence, but they are not alternate current runtime paths and must not be used to infer present storage behavior.

A future payload-format change requires an explicit storage-format/compatibility decision and migration plan; it must not be introduced as an implicit performance shortcut.

## I/O size layers

These values describe different layers and must not be conflated:

| Layer | Current default/reference | Meaning |
| --- | ---: | --- |
| FOD storage block | 32 KiB for newly initialized filesystems | logical block persisted per filesystem in `fod.config`; existing filesystems retain their recorded value |
| FUSE max write | 1 MiB | request ceiling exposed/configured for FUSE writes |
| FUSE max readahead | 512 KiB | readahead ceiling |
| base persist chunk | 128 FOD blocks = 4 MiB at the 32 KiB default | normal PostgreSQL payload batching unit; byte size scales with the filesystem's persisted storage block size |
| PostgreSQL server block | 32 KiB | `BLCKSZ` of the reference PostgreSQL Docker image |

Increasing a FUSE request size does **not** change the on-database FOD storage format. A 32 KiB default applies only when a new filesystem is initialized without an explicit `--block-size`; an existing filesystem continues to use its persisted `block_size`.

The new-filesystem storage default is owned by `fod-rust-mkfs`; current runtime/FUSE defaults are defined by `../fod_config.ini` and `../fod_config.example.ini`.

## PostgreSQL deployment behavior

The reference Docker deployment uses one writable primary and zero or more streaming replicas. Smoke validation requires:

- primary `SHOW block_size = 32768`,
- primary not in recovery,
- every requested replica in recovery,
- at least the requested number of streaming replication connections.

### Repository QNAP PostgreSQL preset

`QNAP=1` selects a persistent server-tuning profile intended for the current
reference QNAP host:

```text
8 GB RAM
2 CPU
HDD
PostgreSQL 16.15
server BLCKSZ = 32 KiB
```

The repository preset is:

```text
shared_buffers                  = 1GB
effective_cache_size            = 4GB
work_mem                        = 4MB
maintenance_work_mem            = 256MB
max_connections                 = 64
random_page_cost                = 4
effective_io_concurrency        = 1
max_wal_size                    = 2GB
checkpoint_timeout              = 15min
checkpoint_completion_target    = 0.9
wal_compression                 = off
autovacuum_max_workers          = 2
autovacuum_work_mem             = 128MB
max_parallel_workers            = 2
max_parallel_workers_per_gather = 1
```

`pg_stat_statements` remains the default shared preload library.

The resolved preset can be inspected without changing the server:

```bash
QNAP=1 make postgres-qnap-config-show
```

The QNAP profile is supplied to PostgreSQL through the container command line,
so it survives recreation of the PostgreSQL volume. Explicit `POSTGRES_*=...`
values supplied on the `make` command line may override the preset for
controlled tests/benchmarks. Ambient shell `POSTGRES_*` values are not treated
as QNAP defaults.

`QNAP=0` keeps the normal local PostgreSQL defaults unless tuning is supplied
explicitly.

The 2026-09-08 live validation confirmed all 15 profile parameters with
`source = command line`; the earlier `ALTER SYSTEM` copies were removed and
`postgresql.auto.conf` contained no remaining profile entries.

## FOD/FUSE deployment behavior

The FOD client container requires `/dev/fuse`, `CAP_SYS_ADMIN`, `rshared` bind propagation and compatible host AppArmor policy.

A host can legitimately show more than one propagated FUSE row for the same mount. Health is determined by one unique FUSE `MAJ:MIN` identity shared by the host and the FOD container, not by requiring exactly one `findmnt` row.

## systemd lifecycle

### First start / inactive service

```text
systemctl start
  -> host-prepare
  -> docker_deploy.sh up
  -> FOD start/reconcile
  -> PostgreSQL smoke
  -> FOD/FUSE smoke
```

### Reinstall / FOD upgrade while service is active

```text
systemctl reload
  -> boot.sh start
  -> reconcile existing PostgreSQL topology
  -> reconcile exact FOD image
  -> smoke checks
```

This path does not execute `ExecStop`. Healthy PostgreSQL primary/replica containers are therefore preserved during an FOD client reinstall/upgrade.

The image reconciliation guard compares the running FOD container image ID with the required exact image ID. If they differ, only the FOD container/mount is recreated.

For systemd installs the selected client version is resolved to a full image name and persisted as `FOD_DOCKER_DEPLOY_CLIENT_IMAGE` in `/etc/fod/docker-deploy.env`. Later reboots therefore keep using that selected image even if the Git checkout advances to another FOD release.

The behavior was live-validated during the `3.4.4 -> 3.4.5` upgrade: the FOD container changed to the new exact image while primary and both replica container IDs remained unchanged.

### Explicit full restart

`make docker-deploy-systemd-restart` intentionally performs a full service restart. FOD is stopped first and the PostgreSQL Compose containers are then stopped/recreated while persistent volumes are retained.

## Data safety boundaries

Normal `down`, reinstall and systemd lifecycle operations preserve PostgreSQL volumes and generated deployment state.

Destructive volume removal is a separate guarded operation:

```bash
make docker-deploy-destroy MASTERS=1 SLAVES=2 DESTROY=YES
```

## Validation entry points

```bash
make test-version
make test-cargo-lock-integrity
make test-docker-deploy-policy
make test-docker-fod-install-policy
make test-docker-deploy-systemd-policy
make test-docker-fod-client-policy
```

Operational smoke:

```bash
sudo make docker-deploy-systemd-smoke MASTERS=1 SLAVES=2
```

For task-oriented navigation see [`README.md`](README.md).
