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
- The latest PostgreSQL optimization comparison in this file was collected on 2026-07-05 from commit `a3076e1` and adds a fresh local/QNAP COPY-buffer matrix with DML, WAL, top statement IO/WAL, and bloat artifacts.
- The current FUSE migration reference was collected locally on 2026-07-11 from commit `7d9ed83` with `fuser 0.14`, ABI 7.31, schema v17, and three samples per block/1 MiB extent mode.

## 2026-07-11 Current FUSE ABI 7.31 Baseline

Measured production base commit: `7d9ed837bec69670501c78262c08723fde5d5f48`
(`FOD 3.2.1: define Rust toolchain baseline`). The runtime code was clean at
that commit; pending measurement-only changes added unique callback counts and
isolated repeated fio filenames. Full methodology, SQL evidence, acceptance
criteria, and local artifact paths are in
`docs/fuse-abi-7-31-current-baseline.md`.

Environment: `lt7300`, Linux 6.17.0-40-generic, libfuse3 3.17.4,
`fuser 0.14` with ABI 7.31, Rust 1.85.1, local PostgreSQL 16.14, debug/test
binaries. Each accepted row is the mean of three runs.

| workload | layout | result | FUSE read/write/copy calls | WAL bytes |
| --- | --- | ---: | --- | ---: |
| Exact 64 MiB object adoption | blocks | `8050.38 MiB/s` | `512 / 64 / 1` | `6413672` |
| Exact 64 MiB object adoption | 1 MiB extents | `9979.55 MiB/s` | `512 / 64 / 1` | `853568` |
| Chunked 64 MiB copy, 4 MiB requests | blocks | `19.05 MiB/s` | `512 / 64 / 16` | `13084834` |
| Chunked 64 MiB copy, 4 MiB requests | 1 MiB extents | `31.66 MiB/s` | `512 / 64 / 16` | `7477992` |
| Sequential 64 MiB write/readback | blocks | `54.66 MiB/s` | `512 / 64 / 0` | `7242445` |
| Sequential 64 MiB write/readback | 1 MiB extents | `113.01 MiB/s` | `512 / 64 / 0` | `1202592` |

| fio workload | blocks read/write KiB/s | 1 MiB extents read/write KiB/s | calls per layout |
| --- | ---: | ---: | --- |
| Sequential 64 MiB | `116053 / 4033` | `40209 / 3878` | `260 read / 16384 write` |
| Mixed 64 MiB | `1480 / 1491` | `643 / 648` | `26 read / 24608 write` |
| Random mixed 64 MiB | `1043 / 1051` | `537 / 541` | `8160 read / 24608 write` |

Exact destinations added no payload rows: all six measured source objects had
two file references, `reference_count = 2`, and only one physical layout. The
final database diagnostic reported zero orphan payload rows, unreferenced
objects, reference-count mismatches, and hybrid block/extent objects. The
partial extent patch and block/extent remount gates passed.

The first mixed-fio attempt reused a fixed filename, so later repeats measured
an existing payload. It is excluded. The accepted isolated series uses a
process-specific filename and produced the expected 24608 write callbacks in
every sample. Extents remain opt-in because their lower row/WAL cost and strong
dedicated sequential result do not offset the measured fio read and mixed-I/O
regressions.

## 2026-07-10 Bounded Extent Execution Smoke

Collected from a Storage Engine v2 worktree based on commit `93f1ab9` (`FOD 3.2.1: add bounded extent planning`). The pending change added bounded payload-row enforcement and peak-payload diagnostics.

Commands:

```bash
FOD_PROFILE_IO=1 FIO_FILE_SIZE=4M make test-fio-sequential-io
make test-fio-mixed-io
make test-fio-random-mixed-io
FOD_PROFILE_IO=1 make test-fio-sequential-io-strace
```

| workload | path | read | write | extent rows | largest extent payload |
| --- | --- | ---: | ---: | ---: | ---: |
| sequential 4 MiB | block | `100 MiB/s` | `4905 KiB/s` | `0` | `0` |
| sequential 4 MiB | extent | `90.9 MiB/s` | `4597 KiB/s` | `4` | `1048576` bytes |
| mixed rw 4 MiB | block | `1699 KiB/s` | `1808 KiB/s` | n/a | n/a |
| mixed rw 4 MiB | extent preset | `824 KiB/s` | `877 KiB/s` | `4` observed after the run | `1048576` bytes |
| random mixed 4 MiB | block | `1196 KiB/s` | `1273 KiB/s` | n/a | n/a |
| random mixed 4 MiB | extent preset | `691 KiB/s` | `736 KiB/s` | `4` observed after the run | `1048576` bytes |

The profiled sequential extent run reported `repo_persist_extents_us=30843`, `prepare_persist_extent_rows_from_extent_ranges_us=1916`, and `prepare_persist_extent_rows_peak_payload_bytes=1048576`. The PostgreSQL check returned `fio-extent-4M.bin|4|1048576|4194304`, confirming four physical rows, a 1 MiB maximum payload, and complete 4 MiB logical coverage.

This is a correctness and diagnostics smoke, not the Phase A decision matrix. It is a single local run and therefore does not justify a default change. Mixed and random results continue to show why the extent path must remain opt-in until Phase B can distinguish sequential segment state from block overlays.

## 2026-07-10 Storage Engine v2 Bounded Extent Matrix

Collected from a worktree based on commit `38af786` (`FOD 3.2.1: persist bounded extent payloads`). The matrix runner and summary generator were pending in the measured worktree.

Repeated core command:

```bash
PROFILE_RUN_ID=storage-extent-core-20260710T201100Z \
PROFILE_STORAGE_EXTENT_REPEAT=3 \
PROFILE_STORAGE_EXTENT_WORKLOADS=test-large-file-multiblock-benchmark \
make profile-storage-extent-size-matrix-local
```

Workload: local Docker PostgreSQL, 64 MiB sequential multi-block write/readback (`4M * 16`), three independent samples per storage mode, with compilation completed before `/usr/bin/time` measurement.

| mode | target | throughput mean | stdev | min-max | elapsed mean | physical inserts | peak payload | max RSS mean | WAL bytes mean |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| block | n/a | `52.82 MiB/s` | `1.63` | `50.68-54.65` | `1.212869s` | `16384 data_blocks` | `0` | `137612 KiB` | `7325238` |
| extent | `64 KiB` | `87.67 MiB/s` | `10.15` | `73.35-95.77` | `0.740795s` | `1024 data_extents` | `65536` | `137681 KiB` | `1303234` |
| extent | `256 KiB` | `94.66 MiB/s` | `2.05` | `92.75-97.51` | `0.676405s` | `256 data_extents` | `262144` | `137689 KiB` | `966427` |
| extent | `1 MiB` | `94.51 MiB/s` | `1.73` | `92.79-96.87` | `0.677414s` | `64 data_extents` | `1048576` | `137756 KiB` | `898442` |
| extent | `4 MiB` | `91.70 MiB/s` | `2.93` | `89.10-95.79` | `0.698601s` | `16 data_extents` | `4194304` | `137755 KiB` | `854096` |

Repeated QNAP command:

```bash
PROFILE_RUN_ID=storage-extent-qnap-core-20260710T202500Z \
PROFILE_STORAGE_EXTENT_REPEAT=3 \
PROFILE_STORAGE_EXTENT_WORKLOADS=test-large-file-multiblock-benchmark \
make profile-storage-extent-size-matrix-qnap
```

Workload: QNAP Docker PostgreSQL over the configured remote connection, the same 64 MiB sequential multi-block write/readback (`4M * 16`), and three independent samples per storage mode.

| mode | target | throughput mean | stdev | min-max | elapsed mean | physical inserts | peak payload | max RSS mean | WAL bytes mean |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| block | n/a | `10.56 MiB/s` | `1.39` | `8.60-11.61` | `6.179208s` | `16384 data_blocks` | `0` | `137781 KiB` | `7317404` |
| extent | `64 KiB` | `19.31 MiB/s` | `1.49` | `17.48-21.14` | `3.334079s` | `1024 data_extents` | `65536` | `137612 KiB` | `1139425` |
| extent | `256 KiB` | `24.70 MiB/s` | `1.89` | `22.49-27.11` | `2.606442s` | `256 data_extents` | `262144` | `137640 KiB` | `1162674` |
| extent | `1 MiB` | `21.20 MiB/s` | `1.02` | `19.75-21.95` | `3.027129s` | `64 data_extents` | `1048576` | `137740 KiB` | `928921` |
| extent | `4 MiB` | `23.13 MiB/s` | `0.86` | `21.95-23.98` | `2.770675s` | `16 data_extents` | `4194304` | `137708 KiB` | `913952` |

Full-workload smoke command:

```bash
PROFILE_RUN_ID=storage-extent-full-smoke-20260710T201500Z \
PROFILE_STORAGE_EXTENT_REPEAT=1 \
PROFILE_STORAGE_EXTENT_FIO_FILE_SIZE=4M \
PROFILE_STORAGE_EXTENT_LARGE_FILE_CHUNK_COUNT=4 \
PROFILE_STORAGE_EXTENT_LARGE_COPY_BLOCK_COUNT=4 \
make profile-storage-extent-size-matrix-local
```

Selected one-run results:

| workload | block | 64 KiB | 256 KiB | 1 MiB | 4 MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| large-file 16 MiB | `44.28 MiB/s` | `92.46` | `91.30` | `89.94` | `88.29` |
| large-copy 16 MiB | `22.69 MiB/s` | `17.14` | `26.64` | `27.39` | `25.12` |
| sequential fio read | `21811 KiB/s` | `28877` | `28058` | `24986` | `26214` |
| sequential fio write | `1690 KiB/s` | `763` | `753` | `607` | `616` |
| mixed fio read/write | `1318/1403 KiB/s` | `677/721` | `662/704` | `662/704` | `628/668` |
| random-mixed fio read/write | `947/1008 KiB/s` | `485/516` | `481/512` | `389/414` | `405/431` |
| remount elapsed | `1.020880s` | `1.015313` | `1.018730` | `1.021011` | `1.020046` |

Artifacts:

- `artifacts/perf/38af786/lt7300-storage-extent-core-20260710T201100Z-storage-extent-summary.md`
- `artifacts/perf/38af786/lt7300-storage-extent-full-smoke-20260710T201500Z-storage-extent-summary.md`
- `artifacts/perf/38af786/lt7300-storage-extent-qnap-core-20260710T202500Z-storage-extent-summary.md`

Conclusion: Phase A passed its repeated local and QNAP gate. Bounded extents approximately doubled the repeated 64 MiB large-file throughput and reduced physical row count by 16x to 1024x without increasing overall RSS. On QNAP, even the slowest extent sample (`17.48 MiB/s`) stayed above the fastest block sample (`11.61 MiB/s`), while mean WAL fell from about `7.32 MB` to `0.91-1.16 MB`. The unchanged RSS shows why Phase B is still needed: `WriteState` continues to hold 4 KiB block vectors before rebuilding bounded payloads. The full local smoke also shows that the current extent selection is not suitable for mixed/random or fio write patterns. Keep the block path as the default, keep `extent_target_bytes=1 MiB` as the balanced opt-in value, and proceed to the sequential segment builder without broadening extent selection.

