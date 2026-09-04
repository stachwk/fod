# FOD quota lock concurrency plan — completed record

## Status

The ordinary-write quota-lock concurrency project is complete in FOD 3.2.76.
This document is retained as the design and validation record for that change.
It is no longer the active performance implementation plan.

Current performance work is defined by:

1. [`block-only-performance-plan.md`](block-only-performance-plan.md) — canonical
   architecture and optimization order;
2. [`mounted-fuse-write-profile-plan.md`](mounted-fuse-write-profile-plan.md) —
   active end-to-end mounted FUSE profiling subplan.

The active storage architecture remains block-only. Nothing in this record
permits restoring extents, changing the 4 KiB logical block size or weakening
quota correctness.

## Problem that was solved

Before FOD 3.2.76, ordinary payload transactions acquired the database-wide
transaction-scoped quota advisory lock before COPY and merge:

```text
BEGIN
  -> pg_advisory_xact_lock(quota)
  -> read max_fs_size_bytes
  -> COPY BINARY staging
  -> data_blocks set-based merge
  -> final persisted + reserved quota calculation
  -> COMMIT or rollback/ENOSPC
```

Because `pg_advisory_xact_lock` was transaction-scoped, the lock stayed held
during long payload transfer and merge work. Independent FOD processes/mounts
therefore serialized even when transaction and task gates allowed concurrency.

The pre-change direct profile showed the failure clearly:

| workload | workers | throughput | quota advisory-lock total |
| --- | ---: | ---: | ---: |
| 4 MiB total | 4 | `26.345 MiB/s` | `117.372 ms` |
| 4 MiB total | 8 | `17.296 MiB/s` | `256.648 ms` |
| 128 MiB total | 4 | `51.522 MiB/s` | `3517.839 ms` |
| 128 MiB total | 8 | `34.046 MiB/s` | `12683.113 ms` |

The lock was required for the final database-wide quota decision, but not for
COPY/merge work that merely prepares a transaction which may later commit or
roll back.

## Implemented design

FOD 3.2.76 changed ordinary block persistence to:

```text
BEGIN

  COPY BINARY staging
  data_blocks merge
  other payload mutation work

  [short serialized quota gate]
    -> pg_advisory_xact_lock(quota)
    -> reread current max_fs_size_bytes
    -> calculate committed payload visible now
       + this transaction's uncommitted payload
       + active reservations
    -> if within quota: COMMIT
    -> if over quota: ROLLBACK and ENOSPC
```

This applies to ordinary `persist_file_blocks*` paths and
`persist_file_blocks_from_path` without a capacity reservation token.

Reservation-token paths remain deliberately conservative: they still acquire
the quota lock before reservation refresh and payload persistence. Their
expiry/reconciliation semantics are a separate concern and were not mixed into
the ordinary-write lock-scope change.

## PostgreSQL visibility contract

The late gate relies on PostgreSQL `READ COMMITTED` behavior.

If transaction A commits while transaction B is waiting on the quota advisory
lock, B's next quota query after acquiring the lock must see:

- A's newly committed payload;
- B's own uncommitted payload;
- current active reservations.

FOD connection setup pins the required transaction isolation. A future path
using `REPEATABLE READ` or another snapshot model must not reuse this design
without proving equivalent visibility.

## Correctness invariants

The redesign must permanently preserve:

1. `max_fs_size_bytes` is authoritative across all FOD processes and mounts
   sharing the database.
2. Concurrent writers near quota cannot both commit beyond the limit.
3. A transaction rejected by quota leaves no committed payload mutation.
4. Active capacity reservations are counted exactly once.
5. Reservation expiry/renewal cannot reclaim capacity already committed or
   reserved by another operation.
6. `statfs` continues to include active reservations.
7. sparse files, overwrite-only writes, truncate, object adoption,
   copy-on-write, object GC and remount retain their allocation semantics.
8. replay and ambiguous-COMMIT handling do not duplicate payload or reservation
   accounting.
9. primary failover/replay cannot confirm a quota decision against an
   unvalidated authority.

These are production correctness contracts, not optional benchmark conditions.

## Validation completed

### FOD 3.2.75 instrumentation

FOD 3.2.75 added separate observability for:

- persist transaction total;
- COPY staging;
- `data_blocks` merge;
- quota-lock wait;
- quota-lock held time;
- final quota calculation.

This step did not change behavior.

### Direct two-repository quota regression

FOD 3.2.76 added
`ordinary_persist_copy_merge_completes_before_final_quota_gate`.

