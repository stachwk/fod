# FOD 3.4.1 primary/replica benchmark baseline

Status: recorded baseline and follow-up performance test plan.

## Recorded run

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

## Result

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

## Interpretation

### Read path

`512 KiB` is the current best read point.

From `4 KiB` to `512 KiB`:

- primary read increases from `0.997` to `36.953 MiB/s`, about `37.06x`;
- replica read increases from `1.114` to `40.310 MiB/s`, about `36.18x`.

Moving from `512 KiB` to `1 MiB` does not improve primary read and materially hurts replica read:

- primary: `36.953 -> 36.400 MiB/s` (`-1.50%`);
- replica: `40.310 -> 32.858 MiB/s` (`-18.49%`).

This supports keeping the storage block at `4 KiB` while using approximately `512 KiB` as the preferred operational read size.

At `512 KiB`, replica read is about `9.08%` faster than primary read. This must not yet be interpreted as an architectural guarantee because host cache is uncontrolled.

### Write path

The primary write path peaks at `64 KiB` with `10.242 MiB/s`.

Compared with that point:

- `512 KiB`: `9.481 MiB/s` (`-7.43%`);
- `1 MiB`: `8.150 MiB/s` (`-20.43%`).

Therefore the read and write paths currently have different optima. Do not force the write path to `512 KiB` merely because `512 KiB` is best for reads. The next write-path work should identify whether the dominant cost is dirty-block handling, persist batching, COPY/staging, PostgreSQL transaction work, WAL, or synchronization.

## Follow-up test suite

The repository helper:

```bash
scripts/perf/run_primary_replica_focus.sh
```

reuses the existing Makefile target `test-fio-primary-write-replica-read-qnap`. It does not create a parallel benchmark implementation.

Default focused test:

- `1G` file;
- block sizes `64k 512k`;
- three complete repetitions;
- AC power required;
- clean Git working tree required;
- each original `summary.tsv` retained;
- aggregate mean/median/min/max/stdev generated;
- internal FOD observability extracted from primary-write, primary-read and replica-read mount logs.

Run:

```bash
cd ~/git/fod
scripts/perf/run_primary_replica_focus.sh
```

Overrides:

```bash
FOD_PRIMARY_REPLICA_FOCUS_REPEAT=5 \
FOD_PRIMARY_REPLICA_FOCUS_FILE_SIZE=2G \
FOD_PRIMARY_REPLICA_FOCUS_BLOCK_SIZES="64k 512k" \
scripts/perf/run_primary_replica_focus.sh
```

Results are written below:

```text
artifacts/perf/<commit>/<host>-qnap-focus-<timestamp>/
```

The aggregate table is `summary.tsv`, human-readable output is `summary.md`, and `profile-run-N.txt` contains selected FOD internal observability from each phase.

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

## Later cold-cache baseline

The current test intentionally does not call `drop_caches`. A separate OS-cold series should be added only with explicit remote-host cache-control hooks, because dropping cache on the laptop does not make a remote QNAP PostgreSQL server cold.

That future series must record cache handling separately for primary and replica and must remain distinct from this baseline.