## 2026-07-11 Direct Sequential Segment Persistence

Collected from a Storage Engine v2 worktree based on commit `f0e0a1c` (`FOD 3.2.1: add sequential segment write state`). The pending change moved bounded segment payloads directly into native extent rows and added segment-mode profiling.

Commands:

```bash
PROFILE_RUN_ID=storage-segment-direct-core-20260711T065722Z \
PROFILE_STORAGE_EXTENT_REPEAT=3 \
PROFILE_STORAGE_EXTENT_SIZES=1048576 \
PROFILE_STORAGE_EXTENT_WORKLOADS=test-large-file-multiblock-benchmark \
make profile-storage-extent-size-matrix-local

PROFILE_RUN_ID=storage-segment-direct-copy-20260711T065838Z \
PROFILE_STORAGE_EXTENT_REPEAT=3 \
PROFILE_STORAGE_EXTENT_SIZES=1048576 \
PROFILE_STORAGE_EXTENT_WORKLOADS=test-large-copy-benchmark \
make profile-storage-extent-size-matrix-local
```

| workload | path | throughput mean | elapsed mean | segment preparation | segment entries/downgrades | physical inserts | WAL bytes mean | max RSS mean |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 64 MiB large-file | block | `49.03 MiB/s` | `1.305976s` | `0 us` | `0/0` | `16384 data_blocks` | `7355280` | `137680 KiB` |
| 64 MiB large-file | direct 1 MiB segments | `98.81 MiB/s` | `0.647826s` | `12.33 us` | `1/0` | `64 data_extents` | `889684` | `141736 KiB` |
| 64 MiB large-copy | block | `18.50 MiB/s` | `3.459414s` | `0 us` | `0/0` | `32768 data_blocks` | `12938304` | `143685 KiB` |
| 64 MiB large-copy | direct 1 MiB segments | `14.07 MiB/s` | `4.549553s` | `18 us` | `2/0` | `128 data_extents` | `1709398` | `137741 KiB` |

The direct large-file result is about `4.5%` faster than the earlier 1 MiB bounded-extent baseline (`94.51 MiB/s`) and removes the approximately `32 ms` block-to-extent preparation step; preparation is now just descriptor validation plus ownership moves. The isolated high RSS sample (`149848 KiB`) makes the three-run large-file RSS mean noisy, while the other two direct samples remain close to the block baseline.

Large-copy is a negative result: direct segment preparation is only `18 us`, but the extent run is about `24%` slower than the block run. SQL profiling points to repeated extent reads while copying rather than payload rebuilding. This closes the Phase B payload-preparation bottleneck, but extents remain opt-in and large-copy must not be routed broadly through the future new-object class until the read amplification is removed.

Artifacts:

- `artifacts/perf/f0e0a1c/lt7300-storage-segment-direct-core-20260711T065722Z-storage-extent-summary.md`
- `artifacts/perf/f0e0a1c/lt7300-storage-segment-direct-copy-20260711T065838Z-storage-extent-summary.md`

## 2026-07-11 Append-Only Sequential Object Persistence

Collected from a Storage Engine v2 worktree based on commit `42c5edf`
(`FOD 3.2.1: classify storage persistence operations`). The pending change
routed complete direct-segment payloads through a replay-confirmed replacement
data-object transaction instead of mutating the previous object's extent rows.

Commands:

```bash
PROFILE_RUN_ID=storage-append-only-core-20260711T073350Z \
PROFILE_STORAGE_EXTENT_REPEAT=3 \
PROFILE_STORAGE_EXTENT_SIZES=1048576 \
PROFILE_STORAGE_EXTENT_WORKLOADS=test-large-file-multiblock-benchmark \
make profile-storage-extent-size-matrix-local

PROFILE_RUN_ID=storage-append-only-copy-20260711T073430Z \
PROFILE_STORAGE_EXTENT_REPEAT=3 \
PROFILE_STORAGE_EXTENT_SIZES=1048576 \
PROFILE_STORAGE_EXTENT_WORKLOADS=test-large-copy-benchmark \
make profile-storage-extent-size-matrix-local
```

| workload | path | throughput mean | stdev | elapsed mean | segment preparation | physical inserts | WAL bytes mean | max RSS mean |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 64 MiB large-file | block | `46.39 MiB/s` | `1.44` | `1.381007s` | `0 us` | `16384 data_blocks` | `7411301` | `137665 KiB` |
| 64 MiB large-file | append-only 1 MiB extents | `94.55 MiB/s` | `5.61` | `0.679384s` | `14.33 us` | `64 data_extents` | `1026872` | `141652 KiB` |
| 64 MiB large-copy | block | `16.07 MiB/s` | `0.82` | `3.992818s` | `0 us` | `32768 data_blocks` | `13415473` | `137733 KiB` |
| 64 MiB large-copy | append-only 1 MiB extents | `12.14 MiB/s` | `0.81` | `5.291845s` | `18.67 us` | `128 data_extents` | `1910705` | `137676 KiB` |

The append-only large-file path remains roughly twice as fast as the block
path, writes 256 times fewer physical payload rows, and keeps each payload at
the configured 1 MiB bound. Compared with the preceding direct-segment sample
(`98.81 MiB/s`), two append-only runs remained near `97-100 MiB/s`, while one
noisy `86.75 MiB/s` run lowered the mean; mean WAL increased by about 15% from
the preceding extent sample but remained about seven times below the block
path.

Large-copy remains a negative result. The append-only destination transaction
does not remove the repeated source extent reads, so the workload is about 24%
slower than blocks despite low segment preparation cost and much lower WAL.
Keep extents opt-in and address range-oriented extent reads or direct object
adoption before considering a broader large-copy selection policy.

Artifacts:

- `artifacts/perf/42c5edf/lt7300-storage-append-only-core-20260711T073350Z-storage-extent-summary.md`
- `artifacts/perf/42c5edf/lt7300-storage-append-only-copy-20260711T073430Z-storage-extent-summary.md`

## 2026-07-05 Local/QNAP COPY Buffer Matrix

Collected from commit `a3076e1` (`FOD 3.2.1: add copy-buffer matrix compare target`).

Command:

```bash
PROFILE_RUN_ID=copy-buffer-matrix-20260705T171509Z \
PROFILE_COPY_BUFFER_INCLUDE_QNAP=auto \
make profile-data-blocks-copy-buffer-matrix-compare
```

QNAP was reachable in this run. The smoke probe started the QNAP Docker PostgreSQL container through `DOCKER_HOST=tcp://192.168.1.11:2376` and `DOCKER_TLS_VERIFY=1`, then confirmed PostgreSQL readiness.

| backend | buffer | elapsed | throughput | WAL bytes | WAL records | WAL write/sync | data_blocks ins/upd/del/dead | data_blocks relation growth | COPY exec ms | data_blocks merge exec ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| local | default | `3.481625s` | `18.38 MiB/s` | `13042036` | `165637` | `239/20` | `32768/0/0/0` | `3833856` | `1091.109` | `772.312` |
| local | `262144` | `3.500098s` | `18.29 MiB/s` | `12952365` | `166376` | `93/19` | `32768/0/0/0` | `3833856` | `1089.599` | `731.900` |
| local | `1048576` | `3.553295s` | `18.01 MiB/s` | `12845520` | `165625` | `108/18` | `32768/0/0/0` | `3833856` | `1143.632` | `771.468` |
| local | `4194304` | `3.521974s` | `18.17 MiB/s` | `12851532` | `165632` | `113/18` | `32768/0/0/0` | `3833856` | `1163.857` | `791.817` |
| QNAP | default | `25.970985s` | `2.46 MiB/s` | `13151107` | `166391` | `55/55` | `32768/0/0/0` | `3833856` | `4512.280` | `9660.060` |
| QNAP | `262144` | `23.679487s` | `2.70 MiB/s` | `12856017` | `165664` | `40/40` | `32768/0/0/0` | `3833856` | `6479.871` | `6586.574` |
| QNAP | `1048576` | `22.746579s` | `2.81 MiB/s` | `12946217` | `165743` | `45/45` | `32768/0/0/0` | `3833856` | `4432.111` | `6303.256` |
| QNAP | `4194304` | `20.099939s` | `3.18 MiB/s` | `12941251` | `167091` | `40/40` | `32768/0/0/0` | `3842048` | `6762.207` | `5819.007` |

Artifacts:

- `artifacts/perf/a3076e1/lt7300-copy-buffer-matrix-20260705T171509Z-local-buffer-default`
- `artifacts/perf/a3076e1/lt7300-copy-buffer-matrix-20260705T171509Z-local-buffer-262144`
- `artifacts/perf/a3076e1/lt7300-copy-buffer-matrix-20260705T171509Z-local-buffer-1048576`
- `artifacts/perf/a3076e1/lt7300-copy-buffer-matrix-20260705T171509Z-local-buffer-4194304`
- `artifacts/perf/a3076e1/lt7300-copy-buffer-matrix-20260705T171509Z-qnap-buffer-default`
- `artifacts/perf/a3076e1/lt7300-copy-buffer-matrix-20260705T171509Z-qnap-buffer-262144`
- `artifacts/perf/a3076e1/lt7300-copy-buffer-matrix-20260705T171509Z-qnap-buffer-1048576`
- `artifacts/perf/a3076e1/lt7300-copy-buffer-matrix-20260705T171509Z-qnap-buffer-4194304`

Conclusion: the local backend stays in a narrow `18.01-18.38 MiB/s` band, so this run does not justify changing the local default send buffer. QNAP improves across this single matrix and `4194304` bytes is the best QNAP sample here (`3.18 MiB/s`, about `29%` over QNAP default), but one run is not enough to change the runtime default. The data_blocks path stayed insert-only in all eight runs: `32768` inserts, `0` updates, `0` deletes, and `0` new dead tuples.

## 2026-07-09 Local COPY Buffer Repeatability Smoke

Collected from commit `bad53cc` (`FOD 3.2.1: record copy-buffer matrix baseline`).

Command:

```bash
repeat=3
for i in $(seq 1 "$repeat"); do
  PROFILE_RUN_ID=copy-buffer-local-repeat-20260709T085827Z-run$i \
  PROFILE_COPY_BUFFER_INCLUDE_QNAP=0 \
  PROFILE_COPY_BUFFER_SIZES='default 4194304' \
  make profile-data-blocks-copy-buffer-matrix-compare
done
```

Observed local throughput across three repeats:

| run | default | `4194304` |
| --- | ---: | ---: |
| 1 | `14.55 MiB/s` | `18.17 MiB/s` |
| 2 | `18.43 MiB/s` | `17.48 MiB/s` |
| 3 | `17.42 MiB/s` | `17.76 MiB/s` |

