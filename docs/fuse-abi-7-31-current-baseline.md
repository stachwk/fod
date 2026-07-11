# Current FUSE ABI 7.31 Baseline

This document freezes the correctness and performance baseline that must be
preserved when FOD moves from `fuser 0.14` to `fuser 0.17`.

Measurement date: `2026-07-11`

Measured production base commit:
`7d9ed837bec69670501c78262c08723fde5d5f48`

The production runtime code matched that clean commit. The measurement
worktree additionally printed unique FUSE request counts and isolated fio test
filenames. Those test and reporting changes are part of the baseline commit,
not a storage-runtime change.

## Scope

The baseline represents:

- FOD 3.2.1 and schema version 17;
- `fuser 0.14` compiled with `abi-7-31`;
- `libfuse3.so.4` version 3.17.4 on the measurement host;
- object-owned `data_blocks`, `data_extents`, and `copy_block_crc` payloads;
- exact whole-object adoption through `copy_file_range`;
- block storage as the default and 1 MiB extents as an opt-in comparison;
- local Docker PostgreSQL only.

This phase does not enable a post-7.31 capability, modify the FUSE runtime,
change the storage format, or change the database schema.

## Environment

| Component | Measured value |
| --- | --- |
| Host | `lt7300` |
| Kernel | Linux 6.17.0-40-generic |
| CPU | Intel Core i5-8365U, 4 cores / 8 threads |
| Memory | 15 GiB |
| Rust | `rustc 1.85.1`, Cargo 1.85.1 |
| FUSE userspace | libfuse3 3.17.4 |
| FUSE crate/protocol | `fuser 0.14`, ABI 7.31 |
| PostgreSQL client | 17.10 |
| PostgreSQL server | 16.14 Alpine, local Docker |
| Logical block | 4 KiB |
| Opt-in extent target | 1 MiB |
| Build | Debug/test binaries |

QNAP was deliberately excluded. This is a same-host migration baseline, not a
production capacity claim.

## Method

Each primary workload ran three times in block mode and three times with
1 MiB extents. The matrix reset PostgreSQL statement and table statistics,
captured before/after DML and WAL snapshots, enabled `FOD_PROFILE_IO`, and
recorded maximum RSS. FUSE callback counts are unique structured request IDs,
not raw log-line counts.

Representative matrix invocation:

```bash
PROFILE_RUN_ID=fuse-abi731-exact-20260711T193000Z \
PROFILE_STORAGE_EXTENT_REPEAT=3 \
PROFILE_STORAGE_EXTENT_SIZES=1048576 \
PROFILE_STORAGE_EXTENT_WORKLOADS=test-large-copy-object-adoption \
make profile-storage-extent-size-matrix-local
```

The exact-copy timer covers only the copy operation. Source creation happens
before it and destination readback happens after it. Therefore its throughput
is useful for migration comparison on this host but is not an end-to-end file
ingest rate.

## Copy Results

All values are arithmetic means over three runs.

| Workload | Layout | Throughput MiB/s | Stdev | FUSE read/write/copy calls | Payload inserts | WAL bytes |
| --- | --- | ---: | ---: | --- | --- | ---: |
| Exact 64 MiB whole-object copy | Blocks | 8050.38 | 220.85 | 512 / 64 / 1 | 16384 blocks | 6413672 |
| Exact 64 MiB whole-object copy | 1 MiB extents | 9979.55 | 855.85 | 512 / 64 / 1 | 64 extents | 853568 |
| Chunked 64 MiB copy, 4 MiB requests | Blocks | 19.05 | 1.13 | 512 / 64 / 16 | 32768 blocks | 13084834 |
| Chunked 64 MiB copy, 4 MiB requests | 1 MiB extents | 31.66 | 2.07 | 512 / 64 / 16 | 16384 blocks + 68 extents | 7477992 |

The payload insert counts include source preparation. For each exact-copy run,
the destination reused the source data object and added no payload row. A final
database query found all six measured 64 MiB source objects with two file
references, `reference_count = 2`, and exactly one physical layout. Block
objects had 16384 block rows; extent objects had 64 extent rows.

The chunked extent case first stored the source and the first destination chunk
as extents, then safely converted the partially updated destination to blocks.
That explains its 68 extent inserts plus 16384 block inserts and is the expected
non-hybrid behavior.

## Storage and fio Results

| Workload | Layout | Result | FUSE read/write/copy calls | Payload inserts | WAL bytes |
| --- | --- | --- | --- | --- | ---: |
| Sequential 64 MiB write/readback | Blocks | 54.66 MiB/s | 512 / 64 / 0 | 16384 blocks | 7242445 |
| Sequential 64 MiB write/readback | 1 MiB extents | 113.01 MiB/s | 512 / 64 / 0 | 64 extents | 1202592 |
| fio sequential 64 MiB | Blocks | read 116053, write 4033 KiB/s | 260 / 16384 / 0 | 16384 blocks | 7025483 |
| fio sequential 64 MiB | 1 MiB extents | read 40209, write 3878 KiB/s | 260 / 16384 / 0 | 64 extents | 1158044 |
| fio mixed 64 MiB | Blocks | read 1480, write 1491 KiB/s | 26 / 24608 / 0 | 16384 blocks | 7507563 |
| fio mixed 64 MiB | 1 MiB extents | read 643, write 648 KiB/s | 26 / 24608 / 0 | 64 extents | 976979 |
| fio random mixed 64 MiB | Blocks | read 1043, write 1051 KiB/s | 8160 / 24608 / 0 | 16384 blocks | 7091326 |
| fio random mixed 64 MiB | 1 MiB extents | read 537, write 541 KiB/s | 8160 / 24608 / 0 | 64 extents | 2366792 |
| Remount durability, 64 KiB | Blocks | 1.021082 s | 1 / 1 / 0 | 16 blocks | 53209 |
| Remount durability, 64 KiB | 1 MiB extents | 1.018394 s | 1 / 1 / 0 | 1 extent | 11503 |

