# FOD execution roadmap after 3.2.61

This roadmap records the implementation order agreed after the FUSE admission
confirmation and production validation. Each stage must keep correctness and
observability ahead of automatic tuning.

## FOD 3.2.62 — PostgreSQL telemetry reliability

Goal: eliminate the `pg_stat_database` snapshot warnings seen during the 3.2.61
production validation and make telemetry failure explicit.

Implementation:
- resolve telemetry connections from the same exported `FOD_PG_*` variables
  used by FOD and Makefile targets;
- retain `POSTGRES_*` only as a compatibility fallback;
- never print the PostgreSQL password;
- add a fast real-workload telemetry smoke test with before/after
  `pg_stat_database` snapshots;
- make production validation fail when required PostgreSQL snapshots are
  unavailable instead of silently accepting warning-only telemetry;
- keep production FUSE and hot-path behavior unchanged.

Acceptance:
- `make test-fuse-postgres-telemetry` succeeds against the selected backend;
- before and after snapshots are available;
- the smoke workload produces a positive transaction delta;
- the report records the selected host/port/database/user and environment source
  names without exposing the password;
- normal production validation cannot finish with unresolved snapshot warnings.

## FOD 3.2.63 — PostgreSQL transaction admission

Status: implemented.

Add explicit limits at the PostgreSQL transaction boundary rather than only at
the FUSE callback boundary. Preserve cancellation safety, accounting and
read/write/control/lease separation.

Implementation:
- acquire a process-local FIFO transaction permit immediately before `BEGIN`;
- release it by RAII after `COMMIT`, `ROLLBACK`, disconnect/replay, or error;
- keep independent write and control/lease gates;
- keep zero as the disabled compatibility fallback;
- persist balanced base values `4` for write and `2` for control/lease;
- expose active, queued, peak, backpressure, fairness and accounting metrics;
- verify limits with real PostgreSQL transactions held by `pg_sleep()`.

Acceptance:
- six concurrent write-lane workers never exceed two active transactions when
  the test limit is `2`;
- four control-lane workers never exceed one active transaction when the test
  limit is `1`;
- both lanes return to zero active/queued with zero accounting errors;
- sequential strace and mixed/random-mixed FUSE tests pass with I/O profiling.

## FOD 3.2.64 — Payload byte budget and backpressure

Status: implemented.

Bound payload bytes in flight independently of task count. Integrate the budget
with admission observability and verify that large requests cannot starve small
operations or exhaust memory.

Implementation:
- one process-local global payload budget shared by PostgreSQL repository lanes;
- FIFO admission based on requested payload bytes;
- base limit `64MiB`, with `0` preserving disabled compatibility behavior;
- an oversized request is admitted alone when the budget is otherwise empty,
  avoiding deadlock while preventing concurrent payload amplification;
- observability for reserved/queued bytes, queue depth, peaks, admissions,
  backpressure, fairness, oversized requests and accounting failures;
- integration at `PayloadPersistGuard`, before persist work is counted active;
- Makefile secret-echo audit and silent execution for password-bearing recipes.

Acceptance:
- a blocked payload request stays queued until enough bytes are released;
- reserved and in-flight counters return to zero;
- oversized single requests do not deadlock;
- accounting errors stay zero;
- hot-path fio/strace and telemetry smoke continue to pass;
- Makefile audit finds no echoable password-bearing recipe command.

## FOD 3.2.65 — Role-aware startup routing and failover

Status: implemented.

- bootstrap propagates configured endpoint lists to Rust FUSE;
- startup probes all candidates before repository pools are created;
- `primary` selects only a healthy writable primary;
- `replica` selects only an observed healthy replica;
- `auto` prefers writable primary and can fall back to a healthy read-only endpoint;
- writable mounts remain primary-pinned for read-after-write consistency;
- startup failover skips unavailable/ineligible candidates in configured order;
- diagnostics expose endpoint mode, candidate count, selected authority and
  startup failover count;
- a real mount test refuses the first primary and proves write/read through the
  second primary.

## FOD 3.2.66 — Runtime primary failover

Status: implemented.

- share one runtime target generation across the current PostgreSQL lane
  repositories;
- rotate to the next configured primary entrypoint after a replayable
  connection failure;
- revalidate every newly opened failover target as a writable primary;
- tag cached connections with their routing generation and discard stale
  generations after a target transition;
- preserve existing one-replay and durable replay-confirmation behavior;
- expose active authority, generation, failure/failover counters, role
  rejections and stale cached-connection discards;
- verify runtime failover by terminating a live PostgreSQL backend and requiring
  the replayed query to complete through the second primary target.

## FOD 3.2.67+ — Replica consistency, read routing and advanced failover

- add primary write-LSN capture and replica replay-LSN checks;
- route eligible reads to replicas only after read-after-write consistency can
  be proved or after explicit stale-tolerant classification;
- add endpoint latency, replica lag and pool-pressure scoring;
- add hysteresis/circuit-breaker cooldown behavior;
- cover promotion, split-brain prevention and lock/lease safety across failover;
- add cross-process fairness where required.

The confirmed `8/4` FUSE admission configuration remains the production
candidate while these later layers are implemented.
