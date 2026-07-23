# PostgreSQL multi-endpoint phase 4: opt-in DbRepo lanes

## Status

Phase 4 wires the phase-3 health and pool-plan contracts into `fod-rust-fuse`
startup without enabling multi-endpoint routing. Stage 1 mounted correctness is
complete. Stage 2 observability now covers connection pools, transactions,
heartbeat timing, process memory, payload persistence, and portable
PostgreSQL pressure indicators. Logical queueing, memory-policy, and
endpoint-routing stages remain separate work.

The feature is opt-in through:

```text
FOD_PG_POOL_LANES_ENABLED=true
```

The default is `false`.

## Default compatibility path

When the flag is absent or false:

- the lane wrapper is bypassed completely;
- FOD creates one `DbRepo` with the existing `pool_max_connections` limit;
- the FOD 3.2.27 PostgreSQL diagnostics, startup snapshot, and mount path are retained;
- the same repository object is moved into `FodFuse` without an intermediate clone;
- no lane health probe is executed;
- all operations continue to use `FOD_DSN_CONNINFO`.

This is the normal production path.

## Opt-in lane path

When the flag is true and `pool_max_connections >= 4`, FOD creates four independent `DbRepo` instances:

| Lane | Purpose | Default share for total=10 |
| --- | --- | ---: |
| read | future eligible metadata and payload reads | 2 |
| write | authoritative filesystem mutations and current mount repository | 6 |
| control | startup, diagnostics, schema/control work | 1 |
| lease | future lock lease and heartbeat isolation | 1 |

Each repository receives the limit calculated by `PgPoolPlan`.

All four repositories still use exactly the same legacy DSN. This phase isolates connection capacity only; it does not select different PostgreSQL endpoints.

When the configured total is below four, the phase-3 `shared-fallback` plan remains active even if the flag is true. This avoids pretending that four independently reserved lanes fit into an undersized limit.

## FUSE startup integration

With opt-in enabled, `fod-rust-fuse`:

1. constructs `DbRepoLanes`;
2. logs the opt-in state and lane limits;
3. performs an exact, non-routing health probe using:

   ```sql
   SELECT pg_is_in_recovery()::text
       || '|'
       || current_setting('transaction_read_only')
   ```

4. stores the result in the process-local `PgEndpointHealthRegistry` under the non-secret label `legacy-dsn`;
5. reads PostgreSQL version diagnostics and the startup snapshot through the control lane;
6. moves the original write-lane repository into the existing `FodFuse` implementation;
7. keeps the read, control, and lease repositories alive for the duration of the mount.

A health-probe failure is diagnostic and does not replace the existing startup checks. Failure to read the normal startup snapshot remains fatal and is recorded in the health registry.

## Safety boundary

The following remain disabled:

- automatic endpoint routing;
- replica reads;
- primary failover;
- host selection by health score;
- read-after-write replica selection;
- WAL/LSN consistency gates;
- moving lock/session heartbeat operations to another endpoint.

The runtime health snapshot continues to expose `automatic_routing_enabled = false`, and the pool plan continues to expose `routing_enabled = false`.

The write lane is used for the existing mount repository. The read and lease repositories are retained as explicit capacity boundaries but are not yet selected by individual filesystem operations.

## Mounted integration coverage

`rust_fuse/tests/pg_lanes_mount.rs` starts a real FUSE mount with dedicated lanes enabled and a deterministic pool limit of ten. Its target coverage is:

- dedicated-lane startup diagnostics;
- startup snapshot reads through the control lane;
- lifetime of the three non-write repositories;
- create, write, read, rename, stat, and remove operations through the mounted filesystem;
- cleanup of the unique mounted path before unmounting.

The mounted smoke now passes the complete create, write, sync, read, rename,
stat, remove, and cleanup sequence. The investigation separated three
boundaries:

- mounted `CREATE` succeeds and returns an empty-file attribute row;
- the first write persists through the dedicated write lane;
- the write is persisted before the test's `sync_all()` call returns; the
  separate missing explicit fuser `fsync` callback remains tracked as a
  filesystem-durability follow-up.

