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
    "version": "FOD 3.2.x"
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

### `plan list`

```text
fod-indexer plan list --limit N [--cursor CURSOR] [--status STATUS]
```

Lists stored plans without creating, refreshing, or modifying them. Results use
`plan_id DESC` keyset pagination.

### `plan show --id ID`

Reads one stored import plan. It does not run scan, hash, or planning again.

### `report duplicates --id ID`

Reads one existing duplicate set. It does not rebuild duplicate metadata.

### `duplicate-set list`

```text
fod-indexer duplicate-set list [--limit N] [--cursor CURSOR]
```

Lists existing rows from `index_duplicate_sets` without invoking scan, hash, or
the duplicate-set rebuild. Results use `duplicate_set_id ASC` keyset pagination.
Each item contains the stable duplicate-set id, hash algorithm, full hash, file
size, file count, total bytes, and creation/update timestamps. The consistency
model is `live`.

### `file list`

```text
fod-indexer file list [--limit N] [--cursor CURSOR]
    [--source SOURCE]
    [--file-kind KIND]
    [--scan-status STATUS]
    [--hash-status STATUS]
```

Lists existing file records in stable `file_id ASC` order. The returned
`next_cursor` is the last visible `file_id` when another page exists.

### `file search`

```text
fod-indexer file search [QUERY]
    [--path TEXT]
    [--name TEXT]
    [--source SOURCE]
    [--extension EXT]
    [--file-kind KIND]
    [--scan-status STATUS]
    [--hash-status STATUS]
    [--min-size BYTES]
    [--max-size BYTES]
    [--mtime-from NS]
    [--mtime-to NS]
    [--limit N]
    [--cursor CURSOR]
```

Searches existing index rows without scanning or modifying sources. At least one
search filter is required. `QUERY` performs a case-insensitive match against the
indexed relative path and source name. `--path` and `--name` are
case-insensitive containment filters; source, kind, and status filters are exact.
The extension may be passed as `pdf` or `.pdf` and is normalized to lowercase.

### `file show --id ID`

Reads one file record by stable `file_id`. It joins source metadata and optional
hash metadata but does not read file contents.

File catalogue records expose fields owned by FOD:

- stable `file_id`,
- source id, name, kind, and root,
- relative `path` and derived full `source_path`,
- filename and extension derived from the path,
- size, modification time, inode, and device when available,
- file kind and scan status,
- source-changed status,
- hash algorithm, full hash, and hash status when available,
- scan-run provenance,
- record creation and update timestamps.

The file catalogue consistency model is explicitly `live`. FOD does not claim a
frozen `snapshot_id` until the database stores immutable catalogue snapshots.
MIME, extracted text, OCR state, extractor versions, embeddings, and AI metadata
remain outside `fod-indexer`.

### `file read --id ID`

```text
fod-indexer file read --id ID [--offset N] [--length N]
```

The command resolves a stable `file_id`, rejects unsafe relative paths and paths
that resolve outside the registered source root, and opens the source file only
for reading. Before returning data it checks:

- indexed file kind and scan status,
- the stored `source_changed` and hash retry state,
- size, modification time, inode, and device,
- metadata observed during hashing when present,
- the stored full SHA-256 hash, or the stored partial SHA-256 sample when a full
  hash is not available.

It computes the current full SHA-256 for provenance and rechecks the open file and
its source path after reading. Missing, inaccessible, replaced, concurrently
changed, hash-mismatched, or escaped source files fail with an explicit
`file_read_*` error before bytes are emitted.

Omitting `--length` reads from `--offset` to EOF. A requested range extending
past EOF is shortened to the available bytes; an offset beyond EOF is rejected.

With default text output, exact bytes are written to stdout and provenance is
written to stderr, so stdout can be redirected directly to a file or pipe. With
`--output json`, the versioned response contains:

- source, path, and scan-run provenance,
- indexed and hash-observed metadata,
- validation basis and current observed SHA-256,
- requested and returned byte-range information,
- `encoding: "base64"` and `data_base64`.

The operation is strictly read-only. It does not update the source-changed flag,
hash rows, scan runs, duplicate metadata, or plans. It does not perform MIME
classification, text extraction, OCR, embeddings, or AI processing.

## Command that is not strictly read-only

The no-id form:

```bash
fod-indexer report duplicates --limit N
```

currently calls the duplicate-set rebuild before returning the report. It is a
refreshing read over derived state, not a strictly read-only query. `msfind`
should use `duplicate-set list` instead when it requires a no-write contract.