The regression:

1. creates two independent `DbRepo` instances;
2. blocks the final advisory lock with a third connection;
3. proves both writers complete COPY/merge before they wait for that lock;
4. releases the lock;
5. verifies exactly one writer commits and one receives quota `ENOSPC`;
6. verifies the rejected file leaves no `data_blocks` rows;
7. verifies only one block of payload growth is committed.

### Mounted two-FUSE validation

The native restricted execution environment has `NoNewPrivs: 1`, so the mounted
validation used a temporary privileged Docker/chroot wrapper against the host
filesystem.

Through that path, `test-two-mount-quota` passed with:

- two independent FUSE mounts;
- two advisory-lock waiters;
- one committed writer;
- one quota-rejected writer;
- exact `4096` byte payload growth.

This closes the previously blocked mounted quota validation for the late-gate
design.

## Performance result

Direct 128 MiB block-persist after the change:

| workers | throughput | quota advisory-lock total |
| ---: | ---: | ---: |
| 1 | `51.888 MiB/s` | `0.014 ms` |
| 2 | `85.880 MiB/s` | `0.018 ms` |
| 4 | `122.843 MiB/s` | `0.049 ms` |
| 8 | `131.285 MiB/s` | `9.742 ms` |

Compared with the pre-change measurements:

- 128 MiB / 4 workers improved from `51.522 MiB/s` to `122.843 MiB/s`;
- 128 MiB / 8 workers improved from `34.046 MiB/s` to `131.285 MiB/s`;
- 8-worker advisory-lock SQL time fell from about `12683 ms` to about
  `9.742 ms`.

For the new 128 MiB / 8-worker direct profile:

- COPY BINARY staging: about `3240.165 ms` aggregate;
- `data_blocks` merge: about `2662.982 ms` aggregate;
- final quota checks: about `16.563 ms` aggregate;
- advisory-lock SQL: about `9.742 ms` aggregate.

Therefore ordinary-write quota serialization is no longer the measured
bottleneck.

## Why worker tuning was not finalized here

Removing the quota lock revealed workload-size dependence rather than one
obvious universal worker count:

- 4 MiB direct persistence peaked around 2-4 workers and regressed at 8;
- 128 MiB direct persistence continued improving through 8 workers.

That means `workers_write`, transaction limits and task admission should be
selected from mounted end-to-end profiles, not from the quota project alone.

The active worker/performance decision is therefore delegated to
`mounted-fuse-write-profile-plan.md`.

## Capacity reservation follow-up boundary

FOD already has `payload_capacity_reservations`.

Do **not** automatically extend reservations to every ordinary write now. The
simpler late-gate design should remain unless a measured near-full workload shows
that performing large COPY/merge work and then rolling back with `ENOSPC` wastes
a material amount of time or resources.

If ordinary-write pre-reservation is revisited, reserve positive **physical
allocation growth**, not dirty logical bytes.

The accounting must distinguish at least:

- non-zero overwrite of an already allocated block: usually delta `0`;
- new non-zero block: positive allocation;
- allocated block replaced with sparse zero: negative allocation;
- sparse zero write: no payload allocation;
- truncate: may release allocation;
- copy-on-write/shared-object transition: allocation depends on resulting object
  ownership and payload shape.

A blanket `reservation = dirty_bytes` rule is unacceptable because it can return
false `ENOSPC` for overwrite-only workloads on a full filesystem.

## Reopen criteria

Reopen this quota-lock project only if new evidence shows one of:

- quota wait/held time again becomes material after another transaction change;
- a quota race or stale-snapshot bug is found;
- reservation-token paths become the dominant large-write serialization point;
- late `ENOSPC` rollback wastes material work near capacity;
- failover/replay interaction invalidates the current final-gate assumptions.

Do not reopen it merely because COPY, merge, FUSE callback or worker costs are
high; those belong to the current mounted-FUSE performance plan.

## Documentation authority

For current work use:

1. `block-only-performance-plan.md` — canonical active architecture/order;
2. `mounted-fuse-write-profile-plan.md` — active performance subplan;
3. this file — completed quota-lock design record;
4. `conclusions.md` — measured evidence.

## Delivery rule

Any future quota behavior change uses the next sequential FOD version, updates
relevant tests/documentation and must re-run both direct and mounted
cross-process quota regressions.

After every commit compare it with its parent using `git diff HEAD~1..HEAD` or
`git show` and inspect accidental changes, missing files, regressions and
consistency with the active canonical plan before push.