The diagnostic run also found a malformed local test database whose initialized
schema had an empty `config` table. That state is not accepted as an empty-file
allocation. `startup_snapshot` now rejects an initialized schema with missing
or invalid `config.block_size`, missing or invalid
`config.max_fs_size_bytes`, or a missing schema-version row before mounting,
instead of deferring the failure to the first payload write.

## Test isolation

Existing FUSE benchmarks that validate the production compatibility path set:

```text
FOD_PG_POOL_LANES_ENABLED=0
```

explicitly when launching `fod-bootstrap`. This prevents a developer shell or test environment from accidentally converting a legacy regression test into an opt-in lane test.

The data-block conflict benchmark also appends the last 200 lines of its mount log to filesystem-operation failures. A raw `EIO` is therefore accompanied by the underlying FUSE or PostgreSQL diagnostic instead of being lost when the temporary mount workspace is removed.

## Memory-aware lane scaling requirements

Connection count, transfer size, and query memory are separate controls. FOD must not infer a larger PostgreSQL memory allowance from a larger transferred block, a larger `COPY` batch, or a larger number of concurrent file-copy tasks.

The implementation must preserve these invariants:

1. **Transfer size is independent from `work_mem`.** File payload blocks and `COPY` batches use bounded application buffers. They do not justify raising PostgreSQL sort/hash memory.
2. **Logical task concurrency is independent from active PostgreSQL backends.** FOD may accept hundreds of queued copy tasks while allowing only a measured number of transactions to execute concurrently.
3. **Memory is budgeted globally and per task.** The write path must enforce both a per-task buffer limit and a global in-flight byte limit.
4. **Backpressure is mandatory.** When the active-connection or in-flight-byte budget is exhausted, new tasks wait instead of allocating unbounded memory or opening unbounded connections.
5. **Write/copy connections use a low-memory profile.** Bulk transfer should favor short transactions, bounded batches, and predictable memory over large session-level memory settings.
6. **Search memory is granted only to selected queries.** Expensive search, sort, hash, and aggregation operations may use a larger transaction-scoped setting such as `SET LOCAL work_mem`, never a high global default for all FOD connections.
7. **Control and lease capacity remains protected.** Copy saturation must not consume the connections needed for startup checks, schema/control work, lock leases, or session heartbeats.
8. **A target such as 500 means logical tasks first, not automatically 500 physical PostgreSQL backends.** Direct backend counts must be selected from measured throughput, latency, server process overhead, and memory use.
9. **External pooling is optional, not assumed.** Transaction pooling such as PgBouncer may be evaluated for high logical concurrency, but FOD must remain correct without it.

## Lane memory profiles

### Write and copy profile

The write/copy lane should use:

- many queued logical tasks;
- a bounded number of active PostgreSQL transactions;
- small session memory settings;
- bounded per-task payload buffers;
- a global in-flight payload budget;
- short transactions and bounded `COPY` or batch sizes;
- adaptive concurrency only after measurements show that additional active connections improve throughput.

The implementation must avoid a model equivalent to `connection_count × large_buffer_per_connection`. Increasing concurrency must not multiply a large memory reservation across every backend.

### Search and analysis profile

The search/analysis lane should use:

- fewer active connections than bulk copy;
- a separate concurrency limit;
- larger memory only for queries proven to benefit from it;
- transaction-scoped `SET LOCAL work_mem` or an equivalent per-query mechanism;
- timeout and cancellation support so expensive searches cannot starve filesystem traffic.

One query may consume `work_mem` more than once for separate sort/hash nodes and parallel workers. Therefore a high global `work_mem` is explicitly prohibited as a scaling mechanism.

### Control and lease profile

The control and lease lanes should use:

- one or two reserved connections per purpose unless benchmarks justify another value;
- minimal session memory;
- no payload buffering;
- priority over bulk work when the server is saturated;
- independent health and wait-time diagnostics.

## Implementation backlog

### Stage 1: restore mounted correctness

Before increasing concurrency or routing individual operations:

- [x] Diagnose the mounted `CREATE`/write `EIO`.
- [x] Retain the full underlying PostgreSQL/FUSE error in diagnostics.
- [x] Prove create, write, read, rename, stat, remove, and cleanup through
  `pg_lanes_mount`.
