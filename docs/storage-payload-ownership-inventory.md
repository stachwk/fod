# Storage Payload Ownership

## Status

The Storage Engine v2 ownership transition is complete in schema version 17.
`data_objects.id_data_object` is the exclusive owner key for rows in
`data_blocks`, `data_extents`, and `copy_block_crc`. The representative
payload-table `id_file` columns have been removed without a runtime
compatibility branch.

The implementation was completed and validated on 2026-07-11 from a worktree
based on commit `a23bfbb`.

## Current invariant

- `files.data_object_id` selects the payload visible through a file.
- `files.data_object_id` has a non-cascading foreign key to `data_objects`, so
  PostgreSQL rejects deletion of an object still attached to a file.
- `data_blocks.data_object_id`, `data_extents.data_object_id`, and
  `copy_block_crc.data_object_id` are non-null foreign keys with
  `ON DELETE CASCADE`.
- Block uniqueness is `(data_object_id, _order)`.
- Extent uniqueness is `(data_object_id, start_block)`.
- CRC uniqueness and its primary key are `(data_object_id, _order)`.
- Object deletion is the single cleanup operation for an object's block,
  extent, and CRC payload rows.
- Shared files reference one object directly; payload rows do not identify or
  depend on a representative file.

## Delivered transition

### Schema

`migrations/base_schema.sql` creates the final object-owned shape directly.
`migrations/0017_data_object_payload_ownership.sql` upgrades version 16 in one
transaction. It locks the affected tables, rejects orphan object references,
removes the three `id_file` columns and their foreign keys, adds cascading
object foreign keys, and restores the object-keyed CRC primary key.

The Rust schema tool recognizes version 17 in its migration manifest. If the
version row is missing from a complete latest schema, `upgrade` restores the
row only after checking every required relation, the final payload columns,
the ownership foreign keys, the CRC primary key, and the latest migration
marker. It does not infer the latest version from a partial shape.

### Persistence

- Block staging and direct block upserts carry only `data_object_id`, block
  order, payload, and optional CRC.
- Extent and CRC binary COPY encoders match the final PostgreSQL column types
  and no longer encode `id_file`.
- Shared-object detach copies payload rows from one object key to another.
- Extent-to-block conversion preserves the same object key.
- Replay classification matches the object-owned insert and update forms.

### Delete and cleanup

- Immediate full-overwrite cleanup deletes the old object and relies on its
  cascading payload ownership.
- Deferred cleanup marks the old object unreferenced; object GC later deletes
  the object and receives the same cascading behavior.
- `purge_primary_file()` deletes the file before deleting an exclusive object,
  or only decrements the shared object's reference count.
- Whole-object adoption deletes an unreferenced destination object directly.
- `fod-indexer cleanup-failed` deletes imported files before exclusive objects
  and updates only the reference count for shared surviving objects.
- No cleanup path searches for a survivor file to rewrite payload ownership.

### Diagnostics

`scripts/perf/pg/data_blocks_semantics.sql` now reports the final payload
columns, object foreign keys, orphan counts, reference-count mismatches, shared
objects, and hybrid block/extent objects. The merge EXPLAIN scripts reproduce
the current object-keyed staging and conflict update shape without an
`id_file` column.

## Pre-migration inventory

Before version 17, reads and uniqueness already used `data_object_id`, but the
three payload tables also required an `id_file` value. That value behaved as a
representative pointer for a shared object rather than as its owner. It forced
block staging, binary COPY, shared detach, file purge, object adoption, and
failed materialization cleanup to carry or rewrite a file ID that was not part
of payload identity.

The old shape also lacked cascading foreign keys from payload tables to
`data_objects`, so immediate cleanup issued explicit deletes against each
payload table. The transition removed these dependencies together rather than
retaining parallel old/new runtime paths.

## Verification

The 2026-07-11 local gate based on commit `a23bfbb` covered:

- fresh schema initialization;
- recovery of a missing latest-version row after structural verification;
- data-preserving upgrades from schema versions 1 and 16;
- object-level cascading deletion for block, extent, and CRC rows;
- block, extent, CRC, partial-tail, and extent-to-block conversion behavior;
- whole-object adoption and source/destination purge order;
- shared-object detach, hardlinks, and hardlink promotion;
- failed materialization cleanup with an external shared-object survivor;
- immediate and deferred replacement cleanup;
- body and commit disconnect replay;
- remount durability, persist chunking, unlink-after-write, and large-copy
  object adoption;
- the full hot-path suite, the ordinary FUSE suite, sequential/mixed/random fio
  checks, and the required profiled strace run.

The post-test diagnostic reported zero orphan files, blocks, extents, and CRC
rows, zero reference-count mismatches, and zero hybrid block/extent objects.
Two unreferenced objects were expected leftovers from the deferred-cleanup
tests and remained eligible for the existing object-GC path.