Conclusion: the local repeat sample is still mixed and does not show a stable winner. The `4194304` buffer is slightly ahead in two of three repeats, but the spread overlaps the default result, so this is not enough to change the default or claim a durable local throughput gain. QNAP remains the deciding signal, and it was not reachable in the repeated attempt from this session.

## 2026-07-05 COPY Buffer Compare Target Smoke

Collected from commit `ef0e782` (`FOD 3.2.1: add indexer parallel smoke to full suite`) before committing the compare-target change.

Command:

```bash
PROFILE_RUN_ID=copy-buffer-compare-smoke-20260705T171117Z \
PROFILE_COPY_BUFFER_INCLUDE_QNAP=0 \
PROFILE_COPY_BUFFER_SIZES=default \
PROFILE_COPY_BUFFER_BLOCK_SIZE=64k \
PROFILE_COPY_BUFFER_BLOCK_COUNT=1 \
make profile-data-blocks-copy-buffer-matrix-compare
```

Result:

- status: `0`
- mode: local only, QNAP skipped intentionally with `PROFILE_COPY_BUFFER_INCLUDE_QNAP=0`
- workload: `test-large-copy-benchmark`
- bytes: `65536`
- elapsed: `0.014335s`
- throughput: `4.36 MiB/s`
- `data_blocks_n_tup_ins_delta`: `32`
- `data_blocks_n_tup_upd_delta`: `0`
- `data_blocks_n_tup_del_delta`: `0`
- `data_blocks_n_dead_tup_delta`: `0`
- `wal_bytes_delta`: `223978`
- artifact directory: `artifacts/perf/ef0e782/lt7300-copy-buffer-compare-smoke-20260705T171117Z-local-buffer-default`

Conclusion: this is a target-smoke validation only. It proves the compare entry point runs the local matrix path, captures DML/WAL/top-IO artifacts, and respects local-only QNAP skipping, but the tiny `64 KiB` workload is not a production COPY-buffer baseline.

## 2026-07-04 FOD Indexer Allocation Profile Harness Smoke

Collected from commit `deabdf6` (`FOD 3.2.1: add metadata lookup profiling report`) before committing the profiling-helper change.

Command:

```bash
PROFILE_RUN_ID=indexer-alloc-smoke-20260704T083132Z \
make profile-indexer-alloc PROFILE_INDEXER_ALLOC_TOOL=time PROFILE_INDEXER_ARGS='--help'
```

Result:

- status: `0`
- tool: `/usr/bin/time -v`
- workload: `fod-indexer --help`
- maximum resident set size: `8504 kB`
- minor page faults: `875`
- file system outputs: `8`
- artifact: `artifacts/perf/deabdf6/lt7300-indexer-alloc-smoke-20260704T083132Z/indexer_alloc.txt`

Conclusion: this is only a harness smoke for the allocation profiling target. It proves metadata, stdout/stderr, exit status, and RSS are captured, but it is not a representative allocation profile for `scan` or `hash`.

## 2026-07-04 FOD Indexer Synthetic Allocation Baseline

Collected from commit `8d90a6e` (`FOD 3.2.1: add indexer allocation profiling helper`).

Setup:

```bash
RUN_ID=indexer-alloc-synthetic-20260704T104340Z
SOURCE=profile_alloc_20260704T104340Z
SRC=/tmp/fod-indexer-alloc-src-indexer-alloc-synthetic-20260704T104340Z
```

The source tree contained 200 small `.txt` files, 50 small `.jpg` files, and 30 cache/hidden files intended to exercise the indexer path filters. The temporary source was removed from PostgreSQL and `/tmp` after profiling.

Commands:

```bash
make init
make profile-indexer-alloc PROFILE_RUN_ID="$RUN_ID" PROFILE_CAPTURE_LABEL=source-add PROFILE_INDEXER_ALLOC_TOOL=time PROFILE_INDEXER_ARGS="source add --name $SOURCE --path $SRC --kind local"
make profile-indexer-alloc PROFILE_RUN_ID="$RUN_ID" PROFILE_CAPTURE_LABEL=scan PROFILE_INDEXER_ALLOC_TOOL=time PROFILE_INDEXER_ARGS="scan --source $SOURCE"
make profile-indexer-alloc PROFILE_RUN_ID="$RUN_ID" PROFILE_CAPTURE_LABEL=hash PROFILE_INDEXER_ALLOC_TOOL=time PROFILE_INDEXER_ARGS="hash --source $SOURCE --candidates-only"
```

| phase | status | files | elapsed | max RSS | minor faults | file outputs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `source-add` | `0` | n/a | `0.03s` | `12820 kB` | `1031` | `8` |
| `scan` | `0` | `250 scanned / 250 ok` | `0.10s` | `13104 kB` | `1063` | `16` |
| `hash --candidates-only` | `0` | `250 partial hashed` | `0.05s` | `13628 kB` | `1187` | `16` |

Artifacts:

- `artifacts/perf/8d90a6e/lt7300-indexer-alloc-synthetic-20260704T104340Z/indexer_alloc-source-add.txt`
- `artifacts/perf/8d90a6e/lt7300-indexer-alloc-synthetic-20260704T104340Z/indexer_alloc-scan.txt`
- `artifacts/perf/8d90a6e/lt7300-indexer-alloc-synthetic-20260704T104340Z/indexer_alloc-hash.txt`

Conclusion: the small local allocation baseline does not justify changing `rust_indexer` buffer reuse. `scan` stages rows in memory and `hash` uses bounded read buffers; the observed process RSS stayed below `14 MiB`. Larger real-source `heaptrack` or `massif` captures are still the right trigger for any future indexer memory optimization.

## 2026-07-05 FUSE Sequential I/O Profile Harness

Collected from commit `d55b555` (`FOD 3.2.1: record indexer allocation baseline`) before committing the FUSE profile-helper change.

Commands:

```bash
PROFILE_RUN_ID=fuse-seq-20260705T163713Z make profile-fuse-sequential-io
PROFILE_RUN_ID=fuse-perf-20260705T163728Z make profile-fuse-sudo-perf-stat PROFILE_FUSE_WORKLOAD=test-fio-sequential-io-strace
```

Workload: local Docker PostgreSQL, `test-fio-sequential-io-strace`, `FIO_FILE_SIZE=64k`, `FOD_PROFILE_IO=1`, `FOD_FOPEN_DIRECT_IO=1`, and `FOD_STRACE=1`.

| path | write BW | read BW | fuse_read_total_us | fuse_write_total_us | strace total seconds | strace total calls |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| block | `7111 KiB/s` | `2065 KiB/s` | `27763` | `38954` | `0.050281` | `2932` |
| extent | `7111 KiB/s` | `2207 KiB/s` | `27194` | `25915` | `0.062811` | `2668` |

Dominant strace entries stayed in the expected short-smoke shape:

- block: `wait4`, `futex`, `restart_syscall`, then smaller `write`, `sendto`, `poll`, and `recvfrom`.
- extent: `wait4`, `futex`, `restart_syscall`, then smaller `write`, `sendto`, `poll`, and `recvfrom`.

The sudo `perf stat` wrapper also completed with `workload_status=0` and `perf_status=0`:

- `cpu-clock`: `42,387,038,738`
- `context-switches`: `85,401`
- `page-faults`: `122,290`
- elapsed: `5.298488771s`

Artifacts:

- `artifacts/perf/d55b555/lt7300-fuse-seq-20260705T163713Z/fuse-test-fio-sequential-io-strace.txt`
- `artifacts/perf/d55b555/lt7300-fuse-perf-20260705T163728Z/perf-stat-system-test-fio-sequential-io-strace-fuse.txt`

Conclusion: the new FUSE profile wrappers work and can collect both `FOD_PROFILE_IO`/strace and sudo `perf stat` counters while the workload runs normally. This short 64 KiB smoke does not justify changing FUSE cache, timeout, or `max_background`; use larger sequential/mixed workloads through the same targets before tuning. The perf-wrapped run also printed a missing extent marker while exiting successfully, so the test-side marker handling needs a separate review before using that message as a hard signal.

## 2026-07-04 Data Blocks COPY Buffer Matrix And Fillfactor Clone

Collected from commit `adeaa35` (`FOD 3.2.1: add storage DML and statement IO profiling`). The working tree also contained uncommitted profiling-target additions for this run; those additions only orchestrate diagnostics and do not change runtime SQL.

### Local COPY Buffer Matrix

Command:

```bash
make profile-data-blocks-copy-buffer-matrix \
  PROFILE_RUN_ID=copy-buffer-matrix-20260704T081308Z \
  PROFILE_COPY_BUFFER_SIZES='default 262144 1048576 4194304'
```

Workload: local Docker PostgreSQL, real `test-large-copy-benchmark`, 64 MiB payload (`4M * 16`), with storage DML delta, WAL delta, `pg_stat_statements` IO/WAL, and bloat snapshots around each buffer run.

| buffer bytes | elapsed_s | MiB/s | COPY sends | client COPY send seconds | server COPY total_exec_ms | data_blocks merge total_exec_ms | wal_bytes_delta | wal_buffers_full_delta | data_blocks inserts | data_blocks updates/dead |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| default | `3.882610` | `16.48` | `130` | `0.045381` | `1243.342` | `1255.802` | `12881058` | `260` | `32768` | `0/0` |
| `262144` | `3.779636` | `16.93` | `512` | `0.039839` | `1197.572` | `1248.430` | `13059541` | `124` | `32768` | `0/0` |
| `1048576` | `3.782537` | `16.92` | `130` | `0.043842` | `1234.466` | `1175.511` | `12858595` | `130` | `32768` | `0/0` |
| `4194304` | `3.870289` | `16.54` | `34` | `0.051633` | `1213.832` | `1202.329` | `12864209` | `204` | `32768` | `0/0` |

Artifacts:

- `artifacts/perf/adeaa35/lt7300-copy-buffer-matrix-20260704T081308Z-local-buffer-default`
- `artifacts/perf/adeaa35/lt7300-copy-buffer-matrix-20260704T081308Z-local-buffer-262144`
- `artifacts/perf/adeaa35/lt7300-copy-buffer-matrix-20260704T081308Z-local-buffer-1048576`
- `artifacts/perf/adeaa35/lt7300-copy-buffer-matrix-20260704T081308Z-local-buffer-4194304`

Conclusions:

- The local spread is small. `262144` and `1048576` were fastest in this single pass, but not by enough to justify changing the default.
- The new DML delta confirms this large-copy path is insert-heavy in `data_blocks`: each run inserted `32768` rows and produced `0` `data_blocks` updates and `0` new dead tuples in the measured window.
- The per-statement report keeps separating client COPY send behavior from server-side work: fewer client sends at `4194304` did not produce a throughput win, so server-side COPY plus merge remains the main area to analyze.
- Keep `FOD_PERSIST_COPY_SEND_BUFFER_BYTES` as diagnostic. Do not change its default from this single local pass.

### QNAP COPY Buffer Matrix

Attempted command:

```bash
make qnap-smoke
```

