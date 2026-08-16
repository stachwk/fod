# FOD block-only performance plan

## Status

This is the canonical active storage-performance plan after FOD 3.2.71-3.2.76.
Older storage-engine plans and ADRs may retain benchmark history, but they must
not define an active runtime path or optimization order that conflicts with this
document.

The production storage architecture is intentionally block-only:

```text
FUSE write/read
    -> block-oriented write/read state
    -> COPY BINARY staging / set-based block merge
    -> canonical data_blocks
    -> PostgreSQL
```

The logical block size remains 4 KiB. `data_extents`, extent planning,
sequential-segment extent persistence and extent-first reads are retired and
must not return as an optimization shortcut.

Do not introduce another payload representation unless a later measured
workload proves that the single `data_blocks` model is insufficient and a new
ADR explicitly replaces this boundary.

## Completed performance decisions

### FOD 3.2.71-3.2.73 — retire the extent PoC

- FOD 3.2.71 disabled production extent persistence;
- FOD 3.2.72 migrated remaining `data_extents` rows to `data_blocks` with
  integrity checks;
- FOD 3.2.73 removed extent planner/payload/read/persist code and dropped the
  retired table after the migration gate.

The deciding 128 MiB profile showed an extent-to-block conversion taking about
`13.65 s` inside a roughly `16.08 s` persist operation. Dual block/extent
representation is therefore not an acceptable production hot-path tradeoff.

### FOD 3.2.74 — remove repeated read-side allocation COUNT

Normal `read()` no longer executes the full `FileAttr.blocks` allocation query.
Real `lookup`/`getattr` keep exact allocation semantics, while reads use narrower
by-file-id metadata.

The 128 MiB large-file profile dropped from roughly `1031` allocation-query
calls / `3570 ms` to about `5` calls / `12 ms`.

Do not replace this with a handle-local size cache unless cross-mount, truncate,
copy-on-write, replay and remount invalidation are proven.

### FOD 3.2.75 — quota/persist observability

FOD 3.2.75 added separate timings for:

- total persist transaction;
- COPY BINARY staging;
- `data_blocks` merge;
- quota advisory-lock wait;
- quota lock held time;
- final quota calculation.

This made the quota critical section measurable before behavior changed.

### FOD 3.2.76 — late quota gate

Ordinary block persistence now performs COPY/merge before taking the global
quota advisory lock. The transaction takes the lock only for the final
serialized quota decision, rereads `max_fs_size_bytes`, counts persisted payload
plus active reservations and commits or rolls back with `ENOSPC`.

Reservation-token paths intentionally remain conservative and still hold the
quota lock across reservation refresh and payload persistence until their
expiry/reconciliation semantics are reviewed separately.

The direct 128 MiB profile after the change was:

| workers | throughput | advisory-lock SQL total |
| ---: | ---: | ---: |
| 1 | `51.888 MiB/s` | `0.014 ms` |
| 2 | `85.880 MiB/s` | `0.018 ms` |
| 4 | `122.843 MiB/s` | `0.049 ms` |
| 8 | `131.285 MiB/s` | `9.742 ms` |

Compared with the pre-change profile:

- 4 workers improved from `51.522 MiB/s` to `122.843 MiB/s`;
- 8 workers improved from `34.046 MiB/s` to `131.285 MiB/s`;
- 8-worker advisory-lock SQL time fell from about `12683 ms` to about `9.7 ms`.

For 128 MiB / 8 workers, the direct SQL profile is now dominated by COPY BINARY
staging (`3240.165 ms` aggregate) plus the set-based `data_blocks` merge
(`2662.982 ms` aggregate), not by quota serialization.

Quota correctness was proven twice:

1. a direct two-`DbRepo` regression forces both writers past COPY/merge and then
   through the final quota gate, with one commit and one atomic `ENOSPC` rollback;
2. mounted `test-two-mount-quota` passed through the privileged Docker/chroot
   path with two independent FUSE mounts, two advisory-lock waiters, exactly one
   committed writer and one rejected writer.

