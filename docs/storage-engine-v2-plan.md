# Storage Engine v2 Plan

## Status and scope

Storage Engine v2 is an incremental redesign of the FOD storage hot path. It
does not replace the runtime, the FUSE API, or the default block-storage path.
The existing extension boundary remains:

```text
WriteState
    -> PersistPlan
       -> Blocks
       -> Extents
    -> PersistExecutionPlan
    -> DbRepo
    -> PostgreSQL
```

The redesign is limited to write buffering, persistence planning, physical
payload representation, extent persistence, read assembly, and storage GC.
The logical filesystem block size remains 4 KiB. The physical persistence unit
for large sequential writes should become a bounded extent or segment in the
64 KiB to 4 MiB range.

The extent path remains opt-in through `enable_extents = true` until correctness
tests and repeated benchmarks prove that it is safe and useful. The block path
must remain the default and must not read extent-only tuning values.

Implementation status:

- direction documented in commit `3fe5590`;
- bounded planning and `extent_target_bytes` added in commit `93f1ab9`;
- bounded payload-row enforcement and peak-payload profiling added in commit
  `38af786`;
- the repeated local and QNAP extent-size matrices passed for the 64 MiB core
  workload;
- Phase A is complete and Phase B may begin while extents remain opt-in and the
  block path remains the default;
- Phase B1 now buffers new empty-file writes from offset zero in a bounded
  `SequentialSegmentState`, with gaps, backward writes, existing-file writes,
  and state merging downgraded to `BlockOverlay`;
- Phase B2 moves eligible bounded segment buffers directly into
  `PersistExtentRow`, restores them after a failed repository call, and records
  segment-mode diagnostics without changing the default block path;
- Phase C classifies persistence semantics and routes eligible full sequential
  payloads through one replay-confirmed append-only data-object transaction;
- the append-only transaction passed disconnect replay, shared-object,
  hardlink, CRC, cleanup-policy, remount, and mounted FUSE regression coverage;
- FUSE ABI 7.31 now exposes the implemented `copy_file_range` callback, and an
  exact clean whole-file copy into an empty destination adopts the source data
  object without copying payload rows;
- chunked 4 MiB copy requests remain payload copies, but the corrected extent
  path is both data-safe and faster than the block baseline in the repeated
  local 64 MiB matrix;
- Phase D is closed by `docs/adr/storage-object-segment-manifest.md`: a segment
  manifest is deferred until a measured partial-clone, patch-amplification,
  chunk-dedupe, snapshot, or compression workload requires it;
- the payload ownership inventory is recorded in
  `docs/storage-payload-ownership-inventory.md` before the schema migration
  removes representative `id_file` columns;
- extents remain opt-in because mixed and random workloads still regress.

## Original problem and remaining copy issue

The block path splits a large sequential stream into 4 KiB allocations and
persists thousands of PostgreSQL rows. Before Phase A, the extent proof of
concept avoided those physical rows but coalesced a full contiguous file into
one extent and rebuilt one payload `Vec` proportional to the whole file. A
64 MiB write could therefore become one 64 MiB `PersistExtentRow`.

Phase A bounds each physical extent payload. Phase B1 buffers eligible new-file
writes as bounded segments. Phase B2 removes the compatibility conversion at
flush: owned segment vectors now become extent rows directly, so bounded
segment payloads no longer coexist with a reconstructed 4 KiB block map. On a
repository error, ownership is moved back into `SequentialSegmentState` before
the FUSE operation returns `EIO`.

The earlier large-copy result came from a FUSE build that advertised ABI 7.17,
so the kernel never dispatched `FUSE_COPY_FILE_RANGE` to the implemented
callback. ABI 7.31 makes the callback reachable. An exact whole-file request
now adopts the source data object, while chunked requests still read and write
payload. The corrected three-run 64 MiB chunked matrix averaged `26.68 MiB/s`
with 1 MiB extents versus `17.74 MiB/s` on blocks. Partial destination patches
are made safe by converting existing extent rows to blocks inside the write
transaction before applying the dirty range.

