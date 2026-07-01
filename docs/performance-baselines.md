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

Unprivileged `perf` was installed at `/usr/bin/perf`, but the first run failed before collecting counters:

```text
perf_event_paranoid setting is 4
Access to performance monitoring and observability operations is limited.
```

Follow-up sudo observability run on the same host:

- Commit at run time: `918f8b1`
- Artifact directory: `artifacts/perf/918f8b1/lt7300-20260701T120934Z`
- `sudo -n` was available.
- Direct `sudo perf stat -- make ...` worked, but it ran the workload as root and temporarily left root-owned files under `target/`; ownership was restored after the run. Treat direct sudo-wrapped workload counters as diagnostic only.
- A cleaner system-wide `sudo perf stat -a ... sleep 12` run kept the workload as the normal user.

Warm direct sudo `perf stat` around `test-large-copy-benchmark`, after compilation:

| Metric | Value |
| --- | ---: |
| Runs | 3 |
| task-clock | `4.486 s` |
| CPUs utilized | `0.556` |
| elapsed time | `8.069 +/- 0.110 s` |
| context switches | `15661` |
| page faults | `97091` |
| instructions | `24.405 B` |
| cycles | `15.288 B` |
| IPC | `1.60` |

System-wide sudo `perf stat -a` while the workload ran as the normal user:

| Metric | Value |
| --- | ---: |
| elapsed time | `12.007 s` |
| cpu-clock | `96.045 s` |
| CPUs utilized | `7.999` |
| context switches | `313891` |
| page faults | `381500` |
| instructions | `62.821 B` |
| cycles | `61.779 B` |
| IPC | `1.02` |

The system-wide counters include host noise, but they confirm the host can collect privileged performance counters without running FOD as root.

### `bpftrace`

`sudo -n bpftrace` worked. A generic `syscalls_by_comm` capture during a warm `test-large-copy-benchmark` made FOD and PostgreSQL visible:

- `fod-rust-fuse` appeared in repeated one-second samples, including intervals around `6818`, `5101`, `16729`, `12351`, `13627`, `7702`, and `9508` syscalls.
- `postgres` appeared in repeated one-second samples, including intervals around `10170`, `64912`, `29529`, `21662`, `28689`, `66085`, and `17142` syscalls.
- `docker-proxy` was also high in some intervals, so this generic comm-level trace is useful for visibility but too noisy for final bottleneck attribution.

Future syscall tracing should use a filtered script or attach to specific PIDs once the exact process boundary is selected.

### Interpretation

The strongest measured signal is SQL payload persistence in the large-copy path:

- `COPY fod_persist_block_stage` plus two `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` statements account for about `2259 ms` of PostgreSQL execution time.
- That is about 58% of the `3.892 s` measured 64 MiB write workload time.
- The repeated path/entry metadata lookups are visible (`2076` and `2067` calls), but their combined total is about `222 ms`, so they are secondary on this baseline.
- The indexer smokes show small absolute SQL times dominated by temp staging table setup and cleanup, not by large repeated query families.
- Unprivileged `perf stat` was blocked, but sudo `perf` and sudo `bpftrace` confirmed that privileged host observability works. The safest mode is attach or system-wide capture while the workload still runs as the normal user.
- WAL/checkpoint counters did not show a new checkpoint during the large-copy capture.

First optimization target from this baseline:

```text
SQL payload persistence path: benchmark and optimize the `fod_persist_block_stage` COPY plus `data_blocks` merge shape before tuning FUSE cache/backpressure or PostgreSQL durability settings.
```

Secondary follow-up:

```text
Review repeated metadata lookup/prepared statement reuse after the payload persistence path has a before/after measurement, because high-call path lookups are visible but not dominant in this run.
```

## 2026-07-01 SQL Payload Persistence Batching Check

### Run Metadata

- Base commit before implementation: `024547a` (`FOD 3.2.1: add safe sudo profiling helpers`)
- FOD version: `3.2.1`
- Host: local workstation, local Docker PostgreSQL
- Workload: `make test-large-copy-benchmark`
- Payload size: 64 MiB
- Implementation under test: batch binary `COPY` payload sends into 1 MiB client buffers for `fod_persist_block_stage`; staging table, transaction scope, and `data_blocks` merge SQL stay unchanged.
- Before artifact directory: `artifacts/perf/024547a/local-sql-persist-before-20260701-143036`
- After artifact directory: `artifacts/perf/024547a/local-sql-persist-after-20260701-143317`
- After results were collected from a working tree based on `024547a` with the batching patch applied; the final commit contains the same code.

