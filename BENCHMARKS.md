# FOD Benchmarks

This file records the current comparison baselines for the main performance-sensitive paths.
Current runtime note: FOD (Filesystem On DataBaseEngine) is Rust-backed end to end. Any Python references below are historical migration baselines, not active runtime fallback paths.

## Current Status

- The benchmark suite is now tied to documented runtime profiles and CI-visible regression targets.
- Throughput, finalization, read-cache, and atime numbers are treated as baselines, not fixed promises.
- `make test-throughput` and `make test-flush-release-profile` are the current write-path and finalization entry points.
- Additional write-oriented baselines now cover large `copy_file_range()` transfers, large multi-block file writes, and remount durability checks.
- The mounted fio smoke suite now compares the current block-storage path against the opt-in extent preset across sequential, mixed sequential rw, and random mixed rw workloads.
- The matrix now also includes `FIO_BLOCK_SIZE=64k` comparisons for the same sequential and mixed workloads where fio accepts the file size.
- Latest mounted fio smoke runs for this repo were collected on FOD 3.0.2 against the local Docker/PostgreSQL setup on 2026-05-06; the historical throughput snapshot below remains on FOD 2.4.7.
- The current `bulk_write` vs `metadata_heavy` large-copy comparison is the baseline for profile selection; the Rust hot path now lives in `rust_hotpath/` and is built into `libfod-2.so`, covering the copy planner, changed-run packing, persist padding, read assembly, logical resize planning for `truncate()`/`fallocate()`, startup config queries, and the first repository lookups/mutations while the historical Python-era baseline remains available for reference.
- Rust hot-path dedupe remains opt-in and off by default because it can be slower than the historical Python-era baseline on repeated-copy workloads.
- The current runtime orchestration, cache invalidation, storage, and journal handling live in Rust.
- `test-tree-scale` now seeds a unique root per run and cleans it up afterward, so profile comparisons can be rerun on the same seed without duplicate-key conflicts.
- When a tuning change matters, the repository should record the before/after numbers here and in `TODO.md`.
- FOD assumes transactional PostgreSQL connections with `autocommit` disabled; the practical operating floor is PostgreSQL 9.5+, `read committed`, and `max_connections` above `pool_max_connections + 2`.
- The next write-path comparison should separate `write` without `fsync`, `write` with `fsync`, and a larger `THROUGHPUT_BLOCK_SIZE` batch so the dominant bottleneck becomes explicit.
- `persist_buffer_chunk_blocks` is now a separate runtime knob for flush batching; larger batches can reduce SQL round-trips on dirty-write finalization.
- `persist_block_transport` is now a separate runtime knob for write-path transport comparison; use it to compare `copy_binary_staging`, `binary_bytea`, and `legacy_hex` on the same workload.
- `synchronous_commit` is now a separate runtime knob; the latest local comparison was mixed across block sizes, so it is exposed for tuning rather than forced as the default.
- PostgreSQL session normalization to UTC is now initialized once per physical pooled connection; the measured steady-state overhead is effectively the pool acquire/release plus a cheap `rollback()`.

### Persist Block Transport Comparison

Observed on the current throughput target with the same short smoke workload and the default local FOD profile:

- Profile: default local config profile (`bulk_write`)
- Mount mode: writable primary
- FOD version: `FOD 2.01.118`
- PostgreSQL mode: local Docker/PostgreSQL
- sync/fsync: no fsync in this block
- date: not recorded

- `copy_binary_staging`, `bs=4k`, `count=100`
  - `409600 bytes in 0.201s (1.95 MiB/s)`
- `copy_binary_staging`, `bs=1M`, `count=100`
  - `104857600 bytes in 20.845s (4.80 MiB/s)`
- `binary_bytea`, `bs=4k`, `count=100`
  - `409600 bytes in 0.134s (2.92 MiB/s)`
- `binary_bytea`, `bs=1M`, `count=100`
  - `104857600 bytes in 20.531s (4.87 MiB/s)`
- `legacy_hex`, `bs=4k`, `count=100`
  - `409600 bytes in 0.112s (3.50 MiB/s)`
- `legacy_hex`, `bs=1M`, `count=100`
  - `104857600 bytes in 20.604s (4.85 MiB/s)`

On this host and smoke workload, the three transports are close enough that the small-run benchmark does not show a decisive winner. `legacy_hex` is still competitive on the tiny `4k` burst, while the staged `COPY BINARY` and `binary_bytea` paths stay in the same band on `1M`. Treat this as a comparison baseline, not a long-run conclusion.

## FOD 2.01.116 Extent PoC Baseline Plan

The extent direction is still a planning baseline, not a shipping storage format. The first PoC should stay intentionally narrow.

- logical block size = 4 KiB
- persist model = extents
- extent classes = 4 KiB .. 4 MiB
- first PoC scope = sequential write/read only
- defer merge/split logic until the sequential path proves the model

## FOD 3.0.2 Extent PoC Planner Benchmark

