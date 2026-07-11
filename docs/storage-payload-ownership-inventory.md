# Storage Payload Ownership Inventory

## Scope

This inventory covers the Storage Engine v2 ownership follow-up for
`data_blocks`, `data_extents`, and `copy_block_crc`. It records the current use
of `id_file` before removing that compatibility-era ownership column.

## Current invariant

The logical owner of payload rows is already `data_object_id`:

- `files.data_object_id` selects the file payload;
- block reads join `files` to `data_blocks` by `data_object_id`;
- extent reads join `files` to `data_extents` by `data_object_id`;
- range uniqueness is keyed by `(data_object_id, _order)` or
  `(data_object_id, start_block)`;
- deduplication, full-object adoption, data-object swap, deferred cleanup, and
  object GC operate on `data_object_id`.

No runtime read requires `data_blocks.id_file`, `data_extents.id_file`, or
`copy_block_crc.id_file` to locate payload.

## Remaining `id_file` dependencies

### Schema

- `data_blocks.id_file` is non-null and references `files.id_file`.
- `data_extents.id_file` is non-null and references `files.id_file`.
- `copy_block_crc.id_file` is non-null, references `files.id_file`, and retains
  an `ON DELETE CASCADE` relationship even though its unique key uses the data
  object.
- Payload tables do not yet have explicit cascading foreign keys from
  `data_object_id` to `data_objects.id_data_object`.

These constraints make `id_file` a representative file pointer rather than
the payload owner. A shared object can have many files but each payload row can
store only one representative file ID.

### Persistence

- Block staging and direct block upserts carry `id_file` into every row and
  update it on conflict.
- Extent and CRC binary COPY encoders include `id_file` fields.
- Detaching a shared data object copies payload rows while replacing their
  representative `id_file`.
- Extent-to-block conversion supplies the destination file ID only because the
  target schema still requires it.

### Delete and cleanup

- `purge_primary_file()` searches for another file that references a shared
  object and rewrites all payload rows to that survivor before deleting the
  original file.
- Whole-object adoption and primary-file purge contain legacy
  `data_object_id = ... OR id_file = ...` deletion predicates.
- `fod-indexer cleanup-failed` rewrites all three payload tables to a survivor
  file when an imported data object remains shared.
- Immediate object cleanup explicitly deletes rows from all payload tables
  before deleting `data_objects` because object-level cascading ownership is
  not encoded in the schema.

### Diagnostics and documentation

- `scripts/perf/pg/data_blocks_semantics.sql` measures whether `id_file`
  behaves as a stale or representative owner.
- Historical EXPLAIN scripts include `id_file` in synthetic merge tables.
- `README.pl` still names the removed historical
  `data_blocks(id_file, _order)` index even though current uniqueness is object
  based.

## Required migration

The ownership cleanup should be one schema transition, without a permanent
dual-path compatibility branch:

1. Verify that every payload `data_object_id` resolves to `data_objects`.
2. Add cascading foreign keys from all payload tables to `data_objects`.
3. Drop the three payload-table `id_file` columns and their file foreign keys.
4. Remove `id_file` from staging tables, COPY encoders, upserts, detach copies,
   cleanup, purge, and replay classification.
5. Delete payloads by deleting the owning data object; retain explicit
   object-keyed deletes only where transaction ordering requires them.
6. Update diagnostics and documentation to describe object ownership.

The base schema and an upgrade migration must reach the same final shape.

## Verification gate

The migration must cover:

- fresh schema initialization and upgrade from schema version 16;
- block, extent, CRC, partial-tail, and extent-to-block conversion reads;
- full-object adoption and both source/destination purge orders;
- shared-object detach, hardlinks, and hardlink promotion;
- failed materialization cleanup with a surviving shared object;
- immediate and deferred object cleanup plus object GC;
- body and commit disconnect replay;
- remount durability and the full storage hot-path gate.
