# FOD PostgreSQL 32K container image

The FOD PostgreSQL image is PostgreSQL rebuilt with a compile-time table-page block size (`BLCKSZ`) of 32 KiB. The image is intended for controlled FOD deployments and performance experiments where FOD itself uses a 32 KiB logical block size.

## Canonical image name

Default registry and repository:

```text
ghcr.io/stachwk/postgres-16-fod-32k
```

The repository name is lowercase for compatibility with container registries. The exact PostgreSQL minor version is carried in the tag.

Current version:

```text
ghcr.io/stachwk/postgres-16-fod-32k:16.15
```

Optional major-version alias:

```text
ghcr.io/stachwk/postgres-16-fod-32k:16
```

`latest` is not published by default because it is less reproducible.

## Build and verify locally

```bash
bash scripts/publish_postgres_fod_32k.sh
```

The script verifies both:

- PostgreSQL version equals the requested version;
- `postgres -C block_size` returns `32768` after `initdb`.

No image is pushed unless `FOD_CONTAINER_PUSH=1` is set.

## Publish to GitHub Container Registry

GitHub Container Registry uses `ghcr.io`. Authenticate first with a GitHub personal access token that has package write permission:

```bash
echo "$CR_PAT" | docker login ghcr.io -u stachwk --password-stdin
```

Then publish:

```bash
FOD_CONTAINER_PUSH=1 \
bash scripts/publish_postgres_fod_32k.sh
```

The default publish creates:

```text
ghcr.io/stachwk/postgres-16-fod-32k:16.15
ghcr.io/stachwk/postgres-16-fod-32k:16
```

To also publish `latest`:

```bash
FOD_CONTAINER_PUSH=1 \
FOD_CONTAINER_TAG_LATEST=1 \
bash scripts/publish_postgres_fod_32k.sh
```

## Publish to Docker Hub

Create a Docker Hub repository named, for example:

```text
postgres-16-fod-32k
```

Authenticate:

```bash
docker login docker.io
```

Then publish with your Docker Hub namespace:

```bash
FOD_CONTAINER_REGISTRY=docker.io \
FOD_CONTAINER_NAMESPACE=<dockerhub-user> \
FOD_CONTAINER_PUSH=1 \
bash scripts/publish_postgres_fod_32k.sh
```

## Publishing another PostgreSQL 16 minor version

The image is intentionally version-pinned. To publish a newer PostgreSQL 16 minor release, set the exact version explicitly, for example:

```bash
FOD_POSTGRES_IMAGE_VERSION=16.16 \
FOD_CONTAINER_PUSH=1 \
bash scripts/publish_postgres_fod_32k.sh
```

The build uses `postgres:<version>-alpine` as the reference runtime and rebuilds the matching PostgreSQL source with `--with-blocksize=32`.

## Important compatibility note

A PostgreSQL cluster initialized with 32 KiB pages is not data-directory-compatible with a standard PostgreSQL build using 8 KiB pages. Always use a fresh `initdb`, separate volumes, or logical migration/dump-and-restore when moving between builds with different `BLCKSZ` values.
