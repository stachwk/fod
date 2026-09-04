# FOD plans

This directory contains only maintained current/future planning documents. A
plan belongs here only while it still directs a future change; completed plans
move to [`../history/`](../history/).

For current implemented behavior use [`../CURRENT_STATE.md`](../CURRENT_STATE.md)
and the task guides from [`../README.md`](../README.md).

## Current implementation order

- [`CURRENT.md`](CURRENT.md) - compact maintained next-step plan.
- [`../../ROADMAP.md`](../../ROADMAP.md) - long-term project direction.
- [`../../TODO.md`](../../TODO.md) - mixed follow-up/archive record; use it as
  supporting context rather than as the source of current defaults.

## Future ideas

- [`FOD_FUTURE_IDEAS.md`](FOD_FUTURE_IDEAS.md) - longer-term ideas and
  proposals that are not yet current implementation commitments.

## Lifecycle rule

A document moves to [`../history/`](../history/) when its primary purpose becomes
explaining a completed decision, benchmark, migration or implementation sequence.
Do not keep a completed plan in this directory merely because its filename
contains `plan` or `project`.

The historical 2026-08-26 action plan, block-only write optimization plan,
mounted-FUSE write profile plan, quota-lock concurrency plan and transactional
replay project are retained under `../history/` and indexed by
[`../HISTORY.md`](../HISTORY.md).
