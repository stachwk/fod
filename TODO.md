# FOD Decisions, Follow-ups & Archive

This document records the small set of open follow-ups plus completed work, closed decisions, and regression notes for FOD. It is not an active implementation backlog.

## Current Follow-ups

- [x] Detect single-node vs read-only replica mode early and let runtime choose the appropriate lock strategy before mount. Handled in `rust_fuse/src/startup.rs` via `effective_read_only()` and `lock_settings(read_only)`.
- [x] Keep `workers_read` and `workers_write` constrained to the cases where they really help: disjoint read gaps and segmented copy operations, not small contiguous fetches. Read-side gating now goes through `rust_hotpath::read_missing_range_worker_count`; write-side gating already goes through the shared hot-path worker planner.
- [x] Primary mounts keep the PostgreSQL lease backend. Covered by `rust_fuse/tests/lock_backend_smoke.rs::primary_uses_pg_leases_and_replica_stays_memory_backed`.
- [x] Replica mounts stay on the in-memory lock backend. Covered by the same smoke.
- [x] `auto` resolves to the PostgreSQL lease backend on a writable primary DB. Covered by `rust_runtime/src/lib.rs::resolves_auto_and_replica_lock_roles`.
- [x] Test auto role on a recovery/read-only DB, if easy to simulate. Covered by `tests/integration/test_runtime_profile.py` using a temporary standby container created from `pg_basebackup`, plus the existing `FOD_RUST_FUSE_READONLY=1` read-only mount branch coverage.
- [x] Repository-level lease expiry is already covered by `rust_hotpath/tests/lock_manager.rs::expired_flock_lease_is_pruned_on_reacquire`; keep the remaining follow-up focused on mount/process end-to-end expiry.
- [x] Add an end-to-end lease-expiry regression between two independent mounts or processes so stale PostgreSQL leases are validated at the mount layer, not only in repository tests. Covered by `rust_fuse/tests/lock_backend_smoke.rs::primary_lease_expiry_allows_second_mount_reacquire`.
- [x] `FOD_ATIME_POLICY` is covered by `tests/integration/test_atime_policy.sh`.
- [x] `FOD_SYNCHRONOUS_COMMIT`, `FOD_PG_VISIBLE_PATH`, and cache TTL overrides are covered by `tests/integration/test_runtime_profile.py`.
- [x] SELinux mount label contexts (`FOD_SELINUX_CONTEXT`, `FOD_SELINUX_FSCONTEXT`, `FOD_SELINUX_DEFCONTEXT`, `FOD_SELINUX_ROOTCONTEXT`) are covered by `tests/integration/test_runtime_profile.py`.

### FOD Runtime / FUSE Refactor Follow-ups

- [x] Confirm a fully green CI after the `DbfsFuse` -> `FodFuse` rename.
- [x] Audit tests and user-facing messages for remaining `FodFuse` / `fod-*` vs legacy `DbfsFuse` / `dbfs-*` names where the legacy name is not intentional.
- [x] Decide whether `AGENTS.md` needs any extra clarification after the extent PoC started.
- [x] Verify the benchmark runner covers the full benchmark target set and keeps regression tests separate from benchmarks.
- [x] Add fio-based coverage for sequential read/write and extent PoC paths, keeping those cases separate from the Rust unit and integration suites.
- [x] Add a compatibility check for POSIX and FUSE semantics so the filesystem behavior stays aligned with the expected mount contract.
- [x] Verify that the `extents` runtime profile only flips `enable_extents` and leaves the rest of the baseline unchanged.

### Live Runtime Change

- [x] Add a runtime snapshot helper in `rust_runtime` for the live-reloadable tuning subset so future change commands can diff current vs requested state.
- [x] Define the live-reloadable subset explicitly: profile, logging, cache TTLs, read/write worker tuning, and other pure knobs that do not require remount.
- [x] Keep mount-only semantics startup-only for now: `read_only`, `default_permissions`, `lazytime`, `sync`, `dirsync`, `use_fuse_context`, `fopen_direct_io`, `fuse_writeback_cache`, SELinux/ACL mount labels, and lock backend selection.
- [x] Decide the command surface and auth for `fod.change --password`, and persist the canonical reloadable snapshot for the next runtime stage. The command surface now also includes `--get KEY` and `--list` inspection helpers over the effective reloadable snapshot.
- [x] Teach the running FUSE process to consume the stored reloadable snapshot without remount.
- [x] Apply live config updates without remount for the safe subset and reject unsafe keys with a clear error.
- [x] Add end-to-end tests once the reload transport exists, covering both accepted live updates and rejected mount-only changes. Covered by `tests/integration/test_runtime_reload.py` and `make test-runtime-reload`.

### Obszary do rozwoju

- [ ] Dodać pełniejszy replay in-flight SQL po błędach.
  - Progress: `DbRepo::query_rows_text()` now retries read-only SQL once after a transient disconnect, and `DbRepo::exec()` now retries the idempotent replayable command set once too; `index_import_plan_entries` inserts are replay-safe via `DELETE + INSERT`, and `index_scan_runs` plus `index_import_plans` now carry explicit `request_token` columns so retry can return the same running row, but broader transactional replay still needs a separate design.
- [x] Review `fod-indexer` CLI ergonomics after manual use; keep the explicit `--source` contract if that remains the intended API, but consider clearer examples or a positional alias if users keep trying the old style. Added positional source shorthand for `scan`, `hash`, `plan-import`, and `materialize` while preserving `--source`.
- [x] Usprawnić automatyczne sugerowanie nazw źródłom w `fod-indexer source add`, bez usuwania `--name`.
  - Progress: local sources default to the current hostname; SMB/QNAP try to infer the remote host or IP from the mounted share; ADB prefers the device serial from `ANDROID_SERIAL`, `ADB_SERIAL`, `ADB_DEVICE_SERIAL`, or `adb devices`; GitHub sources try the git remote slug or repository name. Explicit `--name` still overrides the suggestion.