Observed on the current host with the Rust-only hot-path benchmark harness, `enable_extents = true`, and the sequential-only PoC planner. This benchmark does not touch the mounted PostgreSQL storage path yet; it measures the planner gate and contiguous extent coalescing itself.

- `make test-rust-hotpath-extent-poc-benchmark`
  - `4 KiB`
    - `bytes=4096`
    - `blocks=1`
    - `iterations=10000`
    - `elapsed_s=0.003135`
    - `per_op_ns=313.50`
  - `64 KiB`
    - `bytes=65536`
    - `blocks=16`
    - `iterations=10000`
    - `elapsed_s=0.007728`
    - `per_op_ns=772.76`
  - `1 MiB`
    - `bytes=1048576`
    - `blocks=256`
    - `iterations=10000`
    - `elapsed_s=0.067972`
    - `per_op_ns=6797.25`
  - `4 MiB`
    - `bytes=4194304`
    - `blocks=1024`
    - `iterations=10000`
    - `elapsed_s=0.231783`
    - `per_op_ns=23178.32`

The planner stays cheap even at 4 MiB inputs, which is the expected shape for a narrow opt-in PoC gate. The mounted fio smoke suite now covers the actual extent-backed storage path; the next useful benchmark should go beyond sequential smoke and compare larger random mixed workloads or longer sustained runs.

## FOD 3.0.2 Fio Sequential IO Extents Smoke

Observed on the current host with the mounted PostgreSQL-backed runtime, `tests/integration/test_fio_sequential_io.sh`, `FIO_BLOCK_SIZE=4k`, and file sizes from `4 KiB` through `4 MiB`. The test covers both the current block-storage path and the opt-in extent preset.

| File size | Block write | Extent write | Block read | Extent read |
| --- | --- | --- | --- | --- |
| `4 KiB` | `0.78 MiB/s` | `0.78 MiB/s` | `0.78 MiB/s` | `0.43 MiB/s` |
| `64 KiB` | `3.29 MiB/s` | `2.84 MiB/s` | `4.17 MiB/s` | `4.81 MiB/s` |
| `1 MiB` | `3.64 MiB/s` | `4.13 MiB/s` | `27.0 MiB/s` | `15.2 MiB/s` |
| `4 MiB` | `4.73 MiB/s` | `4.77 MiB/s` | `32.0 MiB/s` | `19.8 MiB/s` |

Notes from the current host:

- The `4 KiB` smoke is dominated by fixed per-operation overhead and is useful mainly as a correctness guard.
- Extents do not show a consistent write win at `64 KiB`, but they do pull ahead on the larger `1 MiB` and `4 MiB` sequential writes.
- Read results are mixed and not yet a clear extent win; the current PoC should stay opt-in and workload-specific.
- The new extents profile is still a benchmarked preset, not the default storage path.

## FOD 3.0.4 Fio Sequential IO Direct I/O Smoke

Observed on the current host with the mounted PostgreSQL-backed runtime, `tests/integration/test_fio_sequential_io.sh`, `FIO_BLOCK_SIZE=4k`, `FIO_FILE_SIZE=4M`, and a direct-I/O mount toggle. The test covers both the current block-storage path and the opt-in extent preset.

| Mode | Block write | Block read | Extent write | Extent read |
| --- | --- | --- | --- | --- |
| normal | `1768 KiB/s` | `14.8 MiB/s` | `244 KiB/s` | `9.3 MiB/s` |
| `fopen_direct_io=1` | `1746 KiB/s` | `20.2 MiB/s` | `261 KiB/s` | `9.37 MiB/s` |

Notes from the current host:

- `fopen_direct_io=1` is no longer a dramatic regression on the block path, and extent reads are now close to the normal run after the binary-result fetch landed. It should still stay a diagnostic or compatibility toggle rather than a performance default.
- The extent path was initially hit much harder than the block path under direct I/O, which made direct I/O a poor benchmark for judging the extent PoC itself. Fetching extent payloads in binary format and then keeping extent slices zero-copy removed the biggest read-side penalty and pushed the extent read path to about `9.37 MiB/s` under direct I/O and about `9.3 MiB/s` in the normal run.
- After the direct-I/O cache / read-slice cleanup pass, exact-size read assembly, pre-reserved extent payloads, and shared cached read blocks improved the observed direct-I/O smoke on both read and write; the latest single-block cache-hit fast path plus the atime-touch throttle pushed block write around `2.3 MiB/s`, block read around `2.1 MiB/s`, and extent read around `141 KiB/s`, but the mode still stayed far behind the normal path and extent writes remained the biggest regression.
- After the single-extent direct path and small-extent staging fast path, the direct-I/O smoke remained roughly in the same band: block write around `2.1 MiB/s`, block read around `3.2 MiB/s`, extent write around `260 KiB/s`, and extent read around `1.0 MiB/s`. That means the extra staging/fast-path work did not move the write-side bottleneck enough to matter on this workload.
- After the shared repo-fetch Arc path landed and the extra recent-write retention tweak was reverted, the direct-I/O smoke settled closer to block write `932 KiB/s`, block read `1.6 MiB/s`, extent write `14.4 KiB/s`, and extent read `125 KiB/s`. The shared read-block path was the useful win to keep; the wider recent-write retention did not produce a stable benefit worth keeping.
- If we want to compare storage engines fairly, we should keep `fopen_direct_io=0` for the baseline and treat direct I/O as a separate compatibility smoke.

