# FOD mounted FUSE write profile plan

## Status

This is the active performance subplan after FOD 3.2.76 removed the artificial
quota-lock serialization from ordinary block persistence.

The canonical architecture remains defined by
[`block-only-performance-plan.md`](block-only-performance-plan.md). This document
only decomposes the next measured problem: the gap between direct PostgreSQL
block-persist throughput and end-to-end mounted FUSE throughput.

Do not change payload format, logical block size, quota semantics, replica
routing or failover behavior as part of this work.

## Why this is the next step

FOD 3.2.76 proved that the quota advisory lock is no longer the dominant cost for
ordinary block persistence.

Direct 128 MiB block-persist results after the late quota gate were:

| workers | throughput | advisory-lock SQL total |
| ---: | ---: | ---: |
| 1 | `51.888 MiB/s` | `0.014 ms` |
| 2 | `85.880 MiB/s` | `0.018 ms` |
| 4 | `122.843 MiB/s` | `0.049 ms` |
| 8 | `131.285 MiB/s` | `9.742 ms` |

For the 8-worker 128 MiB profile, PostgreSQL time was dominated by COPY BINARY
staging (`3240.165 ms` aggregate) and the set-based `data_blocks` merge
(`2662.982 ms` aggregate), not by the quota gate.

Mounted FUSE tells a different end-to-end story. The sudo-capable Docker/chroot
validation on commit `d6e356f` measured the 128 MiB direct-I/O fio workload at
about `18.2 MiB/s` write and `17.6 MiB/s` read without strace. In that same
write-side profile:

- COPY staging was about `1.102 s`;
- `data_blocks` merge was about `0.743 s`;
- quota-lock wait was about `0.611 ms`;
- quota-lock held time was about `8.492 ms`;
- final quota check was about `7.788 ms`.

The full 128 MiB write therefore contains material time outside the measured
COPY + merge section. The next optimization must identify that time instead of
assuming that the direct-hotpath bottleneck is also the end-to-end FUSE
bottleneck.

## Main question

Break one real mounted 128 MiB sequential write into a measured timeline:

```text
FUSE write callback admission
    -> callback/input handling
    -> write-state / block preparation
    -> write-buffer accumulation
    -> flush decision / flush scheduling
    -> task and transaction admission waits
    -> PostgreSQL connection checkout
    -> transaction begin / replay wrapper
    -> COPY BINARY staging
    -> data_blocks merge
    -> CRC / metadata / object-finalization work
    -> final quota gate
    -> COMMIT
    -> post-commit cache/statfs invalidation
    -> FUSE callback completion
```

For every stage distinguish:

- total time;
- maximum single-operation time;
- call count;
- bytes processed;
- time waiting versus time doing work.

The accounting should make it possible to explain most of the measured wall
clock rather than producing another set of overlapping counters.

## Phase 1 — pin the current mounted baseline

Use the privileged Docker/chroot path already proven on this host because the
native process has `NoNewPrivs: 1` and cannot use setuid FUSE helpers directly.

Repeat at least three samples of:

```text
4 MiB sequential fio, direct I/O
128 MiB sequential fio, direct I/O
```

For the 128 MiB case also keep one strace profile and one perf-stat profile, but
do not compare strace throughput directly with non-strace throughput.

Capture at minimum:

- fio write/read throughput and latency distribution;
- FUSE read/write callback counts;
- `flush_write_state_us`;
- `repo_persist_blocks_us`;
- persist transaction total;
- COPY stage total/max/count;
- `data_blocks` merge total/max/count;
- quota wait/held/final-check timing;
- task queue/admission wait and active counts;
- PostgreSQL pool checkout/connection wait;
- PostgreSQL top statements and WAL counters;
- process CPU/task-clock, cycles, instructions, context switches and faults;
- strace syscall totals, especially `read`, `write`, `futex`, `poll`, `mprotect`
  and `wait4` where material.

## Phase 2 — add only missing timing boundaries

Do not add broad instrumentation if existing counters already explain a stage.
Add focused timing only where the mounted wall-clock gap remains unaccounted.

Priority candidates are:

1. FUSE callback entry-to-return time split into read/write/flush/fsync;
2. write-state preparation and dirty-block normalization;
3. flush queue wait versus actual flush execution;
4. logical-task admission wait;
5. PostgreSQL transaction-admission wait;
6. connection-pool checkout wait;
7. pre-COPY metadata/object preparation;
8. post-merge metadata, CRC and object-finalization work;
9. COMMIT/replay-confirmation time;
10. post-commit invalidation and callback completion.

