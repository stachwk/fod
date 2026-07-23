# PostgreSQL multi-endpoint phase 4: opt-in DbRepo lanes

## Status

Phase 4 wires the phase-3 health and pool-plan contracts into `fod-rust-fuse` startup without enabling multi-endpoint routing.

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

The write lane is used for the existing mount repository so the operation semantics are unchanged. The read and lease repositories are retained as explicit capacity boundaries but are not yet selected by individual filesystem operations.

## Test isolation

Existing FUSE benchmarks that validate the production compatibility path set:

```text
FOD_PG_POOL_LANES_ENABLED=0
```

explicitly when launching `fod-bootstrap`. This prevents a developer shell or test environment from accidentally converting a legacy regression test into an opt-in lane test.

The data-block conflict benchmark also appends the last 200 lines of its mount log to filesystem-operation failures. A raw `EIO` is therefore accompanied by the underlying FUSE or PostgreSQL diagnostic instead of being lost when the temporary mount workspace is removed.

Dedicated-lane behavior is covered separately by the `pg_lanes` unit tests and must later receive its own explicit mounted integration suite.

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

The conflict benchmark should be run first because it exercises mount startup and large file creation:

```bash
cargo test --locked \
  -p fod-rust-fuse \
  --test data_blocks_conflict_benchmark \
  -- --nocapture
```

Live startup should then be tested twice:

```bash
FOD_PG_POOL_LANES_ENABLED=false ...
FOD_PG_POOL_LANES_ENABLED=true ...
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

The next implementation step is operation classification inside the FUSE layer:

- direct read-only methods to the read lane;
- keep all mutations on the write lane;
- direct schema and administrative checks to the control lane;
- direct lock leases and session heartbeats to the lease lane.

That step must first prove that moving an operation between independent connections does not weaken transaction, replay-confirmation, lock-owner, or session-heartbeat guarantees. Multi-endpoint host selection remains a later phase.
