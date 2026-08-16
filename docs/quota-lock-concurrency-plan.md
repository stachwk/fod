# FOD quota lock concurrency plan

## Status

This document records the next measured write-side optimization after FOD 3.2.74 reduced the repeated read-side `FileAttr.blocks` allocation query.

The active storage architecture remains block-only. Do not change the 4 KiB logical block size or introduce another payload representation as part of this work.

FOD 3.2.75 completes the first implementation step by adding quota/persist timing observability only. It does not move the advisory lock yet and therefore does not claim a throughput improvement.

FOD 3.2.76 moves ordinary block-persist writes to the short final quota gate. Reservation-token writes remain conservative and still acquire the quota lock before reservation refresh/payload persistence until their expiry/reconciliation semantics are reviewed separately.

## Why worker tuning is not the next step

The 2026-08-16 direct concurrent block-persist profile on commit `6797299` showed that increasing write parallelism makes the current persistence path slower because all payload transactions serialize on the quota advisory lock.

Measured direct hotpath results:

| workload | workers | elapsed | throughput | quota advisory-lock total |
| --- | ---: | ---: | ---: | ---: |
| 4 MiB total | 4 | `0.151831 s` | `26.345 MiB/s` | `117.372 ms` |
| 4 MiB total | 8 | `0.231268 s` | `17.296 MiB/s` | `256.648 ms` |
| 128 MiB total | 4 | `2.484365 s` | `51.522 MiB/s` | `3517.839 ms` |
| 128 MiB total | 8 | `3.759582 s` | `34.046 MiB/s` | `12683.113 ms` |

At 128 MiB, raising concurrency from 4 to 8 workers reduced throughput while aggregate advisory-lock wait increased from about `3.5 s` to about `12.7 s`.

Therefore do not tune `workers_write`, write transaction limits, or task write admission limits as the primary optimization yet. Their useful operating point cannot be measured while a global quota lock serializes the long persistence transaction.

## Historical quota lock scope

Before FOD 3.2.76, the ordinary block-persist path effectively had this shape:

```text
BEGIN
  -> pg_advisory_xact_lock(quota)
  -> read max_fs_size_bytes
  -> optional reservation refresh
  -> COPY BINARY staging
  -> data_blocks set-based merge
  -> final persisted + reserved quota calculation
  -> COMMIT or rollback/ENOSPC
```

Because `pg_advisory_xact_lock` is transaction-scoped, the lock remains held during the long COPY and merge section. Independent processes and mounts therefore cannot perform payload persistence concurrently even when PostgreSQL transaction and payload-in-flight gates allow it.

The lock is required for correctness of the final quota decision, but it does not need to protect CPU/network transfer work that does not itself decide which transaction is allowed to consume the remaining capacity.

## Target design: short final quota gate

### Phase 1 — ordinary block persist

For ordinary block persistence without a pre-existing capacity reservation, move the global quota serialization to the shortest possible final section:

```text
BEGIN

  COPY BINARY staging
  data_blocks merge
  other payload mutation work

  [short serialized quota section]
    -> pg_advisory_xact_lock(quota)
    -> read current max_fs_size_bytes
    -> calculate committed payload visible now
       + this transaction's uncommitted payload
       + active reservations
    -> if within quota: COMMIT
    -> if over quota: ROLLBACK and return ENOSPC
```

The objective is that different writers may execute COPY and merge concurrently, while only the final capacity decision is serialized.

Do not remove quota serialization entirely. Two independent writers close to the limit must still be unable to both commit when their combined result exceeds `max_fs_size_bytes`.

## PostgreSQL isolation requirement

The late quota-check design relies on the final quota query seeing a transaction that committed while the current transaction was waiting for the advisory lock.

Pin this assumption explicitly to PostgreSQL `READ COMMITTED` semantics. Do not leave it as an undocumented dependency on server defaults.

Required regression shape:

1. transaction A and transaction B perform payload work concurrently;
2. A obtains the quota advisory lock first, validates and commits;
3. B obtains the lock afterwards;
4. B's next quota query must observe A's committed payload plus B's own uncommitted changes;
5. if the combined allocation exceeds the limit, B must roll back with ENOSPC.

If a future transaction path uses `REPEATABLE READ` or another snapshot policy, it must not silently reuse this late-check design without proving equivalent visibility.

## Capacity reservations

FOD already has `payload_capacity_reservations`, which is the preferred foundation for a later stronger design.

The reservation path already performs the important capacity decision under a short quota lock:

