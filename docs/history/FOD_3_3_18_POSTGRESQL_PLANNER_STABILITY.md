# FOD 3.3.18 - PostgreSQL planner stability for block ranges

## Status

Status: validated and completed for FOD 3.3.18.

The FOD 3.3.17 bulk read-after-write path remains unchanged. Storage stays at
4 KiB per block. This change targets PostgreSQL planner stability around
`data_blocks`; it does not change the FUSE request ceiling or batching
semantics.

## Evidence from FOD 3.3.17

The range path scales correctly:

- 512 KiB -> 128 rows per `fod_fetch_block_range` call;
- 1 MiB -> 256 rows per `fod_fetch_block_range` call;
- the tested host accepts a 1 MiB FUSE request ceiling with 4 KiB pages and
  `max_pages_limit=256`.

A 512 KiB mixed-random workload without refreshing PostgreSQL statistics after
preparing a dense 1 GiB file became unstable:

| run | read MiB/s | `fod_fetch_block_range` avg |
| --- | ---: | ---: |
| 1 | 41.472 | 0.987 ms |
| 2 | 11.411 | 32.541 ms |
| 3 | 9.717 | 40.494 ms |

The degraded runs processed about 257 thousand fetched tuples per range query
while returning only 128 block rows. A 1 GiB file has 262,144 FOD blocks at
4 KiB, so the bad plan approached a whole-object scan for each range.

The schema offered two competing indexes:

- `idx_data_blocks_object_order(data_object_id, _order)` - canonical range
  index;
- `idx_data_blocks_data_object_id(data_object_id)` - redundant prefix index.

The compound index can also serve lookups constrained only by
`data_object_id`.

## Controlled 2x2 result

After building the 1 GiB dataset, `ANALYZE fod.data_blocks`,
`ANALYZE fod.data_objects`, and `ANALYZE fod.files` were run before measurement.

### INDEX + ANALYZE

| run | read MiB/s | write MiB/s | range avg |
| --- | ---: | ---: | ---: |
| 1 | 40.383 | 42.653 | 1.211 ms |
| 2 | 40.900 | 43.200 | 1.285 ms |
| 3 | 40.923 | 41.405 | 1.358 ms |

### NO INDEX + ANALYZE

| run | read MiB/s | write MiB/s | range avg |
| --- | ---: | ---: | ---: |
| 1 | 41.355 | 43.680 | 1.233 ms |
| 2 | 41.569 | 43.907 | 1.273 ms |
| 3 | 40.244 | 40.718 | 1.402 ms |

The analyzed variants are effectively equal within measurement noise. The
catastrophic regression therefore came from stale/inadequate planner statistics
after the large data load, not from 512 KiB FUSE requests and not from the
3.3.17 bulk range implementation.

Removing the single-column index is retained as a structural simplification and
planner guard, not as a claimed throughput optimization.

## FOD 3.3.18 implementation

1. Keep `idx_data_blocks_object_order(data_object_id, _order)`.
2. Stop creating `idx_data_blocks_data_object_id(data_object_id)` on fresh
   installs.
3. Add migration 0023 to drop that redundant index on existing databases.
4. Run `ANALYZE fod.data_blocks` once from migration 0023 after the drop.
   Never add `ANALYZE` to normal read/write/flush hot paths.
5. Require the compound index and absence of the redundant index in the
   latest-schema shape check, so missing `schema_version` recovery cannot skip
   migration 0023.
6. Cover the final index shape and migration manifest in schema tests.
7. Bump FOD to 3.3.18.
8. Run `fod-rust-mkfs` tests and the normal full project gate.
9. Re-run 4 KiB, 256 KiB, 512 KiB and 1 MiB performance validation after the
   schema change.

Large synthetic planner diagnostics must refresh table statistics after
building the dataset and before measuring. Production FOD continues to rely on
normal PostgreSQL autovacuum/autoanalyze behavior plus the one-time migration
`ANALYZE`.

## Final validation

Final pre-push validation used a dense 1 GiB file, 512 MiB of mixed
`randrw50` I/O per run, three runs per block size, and explicit `ANALYZE` after
dataset preparation.

| fio block size | median read MiB/s | median write MiB/s |
| --- | ---: | ---: |
| 4 KiB | 13.642 | 13.637 |
| 256 KiB | 42.908 | 44.341 |
| 512 KiB | 41.390 | 43.431 |
| 1 MiB | 44.487 | 47.090 |

The critical 512 KiB case stayed stable across all runs:

| run | read MiB/s | write MiB/s | `fod_fetch_block_range` avg |
| --- | ---: | ---: | ---: |
| 1 | 40.914 | 43.214 | 1.269 ms |
| 2 | 41.390 | 43.717 | 1.377 ms |
| 3 | 42.925 | 43.431 | 1.331 ms |

The previous warm-run collapse to roughly 10 MiB/s and 30-40 ms range-query
latency did not recur.

The 1 MiB case was also stable at 43.409-44.584 MiB/s read throughput.
Each `fod_fetch_block_range` returned 256 rows per query, confirming a full
1 MiB range at the 4 KiB storage block size.

Conclusion: migration 0023 and the final schema shape pass the performance
regression gate. FOD 3.3.18 is complete.

## Explicit non-goals

FOD 3.3.18 does not change:

- storage `block_size=4096`;
- `write_flush_threshold=64MiB`;
- base `persist_buffer_chunk_blocks=128` (512 KiB);
- read-ahead settings;
- `fs.fuse.max_pages_limit`;
- `fuse_max_readahead_bytes`;
- the standard configuration's `fuse_max_write_bytes` pin.

Normalization of the standard `fuse_max_write_bytes` from 512 KiB to the
already validated 1 MiB code default stays a separate follow-up change so its
A/B remains independent.

## Commit review

After committing 3.3.18, compare it with its parent using
`git diff HEAD~1..HEAD` or `git show` and inspect the complete change for
accidental edits, missing migration wiring, version mismatches, regressions, or
scope drift before continuing.