### Commands

```bash
make build-debug
make profile-env PROFILE_RUN_ID=sql-persist-before-20260701-143036 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=before
make profile-pg-reset PROFILE_RUN_ID=sql-persist-before-20260701-143036 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=before
make test-large-copy-benchmark
make profile-pg-top PROFILE_RUN_ID=sql-persist-before-20260701-143036 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=before
make profile-pg-wal PROFILE_RUN_ID=sql-persist-before-20260701-143036 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=before
make profile-sudo-perf-stat-system PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_RUN_ID=sql-persist-before-20260701-143036 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=before PROFILE_SUDO='sudo -n'

make build-debug
make profile-env PROFILE_RUN_ID=sql-persist-after-20260701-143317 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=after
make profile-pg-reset PROFILE_RUN_ID=sql-persist-after-20260701-143317 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=after
make test-large-copy-benchmark
make profile-pg-top PROFILE_RUN_ID=sql-persist-after-20260701-143317 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=after
make profile-pg-wal PROFILE_RUN_ID=sql-persist-after-20260701-143317 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=after
make profile-sudo-perf-stat-system PROFILE_WORKLOAD=test-large-copy-benchmark PROFILE_RUN_ID=sql-persist-after-20260701-143317 PROFILE_HOST=local PROFILE_CAPTURE_LABEL=after PROFILE_SUDO='sudo -n'
```

### Before

Runtime output:

- `elapsed_s=3.523786`
- `throughput_mib_s=18.16`

Top payload SQL:

| Query family | Calls | Total ms | Rows |
| --- | ---: | ---: | ---: |
| `COPY fod_persist_block_stage (...) FROM STDIN BINARY` | 2 | `1224.010` | 32768 |
| `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` | 1 | `390.044` | 16384 |
| `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` | 1 | `381.377` | 16384 |

System-wide sudo `perf stat` around the same workload:

- workload output: `elapsed_s=3.719708`, `throughput_mib_s=17.21`
- elapsed wall time: `9.126 s`
- context switches: `277466`
- page faults: `372622`
- instructions: `52.254 B`
- cycles: `47.025 B`

### After

First SQL-captured run:

- `elapsed_s=3.766381`
- `throughput_mib_s=16.99`

Top payload SQL:

| Query family | Calls | Total ms | Rows |
| --- | ---: | ---: | ---: |
| `COPY fod_persist_block_stage (...) FROM STDIN BINARY` | 2 | `1284.245` | 32768 |
| `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` | 1 | `463.377` | 16384 |
| `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` | 1 | `449.113` | 16384 |

Follow-up repeated workload timings after the patch:

| Run | elapsed_s | throughput_mib_s |
| --- | ---: | ---: |
| repeat 1 | `3.818472` | `16.76` |
| repeat 2 | `3.557921` | `17.99` |
| repeat 3 | `3.542274` | `18.07` |
| `FOD_PROFILE_IO=1` smoke | `3.504494` | `18.26` |
| sudo `perf stat` workload | `3.494957` | `18.31` |

System-wide sudo `perf stat` after the patch:

- elapsed wall time: `8.716 s`
- context switches: `192068`
- page faults: `372363`
- instructions: `54.085 B`
- cycles: `46.305 B`

### Interpretation

- The batching patch is safe transport hardening: it reduces the number of client-side `PQputCopyData` calls by buffering binary COPY rows before sending, without changing persistence semantics, replay boundaries, PostgreSQL durability, or the merge SQL.
- The local throughput result is mixed. Some after runs were faster than the before perf run, but the SQL-captured after run was slower than the first before run. Treat this as no proven large end-to-end speedup on this host.
- The dominant measured area remains server-side payload persistence: `COPY fod_persist_block_stage` plus `INSERT INTO data_blocks ... ON CONFLICT`.
- Generic `bpftrace` syscall capture worked but was too noisy for attribution because it included host and Docker processes. A filtered PID/comm script is needed before syscall counts can drive the next decision.
- A profiling harness follow-up is needed because `FOD_PROFILE_IO=1` during Rust FUSE cargo tests can write useful `pg.copy_put_data.aggregate` data to the temporary mount log, but the test support removes that log before the aggregate is visible in successful runs.