- [ ] Rozdzielić adaptery path-backed od przyszłych crawlerów SMB/QNAP/ADB/GitHub.
  - Progress: the source kinds and naming heuristics are in place, but scan/hash/plan/materialize still operate on mounted or mirrored filesystem roots.
  - Next: decide which adapters should grow direct remote crawlers versus staying path-backed indefinitely.
- [ ] Plan implementacji ioctl:
  - [x] Najpierw `FIGETBSZ`. Zaimplementowane w `rust_fuse/src/fs.rs` jako odpowiedź oparta o bieżący `blksize`.
  - [x] Potem `FS_IOC_GETFLAGS`. Na razie zwracane jest neutralne `0`, bo flags nie są jeszcze trwale przechowywane.
  - [x] `FS_IOC_SETFLAGS` przyjmuje teraz tylko `0` jako bezpieczny no-op; inne flagi dostają `EOPNOTSUPP` do czasu decyzji o trwałej polityce.
  - [x] `FS_IOC_FSGETXATTR` zwraca teraz wyzerowany `fsxattr`, a `FS_IOC_FSSETXATTR` przyjmuje tylko zero/no-op do czasu decyzji o trwałej polityce xflags.
  - [ ] `FICLONE` zostaje na razie eksperymentalny, bo na tym hoście obecny kernel/FUSE stack ucina ten request przed userspace; FOD nie ma jeszcze end-to-end potwierdzenia dla tego pathu.
- [ ] Zaprojektować pełną politykę mount-label SELinux.

### Direct I/O Microscope

- [x] Add fine-grained hot-path profiling around `read_block_map()`, `fetch_block_range_chunk()`, `fetch_block_range_parallel()`, `assemble_read_slice()`, `update_write_buffer()`, `flush_write_state()`, `prepare_persist_rows_from_block_plan()`, `prepare_persist_extent_rows_from_extent_ranges()`, `clear_read_cache_for_file()`, and the cache / write-state clone paths so `FOD_FOPEN_DIRECT_IO=1` can be used as a stress microscope instead of a production mode.

## FOD 3.0.9 — fod-indexer MVP

- [x] Add `fod-indexer` crate/binary.
- [x] Add indexer metadata schema.
- [x] Implement local source registration.
- [x] Implement local filesystem scan.
- [x] Implement staged duplicate detection.
- [x] Implement dry-run import plan.
- [x] Document import/materialization phase.

## FOD 3.0.9 — fod-indexer follow-up

- [x] Add materialization/import outside dry-run.
- [x] Wire `fod-indexer` into `Makefile`.
- [x] Add a smoke test for `fod-indexer materialize` that covers duplicate payload reuse, unique payloads, and zero-size imports.
- [x] Add `make test-fod-indexer-smoke` as the canonical Makefile entrypoint for the materialize smoke.
- [x] Scope `fod-indexer plan-import` by source with explicit `--source` and `--all-sources` flags, and cover both modes in smoke tests.
- [x] Add cleanup for failed indexer materialization via `fod-indexer cleanup-failed --plan <id>`, with smoke coverage for duplicate payload reuse and zero-size files.
- [x] Align the remaining `fod-indexer` integration smokes with the zero-length skip contract so stale `empty.txt` assertions do not keep documenting the old pipeline shape.

## FOD indexer: dalszy plan dla Codex

- [ ] Utrwal granice miedzy core engine a adapterami zrodel. `fod-indexer` ma zostac wspolnym silnikiem indeksowania, a nie zbiorem osobnych crawlerow; wszystko, co da sie przedstawic jako lokalny katalog, mount albo mirror, powinno przechodzic przez jeden path-backed flow.
  - Progress: source kinds now carry explicit capability metadata and the CLI surfaces it, but the actual adapter split is still pending.
- [x] Wydziel model zdolnosci zrodla i trzymaj go osobno od samego skanu. Dla kazdego `source kind` doprecyzuj metadane takie jak `path_backed`, `readonly`, `mirror_required`, `needs_export` i `direct_crawler_possible`, zeby sposob pobrania danych byl deklaratywny.
  - Progress: implemented in `rust_indexer/src/capabilities.rs` and surfaced through `SourceKind::capabilities()`, with `source add` / `source list` now printing the profile.
- [ ] Ustal polityke dla `local`, `qnap`, `smb`, `adb` i `github`. Domyslnie maja byc path-backed albo mirrored, a direct crawler tylko wtedy, gdy naprawde nie da sie tego sensownie sprowadzic do katalogu.
- [x] Dopnij nazewnictwo i rejestracje zrodel do modelu capabilities. Heurystyki nazw maja pozostac pomocnicze, ale `--name` musi zostac jawna nadpiska; nazwa nie moze ukrywac, czy zrodlo jest mounted, mirrored, czy tylko importowane do katalogu roboczego.
  - Progress: `--name` remains the explicit override, and the capability profile is now shown alongside source registration and listing so the kind stays visible.
