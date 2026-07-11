# ADR: Defer the Object Segment Manifest

## Status

Accepted on 2026-07-11. The manifest is deferred, not rejected permanently.

## Context

Storage Engine v2 reduced large sequential payload persistence to bounded,
opt-in extents and added append-only replacement objects. The remaining design
question was whether FOD also needed this physical model immediately:

```text
files
    -> data_objects
    -> object_segments
    -> payload_chunks
```

The decision had to separate exact whole-file copies from partial or chunked
copies. Earlier measurements were misleading because the FUSE crate advertised
only ABI 7.17. The kernel therefore did not dispatch `FUSE_COPY_FILE_RANGE` to
the implemented callback and used a generic read/write fallback instead.

With FUSE ABI 7.31 enabled, a clean copy of an entire source file into an empty
destination can attach the source `data_object_id` directly. This works for
both block-backed and extent-backed objects and does not depend on optional
changed-block deduplication.

## Evidence

The local three-run 64 MiB measurements were collected on 2026-07-11 from a
worktree based on commit `16bf0f8`:

- whole-object adoption averaged `1219.23 MiB/s` for a block-backed source and
  `1282.86 MiB/s` for an extent-backed source;
- the destination shared the source object and added no destination payload
  rows;
- chunked 4 MiB requests averaged `17.74 MiB/s` on the block path and
  `26.68 MiB/s` with 1 MiB extents;
- chunked extent runs averaged about `7.71 MB` of WAL, compared with about
  `13.22 MB` for the block runs;
- a partial block patch of an extent-backed object now converts the existing
  extent payload to block rows inside the same PostgreSQL transaction before
  applying the dirty range. This preserves bytes outside the patch and then
  removes the extent rows so extent-first reads cannot shadow fresh blocks.

Artifacts:

- `artifacts/perf/16bf0f8/lt7300-storage-whole-object-adoption-20260711T080000Z-storage-extent-summary.md`
- `artifacts/perf/16bf0f8/lt7300-storage-abi31-chunked-copy-fixed-20260711T090000Z-storage-extent-summary.md`

## Decision

Do not introduce `object_segments` or `payload_chunks` now.

Keep the current physical model:

```text
files
    -> data_objects
       -> data_blocks
       -> data_extents
       -> copy_block_crc
```

Use direct data-object adoption only when all of these conditions hold:

- source offset is zero;
- destination offset is zero;
- the requested range covers the complete source;
- the destination is empty;
- neither file has pending dirty state.

Keep block persistence as the default. Keep extents opt-in. Partial and chunked
copies remain ordinary copy operations and may downgrade an extent-backed
destination to block storage when a patch must preserve existing bytes.

## Consequences

The current design avoids a new manifest traversal, chunk reference-counting
model, garbage collector, schema family, and replay contract. Exact whole-file
copies already achieve the primary zero-payload-copy goal.

The current downgrade has write amplification when a small patch touches a
large extent-backed object, because existing extents are expanded to block
rows. This is a deliberate correctness-first boundary rather than a silent
hybrid representation.

Reopen the manifest decision only when repeated real workloads show at least
one of these unmet requirements:

- aligned partial clone or range-copy reuse is a material workload;
- extent-to-block conversion dominates WAL, latency, or relation growth;
- chunk-level deduplication materially exceeds whole-object reuse;
- compression, snapshots, or object versioning require immutable chunks;
- object garbage collection becomes more complex than a manifest would make it.

Any reopened design must include replay-safe reference updates, remount and CRC
coverage, partial-write semantics, and local plus remote PostgreSQL benchmarks.
