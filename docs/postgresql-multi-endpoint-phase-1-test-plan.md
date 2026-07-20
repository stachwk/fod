# PostgreSQL endpoint configuration phase 1: local test plan

Run the following on the FOD development host before merge:

```bash
cargo fmt --all -- --check
cargo test --locked -p fod-rust-mkfs
RUSTFLAGS="-D warnings" cargo check --workspace --locked
make test-version

./target/debug/fod-config endpoint-config
```

Then create temporary configurations for the explicit and discovery modes and verify:

```bash
FOD_PG_PRIMARY_HOSTS='127.0.0.1:15432,127.0.0.1:15433' \
FOD_PG_REPLICA_HOSTS='127.0.0.1:15442,127.0.0.1:15443' \
./target/debug/fod-config endpoint-config

FOD_PG_HOSTS='127.0.0.1:15432,127.0.0.1:15442' \
./target/debug/fod-config endpoint-config
```

Expected for both commands:

- valid JSON;
- `routing_enabled` is `false`;
- endpoint counts match the provided lists;
- explicit mode reports primary/replica roles;
- discovery mode reports unknown roles and `role_discovery_required: true`.
