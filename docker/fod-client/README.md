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

## Build

```bash
make docker-fod-client-build
```

Default local image/tag:

```text
ghcr.io/stachwk/fod-client:<FOD version>
```

For FOD 3.4.1 this is `ghcr.io/stachwk/fod-client:3.4.1`; a `:3.4` series alias is also created. `:latest` is disabled by default.

The PostgreSQL client major can be overridden when a future server generation requires it:

```bash
FOD_CLIENT_POSTGRES_MAJOR=16 make docker-fod-client-build
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

## Database diagnostics

Example connectivity check:

```bash
docker run --rm \
  ghcr.io/stachwk/fod-client:3.4.1 \
  pg_isready -h postgres-host -p 5432 -d foddbname
```

Example interactive SQL session:

```bash
docker run --rm -it \
  -e PGPASSWORD='secret' \
  ghcr.io/stachwk/fod-client:3.4.1 \
  psql -h postgres-host -p 5432 -U foduser -d foddbname
```

## FUSE runtime

A container that actually mounts FOD through FUSE needs access to the host FUSE device and the corresponding mount capability. A typical invocation is:

```bash
docker run --rm -it \
  --device /dev/fuse \
  --cap-add SYS_ADMIN \
  -v /path/on/host:/mnt/fod:rshared \
  ghcr.io/stachwk/fod-client:3.4.1 \
  fod-bootstrap --help
```

Exact FOD configuration/database parameters should be supplied in the same way as for the native FOD client. The PostgreSQL database remains an external service; it is never started inside this container.
