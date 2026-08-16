# FOD block-only performance plan

## Status

This is the canonical active storage-performance plan after FOD 3.2.71-3.2.76.
Older storage-engine plans and ADRs may retain benchmark history, but they must
not define an active runtime path that conflicts with this document.

The active storage architecture is intentionally block-only:

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

## Completed decisions

### Extent PoC retirement

The extent PoC completed its purpose and was removed from the production
architecture:

- FOD 3.2.71 disabled production extent persistence;
- FOD 3.2.72 migrated remaining `data_extents` rows to `data_blocks` with
  integrity checks;
- FOD 3.2.73 removed extent planner/payload/read/persist code and dropped the
  retired table after the migration gate.

The deciding 128 MiB profile showed that converting existing extents back to
4 KiB blocks cost about `13.65 s` inside a roughly `16.08 s` persist operation.
That conversion and the dual-representation complexity are no longer acceptable
hot-path tradeoffs.

### Read-side allocation query

FOD 3.2.74 removed the repeated allocated-block COUNT from normal `read()`.
Real `lookup`/`getattr` keep the exact `FileAttr.blocks` semantics, while normal
reads use narrow by-file-id metadata.

Measured change for the 128 MiB large-file profile:

- old allocation query: about `1031` calls / `3570 ms`;
- after FOD 3.2.74: about `5` calls / `12 ms`;
- large-file wall throughput stayed roughly flat, so the next limiting work is
  on the write/concurrency side rather than the old allocation query.

Do not replace this with a handle-local file-size cache unless cross-mount,
truncate, copy-on-write, replay and remount invalidation are proven.

## Current measured write-side problem

The 2026-08-16 concurrent direct block-persist profile on commit `6797299`
showed that increasing write concurrency makes the current persistence path
slower because payload transactions serialize on the global quota advisory
lock.

| workload | workers | throughput | quota advisory-lock total |
| --- | ---: | ---: | ---: |
| 4 MiB total | 4 | `26.345 MiB/s` | `117.372 ms` |
| 4 MiB total | 8 | `17.296 MiB/s` | `256.648 ms` |
| 128 MiB total | 4 | `51.522 MiB/s` | `3517.839 ms` |
| 128 MiB total | 8 | `34.046 MiB/s` | `12683.113 ms` |

At 128 MiB, moving from 4 to 8 workers reduced throughput while aggregate quota
lock wait grew from about `3.5 s` to about `12.7 s`.

Therefore **do not tune write-worker defaults yet**. Worker-count measurements
are distorted while a global transaction-scoped quota lock spans the long
COPY/merge section.

FOD 3.2.75 adds the observability needed for the next comparison:
`persist_transaction_*`, `persist_copy_stage_*`, `persist_data_blocks_merge_*`,
`quota_lock_wait_*`, `quota_lock_held_*`, and `quota_final_check_*`. This is an
instrumentation step only; the ordinary block-persist transaction shape is
unchanged until the quota lock is moved to the final validation gate.

FOD 3.2.76 moves ordinary block-persist writes to that final validation gate
and keeps reservation-token writes conservative. The direct 128 MiB profile now
scales from `51.888 MiB/s` at 1 worker to `131.285 MiB/s` at 8 workers, while
the aggregate advisory-lock SQL time at 8 workers drops from the old
`12683.113 ms` to `9.742 ms`. The next measured bottleneck is no longer quota
serialization; it is COPY BINARY staging plus the set-based `data_blocks`
merge under real concurrency.

The follow-up mounted validation for commit `d6e356f` used a temporary
sudo-capable Docker/chroot wrapper because the native Codex process has
`NoNewPrivs: 1`. Through that wrapper, `test-fio-sequential-io-strace` passed
for 4 MiB and 128 MiB with `FOD_FOPEN_DIRECT_IO=1`, direct-io `perf stat`
passed for both sizes, and `test-two-mount-quota` passed with two independent
FUSE mounts and exactly one 4 KiB quota-admitted writer.

The detailed implementation subplan is:

- [`quota-lock-concurrency-plan.md`](quota-lock-concurrency-plan.md)

That document is subordinate to this canonical block-only architecture; it may
change quota synchronization, but it may not introduce another payload format
or weaken the storage correctness contracts below.

## Current optimization order

### Phase 1 — narrow the quota critical section

For ordinary block persistence, change the transaction shape from:

```text
BEGIN
  quota advisory lock
  COPY BINARY staging
  data_blocks merge
  quota check
COMMIT
```

toward:

```text
BEGIN
  COPY BINARY staging
  data_blocks merge

  short serialized quota gate
    -> advisory lock
    -> reread max_fs_size_bytes
    -> calculate committed payload + this transaction + active reservations
    -> commit or rollback/ENOSPC
```

The final quota decision remains database-wide and serialized. COPY and merge
must not be globally serialized merely because they happen before that decision.

This first design relies on a final quota query that sees a writer committed
while the current transaction waited for the advisory lock. Pin and test the
required PostgreSQL `READ COMMITTED` visibility; do not rely silently on server
defaults.

Keep reservation-token paths conservative until refresh/expiry semantics are
reviewed separately.

### Phase 2 — prove quota correctness under real concurrency

Before performance tuning, preserve all existing quota invariants:

1. `max_fs_size_bytes` remains authoritative across independent FOD processes
   and mounts sharing one database.
