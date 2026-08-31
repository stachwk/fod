# FOD Docker client installation

This document covers the persistent FOD/FUSE client that completes the Docker deployment.

The database deployment and the FOD client are intentionally separate layers:

- PostgreSQL: one writable primary plus `SLAVES=N` streaming replicas,
- FOD schema: initialized once in the primary database,
- FOD client: a persistent `fod-client` container that mounts the FOD filesystem through `/dev/fuse`.

The complete installation target combines all three layers:

```bash
make docker-deploy-install MASTERS=1 SLAVES=2
```

## FOD-only lifecycle

When PostgreSQL is already deployed, install or repair just the FOD container with:

```bash
make docker-deploy-fod-install MASTERS=1 SLAVES=2
```

Available FOD-container operations:

```bash
make docker-deploy-fod-plan MASTERS=1 SLAVES=2
make docker-deploy-fod-render MASTERS=1 SLAVES=2
make docker-deploy-fod-preflight MASTERS=1 SLAVES=2
make docker-deploy-fod-host-prepare MASTERS=1 SLAVES=2
make docker-deploy-fod-install MASTERS=1 SLAVES=2
make docker-deploy-fod-up MASTERS=1 SLAVES=2
make docker-deploy-fod-status MASTERS=1 SLAVES=2
make docker-deploy-fod-smoke MASTERS=1 SLAVES=2
make docker-deploy-fod-logs MASTERS=1 SLAVES=2
make docker-deploy-fod-shell MASTERS=1 SLAVES=2
make docker-deploy-fod-down MASTERS=1 SLAVES=2
```

Database schema operations use separate names so they cannot be confused with the FOD container lifecycle:

```bash
make docker-deploy-schema-init MASTERS=1 SLAVES=2
make docker-deploy-schema-upgrade MASTERS=1 SLAVES=2
```

## Default image

The persistent client uses:

```text
ghcr.io/stachwk/fod-client:3.4
```

Override it with:

```bash
FOD_DOCKER_DEPLOY_CLIENT_IMAGE=ghcr.io/stachwk/fod-client:3.4.1 \
  make docker-deploy-fod-install MASTERS=1 SLAVES=2
```

The image contains FOD/FUSE runtime plus PostgreSQL client/diagnostic utilities, but no PostgreSQL server.

## Host mountpoint

The default host-visible FOD mountpoint is:

```text
~/.local/share/fod/mount
```

Use another path with:

```bash
FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR=/srv/fod \
  make docker-deploy-fod-install MASTERS=1 SLAVES=2
```

Inside the container FOD is mounted at `/mnt/fod`.

The generated Compose overlay bind-mounts the host directory with `rshared` propagation. This is required so the FUSE submount created inside the container becomes visible on the Docker host.

## FUSE host preparation

The host needs `/dev/fuse`, Docker Compose v2 and a shared/rshared source mount.

Check without changing the host:

```bash
make docker-deploy-fod-preflight MASTERS=1 SLAVES=2
```

If the mountpoint is on a private mount, prepare it explicitly:

```bash
make docker-deploy-fod-host-prepare MASTERS=1 SLAVES=2
```

That target performs the equivalent of:

```bash
sudo mount --bind ~/.local/share/fod/mount ~/.local/share/fod/mount
sudo mount --make-rshared ~/.local/share/fod/mount
```

This preparation is a host mount-namespace setting and normally does not survive reboot. For an unattended final installation, configure the equivalent persistent systemd mount/unit or host mount policy.

## Container privileges

The FOD service receives only the privileges needed for the FUSE mount:

```text
/dev/fuse
CAP_SYS_ADMIN
```

It is not configured as a fully privileged container.

The service receives the generated `fod-container.ini` read-only and connects to PostgreSQL through the deployment Docker network using service DNS (`primary`, `replica1`, ...).

## Installation order

`docker-deploy-install` performs the final order:

1. render PostgreSQL topology and secrets,
2. pull/start PostgreSQL 32K primary and replicas,
3. wait for healthy `block_size=32768` nodes,
4. initialize the FOD schema if missing,
5. render the FOD Compose overlay,
6. verify `/dev/fuse` and host mount propagation,
7. start the persistent FOD container,
8. wait until the FUSE mount is healthy,
9. run PostgreSQL and FOD smoke checks.

The FOD-only installer is idempotent with respect to the schema: if the FOD schema already exists, schema initialization is skipped by the existing deployment logic.

## Generated files

The database deployment state remains under:

```text
~/.local/state/fod/docker-deploy/
```

The FOD layer adds:

```text
compose-fod.yml
```

The Compose overlay references the existing generated files:

```text
compose.yml
postgres.env
fod-container.ini
```

Secrets remain outside the repository.

## Diagnostics

Status:

```bash
make docker-deploy-status MASTERS=1 SLAVES=2
make docker-deploy-fod-status MASTERS=1 SLAVES=2
```

FOD logs:

```bash
make docker-deploy-fod-logs MASTERS=1 SLAVES=2
```

Interactive shell with `psql`, `pg_isready`, `pg_dump`, FOD binaries and the mounted filesystem:

```bash
make docker-deploy-fod-shell MASTERS=1 SLAVES=2
```

Smoke test:

```bash
make docker-deploy-fod-smoke MASTERS=1 SLAVES=2
```

The FOD smoke check verifies that `/mnt/fod` is a mount inside the container and that the same container can reach the PostgreSQL primary.

## Policy tests

Run:

```bash
make test-docker-deploy-policy
make test-docker-fod-install-policy
```

The FOD install policy checks the generated client image, `/dev/fuse`, `SYS_ADMIN`, `rshared` propagation, read-only FOD configuration and the public Make target contract.