Result: blocked before the matrix could run. The QNAP endpoint was unreachable from this host:

```text
192.168.1.11:2376 -> No route to host
192.168.1.11:5432 -> No route to host
```

The local route lookup still selected `wlo1`:

```text
192.168.1.11 dev wlo1 src 192.168.1.116
```

Conclusion: repeat `QNAP=1 make profile-data-blocks-copy-buffer-matrix` only after the laptop is on the correct network or LAN path and both Docker TLS `2376` and PostgreSQL `5432` are reachable.

### Fillfactor Clone EXPLAIN

Command:

```bash
make profile-pg-data-blocks-merge-fillfactor-explain \
  PROFILE_RUN_ID=data-blocks-fillfactor-20260704T081254Z \
  DATA_BLOCKS_EXPLAIN_FILLFACTORS='100 90 75'
```

Workload: temporary clone tables only, 16k staged rows, 4 KiB payload per row. The script checked real `fod.data_blocks` count before and after every run; it remained `16384 -> 16384`.

| heap fillfactor | fresh insert ms | identical conflict ms | changed conflict ms | changed updates | HOT updates | temp heap size | temp total size |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `100` | `244.091` | `412.155` | `357.399` | `16384` | `0` | `3240 kB` | `4920 kB` |
| `90` | `243.004` | `404.545` | `307.077` | `16384` | `1800` | `3400 kB` | `5064 kB` |
| `75` | `236.696` | `419.949` | `298.556` | `16384` | `5380` | `3592 kB` | `5224 kB` |

Artifacts:

- `artifacts/perf/adeaa35/lt7300-data-blocks-fillfactor-20260704T081254Z/pg_data_blocks_merge_fillfactor_100-fillfactor-100.txt`
- `artifacts/perf/adeaa35/lt7300-data-blocks-fillfactor-20260704T081254Z/pg_data_blocks_merge_fillfactor_90-fillfactor-90.txt`
- `artifacts/perf/adeaa35/lt7300-data-blocks-fillfactor-20260704T081254Z/pg_data_blocks_merge_fillfactor_75-fillfactor-75.txt`

Conclusions:

- The safe clone confirms fillfactor can make changed-payload conflict updates HOT-capable on a temp clone, but lower fillfactor grows the heap/total relation size.
- This does not justify changing real `fod.data_blocks` yet. The current runtime already avoids changed-payload full-overwrite non-HOT updates through data-object swap, so fillfactor is only a future option for workloads that still perform conflict updates.
- Any real-table fillfactor change needs a separate migration design, repeated local/QNAP runs, and correctness tests before it is considered.

### Metadata Lookup Review

Collected from commit `48d132a` with the new metadata-only `pg_stat_statements` filter added in the working tree.

Command:

```bash
make profile-pg-metadata-top \
  PROFILE_RUN_ID=metadata-top-smoke \
  PROFILE_CAPTURE_LABEL=smoke
```

The report used the current PostgreSQL statement statistics after the local COPY-buffer matrix above.

| category | calls | total_exec_ms | mean_exec_ms | rows | wal_bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| `path_walk` | `2076` | `92.234` | `0.044` | `2067` | `0` |
| `child_lookup` | `2067` | `85.318` | `0.041` | `2062` | `0` |
| `file_attrs` | `1033` | `20.072` | `0.019` | `1033` | `0` |
| `special_file_metadata` | `1037` | `13.350` | `0.013` | `0` | `0` |
| `xattr_value` | `1026` | `13.174` | `0.013` | `0` | `0` |

Conclusion:

- The high-call metadata path remains visible but secondary. The main lookup classes already map to prepared statements in `rust_hotpath/src/pg.rs`, so the next metadata change should be driven by a workload where this report becomes dominant, not by the current large-copy path.

## 2026-07-04 Data Blocks Repeated Full-Overwrite Cleanup Profile

Collected from commit `60658e878c136728df023d08f9c88a88176cb824` (`FOD 3.2.1: fix deferred data object GC script`) with the FOD version at `3.2.1`.

Workload: local Docker PostgreSQL, `make reset` before each policy, then `PROFILE_DATA_BLOCKS_SWAP_REPEAT=5 make profile-data-blocks-swap-repeat-dml` over one seeded 64 MiB file.

Artifact summary: `docs/performance-data-blocks-swap-repeat-profile-2026-07-04.md`.

| Metric | `immediate` | `deferred` |
| --- | ---: | ---: |
| mean overwrite elapsed_s | `1.435899` | `1.405703` |
| mean throughput_mib_s | `44.59` | `45.53` |
| wal_bytes_delta | `43069493` | `37976180` |
| wal_records_delta | `499295` | `415428` |
| `data_blocks_n_tup_ins_delta` | `81920` | `81920` |
| `data_blocks_n_tup_del_delta` | `81920` | `0` |
| `data_blocks_n_dead_tup_delta` | `49152` | `0` |
| `data_blocks_n_live_tup_delta` | `0` | `81920` |
| `data_blocks_relation_size_bytes_delta` | `10174464` | `15605760` |

Deferred GC on the same run removed `6` unreferenced data objects and `81920` `data_blocks` rows, generating `4429524` WAL bytes and `81920` new `data_blocks` dead tuples. After GC, the consistency query returned `0` unreferenced data objects, `0` blocks without objects, and `0` files without objects.

Interpretation:

- `deferred` shifts delete/dead-tuple cost out of the overwrite transaction; it does not eliminate that cost.
- On this local five-run smoke, deferred hot-path WAL was lower by about `5.09 MB`, but GC added about `4.43 MB`, so combined WAL was only marginally lower than immediate cleanup.
- Keep `data_object_swap_cleanup = immediate` as the default. Use `deferred` only as an opt-in policy when shorter write transactions are worth temporary relation growth and a scheduled object-GC pass.

## 2026-07-03 Data Blocks Full-Overwrite Data-Object Swap Profile

Collected from commit `0eb2d0efb2b62bb38b6c1e0be16a16f8a0b44524` (`FOD 3.2.1: swap data objects for full block overwrites`) with the FOD version at `3.2.1`.

Workload: `make profile-data-blocks-conflict-dml` on local Docker PostgreSQL. The target first seeds a 64 MiB file, then snapshots DML/WAL around overwriting that same logical file with a different 64 MiB payload.

Artifact summary: `docs/performance-data-blocks-swap-profile-2026-07-03.md`, run ID `data-blocks-swap-20260703T215237Z`.

| Metric | Value |
| --- | ---: |
| overwrite elapsed_s | `2.955563` |
| overwrite throughput_mib_s | `21.65` |
| COPY `fod_persist_block_stage` total_exec_ms | `1217.262` |
| `data_blocks` insert/merge total_exec_ms | `1244.614` |
| wal_bytes_delta | `7754478` |
| wal_records_delta | `99199` |
| `data_blocks_n_tup_ins_delta` | `16384` |
| `data_blocks_n_tup_upd_delta` | `0` |
| `data_blocks_n_tup_hot_upd_delta` | `0` |
| `data_blocks_non_hot_update_delta` | `0` |
| `data_blocks_n_tup_del_delta` | `16384` |
| `data_blocks_n_dead_tup_delta` | `33883` |
| `idx_data_blocks_object_order_idx_scan_delta` | `16385` |
| `idx_data_blocks_data_object_id_idx_tup_read_delta` | `16384` |
| `data_blocks_relation_size_bytes_delta` | `2318336` |
| `idx_data_blocks_object_order_relation_size_bytes_delta` | `376832` |

Interpretation:

- Full-overwrite data-object swap removes the changed-payload `data_blocks` conflict-update path from this profiled overwrite: `data_blocks_n_tup_upd_delta=0`, `data_blocks_non_hot_update_delta=0`.
- The remaining cost is now new-object insert plus old-object cleanup: `16384` inserts and `16384` deletes for a 64 MiB overwrite.
- `n_dead_tup` is approximate and was affected by an autoanalyze in the measured window, but the profile still shows the relevant direction: no heap rewrite updates, with insert/delete churn and cleanup now the next bottleneck.
- The read path now fetches block/extent data by joining through `files.data_object_id` in the same statement, avoiding a stale object-id lookup window during data-object swaps.
- A post-run consistency query returned `0` unreferenced data objects, `0` data blocks without objects, and `0` files without data objects.
- The next performance question is repeated full-overwrite bloat/WAL growth and whether delayed cleanup or object GC is better than immediate delete for production workloads.

## 2026-07-03 Data Blocks Unchanged Conflict Filter Profile

Collected from commit `76867aa765d9cee4406c522d37c3e0dd5ec812c8` (`FOD 3.2.1: skip unchanged data block conflict updates`) with the FOD version at `3.2.1`.

Workload: `make profile-data-blocks-conflict-noop-dml` on local Docker PostgreSQL. The target first seeds a 64 MiB file, then snapshots DML/WAL around overwriting that same logical file with the same 64 MiB payload.

Artifact summary: `docs/performance-data-blocks-conflict-noop-profile-2026-07-03.md`, run ID `data-blocks-conflict-noop-20260703T140759Z`.

| Metric | Value |
| --- | ---: |
| same-payload overwrite elapsed_s | `2.585200` |
| same-payload overwrite throughput_mib_s | `24.76` |
| wal_bytes_delta | `1266` |
| wal_records_delta | `16` |
| `data_blocks_n_tup_ins_delta` | `0` |
| `data_blocks_n_tup_upd_delta` | `0` |
| `data_blocks_n_tup_hot_upd_delta` | `0` |
| `data_blocks_non_hot_update_delta` | `0` |
| `data_blocks_n_dead_tup_delta` | `0` |
| `idx_data_blocks_object_order_idx_scan_delta` | `922` |
| `idx_data_blocks_object_order_idx_tup_read_delta` | `32768` |
| `idx_data_blocks_object_order_idx_tup_fetch_delta` | `32768` |

Temp-table merge reproducer on the same commit, run ID `data-blocks-merge-filter-explain-20260703T140901Z`:

| Reproducer step | Rows | Execution Time |
| --- | ---: | ---: |
| fresh insert | `16384 inserted, 0 conflicts` | `230.725 ms` |
| identical conflict | `16384 conflicts, 16384 removed by conflict filter` | `378.997 ms` |
| changed conflict | `16384 conflicts, 16384 updated` | `319.994 ms` |

Interpretation:

- The end-to-end same-payload overwrite produced no `data_blocks` inserts, updates, deletes, dead tuples, or relation-size growth.
- The temp-table reproducer confirms the SQL filter itself: identical conflicts are removed by the `ON CONFLICT ... WHERE` filter and do not dirty/write target pages, while changed payloads still update rows.
- The same-payload path still pays conflict lookup and payload comparison cost when it reaches SQL; optimizing changed-payload full overwrites remains a separate problem.

## 2026-07-03 Data Blocks Conflict Update Profile

Collected from commit `19696742e82220e2c46355d55078be463759ee65` (`FOD 3.2.1: add data block conflict update benchmark`) with the FOD version at `3.2.1`.

