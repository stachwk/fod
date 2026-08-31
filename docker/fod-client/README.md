# FOD client container

This image contains the FOD client/runtime only. It does **not** contain a PostgreSQL server or PostgreSQL command-line tools.

The final image is based on `debian:bookworm-slim`. FOD is built as the normal Debian package and then installed into the runtime stage so Debian resolves the exact shared-library dependencies detected by `dpkg-shlibdeps`. PostgreSQL support is therefore limited to the `libpq` client runtime required by FOD.

Expected FOD commands in the image:

- `fod-bootstrap`
- `fod-change` / `fod.change`
- `fod-indexer`
- `fod-monitor`
- `fod-rust-fuse`
- `mkfs.fod`
- `mount.fod`

The image build and publisher explicitly verify that `postgres`, `initdb`, `pg_ctl`, and `psql` are absent.

## Build

```bash
make fod-client-build
```

Default local image/tag:

```text
ghcr.io/stachwk/fod-client:<FOD version>
```

For FOD 3.4.1 this is `ghcr.io/stachwk/fod-client:3.4.1`; a `:3.4` series alias is also created. `:latest` is disabled by default.

## Publish to GHCR

After Docker is logged in to `ghcr.io`:

```bash
make fod-client-publish
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
