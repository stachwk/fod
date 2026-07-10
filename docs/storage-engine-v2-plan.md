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

## Current problem

The block path splits a large sequential stream into 4 KiB allocations and
persists thousands of PostgreSQL rows. The current extent proof of concept
avoids those physical rows, but it coalesces a full contiguous file into one
extent and rebuilds one payload `Vec` proportional to the whole file. A 64 MiB
write can therefore become one 64 MiB `PersistExtentRow`.

That shape is not suitable as a general physical representation:

- peak payload memory grows with file size;
- a large `BYTEA` is tied to the whole contiguous range;
- future patching, GC, and read assembly have no bounded physical unit;
- the current write state still creates 4 KiB block vectors before rebuilding
  the extent payload.

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

### Extent proof of concept

```text
full contiguous dirty block set
    -> one coalesced extent
    -> one rebuilt payload Vec
    -> PersistExtentRow
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

## Phase C: append-only new-object persistence

After the segment builder is stable, classify persistence semantics:

```rust
enum PersistWriteClass {
    NewObjectSequential,
    ExistingObjectPatch,
    TruncateOnly,
}
```

`NewObjectSequential` may create a new data object, insert bounded payload rows
without generic conflict updates, attach or swap ownership atomically, adjust
reference counts, and clean up the old object according to policy.

`ExistingObjectPatch` retains the safe block staging/merge path and its CRC and
partial-write semantics. `TruncateOnly` remains a separate metadata/storage
boundary.

The append-only transaction must use the existing
`transactional_replay_confirmed()` infrastructure and durable outcome
confirmation. It must not introduce an independent retry model. Required
coverage includes body and commit disconnects, retry confirmation, shared data
objects, hardlinks, full overwrite, remount durability, and immediate/deferred
cleanup.

## Phase D: object segment manifest decision

Do not implement a manifest automatically. Re-evaluate after Phases A-C.

If large extent rewrites, partial-overwrite amplification, duplicate payload
storage, `copy_file_range` payload copies, or GC complexity remain material,
prepare `docs/adr/storage-object-segment-manifest.md` for this model:

```text
files
    -> data_objects
    -> object_segments
    -> payload_chunks
```

The potential design allows copy-on-write segments, chunk reuse, aligned
`copy_file_range`, chunk-level deduplication, compression, snapshots, and
versioning. It should not be added if bounded extents and direct segment
persistence already solve the measured bottleneck.

## Storage ownership follow-up

Payload ownership should eventually be centered on `data_object_id`, not
`id_file`. Before any schema migration, inventory every use of
`data_blocks.id_file`, `data_extents.id_file`, `copy_block_crc.id_file`, failed
materialization cleanup, hardlink promotion, data-object swap, GC, and dedupe.

The order is diagnostic inventory, removal of runtime dependence on `id_file`
ownership, schema migration, then obsolete-field cleanup. Do not remove
`id_file` without a migration and complete correctness coverage.

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

Manifest and ownership changes require separate decisions after measured
results from the earlier phases.
