# FOD Data Blocks Repeated Full-Overwrite Swap Profile - 2026-07-04

## Context

- Commit: `60658e878c136728df023d08f9c88a88176cb824`
- Commit subject: `FOD 3.2.1: fix deferred data object GC script`
- FOD version: `3.2.1`
- Host: `lt7300`
- PostgreSQL: local Docker PostgreSQL
- Workload: `profile-data-blocks-swap-repeat-dml`
- Shape: seed one 64 MiB file, then run five changed-payload full overwrites of the same logical file.
- Isolation: `make reset` before the immediate run and again before the deferred run.

## Runs

| Policy | Run ID | Mean overwrite elapsed_s | Mean throughput MiB/s |
| --- | --- | ---: | ---: |
| `immediate` | `data-blocks-swap-repeat-immediate-20260703T221936Z` | `1.435899` | `44.59` |
| `deferred` | `data-blocks-swap-repeat-deferred-20260703T222026Z` | `1.405703` | `45.53` |

## Hot Path DML And WAL

| Metric | `immediate` | `deferred` |
| --- | ---: | ---: |
| `data_blocks_n_tup_ins_delta` | `81920` | `81920` |
| `data_blocks_n_tup_del_delta` | `81920` | `0` |
| `data_blocks_n_tup_upd_delta` | `0` | `0` |
| `data_blocks_non_hot_update_delta` | `0` | `0` |
| `data_blocks_n_live_tup_delta` | `0` | `81920` |
| `data_blocks_n_dead_tup_delta` | `49152` | `0` |
| `data_blocks_autovacuum_count_delta` | `1` | `1` |
| `data_blocks_autoanalyze_count_delta` | `1` | `1` |
| `data_blocks_relation_size_bytes_delta` | `10174464` | `15605760` |
| `data_blocks_total_size_bytes_delta` | `14385152` | `19816448` |
| `idx_data_blocks_object_order_relation_size_bytes_delta` | `1835008` | `1835008` |
| `idx_data_blocks_data_object_id_relation_size_bytes_delta` | `532480` | `532480` |
| `wal_records_delta` | `499295` | `415428` |
| `wal_bytes_delta` | `43069493` | `37976180` |
| `wal_buffers_full_delta` | `904` | `754` |
| `wal_write_delta` | `959` | `807` |
| `wal_sync_delta` | `54` | `53` |

## Deferred GC Phase

After the deferred hot-path run:

| Metric | Value |
| --- | ---: |
| `candidate_data_objects` | `6` |
| `deleted_data_blocks` | `81920` |
| `deleted_data_objects` | `6` |
| `data_blocks_n_tup_del_delta` | `81920` |
| `data_blocks_n_live_tup_delta` | `-81920` |
| `data_blocks_n_dead_tup_delta` | `81920` |
| `data_blocks_relation_size_bytes_delta` | `0` |
| `wal_records_delta` | `81981` |
| `wal_bytes_delta` | `4429524` |
| `wal_write_delta` | `45` |
| `wal_sync_delta` | `6` |

Post-GC consistency check:

| Check | Value |
| --- | ---: |
| `unreferenced_data_objects` | `0` |
| `blocks_without_object` | `0` |
| `files_without_object` | `0` |

## Interpretation

The full-overwrite data-object swap still eliminates changed-payload `data_blocks` non-HOT updates in both cleanup policies. The remaining tradeoff is where old-object cleanup happens.

`deferred` slightly improved the hot overwrite phase on this local five-run smoke: lower mean elapsed time, lower WAL bytes, fewer WAL records, and no hot-path `data_blocks` deletes/dead tuples. The cost reappeared in the explicit GC phase as `81920` deletes, `81920` new dead tuples, and about `4.43 MB` WAL.

Combined hot path plus GC, the deferred policy is only marginally better on WAL in this local run (`37976180 + 4429524 = 42405704` bytes) than immediate cleanup (`43069493` bytes). It also keeps extra live rows until GC runs and grows the relation more during the hot phase.

## Decision

Keep `data_object_swap_cleanup = immediate` as the default because it is simpler and does not require a maintenance contract. Keep `deferred` as an opt-in policy for workloads where shorter write transactions matter more than temporary relation growth, but only with scheduled `profile-pg-data-object-gc` / object-GC maintenance.

Delayed cleanup/object-GC is no longer a theoretical next step: the minimal implementation works and has measured behavior. Further work should focus on production policy, scheduling, and remote/QNAP repeats rather than reintroducing changed-payload conflict updates.
