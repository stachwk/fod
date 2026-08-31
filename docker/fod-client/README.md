# FOD client container

This image contains the FOD client/runtime plus PostgreSQL client and diagnostic tools. It does **not** contain a PostgreSQL server.

The final image is based on `debian:bookworm-slim`. FOD is built as the normal Debian package and then installed into the runtime stage so Debian resolves the exact shared-library dependencies detected by `dpkg-shlibdeps`. PostgreSQL access uses `libpq` plus the PostgreSQL 16 client package from the PGDG repository.

Expected FOD commands in the image:

- `fod-bootstrap`
- `fod-change` / `fod.change`
- `fod-indexer`
- `fod-monitor`
- `fod-rust-fuse`
- `mkfs.fod`
- `mount.fod`
- `fod-container-preflight`

PostgreSQL diagnostic/client commands include:

- `psql`
- `pg_isready`
- `pg_dump`
- `pg_restore`
- `createdb`
- `dropdb`
- `reindexdb`
- `vacuumdb`

The image build and publisher explicitly verify that the PostgreSQL server-side commands `postgres`, `initdb`, and `pg_ctl` are absent. `psql` is intentionally present and its major version is checked against the configured client major (16 by default).

## Image revision

The FOD binaries remain version `3.4.1`, while the corrected container/FUSE runtime is published as:

```text
ghcr.io/stachwk/fod-client:3.4.1-fuse1
```

The `fuse1` suffix identifies the container runtime revision. Publishing also updates the `:3.4` series alias to this image. The historical `:3.4.1` image is not overwritten, and `:latest` remains disabled by default.

## Build

```bash
make docker-fod-client-build
```

Default local image/tag:

```text
ghcr.io/stachwk/fod-client:3.4.1-fuse1
```

The PostgreSQL client major can be overridden when a future server generation requires it:

```bash
FOD_CLIENT_POSTGRES_MAJOR=16 make docker-fod-client-build
```

The container revision can also be overridden explicitly:

```bash
FOD_CLIENT_IMAGE_VARIANT=fuse2 make docker-fod-client-build
```

## Validate container policy

```bash
make test-docker-fod-client-policy
```

## Publish to GHCR

After Docker is logged in to `ghcr.io`:

```bash
make docker-fod-client-publish
```

This publishes the immutable revision tag and updates the series alias:

```text
ghcr.io/stachwk/fod-client:3.4.1-fuse1
ghcr.io/stachwk/fod-client:3.4
```

## Database diagnostics

Example connectivity check:

```bash
docker run --rm \
  ghcr.io/stachwk/fod-client:3.4.1-fuse1 \
  pg_isready -h postgres-host -p 5432 -d foddbname
```

Example interactive SQL session:

```bash
docker run --rm -it \
  -e PGPASSWORD='secret' \
  ghcr.io/stachwk/fod-client:3.4.1-fuse1 \
  psql -h postgres-host -p 5432 -U foduser -d foddbname
```

## FUSE and AppArmor runtime

AppArmor is enforced by the **Docker host**, not by files inside the image. The image therefore does not pretend that it can load or change the host AppArmor profile. Instead it contains a runtime preflight and labels that state the required host configuration:

- device `/dev/fuse`,
- capability `SYS_ADMIN`,
- `rshared` mount propagation for a host-visible FUSE mount,
- AppArmor `unconfined` or a host-loaded custom profile that explicitly permits the FUSE mount operations.

The following labels are included in the image:

```text
org.fod.fuse.required-device=/dev/fuse
org.fod.fuse.required-capability=SYS_ADMIN
org.fod.fuse.mount-propagation=rshared
org.fod.apparmor.policy=host-managed
org.fod.apparmor.recommended=unconfined-or-custom-fod-profile
```

`fod-container-preflight` verifies the image prerequisites and, at runtime, checks `/dev/fuse`, `CAP_SYS_ADMIN` and the active AppArmor profile.

Image-only validation:

```bash
docker run --rm \
  ghcr.io/stachwk/fod-client:3.4.1-fuse1 \
  fod-container-preflight --image-only
```

Runtime validation:

```bash
docker run --rm \
  --device /dev/fuse \
  --cap-add SYS_ADMIN \
  --security-opt apparmor=unconfined \
  ghcr.io/stachwk/fod-client:3.4.1-fuse1 \
  fod-container-preflight --runtime
```

A container that actually mounts FOD through FUSE typically needs:

```bash
docker run --rm -it \
  --device /dev/fuse \
  --cap-add SYS_ADMIN \
  --security-opt apparmor=unconfined \
  -v /path/on/host:/mnt/fod:rshared \
  ghcr.io/stachwk/fod-client:3.4.1-fuse1 \
  mount.fod none /mnt/fod -o ini=/etc/fod/fod.ini,role=auto
```

The image entrypoint automatically runs `fod-container-preflight --runtime` before `mount.fod` and `fod-rust-fuse`, so missing FUSE privileges or the default restrictive Docker AppArmor profile fail with a direct diagnostic instead of an opaque mount error.

Exact FOD configuration/database parameters should be supplied in the same way as for the native FOD client. The PostgreSQL database remains an external service; it is never started inside this container.