```text
quota lock
  -> committed payload + active reservations
  -> insert/refresh reservation
  -> commit reservation
```

Active reservations are also included in `statfs` free-space accounting.

### Phase 2 — reservation-backed long writes

After the late final gate is proven, evaluate extending reservations to ordinary large writes:

```text
short quota lock
  -> reserve expected positive allocation delta
  -> commit reservation

long COPY / merge without global quota lock

short reconciliation
  -> compare actual allocation delta with reservation
  -> consume/release reservation
  -> commit payload transaction
```

Do not implement this by reserving all dirty bytes blindly. The reservation must represent positive physical allocation growth, not logical write size.

Cases that must be distinguished:

- overwrite non-zero block with non-zero block: usually allocation delta `0`;
- create a new non-zero block: positive allocation delta;
- replace allocated block with sparse zero: negative allocation delta;
- sparse zero write: no payload allocation;
- truncate: may release allocation;
- copy-on-write/shared data object transition: allocation depends on the resulting object ownership and payload shape.

An overly conservative `reservation = dirty_bytes` policy could return false ENOSPC for overwrite-only workloads on a full filesystem and is therefore not an acceptable final design.

## Correctness invariants

Any quota-lock redesign must preserve all existing contracts:

1. `max_fs_size_bytes` is authoritative across all FOD processes and mounts sharing the database.
2. Two concurrent writers near quota cannot both commit beyond the limit.
3. A transaction rejected by quota leaves no committed payload mutation.
4. Existing capacity reservations remain accounted for exactly once.
5. Reservation expiry/renewal cannot reclaim capacity already committed or reserved by another operation.
6. `statfs` continues to include active reservations in used/free-space accounting.
7. sparse files, overwrite-only writes, truncate, object adoption, copy-on-write, object GC and remount keep their existing allocation semantics.
8. transaction replay and ambiguous-COMMIT handling do not duplicate reservations or payload accounting.
9. primary failover/replay logic must not allow a quota decision to be confirmed against a different unvalidated authority.

## Implementation sequence

### Step 1 — instrument the quota critical section

Before behavior changes, make the profile distinguish at least:

- advisory-lock wait time;
- time from lock acquisition to quota-check completion;
- COPY staging time;
- `data_blocks` merge time;
- final quota calculation time;
- transaction total time.

This prevents a later speedup from merely moving wait time to another unlabeled statement.

Status: implemented in FOD 3.2.75. PostgreSQL lane and global payload logs now report `persist_transaction_*`, `persist_copy_stage_*`, `persist_data_blocks_merge_*`, `quota_lock_wait_*`, `quota_lock_held_*`, and `quota_final_check_*` counters/totals/maxima in microseconds. The current lock-held timer intentionally measures the transaction-scoped quota lock from successful acquisition until the guard is dropped in the persist/reservation transaction body; it is diagnostic instrumentation, not a lock-scope behavior change.

### Step 2 — move ordinary block-persist lock to the final check

Apply the smallest change first:

- ordinary `persist_file_blocks*` paths perform payload persistence before taking the global quota lock;
- acquire the advisory lock immediately before final quota validation;
- reread `max_fs_size_bytes` while holding the lock;
- run final persisted+reserved usage calculation while holding the lock;
- commit on success or roll back the entire payload transaction on ENOSPC.

Keep reservation-token paths conservative until their refresh/expiry semantics are separately reviewed.

Status: implemented in FOD 3.2.76 for ordinary `persist_file_blocks*` paths without a capacity reservation token. `persist_file_blocks_from_path` and normal block-row persistence now perform COPY/merge before taking the quota advisory lock, then acquire the lock immediately before rereading `max_fs_size_bytes` and running the final persisted+reserved quota check. The final quota query relies on FOD's session setup pinning PostgreSQL `READ COMMITTED`, which is already part of runtime requirements.

### Step 3 — prove two-process/two-mount quota safety

The existing concurrent quota regression must continue to prove the database-wide invariant.

Add or strengthen cases for:

- two writers where only one fits in remaining capacity;
- both writers beginning COPY before either reaches the final quota gate;
- first writer commits, second then sees the new committed usage and returns ENOSPC;
- rejected writer leaves zero unexpected `data_blocks` rows and no leaked reservation;
- same test using independent repository connections/processes;
- mounted two-FUSE-instance regression when the host environment permits it.

