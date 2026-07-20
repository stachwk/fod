# msfind requests for fod-indexer

This file collects functionality that `msfind` wants from `fod-indexer` so
`msfind` can stay thin and reuse the shared FOD indexing core instead of
building a separate indexing engine.

New requests should describe reusable capabilities that belong in
`fod-indexer`. `msfind` must not copy FOD SQL, scanning rules, hashing logic, or
materialization behavior.

## Delivered foundation

- [x] Provide machine-readable output for source, scan, hash, duplicate, plan,
  clean, materialize, and cleanup operations.
- [x] Expose source kind, policy, capability metadata, and browsable roots.
- [x] Export one stored plan or duplicate set by id without rerunning the main
  pipeline.
- [x] Document the bounded retry boundary.
- [x] Keep text extraction, classification, embeddings, OCR, and other AI work
  outside the shared indexing core.
- [x] Add `fod-indexer capabilities` with producer version, API schema version,
  available and planned commands, filters, sort order, pagination, limits, and
  consistency.
- [x] Add `schema_version` and `producer` to JSON responses without moving the
  existing payload fields.
- [x] Cover capability, plan-list, duplicate-set-list, file-list, file-search,
  file-show, file-read, row parsing, path safety, source-change detection,
  filter normalization, and range validation in the local Rust test suite.

See [`fod-indexer-read-api.md`](fod-indexer-read-api.md).

## Delivered P0: list stored import plans

The strictly read-only command is available:

```text
fod-indexer plan list --limit N [--cursor CURSOR] [--status STATUS]
```

It returns:

```text
schema_version
producer
items:
  plan_id
  status
  source_filter
  dry_run
  created_at
  updated_at
  entry_count
next_cursor
total
```

The command does not create, refresh, or modify plans.

## Delivered P0: list existing duplicate sets without rebuild

The strictly read-only command is available:

```text
fod-indexer duplicate-set list --limit N [--cursor CURSOR]
```

It reads existing rows from `index_duplicate_sets` only. It does not invoke
scan, hash, or the duplicate-set rebuild. The response contains:

```text
schema_version
producer
consistency: live
sort: duplicate_set_id ASC
items:
  duplicate_set_id
  hash_algorithm
  full_hash_hex
  file_size
  file_count
  total_bytes
  created_at
  updated_at
next_cursor
total
```

`msfind` can use this command to discover valid set ids without calling the
refreshing no-id `report duplicates` path.

## Delivered P0: list or search the whole file index

The following strictly read-only commands are available:

```text
fod-indexer file list ...
fod-indexer file search ...
fod-indexer file show --id ID
```

List and search responses include:

```text
schema_version
producer
consistency: live
items
next_cursor
total
sort: file_id ASC
```

`file show` returns the same record shape below one `item` field.

Each record contains fields owned by FOD:

```text
file_id
source_id
source_name
source_kind
source_root
path
source_path
name
extension
size
mtime_ns
inode
device
file_kind
scan_status
source_changed
hash_algorithm
full_hash_hex
hash_status
scan_run_id
created_at
updated_at
```

Available search filters include query text, path, basename, source, extension,
file kind, scan status, hash status, size range, modification-time range, limit,
and deterministic keyset cursor pagination.

The first contract is explicitly a live view ordered by stable `file_id`; it does
not claim a frozen `snapshot_id`. MIME, extracted text, extractor versions, OCR,
embeddings, and AI metadata remain owned by `msfind`.

## Delivered P1: revalidated source-byte read

The strictly read-only command is available:

```text
fod-indexer file read --id ID [--offset N] [--length N]
```

It resolves the stable `file_id`, validates the registered source path, checks
scan and hash metadata, detects missing, inaccessible, replaced, or changed
source files, and returns no bytes when validation fails.

The JSON response contains:

```text
schema_version
producer
consistency: revalidated-source-read
provenance:
  file_id
  source_id
  source_name
  source_kind
  source_root
  path
  source_path
  resolved_source_path
  scan_run_id
  indexed_size
  indexed_mtime_ns
  indexed_inode
  indexed_device
  hash_algorithm
  partial_hash_hex
  full_hash_hex
  hash_status
  hash_observed_size
  hash_observed_mtime_ns
  hash_observed_inode
  hash_observed_device
validation:
  status
  basis
  metadata_match
  indexed_hash_match
  observed_hash_algorithm
  observed_full_hash_hex
  observed_size
  observed_mtime_ns
  observed_inode
  observed_device
range:
  offset
  requested_length
  returned_length
  end_offset
  eof
encoding: base64
data_base64
```

Text mode writes exact source bytes to stdout and provenance to stderr. JSON
transports the requested bytes as Base64. Omitting `--length` reads from the
offset to EOF.

The command does not update any index row. Text extraction, MIME classification,
extractor versions, OCR, embeddings, and AI classification remain owned by
`msfind`.

## Constraints

- All catalogue, duplicate-set-list, plan-list, and file-read operations are
  strictly read-only.
- `msfind` does not call `scan`, `hash`, `clean`, `materialize`, or refreshing
  duplicate reports while serving read-only queries.
- FOD does not depend on `msfind` code.
- `msfind` does not duplicate FOD SQL or indexing logic.
