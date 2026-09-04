# fuser 0.17 Migration Baseline

This report compares the retained `fuser 0.17.0` migration with the frozen
`fuser 0.14` / FUSE ABI 7.31 baseline. It measures functional parity and
storage invariants; it does not claim that a dependency upgrade must improve
throughput.

Measurement date: `2026-07-12`

Measured commit:
`522b1b51e4ddd1d2deffe7e32084ce3ffb6f3547`

Comparison commit:
`7d9ed837bec69670501c78262c08723fde5d5f48`

## Environment

Both series used the same `lt7300` host, Linux 6.17.0-40-generic, libfuse3
3.17.4, Rust 1.85.1, local Docker PostgreSQL 16.14, schema version 17, debug
and test binaries, 4 KiB logical blocks, and opt-in 1 MiB extents. The new
runtime used `fuser 0.17.0`, compiled protocol maximum 7.40, and negotiated
protocol 7.40 with a kernel advertising 7.44.

Each primary row is the arithmetic mean of three independent runs. Block and
extent modes were measured separately with PostgreSQL DML, WAL, statement,
memory, and `FOD_PROFILE_IO` captures. The first exact-copy and sequential
series showed short-lived noise and were repeated with the same three-run
method before making the migration decision.

## Copy And Sequential Results

The accepted exact-copy and sequential rows use their repeated series. The
initial series remain in the artifact map and are not discarded.

| Workload | Layout | ABI 7.31 | fuser 0.17 | Change | FUSE read/write/copy calls |
| --- | --- | ---: | ---: | ---: | --- |
| Exact 64 MiB whole-object copy | Blocks | 8050.38 MiB/s | 8012.34 MiB/s | -0.5% | 512 / 64 / 1 |
| Exact 64 MiB whole-object copy | 1 MiB extents | 9979.55 MiB/s | 10234.49 MiB/s | +2.6% | 512 / 64 / 1 |
| Chunked 64 MiB copy, 4 MiB requests | Blocks | 19.05 MiB/s | 18.44 MiB/s | -3.2% | 512 / 64 / 16 |
| Chunked 64 MiB copy, 4 MiB requests | 1 MiB extents | 31.66 MiB/s | 32.13 MiB/s | +1.5% | 512 / 64 / 16 |
| Sequential 64 MiB write/readback | Blocks | 54.66 MiB/s | 58.18 MiB/s | +6.4% | 512 / 64 / 0 |
| Sequential 64 MiB write/readback | 1 MiB extents | 113.01 MiB/s | 123.61 MiB/s | +9.4% | 512 / 64 / 0 |

The first exact block series averaged 7161.12 MiB/s, but each timed copy lasted
only 6-10 ms. The repeated series averaged 8012.34 MiB/s and restored the old
baseline range. The first sequential block series contained one 42.82 MiB/s
outlier; its repeat was stable at 57.01-59.84 MiB/s. These repeats do not show
a persistent migration regression.

Exact destinations added no payload rows. Across both exact series, all 12
measured 64 MiB source objects had two file references, `reference_count = 2`,
and one physical layout. Block objects had 16384 rows; extent objects had 64
rows. Chunked copies retained 16 native `copy_file_range` callbacks and the
expected `32768` block inserts or `16384` block plus `68` extent inserts,
including source preparation and safe destination conversion.

## fio Results

| Workload | Layout | ABI 7.31 read/write KiB/s | fuser 0.17 read/write KiB/s | Calls |
| --- | --- | ---: | ---: | --- |
| Sequential 64 MiB | Blocks | 116053 / 4033 | 122880 / 4086 | 260 read / 16384 write |
| Sequential 64 MiB | 1 MiB extents | 40209 / 3878 | 47514 / 4425 | 260 read / 16384 write |
| Mixed 64 MiB | Blocks | 1480 / 1491 | 1565 / 1577 | 26 read / 24608 write |
| Mixed 64 MiB | 1 MiB extents | 643 / 648 | 765 / 771 | 26 read / 24608 write |
| Random mixed 64 MiB | Blocks | 1043 / 1051 | 1070 / 1079 | 8160 read / 24608 write |
| Random mixed 64 MiB | 1 MiB extents | 537 / 541 | 625 / 630 | 8160 read / 24608 write |

All fio means stayed at or above the old baseline while preserving callback
counts. Extents remain opt-in: their relative mixed/random behavior is still
materially slower than block storage even though this migration series was
faster than the earlier extent samples.

## Durability, SQL, And WAL

Block remount durability averaged 1.020232 s versus 1.021082 s before the
migration. The extent result was 1.019894 s versus 1.018394 s. All six samples
survived unmount and remount with one write and one read callback.

The storage SQL and DML shapes remained unchanged:

- exact block source: 16384 block inserts and no destination payload copy;
- exact extent source: 64 extent inserts and no destination payload copy;
- chunked block copy: 32768 block inserts;
- chunked extent copy: 16384 block inserts and 68 extent inserts;
- sequential and fio block runs: 16384 block inserts;
- sequential and fio extent runs: 64 extent inserts.

Mean WAL stayed in the same order of magnitude and preserved the block/extent
relationship. Some fio samples include higher full-page-image or background
checkpoint noise, so their WAL means are retained as observations rather than
treated as a protocol regression. No new SQL statement or payload mutation
shape appeared after the fuser upgrade.

## Profile And Correctness Gates

The required 64 KiB strace profile passed in both layouts with 16 reads and 16
writes. Total traced calls changed from 3147 to 3253 for blocks (+3.4%) and
from 3112 to 3193 for extents (+2.6%). The one-run traced wall accounting was
0.082273 s and 0.082937 s, respectively. This small traced system-call delta
did not produce a regression in the repeated 64 MiB workloads.

The partial-patch regression converted an existing extent object to blocks,
preserved unchanged bytes, and left no hybrid object. The final schema-v17
diagnostic reported:

- zero orphan files, blocks, extents, and CRC rows;
- zero unreferenced data objects;
- zero `reference_count` mismatches;
- zero hybrid block/extent objects;
- 12 exact-copy objects with two real references and one physical payload.

## Decision

Retain `fuser 0.17.0`, explicit libfuse3, and protocol maximum 7.40. The
migration preserves correctness, copy semantics, locking gates, storage
ownership, durability, SQL shape, and workload callback counts without a
material repeated performance regression. Do not enable post-7.31
capabilities as part of this decision; evaluate them independently.

## Artifact Map

Raw artifacts are local and ignored by Git under `artifacts/perf/522b1b5/`:

```text
lt7300-fuser017-exact-20260712T074000Z-storage-extent-summary.md
lt7300-fuser017-exact-repeat-20260712T080000Z-storage-extent-summary.md
lt7300-fuser017-chunked-20260712T074200Z-storage-extent-summary.md
lt7300-fuser017-sequential-20260712T074300Z-storage-extent-summary.md
lt7300-fuser017-sequential-repeat-20260712T080100Z-storage-extent-summary.md
lt7300-fuser017-fio-sequential-20260712T074400Z-storage-extent-summary.md
lt7300-fuser017-fio-mixed-20260712T074600Z-storage-extent-summary.md
lt7300-fuser017-fio-random-mixed-20260712T075100Z-storage-extent-summary.md
lt7300-fuser017-remount-20260712T075800Z-storage-extent-summary.md
lt7300-fuser017-strace-20260712T080200Z/fuse-test-fio-sequential-io-strace-fuser017.txt
lt7300-fuser017-final-20260712T080300Z/pg_data_blocks_semantics-final.txt
lt7300-fuser017-final-20260712T080300Z/whole-object-adoption-objects.txt
```
