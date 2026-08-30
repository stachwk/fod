# FOD storage block size selection

## Decision

The default storage block size for **newly initialized** FOD filesystems is **32 KiB (32768 bytes)**.

`fod-rust-mkfs init` still accepts `--block-size` as an explicit override. The storage block size is a filesystem-format choice persisted in the FOD database at initialization time; it is not a normal runtime tuning knob.

This decision does **not** migrate existing filesystems. An existing FOD instance keeps the `block_size` already stored in its `fod.config` table. Changing an existing filesystem from 4 KiB, 16 KiB, 32 KiB, or 64 KiB to another block size would require an explicit data-format migration/rewrite and must not be implemented as a simple config update.

## Why 32 KiB is the default

The benchmark series showed two opposing effects:

1. Larger storage blocks reduce the number of PostgreSQL rows needed for the same logical data volume. This lowers tuple/index/MVCC and per-statement overhead and strongly improves large-I/O throughput compared with the historical 4 KiB default.
2. Larger storage blocks increase read-modify-write work when an application overwrites only a small part of an existing block. At 64 KiB this becomes visible for small and medium random overwrites through lower throughput and higher WAL/page-write amplification.

32 KiB is the best general-purpose compromise between those effects.

## Final 32 KiB vs 64 KiB overwrite results

The decision workload used an existing incompressible file for random-overwrite tests, with warm-up, host I/O settling, PostgreSQL profiling, and retries for objective WAL/storage stalls. The last stall-filtered run still reported `selection_status=invalid` because one 64 KiB/4 KiB-randwrite cell reached the attempt limit with only two accepted runs and the 32 KiB/16 KiB-randwrite cell exceeded the conservative 20% spread threshold by a small amount. The direction of the clean measurements was nevertheless consistent enough for the default selection and agreed with the earlier stable large-I/O matrix.

| Workload | 32 KiB storage block | 64 KiB storage block | 64K vs 32K | Interpretation |
| --- | ---: | ---: | ---: | --- |
| 4 KiB random overwrite | 10.484 MiB/s | 10.264 MiB/s | -2.10% | Throughput is effectively tied; 32K has materially lower WAL amplification. |
| 16 KiB random overwrite | 21.511 MiB/s | 17.688 MiB/s | -17.77% | Clear 32K advantage; 64K pays a real RMW penalty. |
| 64 KiB random overwrite | 55.834 MiB/s | 59.576 MiB/s | +6.70% | 64K is better once writes naturally match the larger storage block. |
| 512 KiB sequential write | 79.404 MiB/s | 86.022 MiB/s | +8.33% | 64K is better for large sequential traffic. |

### WAL amplification

The small-overwrite tests show why throughput alone is not enough when choosing a default:

| Workload | 32 KiB WAL amplification | 64 KiB WAL amplification | Preferred |
| --- | ---: | ---: | --- |
| 4 KiB random overwrite | 6.6038 | 8.0540 | 32K |
| 16 KiB random overwrite | 2.4294 | 4.6967 | 32K |
| 64 KiB random overwrite | 2.1387 | 2.1263 | approximately equal |
| 512 KiB sequential write | 1.0782 | 0.8047 | 64K |

For 4 KiB overwrites, 64K generated about 22% more WAL per logical byte than 32K while providing no throughput benefit. For 16 KiB overwrites, 64K was about 18% slower and its WAL amplification was close to twice the 32K value.

## Earlier large-I/O candidate result

The earlier stable candidate matrix, based on three clean runs per candidate, showed the expected benefit of reducing PostgreSQL row count:

| Storage block | Median primary write |
| ---: | ---: |
| 16 KiB | 64.024 MiB/s |
| 32 KiB | 76.236 MiB/s |
| 64 KiB | 83.776 MiB/s |

64K was about 9.9% faster than 32K in that large-I/O write workload. This is a real advantage, but it is smaller than the penalty observed for 16 KiB random overwrites and does not justify making 64K the universal default.

The historical 4 KiB layout was substantially worse for PostgreSQL-backed persistence because it multiplied the number of `data_blocks` rows. For the same 64 MiB logical flush, the row counts are approximately:

| Storage block | Rows per 64 MiB |
| ---: | ---: |
| 4 KiB | 16384 |
| 16 KiB | 4096 |
| 32 KiB | 2048 |
| 64 KiB | 1024 |

The main performance problem at 4K was structural row/tuple/index/MVCC overhead, not merely FUSE request size.

## Which block size to use

### 32 KiB — recommended default

Use 32K when:

- the filesystem is general-purpose or the future workload is not known precisely;
- applications mix small/medium overwrites with larger transfers;
- 4 KiB to 16 KiB random updates matter;
- reducing WAL and RMW amplification is important;
- predictable behavior across mixed workloads matters more than maximizing one sequential benchmark.

For a normal new filesystem, omit `--block-size` and use the default:

```bash
fod-rust-mkfs init --schema-admin-password '...'
```

### 64 KiB — large-I/O optimized profile

Use an explicit 64K override when the workload is dominated by:

- large files;
- streaming or sequential writes;
- archive/object-like data;
- application writes naturally at 64 KiB or larger;
- workloads where small random in-place overwrites are rare or unimportant.

Example:

```bash
fod-rust-mkfs init --block-size 65536 --schema-admin-password '...'
```

In the measured workloads, 64K gained roughly 7% for 64 KiB random overwrite and 8% for 512 KiB sequential write, while the earlier large-I/O matrix showed roughly a 10% write advantage over 32K.

### 16 KiB — conservative specialist option

16K remains a valid explicit format choice and passed the storage-block correctness matrix. It can be considered for a workload intentionally biased toward smaller writes where minimizing RMW scope is more important than large-I/O throughput. It is not selected as the default because the broader candidate matrix showed a meaningful throughput gain from 16K to 32K, while 32K still handled small overwrites much better than 64K.

Example:

```bash
fod-rust-mkfs init --block-size 16384 --schema-admin-password '...'
```

### 4 KiB — legacy/compatibility or targeted experiments

4K also remains available explicitly, but it should not be the normal choice for a new PostgreSQL-backed FOD filesystem. It creates many more persistence rows per logical data volume and therefore pays much higher per-row PostgreSQL overhead.

Example:

```bash
fod-rust-mkfs init --block-size 4096 --schema-admin-password '...'
```

## Correctness status

Before performance selection, the storage-block correctness matrix passed for 4 KiB, 16 KiB, and 64 KiB after fixing truncate/extend zero-fill handling at partial block boundaries. The coverage included unaligned writes, boundary crossings, append, truncate shrink/extend, sparse holes, `posix_fallocate`, `copy_file_range`, concurrent partial writes, remount durability, and dedupe/CRC behavior.

32K was then exercised by the candidate/performance matrices and the random-overwrite decision workloads. The default change therefore does not introduce a new storage-block mechanism; it changes only which already-supported block size `mkfs init` selects when the user does not provide an explicit `--block-size`.

## Operational rule

- **New general-purpose filesystem:** 32 KiB default.
- **Known large-I/O / streaming filesystem:** consider explicit 64 KiB.
- **Known small-write-biased specialist filesystem:** consider explicit 16 KiB after workload-specific validation.
- **Existing filesystem:** keep its recorded block size unless a dedicated data migration/reformat procedure is intentionally performed.