Workload: `make profile-data-blocks-conflict-dml` on local Docker PostgreSQL. The target first seeds a 64 MiB file, then snapshots DML/WAL only around overwriting that same logical file with a different 64 MiB payload.

Artifact summary: `docs/performance-data-blocks-conflict-profile-2026-07-03.md`, run ID `data-blocks-conflict-20260703T135637Z`.

| Metric | Value |
| --- | ---: |
| overwrite elapsed_s | `1.169478` |
| overwrite throughput_mib_s | `54.73` |
| COPY `fod_persist_block_stage` total_exec_ms | `534.021` |
| `data_blocks` conflict merge total_exec_ms | `397.522` |
| wal_bytes_delta | `8493118` |
| wal_records_delta | `99680` |
| `data_blocks_n_tup_ins_delta` | `0` |
| `data_blocks_n_tup_upd_delta` | `16384` |
| `data_blocks_n_tup_hot_upd_delta` | `0` |
| `data_blocks_non_hot_update_delta` | `16384` |
| `data_blocks_n_dead_tup_delta` | `16384` |
| `idx_data_blocks_object_order_idx_scan_delta` | `16385` |
| `data_blocks_relation_size_bytes_delta` | `2310144` |
| `idx_data_blocks_object_order_relation_size_bytes_delta` | `368640` |

Interpretation:

- This run isolates the real conflict-update phase; it does not include the seed insert in the DML delta.
- The current overwrite path is entirely non-HOT for `data_blocks`: `16384` updates, `0` HOT updates, and `16384` new dead tuples.
- Future SQL work should focus on reducing repeated row rewrites or avoiding conflict updates when a higher-level data-object swap is safe.

## 2026-07-03 Data Blocks DML Delta Profile

Collected from commit `c5d7f241fc98fd5ac410e1ad89d63b7a2d23cd50` (`FOD 3.2.1: add data block DML delta profiling`) with the FOD version at `3.2.1`.

Workload: `FOD_PROFILE_IO=1 make test-large-copy-benchmark` on local Docker PostgreSQL with a 64 MiB payload.

Artifact summary: `docs/performance-data-blocks-dml-profile-2026-07-03.md`, run ID `data-blocks-dml-20260703T134344Z`.

| Metric | Value |
| --- | ---: |
| elapsed_s | `3.798770` |
| throughput_mib_s | `16.85` |
| COPY `fod_persist_block_stage` total_exec_ms | `1221.565` |
| `data_blocks` merge total_exec_ms | `1089.486` |
| wal_bytes_delta | `13045983` |
| wal_records_delta | `165633` |
| `data_blocks_n_tup_ins_delta` | `32768` |
| `data_blocks_n_tup_upd_delta` | `0` |
| `data_blocks_n_tup_hot_upd_delta` | `0` |
| `data_blocks_non_hot_update_delta` | `0` |
| `data_blocks_n_tup_del_delta` | `0` |
| `data_blocks_n_dead_tup_delta` | `0` |
| `idx_data_blocks_object_order_idx_scan_delta` | `32768` |
| `data_blocks_relation_size_bytes_delta` | `3833856` |
| `idx_data_blocks_object_order_relation_size_bytes_delta` | `737280` |

Interpretation:

- This large-copy run is insert-heavy: it inserted `32768` `data_blocks` rows and did not exercise a real conflict-update rewrite.
- HOT eligibility cannot be judged from this workload because `data_blocks_n_tup_upd_delta=0`; the HOT ratio is therefore `n/a`, not `0%`.
- `idx_data_blocks_object_order` still saw `32768` scans, so conflict lookup cost is measurable even when the outcome is insert rather than update.
- The next SQL evidence should use a targeted overwrite/conflict workload if the question is heap rewrite/HOT behavior under `ON CONFLICT DO UPDATE`.

## 2026-07-01 SQL Payload Persistence Send Batching

Collected from a working tree based on commit `024547a` (`FOD 3.2.1: add safe sudo profiling helpers`) with the FOD version at `3.2.1`.

Workload: `make test-large-copy-benchmark` on local Docker PostgreSQL with a 64 MiB payload.

Change under test: batch binary `COPY` rows for `fod_persist_block_stage` into 1 MiB client-side send buffers. The SQL shape, staging table, transaction, and `INSERT INTO data_blocks ... ON CONFLICT ...` merge are unchanged.

| Run | elapsed_s | throughput_mib_s | COPY total ms | data_blocks merge total ms |
| --- | ---: | ---: | ---: | ---: |
| before | `3.523786` | `18.16` | `1224.010` | `771.421` |
| after SQL capture | `3.766381` | `16.99` | `1284.245` | `912.490` |
| after repeat 1 | `3.818472` | `16.76` | n/a | n/a |
| after repeat 2 | `3.557921` | `17.99` | n/a | n/a |
| after repeat 3 | `3.542274` | `18.07` | n/a | n/a |
| after sudo `perf stat` workload | `3.494957` | `18.31` | n/a | n/a |

Interpretation:

- The patch is low-risk transport hardening, not a proven major throughput win on this host.
- End-to-end timing stayed inside the observed local variance.
- `COPY fod_persist_block_stage` plus the `data_blocks` merge remains the dominant SQL area for future optimization.
- The next useful measurement is a filtered profile or preserved `FOD_PROFILE_IO` aggregate showing actual `PQputCopyData` send counts and sizes during successful Rust FUSE benchmarks.

## 2026-06-28 PostgreSQL Planner Preset Sweep

Collected from working tree based on commit `1fee771`.

This run used `make postgres-benchmarks-planner-preset`, which applies the shared planner/autovacuum profile to both local Docker and QNAP before running the same comparison suite on each backend.

### WAL Pressure Benchmark

Observed with `PG_WAL_PRESSURE_COUNT=128`.

| Backend | Mode | Checkpoint | Elapsed | Throughput | wal_bytes | wal_records | checkpoints_req | checkpoints_timed | checkpoint_write_time | checkpoint_sync_time | buffers_checkpoint | buffers_backend |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| local | base | `0` | `5.698s` | `11.23 MiB/s` | `8114698` | `91060` | `0` | `0` | `0.0` | `0.0` | `0` | `0` |
| qnap | base | `0` | `86.606s` | `0.74 MiB/s` | `9485772` | `103763` | `0` | `0` | `0.0` | `0.0` | `0` | `0` |
| local | checkpoint | `1` | `4.792s` | `13.35 MiB/s` | `7603229` | `87849` | `1` | `0` | `7.0` | `38.0` | `1022` | `129` |
| qnap | checkpoint | `1` | `62.008s` | `1.03 MiB/s` | `8975106` | `104120` | `1` | `0` | `365.0` | `503.0` | `984` | `826` |

Notes:

- The shared planner/autovacuum preset does not erase the backend gap; QNAP is still dominated by remote latency and checkpoint cost.
- The forced-checkpoint variant is still much cheaper locally than remotely, so checkpoint tuning on QNAP remains a separate concern from planner tuning.

### Connection Churn Benchmark

Observed on the same run of `make postgres-benchmarks-planner-preset`.

| Backend | Connections | Elapsed | Connect avg | Connect p95 | Query avg | Query p95 |
| --- | --- | --- | --- | --- | --- | --- |
| local | `100` | `1.076s` | `10.159 ms` | `14.932 ms` | `0.572 ms` | `0.685 ms` |
| qnap | `100` | `8.937s` | `81.769 ms` | `193.997 ms` | `7.490 ms` | `19.837 ms` |

Notes:

- Planner/autovacuum changes did not materially reduce the backend split on this workload.
- QNAP connection churn is still roughly an order of magnitude slower than local Docker on this run, so network and remote-docker overhead remain the first thing to account for.

## 2026-06-28 Local PostgreSQL Timeout Sweep

Collected from working tree based on commit `e66e66c`.

This run stayed local-only and used `PG_WAL_PRESSURE_COUNT=10000` with `PG_WAL_PRESSURE_BLOCK_SIZE=512k` so the workload would run long enough to cross the checkpoint boundary on the local backend. It is the first pass in this thread that really separated the 5-minute timeout effect from the later WAL-request effect.

### WAL Pressure Benchmark

Observed with `PG_WAL_PRESSURE_COUNT=10000` and `PG_WAL_PRESSURE_BLOCK_SIZE=512k`.

| Profile | Elapsed | Throughput | wal_bytes | wal_records | checkpoints_req | checkpoints_timed | buffers_checkpoint | buffers_backend | activity_total_peak | activity_active_peak |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| baseline | `459.581s` | `10.88 MiB/s` | `670184101` | `8077820` | `0` | `1` | `1372` | `26061` | `6` | `0` |
| `max_wal_size=8GB` | `437.092s` | `11.44 MiB/s` | `950195482` | `9382337` | `0` | `1` | `6268` | `9397` | `6` | `0` |
| `checkpoint_timeout=15min` | `434.652s` | `11.50 MiB/s` | `962282038` | `9391764` | `1` | `0` | `5868` | `12049` | `6` | `0` |
| `checkpoint_timeout=30min` | `426.220s` | `11.73 MiB/s` | `948620638` | `9370960` | `1` | `0` | `3893` | `6773` | `6` | `1` |

`pg_stat_user_tables` delta:

| Profile | n_tup_ins | n_tup_upd | n_tup_del | n_tup_hot_upd | seq_scan | idx_scan |
| --- | --- | --- | --- | --- | --- | --- |
| baseline | `1308466` | `50711` | `514` | `30010` | `274341` | `2992156` |
| `max_wal_size=8GB` | `1280002` | `60176` | `1280002` | `29898` | `342324` | `3211982` |
| `checkpoint_timeout=15min` | `1280002` | `60169` | `1280002` | `29992` | `342309` | `3211951` |
| `checkpoint_timeout=30min` | `1280002` | `60181` | `1280002` | `30013` | `342277` | `3211916` |

Notes:

- The default 5-minute timeout still produced a timed checkpoint, even though the run stayed under 1 GB of WAL.
- `max_wal_size=8GB` alone did not remove that timed checkpoint, so timeout was still the dominant trigger at this workload size.
- Once `checkpoint_timeout` moved to 15 or 30 minutes, the timed checkpoint disappeared and the checkpoint switched to the requested path instead.
- `pg_stat_activity` stayed flat at six total backend rows; the sampler only caught 0 to 1 active sessions, so this workload is still I/O and checkpoint dominated rather than connection churn dominated.
- The local-only long smoke is the first run here that makes the timeout effect visible without QNAP noise, so it is the better baseline for the next PostgreSQL tuning pass.

## 2026-06-28 Local PostgreSQL Max WAL Sweep

Collected from working tree based on commit `be642a6`.

This run stayed local-only and reused `PG_WAL_PRESSURE_COUNT=15000`, `PG_WAL_PRESSURE_BLOCK_SIZE=512k`, and `POSTGRES_CHECKPOINT_TIMEOUT=30min`. It compares the current default `max_wal_size` against `8GB` on the same long WAL-pressure workload so the size-based checkpoint threshold is the only major moving part.

