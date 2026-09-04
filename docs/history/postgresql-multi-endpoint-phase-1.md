# PostgreSQL multi-endpoint routing: phase 1

Status: implemented as the configuration and diagnostics foundation.

## Goal

Introduce an explicit, order-independent PostgreSQL endpoint model before any runtime query routing is enabled.

This phase deliberately does not redirect reads or writes. Existing FOD processes continue to use the legacy resolved `host` and `port` connection parameters until pool separation, endpoint probing, and consistency enforcement are implemented in later phases.

## Configuration contract

The `[database]` section accepts one of three forms.

### Legacy single endpoint

```ini
host = 127.0.0.1
port = 5432
```

This remains fully backward compatible. The endpoint role is reported as `unknown` and must be verified after connecting.

### Explicit roles

```ini
primary_hosts = 127.0.0.1:15432,127.0.0.1:15433
replica_hosts = 127.0.0.1:15442,127.0.0.1:15443
```

At least one primary is required. An endpoint may not occur more than once or appear in both role lists. List order has no role meaning.

### Transitional role discovery

```ini
hosts = 127.0.0.1:15432,127.0.0.1:15442
```

Every endpoint is marked `unknown`. A later runtime phase must discover roles using `pg_is_in_recovery()` and `transaction_read_only` before selecting an endpoint.

The transitional form cannot be combined with `primary_hosts` or `replica_hosts`.

## Environment overrides

- `FOD_PG_PRIMARY_HOSTS`
- `FOD_PG_REPLICA_HOSTS`
- `FOD_PG_HOSTS`
- existing `FOD_PG_HOST` and `FOD_PG_PORT` for the legacy form

Empty environment values do not erase non-empty configuration-file values. A non-empty endpoint-mode environment variable selects that mode over a different mode present in the configuration file. Setting both the explicit-role environment variables and `FOD_PG_HOSTS` is rejected as ambiguous.

## Validation

Endpoint lists use comma-separated `host:port` authorities. IPv6 addresses require bracket notation such as `[::1]:15432`. Ports must be in the range `1..65535`.

The parser rejects:

- explicit role lists mixed with transitional `hosts`;
- explicit role mode without any primary endpoint;
- duplicate endpoints, including case-insensitive hostname duplicates across roles;
- empty list elements;
- missing or invalid ports;
- unbracketed IPv6 authorities.

## Diagnostics

Run:

```bash
fod-config endpoint-config
```

The JSON output includes:

- configuration mode;
- `routing_enabled: false`, making the phase boundary explicit;
- whether role discovery is required;
- primary, replica, and unknown endpoint counts;
- normalized endpoint host, port, role, and authority.

Passwords and TLS private-key contents are not included.

## Next phase

The next implementation phase should move the endpoint model into the shared runtime layer and add connection-time role probes. It must verify writable endpoints before any write/control/lease operation and expose probe results without routing production traffic yet.

Only after that should FOD split write/control/lease pools from read pools and introduce read-after-write LSN rules.

## Validation commands

```bash
cargo test --locked -p fod-rust-mkfs
RUSTFLAGS="-D warnings" cargo check --workspace --locked
make test-version

./target/debug/fod-config endpoint-config
```
