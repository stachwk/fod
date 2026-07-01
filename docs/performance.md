# FOD Performance Profiling

## Purpose

This workflow measures runtime performance before changing hot paths.

Rust debug build reuse and Python venv stamping are already in place, so new profiling work should focus on runtime behavior, PostgreSQL, and source-specific workloads rather than Makefile harness overhead.

## Workload Classes

- FUSE I/O workloads.
- `fod-indexer` scan, hash, plan, materialize, and cleanup workloads.
- PostgreSQL WAL and checkpoint workloads.
- Local PostgreSQL versus QNAP or other remote PostgreSQL runs.

## Environment Fingerprint

Capture host, toolchain, PostgreSQL client, CPU, memory, and filesystem context:

```bash
make profile-env
```

The output is written under `artifacts/perf/<commit>/<host>-<run-id>/env.txt` by default. Override `ARTIFACTS_DIR`, `PROFILE_HOST`, or `PROFILE_RUN_ID` when comparing controlled runs.

## PostgreSQL Capture

Reset statement statistics, run a workload, then capture SQL and WAL/checkpointer state:

```bash
make profile-pg-reset
make test-fod-indexer-materialize-rollback
make profile-pg-top
make profile-pg-wal
make profile-pg-activity
```

When one profiling run captures several workloads into the same `PROFILE_RUN_ID`, set `PROFILE_CAPTURE_LABEL` to avoid overwriting PostgreSQL snapshots:

```bash
make profile-pg-top PROFILE_CAPTURE_LABEL=rollback
make profile-pg-wal PROFILE_CAPTURE_LABEL=rollback
```

`profile-pg-reset` and `profile-pg-top` require `pg_stat_statements`. If PostgreSQL reports that the extension must be loaded through `shared_preload_libraries`, restart the database with the existing project setting instead of ignoring the failure.

`profile-pg-wal` records `pg_stat_wal` and then uses `pg_stat_checkpointer` when the PostgreSQL version exposes it. Older versions fall back to `pg_stat_bgwriter` and print that source in the output.

`profile-pg-io` uses `pg_stat_io` and is optional because it is PostgreSQL-version dependent:

```bash
make profile-pg-io
```

## Local Baseline

Run a repeatable workload with environment and PostgreSQL statement/WAL capture:

```bash
make profile-local-baseline PROFILE_WORKLOAD=test-fod-indexer-materialize-rollback
```

The baseline target does not run `perf record` automatically. CPU profiling can need elevated permissions and can create large local artifacts, so it stays explicit.

## perf CPU Profiling

Use `perf stat` for a low-friction repeated counter snapshot:

```bash
make profile-perf-stat PROFILE_WORKLOAD=test-large-copy-benchmark
```

Use `perf record` when call stacks are needed:

```bash
make profile-perf-record PROFILE_WORKLOAD=test-large-copy-benchmark
perf report -i artifacts/perf/$(git rev-parse --short HEAD)/perf-test-large-copy-benchmark.data
```

## Attach Profiling

For a running FUSE daemon:

```bash
make mount
pgrep -af fod-rust-fuse
make profile-fuse-attach PROFILE_PID=<pid> PROFILE_SECONDS=60
```

For a running indexer process:

```bash
pgrep -af fod-indexer
make profile-indexer-attach PROFILE_PID=<pid> PROFILE_SECONDS=60
```

The attach targets require an explicit `PROFILE_PID`. Do not auto-guess the process when comparing runs.

## bpftrace Syscall Tracing

These helpers are generic and do not hardcode FOD process names:

```bash
make profile-bpftrace-syscalls PROFILE_SECONDS=30
make profile-bpftrace-read-hist PROFILE_SECONDS=30
make profile-bpftrace-write-hist PROFILE_SECONDS=30
```

They are host-dependent and require `bpftrace` plus sufficient privileges.

## QNAP And Remote Rules

Always record:

- Host name.
- Storage type.
- Local or remote PostgreSQL.
- Network path and transport.
- PostgreSQL version.
- Median and p95 across repeated runs.
- Commit and FOD version.

Use the existing QNAP profile where appropriate:

```bash
QNAP=1 make profile-env
QNAP=1 make profile-pg-reset
QNAP=1 make test-postgresql-wal-pressure
QNAP=1 make profile-pg-top
QNAP=1 make profile-pg-wal
```

## Interpreting Results

- High SQL total time: inspect query shape, indexes, prepared statement use, and connection reuse.
- High WAL or checkpoint pressure: inspect write pattern, batch size, checkpoint behavior, and durability settings.
- High context switches: inspect FUSE request flow, blocking points, and backpressure.
- High read/write syscall count: inspect buffer sizing, batching, and short I/O.
- High allocation volume: inspect `rust_indexer` and `rust_fuse` buffer reuse.

## Rule For Optimization Commits

Do not merge a performance optimization without before/after numbers from the relevant workload. The profiling baseline decides whether the next target is SQL shape, FUSE behavior, PostgreSQL WAL/checkpoint pressure, or allocation/buffer reuse.
