# FOD 3.4.1 primary-write PostgreSQL profile

Status: diagnostic result recorded; runtime behavior unchanged.

## 4 GiB local diagnostic run

- Runtime: FOD `3.4.1`.
- Commit under test: `5f645c0711cf88f18c5adbbddd3366ed1a93332d`.
- Client host: `lt7300`.
- Backend: local Docker primary/replica benchmark.
- File size: `4G`.
- Fio block size: `512k`.
- FOD read cache/read-ahead/prefetch disabled by the benchmark.
- FUSE direct I/O enabled.
- Host kernel page cache not dropped.

Observed throughput for the PostgreSQL-wait diagnostic run:

| phase | MiB/s | IOPS |
| --- | ---: | ---: |
| primary write | 60.119 | 120.237 |
| primary read | 294.888 | 589.777 |
| replica read | 276.439 | 552.878 |

This single run is not promoted to a throughput baseline because another 4 GiB diagnostic run on the same commit produced materially different throughput (`49.639 MiB/s` primary write). With host page cache uncontrolled, these long runs are used for profiling phase duration rather than for regression thresholds.

## FOD CPU result

A previous `perf stat` capture covered about 78.08 seconds of the primary-write phase:

```text
task-clock       14,236,142,833 ns
CPUs utilized    0.182
cycles            44,830,537,378
instructions      35,094,559,265
IPC               0.78
context-switches  10,932
cpu-migrations    750
page-faults       39,993
wall time          78.079722420 s
```

The process used only about 18.2% of one CPU on average. Primary write is therefore not limited by CPU execution in `fod-rust-fuse`; most wall time is spent waiting for PostgreSQL-facing work to complete.

## PostgreSQL wait sampling

During the primary-write phase the sampled client-backend states were:

```text
652 idle                 Client ClientRead
225 active               -      -
1   idle in transaction  Client ClientRead
1   idle in transaction  -      -
```

There were no material `IO`, `WAL`, or lock wait events in the captured client backends.

The ratio is close to one active backend for three idle `ClientRead` backends. Combined with the configured four write workers, this is consistent with a pool in which one PostgreSQL backend executes the current persist SQL while the remaining connections are idle. This is a diagnostic hypothesis, not yet proof of transaction-level serialization; the active SQL shape must be inspected next.

The `active / - / -` samples mean PostgreSQL was actively executing work rather than sleeping on a PostgreSQL wait event. Together with low FOD CPU utilization, this moves the next investigation toward the SQL executor path, especially the measured `data_blocks_merge` stage.

## WAL snapshot caveat

The first manual sampler attempted to capture `pg_stat_io` and `pg_stat_wal` after the primary-write FOD process exited. The benchmark intentionally stops/restarts primary between phases, so the post-write queries raced with shutdown and failed with:

```text
FATAL: the database system is shutting down
```

Consequently the captured WAL and I/O output is only a point-in-time snapshot and must not be treated as a before/after delta.

The point-in-time WAL snapshot showed non-zero `wal_buffers_full`, writes and syncs, but without a valid end snapshot those counters are not sufficient to attribute the write bottleneck to WAL.

## Follow-up sampler

Use:

```bash
scripts/perf/profile_primary_write_postgres.sh
```

in a second terminal while the existing primary/replica matrix runs. The helper continuously records:

- client-backend state, wait event and query text;
- `pg_stat_wal` counters;
- aggregated `pg_stat_io` counters for client backends, checkpointer and background writer;
- `track_io_timing`, `track_wal_io_timing` and `synchronous_commit` settings.

It intentionally does not run `pg_stat_statements_reset()` and does not require `pg_stat_statements` to be present in `shared_preload_libraries`.

It also intentionally does not query PostgreSQL after the primary-write process exits. The last successful continuous samples are used as the end-of-phase observations, avoiding the primary shutdown race.

The sampler waits for the `fod-rust-fuse` primary-write process. Absence of that process is a normal waiting state and must not terminate the helper under `set -euo pipefail`. Process matching is a literal substring match, not an awk regular expression. The preferred override is:

```bash
FOD_PG_WRITE_PROFILE_PROCESS_MATCH='/tmp/fod-primary-write.' \
    scripts/perf/profile_primary_write_postgres.sh
```

The historical `FOD_PG_WRITE_PROFILE_PROCESS_PATTERN` variable remains accepted for compatibility, but its value is also treated literally.

Recommended workload in terminal 1:

```bash
FOD_CARGO_PROFILE=profiling \
FOD_RUNTIME_PROFILE=profiling \
REPLICA_READ_FIO_FILE_SIZE=4G \
REPLICA_READ_FIO_BLOCK_SIZES=512k \
FOD_REQUIRE_AC_POWER=1 \
make test-fio-primary-write-replica-read-matrix
```

Sampler in terminal 2:

```bash
scripts/perf/profile_primary_write_postgres.sh
```

The summary reports wait-state counts, active-query sample counts, settings, and first/last WAL and I/O samples. The next decision is:

1. if active samples are dominated by the `data_blocks` merge statement with no wait event, run the existing `scripts/perf/pg/explain_data_blocks_merge.sql` reproducer and optimize the SQL/index/executor shape;
2. if continuous WAL/I/O samples show pressure or explicit waits, investigate WAL/checkpoint/storage behavior instead;
3. do not optimize FUSE request size for write based on these results, because earlier internal profiling already showed that `64k` and `512k` requests converge to the same sixteen 64 MiB persists for a 1 GiB file.