Instrumentation must remain observational. Do not change scheduling or batching in
the same commit that establishes the missing timing baseline.

## Phase 3 — reconcile direct and mounted profiles

Create one summary for 128 MiB showing:

```text
mounted wall time
  = FUSE/callback + buffering/admission + PostgreSQL persist + post-commit work
```

The summary must separately show:

- time that can overlap across workers;
- time serialized inside one file/flush;
- time serialized globally;
- CPU time versus blocked/wait time;
- PostgreSQL server execution versus client/FUSE time.

If measured component totals overlap, state the overlap explicitly instead of
adding them as if they were disjoint.

## Phase 4 — choose exactly one next optimization

Choose the next production change from the measured largest end-to-end
component.

Possible outcomes include:

### COPY BINARY dominates

Then inspect staging row construction, libpq COPY transfer size/call shape,
client buffer copies and server COPY receive cost. Keep set-based merge and quota
semantics unchanged while measuring the COPY-only change.

### `data_blocks` merge dominates

Then inspect insert-versus-update/conflict shape, indexes, target-row churn, WAL,
page locality and whether the statement performs avoidable work for known-new or
known-existing ranges. Prefer set-based changes; avoid per-block SQL/triggers.

### FUSE callback / write-state preparation dominates

Then reduce callback-side copies, repeated block normalization, map churn or
unnecessary metadata lookup. Do not change 4 KiB logical semantics merely to
reduce callback count.

### Flush/admission/futex wait dominates

Then inspect queue handoff, wakeup shape, worker ownership, transaction admission
and batching. Only at this point tune `workers_write` or admission defaults.

### COMMIT/WAL dominates

Then profile PostgreSQL durability/checkpoint/WAL behavior using the existing
local/QNAP methodology before changing database defaults.

## Worker policy

Do not set one worker count from the direct 128 MiB result alone.

Current direct data already shows workload-size dependence:

- 4 MiB peaked around 2-4 workers and regressed at 8;
- 128 MiB continued improving through 8 workers.

Therefore any production worker/admission default must be chosen only after the
mounted profile identifies whether FUSE-side work scales similarly.

A later adaptive policy is acceptable only if a fixed policy cannot cover the
measured workload classes cleanly and the adaptive decision can be bounded,
observable and deterministic enough for regression testing.

## Quota and reservation boundary

The late quota gate implemented in FOD 3.2.76 is considered complete for
ordinary writes unless new evidence shows a correctness or measurable
performance problem.

Do not widen the quota lock again to solve another bottleneck.

Reservation-token writes intentionally remain conservative. Evaluate
reservation-backed ordinary writes only if measurements near a full filesystem
show that late ENOSPC rollback wastes a material amount of COPY/merge work.
Do not reserve all dirty bytes blindly; any reservation must model positive
physical allocation growth.

## Correctness gates

Every optimization selected from this plan must preserve:

- canonical `data_blocks` storage only;
- 4 KiB logical block semantics;
- sparse-file `st_blocks` behavior;
- overwrite/truncate allocation correctness;
- shared `data_object_id` and copy-on-write behavior;
- CRC behavior;
- replay and ambiguous-COMMIT handling;
- database-wide quota and active reservations;
- two-mount quota safety;
- remount durability;
- failover/primary authority guarantees.

Do not accept a throughput improvement that weakens any of these contracts.

## Acceptance criteria for the profiling phase

The profiling phase is complete when:

1. repeated mounted 4 MiB and 128 MiB baselines are recorded;
2. the 128 MiB write wall time is mostly explained by named stages;
3. quota wait/held time remains negligible relative to the old pre-3.2.76
   serialization;
4. direct and mounted measurements are reconciled without treating overlapping
   counters as additive;
5. one dominant next optimization target is selected from evidence;
6. `TODO.md`, `conclusions.md`, `commands.md` and the canonical performance plan
   are updated with that measured decision before production code changes.

## Documentation authority

This document is the current detailed performance subplan.

Authority order:

1. `block-only-performance-plan.md` — canonical architecture and global order;
2. this file — current mounted-FUSE profiling and next-target selection;
3. `quota-lock-concurrency-plan.md` — completed quota-lock redesign record and
   reservation follow-up boundary;
4. `conclusions.md` — measured evidence;
5. `performance-baselines.md` — benchmark baselines.

## Delivery rule

A pure instrumentation change that is needed to locate the bottleneck should use
the next sequential FOD version, update documentation/tests and remain behavior
neutral.

After every commit compare it with its parent using `git diff HEAD~1..HEAD` or
`git show` and inspect accidental changes, missing files, regressions and
consistency with this plan before push.
