# Storage Engine v2 — historical record

## Status

This document is no longer an active implementation plan.

The extent-based Storage Engine v2 experiment was completed and retired in
FOD 3.2.71-3.2.73. The current authoritative storage-performance plan is:

- [`block-only-performance-plan.md`](block-only-performance-plan.md)

The current quota/concurrency implementation subplan is:

- [`quota-lock-concurrency-plan.md`](quota-lock-concurrency-plan.md)

Nothing in this historical document may be used to re-enable extent planning,
`data_extents`, sequential-segment extent persistence, extent-first reads or an
extent fallback in production runtime code.

## Current storage boundary

The production storage path after FOD 3.2.73 is:

```text
FUSE write/read
    -> block-oriented write/read state
    -> COPY BINARY staging / set-based block merge
    -> canonical data_blocks
    -> PostgreSQL
```

The logical filesystem block size remains 4 KiB.

The production schema no longer contains `data_extents` after migration 21.
Legacy extent payload was converted administratively by migration 20 before the
retired table was dropped.

## Why the extent experiment was retired

The extent PoC was useful because it tested whether fewer, larger physical
payload rows could improve large sequential writes. Several intermediate local
benchmarks showed that bounded extent persistence could reduce row count and
payload preparation cost for selected sequential workloads.

The decisive later profile exposed the architectural cost of maintaining two
physical representations. A 128 MiB sequential write crossed a 64 MiB flush
boundary, entered the block patch path and materialized existing extents back to
4 KiB blocks. The server-side extent-to-block expansion alone consumed about
`13.65 s` of a roughly `16.08 s` persist operation.

That result established the final decision:

- one canonical payload representation is preferable to dual block/extent
  runtime state;
- extent-to-block conversion must not exist in the hot path;
- performance work continues on the block-only representation and PostgreSQL
  transaction/query shape;
- another physical representation requires a new measured problem and a new
  architecture decision, not reuse of the old PoC.

## Closed historical phases

The following phases are closed historical work. They are intentionally not
active tasks anymore.

### Bounded extent planning

Historical work added bounded extent planning and tested several payload sizes
instead of creating one whole-file extent. This reduced peak payload assembly
and physical row count for sequential PoC workloads.

It does **not** imply that an extent planner belongs in the current runtime.

### Sequential segment builder

Historical work introduced a sequential-segment write state for eligible new
files and moved owned buffers directly into extent persistence.

This state and its extent execution path were removed in FOD 3.2.73. There is
no active requirement to restore `SequentialSegmentState`, `PersistExtentRow`
or segment-mode observability.

### Append-only extent persistence

Historical work classified sequential new-object persistence and evaluated an
append-only replacement-object path backed by extents.

The useful semantic lesson was that whole-object replacement/adoption can avoid
unnecessary payload copying. The extent-backed implementation itself is retired.
Current copy/object-adoption semantics must operate with canonical
`data_blocks` and the current data-object ownership model.

### Extent-to-block compatibility conversion

The earlier compatibility conversion preserved bytes when a partial patch hit
an extent-backed object. It was correctness-first but caused severe write
amplification and became the decisive reason to remove the dual representation.

This conversion is permanently excluded from the production hot path. Legacy
conversion exists only as the completed administrative migration history.

## Schema and ownership history

Storage ownership work remains valid independently of extent retirement:

- schema version 17 made `data_object_id` the payload owner;
- schema version 18 added transactional payload-capacity reservations;
- schema version 19 added immutable index catalogue snapshots;
- schema version 20 migrated legacy extent payload into `data_blocks`;
- schema version 21 dropped `data_extents` after the migration gate verified no
  legacy rows remained.

The current payload ownership model is:

```text
files
    -> data_objects
       -> data_blocks
       -> copy_block_crc
```

Do not add `data_extents` back to this model.

## Object segment manifest decision

The object-segment/payload-chunk manifest remains deferred. Exact whole-object
adoption already addresses the main zero-copy full-file case without adding a
new payload hierarchy.

If a future measured workload demonstrates a real need for partial clone reuse,
chunk-level deduplication, compression, snapshots, immutable versioning or a
similar capability, reopen the manifest decision through a new ADR.

Do not reopen it merely to recreate extents under another name.

## Historical benchmark interpretation

Old benchmark documents may still contain comparisons between blocks and
extents. Treat those as historical evidence only.

They are useful for understanding why experiments were performed and why the
architecture changed, but they must not be used as current default-selection
criteria. Current performance decisions must be based on the block-only profiles
recorded after FOD 3.2.73.

## Current optimization direction

Active work is defined by `block-only-performance-plan.md`.

At the time this historical plan was archived, the current measured sequence
was:

1. FOD 3.2.74 removed the repeated `COUNT(data_blocks)` allocation query from
   normal `read()`;
2. concurrent write profiling showed that the global transaction-scoped quota
   advisory lock serializes long COPY/merge work;
3. the next write-side task is to narrow that quota critical section while
   preserving database-wide quota correctness;
4. only after that change should worker counts and the next PostgreSQL
   bottleneck be selected from fresh measurements.

This ordering supersedes earlier statements in this file that proposed extent
size tuning, extent defaults, extent read work, hardlink-count optimization or
COPY/merge tuning as the immediate next task.

## Current verification boundary

Storage hot-path changes should use the targeted subset during development and
then cover the relevant block-only gates before completion, including:

```bash
cargo fmt --all
cargo check --workspace --locked
cargo test -p fod-rust-hotpath
cargo test -p fod-rust-fuse
make test-copy-block-crc-table
make test-remount-durability-benchmark
make test-persist-buffer-chunking
make test-unlink-after-write
FOD_PROFILE_IO=1 make test-fio-sequential-io
make test-fio-mixed-io
make test-fio-random-mixed-io
```

Quota/concurrency changes additionally require the database-wide quota,
reservation, replay and independent-connection/mount regressions documented in
`quota-lock-concurrency-plan.md`.

## Non-goals retained from the experiment

The following restrictions remain valid:

- do not restore alternate payload-format experiments without new measured
  evidence;
- do not change the 4 KiB logical block size as a side effect;
- do not weaken partial-write, sparse, truncate, CRC, remount, replay,
  data-object ownership or quota semantics for throughput;
- do not choose production defaults from a single noisy sample;
- do not optimize a historical bottleneck after a newer profile has shown a
  different dominant cost.

## Documentation authority

For current work, use this order:

1. `block-only-performance-plan.md` — canonical active storage-performance plan;
2. `quota-lock-concurrency-plan.md` — current quota/concurrency subplan;
3. `conclusions.md` — measured results and durable conclusions;
4. this file — historical rationale only.
