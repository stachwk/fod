# FOD Data Blocks Profile - 2026-07-03

## Run Metadata

- Run ID: `data-blocks-dml-20260703T134344Z`
- Host: `lt7300`
- Commit: `c5d7f241fc98fd5ac410e1ad89d63b7a2d23cd50`
- FOD version: `3.2.1`
- Artifact directory: `artifacts/perf/c5d7f24/lt7300-data-blocks-dml-20260703T134344Z`

## Large Copy Workload

- `elapsed_s`: `3.798770`
- `throughput_mib_s`: `16.85`
- `COPY fod_persist_block_stage total_exec_ms`: `1221.565`
- `data_blocks merge total_exec_ms`: `1089.486`

## WAL Delta

- `wal_records_delta`: `165633`
- `wal_fpi_delta`: `57`
- `wal_bytes_delta`: `13045983`
- `wal_buffers_full_delta`: `104`
- `wal_write_delta`: `128`
- `wal_sync_delta`: `24`
- `buffers_checkpoint_delta`: `0`
- `buffers_backend_delta`: `0`
- `buffers_backend_fsync_delta`: `0`

## Table DML Delta

- `data_blocks_n_tup_ins_delta`: `32768`
- `data_blocks_n_tup_upd_delta`: `0`
- `data_blocks_n_tup_hot_upd_delta`: `0`
- `data_blocks_non_hot_update_delta`: `0`
- `data_blocks_hot_update_ratio_percent`: `n/a`
- `data_blocks_n_tup_del_delta`: `0`
- `data_blocks_n_dead_tup_delta`: `0`
- `idx_data_blocks_object_order_idx_scan_delta`: `32768`
- `idx_data_blocks_object_order_idx_tup_read_delta`: `0`
- `idx_data_blocks_object_order_idx_tup_fetch_delta`: `0`

## Bloat / Churn Snapshot

- `data_blocks_n_live_tup`: `1704115`
- `data_blocks_n_dead_tup`: `91`
- `data_blocks_relation_size`: `190 MB`
- `idx_data_blocks_object_order_relation_size`: `37 MB`

## Conclusion

The real local large-copy path inserted 32768 data_blocks rows with zero data_blocks UPDATE/HOT/dead-tuple growth; this run measures insert-heavy COPY plus conflict lookup, not a conflict-update heap rewrite case.

## Next Candidate

Add or run a targeted overwrite/conflict workload if the next question is HOT update eligibility for real data_blocks rewrites; keep production SQL unchanged until that separate update-heavy evidence exists.