### WAL Pressure Benchmark

Observed with `PG_WAL_PRESSURE_COUNT=15000` and `PG_WAL_PRESSURE_BLOCK_SIZE=512k`.

| Profile | Elapsed | Throughput | wal_bytes | wal_records | checkpoints_req | checkpoints_timed | buffers_checkpoint | buffers_backend | activity_total_peak | activity_active_peak |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| current max_wal_size | `644.558s` | `11.64 MiB/s` | `1291798464` | `13423472` | `2` | `0` | `10034` | `11809` | `6` | `1` |
| `max_wal_size=8GB` | `655.934s` | `11.43 MiB/s` | `1422527425` | `14081659` | `0` | `0` | `0` | `29517` | `6` | `0` |

`pg_stat_user_tables` delta:

| Profile | n_tup_ins | n_tup_upd | n_tup_del | seq_scan | idx_scan |
| --- | --- | --- | --- | --- | --- |
| current max_wal_size | `1935002` | `85256` | `1280002` | `477608` | `4702022` |
| `max_wal_size=8GB` | `1920002` | `90283` | `1920002` | `513492` | `4817928` |

Notes:

- The current 1 GB-style cap still forced two requested checkpoints on this workload even with `checkpoint_timeout=30min`.
- `max_wal_size=8GB` removed both requested and timed checkpoints entirely on the same run shape.
- Throughput stayed in the same band, with the larger WAL cap slightly slower on this sample, so the main benefit here is checkpoint-shape reduction rather than raw speed.
- `pg_stat_activity` stayed flat at six total backend rows in both profiles, which keeps this workload squarely in the I/O/checkpoint bucket rather than the connection-churn bucket.
- This is the first local-only run in the thread that isolates the size cap itself after the timeout was already relaxed.

## 2026-06-27 PostgreSQL Local Planner Preset Refresh

Collected from working tree based on commit `790feab`.

This historical run used `make postgres-benchmarks-local-planner-preset`, which set `POSTGRES_SHARED_BUFFERS=512MB`, `POSTGRES_RANDOM_PAGE_COST=1.1`, `POSTGRES_EFFECTIVE_CACHE_SIZE=4GB`, `POSTGRES_MAINTENANCE_WORK_MEM=512MB`, `POSTGRES_AUTOVACUUM_MAX_WORKERS=3`, and `POSTGRES_AUTOVACUUM_WORK_MEM=256MB` before running the local-only PostgreSQL benchmark suite. The current shared preset is `make postgres-benchmarks-planner-preset`.

### WAL Pressure Benchmark

Observed with `make postgres-benchmarks-local-planner-preset` using `PG_WAL_PRESSURE_COUNT=128`.

| Backend | Mode | Files | Block size | Sync | Checkpoint | Elapsed | Throughput |
| --- | --- | --- | --- | --- | --- | --- | --- |
| local | base | `128` | `512k` | `1` | `0` | `5.841s` | `10.96 MiB/s` |

Stats delta:

- local base: `wal_records=102897`, `wal_fpi=114`, `wal_bytes=9120370`, `wal_write=518`, `wal_sync=517`, `buffers_alloc=589`

Notes:

- This preset is focused on the planner/autovacuum side, so the WAL-pressure smoke is best read as a local consistency check rather than a direct endorsement of those exact values.
- On this host, the local preset did not materially outperform the earlier WAL-tuned compare run on this workload.

### Connection Churn Benchmark

Observed on the same run of `make postgres-benchmarks-local-planner-preset`.

| Backend | Connections | Elapsed | Connect avg | Connect p95 | Query avg | Query p95 |
| --- | --- | --- | --- | --- | --- | --- |
| local | `100` | `0.843s` | `7.865 ms` | `8.356 ms` | `0.534 ms` | `0.648 ms` |

Notes:

- The local planner/autovacuum preset kept connection churn in the same general shape as the baseline run.
- This is the local-only preset baseline to compare against future planner and autovacuum tweaks.

## 2026-06-27 PostgreSQL WAL Preset Refresh

Collected from commit `c24daeb` (`main` at the time of the run).

This run used `make postgres-benchmarks-wal-preset`, which set `POSTGRES_MAX_WAL_SIZE=8GB`, `POSTGRES_CHECKPOINT_TIMEOUT=15min`, and `POSTGRES_WAL_COMPRESSION=pglz` before re-running the local and QNAP comparison suite.

### WAL Pressure Benchmark

Observed with `make postgres-benchmarks-wal-preset` using `PG_WAL_PRESSURE_COUNT=128`.

| Backend | Mode | Files | Block size | Sync | Checkpoint | Elapsed | Throughput |
| --- | --- | --- | --- | --- | --- | --- | --- |
| local | base | `128` | `512k` | `1` | `0` | `5.128s` | `12.48 MiB/s` |
| qnap | base | `128` | `512k` | `1` | `0` | `73.593s` | `0.87 MiB/s` |
| local | checkpoint | `128` | `512k` | `1` | `1` | `5.349s` | `11.97 MiB/s` |
| qnap | checkpoint | `128` | `512k` | `1` | `1` | `105.948s` | `0.60 MiB/s` |

Stats delta:

- local base: `wal_records=100623`, `wal_fpi=102`, `wal_bytes=8720371`, `wal_write=519`, `wal_sync=516`, `buffers_alloc=587`
- qnap base: `wal_records=103794`, `wal_fpi=114`, `wal_bytes=9091535`, `wal_write=669`, `wal_sync=669`, `buffers_alloc=770`
- local checkpoint: `CHECKPOINT elapsed_s=0.076`, `wal_records=97326`, `wal_fpi=3`, `wal_bytes=8362514`, `wal_write=503`, `wal_sync=502`, `checkpoints_req=1`, `checkpoint_write_time=20.0`, `checkpoint_sync_time=36.0`, `buffers_checkpoint=1001`, `buffers_backend=765`, `buffers_alloc=409`
- qnap checkpoint: `CHECKPOINT elapsed_s=10.924`, `wal_records=104514`, `wal_fpi=87`, `wal_bytes=9118605`, `wal_write=724`, `wal_sync=719`, `checkpoints_req=1`, `checkpoint_write_time=7330.0`, `checkpoint_sync_time=1729.0`, `buffers_checkpoint=977`, `buffers_backend=871`, `buffers_alloc=465`

Notes:

- The WAL preset did not materially change the local profile, but the QNAP backend stayed much slower on the same workload.
- The forced-checkpoint variant keeps exposing checkpoint cost as the dominant QNAP pain point.

### Connection Churn Benchmark

Observed on the same run of `make postgres-benchmarks-wal-preset`.

| Backend | Connections | Elapsed | Connect avg | Connect p95 | Query avg | Query p95 |
| --- | --- | --- | --- | --- | --- | --- |
| local | `100` | `0.897s` | `8.399 ms` | `9.778 ms` | `0.541 ms` | `0.697 ms` |
| qnap | `100` | `4.149s` | `37.134 ms` | `62.436 ms` | `4.269 ms` | `10.628 ms` |

Notes:

- The connection churn profile stayed in the same shape as the earlier QNAP runs: backend setup cost dominates the total round-trip time.
- This remains a good smoke for pool/session churn, not a raw throughput indicator.

## 2026-06-27 QNAP Benchmark Refresh

Collected from commit `4b20f6d` (`main` at the time of the run).

This run was collected after `make qnap-reset`, because the previous QNAP schema state had pending migrations and a schema-admin secret mismatch that blocked the benchmark init path.

### WAL Pressure Benchmark

Observed with `make postgres-benchmarks-qnap` using `PG_WAL_PRESSURE_COUNT=128`.

| Backend | Mode | Files | Block size | Sync | Checkpoint | Elapsed | Throughput |
| --- | --- | --- | --- | --- | --- | --- | --- |
| qnap | base | `128` | `512k` | `1` | `0` | `34.609s` | `1.85 MiB/s` |

Stats delta:

- qnap base: `wal_records=103955`, `wal_fpi=38`, `wal_bytes=9254210`, `wal_write=611`, `wal_sync=609`, `buffers_alloc=742`

Notes:

- This is a fresh post-reset baseline for the current QNAP stack.
- The QNAP backend remains significantly slower than local Docker on the same WAL-pressure workload.

### Connection Churn Benchmark

Observed with `make postgres-benchmarks-qnap` on the same QNAP backend.

| Backend | Connections | Elapsed | Connect avg | Connect p95 | Query avg | Query p95 |
| --- | --- | --- | --- | --- | --- | --- |
| qnap | `100` | `4.277s` | `37.631 ms` | `142.270 ms` | `5.061 ms` | `7.587 ms` |

Notes:

- This run is still best read as a pool/session churn smoke rather than a raw throughput benchmark.
- The QNAP connection setup cost is still the dominant part of the round-trip shape.

## 2026-06-27 PostgreSQL Benchmark Comparison Refresh

Collected from commit `4b20f6d` (`main` at the time of the run).

This compare run reused the fresh QNAP reset state and also re-ran the local Docker backend so the same workload can be compared side by side.

### WAL Pressure Benchmark

Observed with `make postgres-benchmarks-compare` using `PG_WAL_PRESSURE_COUNT=128`.

| Backend | Mode | Files | Block size | Sync | Checkpoint | Elapsed | Throughput |
| --- | --- | --- | --- | --- | --- | --- | --- |
| local | base | `128` | `512k` | `1` | `0` | `5.220s` | `12.26 MiB/s` |
| qnap | base | `128` | `512k` | `1` | `0` | `31.992s` | `2.00 MiB/s` |
| local | checkpoint | `128` | `512k` | `1` | `1` | `4.538s` | `14.10 MiB/s` |
| qnap | checkpoint | `128` | `512k` | `1` | `1` | `37.960s` | `1.69 MiB/s` |

Stats delta:

- local base: `wal_records=97271`, `wal_fpi=92`, `wal_bytes=8546442`, `wal_write=500`, `wal_sync=500`, `buffers_alloc=428`
- qnap base: `wal_records=104263`, `wal_fpi=0`, `wal_bytes=9004810`, `wal_write=612`, `wal_sync=603`, `buffers_alloc=458`
- local checkpoint: `CHECKPOINT elapsed_s=0.064`, `wal_records=91715`, `wal_fpi=2`, `wal_bytes=7878843`, `wal_write=472`, `wal_sync=470`, `checkpoints_req=1`, `checkpoint_write_time=7.0`, `checkpoint_sync_time=37.0`, `buffers_checkpoint=1030`, `buffers_backend=818`, `buffers_alloc=383`
- qnap checkpoint: `CHECKPOINT elapsed_s=1.915`, `wal_records=104311`, `wal_fpi=140`, `wal_bytes=9679221`, `wal_write=640`, `wal_sync=630`, `checkpoints_req=1`, `checkpoint_write_time=36818.0`, `checkpoint_sync_time=1409.0`, `buffers_checkpoint=1716`, `buffers_backend=1646`, `buffers_alloc=420`

