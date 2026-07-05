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
make profile-pg-top-io-wal
make profile-pg-metadata-top
make profile-pg-wal
make profile-pg-activity
```

When one profiling run captures several workloads into the same `PROFILE_RUN_ID`, set `PROFILE_CAPTURE_LABEL` to avoid overwriting PostgreSQL snapshots:

```bash
make profile-pg-top PROFILE_CAPTURE_LABEL=rollback
make profile-pg-wal PROFILE_CAPTURE_LABEL=rollback
```

`profile-pg-reset` and `profile-pg-top` require `pg_stat_statements`. If PostgreSQL reports that the extension must be loaded through `shared_preload_libraries`, restart the database with the existing project setting instead of ignoring the failure.

`profile-pg-top-io-wal` uses the same extension but includes local buffer counters and per-statement WAL counters. Use it when separating server-side `COPY` into a temporary staging table from the target-table merge cost. PostgreSQL exposes `wal_records`, `wal_fpi`, and `wal_bytes` per statement there; `wal_buffers_full` remains a cluster-level counter from `pg_stat_wal`. This capture expects a PostgreSQL/`pg_stat_statements` version that exposes the local-buffer and WAL columns.

`profile-pg-metadata-top` filters `pg_stat_statements` to high-call metadata and lookup paths, including path walking, child lookup, attr fetch, xattr, and block/extent lookups. Use it after a representative workload before changing prepared statement coverage or metadata caching.

`profile-pg-wal` records `pg_stat_wal` and then uses `pg_stat_checkpointer` when the PostgreSQL version exposes it. Older versions fall back to `pg_stat_bgwriter` and print that source in the output.

For real `data_blocks` DML behavior, capture a before/after table/index snapshot around the workload:

```bash
make profile-pg-table-dml-snapshot PROFILE_CAPTURE_LABEL=before
make test-large-copy-benchmark
make profile-pg-table-dml-snapshot PROFILE_CAPTURE_LABEL=after
make profile-pg-table-dml-delta
```

The DML delta records `n_tup_ins`, `n_tup_upd`, `n_tup_hot_upd`, `n_tup_del`, `n_dead_tup`, relation-size changes, and index lookup counters for the storage tables involved in the block path: `data_blocks`, `copy_block_crc`, `files`, and `data_objects`. Use it before changing the `data_blocks` conflict merge, because it shows whether the live path is insert-heavy, HOT-update friendly, or doing non-HOT heap rewrites.

To isolate real conflict-update behavior, seed the file first and snapshot only the overwrite phase:

```bash
make profile-data-blocks-conflict-dml \
  PROFILE_RUN_ID=data-blocks-conflict-$(date -u +%Y%m%dT%H%M%SZ) \
  DATA_BLOCKS_CONFLICT_ID=conflict-$(date -u +%Y%m%dT%H%M%SZ)
```

The target runs `test-data-blocks-conflict-seed`, captures DML/WAL snapshots, then runs `test-data-blocks-conflict-overwrite-benchmark` against the same logical file. The resulting table DML delta should show the update-only phase, including `n_tup_upd`, `n_tup_hot_upd`, non-HOT updates, dead-tuple growth, and `idx_data_blocks_object_order` activity.

To verify that unchanged-block filtering avoids needless rewrites, use the same flow with a same-payload overwrite:

```bash
make profile-data-blocks-conflict-noop-dml \
  PROFILE_RUN_ID=data-blocks-conflict-noop-$(date -u +%Y%m%dT%H%M%SZ) \
  DATA_BLOCKS_CONFLICT_ID=conflict-noop-$(date -u +%Y%m%dT%H%M%SZ)