## Measured bottlenecks

Existing FOD profiles show that the 64 MiB block path is dominated by
server-side staging `COPY` plus the `data_blocks` insert/merge. The first local
baseline measured about 2259 ms of PostgreSQL execution in those statements
against 3.892 s of workload time. Later 64 MiB profiles remained insert-heavy:
32768 `data_blocks` rows, no target-table updates, and visible conflict-index
lookup cost.

Client-side `COPY` send-buffer tuning did not create a stable local winner.
Local matrix results stayed around 18.01-18.38 MiB/s, and three repeated
`default` versus 4 MiB buffer samples overlapped. A single QNAP matrix improved
from 2.46 MiB/s to 3.18 MiB/s, but it still needs repeatability evidence before
any default change.

These results justify changing physical row and payload shape before spending
more effort on fillfactor, extra `ON CONFLICT` predicates, or client-side
`PQputCopyData` micro-tuning.

## Current storage paths

### Block path

```text
write stream
    -> 4 KiB Vec values
    -> WriteState.blocks BTreeMap
    -> PersistBlockPlan
    -> binary COPY staging
    -> data_blocks insert/merge
```

This path owns partial writes, random writes, sparse writes, mixed writes,
truncate behavior, CRC behavior, and the current safe fallback. Storage Engine
v2 must not weaken those semantics.

### Bounded extent path

```text
full contiguous dirty block set
    -> bounded extent plan
    -> bounded payload Vec values
    -> PersistExtentRow values
    -> data_extents
```

The planner selects this path only for full-file sequential coverage while
`enable_extents = true`. Non-contiguous writes fall back to block storage.

## Phase A: bounded extents

### A1. Bounded planning

Add a planner with the following contract:

```rust
pub struct BoundedExtentPlanner {
    pub target_bytes: u64,
    pub max_bytes: u64,
}
```

It must coalesce the logical block input according to the existing contract,
then split every contiguous range into bounded extents. No output extent may
exceed `max_bytes`; the final extent preserves the remaining block count.

Add opt-in runtime tuning:

```text
FOD_EXTENT_TARGET_BYTES
```

The extent proof-of-concept default is 1 MiB. Initial test values are 64 KiB,
256 KiB, 1 MiB, and 4 MiB. The value only affects planning when
`enable_extents = true`.

Do not add merge, overlap resolution, or split-on-patch semantics in this step.

### A2. Bounded execution

`prepare_persist_extent_rows_from_extent_ranges()` must build one bounded
payload for each bounded range. For a 64 MiB write with a 1 MiB target, the
expected result is 64 rows rather than one row.

Add an I/O profile diagnostic for the largest payload prepared during a run.
The measured value must never exceed the configured maximum extent size.

### A3. Benchmark matrix

Add `profile-storage-extent-size-matrix` and compare:

- the default block path;
- extents at 64 KiB, 256 KiB, 1 MiB, and 4 MiB.

Use sequential write/read, large copy, fio sequential, fio mixed, fio random
mixed, and remount durability workloads. Capture elapsed time, throughput,
`FOD_PROFILE_IO`, PostgreSQL statement and WAL counters, extent row count,
relation/index growth, dead tuples, maximum RSS, and largest extent payload.

Run at least three local repetitions. Run at least three QNAP repetitions when
the backend is available. Do not change a default from one sample.

Phase A succeeds only if bounded extents improve sequential write or large copy,
preserve correctness and remount durability, avoid catastrophic read/RSS
regressions, and materially reduce physical row count. Mixed and random paths
do not need to win because they retain the block fallback.

## Phase B: sequential segment builder

Phase B starts only after Phase A passes its benchmark gate.