- [ ] Rozszerz testy integracyjne o scenariusze miedzyzrodlowe i adapterowe. Sprawdzaj osobno lokalny mount, mirror/backed source, cleanup po zniknieciu zrodla, ignorowanie hidden/cache paths oraz stabilnosc import/materialize przy kilku `source kind`ach.
- [ ] Doprecyzuj dokumentacje FOD jako wspolnego silnika indeksowania. Wprost zapisz, ze `fod-indexer` ma byc wspolnym core dla `scan/hash/dedupe/plan/materialize/cleanup`, a `msfind` ma korzystac z tego rdzenia zamiast implementowac drugi, podobny pipeline.
- [ ] Zostaw w backlogu tylko to, czego realnie brakuje: decyzje, ktore `source kind`y beda kiedys potrzebowaly direct crawlerow, oraz czy maja byc mirror-only czy native API adapters. Nie rozszerzaj core o protokoly, jesli nie ma konkretnego, nieobslugiwnego jeszcze przypadku.
- [ ] Uporzadkuj safety i retry tylko tam, gdzie sa jeszcze luki. Read-only i idempotentne operacje maja zostac bounded-retry friendly, ale nie dokladaj pelnego replay nieidempotentnych transakcji bez osobnego projektu.
- [ ] Nie wracaj do implementacji podstawowego pipeline jako nowego zadania. `scan`, `hash`, `duplicate detection`, `plan-import`, `materialize` i `cleanup` traktuj jako juz dostarczone; dalsza praca ma byc wokol granic, adapterow i hardeningu.

## FOD 3.0.9 — Cleanup and recovery safety

- [x] Guard `fod-indexer cleanup-failed` against deleting shared data objects.
- [x] Report preserved shared data objects in the cleanup summary.
- [x] Extend cleanup smoke coverage for shared-object safety.
- [x] Document that recovery after errors is currently limited to one retry and does not yet replay in-flight SQL.
- [x] Document that ioctl support is currently limited to `FIONREAD`.
- [x] Document that full SELinux mount-label policy remains out of scope.
- [x] Document that FOD is still early-stage and APIs, benchmarks, and performance defaults may still evolve.

### Storage Hot-Path Prep

- [x] Split storage hot-path planning from SQL execution now that `fs.rs` has been broken up.
- [x] Define minimal planner interfaces before extending the extent engine.
- [x] Keep the current storage schema unchanged for now.
- [x] Preserve the existing helper parity tests while the hot-path refactor moves forward.
- [x] Route the opt-in extent PoC through a dedicated execution branch while keeping the block-storage fallback intact.
- [x] Define a narrow `PersistPlan` boundary for the remaining storage hot-path work so Rust returns the plan and SQL only executes the transaction, without changing the storage schema or the default block-storage path. Implemented as `PersistPlan::Blocks(...)` / `PersistPlan::Extents(...)` in `rust_hotpath/src/persist_plan.rs` with `choose_persist_plan(...)` as the selection boundary.
- [x] Split persist execution into explicit stages: plan persist, prepare rows, execute block storage, and execute extent storage, so the current extent PoC stops being a block-row adapter and becomes a real storage-engine boundary. Implemented in `rust_fuse/src/write_buffer.rs` with `prepare_persist_rows(...)`, `execute_block_storage_persist(...)`, and `execute_extent_storage_persist(...)`; the extent execution path still delegates to block-row persistence for now.

### Extent Engine PoC Scope

The PoC stays intentionally narrow:

- it is opt-in only through `enable_extents = true`,
- it starts with sequential write and read only,
- it accepts a single contiguous extent plan and falls back to block storage otherwise,
- it does not add merge/split/compaction logic yet,
- it does not replace the existing default block-storage path.

- [x] Describe the exact PoC scope so the extent work stays narrow and isolated.
- [x] Keep the PoC behind an explicit `enable_extents = true` gate and out of the default block-storage path.
- [x] Add the smallest possible feature/config gate if one is still missing.
- [x] Start with sequential write and read only.
- [x] Do not add merge/split extent logic at the start.
- [x] Benchmark the PoC on `4 KiB`, `64 KiB`, `1 MiB`, and `4 MiB`.

## Extent Engine Direction

- Keep the logical filesystem model at 4 KiB blocks for now.
- Make Rust the owner of the extent and overlay engine.
- Move PostgreSQL away from storing thousands of 4 KiB blocks directly and toward dynamic extents.
- SQL/query execution already lives in Rust for the runtime paths that remain.
- Keep the non-runtime docs and tests separate from the Rust runtime; do not reintroduce Python as an execution layer.
- Treat this as the next major direction rather than a hard rewrite backlog; when adjacent work touches `truncate`, `fallocate`, `write`, `copy`, `flush`, or `persist`, prefer carving out the corresponding extent/storage piece in Rust if it can be done safely and incrementally.
- Capture the next architectural step explicitly:
  - `logical_block_size = 4k`
  - `persist model = extents`
  - `persist extent classes = 4k..4MiB`
  - `payload stores only used bytes`
  - `Rust returns PersistPlan`
  - `Rust executes transaction`
  - extent support must stay opt-in behind an explicit boolean flag such as `enable_extents = true`; it must not become the default path until that flag is set
- Next concrete step:
  - keep the Rust repository/query boundary intact for the remaining runtime paths,
  - narrow and isolate the existing extent-engine PoC instead of expanding it,
  - keep sequential write/read as the only initial flow,
  - keep merge/split extent logic out of the first pass,
  - preserve the current block-storage path unless `enable_extents = true` is set,
  - benchmark `4 KiB`, `64 KiB`, `1 MiB`, and `4 MiB` before broadening the scope.

## Target Architecture

### Warstwa 1 - Rust runtime

Rust is the runtime and control plane for FOD:

- mount bootstrap
- schema tooling
- config and profile loading
- FUSE callbacks
- administrative logic
- schema migrations
- ACL / permissions / journal / runtime validation policy layers

### Warstwa 2 - Rust storage hot-path

Rust owns the CPU/memory-heavy hot path:

- block engine
- read block assembly
- range slicing
- overlay plus `block_map` merging
- read buffer preparation
- write overlay engine
- `write_into_state()`
- `truncate_to_size()`
- dirty block management
- block payload assembly for flush
- copy engine
- `copy_file_range_into_state()`
- copy segmentation
- worker coordination for read/write copy paths
- segment ordering
- persist preparation
- list preparation for `(file_id, block_index, data)`
- block padding
- deleting blocks beyond EOF
- dirty range accounting

