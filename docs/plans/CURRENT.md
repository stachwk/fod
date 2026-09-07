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

### FOD 3.4.16 implementation and measurement

FOD 3.4.16 extends read-metadata reuse to a deliberately narrow primary case:

- FUSE direct I/O is enabled;
- the file handle is `O_RDONLY`;
- atime policy is `noatime`;
- `metadata_cache_ttl_seconds` is greater than zero.

Primary reuse is bounded by the existing metadata cache TTL. The existing
read-only/replica direct-I/O behavior is preserved. Local file mutations
invalidate cached data blocks and per-handle read metadata through the same
file-cache invalidation boundary.

The isolated benchmark now parameterizes and records
`metadata_cache_ttl_seconds`. Compact profile output reports
`file_read_metadata_calls`, `file_read_metadata_total_us`, and
`file_read_metadata_us_per_callback`.

Verify with:

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
FOD_METADATA_CACHE_TTL_SECONDS=1 \
make test-fio-primary-write-replica-read-matrix
```

Acceptance criteria:

- primary `file_read_metadata_calls` falls from one per 4 KiB callback to a
  small TTL-bounded number;
- primary 4 KiB throughput materially improves from 27.682 MiB/s;
- production range-fetch efficiency does not regress;
- replica remains read-only with zero operation failures and rejected writes;
- storage format, quota, write path and routing policy remain unchanged.

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