The late quota gate is therefore considered complete for ordinary block
persistence unless later evidence shows a correctness or measurable performance
problem.

## Current measured problem

Direct PostgreSQL hotpath and mounted FUSE no longer point to exactly the same
next optimization.

The direct 128 MiB workload shows COPY + merge as the largest PostgreSQL work.
However, the mounted 128 MiB direct-I/O fio profile on commit `d6e356f` measured
about:

- write `18.2 MiB/s`;
- read `17.6 MiB/s`;
- COPY stage `1.102 s`;
- `data_blocks` merge `0.743 s`;
- quota wait `0.611 ms`;
- quota held `8.492 ms`;
- final quota check `7.788 ms`.

The complete mounted write takes materially longer than COPY + merge alone.
Therefore the next production optimization must **not** assume that COPY or
merge is the dominant end-to-end FUSE cost merely because it dominates the
direct repository profile.

The active detailed subplan is:

- [`mounted-fuse-write-profile-plan.md`](mounted-fuse-write-profile-plan.md)

The completed quota redesign record is:

- [`quota-lock-concurrency-plan.md`](quota-lock-concurrency-plan.md)

## Current optimization order

### Phase 1 — decompose real mounted 128 MiB write wall time

Measure the complete path:

```text
FUSE callback/admission
    -> write-state/block preparation
    -> buffer/flush scheduling
    -> task/transaction/pool wait
    -> PostgreSQL transaction
       -> COPY
       -> data_blocks merge
       -> metadata/CRC/object-finalization
       -> final quota gate
       -> COMMIT/replay confirmation
    -> post-commit invalidation
    -> callback completion
```

Reuse existing observability first. Add only missing timing boundaries needed to
explain the wall-clock gap.

At minimum separate:

- callback entry-to-return time;
- write-state preparation;
- flush queue wait and flush execution;
- logical-task admission wait;
- PostgreSQL transaction-admission wait;
- pool checkout wait;
- pre-COPY metadata work;
- COPY staging;
- merge;
- post-merge metadata/CRC/object-finalization;
- quota wait/held/check;
- COMMIT/replay confirmation;
- post-commit invalidation.

Do not change scheduling or batching in the same commit that establishes a
missing timing baseline.

### Phase 2 — pin repeated mounted baselines

Use the already proven sudo-capable Docker/chroot path because the native
restricted process has `NoNewPrivs: 1`.

Run at least three comparable samples of:

```text
4 MiB sequential fio with direct I/O
128 MiB sequential fio with direct I/O
```

Keep representative 128 MiB `perf stat` and `strace -f -c` captures. Do not
compare strace throughput directly with non-strace throughput.

Capture PostgreSQL top statements, WAL/checkpoint counters, callback counts,
queue/admission metrics and persist-stage timings for the same workload.

### Phase 3 — reconcile direct repository and mounted FUSE results

Produce one 128 MiB timing summary that explains most of mounted wall time and
explicitly marks overlapping counters.

Separate:

- CPU work versus blocked/wait time;
- per-file serialization versus global serialization;
- PostgreSQL server execution versus client/FUSE time;
- work that can overlap across workers versus work that cannot.

Only after this reconciliation choose the next production optimization.

### Phase 4 — optimize one measured dominant component

Possible outcomes include:

1. COPY BINARY staging;
2. `data_blocks` set-based merge;
3. FUSE callback/write-state preparation;
4. flush scheduling / queue handoff / futex wait;
5. transaction or pool admission;
6. metadata/CRC/object-finalization;
7. COMMIT/WAL/checkpoint cost;
8. remaining read-side metadata work if it becomes material again.

Do not optimize multiple layers in one measurement step. Make one narrow change,
re-run the same 4 MiB and 128 MiB baselines and compare stage timings.

### Phase 5 — tune worker/admission defaults only after mounted scaling is known

Direct results are workload-size dependent:

