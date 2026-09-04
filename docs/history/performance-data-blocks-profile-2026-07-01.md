# FOD Data Blocks Profile - 2026-07-01

## Run Metadata

- Run ID: `data-blocks-current-20260701T205319Z`
- Host: `lt7300`
- Commit: `ac47828eb56c92226d29cafca78ee47304cedcdf`
- FOD version: `3.2.1`
- Artifact directory: `artifacts/perf/ac47828/lt7300-data-blocks-current-20260701T205319Z`

## Large Copy Workload

- `elapsed_s`: `3.882019`
- `throughput_mib_s`: `16.49`
- `COPY fod_persist_block_stage total_exec_ms`: `1224.804`
- `data_blocks merge total_exec_ms`: `996.545`

## WAL Delta

- `wal_records_delta`: `165652`
- `wal_fpi_delta`: `72`
- `wal_bytes_delta`: `13144230`
- `wal_buffers_full_delta`: `222`
- `wal_write_delta`: `243`
- `wal_sync_delta`: `21`
- `buffers_checkpoint_delta`: `131`
- `buffers_backend_delta`: `676`
- `buffers_backend_fsync_delta`: `0`

## Bloat / Churn Snapshot

- `data_blocks_n_live_tup`: `1507507`
- `data_blocks_n_dead_tup`: `91`
- `data_blocks_relation_size`: `168 MB`
- `idx_data_blocks_object_order_relation_size`: `32 MB`

## Conclusion

The real local path still shows server-side COPY plus data_blocks merge as the dominant cost; WAL is measurable but checkpoints did not interfere in this run.

## Next Candidate

Run COPY send buffer matrix and keep runtime SQL unchanged until repeated local/QNAP data identifies a stable next bottleneck.

## COPY Send Buffer Matrix

The local matrix below used the same commit, host, and real runtime path:

- Commit: `ac47828`
- Host: `lt7300`
- Workload: `FOD_PROFILE_IO=1 make test-large-copy-benchmark`
- Values: `262144`, `1048576`, `4194304`, `16777216`

| Send buffer bytes | Elapsed seconds | Throughput MiB/s | COPY send count/pass | COPY send seconds pass 1 | COPY send seconds pass 2 | `COPY ... FROM STDIN` total ms | `data_blocks` merge total ms | `wal_bytes_delta` | `wal_write_delta` | `wal_sync_delta` | `buffers_backend_delta` |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `262144` | `3.660268` | `17.49` | `256` | `0.017149` | `0.018755` | `1075.260` | `923.764` | `12871436` | `261` | `21` | `673` |
| `1048576` | `3.616664` | `17.70` | `65` | `0.021429` | `0.023077` | `1133.993` | `943.022` | `12878664` | `69` | `20` | `674` |
| `4194304` | `3.787409` | `16.90` | `17` | `0.025870` | `0.026203` | `1158.919` | `955.387` | `12850860` | `190` | `21` | `674` |
| `16777216` | `3.824431` | `16.73` | `5` | `0.043647` | `0.036780` | `1288.005` | `986.874` | `12853874` | `153` | `20` | `677` |

The bloat/churn snapshots after each run showed growing table size because each benchmark creates new test files, but not growing dead tuple pressure:

| Send buffer bytes | `data_blocks_n_live_tup` | `data_blocks_n_dead_tup` | `data_blocks_relation_size` | `idx_data_blocks_object_order` size |
| ---: | ---: | ---: | ---: | ---: |
| `262144` | `1540275` | `91` | `172 MB` | `33 MB` |
| `1048576` | `1573043` | `91` | `176 MB` | `34 MB` |
| `4194304` | `1605811` | `91` | `179 MB` | `34 MB` |
| `16777216` | `1638579` | `91` | `183 MB` | `35 MB` |

Interpretation:

- Client-side COPY send buffer size changes the number of `PQputCopyData` calls, but the elapsed time stays in the same local band.
- The `1 MiB` default was fastest in this single local matrix, and `256 KiB` was close enough that the difference should be treated as normal run-to-run variance.
- WAL volume stayed stable at about `12.85-12.88 MB`; no checkpoint request or timed checkpoint happened during the matrix runs.
- The next candidate is server-side staging/COPY and `data_blocks` conflict merge analysis, not a default change to `FOD_PERSIST_COPY_SEND_BUFFER_BYTES`.