Why this matters:

- the benchmarks show that write path and finalization are the main bottleneck, not the project structure itself
- the highest-value Rust work is the part that reshapes data before SQL, not the SQL layer itself

## Archived Work

### FOD 3.0.0 Compatibility Cut

- [x] Remove `DBFS_*` env/config fallbacks.
- [x] Remove `copy_skip_*` transition alias shims.
- [x] Remove legacy `dbfs` / public schema migration handling from `rust_mkfs/src/main.rs`.

### Recent Architecture Cleanup

- [x] Split metadata cache payloads by purpose: attribute cache and directory-entry cache are now stored separately instead of sharing one mixed payload shape.
- [x] Fix the listing/getattr cache regression where `ls -al` on a directory could disagree with `ls file` because `readdir()` and `getattr()` interpreted the same cache key differently.
- [x] Remove runtime method rebinding from `fod_fuse.py` and replace it with explicit wrapper methods that delegate through `mod/repository/`.
- [x] Unify lookup helpers through the repository layer so `get_file_id`, `get_dir_id`, `get_entry_kind_and_id`, `entry_exists`, and related helpers no longer depend on hidden `__init__` rebinding.
- [x] Move the `getattr()` / `readdir()` query layer into `mod/repository/` so the main FUSE module no longer owns direct listing/attribute SQL for those hot paths.
- [x] Confirm the current refactor through `make test-all` plus an explicit regression check for create/list/remove consistency on a mounted FOD instance.

### FOD Runtime / FUSE Refactor

- [x] Centralize the version in the root `Cargo.toml`.
- [x] Migrate the main naming to FOD / `fod-*`.
- [x] Align CI with the workspace layout and FOD names.
- [x] Update `AGENTS.md` with the current working rules.
- [x] Add `FodFuseSettings` for mount initialization.
- [x] Centralize FUSE settings construction.
- [x] Centralize the `mount_options` helper.
- [x] Move FUSE mount startup out of `rust_fuse/src/main.rs`.
- [x] Keep `rust_fuse/src/main.rs` thin.
- [x] Split FUSE startup helpers into `rust_fuse/src/startup.rs`.
- [x] Split FUSE settings into grouped structs.
- [x] Split read cache helpers out of `rust_fuse/src/fs.rs`.
- [x] Split write buffer helpers out of `rust_fuse/src/fs.rs`.
- [x] Split copy range planner helpers out of `rust_fuse/src/fs.rs`.
- [x] Split runtime settings into grouped structs in `rust_runtime`.
- [x] Rename `DbfsFuse` to `FodFuse`.
- [x] Start the extent-engine PoC.
- [x] Add the benchmark runner to `Makefile`.

### Performance Plan

- Current numeric baselines and profile outputs live in [`BENCHMARKS.md`](BENCHMARKS.md). Keep this section focused on decisions, accepted changes, and rejected experiments.