- 4 MiB peaked around 2-4 workers and regressed at 8;
- 128 MiB continued scaling through 8 workers.

Therefore do not set one production `workers_write` value from the direct 128 MiB
result alone. Choose worker, transaction and task-admission defaults only after
mounted FUSE profiles show how the same workload classes scale end to end.

An adaptive policy is acceptable only if fixed settings cannot cover measured
workloads cleanly and the adaptive decision remains bounded, observable and
regression-testable.

### Phase 6 — evaluate ordinary-write pre-reservation only if needed

The current late quota gate may waste work only when a large transaction reaches
final validation near a full filesystem and is rejected after COPY/merge.

Do not add more quota state preemptively. Evaluate reservation-backed ordinary
writes only if a measured near-full workload shows material wasted work.

Any reservation must represent positive **physical allocation delta**, not dirty
logical bytes. Overwrite-only writes on a full filesystem must not receive false
`ENOSPC` merely because they touched many bytes.

## Correctness gates for all block-only performance work

Every optimization must preserve:

- canonical `data_blocks` storage only;
- 4 KiB logical block semantics;
- sparse-file `st_blocks` behavior in POSIX 512-byte units;
- exact allocation after overwrite and truncate;
- shared `data_object_id` ownership without filesystem-wide double counting;
- full-object adoption and copy-on-write behavior;
- CRC behavior;
- transaction rollback and replay confirmation;
- database-wide quota and reservation semantics;
- two-mount quota safety;
- two-mount visibility according to the documented consistency contract;
- remount durability;
- primary/failover safety.

Do not trade these contracts for benchmark throughput.

## Performance acceptance gate

For a write-side optimization record at minimum:

```text
4 MiB mounted sequential fio, direct I/O, repeated samples
128 MiB mounted sequential fio, direct I/O, repeated samples
representative 128 MiB perf stat
representative 128 MiB strace -f -c
1/2/4/8 direct concurrent block-persist matrix when relevant
PostgreSQL top statements and WAL/checkpoint counters
FUSE callback counts
flush_write_state_us
repo_persist_blocks_us
persist transaction total
COPY stage total/max/count
data_blocks merge total/max/count
quota wait/held/final-check timing
queue/admission/pool wait metrics
```

Use repeated samples when the difference is close to the noise floor. Do not
change production defaults from one noisy run.

## Documentation authority

For active storage-performance decisions use this order:

1. this file — canonical architecture and global optimization order;
2. `mounted-fuse-write-profile-plan.md` — current detailed profiling subplan;
3. `quota-lock-concurrency-plan.md` — completed quota redesign record and
   reservation follow-up boundary;
4. `conclusions.md` — measured conclusions and historical evidence;
5. `performance-baselines.md` — recorded benchmark baselines.

`storage-engine-v2-plan.md` is historical after FOD 3.2.73 and must not contain
active extent phases. The object-segment-manifest ADR remains useful only for
its manifest decision and historical evidence; its old extent runtime model is
superseded.

## Non-goals

- Do not restore extents or another alternate payload format.
- Do not change the 4 KiB logical block size in this phase.
- Do not weaken database-wide quota for concurrency.
- Do not widen the quota lock again to solve an unrelated bottleneck.
- Do not replace cross-process correctness with process-local accounting.
- Do not tune worker defaults before mounted FUSE scaling is measured.
- Do not add PostgreSQL per-block quota triggers without measured evidence.
- Do not optimize hardlink counting merely because its call count is high.
- Do not assume COPY/merge is the full-system bottleneck until mounted wall time
  is decomposed.
- Do not mix unrelated replica routing, promotion/fencing or schema redesign into
  a narrow storage-performance change.

## Delivery rule

Production-code or profiling-instrumentation changes use the next sequential FOD
version, update the relevant documentation and tests, and are committed on
`main` using `FOD <version>: <English description>`.

After every commit, compare it with its parent using `git diff HEAD~1..HEAD` or
`git show` and inspect accidental changes, missing files, regressions and
consistency with this plan before push.