```

This target should keep `data_blocks_n_tup_upd_delta` and `data_blocks_n_dead_tup_delta` at zero when the staged block data is identical to the existing block rows.

To measure repeated changed-payload full overwrites after the data-object swap optimization, run:

```bash
make profile-data-blocks-swap-repeat-dml \
  PROFILE_RUN_ID=data-blocks-swap-repeat-$(date -u +%Y%m%dT%H%M%SZ) \
  DATA_BLOCKS_CONFLICT_ID=swap-repeat-$(date -u +%Y%m%dT%H%M%SZ) \
  PROFILE_DATA_BLOCKS_SWAP_REPEAT=5
```

The target seeds one logical file, then overwrites it several times with a different payload marker on each pass. It captures before/after table DML, WAL, top SQL, and `data_blocks` bloat signals. Use this target for WAL, relation growth, dead tuple, and autovacuum behavior before changing the full-overwrite strategy again.

The default full-overwrite swap cleanup policy is immediate cleanup inside the write transaction. For an opt-in delayed-cleanup experiment, run the same profile with:

```bash
FOD_DATA_OBJECT_SWAP_CLEANUP=deferred \
make profile-data-blocks-swap-repeat-dml \
  PROFILE_RUN_ID=data-blocks-swap-repeat-deferred-$(date -u +%Y%m%dT%H%M%SZ) \
  DATA_BLOCKS_CONFLICT_ID=swap-repeat-deferred-$(date -u +%Y%m%dT%H%M%SZ) \
  PROFILE_DATA_BLOCKS_SWAP_REPEAT=5
