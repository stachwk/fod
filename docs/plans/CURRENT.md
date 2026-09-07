# FOD current implementation plan

Status: 2026-09-07.

This file is the compact maintained implementation plan. It contains only work
that is still current enough to direct the next change. Completed execution
plans are retained under [`../history/`](../history/) and are not allowed to
compete with this file for current priority.

For current implemented behavior use [`../CURRENT_STATE.md`](../CURRENT_STATE.md).
For long-term direction use [`../../ROADMAP.md`](../../ROADMAP.md). `TODO.md`
remains a mixed follow-up/archive record and may contain historical sections
whose wording predates later architectural decisions.

## P1 — replica read-path efficiency

The next measured performance target is the PostgreSQL-backed replica read path.
Preserve the strict read-only correctness boundary while reducing avoidable work
behind one FUSE read callback.

Current measurement boundary:

- treat 512 KiB as the effective measured FUSE read-callback ceiling until a
  newer negotiation/profile proves otherwise;
- keep the FOD logical storage block at 4 KiB;
- do not infer write sizing from read sizing — read and write tuning remain
  separate;
- preserve primary-unreachable replica-read validation, zero PostgreSQL write
  attempts on the read-only path, role validation and WAL/replay-LSN safety.

FOD 3.4.12 added a compact read-path attribution report assembled from existing
`FOD_PROFILE_IO=1` telemetry. A real 3.4.12 matrix exposed that the live read
path uses prepared statement `fod_fetch_block_range_with_size`, while the first
extractor revision looked only for the older `fod_fetch_block_range` name.
Therefore the raw 3.4.12 run was valid, but its compact attribution incorrectly
reported `profile_attribution_available=0` and zero fetch calls.

FOD 3.4.13 fixes the extractor to prefer
`fod_fetch_block_range_with_size` while retaining compatibility with historical
`fod_fetch_block_range` logs. Compact output also reports the selected
`fetch_statement_name` explicitly.

### Measured 3.4.12 strict matrix

Host: `lt7300`; file size: 128 MiB; payload: pattern; read cache/read-ahead/direct
prefetch disabled; FUSE direct I/O enabled; primary stopped before replica read.
The strict WAL and read-only guards passed for every row.

| fio block size | primary read MiB/s | replica read MiB/s | FUSE read callbacks | live range-fetch calls from raw log |
| --- | ---: | ---: | ---: | ---: |
| 4 KiB | 16.465 | 16.260 | 32768 | 32768 |
| 64 KiB | 178.273 | 179.021 | 2048 | 2048 |
| 512 KiB | 289.593 | 315.271 | 256 | 256 |

The raw boundary profile already shows that `read_block_map` dominates the FUSE
read time and that the live `fod_fetch_block_range_with_size` count is one per
callback in all three rows. The corrected 3.4.13 extractor must now quantify
SQL time, decode time, bytes/rows per fetch and fixed non-fetch operations from
the same artifact before any Rust hot-path change is selected.

### Next measurement step

Do **not** rerun the full matrix just to repair attribution. After updating to
3.4.13, re-extract the existing successful matrix artifact:

```bash
MATRIX_DIR=artifacts/perf/10b7acd/lt7300-docker-matrix-matrix-20260907T084527Z

scripts/perf/extract_primary_replica_profile.sh "$MATRIX_DIR" \
  | tee "$MATRIX_DIR/read-path-attribution-3.4.13.txt"
```

Compare the `replica-read` rows, especially:

- `fetch_statement_name` and `profile_attribution_available`;
- `pg_operations_per_callback` and `fetch_calls_per_callback`;
- `fetch_rows_per_call`, `fetch_bytes_per_call` and
  `fetch_bytes_per_callback`;
- `read_block_map_us_per_callback`, `pg_fetch_us_per_callback` and
  `pg_decode_us_per_callback`;
- `non_fetch_operation_count`.

Only after that corrected attribution choose one narrow optimization. The first
candidate must be supported by the measured split between prepared-statement
execution, result decoding and fixed non-fetch operations; do not infer an
extra round trip when the raw log already shows one range-fetch call per FUSE
callback.

For a future repeat of the focused matrix, the Make variables are:

```bash
REPLICA_READ_PRIMARY_PORT=56441 \
REPLICA_READ_REPLICA_PORT=56442 \
REPLICA_READ_FIO_BLOCK_SIZES='4k 64k 512k' \
REPLICA_READ_FIO_FILE_SIZE=128M \
make test-fio-primary-write-replica-read-matrix
```

Do not change the storage format, quota model, write request default or unrelated
routing policy in the same optimization commit.

## P2 — remaining multi-endpoint hardening

The base role-aware routing design is already implemented: startup endpoint role
selection, runtime primary failover for HA/proxy entrypoints representing one
authoritative PostgreSQL cluster, WAL-gated replica reads, replica scoring,
promotion validation and process-local generation fencing all have delivered
runtime support.

Remaining work must be phrased as a specific hardening gap rather than reopening
the original broad multi-endpoint design. In particular:

- never present independent writable PostgreSQL primaries as one safe
  multi-primary filesystem;
- keep authoritative writes/control/lease work on a verified writable primary;
- keep replica reads behind role and replay-consistency checks;
- treat external/cross-process fencing or fairness as separate work only when
  its ownership and acceptance tests are explicit.

## Delivery rule

Each implementation commit uses the next sequential FOD version, updates the
relevant current documentation/tests, stays on `main`, and uses:

```text
FOD X.Y.Z: <English description>
```

After every commit compare it with its parent using `git diff HEAD~1..HEAD` or
`git show` and inspect the complete change for accidental files, missing updates,
regressions and scope drift.

## Historical plans

The former 2026-08-26 action plan and the completed block/write/replay plans are
kept under [`../history/`](../history/) as implementation evidence. They explain
how earlier states were reached but no longer define the next task.
