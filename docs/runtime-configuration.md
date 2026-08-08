# FOD runtime configuration

## Precedence

For normal mounts through `fod-bootstrap`:

1. `[fod]` supplies persistent settings;
2. a selected `[fod.profile.<name>]` may override base values;
3. explicit `FOD_*` environment variables override INI;
4. bootstrap CLI options override startup fields they own.

FOD 3.2.61 makes FUSE startup/admission controls available through INI while
preserving environment overrides.

## FUSE concurrency and logical admission

### `fuse_event_threads`

Environment override: `FOD_FUSE_EVENT_THREADS`.

Number of `fuser` event-loop workers. Valid range: `1..=256`. Historical
fallback when not configured is `1`. FOD 3.2.60 confirmed `8` as the best
tested value when paired with write admission `4`.

### `fuse_clone_fd`

Environment override: `FOD_FUSE_CLONE_FD`.

Boolean `fuser::Config::clone_fd` control. FOD 3.2.58–3.2.60 used `false`.
Keep it disabled unless a separate benchmark proves a benefit.

### `task_read_active_limit`

Environment override: `FOD_TASK_READ_ACTIVE_LIMIT`.

Non-negative process-local logical read admission limit. `0` disables
enforcement while retaining observability. Current recommended value: `0`.

### `task_write_active_limit`

Environment override: `FOD_TASK_WRITE_ACTIVE_LIMIT`.

Non-negative process-local logical write/copy admission limit. `0` disables
enforcement. Current recommended value: `4` with eight FUSE event threads.

The effective positive write concurrency cannot exceed the smaller of event
threads and the admission limit.

FOD 3.2.60 confirmed `8/4` over ten runs:

- throughput median `121.677 MiB/s`;
- throughput improvement vs `8/0`: `54.02%`;
- small-write median improvement: `31.44%`;
- small-write p95 improvement: `29.95%`;
- stability CV: `18.63%`;
- peak active writes: exactly `4`;
- peak queued writes: `5–6`;
- write overlap: `100%`.

## Additional FUSE startup controls

### `allow_other`

Environment override: `FOD_ALLOW_OTHER`. Boolean FUSE session access policy.
The base INI keeps it `false`.

### `entry_timeout_seconds`

Environment override: `FOD_ENTRY_TIMEOUT_SECONDS`. Optional non-negative FUSE
entry-cache timeout. Kept commented in base INI to preserve defaults.

### `attr_timeout_seconds`

Environment override: `FOD_ATTR_TIMEOUT_SECONDS`. Optional non-negative FUSE
attribute-cache timeout. Kept commented by default.

### `negative_timeout_seconds`

Environment override: `FOD_NEGATIVE_TIMEOUT_SECONDS`. Optional non-negative
negative-lookup timeout. Kept commented by default.

## Environment-only audit

Run:

```bash
python3 scripts/audit_runtime_env_ini.py \
  --root . \
  --output docs/runtime-env-ini-audit.md \
  --check
```

The generated report scans production Rust sources and distinguishes settings
present in INI from secrets, selectors, internal handoffs and diagnostic/test
variables that intentionally remain environment-only.

## FOD 3.2.61 production validation

The final validation compares `8/4` with `8/0` across sequential, mixed and
random-mixed FUSE workloads, one strace-backed sequential pass per
configuration, the mount suite under `8/4`, and an approximately five-minute
endurance phase (150 seconds per configuration by default).

`pg_stat_database` snapshots are taken around normal and endurance workloads.
The report includes transaction counts, cache/block counters and
`active_time_per_transaction_ms_proxy`. This metric is a database active-time
per transaction proxy, not direct client transaction latency.

## Environment audit decisions in FOD 3.2.61

The environment audit now scans only quoted `FOD_*` literals. This avoids
misclassifying Rust identifiers, such as profiling aggregate statics, as
runtime environment variables.

Persistent/documented configuration in this stage includes the confirmed FUSE
`8/4` controls, `allow_other`, optional FUSE cache timeouts, optional SELinux
context overrides, and the PostgreSQL-visible-path override.

The following controls intentionally remain environment-only:

