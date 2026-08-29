# FOD 3.4.1 primary/replica benchmark baseline

Status: recorded QNAP baseline, local focused baseline and follow-up performance test plan.

## Recorded QNAP run

- Runtime: FOD `3.4.1`.
- Commit: `1ccd2435fab1458ec27bf9054a12f623b1479e12`.
- Client host: `lt7300`.
- Backend: isolated QNAP primary/replica benchmark.
- File size: `1G`.
- Matrix artifact: `artifacts/perf/1ccd243/lt7300-qnap-matrix-20260828T234017Z`.
- Block sizes: `4k`, `16k`, `64k`, `256k`, `512k`, `1m`.
- Existing benchmark sequence: primary write, WAL replay, primary restart and fresh primary read, WAL replay, primary stop, replica restart and fresh replica read, replica write-rejection check.
- FOD read cache/read-ahead/prefetch are disabled by the benchmark and the FUSE mount uses `direct_io`.
- The host kernel page cache is **not** dropped. These numbers are therefore a reproducible process/remount baseline with uncontrolled host page cache, not a cold-storage benchmark.

## QNAP result

| block size | primary write MiB/s | primary write IOPS | primary read MiB/s | primary read IOPS | replica read MiB/s | replica read IOPS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `4k` | 6.501 | 1664.142 | 0.997 | 255.177 | 1.114 | 285.209 |
| `16k` | 6.937 | 443.990 | 3.757 | 240.466 | 4.054 | 259.473 |
| `64k` | **10.242** | 163.865 | 12.745 | 203.918 | 13.844 | 221.507 |
| `256k` | 8.713 | 34.851 | 24.836 | 99.343 | 27.419 | 109.674 |
| `512k` | 9.481 | 18.963 | **36.953** | 73.906 | **40.310** | 80.620 |
| `1m` | 8.150 | 8.150 | 36.400 | 36.400 | 32.858 | 32.858 |

Replica correctness stayed clean for every cell:

```text
replica_operation_failures=0
replica_write_guard=read_only_rejected
```

## QNAP interpretation

### Read path

`512 KiB` is the current best QNAP read point.

From `4 KiB` to `512 KiB`:

- primary read increases from `0.997` to `36.953 MiB/s`, about `37.06x`;
- replica read increases from `1.114` to `40.310 MiB/s`, about `36.18x`.

Moving from `512 KiB` to `1 MiB` does not improve primary read and materially hurts replica read:

- primary: `36.953 -> 36.400 MiB/s` (`-1.50%`);
- replica: `40.310 -> 32.858 MiB/s` (`-18.49%`).

This supports keeping the storage block at `4 KiB` while using approximately `512 KiB` as the preferred operational read size.

At `512 KiB`, replica read is about `9.08%` faster than primary read. This must not yet be interpreted as an architectural guarantee because host cache is uncontrolled.

### Write path

The QNAP primary write path peaks at `64 KiB` with `10.242 MiB/s`.

Compared with that point:

- `512 KiB`: `9.481 MiB/s` (`-7.43%`);
- `1 MiB`: `8.150 MiB/s` (`-20.43%`).

The QNAP result alone therefore suggested different read and write optima. The local focused baseline below refines that interpretation: on the local backend the write difference between `64 KiB` and `512 KiB` disappears, so the QNAP `64 KiB` advantage should be treated as backend-dependent until profiling isolates the cause.

## Recorded local focused run

- Runtime: FOD `3.4.1`.
- Commit: `9ec5ae310767f36037ec7301053788f6158b4036`.
- Client host: `lt7300`.
- Backend: local Docker primary/replica matrix (`QNAP=0`).
- File size: `1G`.
- Repetitions: `3` complete matrix runs per block size.
- Block sizes: `64k`, `512k`.
- Focus artifact: `artifacts/perf/9ec5ae3/lt7300-local-focus-20260829T102324Z`.
- AC power was required by the runner.
- The same existing primary/replica matrix implementation was reused; only backend selection and repeated aggregation differ.

## Local focused result

| block size | runs | primary write MiB/s mean ± stdev | primary read MiB/s mean ± stdev | replica read MiB/s mean ± stdev | replica vs primary read |
| --- | ---: | ---: | ---: | ---: | ---: |
| `64k` | 3 | `64.025 ± 0.580` | `202.566 ± 16.363` | `207.283 ± 9.134` | `+2.33%` |
| `512k` | 3 | `64.542 ± 0.653` | `395.730 ± 8.077` | `415.975 ± 5.753` | `+5.12%` |

Medians were:

| block size | primary write MiB/s median | primary read MiB/s median | replica read MiB/s median |
| --- | ---: | ---: | ---: |
| `64k` | 63.880 | 207.371 | 211.178 |
| `512k` | 64.419 | 393.846 | 412.737 |

## Local focused interpretation

### Read path

Moving from `64 KiB` to `512 KiB` gives a clear and repeatable improvement:

- primary read: `202.566 -> 395.730 MiB/s`, `+95.36%`;
- replica read: `207.283 -> 415.975 MiB/s`, `+100.68%`.

The `512 KiB` read results are also more stable across the three runs:

- primary read coefficient of variation: about `2.04%` at `512 KiB` versus `8.08%` at `64 KiB`;
- replica read coefficient of variation: about `1.38%` at `512 KiB` versus `4.41%` at `64 KiB`.

This independently reinforces `512 KiB` as the preferred operational read size while keeping the storage block at `4 KiB`.

### Write path

Local primary write throughput is effectively flat between the two focused block sizes:

- `64 KiB`: `64.025 ± 0.580 MiB/s`;
- `512 KiB`: `64.542 ± 0.653 MiB/s`.