All remount samples preserved the expected content. Extents remain opt-in:
they reduce physical rows and WAL and improve the dedicated sequential
large-file workload, but materially regress fio sequential reads and both
mixed workloads on this host.

The first mixed-fio series reused a fixed filename. Its first run issued 24608
writes, while later runs issued 8224 and mostly reused existing payload. That
series is excluded. The accepted series uses a process-specific filename,
removes it after each case, and reports 24608 writes for every sample.

## FOD Profile Shape

Selected mean `FOD_PROFILE_IO` times are in microseconds.

| Workload | Layout | FUSE read | FUSE write | Repository fetch | Persist blocks | Persist extents | Flush |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Exact copy | Blocks | 3644484 | 1255531 | 1737092 | 1159631 | 0 | 1199237 |
| Exact copy | Extents | 2774937 | 503318 | 1279988 | 0 | 456996 | 462806 |
| Chunked copy | Blocks | 1297354 | 4454969 | 0 | 2507479 | 0 | 2600825 |
| Chunked copy | Extents | 1418936 | 2495259 | 112841 | 1736638 | 482108 | 2260241 |
| Sequential file | Blocks | 1300262 | 1118202 | 0 | 996405 | 0 | 1034644 |
| Sequential file | Extents | 2744062 | 516863 | 1253370 | 0 | 450959 | 456129 |

The required strace profile also passed in both layouts with a 64 KiB smoke.
It recorded 16 reads and 16 writes per layout. The block run made 3147 traced
system calls in 48.964 ms; the extent run made 3112 calls in 76.996 ms.

## PostgreSQL Statement Shape

`pg_stat_statements` confirms the expected distinction:

- exact block source preparation used one staging COPY and one block merge;
- exact extent source preparation used one binary COPY into `data_extents`;
- the destination adoption used two lightweight `files.data_object_id` updates
  and no destination payload COPY or merge;
- chunked block copy used 17 staging COPY calls, including source preparation;
- chunked extent copy used two extent COPY calls, 15 staging COPY calls, and 15
  server-side extent-to-block expansion statements before block merges.

The representative first-run statement totals were approximately 549 ms for
the exact block staging COPY, 577 ms for its block merge, 424 ms for exact
extent COPY, 785 ms for chunked extent staging COPY, 469 ms for its two extent
COPY calls, and 301 ms for its 15 extent-to-block expansions.

## Correctness Gates

The baseline passed these invariants after all measured runs:

- zero orphan files;
- zero orphan block, extent, and CRC rows;
- zero unreferenced data objects;
- zero `reference_count` mismatches;
- zero hybrid block/extent objects;
- exact-copy objects had two real references and stored one payload;
- partial patching of an existing three-block extent preserved unchanged
  blocks, changed only the requested block, converted to three block rows, and
  left zero extent rows;
- block and extent payloads survived unmount and remount.

The partial-patch gate is the exact test:

```bash
cargo test -p fod-rust-hotpath --test pg_query \
  switching_between_block_and_extent_storage_keeps_reads_and_cleanup_consistent \
  -- --exact --nocapture
```

## Migration Acceptance Criteria

The `fuser 0.17` migration must preserve all of the following before any new
capability is enabled:

- one native `copy_file_range` callback for exact 64 MiB copies;
- 16 callbacks for the 4 MiB chunked-copy workload;
- shared source/destination data objects and no duplicated destination payload
  for exact copies;
- correct partial extent conversion with no hybrid object;
- zero orphan payload, unreferenced object, and reference-count mismatch rows;
- successful block and extent remount durability;
- the same logical callback shapes for sequential and fio workloads;
- no material regression across repeated same-host samples.

The migration does not have to improve throughput. Investigate any new mean
below the old minimum, a changed callback count, changed SQL shape, or failed
storage invariant before accepting it. Noise-sensitive results require another
three-run series rather than an automatic rollback.

## Artifact Map

The raw artifacts are local and ignored by Git under
`artifacts/perf/7d9ed83/`. The accepted summaries are:

```text
lt7300-fuse-abi731-exact-20260711T193000Z-storage-extent-summary.md
lt7300-fuse-abi731-chunked-20260711T193500Z-storage-extent-summary.md
lt7300-fuse-abi731-sequential-20260711T194000Z-storage-extent-summary.md
lt7300-fuse-abi731-fio-sequential-20260711T194500Z-storage-extent-summary.md
lt7300-fuse-abi731-fio-mixed-isolated-20260711T200000Z-storage-extent-summary.md
lt7300-fuse-abi731-fio-random-mixed-20260711T201000Z-storage-extent-summary.md
lt7300-fuse-abi731-remount-20260711T202000Z-storage-extent-summary.md
lt7300-fuse-abi731-final-20260711T202500Z/pg_data_blocks_semantics-final.txt
lt7300-fuse-abi731-final-20260711T202500Z/whole-object-adoption-objects.txt
lt7300-fuse-abi731-strace-20260711T203000Z/fuse-test-fio-sequential-io-strace-abi731.txt
```

Do not use the earlier `lt7300-fuse-abi731-fio-mixed-20260711T195000Z`
series for comparison because its repeated runs were not isolated.
