# FOD current implementation plan

Status: 2026-09-13.

This file contains only work that is current enough to direct the next change.

For current implemented behavior use [`../CURRENT_STATE.md`](../CURRENT_STATE.md).
For long-term direction use [`../../ROADMAP.md`](../../ROADMAP.md).
`TODO.md` is a mixed follow-up/archive record rather than the implementation
queue.

The completed FOD 3.4.16-3.4.20 read/write optimization sequence is archived in
[`../history/FOD_CURRENT_PLAN_2026-09-08_READ_WRITE_TUNING.md`](../history/FOD_CURRENT_PLAN_2026-09-08_READ_WRITE_TUNING.md).

The completed FOD 3.4.23 cross-mount write ownership, stale-writer fencing and
FUSE hang-guard sequence is archived in
[`../history/FOD_CURRENT_PLAN_2026-09-12_WRITE_OWNERSHIP.md`](../history/FOD_CURRENT_PLAN_2026-09-12_WRITE_OWNERSHIP.md).

The completed P2 QNAP COPY-buffer repeatability follow-up is archived in
[`../history/FOD_CURRENT_PLAN_2026-09-12_QNAP_COPY_BUFFER.md`](../history/FOD_CURRENT_PLAN_2026-09-12_QNAP_COPY_BUFFER.md).
It produced no runtime tuning change: the current
`FOD_PERSIST_COPY_SEND_BUFFER_BYTES` default remains unchanged.

The completed P3 external-unmount/session-teardown validation is archived in
[`../history/FOD_CURRENT_PLAN_2026-09-13_EXTERNAL_UNMOUNT.md`](../history/FOD_CURRENT_PLAN_2026-09-13_EXTERNAL_UNMOUNT.md).
On the current `fuser 0.18.0` / libfuse3 stack the historical benign teardown
`EINVAL` was not reproduced, so no runtime teardown change was made.

The next active priority is P4.

## P4 — Compatibility diagnostics aggregation

The individual FUSE, PostgreSQL, libpq, runtime and storage-format boundaries
already expose substantial diagnostics.

Aggregate them only after each source remains trustworthy and machine-readable.
Do not introduce another compatibility abstraction merely to combine incomplete
or ambiguous signals.

Current execution sequence:

1. P4.1 completed in FOD 3.4.24: PostgreSQL/libpq/schema/storage diagnostics are
   machine-readable through versioned `fod-rust-mkfs status --json`;
2. P4.2 completed in FOD 3.4.25: negotiated FUSE/runtime compatibility is
   published through versioned shared monitor telemetry without parsing logs;
3. aggregate only those versioned sources in `fod-monitor report --json`;
4. add a compatibility summary only after source-level tests prove that missing
   data stays explicit rather than being guessed.

P4.3 is the active slice: aggregate the two versioned diagnostic sources in
`fod-monitor report --json` while preserving explicit unavailable/null state.

The teardown warning was closed separately as P3 and must not be hidden inside
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