- [x] Extract CLI parsing and mount startup into `fod_bootstrap.py` so `fod_fuse.py` only carries the filesystem operation layer.
- [x] Extract inode/path identity helpers into `fod_identity.py` so path normalization, inode generation, and ownership defaults are shared in one place.
- [x] Extract PostgreSQL connection pooling and config query helpers into `fod_backend.py` so the filesystem layer stops owning raw pool lifecycle.
- [x] Extract xattr and ACL policy helpers into `fod_xattr_acl.py` so xattr normalization, ACL encoding, and ACL checks live outside the filesystem core.
- [x] Remove the old in-class POSIX ACL constants and helpers from `fod_fuse.py` so `fod_xattr_acl.py` is the single source of truth for xattr/ACL policy.
- [x] Move xattrs from path-based storage to inode-based storage in `fod_xattr_store.py` so rename no longer has to rewrite xattr keys.
- [x] Extract lock state and lock conflict handling into `fod_locking.py` so advisory lock policy no longer lives in the filesystem core.
- [x] Extract data block loading, write cache management, dirty tracking, and buffer persistence into `fod_storage.py` so the write path is no longer embedded in the filesystem core.
- [x] Add an optional PostgreSQL-side CRC cache for unchanged-block copy detection. The cache is populated lazily on demand and refreshed during block persistence, so repeated copy-heavy workloads can reuse stored CRCs instead of rereading full destination blocks every time.
- [x] Remove the in-class write-buffer persistence implementation from `fod_fuse.py` so `fod_storage.py` is the single source of truth for data block persistence.
- [x] Move the read path to block-range loading with a small block cache and read-ahead so `read()` no longer has to load whole files on every access.
- [x] Benchmark the current write path on mounted FOD and record the baseline for large sequential writes. The live baseline is tracked in [`BENCHMARKS.md`](BENCHMARKS.md) and the profile entry points are `make test-throughput` and `make test-throughput-sync`.
- [x] Batch buffered writes in memory and persist them once per flush/release instead of rewriting the whole file repeatedly during a write burst.
- [x] Add block-delta persistence so unchanged blocks are not rewritten on every flush.
- [x] Profile the remaining hot paths (`getattr`, `readdir`, `persist_buffer`) and add only the indexes that move the benchmark. Added directory-parent indexes and confirmed the block-order index is used on `data_blocks`.
- [x] Capture a live throughput baseline for different write sizes on a mounted FOD instance. The measured values are recorded in [`BENCHMARKS.md`](BENCHMARKS.md).
- [x] Add schema versioning so the schema can be repaired in a controlled way instead of relying only on `init` to recreate the database. Current version is `3`, exposed via `schema_version`, exported by `mkfs.fod.py status`, and checked by `make test-schema-upgrade` and `make test-schema-status`; `init` is now idempotent and non-destructive.
- [x] Add named runtime profiles for production-style tuning in `fod_config.ini`. Current profiles include `bulk_write` and `metadata_heavy`, selected with `FOD_PROFILE`.
- [x] Add optional PostgreSQL TLS connection parameters (`sslmode`, `sslrootcert`, `sslcert`, `sslkey`) in `fod_pg_tls.py`, and move client cert/key generation to `mkfs.fod.py` for `init` and `upgrade`.
- [x] Add a regression test for the flush/release dirty gate so clean closes stay cheap and dirty data is persisted exactly once. Added `make test-flush-release-profile`.
- [x] Try skipping the tail-delete optimization for normal growth writes. Rejected: the change regressed small-write throughput, so it was reverted.
- [x] Why it was rejected: the added bookkeeping outweighed the saved `DELETE`; see the historical benchmark notes in [`BENCHMARKS.md`](BENCHMARKS.md) if you need the exact comparison.
- [x] Record a live profile of the current write path so the next performance step starts from measured data instead of guesswork. The current profile split between write, persist, flush, and finalization is kept in [`BENCHMARKS.md`](BENCHMARKS.md) and surfaced by `make test-flush-release-profile`.
- [x] Reject a second tail-delete shortcut based on in-memory persisted-size tracking. Rejected for the same reason as the first shortcut: it hurt small-write throughput.
- [x] Do not retry tail-delete shortcuts or in-memory persisted-size tracking as a default performance strategy. Any future attempt must start from a different hypothesis and be benchmarked before merge.
- [x] Reject the per-block copy/fast-path rewrite in `persist_buffer`. Reverted after it regressed both small and medium writes.
- [x] Reject the shrink-marker variant for skipping tail `DELETE` on growth writes. Reverted because it was still worse than the stable baseline.
- [x] Gate `write()` profiling so it only runs when `FOD_PROFILE_IO=1`. This removed hot-path timing overhead.
- [x] Reject the memoryview-based `persist_buffer` copy path. Reverted after it regressed the write benchmark.
- [x] Cache `block_size` on the FOD instance instead of querying config during write-path operations. This removed a hot-path DB lookup.
- [x] Remove the extra `bytes()` copy from `write()` and assign the incoming buffer directly into the in-memory cache.
- [x] Skip `persist_buffer()` work in `flush()` / `release()` / `fsync()` when the file is not dirty.
- [x] Combine the tail `DELETE` and file-size `UPDATE` into one SQL round-trip inside `persist_buffer()`.
- [x] Avoid path-based lock cleanup work in `release()` and use the already-known file handle as the resource key.
- [x] Set `execute_values(..., page_size=len(blocks))` inside `persist_buffer()` so large block batches stay in one SQL round-trip.
- [x] Replace the `DELETE; UPDATE` multi-statement block in `persist_buffer()` with a single CTE-based statement.
- [x] Stop sorting dirty block indexes before persisting.
- [x] Stop copying the whole write buffer into `bytes()` before persisting and use `bytearray` slices directly for block payloads.
- [x] Split very large `persist_buffer()` batches into smaller SQL chunks instead of building one massive `execute_values()` payload.
- [x] Add a large-write auto-flush threshold so very large dirty buffers can be persisted before close instead of concentrating the entire cost in `release()`.
- [x] Expose `synchronous_commit` as a separate runtime knob for PostgreSQL sessions. Keep the default at `on` and treat `off` as an explicit tuning choice unless a future workload benchmark proves otherwise. The current local comparison is recorded in [`BENCHMARKS.md`](BENCHMARKS.md).
- [x] Expose `persist_buffer_chunk_blocks` as a separate runtime knob for flush batching. Keep the default conservative and let profiles override it when larger `execute_values()` batches help. The current comparison is recorded in [`BENCHMARKS.md`](BENCHMARKS.md).
- [x] Use the current benchmark baseline in [`BENCHMARKS.md`](BENCHMARKS.md) to decide whether the next performance work should focus on fewer SQL round-trips for small writes or on additional batching around flush/release. Decision: the current baseline is good enough; stop further tuning here unless a regression appears or a new benchmark target is introduced.
- [x] Confirm the large-write chunking fix on a real 1 GiB sequential write. The `dd if=/dev/zero of=test bs=1M count=1024` scenario completed successfully on `/mnt/fod`, and the file was visible afterward at the expected size.
- [x] Record the interpretation of the 1 GiB `dd` timing correctly: the data path itself finished in about `12 s`, while the remaining wall time was spent draining `flush()` / `release()` / `persist_buffer()` work. Future throughput work should measure `write` and finalization separately instead of treating `dd` wall time as pure copy speed.
- [x] Keep the benchmark suite expanded with explicit coverage for large `copy_file_range()` transfers, large multi-block file writes, and remount durability so write-path tuning stays comparable across releases.
- [x] Compare FOD atime behavior on a short wall-time benchmark. The current measured values live in [`BENCHMARKS.md`](BENCHMARKS.md).

### Finalized Performance Wins

These changes are already merged into the codebase and should be kept:

- `write()` profiling is opt-in via `FOD_PROFILE_IO=1`, so normal hot-path writes do not pay timing overhead.
- `block_size` is cached on the FOD instance instead of being queried from the database in the write path.
- `write()` no longer copies the incoming buffer through an extra `bytes()` conversion before writing into the cache.
- `flush()`, `release()`, and `fsync()` skip `persist_buffer()` work when the buffer is not dirty.
- `persist_buffer()` combines tail `DELETE` and file-size `UPDATE` into one SQL round-trip.
- `release()` uses the known file handle as the resource key for regular files instead of re-resolving the path.
- `persist_buffer()` splits very large block batches into smaller SQL chunks so large sequential writes do not exhaust the PostgreSQL client output buffer.
- Large sequential writes are now confirmed to work end-to-end on the mounted filesystem; the 1 GiB `dd` scenario completed successfully after chunking was added.
- Large dirty buffers can now auto-flush before close when they reach the configured threshold, which helps move finalization work out of `release()`.
- The read path now uses block-range loading with a small cache and read-ahead instead of loading whole files on every access.