## FOD 3.0.4 Fio Sequential IO Direct I/O Strace Smoke

Observed on the current host with `make test-fio-sequential-io-strace`, `FIO_BLOCK_SIZE=4k`, `FIO_FILE_SIZE=1M`, `FOD_FOPEN_DIRECT_IO=1`, and `strace -f -c` wrapped around the mounted FOD process. This is a syscall-level hotspot table, not a replacement for the normal throughput benchmarks.

### Block Mode

| % time | seconds | usecs/call | calls | errors | syscall |
| --- | --- | --- | --- | --- | --- |
| 31.25 | 0.147014 | 49004 | 3 |  | wait4 |
| 30.19 | 0.142015 | 43 | 3258 | 2 | futex |
| 28.66 | 0.134796 | 134796 | 1 |  | restart_syscall |
| 3.85 | 0.018133 | 5 | 3574 |  | sendto |
| 1.98 | 0.009330 | 2 | 3690 |  | poll |
| 1.31 | 0.006182 | 1 | 3685 |  | recvfrom |
| 0.59 | 0.002769 | 2 | 1041 |  | write |
| 0.48 | 0.002272 | 3 | 612 | 1 | read |
| 0.37 | 0.001744 | 3 | 527 |  | writev |
| 0.23 | 0.001071 | 7 | 149 |  | mmap |
| 0.22 | 0.001054 | 263 | 4 |  | execve |
| 0.21 | 0.000977 | 0 | 1040 |  | getgroups |
| 100.00 | 0.470407 | 24 | 19100 | 20 | total |

### Extent Mode

| % time | seconds | usecs/call | calls | errors | syscall |
| --- | --- | --- | --- | --- | --- |
| 30.07 | 0.524144 | 174714 | 3 |  | wait4 |
| 29.46 | 0.513423 | 157 | 3253 | 2 | futex |
| 29.40 | 0.512341 | 512341 | 1 |  | restart_syscall |
| 5.90 | 0.102877 | 6 | 15564 | 699 | recvfrom |
| 1.61 | 0.028049 | 29 | 936 |  | brk |
| 1.53 | 0.026662 | 8 | 3294 |  | sendto |
| 1.10 | 0.019220 | 5 | 3719 |  | poll |
| 0.21 | 0.003676 | 5 | 714 |  | write |
| 0.18 | 0.003215 | 6 | 526 |  | writev |
| 0.15 | 0.002535 | 4 | 615 | 1 | read |
| 0.08 | 0.001406 | 1 | 1038 |  | getgroups |
| 0.06 | 0.000969 | 6 | 153 |  | mmap |
| 100.00 | 1.742833 | 55 | 31329 | 725 | total |

Notes from the strace smoke:

- The block mode is dominated by synchronization and process management syscalls (`wait4`, `futex`, `restart_syscall`) plus PostgreSQL traffic (`sendto`, `poll`, `recvfrom`).
- The extent mode shows noticeably more `recvfrom` pressure, which is consistent with the extent path still spending a lot of time in PostgreSQL round-trips rather than in raw slice assembly.
- This strace table complements the internal FUSE↔DB timers; it does not replace them, but it helps confirm that the remaining work is deeper in synchronization and DB execution than in buffer copying.
- The direct COPY write path for extent CRC rows trimmed a bit of syscall pressure on the same 1 MiB smoke, but the extent path is still dominated by PostgreSQL traffic. The improvement is real, just modest.
- A repeat run after removing the single-extent direct CRC fast path kept the same overall shape: block mode stayed at about `19.1k` syscalls total, extent mode at about `31.4k`, and extent `recvfrom` remained the syscall to watch.

## FOD 3.0.2 Fio Mixed Sequential RW Extents Smoke

Observed on the current host with the mounted PostgreSQL-backed runtime, `tests/integration/test_fio_mixed_io.sh`, `FIO_RW_MODE=rw`, `FIO_BLOCK_SIZE=4k`, and file sizes from `64 KiB` through `4 MiB`. The test covers both the current block-storage path and the opt-in extent preset. The `4 KiB` smoke point was also exercised, but it is too small to be a useful performance signal and is therefore omitted from the table below.

| File size | Block write | Extent write | Block read | Extent read |
| --- | --- | --- | --- | --- |
| `64 KiB` | `444 KiB/s` | `436 KiB/s` | `741 KiB/s` | `727 KiB/s` |
| `1 MiB` | `458 KiB/s` | `359 KiB/s` | `488 KiB/s` | `382 KiB/s` |
| `4 MiB` | `470 KiB/s` | `470 KiB/s` | `441 KiB/s` | `442 KiB/s` |