- [x] Rerun the unchanged compatibility-path regressions.
- [x] Keep multi-endpoint routing disabled.

### Stage 2: add observability before tuning

Record at least:

- [x] Current and peak connection-pool acquisition waiters per lane.
- [x] Current and peak active PostgreSQL connections plus live and idle
  connections per lane.
- [x] Connection-acquisition wait totals and maxima per lane.
- [x] Connection creation counts, failures, and latency totals and maxima per
  lane.
- [x] Repository-operation counts, failures, replay counts, and latency totals
  and maxima per lane.
- [x] Configured persistence chunk blocks and COPY send-buffer bytes.
- [x] FOD process RSS snapshots after startup and after mount completion.
- [x] In-flight payload bytes globally and per lane.
- [x] Observed persistence operation sizes and cumulative rows, bytes, and
  elapsed time. Physical database batch sizes and completed files per second
  require the Stage 3 logical task boundary.
- [x] Transaction-specific latency and error counts, recorded for each
  transaction attempt separately from repository-operation timing.
- [x] Heartbeat scheduling delay, execution latency, and failure counts.
- [x] Periodic current and peak FOD process RSS during mounted workloads.
- [x] PostgreSQL activity, temporary-file, deadlock, memory-setting, and
  current diagnostics-backend memory snapshots.

No automatic concurrency adjustment should be added before these measurements are available.

The pool snapshots are cumulative and shared by all clones of one `DbRepo`.
They are emitted with non-secret lane labels at `post-startup`,
`startup-failed`, `periodic`, and `post-mount`. Periodic snapshots default to
five seconds and can be changed from 100 milliseconds to one hour with
`FOD_PG_OBSERVABILITY_INTERVAL_MS`. Each pressure snapshot issues one
control-plane SQL query. The instrumentation does not alter endpoint
selection, connection limits, replay policy, or automatic routing.

`operation_failures` counts repository closures that returned `Err`. It is a
diagnostic execution count, not yet a health score: operation classification
must separate expected application-level errors from connection, transaction,
and server failures before endpoint selection can consume it.

Transaction counters cover the exact `BEGIN` through `COMMIT` or `ROLLBACK`
scope and count bounded replay attempts independently. A transaction body may
return an expected application error, so `transaction_failures` is also a
diagnostic execution count rather than an endpoint health score.

Heartbeat counters cover one complete lock/session maintenance cycle. Until
operation routing is enabled, the heartbeat remains on the write repository;
the metric follows the repository that actually executed it instead of
pretending it already uses the reserved lease lane.

The PostgreSQL pressure snapshot uses portable cumulative statistics from
`pg_stat_database` and current activity from `pg_stat_activity`. It also records
effective `shared_buffers`, `work_mem`, `maintenance_work_mem`, and
`temp_buffers`. PostgreSQL 13 and newer additionally expose memory allocated by
the diagnostics query's own backend. This is not total server RSS and must be
combined with host or container measurements when sizing a production server.

Payload persistence has a separate cancellation-safe observation scope around
each block, extent, or streaming-file persistence call. The scope starts before
connection-pool acquisition and remains active across one bounded replay, so
its current and peak byte counts represent logical payload attributed to work
that FOD is actively trying to persist. Each dedicated lane has its own
tracker, and all lane repositories also share one process-wide tracker.

The payload snapshot records operation/failure counts, cumulative and maximum
input rows and bytes, and cumulative and maximum elapsed time. A streaming
file import attributes its logical file size to the operation even though its
resident application buffer remains bounded by the configured chunk and COPY
send-buffer sizes. These counters therefore describe persistence flow, not
process RSS or an enforced memory reservation. Counter overflow or release
underflow increments the explicit `payload_accounting_errors` diagnostic
instead of silently wrapping a value.

### Stage 3: separate queues from backend pools

Introduce:

- a logical task queue for bulk file operations;
- queued logical task classification by operation and lane;
- actual database batch-size and completed-file throughput measurements;
- a semaphore or equivalent active-transaction limit per lane;
- a global byte-budget permit for payloads in flight;
- a per-task buffer cap;
- cancellation-safe permit release;
- fairness so one large file cannot permanently block many small files;
- explicit backpressure diagnostics.