### Must have

- [x] Fuller `xattr` family support:
  - `user.*`
  - `security.*`
  - `trusted.*`

#### Definition of done

- Each item in this section has:
  - an implementation in `fod_fuse.py` or a deliberate unsupported decision
  - at least one integration test or smoke test
  - passing coverage in `make test-all`

### Should have

- [x] Optional `mount --help`/README matrix with ready-made profiles:
  - `fod-relaxed`
  - `fod-linux-default`
  - `fod-selinux`

### Later

- [x] A short smoke profile for FOD atime behavior in `relatime` mode, plus a separate one for `noatime`.
- [x] A simple benchmark for FOD atime behavior that can compare `default`, `noatime`, and `nodiratime` runs on file reads and directory listings.
- [x] A target `make test-all-full` if symlinks/locks/journaling grow further.
- [x] Consider a separate test for `statfs` and `use_ino`.

## Already in place

- `getattr`, `readdir`, `open`, `read`, `write`, `truncate`, `rename`, `unlink`, `mkdir`, `rmdir`, `chmod`, `chown`, `utimens`, `statfs`
- `opendir`, `releasedir`, `fsyncdir`
- `destroy`
- `mknod` for FIFO and device nodes; `st_rdev` and `st_dev` are reported, and `open` for special nodes is still unsupported
- `flock`
- `fallocate`
- `copy_file_range`
- `ioctl` for `FIONREAD`
- `read_buf` / `write_buf`
- `poll` as a backend helper for regular files
- `lseek` as a backend helper for `SEEK_SET/SEEK_CUR/SEEK_END`
- `xattr` backend for `user.*`, `trusted.*`, `security.selinux`, and `system.posix_acl_*` with ACL enforcement and default inheritance
- `access()` smoke test for `R_OK`, `W_OK`, `X_OK`
- `access()` test for owner, primary group, and supplementary groups
- `access()` smoke test is part of `make test-all`
- mount suite covers end-to-end file and directory operations: `mkdir`, `rmdir`, `create`, `unlink`, `rename`, `read`, `write`, `truncate`, `chmod`, `chown`, `utimens`, `symlink`, `readlink`, `stat`, `statfs`, `df -Ph`, `df -Phi`, `access()`
- mount profile matrix is documented in README: `fod-relaxed`, `fod-linux-default`, `fod-selinux`
- smoke profiles for FOD atime behavior in `noatime` and `relatime` modes are exposed as `make test-atime-noatime` and `make test-atime-relatime`
- `make test-atime-benchmark` prints a simple wall-time baseline for file and directory atime behavior so `default`, `noatime`, and `nodiratime` runs can be compared directly.
- access-date writes are deduplicated per open handle, so a single `read()` / `readdir()` sequence only touches the timestamp once and does not rewrite it continuously.
- write-side `mtime` / `ctime` persistence is checked through a regression that confirms multiple writes on the same open file only advance metadata when the dirty buffer is flushed, not on every intermediate write call.
- sequential read-ahead now has a regression that verifies a second adjacent `read()` on the same handle preloads additional blocks into the cache instead of only fetching the requested byte range.
- the read cache defaults to a larger LRU size now, and sequential access stretches the read-ahead window so adjacent scans can reuse prefetched blocks more often.
- a dedicated read-cache benchmark can compare `FOD_READ_CACHE_BLOCKS=256` vs `1024` on sequential scans.
- metadata cache TTLs are configurable in `fod_config.ini` (`metadata_cache_ttl_seconds`, `statfs_cache_ttl_seconds`), and `getattr()` / `readdir()` / `statfs()` use short-lived caches with invalidation on mutating operations.
- metadata cache payloads are now split by type, so attribute and directory-entry state no longer share one cache payload layout.
- schema versioning is explicit: `mkfs.fod.py` writes `schema_version = 3`, `mkfs.fod.py status` exports the current version and migration manifest, `init` is idempotent and non-destructive, and `make test-schema-upgrade` / `make test-schema-status` verify that `upgrade` can repair missing schema state, restore the current version, and enforce the schema-admin secret for later `init` / `upgrade` / `clean` calls on an existing database.
- runtime profiles are explicit: `FOD_PROFILE=bulk_write` and `FOD_PROFILE=metadata_heavy` override the base `[fod]` tuning values from `fod_config.ini`.
- `make test-all-full` extends `make test-all` with workflow checks for files/directories/metadata/symlink, shell statfs/use_ino, mount workflow, atime smoke, and throughput
- `make test-tree-scale` benchmarks `getattr`/`readdir` on a larger seeded tree
- stable inode model based on durable `inode_seed` values for directories, files, and symlinks
- ownership inheritance for setgid parent directories, including `mkdir` and `rename` edge cases
- `bmap` as a logical mapping for regular files and hardlinks
- hot-path indexes: `hardlinks.id_file`, `directories.id_parent`, `files.id_directory`, `hardlinks.id_directory`, `symlinks.id_parent`, and `data_blocks(id_file, _order)`
- `st_blocks` heuristic for directories and small files
- `st_nlink` for directories and root counted only from subdirectories
- `poll` as a backend helper for regular files
- `default_permissions` as the default mount option
- `use_ino` as the default mount option
- explicit wrapper/delegation model from `fod_fuse.py` into `mod/repository/`
- repository-owned query helpers for `getattr()` and `readdir()`
- SELinux options:
  - `--selinux auto|on|off`
  - default `off`
  - `auto` only when host-driven detection is desired
  - runtime SELinux is active only when `on` or `auto`
  - ACL activation uses `--acl on|off`
  - `context`
  - `fscontext`
  - `defcontext`
  - `rootcontext`