Notes from the current host:

- The mixed sequential workload does not show a consistent extent win, unlike the larger pure sequential write cases above.
- At `64 KiB` the extent path is close to block storage, but it is not better.
- At `1 MiB` the extent path is slower on both read and write, which reinforces the opt-in nature of the PoC.
- At `4 MiB` the two paths are effectively tied on write and the read difference is in the noise.
- This benchmark is useful as a negative control: extents should remain a benchmarked preset, not the default storage path.

## FOD 3.0.2 Fio Random Mixed RW Extents Negative Control

Observed on the current host with the mounted PostgreSQL-backed runtime, `tests/integration/test_fio_mixed_io.sh`, `FIO_RW_MODE=randrw`, `FIO_RWMIXREAD=50`, `FIO_BLOCK_SIZE=4k`, and file sizes from `4 KiB` through `4 MiB`. The test covers both the current block-storage path and the opt-in extent preset. The `4 KiB` point is again only a smoke guard and should not be over-interpreted.

| File size | Block write | Extent write | Block read | Extent read |
| --- | --- | --- | --- | --- |
| `4 KiB` | `267 KiB/s` | `258 KiB/s` | `333 KiB/s` | `444 KiB/s` |
| `64 KiB` | `267 KiB/s` | `258 KiB/s` | `444 KiB/s` | `430 KiB/s` |
| `1 MiB` | `276 KiB/s` | `197 KiB/s` | `294 KiB/s` | `210 KiB/s` |
| `4 MiB` | `277 KiB/s` | `257 KiB/s` | `260 KiB/s` | `242 KiB/s` |

Notes from the current host:

- This benchmark is intended as a negative control to show that extents are not a universal win.
- At `64 KiB` the two paths are close enough that the result is mostly noise.
- At `1 MiB` and `4 MiB` the extent path is slower on both read and write, which reinforces the opt-in nature of the PoC.
- The random mixed workload is a useful counterexample to the sequential smoke above: it does not justify flipping extents on by default.

## FOD 3.0.2 Fio 64 KiB Block-Size Matrix

Observed on the current host with the mounted PostgreSQL-backed runtime and the same fio smoke scripts, but with `FIO_BLOCK_SIZE=64k`. `fio` rejects file sizes smaller than the block size, so the `4 KiB` point from the smaller-block matrix is not applicable here. The `64 KiB` mixed file-size point was exercised as a smoke guard, but it is too small to give a stable read/write split, so the mixed table starts at `1 MiB`.

### Sequential

| File size | Block write | Extent write | Block read | Extent read |
| --- | --- | --- | --- | --- |
| `64 KiB` | `1333 KiB/s` | `3200 KiB/s` | `4000 KiB/s` | `3765 KiB/s` |
| `1 MiB` | `2909 KiB/s` | `2868 KiB/s` | `9309 KiB/s` | `8904 KiB/s` |
| `4 MiB` | `3122 KiB/s` | `4847 KiB/s` | `5205 KiB/s` | `5894 KiB/s` |

Notes from the current host:

- At `64 KiB`, extents are much faster on write and slightly slower on read.
- At `1 MiB`, the two paths are very close on write and still close on read.
- At `4 MiB`, extents are clearly better on write and also ahead on read.
- This keeps the same overall pattern as the smaller-block sequential smoke: extents help on the larger sequential writes, but the win is workload-sensitive.

### Mixed Sequential RW

| File size | Block write | Extent write | Block read | Extent read |
| --- | --- | --- | --- | --- |
| `1 MiB` | `1164 KiB/s` | `997 KiB/s` | `1939 KiB/s` | `1662 KiB/s` |
| `4 MiB` | `2543 KiB/s` | `1871 KiB/s` | `2107 KiB/s` | `1551 KiB/s` |

Notes from the current host:

- The mixed workload does not show an extent win at `64 KiB`; the one-block smoke at that size is not a useful performance datapoint.
- At `1 MiB` and `4 MiB`, extents are slower on both read and write.
- This is the important counterweight to the sequential matrix: `64 KiB` IO does not turn extents into a general-purpose default path.

## Current Baseline Snapshot

### Latest Local Run

Observed on the current host with local Docker/PostgreSQL and FOD 2.4.7:

- `make test-throughput`
  - `1 MiB` zero-source write
  - `elapsed_s=0.071`
  - `throughput_mib_s=14.13`
- `make test-throughput-sync`
  - `1 MiB` zero-source write with `conv=fsync`
  - `elapsed_s=0.083`
  - `throughput_mib_s=12.08`
- `make test-copy-dedupe-benchmark`
  - `copy-dedupe/off`
    - `bytes=4194304`
    - `elapsed_s=0.000013`
    - `ranges=1`
    - `changed_bytes=4194304`
  - `copy-dedupe/on`
    - `bytes=4194304`
    - `elapsed_s=0.000958`
    - `ranges=0`
    - `changed_bytes=0`
