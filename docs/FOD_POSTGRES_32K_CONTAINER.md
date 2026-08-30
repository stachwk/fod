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

## Extension bundle

The image is intentionally built as a reusable PostgreSQL 16/FOD base so that common extensions do not require rebuilding PostgreSQL for every experiment or deployment.

All PostgreSQL 16 `contrib` modules that are enabled by the configured dependencies are compiled and installed against the same custom PostgreSQL headers and `BLCKSZ=32K`. This includes, among others:

- `amcheck`, `auto_explain`, `bloom`;
- `btree_gin`, `btree_gist`;
- `citext`, `cube`, `dblink`, `earthdistance`;
- `file_fdw`, `fuzzystrmatch`, `hstore`, `intarray`, `ltree`;
- `pg_buffercache`, `pg_freespacemap`, `pg_prewarm`;
- `pg_stat_statements`, `pg_visibility`, `pgcrypto`, `pgrowlocks`, `pgstattuple`;
- `postgres_fdw`, `seg`, `tablefunc`, `tcn`, `unaccent`;
- `uuid-ossp` and `xml2`.

The image also builds the following external extensions from pinned source releases against the custom `pg_config`:

| Extension | Pinned version | Purpose |
| --- | ---: | --- |
| `vector` / pgvector | 0.8.6 | vector types and ANN indexes |
| `pgaudit` | 16.1 | detailed audit logging for PostgreSQL 16 |
| `pg_cron` | 1.6.7 | database job scheduler |
| `pg_repack` | 1.5.3 | online table/index repacking |
| `hypopg` | 1.4.3 | hypothetical indexes |
| `pg_stat_kcache` | 2.3.2 | kernel CPU/I/O statistics per query |
| `pg_hint_plan` | 1.6.2 | planner hints for PostgreSQL 16 |

The build writes the complete extension-name list to:

```text
/opt/postgresql-custom/share/fod/available-extensions.txt
```

To inspect it without starting PostgreSQL:

```bash
docker run --rm --entrypoint cat \
  ghcr.io/stachwk/postgres-16-fod-32k:16.15 \
  /opt/postgresql-custom/share/fod/available-extensions.txt
```

The extensions are **available**, not automatically enabled in every database. Use `CREATE EXTENSION ...` where required. Extensions that use hooks/background workers, such as `pgaudit`, `pg_cron`, `pg_stat_kcache` or `pg_hint_plan`, may additionally require an appropriate `shared_preload_libraries` configuration. The image does not preload all of them by default because that would add runtime overhead and could change benchmark behavior.

### Why extensions are compiled from source

Do not add Alpine/PGDG binary C-extension packages built for a standard PostgreSQL 8 KiB server. C extensions in this image are compiled against the custom PostgreSQL 32 KiB headers so compile-time page-layout assumptions remain consistent.

The Dockerfile deliberately puts PostgreSQL/contrib and each external extension in separate build layers. If a later extension changes or fails to compile, Docker can reuse previously completed layers instead of recompiling PostgreSQL and every earlier extension.

Heavy extension platforms such as PostGIS, TimescaleDB and Citus are deliberately not part of this base bundle yet. They bring large dependency trees and deserve separate compatibility/testing decisions for the nonstandard PostgreSQL block size.

## PostgreSQL build features

To make the reusable image less restricted, PostgreSQL is built with support for:

- ICU;
- OpenSSL;
- XML and XSLT;
- LZ4 and Zstd;
- UUID (`e2fs`/libuuid);
- readline and zlib.

## Multilingual locale support

The runtime image installs `musl-locales`, `musl-locales-lang` and the full ICU data package (`icu-data-full`). `MUSL_LOCPATH` is configured so Alpine's musl locale files are visible to libc-based applications.

The image keeps the neutral UTF-8 default:

```text
LANG=C.UTF-8
```

`LC_ALL` is deliberately **not** pinned. A deployment can therefore override `LANG` or individual `LC_*` variables when required.

The build verifies a representative system-locale set including:

```text
C.UTF-8
en_US.UTF-8
en_GB.UTF-8
de_DE.UTF-8
fr_FR.UTF-8
es_ES.UTF-8
it_IT.UTF-8
pt_BR.UTF-8
ru_RU.UTF-8
cs_CZ.UTF-8
nl_NL.UTF-8
sv_SE.UTF-8
```

For PostgreSQL collations, ICU is the preferred broad-language mechanism. Full ICU data provides substantially wider language/territory coverage than musl system locales. The publish verification explicitly creates temporary PostgreSQL ICU clusters for:

```text
pl-PL
cs-CZ
de-DE
en-US
fr-FR
es-ES
it-IT
pt-BR
ru-RU
uk-UA
hu-HU
ja-JP
zh-CN
ko-KR
tr-TR
```

This verifies Polish as well as Central/Western European, Cyrillic and East Asian ICU locale support. Other ICU locales remain available through the full ICU data set even when they are not individually smoke-tested during image publishing.

Example: initialize a PostgreSQL cluster using Polish ICU collation rules:

```bash
initdb --locale-provider=icu --icu-locale=pl-PL -D /var/lib/postgresql/data/pgdata
```

For ordinary Docker usage the standard PostgreSQL entrypoint still initializes the database according to the environment/options supplied by the deployment; the image does not force Polish or any other language as a global default.

## Build and verify locally

```bash
bash scripts/publish_postgres_fod_32k.sh
```

The script verifies:

- PostgreSQL version equals the requested version;
- `postgres -C block_size` returns `32768` after `initdb`;
- the bundled extension manifest contains at least the expected number of extensions;
- required built-in and external extensions are present;
- the minimum set of musl system locales is present;
- representative ICU PostgreSQL locales, including `pl-PL`, can initialize successfully.

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