The mean difference is only about `+0.81%` in favor of `512 KiB`, which is of the same order as run-to-run variation. Therefore there is no evidence from the local focused run that `64 KiB` is intrinsically better for the FOD write path.

This changes the working interpretation of the QNAP result: the QNAP write optimum at `64 KiB` is likely influenced by the remote PostgreSQL/network/storage path rather than being a universal FOD write-size optimum. CPU and internal FOD profiling should compare `64 KiB` and `512 KiB` write paths before changing write batching policy.

### Local versus QNAP

The local Docker backend is intentionally not treated as a performance-equivalent replacement for QNAP. The large gap is diagnostic rather than a direct regression comparison.

For the two focused points, local throughput is approximately:

- `64 KiB`: `6.25x` QNAP primary write, `15.89x` QNAP primary read, `14.97x` QNAP replica read;
- `512 KiB`: `6.81x` QNAP primary write, `10.71x` QNAP primary read, `10.32x` QNAP replica read.

This strongly indicates that the remote/backend path is a dominant part of the QNAP measurements. The next optimization work should therefore distinguish FOD CPU/request overhead from PostgreSQL, network, WAL and storage costs instead of treating the QNAP numbers as pure FOD limits.

## Follow-up test suite

The repository helper:

```bash
scripts/perf/run_primary_replica_focus.sh
```

reuses the existing Makefile primary/replica matrix targets. It does not create a parallel benchmark implementation.

Backend selection follows the repository convention:

- `QNAP=0` (default) selects `test-fio-primary-write-replica-read-matrix` and local Docker;
- `QNAP=1` selects `test-fio-primary-write-replica-read-qnap` and the QNAP Docker endpoint;
- `FOD_PRIMARY_REPLICA_FOCUS_BACKEND=local|qnap` explicitly overrides the automatic selection.

The runner prints both `backend=` and `make_target=` before any benchmark work starts.

A no-Docker backend-selection precheck is available:

```bash
QNAP=0 FOD_PRIMARY_REPLICA_FOCUS_PRECHECK_ONLY=1 \
    scripts/perf/run_primary_replica_focus.sh

QNAP=1 FOD_PRIMARY_REPLICA_FOCUS_PRECHECK_ONLY=1 \
    scripts/perf/run_primary_replica_focus.sh
```

Regression test:

```bash
bash tests/test_primary_replica_focus_backend.sh
```

Default focused test:

- local Docker backend because repository default is `QNAP=0`;
- `1G` file;
- block sizes `64k 512k`;
- three complete repetitions;
- AC power required;
- clean Git working tree required;
- each original `summary.tsv` retained;
- aggregate mean/median/min/max/stdev generated;
- internal FOD observability extracted from primary-write, primary-read and replica-read mount logs.

Local run:

```bash
cd ~/git/fod
QNAP=0 scripts/perf/run_primary_replica_focus.sh
```

QNAP run:

```bash
cd ~/git/fod
QNAP=1 scripts/perf/run_primary_replica_focus.sh
```

Explicit backend selection can be used when desired:

```bash
FOD_PRIMARY_REPLICA_FOCUS_BACKEND=local \
scripts/perf/run_primary_replica_focus.sh
```

Overrides:

```bash
QNAP=0 \
FOD_PRIMARY_REPLICA_FOCUS_REPEAT=5 \
FOD_PRIMARY_REPLICA_FOCUS_FILE_SIZE=2G \
FOD_PRIMARY_REPLICA_FOCUS_BLOCK_SIZES="64k 512k" \
scripts/perf/run_primary_replica_focus.sh
```

Results are written below:

```text
artifacts/perf/<commit>/<host>-<backend>-focus-<timestamp>/
```

The aggregate table is `summary.tsv`, human-readable output is `summary.md`, and `profile-run-N.txt` contains selected FOD internal observability from each phase.

## QNAP availability failure

The QNAP target requires the configured remote Docker daemon to be reachable. An error such as:

```text
dial tcp 192.168.1.11:2376: connect: no route to host
```

means the QNAP Docker endpoint is not network-reachable from the laptop at that moment. It is not a FOD benchmark result and should not be mixed with performance data. Use `QNAP=0` for the local matrix or restore QNAP network reachability before a remote run.

## CPU profiling helper

For CPU-level profiling, build the `profiling` Cargo profile and run FOD from that profile. The profiling profile matches the production optimization shape (`thin` LTO, one codegen unit, `panic=abort`) but retains debug information and does not strip symbols.

Record a running `fod-rust-fuse` process for a fixed duration:

```bash
PID="$(pgrep -n -x fod-rust-fuse)"
scripts/perf/perf_record_fod.sh "$PID" primary-read-512k 60
```

Use a fixed interval rather than manually interrupting `perf record`; this avoids leaving an incomplete `perf.data` file.

Recommended CPU profiles after the focused matrix:

1. primary read at `512 KiB`;
2. replica read at `512 KiB`;
3. primary write at `64 KiB`;
4. primary write at `512 KiB`.

Before CPU profiling, inspect the already generated per-run internal summaries:

```text
profile-run-1.txt
profile-run-2.txt
profile-run-3.txt
```

Those files should be used to decide which FOD boundary/persist stages need CPU-level profiling first.

## Later cold-cache baseline

The current test intentionally does not call `drop_caches`. A separate OS-cold series should be added only with explicit remote-host cache-control hooks, because dropping cache on the laptop does not make a remote QNAP PostgreSQL server cold.

That future series must record cache handling separately for primary and replica and must remain distinct from this baseline.