- `FOD_PROFILE_IO`, `FOD_PROFILE_IO_VERBOSE`, `FOD_STRACE*` — profiling/test instrumentation;
- `FOD_PG_OBSERVABILITY_INTERVAL_MS` — PostgreSQL lane sampling cadence;
- `FOD_PG_POOL_LANES_ENABLED` — temporary opt-in for dedicated PostgreSQL lane pools until routing is finalized;
- `FOD_PERSIST_COPY_SEND_BUFFER_BYTES` — experimental libpq COPY send-buffer tuning, default 1 MiB;
- `FOD_INDEXER_CONNINFO` — indexer-specific PostgreSQL connection override/handoff;
- `FOD_REQUESTED_CAPABILITIES` — FUSE capability/compatibility override;
- `FOD_RUNTIME_SCHEMA_DDL_LOCK_SQL` and `FOD_RUNTIME_SCHEMA_DDL_UNLOCK_SQL` — internal/test SQL hooks;
- `FOD_PROCESS_NAMES` and `FOD_VERSION` — standalone monitor controls;
- `FOD_VERSION_LABEL` — build-time version label, not runtime configuration.

Any remaining `env-only-or-incomplete-ini` row in
`docs/runtime-env-ini-audit.md` remains an explicit review item instead of
being silently promoted to persistent configuration.

## PostgreSQL telemetry harness connection precedence

The production-validation telemetry harness follows the Makefile/runtime
connection identity:

1. `FOD_PG_HOST`, `FOD_PG_PORT`, `FOD_PG_DBNAME`, `FOD_PG_USER`,
   `FOD_PG_PASSWORD`;
2. compatible `POSTGRES_*` values where a corresponding FOD value is absent;
3. local development fallbacks.

Diagnostics may report which environment-variable name supplied each
non-secret field. They never report the password value.

## PostgreSQL transaction admission controls

`pg_write_transaction_limit` and `pg_control_transaction_limit` are startup-only
process-local controls propagated to `FOD_PG_WRITE_TRANSACTION_LIMIT` and
`FOD_PG_CONTROL_TRANSACTION_LIMIT`.

- `0` disables the corresponding transaction gate.
- positive values bound concurrently active explicit PostgreSQL transactions.
- queue wait happens after connection acquisition but before `BEGIN`.
- permit lifetime ends after commit/rollback/error handling.
- write and control/lease lanes are independent.
- these controls do not replace `pool_max_connections`.

The base INI values are `4` for write and `2` for control/lease.

## PostgreSQL payload byte budget

`pg_payload_in_flight_limit_bytes` is propagated at startup to
`FOD_PG_PAYLOAD_IN_FLIGHT_LIMIT_BYTES` and accepts the same byte-size syntax as
other FOD size settings, for example `64MiB`.

- `0` disables the byte-admission gate.
- the budget is process-local and shared across PostgreSQL lanes;
- admission is FIFO and based on logical persist input bytes;
- the permit is acquired before the persist operation becomes active and is
  released by RAII after success or failure;
- a request larger than the limit may run only as the sole reserved payload;
- transaction limits and connection-pool limits remain independent.

The base configuration uses `64MiB`.

## PostgreSQL endpoint startup routing

`pg_endpoint_routing_enabled` is propagated to
`FOD_PG_ENDPOINT_ROUTING_ENABLED`. The base INI enables it, while legacy
single-node `host` + `port` keeps existing single-DSN behavior. Enabling
endpoint routing also selects the `pg_lanes` startup path even when dedicated
pool lanes are disabled; in that case the selected endpoint is used by the
shared repository pool.

For multi-endpoint configuration:
- endpoint roles come from `primary_hosts` / `replica_hosts` or live discovery,
  never list position;
- `primary` requires a healthy writable primary;
- `replica` requires an observed healthy replica;
- `auto` prefers writable primary and otherwise may select a healthy read-only
  endpoint;
- the selected endpoint overrides only host/port in the base DSN, preserving
  database, credentials and TLS settings;
- writable mounts stay pinned to the selected primary in FOD 3.2.65.