- `make test-tree-scale`
  - `dirs=60`
  - `files_per_dir=100`
  - `ls_ms=137.82`
  - `find_ms=7463.91`
- `make test-atime-benchmark`
  - file reads
    - `default=789 ms`
    - `noatime=917 ms`
    - `nodiratime=770 ms`
  - directory listings
    - `default=7115 ms`
    - `noatime=5390 ms`
    - `nodiratime=5419 ms`
- `make test-large-copy-benchmark`
  - `bytes=67108864`
  - `elapsed_s=9.327550`
  - `throughput_mib_s=6.86`
- `make test-large-file-multiblock-benchmark`
  - `bytes=67108864`
  - `elapsed_s=1.412722`
  - `throughput_mib_s=45.30`
  - `write_seconds=0.072068`
  - `persist_seconds=2.110270`
  - `flush_seconds=2.112674`
  - `finalization_seconds=4.222943`
- `make test-remount-durability-benchmark`
  - `bytes=65536`
  - `elapsed_s=1.072187`

The newer local run confirms the same general shape as the earlier baselines: large batch writes still dominate tiny writes, remount durability is a separate latency budget, `tree-scale` is sensitive to metadata fanout, and `noatime` / `nodiratime` remain workload-specific rather than universal wins.

### Write Path Throughput

Observed on a mounted FOD instance:

- `4 KiB` burst writes: roughly `0.03 MiB/s`
- `1 MiB` write: roughly `4.53 MiB/s`
- `4 MiB` write: roughly `9.87 MiB/s`
- `8 MiB` write: roughly `9.06 MiB/s`
- `16 MiB` write: roughly `7.83 MiB/s`

Recent `bulk_write` profile write timing with `dd`:

- `bs=4k count=10240`
  - `bytes=41943040`
  - `elapsed_s=5.56531`
  - `throughput_mib_s=7.5`
- `bs=4M count=100`
  - `bytes=419430400`
  - `elapsed_s=19.1294`
  - `throughput_mib_s=22`

The `4M` batch remains materially faster than the tiny `4k` burst on this profile, which matches the current tuning direction.

### Binary BYTEA Transport Smoke

Observed after replacing hex text payloads with binary `BYTEA` params in the hot-path batch write:

- `bulk_write`, `bs=4k`, `count=100`
  - `409600 bytes in 0.272s (1.44 MiB/s)`
- `bulk_write`, `bs=1M`, `count=100`
  - `104857600 bytes in 29.393s (3.40 MiB/s)`
- `metadata_heavy`, `bs=4k`, `count=100`
  - `409600 bytes in 0.375s (1.04 MiB/s)`
- `metadata_heavy`, `bs=1M`, `count=100`
  - `104857600 bytes in 22.069s (4.53 MiB/s)`

Binary `BYTEA` removes the hex/decode hop, but the short smoke still stays workload-sensitive: `bulk_write` is better for the small burst, while `metadata_heavy` is ahead on the larger `1 MiB` blocks on this host. Treat this as a smoke baseline rather than a long-run throughput claim.

This smoke stayed close to the earlier hex/decode numbers, so it was not a meaningful short-run jump by itself. That is why the next step moved to staged `COPY BINARY`.

### COPY BINARY + Staging Smoke

Observed after moving the hot-path block write to staged `COPY BINARY` and merge:

- Profile: `bulk_write` and `metadata_heavy` smoke runs
- Mount mode: writable primary
- FOD version: `FOD 2.01.117`
- PostgreSQL mode: local Docker/PostgreSQL
- sync/fsync: no fsync in this block
- date: not recorded

- `bulk_write`, `bs=4k`, `count=100`
  - `409600 bytes in 0.278s (1.41 MiB/s)`
- `bulk_write`, `bs=1M`, `count=100`
  - `104857600 bytes in 27.072s (3.69 MiB/s)`
- `metadata_heavy`, `bs=4k`, `count=100`
  - `409600 bytes in 0.337s (1.16 MiB/s)`
- `metadata_heavy`, `bs=1M`, `count=100`
  - `104857600 bytes in 19.942s (5.01 MiB/s)`

The staged `COPY BINARY` path improves the larger `1 MiB` cases on this host while keeping the small `4k` burst in the same general range. It is still a smoke benchmark, not a long-run claim.

### Recent Throughput Run

Observed on the current default throughput target (`make test-throughput`):

- Profile: current throughput target
- Mount mode: writable primary
- FOD version: not recorded in the benchmark text
- PostgreSQL mode: local Docker/PostgreSQL
- sync/fsync: no fsync in this block
- date: not recorded

- `4M x8`
  - `33554432 bytes in 1.522s (21.02 MiB/s)`
- `8M x4`
  - `33554432 bytes in 1.486s (21.54 MiB/s)`
- `16M x2`
  - `33554432 bytes in 1.588s (20.15 MiB/s)`

### Finalization Profile

Observed on the current mounted FOD instance with `FOD_PROFILE_IO=1`:

