# ADR: Defer the Object Segment Manifest

## Status

Accepted on 2026-07-11 for the manifest decision.

The physical-storage portion of the original ADR was superseded by
FOD 3.2.71-3.2.73. The manifest remains deferred, but the old extent runtime
model is no longer valid.

Current storage authority:

- [`../block-only-performance-plan.md`](../block-only-performance-plan.md)

Historical Storage Engine v2 context:

- [`../storage-engine-v2-plan.md`](../storage-engine-v2-plan.md)

## Current decision

Do not introduce `object_segments` or `payload_chunks` now.

Keep one canonical production payload representation:

```text
files
    -> data_objects
       -> data_blocks
       -> copy_block_crc
```

Do not restore `data_extents`, extent planning, extent-first reads or
extent-to-block runtime conversion as part of the deferred manifest decision.

Whole-object adoption remains the preferred optimization for an exact clean
full-file copy when the existing correctness conditions are satisfied. Partial
and chunked copies continue through the current block-only copy/write semantics.

## Historical context

The original ADR was written while Storage Engine v2 still evaluated bounded,
opt-in extents and append-only replacement objects.

At that time the design question was whether FOD also needed this physical
model immediately:

```text
files
    -> data_objects
    -> object_segments
    -> payload_chunks
```

The decision had to separate exact whole-file copies from partial or chunked
copies. Earlier measurements were misleading because the FUSE crate advertised
only ABI 7.17, so the kernel did not dispatch `FUSE_COPY_FILE_RANGE` to the
implemented callback and used a generic read/write fallback instead.

With FUSE ABI 7.31 enabled, a clean copy of an entire source file into an empty
destination could attach the source `data_object_id` directly. That semantic
result remains relevant: exact whole-object adoption can avoid destination
payload copying without introducing a segment manifest.

## Historical evidence

The local three-run 64 MiB measurements collected on 2026-07-11 from a worktree
based on commit `16bf0f8` showed:

- whole-object adoption averaged `1219.23 MiB/s` for a block-backed source and
  `1282.86 MiB/s` for an extent-backed source;
- the destination shared the source object and added no destination payload
  rows;
- chunked 4 MiB requests averaged `17.74 MiB/s` on the block path and
  `26.68 MiB/s` with the then-experimental 1 MiB extents;
- the old extent PoC reduced WAL in that specific historical comparison;
- partial patches of extent-backed objects required correctness-preserving
  materialization to blocks.

These numbers are historical evidence only. They do not justify restoring the
extent runtime path.

Later profiling showed that the dual representation caused severe conversion
amplification: a 128 MiB sequential write triggered an extent-to-block expansion
that consumed about `13.65 s` inside a roughly `16.08 s` persist operation.
FOD 3.2.71-3.2.73 therefore retired extent persistence, migrated legacy rows to
`data_blocks` and removed `data_extents` from the production schema.

## Consequences

The manifest remains deferred because the current object model already solves
the primary exact whole-file reuse case without adding:

- a segment traversal layer;
- chunk reference counting;
- another payload table family;
- another garbage-collection contract;
- another replay/accounting contract.

The earlier argument that extents could serve as the intermediate physical
format is superseded. Current performance work must improve the block-only path
instead of recreating a second representation.

## Conditions for reopening the manifest decision

Reopen the manifest only when repeated real workloads demonstrate a capability
that the canonical `data_blocks` + whole-object adoption model cannot satisfy
efficiently enough, for example:

- aligned partial clone or range-copy reuse is a material workload;
- chunk-level deduplication materially exceeds whole-object reuse;
- compression requires independently addressable immutable payload chunks;
- snapshots or object versioning require persistent immutable segment history;
- garbage collection or copy-on-write sharing becomes measurably simpler and
  cheaper with a manifest;
- a measured workload proves block-row granularity itself is the limiting
  architecture after current PostgreSQL transaction/concurrency bottlenecks are
  removed.

Do not reopen the ADR merely because an old extent benchmark was faster on one
sequential workload.

Any reopened design must include:

- replay-safe reference updates;
- database-wide quota/accounting semantics;
- sparse and partial-write semantics;
- remount durability;
- CRC/copy correctness;
- shared-object and copy-on-write behavior;
- local and remote PostgreSQL benchmarks;
- comparison against the then-current block-only baseline.

## Relationship to current performance work

The current measured priority is not a manifest or physical-format redesign.
FOD 3.2.74 removed the repeated read-side allocation COUNT, and concurrent write
profiling then exposed the global transaction-scoped quota advisory lock as the
next artificial serialization point.

Current implementation order is defined by:

1. `docs/block-only-performance-plan.md`;
2. `docs/quota-lock-concurrency-plan.md`.

A manifest decision must not interrupt that sequence unless new measurements
show a structural block-only limitation that cannot be solved at the current
transaction/query layer.