- atime / sync options:
  - `noatime`
  - `nodiratime`
  - `relatime`
  - `strictatime`
  - `lazytime`
  - `sync`
  - `dirsync`

## Documented Decisions

### SELinux

Status: intentionally closed as a non-goal for this repo.

- Decision: FOD keeps SELinux as xattr-backed metadata plus runtime gating only; it does not attempt to implement a full mount label policy.
- Keep the existing coverage for:
  - mount with `FOD_SELINUX=on`
  - mount with `FOD_SELINUX=off`
  - mount with `FOD_SELINUX=auto`
  - reading and writing `security.selinux`
- Document explicitly that full SELinux correctness depends on host policy and is not implied by xattr storage alone.

### Full FUSE / Linux Compatibility

Based on verified `libfuse3` behavior and Linux VFS behavior, `libfuse3` stays the strategic baseline when compatibility, standardness, and easier upstream-aligned debugging matter most. This section records the current `ioctl` compatibility state:

#### Plan

1. Extend `ioctl` coverage beyond the current `FIONREAD` path if a real consumer appears in the mount suite or `pjdfstest`.
1. Add a dedicated smoke test for the consumer instead of the raw syscall wrapper, so the behavior is validated end-to-end through the mount.
1. Revisit whether any other libfuse hooks need explicit end-to-end coverage once `ioctl` has a real consumer.

Status: the mount suite now includes a real `ioctl/FIONREAD` consumer smoke test, so the first two items above are closed.
This section is now a documented compatibility note rather than active backlog.

#### Current status

- no known mount-smoke gap remains for `ioctl/FIONREAD`; keep the backend-only unit test as a low-level regression and the mount-suite smoke as the end-to-end check

### Repository / FUSE Boundary

Status: active design direction, but the main decisions below are already taken.

- Decision: `fod_fuse.py` should prefer explicit wrapper methods over runtime rebinding in `__init__`.
- Decision: lookup helpers and the `getattr()` / `readdir()` query layer should live in `mod/repository/`, not directly in the main FUSE module.
- Decision: metadata cache payloads should stay split by type instead of sharing one generic cache payload across attrs and directory listings.
- Follow-up: when `mod/repository/` grows enough, split it into lookup/query and mutation-oriented modules instead of moving SQL back into `fod_fuse.py`.

### Missing Filesystem Features

- No known open `file_metadata` gaps remain beyond the already-covered `change_date`/ctime tracking, `read`-driven `atime`, write/truncate/touch coverage, explicit `touch -a` / `touch -m` semantics, zero-length `write` handling, `truncate` no-op handling for unchanged sizes, and `utimens` no-op handling for unchanged timestamps on files and directories. Keep the regression tests in place.
- No known open permission gaps remain beyond the already-covered sticky-bit enforcement, owner/root checks, supplementary-group-aware `chown`, symlink metadata handling, special-bit clearing on ownership changes, unchanged-ownership `chown`/`chmod` no-op behavior on files and directories, and directory `setgid`/`setuid` inheritance semantics. Keep the regression tests in place.

### pjdfstest Observations

- FOD keeps directory `setgid` bits on ownership changes to match Linux-like behavior observed in `pjdfstest`, while still clearing special bits on regular files.
- FOD keeps `chown(-1, -1)` as an explicit no-op. The upstream `pjdfstest` cases note that POSIX allows timestamps to remain unchanged in that case, and FOD keeps that behavior explicit and tested.
- FOD keeps `unlink()` on directories as `EPERM`. The upstream `pjdfstest` coverage exercises this case, and FOD keeps it covered by a regression test.
- If a future change touches `chmod`, `chown`, `rename`, or `utimens` semantics again, extend compatibility coverage with additional `pjdfstest` subsets.

### Operational

- `LICENSE` is already set to MIT.
- PostgreSQL TLS is optional: `sslmode=require` gives encryption, and `mkfs.fod.py --generate-client-tls-pair` can create a local client cert/key pair for certificate-auth setups during `init` or `upgrade`.
- FOD expects transactional PostgreSQL connections with `autocommit` disabled; `read committed` is sufficient for the current lock and metadata flows.
- Detect single-node vs read-only replica mode early and let the runtime pick an appropriate lock strategy for each case; `postgres_lease` stays the default production backend for writable primary mounts, and the Rust smoke suite already checks two primary mounts plus a replica against the same PostgreSQL database.
- Writable primary mounts now also maintain a `client_sessions` heartbeat row in PostgreSQL. If future cleanup or recovery logic grows around that session state, keep it TTL-driven, host-agnostic, and covered by multi-mount tests.
- Treat `session_id` as part of lease identity and cleanup scope for PostgreSQL-backed locks. `owner_key` alone is not enough once the same FOD dataset can be mounted from different hosts, so any future lock/session cleanup work should key off the session row and keep the semantics host-agnostic.
- Keep `workers_read` and `workers_write` constrained to the cases where they really help: disjoint read gaps and segmented copy operations, not small contiguous fetches.
- The most likely long-term direction for FOD is still userspace FUSE + PostgreSQL backend + a native Rust storage/hot-path engine, with Python kept as the orchestration layer until the native core is ready.

## Notes