- `persist_buffer_chunk_blocks=128`
  - `write_seconds=0.001127`
  - `persist_seconds=0.003594`
  - `flush_seconds=0.003700`
  - `finalization_seconds=0.007293`
- `persist_buffer_chunk_blocks=512`
  - `write_seconds=0.001751`
  - `persist_seconds=0.004242`
  - `flush_seconds=0.004312`
  - `finalization_seconds=0.008554`
- `release()` cleanup after `persist_buffer()`
  - `write_seconds=0.000913`
  - `persist_seconds=0.005033`
  - `flush_seconds=0.005079`
  - `finalization_seconds=0.010112`
- truncate-only flush/release on a large file
  - `persist_seconds=0.002476`
  - `flush_seconds=0.002512`
  - `finalization_seconds=0.004989`

The larger chunk setting shaved a bit off the finalization path on this run, so `bulk_write` now uses the larger batch size.
The write side itself is now effectively negligible in this profile; the remaining work is concentrated in `persist_buffer()` and `flush()`.
The latest small win came from switching block upserts inside `persist_buffer()` to PostgreSQL `execute_values()`, making the batch size configurable, avoiding an extra copy when building block payloads for flush, caching dirty-byte accounting so `maybe_flush_dirty_write_buffer()` does not rescan every dirty block on each write, and adding a single-block fast path that bypasses `execute_values()` when only one dirty block needs to be persisted.
Truncate-only finalization now short-circuits block packing when no dirty blocks remain, which keeps the large-file truncate path from paying extra pre-persist block packing work before the necessary tail delete.

## Historical Throughput Comparison

The write path has also been measured on a large sequential write where chunked persistence prevented PostgreSQL client buffer exhaustion.
These numbers are kept as a migration-period comparison set and should not be read as the current runtime baseline.

Recorded on a migration-period throughput profile:

- Profile: historical throughput comparison harness
- Mount mode: writable primary
- FOD version: not recorded in the benchmark text
- PostgreSQL mode: local Docker/PostgreSQL
- sync/fsync: compared explicitly below
- date: not recorded

- `THROUGHPUT_BLOCK_SIZE=4M THROUGHPUT_COUNT=8`
  - `33554432 bytes in 6.217s (5.15 MiB/s)`
- `THROUGHPUT_BLOCK_SIZE=4M THROUGHPUT_COUNT=8 THROUGHPUT_SYNC=1`
  - `33554432 bytes in 6.476s (4.94 MiB/s)`
- `THROUGHPUT_BLOCK_SIZE=8M THROUGHPUT_COUNT=4`
  - `33554432 bytes in 6.388s (5.01 MiB/s)`

Current read:
- `write` without `fsync` is still the fastest of the three.
- `write` with `fsync` is the clearest durable-write penalty.
- a larger `THROUGHPUT_BLOCK_SIZE` did not beat the current `4M` baseline on this run, so the bottleneck is not just block granularity.

### Synchronous Commit

Observed on the migration-period flush/release profile:

- `FOD_SYNCHRONOUS_COMMIT=on`
  - `write_seconds=0.000605`
  - `persist_seconds=0.007334`
  - `flush_seconds=0.007374`
  - `finalization_seconds=0.014708`
- `FOD_SYNCHRONOUS_COMMIT=off`
  - `write_seconds=0.000870`
  - `persist_seconds=0.005471`
  - `flush_seconds=0.005533`
  - `finalization_seconds=0.011004`

On this local Docker/PostgreSQL run, `off` improved the flush/release path, while the overall throughput comparisons below still remain workload-sensitive, so it is kept as an explicit tuning knob rather than a forced default.

#### Throughput Comparison

Observed on the migration-period throughput profile:

- `4M x8`
  - `FOD_SYNCHRONOUS_COMMIT=off` -> `33554432 bytes in 6.217s (5.15 MiB/s)`
  - `FOD_SYNCHRONOUS_COMMIT=on` -> `33554432 bytes in 6.287s (5.09 MiB/s)`
- `8M x4`
  - `FOD_SYNCHRONOUS_COMMIT=off` -> `33554432 bytes in 6.388s (5.01 MiB/s)`
- `16M x2`
  - `FOD_SYNCHRONOUS_COMMIT=off` -> `33554432 bytes in 6.414s (4.99 MiB/s)`
  - `FOD_SYNCHRONOUS_COMMIT=on` -> `33554432 bytes in 6.484s (4.94 MiB/s)`

The effect is workload-sensitive: `off` helped some batch sizes and slightly hurt another, so the knob remains explicit rather than being forced globally.

#### Fsync-Backed Throughput

Observed on the migration-period throughput profile with `THROUGHPUT_SYNC=1`:

- `4M x8`
  - `33554432 bytes in 1.636s (19.55 MiB/s)`
- `8M x4`
  - `33554432 bytes in 1.522s (21.02 MiB/s)`
- `16M x2`
  - `33554432 bytes in 1.458s (21.95 MiB/s)`

