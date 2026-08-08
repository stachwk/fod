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

Add explicit limits at the PostgreSQL transaction boundary rather than only at
the FUSE callback boundary. Preserve cancellation safety, accounting and
read/write/control/lease separation.

## FOD 3.2.64 — Payload byte budget and backpressure

Bound payload bytes in flight independently of task count. Integrate the budget
with admission observability and verify that large requests cannot starve small
operations or exhaust memory.

## FOD 3.2.65+ — Role-aware multi-endpoint routing

Continue the existing multi-endpoint roadmap:
- verified primary-only write/control/lease routing;
- replica-eligible read routing with read-after-write consistency;
- endpoint health, latency, lag and pool-pressure scoring;
- hysteresis/circuit breaker behavior;
- promotion/failover and split-brain safety tests;
- cross-process fairness where required.

The confirmed `8/4` FUSE admission configuration remains the production
candidate while these later layers are implemented.
