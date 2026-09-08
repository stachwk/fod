# FOD current implementation plan

Status: 2026-09-08.

This file is the compact maintained implementation plan. It contains only work
that is still current enough to direct the next change. Completed execution
plans are retained under [`../history/`](../history/) and are not allowed to
compete with this file for current priority.

For current implemented behavior use [`../CURRENT_STATE.md`](../CURRENT_STATE.md).
For long-term direction use [`../../ROADMAP.md`](../../ROADMAP.md). `TODO.md`
remains a mixed follow-up/archive record and may contain historical sections
whose wording predates later architectural decisions.

## P1 — PostgreSQL-backed read-path efficiency

Preserve the strict read-only correctness boundary while removing measured
per-callback PostgreSQL work. Newly initialized filesystems use the current
32 KiB storage-block default; existing filesystems keep the `block_size`
persisted in `fod.config`.

### Measured FOD 3.4.15 4 KiB A/B

Host: `lt7300`; file size: 128 MiB; payload: pattern; storage block: 32 KiB;
FUSE direct I/O enabled; primary stopped before replica read.

The uncached diagnostic run measured:

```text
replica_read_mib_s=11.689
fetch_block_range_calls=32768
fetch_bytes_per_callback=32784
read_block_map_us_per_callback=294.771118
pg_fetch_us_per_callback=287.976898
```

With the existing production data-read settings
`4096 / 4 / 8 / 512 / 8`, replica 4 KiB throughput rose to
135.881 MiB/s, range-fetch calls fell from 32768 to 3, and fetched payload fell
from about 1 GiB to about 128 MiB. Do not add another data cache and do not
optimize `fod_fetch_block_range` first.

The same run exposed the next primary bottleneck:

```text
primary_read_mib_s=27.682
pg_prepared_statement name=fod_file_read_metadata count=32768 total_us=3367899 avg_us=102
```

The current `file_read_metadata_for_handle()` already keeps a per-handle
metadata snapshot for read-only direct-I/O mounts, but writable primary mounts
refetch metadata on every callback.

### FOD 3.4.16 measured result

FOD 3.4.16 bounded primary direct-I/O read-metadata reuse by the existing
metadata TTL for `O_RDONLY + noatime`, while preserving the existing replica
handle-cache behavior.

The production-read-default matrix on `lt7300`, 128 MiB pattern payload,
32 KiB persisted storage blocks, direct I/O, and
`metadata_cache_ttl_seconds=1` measured:

| fio request | primary read | replica read |
| --- | ---: | ---: |
| 4 KiB | 138.528 MiB/s | 149.184 MiB/s |
| 64 KiB | 258.586 MiB/s | 291.572 MiB/s |
| 512 KiB | 263.918 MiB/s | 288.939 MiB/s |

For the valid 4 KiB primary profile:

```text
fetch_block_range_calls=3
fetch_block_range_result_bytes=134250496
file_read_metadata_calls=1
file_read_metadata_total_us=273
```

The former primary metadata bottleneck is therefore closed. Do not reopen the
data-cache, metadata-cache or SQL-fetch design without a new measured
regression.

### FOD 3.4.17 measured result

FOD 3.4.17 fixed the integration-test log capture race and made missing final
PostgreSQL lane observability explicit. The full production-read-default matrix
on `lt7300`, 128 MiB pattern payload, 32 KiB persisted storage blocks,
`metadata_cache_ttl_seconds=1`, and forced FUSE direct I/O measured:

| fio request | primary read | replica read |
| --- | ---: | ---: |
| 4 KiB | 162.437 MiB/s | 181.047 MiB/s |
| 64 KiB | 303.318 MiB/s | 292.237 MiB/s |
| 512 KiB | 323.232 MiB/s | 314.496 MiB/s |

All six primary/replica read phases reported
`lane_observability_available=1`, one logical `stage=shutdown` snapshot and one
PostgreSQL `stage=post-mount` snapshot. No `no_final_observability=1` or cleanup
grace-period warning was present.

The 4 KiB profile remained efficient at the PostgreSQL/data-cache boundary:

```text
primary:
  admitted_tasks=32768
  fetch_block_range_calls=3
  file_read_metadata_calls=1
  pg_fetch_us_per_callback=9.943359

replica:
  admitted_tasks=32768
  fetch_block_range_calls=3
  file_read_metadata_calls=1
  pg_fetch_us_per_callback=9.625824
```

The remaining 4 KiB throughput gap is therefore not evidence for another
PostgreSQL metadata or range-fetch optimization.

### FOD 3.4.18 — benchmark production buffered-I/O behavior

The isolated benchmark still forces `FOD_FOPEN_DIRECT_IO=1`, while the standard
FOD configuration uses `fopen_direct_io=false`. That means the current matrix
measures every application read reaching FUSE directly and does not show how
the normal kernel page cache/readahead path changes callback count or
throughput.

FOD 3.4.18 is measurement-only:

- parameterize `FOD_FOPEN_DIRECT_IO` in the isolated benchmark, keeping `1` as
  the benchmark default so existing invocations retain their behavior;
- validate the benchmark value as exactly `0` or `1`;
- record `fopen_direct_io` in `read-policy.txt`, `result.tsv`, `PERF_RESULT`
  and matrix `summary.tsv`;
- only auto-label a zero-cache run as `uncached` when FUSE direct I/O is also
  enabled, because buffered kernel caching makes that label misleading;
- do not change Rust read-path, PostgreSQL SQL, storage geometry, or production
  configuration defaults.

After delivery, run the same 4 KiB / 64 KiB / 512 KiB matrix twice:

```text
A: FOD_FOPEN_DIRECT_IO=1   # current diagnostic baseline
B: FOD_FOPEN_DIRECT_IO=0   # standard production FOD default
```

Use explicit policy labels for the A/B runs. Compare throughput,
`admitted_tasks`, range-fetch counts, metadata calls and per-callback profile
cost before selecting any runtime optimization.

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