Notes:

- The local backend stayed much faster than QNAP on the same workload.
- The QNAP checkpoint run is still the clearest sign that checkpoint and backend latency remain the dominant cost center there.

## 2026-06-27 PostgreSQL WAL Knob Short Smoke

Collected from working tree based on commit `a7504c6`.

This run used the same short `PG_WAL_PRESSURE_COUNT=128` smoke on both local Docker and QNAP, but restarted PostgreSQL between profiles so the tuning env vars actually applied to a fresh backend each time. The workload is intentionally short and is best read as a sanity check for WAL and session-shape changes, not as a checkpoint saturation test.

### WAL Pressure Benchmark

Observed with `PG_WAL_PRESSURE_COUNT=128`.

| Backend | Profile | Sync | Wal compression | Max WAL | Checkpoint timeout | Elapsed | Throughput | wal_bytes | wal_records | checkpoints_req | user_ins | activity_total_peak | activity_active_peak |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| local | baseline | `on` | `off` | current | current | `5.258s` | `12.17 MiB/s` | `10264094` | `110974` | `0` | `16578` | `6` | `1` |
| local | synchronous_commit_off | `off` | `off` | current | current | `5.663s` | `11.30 MiB/s` | `12092064` | `118989` | `0` | `16386` | `6` | `0` |
| local | wal_compression_on | `on` | `on` | current | current | `7.449s` | `8.59 MiB/s` | `10139189` | `118927` | `0` | `16386` | `6` | `1` |
| local | wal_compression_lz4 | `on` | `lz4` | current | current | `6.073s` | `10.54 MiB/s` | `10207445` | `118707` | `0` | `16386` | `6` | `0` |
| local | max_wal_size_8GB | `on` | `off` | `8GB` | current | `5.556s` | `11.52 MiB/s` | `12054971` | `118735` | `0` | `16386` | `6` | `0` |
| local | checkpoint_timeout_15min | `on` | `off` | current | `15min` | `4.892s` | `13.08 MiB/s` | `12032927` | `118678` | `0` | `16386` | `6` | `1` |
| local | checkpoint_timeout_30min | `on` | `off` | current | `30min` | `5.301s` | `12.07 MiB/s` | `12067647` | `118767` | `0` | `16386` | `6` | `1` |
| qnap | baseline | `on` | `off` | current | current | `4.642s` | `13.79 MiB/s` | `9339591` | `118641` | `0` | `16386` | `6` | `1` |
| qnap | synchronous_commit_off | `off` | `off` | current | current | `4.629s` | `13.83 MiB/s` | `9294928` | `118527` | `0` | `16386` | `6` | `0` |
| qnap | wal_compression_on | `on` | `on` | current | current | `5.019s` | `12.75 MiB/s` | `9340557` | `118572` | `0` | `16386` | `6` | `1` |
| qnap | wal_compression_lz4 | `on` | `lz4` | current | current | `5.308s` | `12.06 MiB/s` | `9290024` | `118542` | `0` | `16386` | `6` | `0` |
| qnap | max_wal_size_8GB | `on` | `off` | `8GB` | current | `4.784s` | `13.38 MiB/s` | `9330919` | `118655` | `0` | `16386` | `6` | `1` |
| qnap | checkpoint_timeout_15min | `on` | `off` | current | `15min` | `5.654s` | `11.32 MiB/s` | `9297310` | `118595` | `0` | `16386` | `6` | `1` |
| qnap | checkpoint_timeout_30min | `on` | `off` | current | `30min` | `5.117s` | `12.51 MiB/s` | `9348541` | `118644` | `0` | `16386` | `6` | `1` |

Notes:

- `pg_stat_bgwriter.checkpoints_req` stayed `0` in every row, so this short smoke did not actually reach checkpoint pressure.
- `pg_stat_activity` stayed at `6` total backend rows throughout the run, with only occasional active hits because the workload was too short for a stable activity peak.
- On local Docker, `synchronous_commit=off` was slower than the baseline and `wal_compression=on/lz4` shaved only a small amount from `wal_bytes` while also lowering throughput.
- On QNAP, the differences were small enough to stay inside short-run noise; none of the checkpoint-related knobs produced a useful signal in this smoke.

## 2026-06-26 PostgreSQL Benchmark Comparison

Collected from commit `1605384` (`FOD 3.1.1: add PostgreSQL benchmark compare wrappers`).

### WAL Pressure Benchmark

Observed with `make postgres-benchmarks-compare` using `PG_WAL_PRESSURE_COUNT=128`. The base run used `make test-postgresql-wal-pressure`; the checkpoint run used `make test-postgresql-wal-pressure-checkpoint`.

| Backend | Mode | Files | Block size | Sync | Checkpoint | Elapsed | Throughput |
| --- | --- | --- | --- | --- | --- | --- | --- |
| local | base | `128` | `512k` | `1` | `0` | `5.316s` | `12.04 MiB/s` |
| qnap | base | `128` | `512k` | `1` | `0` | `52.863s` | `1.21 MiB/s` |
| local | checkpoint | `128` | `512k` | `1` | `1` | `4.904s` | `13.05 MiB/s` |
| qnap | checkpoint | `128` | `512k` | `1` | `1` | `55.109s` | `1.16 MiB/s` |

Stats delta:

- local base: `wal_records=97543`, `wal_fpi=27`, `wal_bytes=8478071`, `wal_write=512`, `wal_sync=508`, `buffers_alloc=412`
- qnap base: `wal_records=104156`, `wal_fpi=131`, `wal_bytes=9595780`, `wal_write=684`, `wal_sync=676`, `buffers_alloc=455`
- local checkpoint: `CHECKPOINT elapsed_s=0.059`, `wal_records=102996`, `wal_fpi=4`, `wal_bytes=8848181`, `wal_write=516`, `wal_sync=513`, `checkpoints_req=1`, `checkpoint_write_time=8.0`, `checkpoint_sync_time=32.0`, `buffers_checkpoint=1657`, `buffers_backend=843`, `buffers_alloc=417`
- qnap checkpoint: `CHECKPOINT elapsed_s=1.379`, `wal_records=103926`, `wal_fpi=1`, `wal_bytes=8972646`, `wal_write=663`, `wal_sync=660`, `checkpoints_req=1`, `checkpoint_write_time=102.0`, `checkpoint_sync_time=795.0`, `buffers_checkpoint=1012`, `buffers_backend=854`, `buffers_alloc=403`

Notes:

- The QNAP backend is still much slower than local Docker on the same payload size.
- Forcing `CHECKPOINT` exposed a much larger checkpoint cost on QNAP than on the local Docker backend.

### Connection Churn Benchmark

Observed with `make postgres-benchmarks-compare` on the same local/qnap split.

| Backend | Connections | Elapsed | Connect avg | Connect p95 | Query avg | Query p95 |
| --- | --- | --- | --- | --- | --- | --- |
| local | `100` | `0.901s` | `8.437 ms` | `9.606 ms` | `0.541 ms` | `0.668 ms` |
| qnap | `100` | `5.315s` | `47.536 ms` | `147.125 ms` | `5.532 ms` | `7.586 ms` |

Notes:

- The network/backend gap dominates the connection setup cost on QNAP.
- This benchmark remains the cleanest smoke for session churn and pool sizing, not for raw throughput.

## 2026-06-25 QNAP Benchmark Snapshot

Collected from commit `4f3fe83` (`FOD 3.1.1: add qnap compose transport preset`).

### Mounted Fio Smoke

Observed on the QNAP Docker backend with the mounted PostgreSQL-backed runtime. The sequential run used `make test-fio-sequential-io-strace`, and the mixed / random mixed runs used `make test-fio-mixed-io` and `make test-fio-random-mixed-io`.

| Workload | Block read | Block write | Extent read | Extent write |
| --- | --- | --- | --- | --- |
| Sequential 64 KiB smoke | `1561 KiB/s` | `1280 KiB/s` | `1600 KiB/s` | `1306 KiB/s` |
| Mixed sequential rw 4 MiB | `106 KiB/s` | `113 KiB/s` | `7922 B/s` | `8433 B/s` |
| Random mixed rw 4 MiB | `65.7 KiB/s` | `69.9 KiB/s` | `6772 B/s` | `7209 B/s` |

Notes:

- The sequential smoke also confirmed the current internal timing shape: block mode reported `fuse_read_total_us=34960`, `fuse_write_total_us=51333`, and extent mode reported `fuse_read_total_us=33935`, `fuse_write_total_us=50548`.
- The extent path stayed much slower than block mode on the mixed and random mixed workloads, which keeps the extent path clearly opt-in.

### Throughput Smoke

Observed on the QNAP Docker backend with the default local FOD profile.

| Benchmark | Result |
| --- | --- |
| `make test-throughput` | `1048576 bytes in 0.505s (1.98 MiB/s)` |
| `make test-throughput-sync` | `1048576 bytes in 0.665s (1.50 MiB/s)` |

Notes:

- These are short single-block write smokes, so they are useful for relative host comparisons but not for long-run saturation claims.

## 2026-06-25 QNAP `synchronous_commit` Comparison

Collected from commit `1ce18c4` (`FOD 3.1.1: note stock qnap postgres tuning`).

### Throughput Smoke

Observed on the QNAP Docker backend with the mounted PostgreSQL-backed runtime and a larger `32 MiB` write smoke (`THROUGHPUT_BLOCK_SIZE=1M`, `THROUGHPUT_COUNT=32`). The baseline run used `FOD_SYNCHRONOUS_COMMIT=on`; the comparison run used `FOD_SYNCHRONOUS_COMMIT=off`.

| Profile | `make test-throughput-sync` |
| --- | --- |
| `on` | `33554432 bytes in 1.169s (27.38 MiB/s)` |
| `off` | `33554432 bytes in 1.244s (25.73 MiB/s)` |

Notes:

- The smaller `1 MiB` smoke was noisy across repeated runs, so the longer `32 MiB` run is the better comparison point here.
- On this sample, `synchronous_commit=off` did not produce a throughput win on the QNAP Docker backend.

### Sequential Fio Smoke

Observed on the same backend with `make test-fio-sequential-io-strace`.

| Profile | Block read | Block write | Extent read | Extent write |
| --- | --- | --- | --- | --- |
| `on` | `1391 KiB/s` | `1164 KiB/s` | `1422 KiB/s` | `1255 KiB/s` |
| `off` | `1561 KiB/s` | `1085 KiB/s` | `1561 KiB/s` | `1185 KiB/s` |

Notes:

- The read-side numbers bounced around more than the write-side numbers, which is consistent with a small smoke on a live backend.
- The write-side results still did not show a clean win for `off`, so the default `on` setting remains the safer baseline for this QNAP sample.

## 2026-06-25 QNAP PostgreSQL Optimization Profiles

Collected from commit `5abf053` (`FOD 3.1.1: add PostgreSQL optimization benchmarks`).

### WAL Pressure Benchmark

Observed on the QNAP Docker backend with `make test-postgresql-wal-pressure`.

