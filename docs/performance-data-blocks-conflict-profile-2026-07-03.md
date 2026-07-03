# FOD Data Blocks Conflict Update Profile - 2026-07-03

## Run Metadata

- Run ID: `data-blocks-conflict-20260703T135637Z`
- Host: `lt7300`
- Commit: `19696742e82220e2c46355d55078be463759ee65`
- FOD version: `3.2.1`
- Artifact directory: `artifacts/perf/1969674/lt7300-data-blocks-conflict-20260703T135637Z`

## Workload

The profile uses `profile-data-blocks-conflict-dml`:

1. `test-data-blocks-conflict-seed` creates a 64 MiB file with `4M x 16` writes.
2. DML and WAL snapshots are captured.
3. `test-data-blocks-conflict-overwrite-benchmark` overwrites the same logical file with a different 64 MiB payload.
4. DML, WAL, top SQL, and bloat snapshots are captured after the overwrite.

The DML delta therefore measures only the conflict-update overwrite phase, not the initial seed insert.

## Overwrite Workload

- `bytes`: `67108864`
- `elapsed_s`: `1.169478`
- `throughput_mib_s`: `54.73`
- `COPY fod_persist_block_stage total_exec_ms`: `534.021`
- `data_blocks conflict merge total_exec_ms`: `397.522`

## Table DML Delta

- `data_blocks_n_tup_ins_delta`: `0`
- `data_blocks_n_tup_upd_delta`: `16384`
- `data_blocks_n_tup_hot_upd_delta`: `0`
- `data_blocks_n_tup_newpage_upd_delta`: `16384`
- `data_blocks_non_hot_update_delta`: `16384`
- `data_blocks_hot_update_ratio_percent`: `0`
- `data_blocks_n_tup_del_delta`: `0`
- `data_blocks_n_live_tup_delta`: `0`
- `data_blocks_n_dead_tup_delta`: `16384`
- `idx_data_blocks_object_order_idx_scan_delta`: `16385`
- `idx_data_blocks_object_order_idx_tup_read_delta`: `16416`
- `idx_data_blocks_object_order_idx_tup_fetch_delta`: `16416`
- `data_blocks_relation_size_bytes_delta`: `2310144`
- `data_blocks_total_size_bytes_delta`: `3170304`
- `idx_data_blocks_object_order_relation_size_bytes_delta`: `368640`
- `idx_data_blocks_data_object_id_relation_size_bytes_delta`: `122880`

## WAL Delta

- `wal_records_delta`: `99680`
- `wal_fpi_delta`: `5`
- `wal_bytes_delta`: `8493118`
- `wal_buffers_full_delta`: `300`
- `wal_write_delta`: `309`
- `wal_sync_delta`: `8`
- `buffers_checkpoint_delta`: `43`
- `buffers_backend_delta`: `387`
- `buffers_backend_fsync_delta`: `0`

## Conclusion

The real overwrite conflict path is not HOT-update friendly on this local database. The 64 MiB overwrite produced `16384` `data_blocks` updates, all of them non-HOT, and grew `n_dead_tup` by `16384`. That confirms the earlier insert-heavy large-copy profile was not enough to judge heap rewrite behavior.

## Next Candidate

The next SQL optimization should focus on reducing or avoiding non-HOT rewrites in the conflict update path. Candidate directions are: avoid unnecessary `DO UPDATE` when staged block data is unchanged, evaluate whether full overwrite should create a new data object and swap metadata instead of updating every existing block row, and measure table/index bloat under repeated overwrite runs before changing production SQL.