```

Deferred cleanup leaves old unreferenced data objects in place so the hot overwrite transaction avoids delete churn. Purge those candidates explicitly when measuring the maintenance phase:

```bash
make profile-pg-data-object-gc DATA_OBJECT_GC_LIMIT=1000000
```

Do not treat deferred cleanup as a default until repeated local and remote runs show that moving delete work into object GC is better than immediate cleanup for the target workload.

To collect the real large-copy baseline plus COPY send-buffer matrix with storage DML, WAL, top statement IO/WAL, and bloat captures, run:

```bash
make profile-data-blocks-copy-buffer-matrix
```

The default matrix covers the baseline `default` buffer plus `262144`, `1048576`, and `4194304` byte explicit send buffers. Use the same target against QNAP or another remote PostgreSQL profile with:

```bash
QNAP=1 make profile-data-blocks-copy-buffer-matrix
```

This target runs the real `test-large-copy-benchmark` path and writes one artifact directory per buffer under `artifacts/perf/<commit>/<host>-<run-id>-<mode>-buffer-<buffer>/`.

To compare the current `data_blocks` merge shape across heap fillfactor variants without modifying real FOD data, run:

```bash
make profile-pg-data-blocks-merge-fillfactor-explain
```

The target uses temporary clone tables only. It checks the real `fod.data_blocks` row count before and after the reproducer, then runs EXPLAIN for fresh insert, identical-payload conflict, and changed-payload conflict. Override `DATA_BLOCKS_EXPLAIN_FILLFACTORS`, `DATA_BLOCKS_EXPLAIN_STAGE_ROWS`, or `DATA_BLOCKS_EXPLAIN_PAYLOAD_BYTES` when narrowing a specific hypothesis.

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

## FUSE Sequential I/O Profiling

Use the FUSE profile wrapper when deciding whether to tune FUSE cache, kernel timeouts, request backpressure, or `max_background`:

```bash
make profile-fuse-sequential-io
```

The default workload is `test-fio-sequential-io-strace`. It captures the full workload output, including `FOD_PROFILE_IO` boundary summaries and strace syscall tables, under `artifacts/perf/<commit>/<host>-<run-id>/fuse-test-fio-sequential-io-strace.txt`.

Override the workload only when comparing a specific FUSE path:

```bash
make profile-fuse-sequential-io PROFILE_FUSE_WORKLOAD=test-fio-mixed-io
make profile-fuse-sequential-io PROFILE_FUSE_WORKLOAD=test-fio-random-mixed-io
```

Privileged observers stay opt-in and should not run the FOD workload itself as root:

```bash
make profile-fuse-sudo-perf-stat PROFILE_FUSE_WORKLOAD=test-fio-sequential-io-strace
make profile-fuse-sudo-bpftrace-syscalls PROFILE_FUSE_WORKLOAD=test-fio-sequential-io-strace PROFILE_SECONDS=12
```

Use these captures before changing FUSE cache or timeout knobs. A single throughput number is not enough; compare boundary timings, syscall shape, and system counters together.

## Indexer Allocation Profiling

Use the indexer allocation helper before changing `rust_indexer` buffer reuse or data structures:

```bash
make profile-indexer-alloc PROFILE_INDEXER_ARGS='--help'
make profile-indexer-alloc PROFILE_INDEXER_ARGS='scan --source my_source'
make profile-indexer-alloc PROFILE_INDEXER_ARGS='hash --source my_source --candidates-only'
```

The default `PROFILE_INDEXER_ALLOC_TOOL=auto` chooses `heaptrack` when available, then `valgrind --tool=massif`, and finally `/usr/bin/time -v`. Force a specific tool with:

```bash
make profile-indexer-alloc PROFILE_INDEXER_ALLOC_TOOL=time PROFILE_INDEXER_ARGS='scan --source my_source'
make profile-indexer-alloc PROFILE_INDEXER_ALLOC_TOOL=heaptrack PROFILE_INDEXER_ARGS='scan --source my_source'
make profile-indexer-alloc PROFILE_INDEXER_ALLOC_TOOL=massif PROFILE_INDEXER_ARGS='scan --source my_source'
```

The target writes metadata, stdout, stderr, status, and tool output under `artifacts/perf/<commit>/<host>-<run-id>/`. Treat `--help` only as a smoke check for the profiling harness; allocation conclusions need a representative `scan` or `hash` workload.

## perf CPU Profiling

Use `perf stat` for a low-friction repeated counter snapshot:

```bash
make profile-perf-stat PROFILE_WORKLOAD=test-large-copy-benchmark
```

If the host blocks unprivileged `perf` with `perf_event_paranoid`, do not run the whole workload as root through `sudo perf stat -- make ...` unless that is explicitly what you want to measure. That form executes `make` and the test process as root and can leave root-owned build artifacts under `target/`.

Prefer attach or system-wide capture where only `perf` has elevated privileges and the workload still runs as the normal user. The Makefile helper does that and restores ownership of the root-written perf output file:

```bash
make profile-sudo-perf-stat-system PROFILE_WORKLOAD=test-large-copy-benchmark
```

The equivalent manual shape is:

```bash
mkdir -p artifacts/perf/$(git rev-parse --short HEAD)/$(hostname -s)-manual
sudo -n perf stat -a -d -d -d -o artifacts/perf/$(git rev-parse --short HEAD)/$(hostname -s)-manual/perf-stat-system.txt -- sleep 12 &
make test-large-copy-benchmark
wait
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

To run a short bpftrace syscall sample while the workload still runs as the normal user:

```bash
make profile-sudo-bpftrace-syscalls-workload PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_SECONDS=12
```

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
- High metadata lookup time: run `make profile-pg-metadata-top` and compare `path_walk`, `child_lookup`, and `*_attrs` categories before adding new caching or prepared statements.
- High WAL or checkpoint pressure: inspect write pattern, batch size, checkpoint behavior, and durability settings.
- High context switches: inspect FUSE request flow, blocking points, and backpressure.
- High read/write syscall count: inspect buffer sizing, batching, and short I/O.
- High allocation volume: inspect `rust_indexer` and `rust_fuse` buffer reuse.

## Rule For Optimization Commits

Do not merge a performance optimization without before/after numbers from the relevant workload. The profiling baseline decides whether the next target is SQL shape, FUSE behavior, PostgreSQL WAL/checkpoint pressure, or allocation/buffer reuse.