2. Two concurrent writers near the limit cannot both commit beyond quota.
3. A rejected transaction leaves no committed payload mutation.
4. Active capacity reservations are counted exactly once.
5. Reservation expiry/renewal cannot steal capacity already committed or
   reserved elsewhere.
6. `statfs` continues to account for active reservations.
7. sparse writes, overwrite-only writes, truncate, shared objects,
   `copy_file_range`, copy-on-write, GC and remount keep their allocation
   semantics.
8. replay and ambiguous-COMMIT handling do not duplicate accounting.

Required regression shape:

- two independent connections/processes begin payload work concurrently;
- both reach the final quota gate only after COPY/merge work has begun;
- the first valid writer commits;
- the second writer then observes the first commit;
- if the combined result is over quota, the second writer rolls back with
  `ENOSPC` and leaves no unexpected `data_blocks` rows.

Run the existing two-mount quota regression when the host environment permits
FUSE mounting.

### Phase 3 — re-profile before worker tuning

After the long quota serialization is removed, repeat concurrent block-persist
profiles for at least:

```text
4 MiB total:   workers 1, 2, 4, 8
128 MiB total: workers 1, 2, 4, 8
```

Capture:

- wall throughput;
- per-worker maximum time;
- advisory-lock wait total/max;
- lock-held quota-check time;
- COPY staging time;
- `data_blocks` merge time;
- transaction total time;
- PostgreSQL top statements;
- pool and transaction-admission pressure;
- `perf stat` and `strace -f -c` for representative best/worst 128 MiB cases.

Only after this profile may `workers_write`, transaction limits or task write
admission defaults be tuned.

Do not make “8 workers must beat 4 workers” an acceptance criterion. The goal
is to remove artificial global serialization; the best worker count must be a
measured result afterwards.

### Phase 4 — decide whether normal writes need pre-reservation

FOD already has `payload_capacity_reservations`. If late quota rejection wastes
material COPY/merge work near a full filesystem, extend reservations to normal
large writes using the positive **physical allocation delta**, not logical dirty
bytes.

The accounting must distinguish at least:

- overwrite allocated non-zero block with another non-zero block: usually `0`;
- create a new non-zero block: positive allocation;
- replace an allocated block with sparse zero: negative allocation;
- sparse zero write: no payload allocation;
- truncate: may release allocation;
- shared-object/copy-on-write transition: depends on resulting ownership and
  payload shape.

A blanket `reservation = dirty_bytes` policy is not acceptable because it can
return false `ENOSPC` for overwrite-only workloads on a full filesystem.

### Phase 5 — choose the next bottleneck from the new profile

Do not assume the previous single-writer ordering survives true concurrent
persistence.

Possible next targets include:

1. COPY BINARY staging;
2. `data_blocks` set-based merge;
3. PostgreSQL WAL/checkpoint cost under concurrent writes;
4. connection/pool contention;
5. remaining FUSE-side admission/worker overhead;
6. remaining read-metadata round trips only if they become material again.

Hardlink-count optimization remains deferred unless a metadata-heavy profile
shows it becoming significant. High call count alone is not sufficient evidence.

## Correctness gates for all block-only performance work

Every optimization must preserve:

- 4 KiB logical block semantics;
- sparse-file `st_blocks` behavior in 512-byte POSIX units;
- exact allocation after overwrite and truncate;
- shared `data_object_id` ownership without filesystem-wide double counting;
- full-object adoption and copy-on-write behavior;
- CRC behavior;
- transaction rollback and replay confirmation;
- database-wide quota and reservation semantics;
- two-mount visibility according to the documented consistency contract;
- remount durability;
- primary/failover safety.

Do not trade these contracts for benchmark throughput.

## Performance acceptance gate

For a write-side change, record at minimum:

```text
4 MiB sequential fio, FOD_PROFILE_IO=1
128 MiB sequential or large-file multiblock profile
1/2/4/8 direct concurrent block-persist matrix when applicable
PostgreSQL top statements/call counts
quota advisory-lock wait and held time
repo_persist_blocks_us
COPY stage time
data_blocks merge time
transaction total time
```

Use repeated samples when the difference is close to the noise floor. Do not
change production defaults from one noisy run.

## Documentation authority

For active storage-performance decisions use, in this order:

1. this file — canonical architecture and optimization order;
2. `quota-lock-concurrency-plan.md` — detailed current quota-concurrency subplan;
3. `conclusions.md` — measured conclusions and historical evidence;
4. `performance-baselines.md` — recorded benchmark baselines.

`storage-engine-v2-plan.md` is historical after FOD 3.2.73 and must not contain
active extent phases. The object-segment-manifest ADR remains useful only for
its manifest decision and historical evidence; its old extent runtime model is
superseded.

## Non-goals

- Do not restore extents or another alternate payload format.
- Do not change the 4 KiB logical block size in this phase.
- Do not weaken database-wide quota for concurrency.
- Do not replace cross-process correctness with process-local accounting.
- Do not tune worker defaults before re-profiling after quota-lock narrowing.
- Do not add PostgreSQL per-block quota triggers without measured evidence.
- Do not optimize hardlink counting merely because its call count is high.
- Do not mix unrelated primary/replica routing, promotion/fencing or schema
  redesign into a narrow storage-performance change.

## Delivery rule

Production-code changes use the next sequential FOD version, update the relevant
documentation and tests, and are committed on `main` using
`FOD <version>: <English description>`.

After every commit, compare it with its parent using `git diff HEAD~1..HEAD` or
`git show` and inspect accidental changes, missing files, regressions and
consistency with this plan before push.
