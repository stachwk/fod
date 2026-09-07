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

### FOD 3.4.17 — deterministic final observability capture

The 3.4.16 matrix exposed an integration-test artifact race, not a new
read-path bottleneck. Primary archived logs were sometimes copied before the
bootstrap/Rust FUSE process completed normal post-unmount shutdown logging:

- the 64 KiB primary artifact contained startup output but no final logical,
  boundary or PostgreSQL lane observability;
- the 4 KiB and 512 KiB primary artifacts contained useful shutdown profile
  data but no `stage=post-mount` PostgreSQL lane snapshot;
- runtime code emits `stage=post-mount` only after the mount returns, so its
  absence from these archives indicates incomplete log capture.

FOD 3.4.17 changes only test/diagnostic behavior:

- after unmount, `fod_test_cleanup` gives the bootstrap/FUSE process a bounded
  grace period to exit naturally;
- cleanup reaps the bootstrap before inspecting or archiving the log;
- forced termination remains as the bounded fallback;
- a regression test proves that final process output is present in the
  archived log;
- compact profile output exposes `lane_observability_available=0|1` instead of
  silently making a missing final lane snapshot look like genuine zero
  PostgreSQL operations.

Acceptance test: rerun the production 4 KiB / 64 KiB / 512 KiB primary/replica
matrix and require `lane_observability_available=1` for every primary and
replica read phase, with no `no_final_observability=1`.

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