| Profile | Files | Block size | Sync | Elapsed | Throughput |
| --- | --- | --- | --- | --- | --- |
| default | `64` | `512k` | `1` | `25.628s` | `1.25 MiB/s` |

Stats delta:

- `pg_stat_wal`: `wal_records=51831`, `wal_fpi=72`, `wal_bytes=4753249`, `wal_write=336`, `wal_sync=335`
- `pg_stat_bgwriter`: no checkpoint activity showed up in this short run, but `buffers_alloc=257` moved

Notes:

- This run is useful as a WAL volume and fsync-pressure smoke, but it did not push checkpoint counters on this backend.
- A larger or checkpoint-forcing variant would be needed if we want a direct checkpoint-tuning signal.

### Connection Churn Benchmark

Observed on the QNAP Docker backend with `make test-postgresql-connection-churn`.

| Profile | Connections | Elapsed | Connect avg | Connect p95 | Query avg | Query p95 |
| --- | --- | --- | --- | --- | --- | --- |
| default | `100` | `6.430s` | `54.439 ms` | `154.968 ms` | `9.783 ms` | `16.254 ms` |

Notes:

- The connection setup cost dominates the simple `SELECT 1` loop, which makes this a good smoke for pool sizing and session churn.
- This is a direct PostgreSQL-side benchmark, not a FUSE throughput benchmark.

## 2026-06-25 Replay Confirmation Snapshot

Collected from commit `94d9695` (`FOD 3.1.1: confirm create replay after unique conflict`).

### Mounted Fio Smoke

Observed on the current host with the mounted PostgreSQL-backed runtime. The sequential run used `make test-fio-sequential-io-strace`, and the mixed / random mixed runs used `make test-fio-mixed-io` and `make test-fio-random-mixed-io`.

| Workload | Block read | Block write | Extent read | Extent write |
| --- | --- | --- | --- | --- |
| Sequential 64 KiB smoke | `421 KiB/s` | `552 KiB/s` | `762 KiB/s` | `1333 KiB/s` |
| Mixed sequential rw 4 MiB | `1210 KiB/s` | `1289 KiB/s` | `236 KiB/s` | `251 KiB/s` |
| Random mixed rw 4 MiB | `830 KiB/s` | `884 KiB/s` | `181 KiB/s` | `193 KiB/s` |

Notes:

- The sequential smoke also confirmed the current internal timing shape: block mode reported `fuse_read_total_us=129172`, `fuse_write_total_us=156046`, and extent mode reported `fuse_read_total_us=61281`, `fuse_write_total_us=50212`.
- Mixed and random mixed still strongly favor the block path on this host, which keeps the extent path clearly opt-in.

### Throughput Smoke

Observed on the current host with the default local FOD profile.

| Benchmark | Result |
| --- | --- |
| `make test-throughput` | `1048576 bytes in 0.088s (11.40 MiB/s)` |
| `make test-throughput-sync` | `1048576 bytes in 0.100s (10.03 MiB/s)` |

Notes:

- These are short single-block write smokes, so they are useful for relative host comparisons but not for long-run saturation claims.

## 2026-06-25 Benchmark Snapshot

Collected from commit `1ba00b8` (`FOD 3.1.1: organize bounded replay follow-up`).

### Mounted Fio Smoke

Observed on the current host with the mounted PostgreSQL-backed runtime. The sequential run used `make test-fio-sequential-io-strace`, and the mixed / random mixed runs used `make test-fio-mixed-io` and `make test-fio-random-mixed-io`.

| Workload | Block read | Block write | Extent read | Extent write |
| --- | --- | --- | --- | --- |
| Sequential 64 KiB smoke | `481 KiB/s` | `388 KiB/s` | `790 KiB/s` | `615 KiB/s` |
| Mixed sequential rw 4 MiB | `550 KiB/s` | `585 KiB/s` | `84.8 KiB/s` | `90.3 KiB/s` |
| Random mixed rw 4 MiB | `281 KiB/s` | `299 KiB/s` | `61.2 KiB/s` | `65.2 KiB/s` |

Notes:

- The sequential smoke also confirmed the current internal timing shape: block mode reported `fuse_read_total_us=118035`, `fuse_write_total_us=186219`, and extent mode reported `fuse_read_total_us=71296`, `fuse_write_total_us=106128`.
- Mixed and random mixed still strongly favor the block path on this host, which keeps the extent path clearly opt-in.

### Throughput Smoke

Observed on the current host with the default local FOD profile.

| Benchmark | Result |
| --- | --- |
| `make test-throughput` | `1048576 bytes in 0.185s (5.41 MiB/s)` |
| `make test-throughput-sync` | `1048576 bytes in 0.099s (10.08 MiB/s)` |

Notes:

- These are short single-block write smokes, so they are useful for relative host comparisons but not for long-run saturation claims.

## FOD 3.0.9 Read Cache Eviction Policy Comparison

Initial single-run snapshot observed on the current host with the mounted PostgreSQL-backed runtime, `FOD_READ_CACHE_EVICTION_POLICY=fifo` versus `lru`, `FIO_BLOCK_SIZE=4k`, and `FIO_FILE_SIZE=1M` for the sequential workload. The mixed workloads used the default `FIO_FILE_SIZE=4M`, and the random mixed workload used `FIO_RW_MODE=randrw` with `FIO_RWMIXREAD=50`. The `fio` scripts also exercised the extent control run, but the table below only uses the block-storage results that are relevant to `ReadBlockCache`.

| Workload | FIFO read | FIFO write | LRU read | LRU write |
| --- | --- | --- | --- | --- |
| Sequential 1 MiB | `52.6 MiB/s` | `4.15 MiB/s` | `16.4 MiB/s` | `1.58 MiB/s` |
| Mixed rw 4 MiB | `1.52 MiB/s` | `1.62 MiB/s` | `1.49 MiB/s` | `1.59 MiB/s` |
| Random mixed rw 4 MiB | `1.07 MiB/s` | `1.12 MiB/s` | `0.97 MiB/s` | `1.00 MiB/s` |

A follow-up repeat series with six runs per policy/workload changed the picture: sequential reads favored LRU, mixed workloads still favored FIFO, and random mixed was effectively tied. Treat the single-run table above as a preliminary snapshot; the repeat series is the more reliable signal for this host.

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

## 2026-07-11 FUSE ABI 7.31 Copy Baseline

Collected locally from the Storage Engine v2 worktree based on commit
`16bf0f8`. The worktree enabled FUSE ABI 7.31, made exact clean whole-file
copies adopt the source data object, and preserved chunked-copy data when an
extent-backed destination downgraded to block storage.

Commands:

```bash
PROFILE_RUN_ID=storage-whole-object-adoption-20260711T080000Z \
PROFILE_STORAGE_EXTENT_REPEAT=3 \
PROFILE_STORAGE_EXTENT_SIZES=1048576 \
PROFILE_STORAGE_EXTENT_WORKLOADS=test-large-copy-object-adoption \
make profile-storage-extent-size-matrix-local

PROFILE_RUN_ID=storage-abi31-chunked-copy-fixed-20260711T090000Z \
PROFILE_STORAGE_EXTENT_REPEAT=3 \
PROFILE_STORAGE_EXTENT_SIZES=1048576 \
PROFILE_STORAGE_EXTENT_WORKLOADS=test-large-copy-benchmark \
make profile-storage-extent-size-matrix-local
```

Whole-object adoption, 64 MiB:

| mode | samples | mean MiB/s | stdev | mean elapsed s | destination payload inserts | mean WAL bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| block source | 3 | 1219.23 | 49.32 | 0.052578 | 0 | 6492438.67 |
| 1 MiB extent source | 3 | 1282.86 | 91.80 | 0.050159 | 0 | 1008317.67 |

The insert and WAL counters include source creation. The destination shares the
source `data_object_id` and adds no payload rows.

Chunked 4 MiB requests, 64 MiB total:

| mode | samples | mean MiB/s | stdev | min-max MiB/s | mean block inserts | mean extent inserts | mean WAL bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| block | 3 | 17.74 | 0.09 | 17.62-17.82 | 32768 | 0 | 13215098.33 |
| 1 MiB extents | 3 | 26.68 | 3.03 | 22.39-28.84 | 16384 | 68 | 7709446.33 |

The extent count includes 64 source rows and four destination rows created by
the first chunk before the destination safely converts to block storage. This
result supersedes the earlier large-copy measurements that unknowingly used
the kernel's generic fallback because the daemon advertised only ABI 7.17.

## 2026-07-11 Payload Ownership Version 17 Gate

Collected locally from the schema-version-17 ownership worktree based on
commit `a23bfbb`. These are single-run correctness baselines while the pending
worktree removes representative payload `id_file` columns; they are not a
reason to change storage defaults.

Commands:

```bash
make test-large-copy-object-adoption
make test-remount-durability-benchmark
make test-fio-sequential-io
make test-fio-mixed-io
make test-fio-random-mixed-io
FOD_PROFILE_IO=1 make test-fio-sequential-io-strace
PROFILE_CAPTURE_LABEL=storage-ownership-v17 make profile-pg-data-blocks-semantics
PROFILE_CAPTURE_LABEL=storage-ownership-v17 make profile-pg-data-blocks-merge-explain
PROFILE_CAPTURE_LABEL=storage-ownership-v17 make profile-pg-data-blocks-merge-fillfactor-explain-one
```

Mounted fio results:

| workload | path | read | write |
| --- | --- | ---: | ---: |
| sequential 64 KiB | block | `9143 KiB/s` | `3556 KiB/s` |
| sequential 64 KiB | extent | `8000 KiB/s` | `2560 KiB/s` |
| mixed rw 4 MiB | block | `1612 KiB/s` | `1716 KiB/s` |
| mixed rw 4 MiB | extent preset | `776 KiB/s` | `826 KiB/s` |
| random mixed 4 MiB | block | `1196 KiB/s` | `1273 KiB/s` |
| random mixed 4 MiB | extent preset | `677 KiB/s` | `721 KiB/s` |

The profiled direct-I/O sequential run reported
`repo_persist_blocks_us=16683` for blocks and
`repo_persist_extents_us=9357` for one 64 KiB extent. The extent path entered
segment mode once, had zero downgrades, prepared one 65536-byte payload in
`5 us`, and completed the required strace capture.

Whole-object adoption copied 64 MiB at `5068.61 MiB/s` in `0.012627 s` and
confirmed that the destination shared the source object. The remount durability
smoke completed its 64 KiB round trip in `1.019797 s`.

The object-ownership diagnostic found `0` orphan files, blocks, extents, and
CRC rows; `0` reference-count mismatches; and `0` hybrid block/extent objects.
Two unreferenced objects came from the deferred-cleanup tests and remained
eligible for object GC. The current object-keyed merge reproducer completed a
fresh 16384-row insert in `224.411 ms`, filtered an identical conflict set in
`365.851 ms`, and updated the changed conflict set in `349.311 ms`, while the
real `fod.data_blocks` count stayed unchanged.
