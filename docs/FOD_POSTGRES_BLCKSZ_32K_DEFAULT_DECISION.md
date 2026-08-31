# FOD PostgreSQL BLCKSZ=32K default decision

Status: **accepted / default / target**

Date: 2026-08-31

## Decision

For FOD deployments, PostgreSQL compiled with `BLCKSZ=32K` is the default and target PostgreSQL variant.

The canonical PostgreSQL 16 image is:

```text
ghcr.io/stachwk/postgres-16-fod-32k:16
```

The standard PostgreSQL `BLCKSZ=8K` variant remains supported only as a compatibility/reference and regression-comparison variant. It is no longer the FOD default.

The FOD logical filesystem block default remains 32 KiB. The target pairing is therefore:

```text
FOD logical block = 32 KiB
PostgreSQL BLCKSZ = 32 KiB
```

## Evidence

The decision is based on repeated PostgreSQL 8K vs 32K comparisons with FOD fixed at a 32 KiB logical block, including durable SSD runs and RAM-backed PostgreSQL PGDATA isolation runs.

### Stable structural result: INSERT WAL

The strongest repeatable signal is lower WAL generation for the main FOD `INSERT ... ON CONFLICT` persistence query:

- 512 KiB fio workload on tmpfs: PG32 generated **4.48% less INSERT WAL** than PG8;
- 64 KiB fio workload on tmpfs: PG32 generated **4.51% less INSERT WAL** than PG8.

The near-identical result across different I/O sizes is treated as a structural PostgreSQL page/layout effect rather than storage noise.

### 512 KiB tmpfs isolation

Medians from the RAM-backed PGDATA experiment:

| Metric | PG8 | PG32 | PG32 vs PG8 |
| --- | ---: | ---: | ---: |
| primary write | 67.842 MiB/s | 89.245 MiB/s | +31.55%* |
| primary read | 242.539 MiB/s | 253.465 MiB/s | +4.50% |
| replica read | 259.503 MiB/s | 246.866 MiB/s | -4.87% |
| COPY mean | 395.127 ms | 293.305 ms | -25.77% |
| INSERT mean | 482.837 ms | 351.504 ms | -27.20% |
| INSERT WAL | 505,979,756 B | 483,335,172 B | -4.48% |

`*` The write-throughput percentage is not treated as an exact production speedup because PG8 showed large run-to-run spread in that experiment. The SQL execution and INSERT WAL signals are more reliable.

### 64 KiB tmpfs isolation

Medians:

| Metric | PG8 | PG32 | PG32 vs PG8 |
| --- | ---: | ---: | ---: |
| primary write | 71.749 MiB/s | 82.875 MiB/s | +15.51%* |
| primary read | 156.767 MiB/s | 166.884 MiB/s | +6.45% |
| replica read | 165.803 MiB/s | 168.421 MiB/s | +1.58% |
| COPY mean | 298.324 ms | 283.664 ms | -4.91% |
| INSERT mean | 496.776 ms | 394.816 ms | -20.52% |
| INSERT WAL | 216,833,668 B | 207,052,916 B | -4.51% |

`*` One PG32 run was a clear transient outlier (`27.150 MiB/s`, with COPY/INSERT also abnormally slow). The other PG32 write runs were `84.544` and `82.875 MiB/s`. The median remains useful, but the decision does not depend on that one throughput percentage.

## SSD isolation conclusion

Earlier sustained durable tests on the host SSD showed severe WALSync/storage degradation and large cross-run drift. Moving PostgreSQL PGDATA to `/dev/shm` reduced WALSync to approximately hundredths of a millisecond while the PG32 advantages in SQL execution and INSERT WAL remained.

Therefore:

1. the multi-second WALSync stalls observed in durable tests were storage-related;
2. the lower INSERT WAL and faster FOD persistence SQL with PG32 are not explained solely by the SSD;
3. PostgreSQL `BLCKSZ=32K` has an intrinsic advantage for the measured FOD persistence workload.

Sampled `wal_bytes_delta` from the short profiler windows is **not** used for this decision, because some samples missed the start of the workload and could report total sampled WAL smaller than the WAL attributed directly to the measured INSERT. `pg_stat_statements.wal_bytes` for the target INSERT is the authoritative WAL comparison used here.

## Default deployment policy

`docker-compose.yml` must default to:

```text
ghcr.io/stachwk/postgres-16-fod-32k:16
```

and verify at health-check time that:

```sql
SHOW block_size;
```

returns `32768` unless the deployment explicitly overrides both the PostgreSQL image and expected block size.

The generic published PostgreSQL image must not automatically create benchmark replication roles or install benchmark-specific replication startup behavior. Primary/replica benchmark helpers belong in the benchmark compose files and are mounted there explicitly.

## Migration rule

PostgreSQL data directories are not compatible across different compile-time `BLCKSZ` values.

An existing 8 KiB PGDATA must **not** be started with the 32 KiB PostgreSQL binary. Migration from PG8 to PG32 requires a fresh 32K cluster and logical migration, for example dump/restore or another supported logical transfer path.

Do not reuse an existing 8K Docker volume as a 32K cluster.

## 8K status

The 8K image remains available for:

- regression comparisons;
- compatibility testing;
- controlled migration validation;
- reproducing behavior of standard PostgreSQL builds.

It is not the default or target configuration for new FOD deployments.
