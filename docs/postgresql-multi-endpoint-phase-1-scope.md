# PostgreSQL multi-endpoint phase 1 scope

Included:

- explicit primary and replica endpoint lists;
- transitional endpoint lists requiring role discovery;
- environment-mode overrides;
- strict endpoint validation;
- JSON diagnostics through `fod-config endpoint-config`;
- regression coverage for successful and rejected configurations.

Excluded intentionally:

- opening connections to the listed endpoints;
- changing the active runtime connection parameters;
- routing reads to replicas;
- routing writes, locks, leases, schema work, or control operations;
- endpoint health scoring, failover, LSN tracking, or circuit breakers.

The diagnostic output must therefore continue to report `routing_enabled: false` until a later phase explicitly changes the runtime connection architecture.