The queue depth may be high, including experiments with 500 logical tasks, while the active backend count remains independently configurable.

### Stage 4: apply operation-specific memory policy

Implement:

- a low-memory default for write/copy, control, and lease sessions;
- transaction-scoped search memory for approved search operations only;
- validation and safe upper bounds for every memory setting;
- rejection of configurations whose worst-case concurrent memory budget exceeds the configured FOD budget;
- diagnostics that report effective policy without exposing connection secrets.

### Stage 5: benchmark and adaptive scaling

Benchmark a matrix rather than assuming that more PostgreSQL backends are faster. Initial test points should include:

- logical copy tasks: 50, 100, 250, 500;
- active write connections: 10, 25, 50, 100, then higher only if throughput is still improving;
- multiple transfer block and batch sizes;
- mixed small-file and large-file workloads;
- concurrent search and control/lease activity;
- direct PostgreSQL connections and, separately, optional transaction-pool evaluation.

Stop increasing active connections when throughput plateaus, latency rises materially, control/lease responsiveness degrades, or memory growth becomes disproportionate.

## Provisional runtime controls

Names must be finalized against the existing runtime configuration conventions before implementation. The required concepts are:

| Provisional control | Meaning |
| --- | --- |
| `pg_copy_task_limit` | maximum queued or admitted logical copy tasks |
| `pg_write_active_connections` | maximum simultaneously active write/copy database operations |
| `pg_write_inflight_bytes` | global payload-byte budget for the write/copy lane |
| `pg_write_buffer_bytes_per_task` | maximum application buffer held by one task |
| `pg_write_batch_bytes` | bounded payload size for one database batch or `COPY` segment |
| `pg_search_active_connections` | separate active-query limit for search/analysis |
| `pg_search_work_mem` | validated value applied only inside selected search transactions |
| `pg_control_connections` | reserved control capacity |
| `pg_lease_connections` | reserved lease/heartbeat capacity |

The final names may differ, but the controls must remain independent. A single `pool_max_connections` value is insufficient for the target architecture.

## Acceptance criteria

The memory-aware lane work is complete only when local validation demonstrates:

- mounted filesystem correctness with lanes enabled;
- no regression on the default compatibility path;
- bounded FOD RSS under the configured in-flight byte budget;
- no global high-`work_mem` requirement;
- stable control and lease responsiveness during saturated copying;
- no deadlock or permit leak during errors and cancellation;
- throughput measurements across the concurrency matrix;
- a documented default that favors bounded memory and operational safety over maximum connection count;
- version metadata advanced only after the complete stage passes locally.

## Validation targets

Validation is performed locally; the project does not use GitHub Actions.

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check --workspace --locked
cargo test --locked -p fod-rust-runtime
cargo test --locked -p fod-rust-fuse
cargo test --locked -p fod-rust-mkfs
make test-version
```

Run the dedicated-lane mounted smoke explicitly:

```bash
cargo test --locked \
  -p fod-rust-fuse \
  --test pg_lanes_mount \
  -- --nocapture
```

The conflict benchmark exercises the unchanged compatibility path and large file creation:

```bash
cargo test --locked \
  -p fod-rust-fuse \
  --test data_blocks_conflict_benchmark \
  -- --nocapture
```

Expected log fields for the default total of ten in opt-in mode:

```text
opt_in_enabled=true
 dedicated_lanes_active=true
 mode=dedicated-lanes
 total=10
 read=2
 write=6
 control=1
 lease=1
 legacy_dsn_only=true
 routing_enabled=false
```

## Next phase

Stage 2 is complete for the current non-queued architecture. Stage 3 should
introduce the logical task boundary, classify queued work, and measure actual
batch and file completion rates before operation routing or concurrency
tuning. After those measurements are trustworthy, operation classification
inside the FUSE layer may proceed:

- direct read-only methods to the read lane;
- keep all mutations on the write lane;
- direct schema and administrative checks to the control lane;
- direct lock leases and session heartbeats to the lease lane.

That step must prove that moving an operation between independent connections does not weaken transaction, replay-confirmation, lock-owner, or session-heartbeat guarantees. Memory-aware queueing and connection limits must be implemented before attempting very high logical concurrency. Multi-endpoint host selection remains a later phase.
