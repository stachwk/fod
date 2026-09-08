# Historical FOD read/write tuning plan snapshot

> Archived on 2026-09-08. The material below is the former maintained
> implementation plan through the FOD 3.4.16-3.4.20 read/write tuning work.
> It is retained as measurement and implementation evidence and no longer
> defines current priority.

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

### FOD 3.4.18 measured direct-I/O vs buffered-I/O result

FOD 3.4.18 parameterized the benchmark's FUSE `fopen_direct_io` policy without
changing the production default. The same `lt7300`, 128 MiB pattern payload,
32 KiB persisted storage-block matrix measured:

| fio request | operation | direct I/O | buffered I/O |
| --- | --- | ---: | ---: |
| 4 KiB | primary write | 73.437 MiB/s | 5.067 MiB/s |
| 4 KiB | primary read | 166.884 MiB/s | 278.261 MiB/s |
| 4 KiB | replica read | 185.239 MiB/s | 252.465 MiB/s |
| 64 KiB | primary write | 98.462 MiB/s | 50.533 MiB/s |
| 64 KiB | primary read | 278.261 MiB/s | 296.984 MiB/s |
| 64 KiB | replica read | 315.271 MiB/s | 286.353 MiB/s |
| 512 KiB | primary write | 100.550 MiB/s | 88.398 MiB/s |
| 512 KiB | primary read | 300.469 MiB/s | 290.249 MiB/s |
| 512 KiB | replica read | 339.523 MiB/s | 131.282 MiB/s |

Buffered I/O reduced sequential read callbacks to about 512 independently of
the fio request size:

```text
4 KiB:   516 callbacks
64 KiB:  513 callbacks
512 KiB: 512 callbacks
```

This closes the earlier concern that production 4 KiB application reads must
always pay 32768 FUSE callbacks. Do not optimize that direct-I/O callback count
as a production requirement.

The buffered profiles also exposed concurrent read-cache fill amplification.
For a logical 128 MiB read, PostgreSQL returned substantially more than one
file's 4096 storage blocks:

```text
4 KiB primary:    calls=23  rows=7991 bytes=261913016
4 KiB replica:    calls=20  rows=7997 bytes=262109672
64 KiB primary:   calls=28  rows=7880 bytes=258274880
64 KiB replica:   calls=45  rows=7680 bytes=251719680
512 KiB primary:  calls=24  rows=7936 bytes=260110336
512 KiB replica:  calls=132 rows=6900 bytes=226154400
```

The current cache path checks missing blocks under the cache mutex, releases
that mutex before PostgreSQL fetch, and stores the returned blocks afterward.
Concurrent callbacks can therefore observe the same miss and issue duplicate
range fetches before the first callback fills the cache.

The buffered-write regression is tracked separately: 4 KiB write fell to
5.067 MiB/s and showed about three PostgreSQL operations per write callback,
including repeated path/directory lookup work. Do not mix that independent
write-path problem into the read-fill change.

### FOD 3.4.19 — coalesce concurrent buffered read-cache fills

Implement overlap-aware per-file coordination around buffered read-cache miss
fills:

- direct-I/O retains its existing cache/fetch path;
- the first buffered callback owns its currently missing block ranges;
- another callback waits only when its missing ranges overlap an active fill
  for the same `file_id`;
- non-overlapping miss ranges for the same file and ranges for different files
  remain independently fillable in parallel;
- waiters never hold the read-block-cache mutex while blocked;
- after an overlapping owner stores fetched blocks, waiters recheck the cache
  rather than executing a stale duplicate fetch;
- existing `workers_read` parallelism inside the owning fill remains intact;
- owner release wakes waiters even when the fetch returns an error;
- expose `read_fill_wait_count` and `read_fill_wait_us` in the boundary profile
  and compact primary/replica extractor.

Acceptance for the same buffered 128 MiB matrix:

```text
fetch_block_range_result_rows  -> close to 4096
fetch_block_range_result_bytes -> close to 134250496
```

with elimination of duplicate PostgreSQL result rows/bytes and a material
reduction of the worst observed range-fetch case, especially the previous
512 KiB replica run with 132 calls. Read correctness, replica read-only
enforcement and direct-I/O behavior must not regress.

