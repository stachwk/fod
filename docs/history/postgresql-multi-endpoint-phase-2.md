# PostgreSQL multi-endpoint routing: phase 2

Status: shared runtime model and live role probes implemented; automatic routing remains disabled.

## Goal

Move the endpoint model introduced in FOD 3.2.23 into `fod-rust-runtime` and verify each configured PostgreSQL endpoint against the server itself before any operation-aware routing is introduced.

## Shared runtime contract

`fod-rust-runtime` now owns:

- endpoint configuration parsing and validation;
- configured endpoint roles (`primary`, `replica`, and `unknown`);
- endpoint-specific connection parameter derivation;
- probe result parsing and observed-role classification.

`rust_mkfs/src/pg_config.rs` remains only as a compatibility re-export layer. Future FUSE, indexer, and control-plane code can therefore consume one canonical endpoint model without copying the parser.

## Live probe

Run:

```bash
fod-config endpoint-probe
```

For every configured endpoint FOD opens an independent diagnostic connection and reads:

```sql
SELECT pg_is_in_recovery()::text
       || '|'
       || current_setting('transaction_read_only');
```

The result is classified as:

- `primary-writable`: the server is not in recovery and the session is writable;
- `primary-read-only`: the server is not in recovery but `transaction_read_only` is enabled;
- `replica`: the server is in recovery and the session is read-only;
- `inconsistent-recovery-writable`: recovery reports a replica while the session reports writable transactions.

The command reports connectivity, configured and observed roles, write capability, consistency, and role mismatch for every endpoint. Connection errors are returned per endpoint so one failed host does not hide results from the others.

## Safety boundary

The JSON output continues to include:

```json
{
  "routing_enabled": false,
  "probe_only": true
}
```

No read, write, schema, lock, lease, session, replay-confirmation, or maintenance operation consumes the probe result for endpoint selection in this phase. Existing production paths continue to use the legacy resolved `host` and `port` connection.

A configured `primary` matches both a writable primary and a server-level primary whose current session is read-only. Only `primary-writable` is considered write-capable. A configured `replica` matches only the recovery/read-only combination.

## Next phase

Introduce persistent endpoint health state and separate connection pools, initially keeping all authoritative operations primary-only. Routing must remain opt-in until failover revalidation and read-after-write consistency rules are implemented and tested.

## Validation

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check --workspace --locked
cargo test --locked -p fod-rust-runtime
cargo test --locked -p fod-rust-mkfs
make test-version
fod-config endpoint-probe
```