## 2026-07-01 Current `data_blocks` Merge Diagnostics

### Run Metadata

- Base commit at validation time: `4a66459` (`FOD 3.2.1: record rejected data block DO NOTHING probe`)
- FOD version: `3.2.1`
- Host: `lt7300`
- Mode: local Docker PostgreSQL
- Profile run id: `merge-current-20260701T184307Z`
- Artifact directory: `artifacts/perf/4a66459/lt7300-merge-current-20260701T184307Z`
- Runtime workload: `FOD_PROFILE_IO=1 make test-large-copy-benchmark`
- Diagnostic target: `make profile-pg-data-blocks-merge-explain PROFILE_CAPTURE_LABEL=merge-explain`

### Runtime Result

```text
OK large-copy-benchmark bytes=67108864 elapsed_s=3.849497 throughput_mib_s=16.63
```

Visible client-side COPY send aggregates:

| Pass | Seconds | Bytes | Count | Max | Avg |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | `0.022504` | `67993619` | `65` | `0.000664` | `0.000346` |
| 2 | `0.020275` | `67993619` | `65` | `0.000537` | `0.000312` |

The two `PQputCopyData` aggregate passes total about `0.043 s`, so client send time remains small relative to the `3.849 s` workload.

### PostgreSQL Top Statements

| Query family | Calls | Total ms | Rows | shared_blks_hit | shared_blks_dirtied | shared_blks_written |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `COPY fod_persist_block_stage (...) FROM STDIN BINARY` | 2 | `1218.576` | 32768 | 0 | 0 | 0 |
| `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` | 1 | `499.954` | 16384 | 230970 | 345 | 336 |
| `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` | 1 | `435.851` | 16384 | 214587 | 336 | 336 |
| recursive path walk over `directories` | 2076 | `167.903` | 2067 | 8273 | 0 | 0 |
| child entry lookup | 2067 | `115.453` | 2062 | 14469 | 0 | 0 |

Payload SQL time:

```text
1218.576 + 499.954 + 435.851 = 2154.381 ms
2154.381 ms / 3849.497 ms ~= 56.0% of measured workload time
```

### `data_blocks` Semantics Snapshot

The current local database after the workload reported:

- `data_blocks_rows=1245341`
- `data_blocks_without_matching_file_owner=0`
- `data_objects_with_multiple_files=0`
- `objects_with_multiple_block_id_files=0`

This snapshot does not justify weakening `ON CONFLICT DO UPDATE`. It only confirms that the current database state is internally consistent after the correct merge path.

### Temp-Table `EXPLAIN` Reproducer

The new `profile-pg-data-blocks-merge-explain` target creates a temporary target table with a primary key, an index on `data_object_id`, and the unique `(data_object_id, _order)` arbiter index. It then runs the existing merge shape against 16k staged 4 KiB rows.

This target does not alter real `fod.data_blocks`; the real row count was `1245341` before and after the reproducer.

| Scenario | Tuples inserted | Conflicting tuples | Execution ms | Buffers summary |
| --- | ---: | ---: | ---: | --- |
| fresh insert into empty temp target | 16384 | 0 | `235.611` | `local hit=178435 read=3 dirtied=313 written=312` |
| conflict update with identical payload | 0 | 16384 | `307.665` | `local hit=247218 dirtied=309 written=309` |
| conflict update with changed payload and `id_file` | 0 | 16384 | `359.060` | `local hit=277396 dirtied=225 written=271` |

Limitations:

- Temporary tables do not provide production-representative WAL numbers.
- The temp reproducer is useful for plan shape and relative insert/update cost, not final production timing.
- The live runtime merge is still slower than the temp reproducer, so the next diagnostics should inspect real table/index bloat, WAL/write amplification, and whether batch size changes the real merge cost.

### Real Table/Index Size And Churn Snapshot

Captured with:

```bash
make profile-pg-data-blocks-bloat PROFILE_RUN_ID=merge-current-20260701T185415Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=current
```

Current table statistics after the large-copy workload:

