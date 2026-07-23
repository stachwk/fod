# PostgreSQL runtime requirements

FOD validates the PostgreSQL settings required for correctness when its
database-backed tools start. The canonical setting names, minimum values,
session setup SQL, and result evaluator live in
`rust_runtime/src/postgres_requirements.rs`.

## Validation rule

FOD queries a deliberate subset of `pg_settings` rather than treating every
row from `SHOW ALL` as a requirement. This keeps correctness requirements
separate from workload-specific performance tuning.

The startup query reads:

- the effective value;
- the unit;
- the PostgreSQL setting context;
- `pending_restart`.

Connection strings and passwords are never included in requirement messages.

## Per-session enforcement

Every new physical PostgreSQL connection applies:

```sql
SET TIME ZONE 'UTC';
SET SESSION default_transaction_isolation TO 'read committed';
SET SESSION statement_timeout TO 0;
SET SESSION lock_timeout TO 0;
SET SESSION standard_conforming_strings TO on;
```

PostgreSQL 9.6 and newer also receive:

```sql
SET SESSION idle_in_transaction_session_timeout TO 0;
```

FOD verifies the effective values through `pg_settings`. A failure to apply or
verify these session requirements rejects the connection with the setting name
and PostgreSQL error.

The configured FOD `synchronous_commit` value remains a separate explicit
runtime policy. It is not silently replaced by this minimum-requirements check.

## Instance requirements

The following values cannot be safely repaired only inside one FOD session:

| Setting | Required value | Startup behavior |
| --- | --- | --- |
| `server_version_num` | `90500` or newer | reject startup and request a PostgreSQL upgrade |
| `max_connections` | `pool_max_connections + 2` or higher | warn with the required value and setting context |
| `fsync` | `on` | warn that crash-safe WAL flushing must be enabled |
| `full_page_writes` | `on` | warn that full-page WAL protection must be enabled |

Warnings include `context` and `pending_restart` from `pg_settings`, so an
operator can distinguish a reloadable setting from one requiring a PostgreSQL
restart. FOD does not execute `ALTER SYSTEM`, edit `postgresql.conf`, reload,
or restart a remote instance automatically.

`fod-rust-fuse`, the opt-in PostgreSQL lane path, `fod-indexer`, and
`mkfs.fod` all consume the shared contract. `mkfs.fod`, `fod-config`, and
`fod-change` also use the shared per-session setup through their common
connection wrapper.

## Non-requirements

FOD does not classify `work_mem`, `shared_buffers`, `max_wal_size`,
`checkpoint_timeout`, parallel-worker counts, or planner cost values as fixed
correctness minima. Those values depend on the server, workload, and measured
resource budget and remain benchmark-driven tuning.