On this run, the fsync-backed write path stayed in the same general range as the non-fsync batch sizes, with `16M x2` slightly ahead of the smaller batches.
The short version is that `THROUGHPUT_SYNC=1` is worth keeping as a durability-vs-throughput comparison knob, but it is not a universal win over the non-sync baseline.

`copy_dedupe_enabled` should follow the same rule: keep it off for ordinary ingest and one-shot copies, and only enable it for rsync-like workloads or repeated copy-heavy syncs where destination blocks are often already identical. The extra destination reads can easily outweigh the saved writes if the file contents are usually changing anyway.

Worker parallelism is still block-oriented, so `block_size` changes when a workload crosses the thresholds for `workers_read` or `workers_write`. It does not translate directly into "N bytes per thread"; it only changes how many blocks a given transfer is split into before the worker thresholds are applied.

## Historical Migration Baselines

These numbers are kept as historical pre/full-Rust migration baselines. They should not be read as evidence that the current runtime still has an active Python execution fallback.

### Historical Copy Dedupe Benchmark

Observed on a repeated changed-copy workload with the Rust dedupe helper available as an opt-in path:

- Historical Python-era baseline
  - `bytes=67108864`
  - `elapsed_s=55.920010`
  - `throughput_mib_s=1.14`
  - `write_seconds=0.000000`
  - `persist_seconds=0.757722`
  - `flush_seconds=0.758866`
  - `finalization_seconds=1.516588`
- Rust helper
  - `bytes=67108864`
  - `elapsed_s=74.018375`
  - `throughput_mib_s=0.86`
  - `write_seconds=0.000000`
  - `persist_seconds=1.712512`
  - `flush_seconds=1.713539`
  - `finalization_seconds=3.426051`

On this host the Rust dedupe helper did not produce an end-to-end win. The internal changed-copy packing was not enough to offset the total runtime cost, so the historical Python-era baseline remains the comparison point and the Rust path stays opt-in.

### Bulk Write Profile Comparison

Observed on the current `bulk_write` profile after restoring a stronger read-side:

- large sequential copy, 4M batch
  - `bytes=67108864`
  - `elapsed_s=2.498050`
  - `throughput_mib_s=25.62`
- large sequential copy, 8M batch
  - `bytes=67108864`
  - `elapsed_s=2.726928`
  - `throughput_mib_s=23.47`
- large sequential copy, fsync-backed, 4M batch
  - `bytes=67108864`
  - `elapsed_s=2.564408`
  - `throughput_mib_s=24.96`
- large sequential copy, fsync-backed, 8M batch
  - `bytes=67108864`
  - `elapsed_s=2.781986`
  - `throughput_mib_s=23.01`
- large sequential copy, fsync-backed, 16M batch
  - `bytes=67108864`
  - `elapsed_s=2.560397`
  - `throughput_mib_s=25.00`
- large sequential copy
  - `bytes=67108864`
  - `elapsed_s=2.491982`
  - `throughput_mib_s=25.68`
- large multi-block file write
  - `bytes=67108864`
  - `elapsed_s=2.229123`
  - `throughput_mib_s=28.71`
  - `write_seconds=0.072068`
  - `persist_seconds=2.110270`
  - `flush_seconds=2.112674`
  - `finalization_seconds=4.222943`
- flush/release profile
  - `write_seconds=0.001076`
  - `persist_seconds=0.006235`
  - `flush_seconds=0.006303`
  - `finalization_seconds=0.012537`

The write-path optimization that avoids loading brand-new blocks from PostgreSQL before writing them made the `bulk_write` profile much stronger on copy-heavy ingest and large multi-block writes.
The profile is still workload-specific, but it now clearly favors the intended ingest/copy path while keeping finalization cost bounded. On this host the bigger `8M` copy batch did not beat the `4M` batch, and the fsync-backed copy run stayed in the same general range rather than producing a clear win, so the current default copy granularity remains a measured choice rather than an unconditional "larger is better" rule.

### Copy Profile Comparison

Observed on the same large `copy_file_range()` benchmark across runtime profiles:

- `bulk_write`
  - `bytes=67108864`
  - `elapsed_s=2.482370`
  - `throughput_mib_s=25.78`
- `metadata_heavy`
  - `bytes=67108864`
  - `elapsed_s=3.903447`
  - `throughput_mib_s=16.40`

On this host `bulk_write` is materially faster than `metadata_heavy` for large copy-heavy ingest, which matches the intended profile split: `bulk_write` is for ingest and copy throughput, while `metadata_heavy` is for namespace browsing and metadata-heavy workflows.

### Copy Dedupe / Repeated Copy

Observed on a repeated copy where the destination already contained the same block content:

- `copy_dedupe_enabled=off`
  - `bytes=67108864`
  - `elapsed_s=8.557302`
  - `throughput_mib_s=7.48`
  - `write_seconds=0.000000`
  - `persist_seconds=1.753822`
  - `flush_seconds=0.000000`
  - `finalization_seconds=1.753822`
