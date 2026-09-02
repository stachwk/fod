# FOD Docker deployment

This is the final Docker deployment path for FOD. It consists of:

1. PostgreSQL 16 compiled with `BLCKSZ=32K`,
2. exactly one writable primary and `SLAVES=0..32` streaming replicas,
3. the FOD schema in the primary database,
4. one persistent FOD/FUSE client container,
5. optional systemd integration for unattended recovery after a host reboot.

`MASTERS>1` is intentionally rejected. Compose does not implement safe PostgreSQL multi-primary election.

## Release images

The production PostgreSQL image remains pinned to:

```text
ghcr.io/stachwk/postgres-16-fod-32k:16.15
```

The normal Make interface pins the FOD client to the exact repository release. For FOD 3.4.6:

```text
ghcr.io/stachwk/fod-client:3.4.6
```

The mutable `ghcr.io/stachwk/fod-client:3.4` tag remains a convenience series alias only. Override the exact image explicitly with `FOD_DOCKER_DEPLOY_CLIENT_IMAGE` when required.

## Complete installation

```bash
make docker-deploy-plan MASTERS=1 SLAVES=2
make docker-deploy-fod-host-prepare MASTERS=1 SLAVES=2
make docker-deploy-install MASTERS=1 SLAVES=2
make docker-deploy-smoke MASTERS=1 SLAVES=2
```

`docker-deploy-install` renders protected state, pulls and starts the PostgreSQL topology, initializes the FOD schema if needed, starts the FOD/FUSE container and executes PostgreSQL/FOD smoke checks.

Default host endpoints:

| Node | Endpoint |
| --- | --- |
| primary | `127.0.0.1:55441` |
| replica1 | `127.0.0.1:55442` |
| replica2 | `127.0.0.1:55443` |
| replicaN | consecutive ports from `55442` |

The FOD host mount defaults to:

```text
~/.local/share/fod/mount
```

Generated deployment state defaults to:

```text
~/.local/state/fod/docker-deploy/
```

Passwords remain in protected generated state files and are not embedded into Compose or systemd unit files.

## Lifecycle

Whole deployment:

```bash
make docker-deploy-up MASTERS=1 SLAVES=2
make docker-deploy-status MASTERS=1 SLAVES=2
make docker-deploy-smoke MASTERS=1 SLAVES=2
make docker-deploy-down MASTERS=1 SLAVES=2
```

FOD client only:

```bash
make docker-deploy-fod-up MASTERS=1 SLAVES=2
make docker-deploy-fod-status MASTERS=1 SLAVES=2
make docker-deploy-fod-smoke MASTERS=1 SLAVES=2
make docker-deploy-fod-down MASTERS=1 SLAVES=2
```

Schema operations are separate:

```bash
make docker-deploy-schema-init MASTERS=1 SLAVES=2
make docker-deploy-schema-upgrade MASTERS=1 SLAVES=2
```

Destructive volume removal is guarded:

```bash
make docker-deploy-destroy MASTERS=1 SLAVES=2 DESTROY=YES
```

## FUSE lifecycle

The FOD container receives `/dev/fuse`, `CAP_SYS_ADMIN` and an `rshared` bind. AppArmor remains host-managed; on an AppArmor-enabled Docker host the startup guard can apply `apparmor=unconfined` for the FOD service.

A healthy host may show more than one propagated FUSE mount row. The deployment validates one filesystem identity by comparing the unique `MAJ:MIN` FUSE device on the host with the device seen inside the healthy container. Multiple rows with the same device identity are valid propagation views, not multiple FOD filesystems.

`docker-deploy-fod-down` stops/removes the container and removes propagated FUSE rows while preserving the underlying shared host bind.

## Persistent startup after reboot

For unattended boot install the systemd integration:

```bash
make docker-deploy-systemd-plan MASTERS=1 SLAVES=2
make docker-deploy-systemd-install MASTERS=1 SLAVES=2
```

The installer copies the minimal runtime to root-owned `/usr/local/libexec/fod-docker-deploy`; systemd does not execute the user-owned Git checkout. `/etc/fod/docker-deploy.env` records topology/paths/exact image but does not duplicate database passwords.

At boot the service restores the shared host mount, starts PostgreSQL and replicas, starts FOD, then executes smoke checks.

Operations:

```bash
make docker-deploy-systemd-status MASTERS=1 SLAVES=2
make docker-deploy-systemd-smoke MASTERS=1 SLAVES=2
make docker-deploy-systemd-restart MASTERS=1 SLAVES=2
```

See `docs/DOCKER_SYSTEMD.md` for the complete host-boot contract and reboot acceptance test. See `docs/DOCKER_FOD_INSTALL.md` for the FOD/FUSE layer. For day-2 operation and non-disruptive FOD upgrades, see `docs/OPERATIONS.md`.

## Policy checks

```bash
make test-cargo-lock-integrity
make test-docker-deploy-policy
make test-docker-fod-install-policy
make test-docker-deploy-systemd-policy
make test-docker-fod-client-policy
```

The PostgreSQL smoke check requires `SHOW block_size = 32768`, one writable primary, the requested number of recovery replicas and streaming replication. The FOD smoke check requires a healthy container, PostgreSQL reachability and matching FUSE filesystem identity across namespaces.
