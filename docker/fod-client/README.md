# FOD client container

The FOD client image contains the FOD runtime/FUSE frontend plus PostgreSQL client and diagnostic tools. It does **not** contain the PostgreSQL server.

## Release tags

For normal repository-driven builds, the image is tagged with the exact FOD source release from `fod_version.txt`. Resolve the tag from the current checkout instead of copying a stale example:

```bash
FOD_CLIENT_RELEASE="$(cat fod_version.txt)"
printf 'ghcr.io/stachwk/fod-client:%s\n' "${FOD_CLIENT_RELEASE}"
```

Publishing also refreshes the convenience series alias (for example `3.4`):

```bash
printf 'ghcr.io/stachwk/fod-client:%s\n' "${FOD_CLIENT_RELEASE%.*}"
```

The exact source-release tag is created by the image build and is available from the registry only after publishing.

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

The image builds upstream libfuse 3.18.3 from its SHA-256-pinned release archive and installs the resulting Debian package in both build and runtime stages. This avoids Bookworm's older packaged libfuse while preserving package dependency metadata. The image build verifies that FOD binaries, `libpq`, PostgreSQL 16 client tools, `fuse3`, `capsh`, and `findmnt` are available, while server-side commands such as `postgres`, `initdb`, and `pg_ctl` remain absent.

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
FOD_CLIENT_RELEASE="$(cat fod_version.txt)"
docker run --rm \
  "ghcr.io/stachwk/fod-client:${FOD_CLIENT_RELEASE}" \
  fod-container-preflight --image-only
```

Runtime preflight:

```bash
FOD_CLIENT_RELEASE="$(cat fod_version.txt)"
docker run --rm \
  --device /dev/fuse \
  --cap-add SYS_ADMIN \
  --security-opt apparmor=unconfined \
  "ghcr.io/stachwk/fod-client:${FOD_CLIENT_RELEASE}" \
  fod-container-preflight --runtime
```

A host-visible FOD mount additionally needs a shared source mount, for example:

```bash
FOD_CLIENT_RELEASE="$(cat fod_version.txt)"
docker run --rm -it \
  --device /dev/fuse \
  --cap-add SYS_ADMIN \
  --security-opt apparmor=unconfined \
  -v /path/on/host:/mnt/fod:rshared \
  -v /path/to/fod.ini:/etc/fod/fod.ini:ro \
  "ghcr.io/stachwk/fod-client:${FOD_CLIENT_RELEASE}" \
  mount.fod none /mnt/fod -o ini=/etc/fod/fod.ini,role=auto
```

The entrypoint runs `fod-container-preflight --runtime` before `mount.fod` or `fod-rust-fuse`, so missing FUSE privileges or an incompatible AppArmor profile fail with a direct diagnostic.

## PostgreSQL tools

The runtime includes `psql`, `pg_isready`, `pg_dump`, `pg_restore`, `createdb`, `dropdb`, `reindexdb`, and `vacuumdb`. The database remains an external service and is never started inside this image.