- `copy_dedupe_enabled=on`
  - `bytes=67108864`
  - `elapsed_s=56.208670`
  - `throughput_mib_s=1.14`
  - `write_seconds=0.000000`
  - `persist_seconds=0.000000`
  - `flush_seconds=0.000000`
  - `finalization_seconds=0.000000`

This run shows that the dedupe path is only worth enabling for cases where avoiding rewritten destination blocks matters more than the extra comparison cost. For identical destination copies on this host, the comparison overhead is much higher than a normal replay of the write path, so the historical Python-era baseline remains the better comparison point even though the Rust helper is enabled by default.

### Historical Rust Packer Benchmark

Observed on a changed-copy workload with mixed unchanged and changed blocks:

- Historical Python-era baseline (legacy Rust packer disabled)
  - `bytes=67108864`
  - `elapsed_s=55.584490`
  - `throughput_mib_s=1.15`
  - `write_seconds=0.000000`
  - `persist_seconds=1.198202`
  - `flush_seconds=1.201681`
  - `finalization_seconds=2.399884`
- Rust packer comparison run (legacy Rust packer enabled)
  - `bytes=67108864`
  - `elapsed_s=55.978419`
  - `throughput_mib_s=1.14`
  - `write_seconds=0.000000`
  - `persist_seconds=1.157677`
  - `flush_seconds=1.158987`
  - `finalization_seconds=2.316664`

This benchmark did not show a meaningful end-to-end win for the Rust packer on this host. The Rust path was slightly better on the internal persist/flush accounting, but the overall elapsed time stayed effectively flat, so the historical Python-era baseline remains the comparison point even though the Rust packer stays enabled by default.

### PostgreSQL Session Cost

Measured on a pooled FOD backend:

- first pooled connection initialization:
  - `first_ms=1.0561`
- steady state after warmup:
  - `steady_mean_ms=0.2841`
  - `steady_p95_ms=0.4627`

Interpretation:

- the UTC `SET TIME ZONE` cost is paid once per physical connection
- after warmup, the remaining overhead is sub-millisecond per acquire and still small compared with filesystem-level I/O

## Read Cache

Sequential read-cache comparison:

- `FOD_READ_CACHE_BLOCKS=256` -> `elapsed_ms=14379`
- `FOD_READ_CACHE_BLOCKS=1024` -> `elapsed_ms=3244`

The larger cache is the current default and the tests keep the regression covered.

## Tree Scale / Metadata Heavy

Latest seeded-tree benchmark on the current host:

- default profile
  - `dirs=60`
  - `files_per_dir=100`
  - `ls_ms=137.82`
  - `find_ms=7463.91`

Comparison on the same `20 x 20` seeded tree from the earlier baseline:

- default profile
  - `dirs=20`
  - `files_per_dir=20`
  - `ls_ms=621.00`
  - `find_ms=9478.38`
- `metadata_heavy`
  - `dirs=20`
  - `files_per_dir=20`
  - `ls_ms=401.25`
  - `find_ms=8581.42`

`metadata_heavy` is noticeably better for `ls` on this tree and slightly better for `find`, which matches its goal: reduce metadata churn on tree-walking workloads without pushing the write side.

## Atime Behavior

Short wall-time benchmark on file reads and directory listings:

- file reads:
  - `default=789 ms`
  - `noatime=917 ms`
  - `nodiratime=770 ms`
- directory listings:
  - `default=7115 ms`
  - `noatime=5390 ms`
  - `nodiratime=5419 ms`

The benchmark is useful as a smoke baseline, not as a strong microbenchmark for exact atime savings.

## Large Copy

Large `copy_file_range()` benchmark on the current runtime profile:

- `bytes=67108864`
  - `elapsed_s=9.327550`
  - `throughput_mib_s=6.86`

This is the current baseline for large backend copy operations.

## Large Multi-Block Files

Large multi-block file write benchmark on the current runtime profile:

- `bytes=67108864`
  - `elapsed_s=1.412722`
  - `throughput_mib_s=45.30`
  - `write_seconds=0.072068`
  - `persist_seconds=2.110270`
  - `flush_seconds=2.112674`
  - `finalization_seconds=4.222943`

This baseline tracks a large file write split across many blocks so the write/persist split stays visible. The detailed timing breakdown below is from the earlier instrumented profile run and is kept as the split-time reference.

## Remount Durability

Remount durability smoke benchmark on the current runtime profile:

- `bytes=65536`
  - `elapsed_s=1.072187`

This is a durability baseline, not a throughput target. The goal is to keep the remount/reopen path explicit and data-safe.

## Copy Dedupe Planner

Planner microbenchmark on the current hot-path dedupe helper:

- `copy-dedupe/off`
  - `bytes=4194304`
  - `elapsed_s=0.000013`
  - `ranges=1`
  - `changed_bytes=4194304`
- `copy-dedupe/on`
  - `bytes=4194304`
  - `elapsed_s=0.000958`
  - `ranges=0`
  - `changed_bytes=0`

The planner path is intentionally tiny, so the useful number is mostly the changed-range shape rather than the absolute MiB/s figure.