- Storing `security.selinux` in xattr alone is not enough to make the filesystem fully SELinux-aware. It is the foundation, not the whole model.
- `mknod` creates FIFO and char-device metadata, but `open` for special nodes still needs separate semantics.
- `poll` is available through the Rust mount frontend for regular files.
- `fallocate`, `flock`, `copy_file_range`, `ioctl`, `read_buf`, `write_buf`, `opendir`, `releasedir`, `fsyncdir`, `destroy`, `access`, `bmap`, `lseek`, `rename`, `mknod`, `st_blocks`, `st_nlink`, `ownership inheritance`, advisory `locks`, sticky-bit enforcement on `unlink`/`rmdir`, `chown` special-bit clearing, journal UID tracking, `ctime`/`change_date` metadata tracking, and the stable inode model are already in `Already in place`.
- `statfs` and `use_ino` have a dedicated shell smoke test: `make test-statfs-use-ino`.
- metadata cache and statfs cache are TTL-backed and configurable via `fod_config.ini`; keep cache invalidation on mutating operations in sync with the read-side cache helpers.
- schema upgrades are intentionally conservative for now: `init` is idempotent and non-destructive, `upgrade` repairs missing schema objects and restores `schema_version`, the schema-admin secret is required once a database already exists, and the repo still does not ship multi-step migration files for future schema changes.
- production profiles should remain documented and tested when tuning defaults change, because they are part of the supported runtime surface.
- the public roadmap lives in `ROADMAP.md`, and the current comparison baselines live in `BENCHMARKS.md`; keep both in sync with changes to CI or runtime tuning.
- PostgreSQL-backed advisory locking is the supported production path for both `flock` and `fcntl` range locks; crash-recovery/TTL-expiry coverage is in place, and the remaining work is mostly operational hardening plus any edge-case cleanup.
- For Linux VFS, the main priority now is to keep metadata, permission checks, `statfs`, and the repository/FUSE boundary sane and consistent.


------->>2026.04.25 18:45<<----------------

Goal:
Add a mechanism for detecting identical regular files that may have different names and may be located in different directories, but have the same file size and the same content hash. In such cases, FOD should be able to reuse the same stored data object instead of storing duplicate content.

Scope:
- Applies only to regular files.
- File identity for deduplication should be based on:
  - file size,
  - full content hash.
- Preferred final hash: SHA256 or BLAKE3.
- A fast hash may be used only as a preliminary filter.
- In safe mode, FOD may additionally verify equality by byte-by-byte comparison.
- Different paths and filenames may reference the same stored data object.
- Removing one path must not remove the shared data while other references still exist.
- Modifying one file that shares data must either:
  - use copy-on-write, or
  - follow explicit hardlink semantics, depending on the selected FOD mode.

Preferred implementation model:
- Do not automatically create true POSIX hardlinks at the inode/file_id level.
- Start with data-object deduplication instead:

  path_a -> file_id_a -> data_object_id_x
  path_b -> file_id_b -> data_object_id_x

- When one of the files is modified, FOD should detach it from the shared data_object_id and create a new data object.
- Future idea: if `file_size + content_hash` dedupe comes back, implement it as block-level versioning with copy-on-write and immutable shared blocks, not as POSIX hardlinks. The goal is for later writes to diverge per file without mutating every path that started from the same content.

Design tasks:
- Review the current FOD metadata model.
- Identify where file_id/inode metadata is stored.
- Identify where chunks/blocks are stored.
- Add or design a data_objects/content_objects table.
- Store file_size, content_hash, reference_count, and data object metadata.
- Decide when the content hash is calculated:
  - on file release,
  - on flush,
  - after completed import,
  - or during a dedicated dedupe scan.
- Add protection against deduplicating files that are still open for writing.
- Add protection against deduplicating incomplete temporary files.
- Consider a dedicated command or maintenance mode:

  fod dedupe scan

Notes:
- File size alone is not sufficient.
- Weak checksums such as CRC32 should not be used as the final identity check.
- The minimum safe practical identity key should be:

  file_size + strong_content_hash

- Automatic deduplication should prefer copy-on-write semantics.
- True hardlink behavior should remain an explicit operation handled by link().

------->>2026.04.25 18:45<<----------------


[rename-fod-to-fod]
- Rename project branding from BDFS to FOD.
- FOD means FileSystem on Database Engine.
- The initial transition kept FOD-compatible wrappers and environment aliases.
- Do not rename PostgreSQL schema; canonical schema remains fod.
- Compatibility cut target: FOD 3.0.0


### Performance follow-up: Rust FUSE runtime tuning wiring

- [x] Pass read/write runtime tuning from `RuntimeConfig` into `FodFuse`.
- [x] Store `read_cache_blocks`, `read_ahead_blocks`, `sequential_read_ahead_blocks`, `small_file_read_threshold_blocks`, `workers_read`, `workers_read_min_blocks`, `workers_write`, and `workers_write_min_blocks` in the Rust FUSE state.
- [x] Add mount startup logging for these values.
- [x] Add a regression test proving env/config overrides are visible in the mounted Rust runtime.
- [x] Benchmark sequential read and metadata-heavy tree walk before and after the change. Covered by the read-cache benchmark and tree-scale / metadata-heavy entries in `BENCHMARKS.md`.


### Performance / Correctness Follow-ups

- [x] Make partial-block writes fail safely when the existing block cannot be loaded from PostgreSQL; do not replace a failed block read with zero-filled data.
- [x] Make `update_write_buffer()` treat zero-length writes as a no-op before changing the in-memory file size.
- [x] Add a deterministic eviction policy for `recent_write_blocks`.
- [x] Compare FIFO vs LRU behavior for `ReadBlockCache` on sequential, mixed, and random fio workloads. A six-run repeat series showed a mixed picture on the current host: sequential reads favored LRU, mixed workloads favored FIFO, and random mixed was effectively tied, so the cache policy choice is workload-dependent.
- [ ] Keep extents opt-in until end-to-end mixed/random benchmarks show a stable win over the default block path.