| Relation | n_live_tup | n_dead_tup | n_mod_since_analyze | last_autovacuum | last_autoanalyze |
| --- | ---: | ---: | ---: | --- | --- |
| `data_blocks` | 1278109 | 84 | 32768 | `2026-07-01 18:22:35.905885+00` | `2026-07-01 18:44:36.253954+00` |
| `copy_block_crc` | 46 | 22 | 0 | `2026-07-01 13:40:37.637849+00` | `2026-07-01 18:31:35.673848+00` |
| `data_objects` | 121 | 11 | 20 | `2026-07-01 18:30:35.664995+00` | `2026-07-01 18:30:35.67172+00` |
| `files` | 121 | 8 | 24 | `2026-07-01 18:29:35.671599+00` | `2026-07-01 18:30:35.677719+00` |

Real relation sizes:

| Relation | relation_size | total_size |
| --- | ---: | ---: |
| `data_blocks` | `143 MB` | `206 MB` |
| `idx_data_blocks_object_order` | `27 MB` | `27 MB` |
| `idx_data_blocks_data_object_id` | `8176 kB` | `8176 kB` |
| `copy_block_crc` | `8192 bytes` | `72 kB` |

Index usage snapshot:

- `idx_data_blocks_data_object_id`: `idx_scan=69`, `idx_tup_read=28773806`, `idx_tup_fetch=11809106`
- `idx_data_blocks_object_order`: `idx_scan=1291182`, `idx_tup_read=229`, `idx_tup_fetch=225`

Interpretation:

- The current local database does not show a large `n_dead_tup` accumulation in `data_blocks` after the run.
- The `idx_data_blocks_data_object_id` read/fetch counters are high relative to its scan count, so future investigation should check which cleanup/read paths rely on that index before removing or changing it.
- This snapshot is a size/churn signal, not a mathematical bloat estimate.

### Real WAL Delta Snapshot

Captured with:

```bash
make profile-pg-wal-snapshot PROFILE_RUN_ID=wal-current-20260701T185934Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=before
FOD_PROFILE_IO=1 make test-large-copy-benchmark
make profile-pg-wal-snapshot PROFILE_RUN_ID=wal-current-20260701T185934Z PROFILE_HOST=lt7300 PROFILE_CAPTURE_LABEL=after
make profile-pg-wal-delta PROFILE_RUN_ID=wal-current-20260701T185934Z PROFILE_HOST=lt7300
```

Runtime output:

```text
OK large-copy-benchmark bytes=67108864 elapsed_s=3.725767 throughput_mib_s=17.18
```

Visible client-side COPY send aggregates:

| Pass | Seconds | Bytes | Count | Max | Avg |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | `0.023847` | `67993619` | `65` | `0.000641` | `0.000367` |
| 2 | `0.022095` | `67993619` | `65` | `0.000511` | `0.000340` |

WAL/checkpointer delta for this single workload:

| Metric | Before | After | Delta |
| --- | ---: | ---: | ---: |
| `buffers_backend` | 44910 | 44910 | 0 |
| `buffers_backend_fsync` | 0 | 0 | 0 |
| `buffers_checkpoint` | 30188 | 30188 | 0 |
| `wal_buffers_full` | 13176 | 13355 | 179 |
| `wal_bytes` | 751608509 | 764465924 | 12857415 |
| `wal_fpi` | 5616 | 5622 | 6 |
| `wal_records` | 8533154 | 8698779 | 165625 |
| `wal_sync` | 16766 | 16786 | 20 |
| `wal_write` | 30777 | 30976 | 199 |

Interpretation:

- This is the first real production-path WAL delta for the current large-copy workload, not a cumulative-only snapshot.
- The run produced about `12.86 MB` of WAL for the 64 MiB copy workload on this local setup.
- There was no checkpoint/backend-buffer delta in this short run, so the immediate signal is WAL generation/write/sync activity rather than checkpoint pressure.

### Interpretation

- The rejected `DO NOTHING` probe confirms that correctness currently requires the conflict update path.
- The correct live path is still dominated by server-side `COPY` plus two `data_blocks` merges.
- The temp-table reproducer gives a safe way to compare candidate plans before touching production SQL.
- Do not change runtime merge semantics until a candidate improves the real path and passes `test-copy-block-crc-table`, remount durability, chunking, and unlink-after-write.
