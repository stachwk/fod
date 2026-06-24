# Transactional Replay Project

This document defines the separate project for full transactional replay after PostgreSQL disconnects.
It is intentionally split from the current bounded replay path so the existing retry contract stays stable.

## Why This Is Separate

The current hot-path replay already covers:

- read-only SQL
- idempotent command SQL
- a bounded set of replay-safe metadata and cleanup updates

That is a useful baseline, but it is not the same as replaying an arbitrary transaction end to end.
The remaining hard part is commit ambiguity: after a disconnect, the client cannot always know whether the server committed the transaction.

## Non-Goals

- Do not broaden the current bounded retry envelope implicitly.
- Do not auto-replay non-idempotent transactions.
- Do not treat a lost commit acknowledgement as a successful commit.
- Do not merge this work into unrelated indexer cleanup or adapter work.

## Project Scope

The project should focus on:

- inventorying transactional call sites
- classifying them by replayability
- defining an explicit replay envelope for the safe subset
- making commit outcome observable after a disconnect
- adding failure-in-the-middle smoke coverage

## Suggested Phases

1. Inventory transactional SQL call sites in `rust_hotpath/src/pg.rs`.
2. Split them into:
   - idempotent and replay-safe
   - replayable only with extra confirmation
   - non-replayable
3. Define how a transaction is identified across a reconnect.
4. Decide whether outcome confirmation comes from:
   - a transaction journal
   - request tokens plus state verification
   - a different server-side marker
5. Add smoke tests for:
   - disconnect during transaction body
   - disconnect during commit
   - reconnect and recovery
6. Keep rollout explicit and bounded until the project proves safe.

## Initial Candidate Areas

These are the first places worth rechecking once the project starts:

- lock acquisition and renewal
- shared-object purge and adoption
- metadata touch paths
- indexer plan and materialization transactions

## Current Baseline

The current bounded replay stays in place as the default behavior.
This project should expand only what can be proven safe, not the whole transaction model at once.
