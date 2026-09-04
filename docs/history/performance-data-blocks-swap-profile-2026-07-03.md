# FOD Data Blocks Profile - 2026-07-03

## Run Metadata

- Run ID: `data-blocks-swap-20260703T215237Z`
- Host: `lt7300`
- Commit: `0eb2d0efb2b62bb38b6c1e0be16a16f8a0b44524`
- FOD version: `3.2.1`
- Artifact directory: `artifacts/perf/0eb2d0e/lt7300-data-blocks-swap-20260703T215237Z`

## Full-Overwrite Workload

- `elapsed_s`: `2.955563`
- `throughput_mib_s`: `21.65`
- `COPY fod_persist_block_stage total_exec_ms`: `1217.262`
- `data_blocks insert/merge total_exec_ms`: `1244.614`

## WAL Delta

- `wal_records_delta`: `99199`
- `wal_fpi_delta`: `11`
- `wal_bytes_delta`: `7754478`
- `wal_buffers_full_delta`: `0`
- `wal_write_delta`: `13`
- `wal_sync_delta`: `13`
- `buffers_checkpoint_delta`: `0`
- `buffers_backend_delta`: `0`
- `buffers_backend_fsync_delta`: `0`

## Table DML Delta

- `data_blocks_n_tup_ins_delta`: `16384`
- `data_blocks_n_tup_upd_delta`: `0`
- `data_blocks_n_tup_hot_upd_delta`: `0`
- `data_blocks_non_hot_update_delta`: `0`
- `data_blocks_hot_update_ratio_percent`: `n/a`
- `data_blocks_n_tup_del_delta`: `16384`
- `data_blocks_n_dead_tup_delta`: `33883`
- `idx_data_blocks_object_order_idx_scan_delta`: `16385`
- `idx_data_blocks_object_order_idx_tup_read_delta`: `32`
- `idx_data_blocks_object_order_idx_tup_fetch_delta`: `32`

## Bloat / Churn Snapshot

- `data_blocks_n_live_tup`: `1788083`
- `data_blocks_n_dead_tup`: `34395`
- `data_blocks_relation_size`: `205 MB`
- `idx_data_blocks_object_order_relation_size`: `39 MB`

## Consistency Check

- `unreferenced_data_objects`: `0`
- `blocks_without_object`: `0`
- `files_without_object`: `0`

## Conclusion

Full-overwrite data-object swap removed changed-payload data_blocks conflict updates from the profiled local overwrite path: data_blocks updates and non-HOT updates dropped to zero. The path now writes a new data object and deletes the old object rows, so remaining write amplification is insert/delete churn and dead tuple cleanup rather than heap rewrite updates.

## Next Candidate

Measure repeated full-overwrite runs and evaluate delayed cleanup or object-GC policy if insert/delete churn and dead tuples become the next bottleneck; do not reintroduce changed-payload conflict updates.
