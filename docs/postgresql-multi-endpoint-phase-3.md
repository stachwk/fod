# PostgreSQL multi-endpoint routing: phase 3

Status: process-persistent endpoint health state and a four-lane connection-pool plan are implemented as shared runtime contracts; automatic routing and pool activation remain disabled.

## Goal

Turn the one-shot role probe from FOD 3.2.24 into reusable state that a long-running FOD process can retain across operations, and define deterministic capacity boundaries for future read, write, control, and lease pools before those pools are connected to production traffic.

This phase deliberately separates **observation and planning** from **activation**. It gives FUSE, the indexer, and administrative tools one canonical model without changing which PostgreSQL connection they use.

## Process-persistent health registry

`fod-rust-runtime` now provides `PgEndpointHealthRegistry`. Cloned registries share one `Arc<Mutex<...>>` state map, so observations survive across repository clones and operations for the lifetime of the process.

The registry records per endpoint:

- configured and observed role;
- role-match result;
- health state;
- consecutive and total successes/failures;
- last success and failure timestamps;
- a bounded, whitespace-normalized last error;
- eligibility for `read`, `write`, `control`, and `lease` purposes.

The default failure threshold is two consecutive failures:

- first failure: `degraded`;
- second consecutive failure: `unreachable`;
- successful probe: failure streak resets and health is reclassified.

Role mismatches and the impossible recovery/writable combination are represented explicitly as `role-mismatch` and `inconsistent`. Neither state is eligible for traffic.

A healthy replica is eligible only for reads. A healthy writable primary is eligible for all four purposes. A primary whose current session is read-only remains read-eligible but is not eligible for authoritative work.

The registry is intentionally **process-persistent, not database-persistent**. Persisting transient health in the FOD schema would create a new coordination problem and could make stale observations look authoritative after restart. A new process must probe again.

## Connection-pool plan

`PgPoolPlan` derives a deterministic plan from the existing `pool_max_connections` value.

When at least four connections are available, the plan uses dedicated logical lanes:

- one reserved control slot;
- one reserved lease/heartbeat slot;
- roughly one third of the remaining data slots for reads;
- the remaining data slots for writes.

For the current default limit of `10`, the plan is:

```text
read=2 write=6 control=1 lease=1
```

For limits below four, the plan reports `shared-fallback`. Every purpose sees the same global limit because strict four-way isolation cannot be guaranteed without silently increasing the configured connection budget.

The plan is a runtime contract only in this phase. `pool_plan_active` remains `false`.

## Diagnostics

`fod-config endpoint-probe` keeps all FOD 3.2.24 fields and adds:

```json
{
  "routing_enabled": false,
  "probe_only": true,
  "health_state_persistence": "process",
  "health_failure_threshold": 2,
  "pool_plan_active": false,
  "pool_plan": {
    "mode": "dedicated-lanes",
    "total_limit": 10,
    "read_limit": 2,
    "write_limit": 6,
    "control_limit": 1,
    "lease_limit": 1,
    "routing_enabled": false
  }
}
```

Each endpoint also exposes its health counters, timestamps, eligible purposes, and `automatic_routing_enabled: false`.

## Safety boundary

This phase does not modify `DbRepo` connection selection or its current physical pool. It does not:

- route reads to replicas;
- direct writes, schema operations, replay confirmation, locks, leases, or session heartbeats to endpoint lists;
- activate the four-lane plan;
- persist health to PostgreSQL;
- add a schema migration;
- enable automatic failover.

All production operations continue to use the legacy resolved DSN exactly as in FOD 3.2.24.

## Next phase

Wire the shared contracts into `rust_hotpath::pg::DbRepo`:

1. create separate cached lanes for `Read`, `Write`, `Control`, and `Lease`;
2. prevent write traffic from consuming reserved control and lease capacity;
3. update health on connect, query, and reconnect outcomes;
4. keep every lane on the verified legacy primary initially;
5. expose pool checkout wait, live/idle counts, and failure streaks;
6. route any read to a replica only after read-after-write and replay-LSN rules have dedicated integration coverage.

## Validation

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check --workspace --locked
cargo test --locked -p fod-rust-runtime
cargo test --locked -p fod-rust-mkfs
make test-version
fod-config endpoint-probe
```
