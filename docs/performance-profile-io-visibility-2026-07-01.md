# FOD 3.2.1 — FUSE `FOD_PROFILE_IO` Visibility Confirmation

Date: `2026-07-01`

## Summary

The `FOD_PROFILE_IO` visibility fix works: successful Rust FUSE benchmark runs now expose the useful profile summary lines before the temporary mount workspace and `mount.log` are removed.

This confirms that client-side `PQputCopyData` send overhead is not the main limiter in the 64 MiB large-copy workload. The next performance target should remain server-side SQL: `COPY fod_persist_block_stage` plus `INSERT INTO data_blocks ... ON CONFLICT`.

## Run Metadata

- Commit at validation time: `4bac9cf` (`FOD 3.2.1: preserve FUSE profile IO summaries`)
- FOD version: `3.2.1`
- Host: `lt7300`
- Profile run id: `profile-io-visible-20260701T131824Z`
- Artifact directory: `artifacts/perf/4bac9cf/lt7300-profile-io-visible-20260701T131824Z`
- Workload: `FOD_PROFILE_IO=1 make test-large-copy-benchmark`
- Payload size: 64 MiB

## Commands

```bash
echo "PROFILE_RUN_ID=$PROFILE_RUN_ID"
echo "PROFILE_HOST=$PROFILE_HOST"

find "artifacts/perf" -maxdepth 4 -type f -path "*$PROFILE_RUN_ID*" -printf '%p %s\n' | sort

rg "OK large-copy-benchmark|pg.copy_put_data.aggregate|pg.copy_put_end|pg.copy_get_result" /tmp/fod-profile-io-large-copy-profiled.log

find "artifacts/perf" -maxdepth 4 -type f -path "*$PROFILE_RUN_ID*" -name "pg_top_statements*.txt" \
  -exec sh -c 'echo "== $1 =="; sed -n "1,40p" "$1"' sh {} \;

find "artifacts/perf" -maxdepth 4 -type f -path "*$PROFILE_RUN_ID*" -name "pg_wal_checkpointer*.txt" \
  -exec sh -c 'echo "== $1 =="; sed -n "1,80p" "$1"' sh {} \;
```

## Runtime Result

```text
OK large-copy-benchmark bytes=67108864 elapsed_s=3.717123 throughput_mib_s=17.22
```

## `FOD_PROFILE_IO` Output

Successful benchmark output now includes the useful profile summaries:

```text
FOD I/O profile: op=pg.copy_put_data.aggregate seconds=0.021255 blocks=0 bytes=67993619 count=65 max=0.000566 avg=0.000327
FOD I/O profile: op=pg.copy_put_end seconds=0.000030 blocks=0 bytes=0 rc=1
FOD I/O profile: op=pg.copy_get_result seconds=0.019042 blocks=0 bytes=0 status=1
FOD I/O profile: op=pg.copy_get_result seconds=0.000001 blocks=0 bytes=0 status=-1
FOD I/O profile: op=pg.copy_put_data.aggregate seconds=0.018728 blocks=0 bytes=67993619 count=65 max=0.000420 avg=0.000288
FOD I/O profile: op=pg.copy_put_end seconds=0.000044 blocks=0 bytes=0 rc=1
FOD I/O profile: op=pg.copy_get_result seconds=0.018299 blocks=0 bytes=0 status=1
FOD I/O profile: op=pg.copy_get_result seconds=0.000001 blocks=0 bytes=0 status=-1
```

Condensed aggregate:

| Operation | Seconds | Bytes | Count | Max | Avg |
| --- | ---: | ---: | ---: | ---: | ---: |
| `pg.copy_put_data.aggregate` pass 1 | `0.021255` | `67993619` | `65` | `0.000566` | `0.000327` |
| `pg.copy_put_data.aggregate` pass 2 | `0.018728` | `67993619` | `65` | `0.000420` | `0.000288` |

The two visible client-side COPY-send aggregate passes total about `0.040 s`. That is small relative to the measured `3.717123 s` workload time.

## PostgreSQL Top Statements

| Query family | Calls | Total ms | Rows | Notes |
| --- | ---: | ---: | ---: | --- |
| `COPY fod_persist_block_stage (...) FROM STDIN BINARY` | `2` | `1049.863` | `32768` | server-side COPY into temp staging table |
| `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` | `1` | `398.716` | `16384` | first merge pass |
| `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` | `1` | `395.162` | `16384` | second merge pass |
| recursive path walk over `directories` | `2075` | `139.197` | `2066` | visible but secondary |
| child entry lookup | `2066` | `94.959` | `2061` | visible but secondary |

Payload SQL time:

```text
1049.863 + 398.716 + 395.162 = 1843.741 ms
1843.741 ms / 3717.123 ms ~= 49.6% of measured workload time
```

## WAL / Checkpointer Snapshot

```text
wal_records=5998434
wal_fpi=4986
wal_bytes=525127612
wal_buffers_full=12089
wal_write=23714
wal_sync=10819
checkpointer_source=pg_stat_bgwriter fallback
checkpoints_timed=50
checkpoints_req=4
buffers_checkpoint=18786
buffers_clean=4535
buffers_backend=35961
buffers_backend_fsync=0
```

The WAL/checkpointer counters are cumulative from PostgreSQL `stats_reset` and should not be interpreted as exact per-workload deltas without a before/after WAL capture pair.

## Interpretation

- The `FOD_PROFILE_IO` visibility fix worked: successful Rust FUSE benchmark output now exposes aggregate profile lines that were previously lost with the temporary mount log.
- Client-side `PQputCopyData` send time is not the main limiter for this workload: the two visible aggregate passes total about `0.040 s` for about 68 MB each.
- The dominant measured area remains server-side payload SQL: `COPY fod_persist_block_stage` plus `INSERT INTO data_blocks ... ON CONFLICT`, about `1843.741 ms` or roughly `49.6%` of measured workload time.
- Repeated metadata lookups are visible, but still secondary compared with payload persistence SQL.

## Next Recommended Target

The next Codex task should investigate and benchmark the server-side `data_blocks` merge path, especially the `ON CONFLICT DO UPDATE` behavior, using `EXPLAIN (ANALYZE, BUFFERS, WAL)` where possible and before/after measurements on `test-large-copy-benchmark`.
