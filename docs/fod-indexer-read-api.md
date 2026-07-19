# fod-indexer read-only API

This document defines the integration boundary used by read-only consumers such
as `msfind`. The consumer must use `fod-indexer` commands instead of copying SQL
queries or indexing rules.

## Ownership boundary

`fod-indexer` owns:

- source registration and source metadata,
- scan metadata,
- file identity and source paths,
- partial and full hashes,
- duplicate-set metadata,
- import plans,
- materialization and cleanup.

`msfind` remains responsible for:

- MIME recognition beyond values explicitly stored by FOD,
- text extraction and extractor versions,
- OCR,
- classification,
- embeddings and AI processing.

FOD does not depend on `msfind`, and `msfind` must not call FOD index tables
through direct SQL.

## Versioned JSON contract

Every `--output json` response uses API schema version `1` and adds these fields
at the top level:

```json
{
  "schema_version": 1,
  "producer": {
    "name": "fod-indexer",
    "version": "3.2.x"
  }
}
```

The layout is deliberately additive. Existing payload fields remain at the top
level instead of moving below a new `data` key, so consumers that ignore unknown
fields keep working. New consumers must validate `schema_version` before relying
on a response shape.

`schema_version` is the JSON API schema version. It is not the PostgreSQL FOD
schema version and it is not the FOD release version.

## Capabilities discovery

Use:

```bash
fod-indexer capabilities
fod-indexer --output json capabilities
```

The command:

- is read-only,
- does not initialize the FOD indexer configuration,
- does not connect to PostgreSQL,
- identifies the producer and API schema,
- declares the required database schema version,
- lists available and planned commands,
- identifies filters, sorting, pagination, limits, and consistency,
- explicitly marks commands that rebuild derived state.

Consumers should inspect this document instead of inferring support from help
text or attempting example identifiers.

## Currently available read-only commands

### `capabilities`

Static offline capability discovery.

### `source list`

Reads registered sources or browses a filesystem root. Supported filters are
`--kind` and `--path`. Registered sources are ordered by kind, name, and source
id; browsed directories are ordered by path.

### `plan show --id ID`

Reads one stored import plan. It does not run scan, hash, or planning again.

### `report duplicates --id ID`

Reads one existing duplicate set. It does not rebuild duplicate metadata.

## Command that is not strictly read-only

The no-id form:

```bash
fod-indexer report duplicates --limit N
```

currently calls the duplicate-set rebuild before returning the report. It is a
refreshing read over derived state, not a strictly read-only query. `msfind`
should not call it when it requires a no-write contract.

A future `duplicate-set list` command will read existing rows without rebuilding
them.

## Planned P0 commands

### Import-plan listing

```text
fod-indexer plan list --limit N [--cursor CURSOR] [--status STATUS]
```

The command will be read-only and return plan id, status, source filter, dry-run
flag, timestamps, entry count, a deterministic cursor, and a total only when it
is cheap.

### File catalogue queries

```text
fod-indexer file list ...
fod-indexer file search ...
fod-indexer file show --id ID
```

The first version will expose fields already owned by FOD:

- stable `file_id`,
- source id, name, kind, and root,
- relative and full source paths,
- filename and extension derived from the path,
- size and modification time,
- file kind and scan status,
- source-changed status,
- hash algorithm, full hash, and hash status when available,
- scan-run provenance.

Planned filters include source, path, name, extension, file kind, scan status,
hash status, size range, modification-time range, cursor, and limit.

Pagination will be deterministic keyset pagination. Until the schema stores
immutable catalogue snapshots, the consistency model is explicitly `live`, not
a frozen `snapshot_id` contract.

## Planned P1 source-byte read

A future command may provide a revalidated byte-range read by `file_id`:

```text
fod-indexer file read --id ID [--offset N] [--length N]
```

It will return source bytes and provenance only after checking that the source
still matches the indexed identity. It will not perform text extraction, OCR,
MIME classification, embeddings, or AI processing.
