# FOD client container

The FOD client image contains the FOD runtime/FUSE frontend plus PostgreSQL client and diagnostic tools. It does **not** contain the PostgreSQL server.

## Release tags

For normal repository-driven builds, the image is tagged with the exact FOD source release from `fod_version.txt`. For FOD 3.4.10 the immutable source-build image is:

```text
ghcr.io/stachwk/fod-client:3.4.10
```

Publishing also refreshes the convenience series alias:

```text
ghcr.io/stachwk/fod-client:3.4
```

The deployment Make interface may independently select an already-published runtime build with `FOD_CLIENT_VERSION`, for example:

```bash
make docker-deploy-fod-install MASTERS=1 SLAVES=2 FOD_CLIENT_VERSION=3.4.5
```

Deployment resolution is `FOD_DOCKER_DEPLOY_CLIENT_IMAGE` first, then `FOD_CLIENT_VERSION`, then the repository release from `fod_version.txt`.

`FOD_CLIENT_VERSION` is intentionally a deployment selector only. Normal `make docker-fod-client-build` and `make docker-fod-client-publish` continue to use the source release from `fod_version.txt`; selecting an older runtime build does not retag current source binaries as that older release. The existing advanced `FOD_CLIENT_IMAGE_VERSION` variable remains the explicit build/publish tag override when such a workflow is deliberately required.

Historical note: `ghcr.io/stachwk/fod-client:3.4.1-fuse1` was the transitional FUSE/AppArmor container revision before the container fixes were folded into the 3.4 release line. That historical tag is not overwritten.

`latest` remains disabled by default.

## Build and validation

```bash
make test-docker-fod-client-policy
make docker-fod-client-build
```

After `docker login ghcr.io`:

```bash
make docker-fod-client-publish
```

The image build verifies that FOD binaries, `libpq`, PostgreSQL 16 client tools, `fuse3`, `capsh`, and `findmnt` are available, while server-side commands such as `postgres`, `initdb`, and `pg_ctl` remain absent.

## Runtime contract

The image requires:

- `/dev/fuse`,
- `CAP_SYS_ADMIN`,
- `rshared` bind propagation for a host-visible FUSE mount,
- host AppArmor policy that permits FUSE mount operations.

The image carries these labels:

```text
org.fod.fuse.required-device=/dev/fuse
org.fod.fuse.required-capability=SYS_ADMIN
org.fod.fuse.mount-propagation=rshared
org.fod.apparmor.policy=host-managed
org.fod.apparmor.recommended=unconfined-or-custom-fod-profile
```

AppArmor is controlled by the Docker host. On hosts using Docker's restrictive default profile, the FOD deployment startup guard uses `apparmor=unconfined` unless a compatible host policy is selected explicitly.

Image-only preflight:

```bash
docker run --rm \
  ghcr.io/stachwk/fod-client:3.4.10 \
  fod-container-preflight --image-only
```

Runtime preflight:

```bash
docker run --rm \
  --device /dev/fuse \
  --cap-add SYS_ADMIN \
  --security-opt apparmor=unconfined \
  ghcr.io/stachwk/fod-client:3.4.10 \
  fod-container-preflight --runtime
```

A host-visible FOD mount additionally needs a shared source mount, for example:

```bash
docker run --rm -it \
  --device /dev/fuse \
  --cap-add SYS_ADMIN \
  --security-opt apparmor=unconfined \
  -v /path/on/host:/mnt/fod:rshared \
  -v /path/to/fod.ini:/etc/fod/fod.ini:ro \
  ghcr.io/stachwk/fod-client:3.4.10 \
  mount.fod none /mnt/fod -o ini=/etc/fod/fod.ini,role=auto
```

The entrypoint runs `fod-container-preflight --runtime` before `mount.fod` or `fod-rust-fuse`, so missing FUSE privileges or an incompatible AppArmor profile fail with a direct diagnostic.

## PostgreSQL tools

The runtime includes `psql`, `pg_isready`, `pg_dump`, `pg_restore`, `createdb`, `dropdb`, `reindexdb`, and `vacuumdb`. The database remains an external service and is never started inside this image.
