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

The next measured performance target is the PostgreSQL-backed read path. Preserve
the strict read-only correctness boundary while reducing avoidable work behind
small FUSE read callbacks.

Current measurement boundary:

- treat 512 KiB as the effective measured FUSE read-callback ceiling until a
  newer negotiation/profile proves otherwise;
- newly initialized filesystems use the current 32 KiB storage-block default;
  existing filesystems keep the `block_size` persisted in `fod.config`;
- do not infer write sizing from read sizing — read and write tuning remain
  separate;
- preserve primary-unreachable replica-read validation, zero PostgreSQL write
  attempts on the read-only path, role validation and WAL/replay-LSN safety.

FOD 3.4.13 fixed attribution for the live prepared statement
`fod_fetch_block_range_with_size`. FOD 3.4.14 made every replica benchmark
artifact record `storage_block_size_bytes`.

### Measured strict 3.4.14 matrix

Host: `lt7300`; file size: 128 MiB; payload: pattern; persisted FOD storage block:
32 KiB; read cache/read-ahead/direct prefetch disabled; FUSE direct I/O enabled;
primary stopped before replica read. WAL and read-only guards passed for every
row.

| fio block size | primary read MiB/s | replica read MiB/s | FUSE callbacks | range-fetch calls |
| --- | ---: | ---: | ---: | ---: |
| 4 KiB | 16.364 | 16.277 | 32768 | 32768 |
| 64 KiB | 179.775 | 173.677 | 2048 | 2048 |
| 512 KiB | 308.434 | 312.195 | 256 | 256 |

There is exactly one `fod_fetch_block_range_with_size` per FUSE callback. The
4 KiB row nevertheless fetches the containing 32 KiB storage block each time;
over 128 MiB this produced about 1 GiB of fetched payload, roughly 8x logical
byte amplification. Prepared-statement execution dominates `read_block_map`;
result decoding is comparatively small. Primary and replica throughput are
also close, so this is an overall read-path issue, not a replica-routing issue.

The strict matrix intentionally disables the mechanisms that can reuse or
prefetch data across sequential callbacks. Measure the existing production read
policy before changing SQL or adding another cache.

### FOD 3.4.15 measurement step

FOD 3.4.15 keeps zero as the isolated benchmark default but allows these five
existing settings to be supplied externally and records them in `read-policy.txt`,
`result.tsv`, `PERF_RESULT` and matrix `summary.tsv`:

- `FOD_READ_CACHE_BLOCKS`;
- `FOD_READ_AHEAD_BLOCKS`;
- `FOD_SEQUENTIAL_READ_AHEAD_BLOCKS`;
- `FOD_DIRECT_IO_READ_PREFETCH_BLOCKS`;
- `FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS`.

Run a focused 4 KiB A/B comparison with 128 MiB.

Uncached baseline:

```bash
REPLICA_READ_PRIMARY_PORT=56441 \
REPLICA_READ_REPLICA_PORT=56442 \
REPLICA_READ_FIO_BLOCK_SIZES='4k' \
REPLICA_READ_FIO_FILE_SIZE=128M \
REPLICA_READ_POLICY_LABEL=uncached \
FOD_READ_CACHE_BLOCKS=0 \
FOD_READ_AHEAD_BLOCKS=0 \
FOD_SEQUENTIAL_READ_AHEAD_BLOCKS=0 \
FOD_DIRECT_IO_READ_PREFETCH_BLOCKS=0 \
FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS=0 \
make test-fio-primary-write-replica-read-matrix
```

Current production read defaults under the same direct-I/O benchmark boundary:

```bash
REPLICA_READ_PRIMARY_PORT=56441 \
REPLICA_READ_REPLICA_PORT=56442 \
REPLICA_READ_FIO_BLOCK_SIZES='4k' \
REPLICA_READ_FIO_FILE_SIZE=128M \
REPLICA_READ_POLICY_LABEL=production-read-defaults \
FOD_READ_CACHE_BLOCKS=4096 \
FOD_READ_AHEAD_BLOCKS=4 \
FOD_SEQUENTIAL_READ_AHEAD_BLOCKS=8 \
FOD_DIRECT_IO_READ_PREFETCH_BLOCKS=512 \
FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS=8 \
make test-fio-primary-write-replica-read-matrix
```

For both artifacts run `scripts/perf/extract_primary_replica_profile.sh` and
compare replica throughput, `fetch_calls_per_callback`,
`fetch_bytes_per_callback`, `read_block_map_us_per_callback`,
`pg_fetch_us_per_callback` and `pg_decode_us_per_callback`.

If the production read policy materially removes repeated 32 KiB fetches behind
4 KiB callbacks, tune that existing mechanism rather than adding a parallel
cache. If it remains near the uncached baseline, select one narrow runtime/SQL
optimization supported by the A/B profile.

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
