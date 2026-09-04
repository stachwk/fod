# FOD Docker persistent startup with systemd

This unit is the persistent host-boot layer for the final Docker deployment. It restores the host `rshared` mount, starts the PostgreSQL 32K topology, starts the FOD/FUSE client, and finishes with smoke checks.

## Security model

The systemd unit runs as root because preparing mount propagation requires host mount privileges. It does **not** execute scripts from the user-owned Git checkout at boot.

`make docker-deploy-systemd-install` copies the minimal deployment runtime into the root-owned directory:

```text
/usr/local/libexec/fod-docker-deploy/
```

The installed runtime contains only the Docker deployment/FOD lifecycle scripts and PostgreSQL primary/replica bootstrap helpers required after boot.

Systemd configuration is stored in:

```text
/etc/fod/docker-deploy.env
```

That file contains topology, state paths, mount paths and the resolved exact FOD image tag. It does not duplicate PostgreSQL, replication or FOD schema passwords; those remain in the existing protected deployment state under `~/.local/state/fod/docker-deploy/`.

## Exact image selection

The normal Make interface derives the default FOD client image from `fod_version.txt`. FOD 3.4.8 therefore defaults to:

```text
ghcr.io/stachwk/fod-client:3.4.8
```

To install systemd with another already-published client build, pass:

```bash
make docker-deploy-systemd-install MASTERS=1 SLAVES=2 FOD_CLIENT_VERSION=3.4.5
```

The deployment image precedence is:

```text
1. FOD_DOCKER_DEPLOY_CLIENT_IMAGE=registry/path:tag
2. FOD_CLIENT_VERSION=X.Y.Z
3. fod_version.txt
```

The selected version is resolved to a full image and written to `/etc/fod/docker-deploy.env` as `FOD_DOCKER_DEPLOY_CLIENT_IMAGE`. A later reboot therefore keeps using that image even if the Git checkout advances to another release. The mutable `:3.4` tag remains a convenience alias only.

## Install

Inspect the intended unit without changing systemd:

```bash
make docker-deploy-systemd-plan MASTERS=1 SLAVES=2
make docker-deploy-systemd-render MASTERS=1 SLAVES=2
```

The same plan can verify an explicit client build before installation:

```bash
make docker-deploy-systemd-plan MASTERS=1 SLAVES=2 FOD_CLIENT_VERSION=3.4.5
```

Install, enable and start the unit:

```bash
make docker-deploy-systemd-install MASTERS=1 SLAVES=2
```

The installer uses `sudo` only for the root-owned runtime files, `/etc/fod` and systemd operations. Existing Docker volumes and deployment state are reused.

Set `FOD_DOCKER_DEPLOY_SYSTEMD_START_NOW=0` to install and enable the unit without starting it immediately.

## Boot order

The generated service declares:

```text
Requires=docker.service
After=docker.service network-online.target
PartOf=docker.service
ConditionPathExists=/dev/fuse
```

At start the root-owned boot helper performs:

1. `docker_fod_install.sh host-prepare` — restores the bind/rshared host mount for the current boot,
2. `docker_deploy.sh up` — starts primary and requested replicas and waits for the cluster,
3. `docker_fod_start_guard.sh start` — starts/reconciles the FOD FUSE client,
4. PostgreSQL smoke checks,
5. FOD/FUSE smoke checks including host/container FUSE identity validation.

At stop it removes the FOD container/mount first, then stops the PostgreSQL Compose topology. Persistent PostgreSQL volumes and generated state are retained.

## Operations

```bash
make docker-deploy-systemd-status MASTERS=1 SLAVES=2
make docker-deploy-systemd-smoke MASTERS=1 SLAVES=2
make docker-deploy-systemd-restart MASTERS=1 SLAVES=2
```

`docker-deploy-systemd-restart` is a full service restart: FOD is unmounted, PostgreSQL containers are stopped, then the complete deployment is brought back and smoke-tested.

To remove only persistent boot integration:

```bash
make docker-deploy-systemd-uninstall MASTERS=1 SLAVES=2
```

Uninstalling removes the unit, `/etc/fod/docker-deploy.env`, and the root-owned runtime copy. It deliberately preserves Docker volumes and the deployment state/secrets.

## Reinstall after a FOD upgrade

After pulling a newer FOD release, rerun the installer with either the repository-default client image or an explicitly selected published build:

```bash
make docker-deploy-systemd-install MASTERS=1 SLAVES=2
make docker-deploy-systemd-install MASTERS=1 SLAVES=2 FOD_CLIENT_VERSION=3.4.5
```

This refreshes the root-owned runtime copy and environment with the resolved exact image tag.

When the service is already active, reinstall uses `systemctl reload` rather
than a full restart. The reload runs the normal start/reconciliation path
without executing `ExecStop`, so an FOD client image upgrade does not stop or
recreate the PostgreSQL primary and replica containers.

`docker-deploy-systemd-restart` remains an explicit full-service restart and
does stop and recreate the running deployment containers while preserving
persistent volumes.

For the concrete before/after container-ID verification procedure, see `docs/OPERATIONS.md`.

## Validation

Policy checks without root/systemd changes:

```bash
make test-version
make test-cargo-lock-integrity
make test-docker-deploy-systemd-policy
```

After installation:

```bash
make docker-deploy-systemd-smoke MASTERS=1 SLAVES=2
```

The final acceptance test is a real host reboot. After reboot verify:

```bash
systemctl status fod-docker-deploy.service --no-pager
make docker-deploy-status MASTERS=1 SLAVES=2
make docker-deploy-smoke MASTERS=1 SLAVES=2
findmnt -T ~/.local/share/fod/mount -o ID,PARENT,MAJ:MIN,TARGET,SOURCE,FSTYPE,PROPAGATION
```

The host may show more than one propagated FUSE mount row. They are valid when they refer to one FUSE filesystem identity (`MAJ:MIN`) and match the FUSE device seen inside the healthy FOD container.
