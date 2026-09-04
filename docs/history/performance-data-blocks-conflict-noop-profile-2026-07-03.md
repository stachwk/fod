# FOD Data Blocks Profile - 2026-07-03

## Run Metadata

- Run ID: `data-blocks-conflict-noop-20260703T140759Z`
- Host: `lt7300`
- Commit: `76867aa765d9cee4406c522d37c3e0dd5ec812c8`
- FOD version: `3.2.1`
- Artifact directory: `artifacts/perf/76867aa/lt7300-data-blocks-conflict-noop-20260703T140759Z`

## Large Copy Workload

- `elapsed_s`: `2.585200`
- `throughput_mib_s`: `24.76`
- `COPY fod_persist_block_stage total_exec_ms`: `n/a`
- `data_blocks merge total_exec_ms`: `n/a`

## WAL Delta

- `wal_records_delta`: `16`
- `wal_fpi_delta`: `0`
- `wal_bytes_delta`: `1266`
- `wal_buffers_full_delta`: `0`
- `wal_write_delta`: `3`
- `wal_sync_delta`: `3`
- `buffers_checkpoint_delta`: `0`
- `buffers_backend_delta`: `0`
- `buffers_backend_fsync_delta`: `0`

## Table DML Delta

- `data_blocks_n_tup_ins_delta`: `0`
- `data_blocks_n_tup_upd_delta`: `0`
- `data_blocks_n_tup_hot_upd_delta`: `0`
- `data_blocks_non_hot_update_delta`: `0`
- `data_blocks_hot_update_ratio_percent`: `n/a`
- `data_blocks_n_tup_del_delta`: `0`
- `data_blocks_n_dead_tup_delta`: `0`
- `idx_data_blocks_object_order_idx_scan_delta`: `922`
- `idx_data_blocks_object_order_idx_tup_read_delta`: `32768`
- `idx_data_blocks_object_order_idx_tup_fetch_delta`: `32768`

## Bloat / Churn Snapshot

- `data_blocks_n_live_tup`: `1771187`
- `data_blocks_n_dead_tup`: `17499`
- `data_blocks_relation_size`: `201 MB`
- `idx_data_blocks_object_order_relation_size`: `38 MB`

## Temp Merge Reproducer

- Run ID: `data-blocks-merge-filter-explain-20260703T140901Z`
- Artifact: `artifacts/perf/76867aa/lt7300-data-blocks-merge-filter-explain-20260703T140901Z/pg_data_blocks_merge_explain-merge-filter.txt`
- Fresh insert: `16384` inserted rows, `0` conflicts, `230.725 ms`.
- Identical conflict: `16384` conflicts, `16384` rows removed by the conflict filter, `0` inserted rows, `378.997 ms`, and no target-page dirty/write counters in the plan line.
- Changed conflict: `16384` conflicts, `16384` updated rows, `319.994 ms`, with `local hit=247217 dirtied=309 written=309`.

The temp-table timing is not production-representative, but it proves the `ON CONFLICT ... WHERE` guard independently from the higher-level FUSE block-delta path.

## Conclusion

The unchanged-block conflict filter avoided all data_blocks row rewrites for a 64 MiB same-payload overwrite: zero inserts, zero updates, zero dead tuples, and only minimal metadata WAL remained.

## Next Candidate

Keep the filter; next optimize the changed-payload full-overwrite case separately, likely through a data-object-level swap or another design that avoids non-HOT row rewrites without weakening correctness.
