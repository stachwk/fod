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

The delivered foundation does not yet provide a complete read-only catalogue
API. The remaining items below are open.

## P0: capabilities and response versioning

- [x] Add `fod-indexer capabilities` with producer version, API schema version,
  available and planned commands, filters, sort order, pagination, limits, and
  consistency.
- [x] Add `schema_version` and `producer` to JSON responses without moving the
  existing payload fields.
- [ ] Add regression coverage to the normal local test suite after the branch
  implementation passes `cargo test -p fod-rust-indexer` and
  `cargo check --workspace --locked`.

See [`fod-indexer-read-api.md`](fod-indexer-read-api.md).

## P0: list stored import plans

Add a strictly read-only command such as:

```text
fod-indexer plan list --limit N [--cursor CURSOR] [--status STATUS]
```

The response should contain:

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
total, when cheap
```

The command must not create, refresh, or modify plans.

## P0: list existing duplicate sets without rebuild

The current no-id `report duplicates` path rebuilds derived duplicate metadata.
Add a command such as:

```text
fod-indexer duplicate-set list --limit N [--cursor CURSOR]
```

It must only read existing duplicate rows. `msfind` must not call scan, hash, or
a duplicate rebuild to discover valid set ids.

## P0: list or search the whole file index

Add strictly read-only commands that return stable indexed-file records:

```text
fod-indexer file list ...
fod-indexer file search ...
fod-indexer file show --id ID
```

The response should include:

```text
schema_version
producer
consistency: live
items
next_cursor
total, when cheap
sort
```

Each item should contain fields already owned by FOD:

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
file_kind
scan_status
source_changed
hash_algorithm
full_hash
hash_status
scan_run_id
```

Required filters include name/path, source, extension, file kind, scan status,
hash status, size range, modification-time range, and deterministic keyset
pagination.

Do not claim a frozen `snapshot_id` until the database stores immutable catalogue
snapshots. The first contract is explicitly a live view ordered by stable
`file_id`.

## P1: revalidated source-byte read

Add a read-only byte-range command by `file_id`, with indexed identity,
provenance, and an explicit changed/missing-source error.

Text extraction, MIME classification, extractor versions, OCR, embeddings, and
AI classification remain owned by `msfind`; they should not be added to the FOD
index tables merely to satisfy this command.

## Constraints

- All new catalogue and plan-list operations are strictly read-only.
- `msfind` does not call `scan`, `hash`, `clean`, `materialize`, or refreshing
  duplicate reports while serving read-only queries.
- FOD does not depend on `msfind` code.
- `msfind` does not duplicate FOD SQL or indexing logic.
