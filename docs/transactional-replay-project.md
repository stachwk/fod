# Transactional Replay Project

This document records the shared transactional replay mechanism after PostgreSQL disconnects.
The reusable helper now lives in `rust_hotpath/src/pg.rs`, and the remaining notes focus on what still sits outside the replay envelope.

## Why This Matters

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
4. Use request tokens plus state verification as the default confirmation path.
   Keep a transaction journal only as a future fallback if a durable row marker is not available.
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

## Transactional Call-Site Inventory

The inventory below covers the transactional call sites in `rust_hotpath/src/pg.rs`.
It intentionally leaves out single-statement helpers that already sit inside the bounded replay classifier,
because this project is about commit-bound transactional replay rather than the existing safe command retry path.

### Replay-Safe Today

These wrappers are already close enough to idempotent replay to stay in the current bounded envelope:

| Function | Shape | Why it is safe today |
| --- | --- | --- |
| `ensure_lock_schema_ready()` | DDL bootstrap | `CREATE IF NOT EXISTS`, safe `ALTER`, and trigger/index setup can be repeated. |
| `ensure_client_session_schema()` | DDL bootstrap | Same idempotent bootstrap pattern as the lock schema path. |
| `prune_lock_leases()` | cleanup delete | Deletes expired rows only. |
| `prune_lock_range_leases()` | cleanup delete | Deletes expired rows only. |
| `prune_expired_client_sessions()` | cleanup delete | Removes expired sessions and stale zero-session leases deterministically. |
| `persist_lock_range_state_blob()` | delete + reinsert | Replaces the range-lease blob for the current scope. |
| `replace_lock_range_state_blob_for_owner()` | delete + reinsert | Same as above, but scoped to one owner key. |
| `persist_file_blocks_with_crc_flag()` | delete + upsert / COPY staging | The block persist path rewrites deterministic rows and now has commit-disconnect smoke coverage. |
| `persist_file_extents_with_crc_flag()` | COPY-based extent materialization | The extent persist path rewrites deterministic rows and now has commit-disconnect smoke coverage. |
| `acquire_flock_lease()` | advisory lock + request-token-backed upsert | The lease row is keyed and the durable request token confirms a replayed commit before the body runs again. |
| `persist_copy_block_crc_rows()` | delete + upsert | CRC rows are rewritten deterministically for the file/block set. |
| `set_file_size()` | single-row update | The write is keyed by `id_file` and only sets the current size. |
| `purge_primary_file()` | delete + row reassignment | A committed purge is observable because the file row disappears after reconnect. |
| `adopt_source_data_object()` | source/destination row confirmation | A committed adoption is observable because the destination file already points at the source data object with the expected size. |
| `create_data_object()` | request-token-backed insert/reuse | A committed object creation is observable because the durable token row returns the already-chosen `id_data_object` without replaying the refcount increment. |
| `promote_hardlink_to_primary()` | request-token-backed promotion | A committed promotion is observable because the durable token row records the outcome and prevents a second promotion on replay. |
| `touch_client_session_owner_key()` | session-owner-key upsert + session touch | A committed owner-key touch is observable through the durable owner-key row and the touched client-session row. |

The same helper now also short-circuits `set_file_size()` and `adopt_source_data_object()` on durable row probes.
`purge_primary_file()` stays on the older bounded replay shape for now because its cleanup branches are still easier to validate there than through a single durable marker.

### Replayable Only With Extra Confirmation

These wrappers mostly do the right thing, but the transaction as a whole still needs an idempotency key,
request token, or commit-outcome proof before automatic replay can be trusted:

The create-entry family now also uses `transactional_replayable()`, so a lost `COMMIT`
is retried once before the existing natural-key probe confirms the already-committed state.


| Function | Why it still needs more design |
| --- | --- |
| `create_hardlink()` | A natural-key unique violation can now be confirmed against the existing row after a replayed commit disconnect. |
| `create_symlink()` | A natural-key unique violation can now be confirmed against the existing row after a replayed commit disconnect. |
| `create_directory()` | A natural-key unique violation can now be confirmed against the existing row after a replayed commit disconnect. |
| `create_file()` | A natural-key unique violation can now be confirmed against the existing row after a replayed commit disconnect. |
| `create_special_file()` | A natural-key unique violation can now be confirmed against the existing row after a replayed commit disconnect. |

### Keep Out Of Automatic Replay

No explicit transactional wrappers remain in the out-of-envelope bucket. The remaining replay-sensitive paths now either use request-token confirmation or stay inside the bounded replay envelope as deterministic updates.

The bounded replay classifier already covers single-statement helpers such as `touch_data_object()`, `touch_file_entry()`,
`touch_directory_entry()`, `touch_symlink_entry()`, and the `rename_*()` / `delete_*()` helpers. They stay outside this
project because they do not cross an explicit transaction boundary.

## Replay Envelope and Outcome Confirmation

The first replay envelope should stay narrow and proof-driven:

- the transaction must carry a stable request identity that survives reconnects
- the final state must be observable from durable storage after reconnect
- repeating the body must not create a second visible outcome if the first commit already landed
- if any of those conditions is false, the transaction stays out of automatic replay

The preferred confirmation path is simple:

1. Reconnect on a fresh PostgreSQL connection.
2. Probe the durable row or rows keyed by the request identity.
3. If the probe shows a terminal committed state, return success and do not replay.
4. If the probe shows no committed state and the transaction is inside the envelope, replay once with the same request identity.
5. If the probe cannot distinguish committed from rolled back, fail closed.

For the current codebase, the practical marker is `request_token` on rows such as `index_scan_runs` and `index_import_plans`.
That makes the confirmation step a row-level probe instead of a separate transaction journal.
For filesystem object creation paths, the practical confirmation key is the natural unique key itself
(`parent + name`, plus the expected metadata fields); those wrappers now probe the existing row on a replayed
`SQLSTATE 23505` before they give up.
Any future transaction that cannot expose a durable marker with the same properties should remain outside the automatic replay envelope.

## Smoke Coverage

The transactional replay smoke suite now lives in `rust_hotpath/tests/transactional_replay_smoke.rs` and covers:

- body disconnects for directory creation
- multi-statement file creation
- commit disconnect confirmation through a durable `request_token` row
- commit-disconnect replay for `set_file_size()`
- commit-disconnect replay for `persist_copy_block_crc_rows()`
- commit-disconnect replay for `persist_file_blocks_with_crc_flag()`
- commit-disconnect replay for `persist_file_extents_with_crc_flag()`
- commit-disconnect replay for `promote_hardlink_to_primary()`
- commit-disconnect replay for `touch_client_session_owner_key()`
- commit-disconnect replay for lock-lease pruning and lock-range blob replacement

## Current Baseline

The current bounded replay stays in place as the default behavior.
This project should expand only what can be proven safe, not the whole transaction model at once.
