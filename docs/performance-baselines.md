# FOD Performance Baselines

## Rules

- Raw profiling artifacts stay under `artifacts/perf/...` and are not committed.
- Each committed baseline summary must include commit, host, workload, local/QNAP mode, PostgreSQL version, and exact commands.
- No performance optimization should be merged without before/after numbers from the relevant workload.

## 2026-07-01 Local Baseline

### Run Metadata

- Commit: `8e8e95f` (`FOD 3.2.1: add performance profiling baseline`)
- FOD version: `3.2.1`
- Host: `lt7300`
- Kernel: `Linux 6.17.0-35-generic`
- CPU: Intel Core i5-8365U, 8 logical CPUs
- Memory: 15 GiB
- Mode: local Docker PostgreSQL, not QNAP
- PostgreSQL server: `PostgreSQL 16.14` (`server_version_num=160014`)
- PostgreSQL client: `psql 17.10`
- Artifact directory: `artifacts/perf/8e8e95f/lt7300-20260701T115956Z`
- Raw artifacts: intentionally untracked and ignored by `.gitignore`

### Commands

```bash
make build-debug
make venv
make init

make profile-env PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300

make profile-pg-reset PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300
make test-fod-indexer-materialize-rollback
make profile-pg-top PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=rollback
make profile-pg-wal PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=rollback
make profile-pg-activity PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=rollback
make profile-pg-io PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=rollback

make profile-pg-reset PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300
make test-fod-indexer-plan-import-scope
make profile-pg-top PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=plan-import-scope
make profile-pg-wal PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=plan-import-scope

make profile-pg-reset PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300
make test-large-copy-benchmark
make profile-pg-top PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=large-copy
make profile-pg-wal PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=large-copy

make profile-perf-stat PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_RUN_ID=20260701T115956Z PROFILE_HOST=lt7300
```

### Workload: `test-fod-indexer-materialize-rollback`

Top SQL by total execution time:

| Rank | Calls | Total ms | Mean ms | Query family |
| --- | ---: | ---: | ---: | --- |
| 1 | 4 | 11.759 | 2.940 | `DELETE FROM files WHERE id_file = $1` |
| 2 | 2 | 3.866 | 1.933 | `DELETE FROM index_sources WHERE name = ANY(...)` |
| 3 | 2 | 3.675 | 1.838 | create `index_sources_stage` temp table |
| 4 | 3 | 3.155 | 1.052 | create `fod_persist_block_stage` temp table |
| 5 | 2 | 2.954 | 1.477 | create `index_scan_runs_stage` temp table |

WAL/checkpointer snapshot at capture time:

- `wal_records=292154`
- `wal_bytes=38464509`
- `wal_write=10108`
- `wal_sync=1805`
- Checkpoint source: `pg_stat_bgwriter` fallback because PostgreSQL 16 does not expose `pg_stat_checkpointer`
- `checkpoints_timed=34`
- `checkpoints_req=4`
- `pg_stat_io`: available on this server

### Workload: `test-fod-indexer-plan-import-scope`

Top SQL by total execution time:

| Rank | Calls | Total ms | Mean ms | Query family |
| --- | ---: | ---: | ---: | --- |
| 1 | 4 | 6.224 | 1.556 | create `index_duplicate_sets_stage` temp table |
| 2 | 2 | 4.142 | 2.071 | create `index_sources_stage` temp table |
| 3 | 2 | 3.110 | 1.555 | create `index_scan_runs_stage` temp table |
| 4 | 8 | 2.849 | 0.356 | `DELETE FROM index_duplicate_sets` |
| 5 | 3 | 2.403 | 0.801 | `DELETE FROM index_sources WHERE name = ANY(...)` |

WAL/checkpointer snapshot at capture time:

- `wal_records=294400`
- `wal_bytes=38887734`
- `wal_write=10142`
- `wal_sync=1838`
- Checkpoint source: `pg_stat_bgwriter`
- `checkpoints_timed=35`
- `checkpoints_req=4`

### Workload: `test-large-copy-benchmark`

Runtime output:

- Payload size: 64 MiB
- Measured workload time: `elapsed_s=3.892044`
- Throughput: `16.44 MiB/s`
- Full Rust test wall time: `7.87s`

Top SQL by total execution time:

| Rank | Calls | Total ms | Mean ms | Rows | Query family |
| --- | ---: | ---: | ---: | ---: | --- |
| 1 | 2 | 1463.365 | 731.682 | 32768 | `COPY fod_persist_block_stage (...) FROM STDIN BINARY` |
| 2 | 1 | 408.712 | 408.712 | 16384 | `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` |
| 3 | 1 | 386.994 | 386.994 | 16384 | `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` |
| 4 | 2076 | 143.278 | 0.069 | 2067 | recursive path walk over `directories` |
| 5 | 2067 | 78.829 | 0.038 | 2062 | child entry lookup over hardlinks/symlinks/directories/files |

WAL/checkpointer snapshot at capture time:

- `wal_records=460544`
- `wal_bytes=52113771`
- `wal_write=10371`
- `wal_sync=1869`
- Checkpoint source: `pg_stat_bgwriter`
- `checkpoints_timed=35`
- `checkpoints_req=4`

The WAL counters are cumulative from PostgreSQL `stats_reset`, not per-workload deltas. In this short local run, checkpoint counts did not increase between the `plan-import-scope` and `large-copy` captures, so WAL/checkpoint behavior is not the first local limiter shown by this baseline.

### `perf stat`

`perf` was installed at `/usr/bin/perf`, but the run failed before collecting counters:

```text
perf_event_paranoid setting is 4
Access to performance monitoring and observability operations is limited.
```

No `task-clock`, `cycles`, `instructions`, IPC, context-switch, or page-fault counters were available from this run.

### Interpretation

The strongest measured signal is SQL payload persistence in the large-copy path:

- `COPY fod_persist_block_stage` plus two `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` statements account for about `2259 ms` of PostgreSQL execution time.
- That is about 58% of the `3.892 s` measured 64 MiB write workload time.
- The repeated path/entry metadata lookups are visible (`2076` and `2067` calls), but their combined total is about `222 ms`, so they are secondary on this baseline.
- The indexer smokes show small absolute SQL times dominated by temp staging table setup and cleanup, not by large repeated query families.
- `perf stat` could not confirm FUSE CPU/context-switch behavior on this host.
- WAL/checkpoint counters did not show a new checkpoint during the large-copy capture.

First optimization target from this baseline:

```text
SQL payload persistence path: benchmark and optimize the `fod_persist_block_stage` COPY plus `data_blocks` merge shape before tuning FUSE cache/backpressure or PostgreSQL durability settings.
```

Secondary follow-up:

```text
Review repeated metadata lookup/prepared statement reuse after the payload persistence path has a before/after measurement, because high-call path lookups are visible but not dominant in this run.
```