Phase B1 introduced the state boundary. Eligible new empty-file writes enter
`SequentialSegmentState`; unsupported writes and merges downgrade to
`BlockOverlay`. Phase B2 now validates complete bounded segment coverage with
`PersistSegmentPlan` and moves the owned payload vectors directly into the
existing replay-safe extent transaction. No 4 KiB block map is rebuilt on this
path.

Introduce an internal write-payload state without changing the FUSE API:

```rust
enum WritePayloadState {
    BlockOverlay(BlockWriteState),
    SequentialSegments(SequentialSegmentState),
}
```

`BlockOverlay` preserves the current partial/random/sparse/mixed semantics.
`SequentialSegmentState` accepts writes while each offset equals the expected
next offset and builds bounded pending segments directly. A backward write,
random write, unsupported gap, or patch of existing content initially
downgrades the state to `BlockOverlay`.

The direct persist path should become:

```text
sequential stream
    -> bounded segment builder
    -> PersistSegmentPlan
    -> PersistExtentRow or future segment row
    -> binary COPY
```

Profile segment-mode entries, downgrades, payload bytes, segment count, payload
preparation CPU, memory copies, and maximum RSS. Phase B succeeds only when the
real large-copy path shows a measurable reduction in preparation cost or memory
pressure without correctness regressions.

The 2026-07-11 local gate based on commit `f0e0a1c` showed:

- 64 MiB large-file extent throughput of `98.81 MiB/s`, compared with the
  earlier bounded-extent baseline of `94.51 MiB/s`;
- segment-row preparation averaging `12.33 us`, compared with roughly `32 ms`
  for block-to-extent payload rebuilding;
- one segment-mode entry, zero downgrades, 64 bounded 1 MiB segments, and no
  block-row inserts per run;
- 64 MiB large-copy throughput of `14.07 MiB/s` on extents versus
  `18.50 MiB/s` on blocks, despite only `18 us` of segment preparation.

Phase B therefore closes the payload-rebuild bottleneck, but not the copy read
amplification. Extents remain opt-in. Phase C adds semantic classification and
append-only persistence, but the large-copy class must not be widened until
range-oriented extent reads or data-object adoption remove the repeated fetch
cost.

## Phase C: append-only new-object persistence

After the segment builder is stable, classify persistence semantics:

```rust
enum PersistWriteClass {
    NewObjectSequential,
    ExistingObjectPatch,
    TruncateOnly,
}
```

The classification boundary is implemented. `PersistExecutionPlan` now carries
`PersistWriteClass` instead of an indirect execution-only `truncate_only`
flag. Direct sequential segments are classified as `NewObjectSequential`,
ordinary block/extent payloads as `ExistingObjectPatch`, and a pending truncate
without payload as `TruncateOnly`. Runtime debug logs expose
`persist_write_class=<class>` so integration profiles verify the real choice.

`NewObjectSequential` creates a new data object, inserts bounded payload rows
without generic conflict updates, attaches or swaps ownership atomically,
adjusts reference counts, and cleans up the old object according to policy.

`ExistingObjectPatch` retains the safe block staging/merge path and its CRC and
partial-write semantics. `TruncateOnly` remains a separate metadata/storage
boundary.

The append-only transaction now uses the existing
`transactional_replay_confirmed()` infrastructure and a durable request token.
It creates a replacement `data_object`, streams bounded rows directly to
`data_extents`, optionally creates block CRC rows, swaps `files.data_object_id`,
updates reference counts, and applies immediate or deferred cleanup in one
transaction. A lost commit acknowledgement is confirmed by joining the request
token to the target file's currently attached object; a body disconnect rolls
back and replays through the existing bounded mechanism.

Coverage now includes body and commit disconnects, durable retry confirmation,
shared data objects, hardlinks, full overwrite, CRC maintenance, remount
durability, and immediate/deferred cleanup. The full 2026-07-11 local gate
passed.

