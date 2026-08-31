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
make docker-deploy-fod-diagnostics MASTERS=1 SLAVES=2
make docker-deploy-fod-down MASTERS=1 SLAVES=2
```

Database schema operations use separate names so they cannot be confused with the FOD container lifecycle:

```bash
make docker-deploy-schema-init MASTERS=1 SLAVES=2
make docker-deploy-schema-upgrade MASTERS=1 SLAVES=2
```

## Default image

The deployment uses the series alias:

```text
ghcr.io/stachwk/fod-client:3.4
```

The corrected FUSE/AppArmor-aware container revision for FOD 3.4.1 is:

```text
ghcr.io/stachwk/fod-client:3.4.1-fuse1
```

Publishing that revision updates `:3.4` to the same image without overwriting the historical `:3.4.1` tag.

Override the deployment image explicitly with:

```bash
FOD_DOCKER_DEPLOY_CLIENT_IMAGE=ghcr.io/stachwk/fod-client:3.4.1-fuse1 \
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

## Container privileges and AppArmor

The FOD service receives the FUSE-related privileges:

```text
/dev/fuse
CAP_SYS_ADMIN
rshared mount propagation
```

It is not configured as a fully privileged container.

AppArmor is host-managed. The image cannot load or relax the host AppArmor profile itself. On an AppArmor-enabled Docker host the startup guard defaults to an `apparmor=unconfined` runtime override for the FOD service because Docker's default profile can block the mount operations required by FUSE.

Override guard behavior with:

```bash
FOD_DOCKER_DEPLOY_FOD_APPARMOR=auto       make docker-deploy-fod-up MASTERS=1 SLAVES=2
FOD_DOCKER_DEPLOY_FOD_APPARMOR=unconfined make docker-deploy-fod-up MASTERS=1 SLAVES=2
FOD_DOCKER_DEPLOY_FOD_APPARMOR=default    make docker-deploy-fod-up MASTERS=1 SLAVES=2
```

`default` should only be used when the host Docker/AppArmor policy already permits the required FUSE mount operations. A future dedicated host-loaded AppArmor profile can replace `unconfined` without changing the image contract.

The `3.4.1-fuse1` image contains `fod-container-preflight`. Before `mount.fod` or `fod-rust-fuse`, the image entrypoint checks:

- `/dev/fuse` is present as a character device,
- `CAP_SYS_ADMIN` is present,
- Docker's restrictive `docker-default` AppArmor profile is not active,
- `rshared` propagation is available or emits a warning when it cannot be verified.

This makes privilege/AppArmor failures explicit in container logs instead of surfacing as an opaque FUSE mount failure.

The service receives the generated `fod-container.ini` read-only and connects to PostgreSQL through the deployment Docker network using service DNS (`primary`, `replica1`, ...).

## Installation order

`docker-deploy-install` performs the final order:

1. render PostgreSQL topology and secrets,
2. pull/start PostgreSQL 32K primary and replicas,
3. wait for healthy `block_size=32768` nodes,
4. initialize the FOD schema if missing,
5. render the FOD Compose overlay,
6. verify `/dev/fuse` and host mount propagation,
7. pull the current `fod-client:3.4` digest,
8. apply the AppArmor runtime override when needed,
9. start the persistent FOD container with a bounded startup timeout,
10. wait until the FUSE mount is healthy,
11. run PostgreSQL and FOD smoke checks.

The FOD-only installer is idempotent with respect to the schema: if the FOD schema already exists, schema initialization is skipped by the existing deployment logic.

## Generated files

The database deployment state remains under:

```text
~/.local/state/fod/docker-deploy/
```

The FOD layer adds:

```text
compose-fod.yml
compose-fod-runtime.yml
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

Startup diagnostics, including Docker state, `/dev/fuse`, mount propagation and AppArmor:

```bash
make docker-deploy-fod-diagnostics MASTERS=1 SLAVES=2
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

## Publishing the corrected client image

Validate and build:

```bash
make test-docker-fod-client-policy
make docker-fod-client-build
```

Publish to GHCR after `docker login ghcr.io`:

```bash
make docker-fod-client-publish
```

Expected tags:

```text
ghcr.io/stachwk/fod-client:3.4.1-fuse1
ghcr.io/stachwk/fod-client:3.4
```

`latest` is intentionally not created unless explicitly enabled.

## Policy tests

Run:

```bash
make test-docker-fod-client-policy
make test-docker-deploy-policy
make test-docker-fod-install-policy
```

The policies check the generated client image contract, `/dev/fuse`, `SYS_ADMIN`, AppArmor metadata/preflight, `rshared` propagation, bounded startup diagnostics, read-only FOD configuration and the public Make target contract.
