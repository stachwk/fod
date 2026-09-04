# FOD Docker client installation

The persistent FOD/FUSE client completes the Docker deployment on top of PostgreSQL 32K and the FOD schema.

## Exact release image

The normal Make interface selects the exact `fod_version.txt` release by default. For FOD 3.4.9:

```text
ghcr.io/stachwk/fod-client:3.4.9
```

The series alias `ghcr.io/stachwk/fod-client:3.4` is retained for convenience but is not the default final deployment image. The historical transitional image `3.4.1-fuse1` remains immutable.

To run an already-published client build independently of the checkout version, use:

```bash
make docker-deploy-fod-install MASTERS=1 SLAVES=2 FOD_CLIENT_VERSION=3.4.5
```

The resolution order is:

```text
1. FOD_DOCKER_DEPLOY_CLIENT_IMAGE=registry/path:tag
2. FOD_CLIENT_VERSION=X.Y.Z
3. fod_version.txt
```

Use the full-image override when a registry/repository other than the normal FOD client repository is required:

```bash
FOD_DOCKER_DEPLOY_CLIENT_IMAGE=registry.example/fod-client:validated-build \
  make docker-deploy-fod-install MASTERS=1 SLAVES=2
```

`FOD_CLIENT_VERSION` changes only which published runtime image deployment consumes. It does not change the repository/source version and does not alter the default tag used when building/publishing an image from the current checkout.

## FOD-only lifecycle

When the database topology is already present:

```bash
make docker-deploy-fod-host-prepare MASTERS=1 SLAVES=2
make docker-deploy-fod-install MASTERS=1 SLAVES=2
make docker-deploy-fod-status MASTERS=1 SLAVES=2
make docker-deploy-fod-smoke MASTERS=1 SLAVES=2
```

The selector may be supplied to FOD-only lifecycle targets as well:

```bash
make docker-deploy-fod-plan MASTERS=1 SLAVES=2 FOD_CLIENT_VERSION=3.4.5
make docker-deploy-fod-up MASTERS=1 SLAVES=2 FOD_CLIENT_VERSION=3.4.5
```

Other operations:

```bash
make docker-deploy-fod-plan MASTERS=1 SLAVES=2
make docker-deploy-fod-render MASTERS=1 SLAVES=2
make docker-deploy-fod-preflight MASTERS=1 SLAVES=2
make docker-deploy-fod-up MASTERS=1 SLAVES=2
make docker-deploy-fod-logs MASTERS=1 SLAVES=2
make docker-deploy-fod-diagnostics MASTERS=1 SLAVES=2
make docker-deploy-fod-shell MASTERS=1 SLAVES=2
make docker-deploy-fod-down MASTERS=1 SLAVES=2
```

Schema lifecycle remains separate:

```bash
make docker-deploy-schema-init MASTERS=1 SLAVES=2
make docker-deploy-schema-upgrade MASTERS=1 SLAVES=2
```

## Host/FUSE requirements

Default host-visible mountpoint:

```text
~/.local/share/fod/mount
```

Container mountpoint:

```text
/mnt/fod
```

The FOD service receives:

```text
/dev/fuse
CAP_SYS_ADMIN
rshared bind propagation
```

Use another host path with `FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR`.

Check host readiness:

```bash
make docker-deploy-fod-preflight MASTERS=1 SLAVES=2
```

Prepare the shared host mount for the current boot:

```bash
make docker-deploy-fod-host-prepare MASTERS=1 SLAVES=2
```

The equivalent host operations are a bind of the mountpoint onto itself followed by `mount --make-rshared`.

## AppArmor

AppArmor is host-managed. The image includes `fod-container-preflight`; the deployment guard uses `apparmor=unconfined` automatically on an AppArmor-enabled Docker host unless configured otherwise.

```bash
FOD_DOCKER_DEPLOY_FOD_APPARMOR=auto       make docker-deploy-fod-up MASTERS=1 SLAVES=2
FOD_DOCKER_DEPLOY_FOD_APPARMOR=unconfined make docker-deploy-fod-up MASTERS=1 SLAVES=2
FOD_DOCKER_DEPLOY_FOD_APPARMOR=default    make docker-deploy-fod-up MASTERS=1 SLAVES=2
```

The client entrypoint checks `/dev/fuse`, `CAP_SYS_ADMIN` and AppArmor before invoking `mount.fod`/`fod-rust-fuse`.

## FUSE identity

With `rshared` propagation the host may show multiple FUSE mount rows for the same filesystem. FOD validates the unique kernel filesystem identity instead of requiring one row:

```bash
findmnt -T ~/.local/share/fod/mount \
  -o ID,PARENT,MAJ:MIN,TARGET,SOURCE,FSTYPE,PROPAGATION
```

Healthy propagated rows have one unique FUSE `MAJ:MIN`, matching the device seen at `/mnt/fod` inside the healthy container.

`docker-deploy-fod-down` removes the FOD container and propagated FUSE views while preserving the underlying shared host bind.

## Publishing FOD 3.4.9

```bash
make test-version
make test-docker-fod-client-policy
make docker-fod-client-build
```

After `docker login ghcr.io`:

```bash
make docker-fod-client-publish
```

The normal Make path builds/publishes the current source release:

```text
ghcr.io/stachwk/fod-client:3.4.9
ghcr.io/stachwk/fod-client:3.4   # refreshed series alias
```

`FOD_CLIENT_VERSION` is a deployment selector and does not change this default build/publish tag. The existing advanced `FOD_CLIENT_IMAGE_VERSION` build variable remains separate for deliberate image-build workflows.

`latest` remains disabled by default.

## Persistent reboot startup

The one-boot `host-prepare` mount namespace state is made persistent operationally by the FOD systemd deployment unit, which repeats host preparation on every service start:

```bash
make docker-deploy-systemd-install MASTERS=1 SLAVES=2
```

To persist a selected published FOD client build:

```bash
make docker-deploy-systemd-install MASTERS=1 SLAVES=2 FOD_CLIENT_VERSION=3.4.5
```

See `docs/DOCKER_SYSTEMD.md`. For day-2 operation and upgrade verification, see `docs/OPERATIONS.md`.

## Policy checks

```bash
make test-cargo-lock-integrity
make test-docker-fod-client-policy
make test-docker-deploy-policy
make test-docker-fod-install-policy
make test-docker-deploy-systemd-policy
```