The repeated 64 MiB large-file profile based on commit `42c5edf` measured
`94.55 MiB/s` for append-only 1 MiB extents versus `46.39 MiB/s` for blocks,
with 64 extent inserts, bounded payloads, and about `1.03 MB` mean WAL. The
matching large-copy profile measured `12.14 MiB/s` for extents versus
`16.07 MiB/s` for blocks. SQL profiles continue to attribute the copy
regression to repeated extent reads, not append-only payload persistence.
Consequently Phase C is complete without changing the default storage path.

## Phase D: object segment manifest decision

The decision is recorded in
`docs/adr/storage-object-segment-manifest.md`: do not implement a manifest now.
Whole-object adoption solves exact full copies without destination payload
rows, and bounded extents plus safe extent-to-block conversion cover the
current measured paths.

If large extent rewrites, partial-overwrite amplification, duplicate payload
storage, `copy_file_range` payload copies, or GC complexity remain material,
prepare `docs/adr/storage-object-segment-manifest.md` for this model:

```text
files
    -> data_objects
    -> object_segments
    -> payload_chunks
```

The deferred design allows copy-on-write segments, chunk reuse, aligned
`copy_file_range`, chunk-level deduplication, compression, snapshots, and
versioning. Reopen it only after a real partial-copy, patch-amplification,
dedupe, snapshot, compression, or GC workload proves that the current object
model is insufficient.

## Storage ownership follow-up

Payload ownership is centered logically on `data_object_id`, but the schema
still carries representative `id_file` columns. The complete inventory is in
`docs/storage-payload-ownership-inventory.md` and covers payload persistence,
failed materialization cleanup, purge, hardlink behavior, data-object swap,
GC, dedupe, diagnostics, and schema constraints.

The diagnostic inventory is complete. The next step is one explicit schema
migration that removes runtime dependence and the obsolete columns together;
do not leave a permanent dual-path compatibility branch.

## Verification gates

Every storage hot-path change runs:

```bash
cargo fmt --all
cargo check --workspace
cargo test -p fod-rust-hotpath
cargo test -p fod-rust-fuse
make test-copy-block-crc-table
make test-remount-durability-benchmark
make test-persist-buffer-chunking
make test-unlink-after-write
make test-rust-hotpath-copy-dedupe
FOD_PROFILE_IO=1 make test-fio-sequential-io-strace
make test-fio-sequential-io
make test-fio-mixed-io
make test-fio-random-mixed-io
```

Use the smallest targeted subset while developing, then run the complete gate
before closing a phase.

Stop and do not enable a new default if CRC, remount durability, shared object,
hardlink, partial-write, or replay behavior regresses; random/mixed writes are
forced onto the extent path; RSS grows with whole-file size; or one extent
payload exceeds the configured maximum.

## Explicit non-goals

- Do not restore `FOD_DATA_BLOCKS_MERGE_DO_NOTHING` experiments.
- Do not change `DEFAULT_PERSIST_COPY_SEND_BUFFER_BYTES` from current evidence.
- Do not make fillfactor, extra conflict predicates, or client-side COPY buffer
  variants the primary direction.
- Do not remove the block path or globally enable extents.
- Do not implement extent merge/split in Phase A.
- Do not change partial-write, truncate, sparse-block, `copy_file_range`, CRC,
  remount durability, or data-object reference-count semantics as a side effect.

## Stage commits

The planned delivery boundaries are:

1. `FOD 3.2.1: document storage engine v2 direction`
2. `FOD 3.2.1: add bounded extent planning`
3. `FOD 3.2.1: persist bounded extent payloads`
4. `FOD 3.2.1: record bounded extent benchmark matrix`
5. `FOD 3.2.1: add sequential segment write state`
6. `FOD 3.2.1: persist sequential segments directly`
7. `FOD 3.2.1: classify storage persistence operations`
8. `FOD 3.2.1: add append-only sequential object persistence`

Post-gate decisions and cleanup use separate commits:

9. `FOD 3.2.1: optimize whole-object FUSE copies`
10. `FOD 3.2.1: make data objects own payload rows`

Manifest and ownership changes require separate decisions after measured
results from the earlier phases.