Status: implemented in FOD 3.2.76. The new direct hotpath regression `ordinary_persist_copy_merge_completes_before_final_quota_gate` uses two independent `DbRepo` instances, forces both writers to reach the final advisory-lock wait after COPY/merge, verifies one writer commits and one rolls back with quota failure, and checks the rejected file leaves no `data_blocks` rows. The follow-up mounted `test-two-mount-quota` run on commit `d6e356f` passed through a temporary sudo-capable Docker/chroot wrapper with two advisory-lock waiters, one committed writer and one quota rejection.

### Step 4 — measure concurrency before worker tuning

Repeat direct block-persist profiles at minimum for:

```text
4 MiB total:   workers 1, 2, 4, 8
128 MiB total: workers 1, 2, 4, 8
```

Capture:

- wall throughput;
- per-worker maximum time;
- advisory-lock wait total and maximum;
- COPY total;
- merge total;
- transaction total;
- PostgreSQL top statements;
- pool pressure;
- transaction-admission pressure;
- `perf stat` and `strace -f -c` for the best and worst 128 MiB cases.

Only after the long quota serialization is removed should FOD choose or tune write worker defaults.

Status: direct hotpath profile completed for FOD 3.2.76 working tree based on commit `2f95155`. The native Codex process still cannot execute setuid FUSE helpers because it has `NoNewPrivs: 1`, but commit `d6e356f` was additionally validated through a privileged Docker/chroot wrapper with `NoNewPrivs: 0`: mounted `test-fio-sequential-io-strace` passed for 4 MiB and 128 MiB, direct-io `perf stat` passed for both sizes, and `test-two-mount-quota` passed.

### Step 5 — decide whether ordinary writes need pre-reservation

If late quota rejection causes material wasted COPY/merge work near a full filesystem, then implement the reservation-backed design described above.

If late rejection is rare and normal concurrency scales well, keep the simpler late-gate architecture rather than adding accounting state unnecessarily.

## Acceptance criteria

The first implementation phase is successful only when all of the following hold:

- global quota correctness still passes across independent connections/processes/mounts;
- the advisory lock no longer spans the long COPY/merge section for ordinary block persistence;
- 128 MiB concurrent writers spend materially less aggregate time waiting on the quota advisory lock than the current `~3.5 s` at 4 workers and `~12.7 s` at 8 workers;
- 8-worker throughput is no longer slower merely because the quota lock serializes the transaction;
- no quota check is based on a stale snapshot that can miss a writer committed ahead of it;
- COPY and merge timings are still reported separately;
- no regression appears in single-writer 4 MiB/128 MiB profiles;
- quota rejection remains atomic: payload state is rolled back completely;
- all replay, reservation, `statfs`, sparse/truncate/shared-object and remount tests pass.

Do not require 8 workers to beat 4 workers as an architectural acceptance criterion. The purpose of this change is to remove artificial global serialization. The optimal worker count must be selected only from measurements after that serialization is gone.

## Next bottleneck after quota serialization

After the quota gate is narrowed, profile again before changing anything else.

Likely candidates are:

1. COPY BINARY staging;
2. `data_blocks` set-based merge;
3. PostgreSQL WAL/checkpoint cost under real concurrent writes;
4. connection/pool contention;
5. FUSE-side write admission or worker count.

The ordering must come from the new profile. Do not assume the previous single-writer COPY+merge ranking remains the dominant limit after true concurrent persistence becomes possible.

## Non-goals

- Do not disable or weaken the database-wide quota.
- Do not replace cross-process correctness with process-local accounting.
- Do not tune worker defaults before re-profiling after the quota-lock change.
- Do not mix replica routing, promotion/fencing, extent storage, block-size changes or unrelated PostgreSQL tuning into this change.
- Do not introduce per-block quota triggers on the COPY/merge path unless measured evidence shows they are superior to transaction-level accounting.
- Do not reserve logical dirty bytes when the physical allocation delta is smaller.

## Delivery boundary

The next production-code change should use the next sequential FOD version and stay narrowly scoped to quota-lock concurrency.

Expected delivery sequence:

1. add quota critical-section observability and focused tests; completed in FOD 3.2.75;
2. move ordinary block-persist quota serialization to the final validation section;
3. run quota correctness, replay and two-process/two-mount regressions;
4. run 1/2/4/8 concurrent persistence profiles;
5. update `conclusions.md`, `commands.md`, `TODO.md`, README/performance documentation with measured before/after results;
6. commit on `main` using `FOD <version>: <English description>`;
7. after committing, compare the new commit to its parent with `git diff HEAD~1..HEAD` or `git show` and inspect accidental changes, missing files, regressions and consistency with this plan before push.
