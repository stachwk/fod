# FOD current implementation plan

Status: 2026-09-08.

This file contains only work that is current enough to direct the next change.

For current implemented behavior use [`../CURRENT_STATE.md`](../CURRENT_STATE.md).
For long-term direction use [`../../ROADMAP.md`](../../ROADMAP.md).
`TODO.md` is a mixed follow-up/archive record rather than the implementation
queue.

The completed FOD 3.4.16-3.4.20 read/write optimization sequence is archived in
[`../history/FOD_CURRENT_PLAN_2026-09-08_READ_WRITE_TUNING.md`](../history/FOD_CURRENT_PLAN_2026-09-08_READ_WRITE_TUNING.md).

## P1 — Cross-mount destination write serialization

Protect one destination pathname against concurrent write/copy activity from
independent FOD mounts, processes or machines.

Required boundary:

- coordination is authoritative in PostgreSQL rather than process-local;
- destination ownership is first-writer-wins: the first writer atomically
  acquires ownership and continues, while every later concurrent writer uses
  non-blocking try-acquire and fails immediately rather than waiting;
- the losing writer must not truncate, write or otherwise mutate destination
  payload or metadata before ownership is granted;
- concurrent writers never interleave into one logical destination file;
- operations that need multiple namespace resources, especially rename/replace,
  acquire them in one deterministic global order or fail without waiting, so
  destination ownership cannot introduce a wait cycle or deadlock;
- direct create/truncate paths and temporary-file-plus-rename workflows obey
  the same destination ownership contract;
- crash/disconnect cannot leave a permanent lock, orphan payload, leaked quota
  reservation or inconsistent metadata; stale writers are fenced after lease
  expiry/recovery;
- a crashed direct POSIX writer may leave a valid prefix from that one writer,
  but never mixed blocks from multiple writers; temp-file-plus-rename keeps
  atomic replacement semantics;
- add a two-mount/two-host regression covering identical and different source
  content, prompt loser failure, zero loser writes, final size/content/hash,
  no deadlock, stale-writer fencing, remount stability and cleanup.

This is the highest-priority correctness follow-up.

## P2 — QNAP PostgreSQL baseline follow-up

The stable QNAP server preset is already implemented and validated for the
current reference host:

```text
8 GB RAM
2 CPU
HDD
PostgreSQL 16.15
BLCKSZ = 32 KiB
```

`QNAP=1` owns that profile through Make/Compose. Do not retune it merely because
a generic PostgreSQL recommendation differs.

The remaining performance follow-up is conditional:

- repeat the QNAP `FOD_PERSIST_COPY_SEND_BUFFER_BYTES` matrix before changing
  its default;
- require repeated evidence that `4194304` is a stable improvement rather than
  network, Docker or HDD noise;
- leave the current default unchanged without that repeated evidence.

## P3 — External unmount/session teardown

Reproduce the remaining external-unmount/session-teardown warning on the current
`fuser 0.18` / libfuse3 stack.

Required boundary:

- confirm that `fusermount3 -u` leaves no mount behind;
- determine whether session drop still reports the benign `EINVAL`;
- compare current behavior with the verified 2026-07-12 migration-gate result;
- do not fork `fuser`, suppress unrelated warnings or weaken cleanup semantics
  merely to hide the message.

Change teardown behavior only if the warning is reproduced on the current stack
and the public session API provides a correctness-preserving solution.

## P4 — Compatibility diagnostics aggregation

The individual FUSE, PostgreSQL, libpq, runtime and storage-format boundaries
already expose substantial diagnostics.

Aggregate them only after each source remains trustworthy and machine-readable.
Do not introduce another compatibility abstraction merely to combine incomplete
or ambiguous signals.

The teardown warning is tracked separately as P3 and must not be hidden inside
the diagnostics-aggregation task.

## Architecture guardrails

The broad role-aware PostgreSQL routing project is complete: startup role
selection, runtime primary failover, WAL-gated replica reads, replica scoring,
promotion validation and process-local generation fencing are implemented.

Do not reopen that design as a generic project.

Further multi-endpoint work requires a concrete measured or correctness gap,
for example external/cross-process fencing with an explicit ownership and
failure model.

Independent writable PostgreSQL primaries must never be presented as one safe
multi-primary FOD filesystem.

## Delivery rule

Each implementation change stays on `main`, updates the relevant
documentation/tests and follows the repository versioning policy.

After every commit compare it with its parent using `git diff HEAD~1..HEAD` or
`git show`, inspect the complete file set and run `git diff --check`.

Do not add or modify GitHub Actions workflows.

## Historical plans

Completed execution plans and measurement records belong under
[`../history/`](../history/). They explain how the current state was reached but
must not compete with this file for current priority.
