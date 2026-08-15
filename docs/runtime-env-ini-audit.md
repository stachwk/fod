# Runtime environment vs INI audit

Generated from quoted `FOD_*` literals in production Rust sources. Rust identifiers with similar names are excluded.

| environment variable | expected INI key | classification | active in | documented in | source | note |
| --- | --- | --- | --- | --- | --- | --- |
| `FOD_ACL` | `[fod] acl` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_ALLOW_OTHER` | `[fod] allow_other` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_fuse/src/startup.rs, rust_mkfs/src/config.rs | - |
| `FOD_ATIME_POLICY` | `[fod] atime_policy` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_ATTR_TIMEOUT_SECONDS` | `[fod] attr_timeout_seconds` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_fuse/src/startup.rs, rust_mkfs/src/config.rs | - |
| `FOD_CONFIG` | `[fod] config` | intentional-env-only | - | - | rust_mkfs/src/bin/fod-bootstrap.rs, rust_runtime/src/ini_config.rs | config file selector |
| `FOD_COPY_DEDUPE_CRC_TABLE` | `[fod] copy_dedupe_crc_table` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_hotpath/src/ffi.rs, rust_runtime/src/lib.rs | - |
| `FOD_COPY_DEDUPE_ENABLED` | `[fod] copy_dedupe_enabled` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_COPY_DEDUPE_MAX_BLOCKS` | `[fod] copy_dedupe_max_blocks` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_COPY_DEDUPE_MIN_BLOCKS` | `[fod] copy_dedupe_min_blocks` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_DATA_OBJECT_SWAP_CLEANUP` | `[fod] data_object_swap_cleanup` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_DEBUG` | `[fod] debug` | intentional-env-only | - | - | rust_mkfs/src/bin/fod-bootstrap.rs | diagnostic bootstrap override |
| `FOD_DEBUG_SNAPSHOT` | `[fod] debug_snapshot` | intentional-env-only | - | - | rust_fuse/src/startup.rs | diagnostic snapshot |
| `FOD_DEFAULT_PERMISSIONS` | `[fod] default_permissions` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_DIRSYNC` | `[fod] dirsync` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_DSN_CONNINFO` | `[fod] dsn_conninfo` | intentional-env-only | - | - | rust_fuse/src/main.rs, rust_mkfs/src/bin/fod-bootstrap.rs | internal bootstrap-to-FUSE PostgreSQL handoff |
| `FOD_ENTRY_TIMEOUT_SECONDS` | `[fod] entry_timeout_seconds` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_fuse/src/startup.rs, rust_mkfs/src/config.rs | - |
| `FOD_FOPEN_DIRECT_IO` | `[fod] fopen_direct_io` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_FUSE_CLONE_FD` | `[fod] fuse_clone_fd` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_fuse/src/startup.rs, rust_mkfs/src/config.rs | - |
| `FOD_FUSE_EVENT_THREADS` | `[fod] fuse_event_threads` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_fuse/src/startup.rs, rust_mkfs/src/config.rs | - |
| `FOD_FUSE_WRITEBACK_CACHE` | `[fod] fuse_writeback_cache` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_INDEXER_CONNINFO` | `[fod] indexer_conninfo` | intentional-env-only | - | - | rust_indexer/src/db.rs | indexer-specific PostgreSQL conninfo override/handoff |
| `FOD_LAZYTIME` | `[fod] lazytime` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_LOCK_BACKEND` | `[fod] lock_backend` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_LOCK_HEARTBEAT_INTERVAL_SECONDS` | `[fod] lock_heartbeat_interval_seconds` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_LOCK_LEASE_TTL_SECONDS` | `[fod] lock_lease_ttl_seconds` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_LOCK_POLL_INTERVAL_SECONDS` | `[fod] lock_poll_interval_seconds` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_LOG_LEVEL` | `[fod] log_level` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_fuse/src/main.rs, rust_mkfs/src/bin/fod-bootstrap.rs, rust_runtime/src/lib.rs | - |
| `FOD_MAX_FS_SIZE_BYTES` | `[fod] max_fs_size_bytes` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_METADATA_CACHE_TTL_SECONDS` | `[fod] metadata_cache_ttl_seconds` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_NEGATIVE_TIMEOUT_SECONDS` | `[fod] negative_timeout_seconds` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_fuse/src/startup.rs, rust_mkfs/src/config.rs | - |
| `FOD_PERSIST_BLOCK_TRANSPORT` | `[fod] persist_block_transport` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_PERSIST_BUFFER_CHUNK_BLOCKS` | `[fod] persist_buffer_chunk_blocks` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_PERSIST_COPY_SEND_BUFFER_BYTES` | `[fod] persist_copy_send_buffer_bytes` | intentional-env-only | - | - | rust_hotpath/src/pg.rs | experimental libpq COPY send-buffer tuning; default 1 MiB |
| `FOD_PG_CONTROL_TRANSACTION_LIMIT` | `[fod] pg_control_transaction_limit` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_PG_DBNAME` | `[database] dbname` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_PG_ENDPOINT_ROUTING_ENABLED` | `[fod] pg_endpoint_routing_enabled` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_PG_HOST` | `[database] host` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/ini_config/pg_endpoints.rs, rust_runtime/src/lib.rs | - |
| `FOD_PG_HOSTS` | `[database] hosts` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_mkfs/src/bin/fod-bootstrap.rs, rust_runtime/src/ini_config/pg_endpoints.rs | - |
| `FOD_PG_OBSERVABILITY_INTERVAL_MS` | `[fod] pg_observability_interval_ms` | intentional-env-only | - | - | rust_fuse/src/pg_lanes.rs | PostgreSQL lane observability sampler; default 5000 ms, valid 100..3600000 ms |
| `FOD_PG_PASSWORD` | `[database] password` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_PG_PAYLOAD_IN_FLIGHT_LIMIT_BYTES` | `[fod] pg_payload_in_flight_limit_bytes` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_PG_POOL_LANES_ENABLED` | `[fod] pg_pool_lanes_enabled` | intentional-env-only | - | - | rust_fuse/src/pg_lanes.rs | temporary opt-in for dedicated PostgreSQL read/write/control/lease pools |
| `FOD_PG_PORT` | `[database] port` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/ini_config/pg_endpoints.rs, rust_runtime/src/lib.rs | - |
| `FOD_PG_PRIMARY_HOSTS` | `[database] primary_hosts` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_mkfs/src/bin/fod-bootstrap.rs, rust_runtime/src/ini_config/pg_endpoints.rs | - |
| `FOD_PG_REPLICA_HOSTS` | `[database] replica_hosts` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_mkfs/src/bin/fod-bootstrap.rs, rust_runtime/src/ini_config/pg_endpoints.rs | - |
| `FOD_PG_REPLICA_READ_ROUTING_ENABLED` | `[fod] pg_replica_read_routing_enabled` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_PG_RUNTIME_FAILOVER_ENABLED` | `[fod] pg_runtime_failover_enabled` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_PG_SSLCERT` | `[database] sslcert` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_PG_SSLKEY` | `[database] sslkey` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_PG_SSLMODE` | `[database] sslmode` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_PG_SSLROOTCERT` | `[database] sslrootcert` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_PG_USER` | `[database] user` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_PG_VISIBLE_PATH` | `[fod] pg_visible_path` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_PG_WRITE_TRANSACTION_LIMIT` | `[fod] pg_write_transaction_limit` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_POOL_MAX_CONNECTIONS` | `[fod] pool_max_connections` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_PROFILE` | `[fod] profile` | intentional-env-only | - | fod_config.ini, fod_config.example.ini | rust_mkfs/src/bin/fod-bootstrap.rs, rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | profile selector; [fod] profile is also supported |
| `FOD_PROFILE_IO` | `[fod] profile_io` | intentional-env-only | - | - | rust_hotpath/src/pg.rs | profiling instrumentation |
| `FOD_PROFILE_IO_VERBOSE` | `[fod] profile_io_verbose` | intentional-env-only | - | - | rust_hotpath/src/pg.rs | verbose per-call I/O profiling; diagnostic only |
| `FOD_READ_AHEAD_BLOCKS` | `[fod] read_ahead_blocks` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_READ_CACHE_BLOCKS` | `[fod] read_cache_blocks` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_READ_CACHE_EVICTION_POLICY` | `[fod] read_cache_eviction_policy` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_ROLE` | `[fod] role` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_fuse/src/pg_lanes.rs, rust_runtime/src/lib.rs | - |
| `FOD_RUST_FUSE_READONLY` | `[fod] force_read_only` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/bin/fod-bootstrap.rs, rust_runtime/src/lib.rs | - |
| `FOD_SELINUX` | `[fod] selinux` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_SELINUX_CONTEXT` | `[fod] selinux_context` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_SELINUX_DEFCONTEXT` | `[fod] selinux_defcontext` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_SELINUX_FSCONTEXT` | `[fod] selinux_fscontext` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_SELINUX_ROOTCONTEXT` | `[fod] selinux_rootcontext` | documented-in-both-ini | - | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_SEQUENTIAL_READ_AHEAD_BLOCKS` | `[fod] sequential_read_ahead_blocks` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS` | `[fod] small_file_read_threshold_blocks` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_STATFS_CACHE_TTL_SECONDS` | `[fod] statfs_cache_ttl_seconds` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_SYNC` | `[fod] sync` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_SYNCHRONOUS_COMMIT` | `[fod] synchronous_commit` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_TASK_OBSERVABILITY_INTERVAL_MS` | `[fod] task_observability_interval_ms` | intentional-env-only | - | - | rust_monitor/src/lib.rs | diagnostic sampling cadence |
| `FOD_TASK_READ_ACTIVE_LIMIT` | `[fod] task_read_active_limit` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_TASK_WRITE_ACTIVE_LIMIT` | `[fod] task_write_active_limit` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |
| `FOD_USE_FUSE_CONTEXT` | `[fod] use_fuse_context` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_USE_RUST_FUSE` | `[fod] use_rust_fuse` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_VERSION_LABEL` | `[fod] version_label` | intentional-env-only | - | - | rust_runtime/src/lib.rs | build-time version label; not runtime INI |
| `FOD_WORKERS_READ` | `[fod] workers_read` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_WORKERS_READ_MIN_BLOCKS` | `[fod] workers_read_min_blocks` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_WORKERS_WRITE` | `[fod] workers_write` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_WORKERS_WRITE_MIN_BLOCKS` | `[fod] workers_write_min_blocks` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_runtime/src/lib.rs | - |
| `FOD_WRITE_FLUSH_THRESHOLD_BYTES` | `[fod] write_flush_threshold_bytes` | active-in-both-ini | fod_config.ini, fod_config.example.ini | fod_config.ini, fod_config.example.ini | rust_mkfs/src/config.rs, rust_runtime/src/lib.rs | - |

## Classification

- `active-in-both-ini`: active assignment exists in both base INI files.
- `documented-in-both-ini`: represented in both files but may stay commented to preserve defaults.
- `intentional-env-only`: secret, selector, internal handoff, or diagnostic/test control.
- `env-only-or-incomplete-ini`: needs an explicit design decision.