Measured validation on `lt7300` with the 128 MiB pattern payload and
`fopen_direct_io=false`:

| fio request | primary read | replica read | primary fetches | replica fetches |
| --- | ---: | ---: | ---: | ---: |
| 4 KiB | 281.938 MiB/s | 285.078 MiB/s | 49 | 34 |
| 64 KiB | 283.186 MiB/s | 286.353 MiB/s | 53 | 56 |
| 512 KiB | 300.469 MiB/s | 278.261 MiB/s | 56 | 66 |

All six measured read phases returned exactly 4096 storage blocks and
134250496 PostgreSQL result bytes for the logical 128 MiB file, with zero
operation failures. The previous concurrent buffered-read amplification is
therefore closed. Final observability was complete and the replica write guard
remained `read_only_rejected`.

Three additional 512 KiB range-aware repetitions produced primary-read
throughput of 290.909, 324.051 and 285.078 MiB/s and replica-read throughput of
288.939, 286.353 and 284.444 MiB/s. The median remained within 5% of the
simpler per-file single-flight prototype while preserving exact one-file
PostgreSQL payload retrieval in every run.

The remaining buffered-write regression is independent and stays as a
follow-up rather than being mixed into the read-cache coordination change.

### FOD 3.4.20 — avoid repeated xattr pathname resolution during writes

A debug 4 MiB / 4 KiB buffered-write run isolated the remaining hot path:
1024 actual FUSE write callbacks were accompanied by 1025
`getxattr security.capability` probes for the same open file. Those probes
caused `fod_get_dir_id` and `fod_resolve_path_root` to execute approximately
once per write callback because the path-based xattr reader resolved the
pathname before every xattr lookup.

FOD 3.4.20 removes only that redundant owner-resolution work:

- `DbRepo::fetch_xattr_value_for_owner(owner_kind, owner_id, name)` performs
  the actual xattr lookup when the owner is already known;
- `getxattr` reuses the underlying `file_id` already stored in the open FUSE
  handle table for regular files and hardlinks;
- unopened files, directories, symlinks and cases without a usable open-file
  identity retain the existing path-resolution fallback;
- positive and negative xattr values are not cached, so cross-mount xattr
  changes remain visible on the next request;
- boundary/profile extraction reports the getxattr fast-path and fallback
  counters.

The final 128 MiB buffered matrix used the production read policy with
`fopen_direct_io=0`:

| fio request | primary write | primary read | replica read | write callbacks | PG operations | PG ops/callback |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 KiB | 14.339 MiB/s | 291.572 MiB/s | 276.458 MiB/s | 32768 | 32865 | 1.002960 |
| 64 KiB | 75.784 MiB/s | 292.237 MiB/s | 286.996 MiB/s | 2048 | 2131 | 1.040527 |
| 512 KiB | 97.859 MiB/s | 324.051 MiB/s | 285.078 MiB/s | 256 | 339 | 1.324219 |

Compared with the final FOD 3.4.19 buffered matrix, write throughput improved
from 6.157 to 14.339 MiB/s at 4 KiB (+132.9%), from 51.885 to
75.784 MiB/s at 64 KiB (+46.1%), and from 86.312 to 97.859 MiB/s at
512 KiB (+13.4%).

The write-side PostgreSQL operation count fell from 98420 to 32865 at 4 KiB,
from 6229 to 2131 at 64 KiB, and from 853 to 339 at 512 KiB. All three final
runs reported a `getxattr_fast_path_ratio` of 1.0 with zero path-resolution
fallbacks.

Persistence semantics did not change: every 128 MiB run persisted exactly
134217728 bytes in two persist operations. All primary and replica reads
decoded exactly 4096 storage blocks / 134217728 payload bytes, with
134250496 PostgreSQL result bytes, zero operation failures and replica
write protection still reported as `read_only_rejected`.

The remaining approximately one PostgreSQL operation per small write is the
actual xattr lookup triggered by Linux `security.capability` probing. A later
optimization may evaluate `FUSE_HANDLE_KILLPRIV_V2`, but only together with
complete and tested setuid/setgid/file-capability clearing semantics.
FOD 3.4.20 deliberately does not cache xattr values or weaken those semantics.

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
