# Conclusions

Use this file to record concise conclusions that matter for future work.

## 2026-06-27

- `fod-indexer materialize` now streams file payload import from disk instead of building a whole-file `Vec<Vec<u8>>` first. The new `DbRepo::persist_file_blocks_from_path(...)` path keeps the transaction boundary intact, bounds memory to chunk-sized blocks, and the existing `make test-fod-indexer-smoke` still passes after the change.
- The new short local/QNAP WAL knob sweep on commit `a7504c6` stayed checkpoint-free (`checkpoints_req=0` everywhere), so `max_wal_size` and `checkpoint_timeout` did not produce a meaningful signal on this smoke. `synchronous_commit=off` was slightly slower on local and effectively flat on QNAP, while `wal_compression=on/lz4` trimmed only a little WAL volume without improving throughput. This run is a useful sanity check, but it is too short to treat checkpoint tuning as measured.

## 2026-06-28

- `transactional_replay_confirmed()` is now the shared replay-confirmed transaction helper in `rust_hotpath/src/pg.rs`. It covers the request-token-backed session/data-object/promotion paths and the natural-key owner-key touch path, while `acquire_flock_lease()` now writes durable grant/deny outcomes into `lock_lease_request_tokens` so a lost COMMIT can be confirmed for both true and false results.
- The replay classifier now also includes the remaining create-entry inserts plus the `data_objects` reference-count updates, so the in-flight SQL replay envelope is broader than the earlier commit-disconnect-only smoke paths. `cargo fmt --all`, `cargo test -p fod-rust-hotpath --test transactional_replay_smoke`, and `cargo test -p fod-rust-hotpath --test lock_manager` passed after the refactor.
- `register_client_session()` is now request-token-backed too, so the startup session-registration insert can survive a transient disconnect and still return the same `session_id` on retry without duplicating the `client_sessions` row.
- `set_file_size()` and `adopt_source_data_object()` now also use `transactional_replay_confirmed()` with direct durable row probes, so the remaining row-state confirmation paths are centralized with the request-token-backed ones. `purge_primary_file()` still uses the older bounded replay shape because its cleanup branches are more complex than the simple confirmation cases. `cargo fmt --all`, `cargo test -p fod-rust-hotpath --test transactional_replay_smoke`, and `cargo test -p fod-rust-hotpath --test pg_query` passed after the refactor.
- The transactional replay work is now stabilized at the current bounded envelope for FOD 3.2.1. The remaining in-flight SQL replay expansion stays a separate project instead of widening the default retry path in place.
- The local-only long WAL smoke on commit `e66e66c` finally crossed the checkpoint boundary. With `PG_WAL_PRESSURE_COUNT=10000` and `PG_WAL_PRESSURE_BLOCK_SIZE=512k`, the baseline and `max_wal_size=8GB` runs both ended with a timed checkpoint (`checkpoints_timed=1`, `checkpoints_req=0`) around `670 MB` and `950 MB` of WAL respectively. When `checkpoint_timeout` was stretched to `15min` or `30min`, the timed checkpoint disappeared and the checkpoint moved to the requested path (`checkpoints_req=1`, `checkpoints_timed=0`), which suggests the 5-minute timer is the first limiter on this workload and the size/request path becomes visible once the timer is relaxed.
- `pg_stat_activity` stayed flat during the same long pass, so the local write burst still looks checkpoint-dominated rather than connection-churn-dominated. That makes this benchmark a better baseline for the next PostgreSQL tuning pass than the earlier short smoke.
- The follow-up local-only max-WAL sweep on commit `be642a6` isolated the size cap itself. With `checkpoint_timeout=30min`, the default `max_wal_size` still produced two requested checkpoints at `1.29 GB` of WAL, while `POSTGRES_MAX_WAL_SIZE=8GB` removed both requested and timed checkpoints on the same workload. Throughput stayed in the same band, so the benefit here is checkpoint-shape reduction rather than raw speed.
- `fod-indexer` metadata writes are now aligned on the staged `COPY` + set-based merge path all the way through source registration, scan-run creation, and import-plan creation. That removes the last direct metadata insert from the shared indexer flow and keeps the low-volume rows on the same replay-friendly merge shape as files, hashes, duplicate sets, and plan entries.
- The PostgreSQL benchmark presets are now shared across local Docker and QNAP for both WAL tuning and planner/autovacuum tuning. `make postgres-benchmarks-wal-preset` and `make postgres-benchmarks-planner-preset` both apply the same server knobs through the Compose layer, so future A/B runs no longer need image edits to compare backends on identical preset values.
- The new shared planner/autovacuum sweep on commit `1fee771` confirms the backend split is still dominated by remote execution cost, not by the planner knobs themselves. Local came in at `11.23 MiB/s` for the WAL burst and `10.159 ms` average connect latency, while QNAP landed at `0.74 MiB/s` and `81.769 ms` connect latency on the same preset; the forced-checkpoint variant remained much cheaper locally (`4.792s` elapsed, `0.063s` checkpoint) than remotely (`62.008s` elapsed, `1.079s` checkpoint).

## 2026-07-01

- `tests/integration/test_fod_indexer_materialize_rollback.py` now covers two rollback boundaries: an early failure while staging `index_import_plan_entries`, and a late failure on the final `materialize_completed` plan update after all materialized entries have been written. The late case keeps the imported plan entries visible for the plan while still rolling back the materialized root and leaving the plan in `materialize_cleaned`. `make test-fod-indexer-materialize-rollback` passed with both cases.
- `test_fod_indexer_usability.py` now treats missing or unauthorized ADB devices as a host-dependent skip for the ADB browse smoke instead of failing the whole usability target. The local `make test-fod-indexer-usability` run passed with `SKIP fod-indexer adb usability`, and the remaining interrupted indexer targets passed when rerun sequentially.
- Makefile runtime targets now reuse the shared debug build stamp `target/.fod-debug-build.stamp` instead of repeating `cargo run` or per-target debug `cargo build` calls. `make build-debug`, `make init`, `make indexer INDEXER_ARGS='--help'`, and `make test-fod-indexer-materialize-rollback` passed after the change. `timeout 12s make mount` reached successful FUSE startup and was then intentionally terminated because `make mount` runs in foreground mode.
- The runtime profile and live runtime change Makefile targets now also reuse `build-debug`. `make test-runtime-profile` no longer emits per-target `cargo build` lines and passed with the expected SELinux mount-label skip on this host; `make change-runtime-list` used `target/debug/fod-change` directly.
- The remaining host-side Makefile smoke targets now rely on the shared `build-debug` stamp instead of repeating per-target debug `cargo build` calls before `cargo test`. The stamp also depends on `fod_version.txt` and SQL/text migration inputs, so version and schema changes invalidate the shared debug binaries. `docker-selinux-acl-smoke` intentionally keeps its in-container build path because it validates the container-local FUSE/SELinux toolchain.
- Python test virtualenv setup now uses `requirements-test.txt` and `.venv/.fod-venv.stamp`, so venv-dependent Makefile targets no longer rerun `ensurepip` and `pip install` on every invocation. The first `make venv` created the stamp, the next dry run reported nothing to do, and the indexer/runtime profile smokes passed without reinstalling Python dependencies.
- The incremental Python venv plan was rechecked on commit `3af1bda`: `make -n venv` and `make venv` both reported nothing to do with the existing stamp, `ensurepip` and `pip install` only appear in the stamp recipe, and `make test-fod-indexer-materialize-rollback`, `make test-fod-indexer-usability`, `make test-runtime-profile`, and `make test-fod-indexer-plan-import-scope` passed without reinstalling Python dependencies. The only host-dependent note was the expected ADB usability skip when no authorized device was present and the expected SELinux mount-label skip on this host.
- `fod-indexer` integration smokes now use per-test source names, `/tmp/fod-indexer-*` paths, source-scoped materialized-root cleanup, and source-scoped indexer cleanup instead of deleting global indexer state. Global duplicate-set and `--all-sources` checks now verify the current test's entries instead of exact database-wide counts, which makes them tolerant of concurrent sources. `python3 -m py_compile tests/integration/test_fod_indexer_*.py tests/integration/fod_indexer_testlib.py`, the sequential indexer smoke set, direct `test_fod_indexer_source_kinds.py`, and `make test-fod-indexer-parallel-smoke` passed on this host. The ADB usability smoke found an authorized device in this run and passed instead of skipping.
- Performance work should now target runtime and SQL behavior rather than Makefile harness overhead: Rust debug binary reuse and Python venv stamping are already in place. The next optimization phase starts from capture infrastructure: environment fingerprints, `perf`, PostgreSQL top statements, WAL/checkpointer snapshots, and optional bpftrace helpers. Do not optimize FUSE, hot SQL paths, or indexer allocation behavior blindly; use the first profiling baseline to choose the highest-impact target.
- The first profiling-infrastructure validation on base commit `ad72bfc` passed locally: `cargo build --profile profiling --workspace`, `make build-debug`, `make venv`, `make init`, `make profile-env`, `make profile-pg-reset`, `make test-fod-indexer-materialize-rollback`, `make profile-pg-top`, `make profile-pg-wal`, `make profile-pg-activity`, and `make profile-pg-io` all completed. Local PostgreSQL 16 used the documented `pg_stat_bgwriter` fallback because `pg_stat_checkpointer` was not available. `perf` and `bpftrace` were present on the host, but only their Makefile dry-runs were executed here because the real targets are host-permission and workload dependent.
- The first local performance baseline on commit `8e8e95f` is recorded in `docs/performance-baselines.md`; raw captures remain under ignored `artifacts/perf/...`. The strongest signal came from the 64 MiB `test-large-copy-benchmark`: `COPY fod_persist_block_stage` plus the two `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` statements consumed about `2259 ms` of PostgreSQL execution time against `3.892 s` measured workload time. Repeated metadata lookups were visible but secondary, WAL/checkpoint counters did not show a new local checkpoint during the large-copy capture, and `perf stat` was blocked by `perf_event_paranoid=4`. The next optimization should therefore benchmark the SQL payload persistence path before FUSE cache/backpressure or durability tuning.
- A sudo follow-up on commit `918f8b1` confirmed that privileged observability works on this host: warm `sudo perf stat` collected counters for `test-large-copy-benchmark`, and sudo bpftrace made `fod-rust-fuse` plus `postgres` syscall activity visible during the workload. Direct `sudo perf stat -- make ...` runs the workload as root and left root-owned build artifacts, which were corrected with `chown`; future privileged profiling should prefer attach or system-wide capture so only the observer has elevated privileges.
- The safe privileged profiling flow is now wired into Makefile: `profile-sudo-perf-stat-system` runs system-wide sudo `perf stat` while the workload runs as the current user and then restores the perf output ownership, and `profile-sudo-bpftrace-syscalls-workload` runs sudo bpftrace while keeping the workload unprivileged.

## 2026-06-26

- `create_data_object()` is now replay-safe through a durable request-token row in `data_object_request_tokens`, so a lost COMMIT no longer doubles `reference_count` on retry. The new `rust_hotpath/tests/transactional_replay_smoke.rs::transactional_commit_disconnect_is_replayed_for_create_data_object` smoke covers the COMMIT-drop path and confirms the token-backed row reuse.
- `promote_hardlink_to_primary()` is now replay-safe through a durable request-token row in `hardlink_promotion_request_tokens`, so a lost COMMIT no longer risks picking a different survivor on replay. The schema version moved to 16 to carry the new marker table.
- The replay smoke suite now also covers `promote_hardlink_to_primary()` with a dropped commit, so the remaining hardlink-promotion path is pinned end to end instead of only by the unit-style functional test.
- `touch_client_session_owner_key()` now has commit-disconnect smoke coverage too, which closes one more metadata-touch path in the transactional replay project without widening the replay envelope.
- `persist_file_blocks_with_crc_flag()` and `persist_file_extents_with_crc_flag()` are now also replay-safe on commit disconnect. The same smoke suite now covers the block, extent, copy-CRC, set-file-size, and lock replay paths, so the bounded replay envelope has one more verified corner without widening the general commit-ambiguity contract.
- The new local-vs-QNAP PostgreSQL comparison on commit `1605384` makes the backend split obvious. On the same 128-file, 64 MiB WAL-pressure workload the local Docker run finished in `5.316s` at `12.04 MiB/s`, while QNAP took `52.863s` at `1.21 MiB/s`. The forced-checkpoint variant showed the same shape: `4.904s` local versus `55.109s` on QNAP.
- The checkpoint counters are now useful. Local forced checkpointing reported `CHECKPOINT elapsed_s=0.059` and `checkpoint_write_time=8.0`, `checkpoint_sync_time=32.0`; QNAP reported `CHECKPOINT elapsed_s=1.379` with `checkpoint_write_time=102.0` and `checkpoint_sync_time=795.0`. That is a strong sign that checkpoint behavior, not just raw WAL volume, deserves attention on the remote backend.
- Connection churn is also much more expensive on QNAP. The local run averaged `8.437 ms` connect latency and `0.541 ms` for the simple query, while QNAP averaged `47.536 ms` connect latency and `5.532 ms` for the same query, with a much worse p95 tail. That makes the benchmark useful for pool/session tuning and for proving network cost separately from FUSE cost.
- The new `postgres-benchmarks-compare` wrapper is the right shape for future PostgreSQL tuning work because it keeps local Docker and QNAP on the same workload and makes the backend gap visible in one run.

## 2026-06-25

- The QNAP PostgreSQL container is still mostly stock `postgres:16-alpine` configuration. The only explicit server tweak visible in the compose stack is `shared_preload_libraries=pg_stat_statements`; key performance knobs such as `shared_buffers`, `max_wal_size`, `checkpoint_timeout`, `wal_compression`, and `synchronous_commit` are currently at default values. That means the current QNAP numbers still mix network latency, FUSE overhead, and default PostgreSQL durability behavior.
- A first tuning check on the same QNAP backend did not show a clean win for `synchronous_commit=off`. On commit `1ce18c4`, the longer `32 MiB` fsync smoke was `27.38 MiB/s` with `FOD_SYNCHRONOUS_COMMIT=on` and `25.73 MiB/s` with `off`, and the sequential fio smoke stayed in the same low-MiB/s band with the write side still slightly better at `on`. For this QNAP sample, the stock durability setting remains the safer baseline.
- Two new PostgreSQL optimization benchmark profiles are now available. On commit `5abf053`, the WAL pressure smoke wrote `64` files of `512k` each and reported `pg_stat_wal.wal_bytes=4753249` with `wal_write=336` and `wal_sync=335`, but no checkpoint counters moved in that short run. The connection churn smoke opened `100` fresh connections and averaged `54.439 ms` for connect plus `9.783 ms` for the simple query, which makes it a better fit for pool/session tuning than for file-path tuning.
- The QNAP Docker backend is usable as a real FOD benchmark target. `make qnap-reset` succeeded against `tcp://192.168.1.11:2376`, the schema initialized cleanly on `192.168.1.11:5432`, and the benchmark run completed with no fresh PostgreSQL ERROR/FATAL/PANIC entries in the QNAP logs during the measured window.
- On commit `4f3fe83` (`FOD 3.1.1: add qnap compose transport preset`), the QNAP-backed throughput smoke reported `1.98 MiB/s` for `make test-throughput` and `1.50 MiB/s` for `make test-throughput-sync`. The sequential fio smoke reported `1561/1280 KiB/s` on block mode and `1600/1306 KiB/s` on extent mode, while mixed and random mixed stayed clearly slower on extent mode than on block mode.
- The Makefile now has a QNAP transport preset for Compose-backed targets. `QNAP=1` or the `qnap-*` wrappers route `docker compose` to `tcp://192.168.1.11:2376` with TLS certs from `~/.docker`, and the same preset switches the PostgreSQL host/user/password values to the QNAP profile. `make qnap-config-show` prints the resolved transport and database values.
- For `msfind`, the current `fod-indexer` stage already covers the shared indexing pipeline; the remaining gap is a stable machine-readable integration surface, not a second engine.
- Added [docs/msfind-fod-indexer-requests.md](docs/msfind-fod-indexer-requests.md) as the place to collect `msfind`-specific requests against the shared indexer core.
- On commit `94d9695` (`FOD 3.1.1: confirm create replay after unique conflict`), the transactional replay smoke still passed, and the create-path replay confirmation now resolves replayed `SQLSTATE 23505` conflicts against the existing natural key instead of failing closed on duplicate inserts. The fresh fio snapshot on this host stayed in the same general band as the previous one: sequential `421 KiB/s` read / `552 KiB/s` write on the block path versus `762 KiB/s` / `1333 KiB/s` on the extent path, mixed `1210 KiB/s` / `1289 KiB/s` versus `236 KiB/s` / `251 KiB/s`, random mixed `830 KiB/s` / `884 KiB/s` versus `181 KiB/s` / `193 KiB/s`, and throughput `11.40 MiB/s` / `10.03 MiB/s`. No regression showed up on the replay-focused change.
- On commit `1ba00b8` (`FOD 3.1.1: organize bounded replay follow-up`), the mounted fio smoke suite still passed for sequential, mixed sequential rw, and random mixed rw workloads. The sequential strace smoke stayed in the same block-vs-extent shape as before, with extent mode still showing more PostgreSQL pressure than the block path on this host.
- The same commit also produced two short throughput references: `make test-throughput` reported `1048576 bytes in 0.185s (5.41 MiB/s)`, and `make test-throughput-sync` reported `1048576 bytes in 0.099s (10.08 MiB/s)`. These are good host-local comparison points, but they are still only single-block smokes.
- The wrapper inventory follow-up now has a first real code pass: `set_file_size()`, `persist_lock_range_state_blob()`, and `replace_lock_range_state_blob_for_owner()` all use `transactional_replayable()`, and the replay smoke suite now covers commit-disconnect replay for all three.
- `purge_primary_file()` is now commit-replayable too. The successful outcome is observable by the missing file row after reconnect, so a lost COMMIT can be replayed once without widening the retry envelope to the reference-counted object creation paths.
- The create-entry family now also uses `transactional_replayable()`, so `create_hardlink()`, `create_symlink()`, `create_directory()`, `create_file()`, and `create_special_file()` can survive a lost COMMIT and then confirm the already-committed natural-key row instead of failing closed on the duplicate insert.
- `adopt_source_data_object()` is now replayable too. The destination file row itself confirms the commit after reconnect when it already points at the source data object with the expected size, which keeps `copy_file_range()` and the materialize adopt path on the bounded replay envelope.

## 2026-06-24

- `fod-indexer` now supports an optional `allow_extensions` filter in `[fod-indexer]`. When enabled, scan and the later indexer stages keep only files with listed extensions, which is a better fit for phone indexing focused on documents, images, and office exports.
- A phone smoke run with a temporary `allow_extensions` list kept 488 of 579 files and reported 91 filtered entries, so the allowlist behaves as expected on the connected Android source.
- Phone scans now skip common Android game cache trees by default, including `DownloadCacheManager`, `PlatformRequestCache`, `ServerRequestCache`, and `UnityCache`, so scan output stays focused on user files such as documents, images, and office exports instead of large game blobs.
- `fod-indexer` source registration and browsing are now split out of the scan engine into `rust_indexer/src/source_registry.rs`, which makes the path-backed core easier to reason about while leaving the CLI behavior unchanged.
- `fod-indexer` source kinds now surface an explicit capability profile in the CLI and keep the current path-backed flow separate from future direct crawlers. The registration contract still uses `--name` as an override, but the supported kinds are now described as path-backed, mirrored, or export-backed instead of being treated as generic blobs of scan logic.
- The source-kind policy is now explicit too: `local` is treated as path-backed, `smb` / `qnap` as mirrored, and `adb` / `github` as export-backed. That keeps the direct-crawler decision visible instead of letting it hide inside registration or scan code.
- `fod-indexer scan --source <name>` now emits periodic progress lines on stderr while walking the tree, so long scans show scanned counts, status counts, current path, and elapsed time instead of staying silent until the final summary.
- `fod-indexer hash --source <name>` now emits periodic progress lines on stderr while hashing candidate files and rebuilding duplicate sets, and the progress helper is shared with scan so the throttle logic stays in one place.
- Zero-length files are now skipped during scan before they enter the index, and the cleanup tree walk also ignores them so old zero-size rows can be pruned instead of kept alive by later passes.
- The remaining `fod-indexer` integration smokes have been aligned with the zero-length skip contract, so the materialize, cleanup-failed, and plan-import tests no longer assert the old `empty.txt` pipeline shape.
- On this host, the connected Android phone is now discovered through `adb shell` first: `fod-indexer source list --kind adb` probes `EXTERNAL_STORAGE` and `SECONDARY_STORAGE`, then maps the chosen shell root to the matching `gvfs` MTP mount so `source add --path` still gets a local filesystem path. The browse output now shows the adb serial and the shell-detected root alongside the local path, and the generated `source add` commands are shell-quoted so paths with spaces can be copied directly.
- `fod-indexer source list` now has a browse mode with `--path <path>`, and `fod-indexer source list --kind adb` now auto-browses the detected ADB runtime root or the Android MTP mount exposed through `gvfs` before falling back to manual overrides. It can show child directories under the mounted root and mark which ones are already added. `fod-indexer source remove --name <name>` is also available for unregistering a source when a path should be dropped.
- A focused usability smoke now covers `fod-indexer source list --kind adb` end to end with a real attached phone and a temporary fake `gvfs` runtime tree, so the device serial, detected Android storage root, browse suggestions, added-path markers, and hidden/cache filtering stay pinned together.
- The usability smoke now also covers `fod-indexer clean --source <name>` with a stale-row scenario, so the CLI keeps showing dry-run previews, real cleanup counts, and unchanged source trees while it prunes orphaned index rows and plan entries.
- `fod-indexer source list [--kind <kind>]` is now available, so registered source adapters and their canonical root paths can be inspected before scan or materialization. The `--kind adb` filter works the same way as the other adapter kinds and keeps the listing focused on one source family.
- `fod-indexer source add` now shows an explicit usage line with `--path <PATH> [--name <NAME>] [--kind <KIND>]`, so the help makes the adapter choice visible instead of hiding it behind a generic `[OPTIONS]` placeholder. The `adb` and `github` kinds are documented as adapter kinds on top of a path-backed root, not as direct remote crawlers yet.
- `fod-indexer` now reads an optional `[fod-indexer]` section from `fod_config.ini` for skip filters. `skip_hidden`, `skip_components`, `skip_prefixes`, and `skip_paths` now control which source paths are omitted, and the INI parser itself is shared through `rust_runtime` instead of being duplicated in multiple crates.
- `fod-indexer source add` now accepts path-backed source kinds for `local`, `smb`, `qnap`, `adb`, and `github`, with kind-aware name suggestions for mounts, ADB serials, and git remotes. The current adapter layer still walks filesystem roots, so the new kinds are metadata and naming hooks rather than direct remote crawlers.
- Hidden dotfiles and common cache/build directories are now skipped during scan, hash, plan-import, materialize planning, duplicate-report rebuilds, and cleanup-tree walks. That keeps paths such as `.bashrc`, `.venv`, `.git`, `node_modules`, `target`, and `build` out of the index and out of the duplicate-set view.
- `DbRepo::query_rows_text()` now participates in the bounded replay path for read-only SQL, so the indexer's plan/report/cleanup row-fetching paths can retry once after a transient PostgreSQL disconnect. The broader write-side replay follow-up remains open.
- `DbRepo::exec()` now participates in the same bounded replay path for the idempotent replayable command set, so safe indexer writes such as status updates and idempotent upserts can retry once after a transient disconnect. Non-idempotent transactional replay is still open.
- `acquire_flock_lease()` now treats the `lock_leases` upsert as replay-safe too, and it carries a durable `request_token` on the row, so one more lock-acquisition path can survive a transient disconnect without broadening retry to the rest of the transaction.
- `touch_data_object()`'s pure metadata touch and `adopt_source_data_object()`'s source/destination confirmation are now replay-safe too, which closes two more write paths in the hot SQL layer without touching the non-idempotent counter updates.
- `purge_primary_file()` now treats the idempotent row reassignment updates for `data_blocks`, `data_extents`, and `copy_block_crc` as replay-safe too, so the shared-object cleanup path can recover one step further after a transient disconnect without broadening retry to the non-idempotent reference-count decrement.
- Bounded transactional replay now also covers commit-disconnect recovery for the safe schema bootstrap and cleanup paths: `ensure_lock_schema()`, `ensure_client_session_schema()`, `prune_lock_leases()`, `prune_lock_range_leases()`, `prune_expired_client_sessions()`, and `touch_client_session_owner_key()` all use the commit-replay helper now, and `rust_hotpath/tests/transactional_replay_smoke.rs` proves the `prune_lock_leases()` retry path after a dropped `COMMIT`.
- `persist_copy_block_crc_rows()` is now also commit-replayable, and the replay smoke suite proves it by dropping `COMMIT` mid-flight and then confirming the `copy_block_crc` row count on the direct connection.
- `persist_file_blocks_with_crc_flag()` and `persist_file_extents_with_crc_flag()` are now also commit-replayable. The block smoke proves the block path end to end, and the extent smoke now runs with `maintain_copy_crc_table = true` after fixing the binary COPY field widths for `copy_block_crc` to match the `INTEGER` schema columns.
- The bounded retry cleanup follow-up is now closed as a separate maintenance item: the current replay envelope is stabilized, and the remaining ambiguous-commit work stays isolated in `docs/transactional-replay-project.md` instead of widening the default retry path.
- The broader transactional replay follow-up is now split into `docs/transactional-replay-project.md`, so the current bounded retry baseline stays stable while the remaining commit-ambiguity work gets its own project scope.
- The transactional replay project now has a concrete inventory of transactional call sites in `rust_hotpath/src/pg.rs`, split into replay-safe, confirmation-bound, and out-of-envelope groups. That gives the next phase a real map instead of a vague backlog item.
- The first replay envelope is now defined too: only transactions with a stable request identity and a durable post-reconnect state probe can be auto-replayed, and ambiguous commit outcomes stay fail-closed unless the marker already proves success.
- `rust_hotpath/tests/transactional_replay_smoke.rs` now covers both requested smokes: body disconnect replay for `create_directory()` and commit disconnect confirmation through a durable `request_token` probe after reconnect.
- The transactional replay smoke suite now also covers a multi-statement `create_file()` body, and the proxy waits until the extended-protocol `Execute` phase before forcing the disconnect. That makes the retry probe closer to real in-flight SQL instead of only stopping at parse time.
- `fod-indexer materialize` now does a best-effort automatic rollback for partial import roots when a non-dry-run run fails after the root has been created. `cleanup-failed` stays available as the manual fallback when that rollback cannot finish.
- A dedicated failure-path smoke now forces a partial `materialize` failure with a temporary PostgreSQL trigger, verifies that the partial import root disappears automatically, and pins the `materialize_cleaned` plan state after rollback.
- A new user-journey smoke now pins the CLI experience itself: top-level help examples, command-specific usage errors, source browse suggestions, scan/hash progress lines, duplicate reporting, and dry-run planning all stay visible from the user's point of view.
- `index_import_plan_entries` now inserts through a replay-safe `DELETE + INSERT` sequence, so a transient disconnect during `fod-indexer plan-import` or `materialize` no longer leaves duplicate plan-entry rows behind on retry.
- `index_scan_runs` and `index_import_plans` now store explicit `request_token` values and use `ON CONFLICT` on that token, so a transient disconnect can retry the same scan or plan creation without minting a duplicate running row.
- The indexer replay change required schema version 14, which adds the new request-token columns and the missing `updated_at` field for `index_scan_runs`.
- The replay classifier in `rust_hotpath/src/pg.rs` now normalizes SQL whitespace before matching replayable command patterns, so multi-line Rust-built SQL statements can still qualify for bounded retry.
- `fod-indexer` help now includes per-command descriptions and concrete usage examples, and `materialize` still fails closed during validation if a source file has disappeared, so it does not create imported data after a missing-file error.
- `materialize` now says explicitly in both help text and runtime validation errors that missing or changed files abort before any imported data is created, which makes the fail-closed behavior visible without changing the import contract.
- `fod-indexer scan`, `fod-indexer plan-import`, and `fod-indexer materialize` now preflight the indexer schema and refuse to start on databases that are still missing the `request_token` migration, so the user gets a direct upgrade hint instead of a raw PostgreSQL missing-column error.
- `fod-indexer materialize --dry-run` is now a read-only preview: it validates the current indexed state and source files, but it does not refresh scan/hash rows, create an import plan, or write any materialized files.
- `fod-indexer` now accepts positional source shorthand for `scan`, `hash`, `plan-import`, and `materialize` while keeping the explicit `--source` form. That makes the CLI friendlier for interactive use without changing the documented contract.
- `fod-indexer source add` now defaults the source name to the current hostname when `--name` is omitted, but explicit `--name` remains available as an override for unsupported sources or forced naming.
- `fod-indexer clean --source <name> --dry-run` now previews stale-index pruning for registered sources, and a real cleanup removes missing or now-ignored file rows plus dependent plan entries before refreshing duplicate metadata.
- `tests/integration/test_fod_indexer_source_kinds.py` now covers the source-kind matrix end to end for local, smb, and github roots. It verifies hidden/cache pruning in browse mode, source-scoped scan/hash/materialize flows, and cleanup after a source root disappears. One practical lesson from the smoke: `hash --source` keeps source-scanned counts local, but duplicate-set totals and cleanup plan-entry counts reflect broader database state, so future source-scoped tests should avoid exact assertions on those global totals.
- `docs/fod-indexer.md` now spells out `fod-indexer` as the shared indexing core for FOD, and `docs/msfind-fod-indexer-requests.md` now frames future `msfind` work as requests against that core instead of a separate indexing pipeline.
- The adapter boundary is now explicit too: the current `local`, `smb`, `qnap`, `adb`, and `github` kinds stay on the shared filesystem-root flow, and none of them gets a direct remote crawler in the core. Any future non-path-backed source kind should be treated as a separate adapter project instead of broadening the existing flow.

## 2026-06-23

- Read-only SQL paths in `rust_hotpath/src/pg.rs` now participate in the bounded replay path too. The classifier treats `SELECT` and recursive `WITH` queries as replayable, so helper reads such as `query_config_value()`, xattr lookups, directory listings, and prepared lookup statements retry once on a fresh connection when the first socket dies mid-flight.
- Transactional SQL paths in `rust_hotpath/src/pg.rs` now retry once after a replayable disconnect. The transaction helper tags only begin/body failures as replayable, then `with_connection(...)` reconnects and reruns the closure once. Plain SQL errors still surface without retry, so full arbitrary in-flight SQL replay remains open.
- The replay helper is covered by unit tests that verify the replayable marker is stripped, the read-only classifier recognizes `SELECT` and recursive `WITH` queries, a replayable closure runs twice and then succeeds, and a normal error is not retried.
- Idempotent command SQL paths now participate in the bounded replay path too. Lock heartbeats, metadata touch/rename/delete helpers, and xattr upserts/deletes use `exec_command_params(...)` so a disconnect can retry the same safe command once; `remove_xattr_for_owner()` now returns the correct deleted-row count via `DELETE ... RETURNING 1`. The new unit test pins the safe replay classifier against a non-replayable journal insert, and `cargo fmt --all` plus `cargo test -p fod-rust-hotpath` both passed after the change.
- The replay boundary now also covers the safe no-parameter command path and two more transactional inserts: `exec_command(...)` now retries replayable command SQL after a disconnect, `touch_client_session_owner_key()` can replay its owner-key upsert, and the lock-range state writers can replay their `INSERT INTO lock_range_leases` steps. That lets the session/lease cleanup path survive a transient disconnect more cleanly without broadening replay to the still-unsafe session-registration insert. The hot-path package tests passed again after this extension.
- `fod-indexer cleanup-failed --plan <id>` is now available for failed materialization cleanup. The new smoke test covers duplicate payload reuse, unique payloads, and a zero-length file, and it verified that a failed import root can be removed cleanly while the source tree stays unchanged.
- `fod-indexer cleanup-failed` now preserves shared data objects by reassigning their storage rows to a surviving outside file, and the cleanup smoke verifies that a shared top-level file still reads correctly after failed-import cleanup.
- The root Cargo workspace version now matches `fod_version.txt`, so `cargo metadata` and the runtime version label no longer advertise different FOD release numbers.
- `make test-fod-indexer-smoke` is now the preferred Makefile entrypoint for the materialize end-to-end smoke, and it reuses the existing comprehensive Python integration test.
- `make test-all-full` now gets past the runtime-profile cache-line expectation after the `read_cache_eviction_policy=fifo` adjustment, but it still stalls later in the recovery standby smoke on this host because `pg_basebackup` hits `FATAL: no pg_hba.conf entry for replication connection ... no encryption`. That blocker looks environment-specific and is unrelated to the cleanup guard.
- The targeted cleanup and indexer smokes passed after the shared-object fix: `cargo fmt --all`, `cargo check --workspace`, `cargo test -p fod-rust-indexer`, `make test-fod-indexer-smoke`, and `make test-fod-indexer-cleanup-failed`.
- `make test-runtime-reload` now covers the live config control plane end to end: it resets the database to a known schema-admin secret, accepts `read_ahead_blocks` as a live reload, rejects mount-only keys such as `fopen_direct_io`, and restores the original runtime snapshot before exit.
- `FOD_PG_HOST`, `FOD_PG_PORT`, `FOD_PG_DBNAME`, `FOD_PG_USER`, and `FOD_PG_PASSWORD` now override the PostgreSQL endpoint from the `[database]` config section, so the same checkout can mount against a remote server such as the QNAP preset at `192.168.1.11:5432` without editing `fod_config.ini`. The new `make init-qnap` and `make mount-qnap` helpers prefill that remote profile for local smoke runs.

## 2026-05-06

- `test_runtime_profile` and `test_runtime_profile_extents` need `sudo` on a host that exposes `/dev/fuse`; the sandbox can hide that device even when the host has it.
- The recovery `role=auto` smoke is valid only when the standby is prepared with `pg_basebackup`; the correct mount contract is `recovery_mode=true`, `read_only=true`, and `lock backend=Memory`.
- Recovery setup should use a unique temporary backup path inside the primary container; reusing a fixed path caused stale cleanup residue and `pg_basebackup` failures.
- SELinux mount-label coverage is host-dependent; when FUSE rejects the label options with `EINVAL`, the test should skip instead of failing.
- `test_mount_suite` permission checks must run negative cases as `nobody`, not as root; root makes `os.access()` and `PermissionError` misleading.
- `strace` on the POSIX/FUSE mount suite did not reveal unexpected mount-time syscalls; the observed failures were only the expected permission and lookup errors from normal FUSE activity.
- FIO results so far keep `enable_extents` opt-in: sequential writes benefit on larger sizes, while mixed and random mixed workloads do not show a stable win.
- Fresh mixed fio runs on this host still favor the block path: `test-fio-mixed-io` reported about `1205 KiB/s` read and `1282 KiB/s` write for block mode versus about `274 KiB/s` read and `291 KiB/s` write for extent mode, and `test-fio-random-mixed-io` reported about `902 KiB/s` read and `960 KiB/s` write for block mode versus about `255 KiB/s` read and `271 KiB/s` write for extent mode. That reinforces the current decision to keep `enable_extents` opt-in.
- `PersistPlan` remains the selection boundary in `rust_hotpath/src/persist_plan.rs`, but `choose_persist_execution_plan(PersistPlanInput { ... })` now returns `PersistExecutionPlan` with explicit `PersistPayloadPlan::Blocks(...)` / `PersistPayloadPlan::Extents(...)` stages; `write_buffer` consumes that plan directly.
- `PersistExtentRow` is now the explicit extent-shaped persist row with `start_block`, `block_count`, `used_bytes`, and `payload`; `persist_file_extents_native(...)` is the new repo boundary for the extent path, while the current implementation still validates the row coverage and adapts to the existing block engine underneath.
- `Extent::new_checked(...)` is now covered by a unit test that rejects reversed ranges, so the public constructor contract is explicit in both release and test builds.
- `Extent::new_checked(...)` now guards invalid ranges in release builds; `coalesce_sorted_blocks(...)` uses it and only expects ordered spans that the planner already guarantees.
- `release-lto` is an opt-in Cargo profile for final installs; `install-root-scripts` and `install-rust-hotpath` now follow `FOD_CARGO_PROFILE`, while the default `release` path stays unchanged for everyday builds.
- `fod.change` now validates schema-admin auth, accepts only the reloadable runtime subset, and persists a canonical live-change snapshot in PostgreSQL; the running FUSE consumer still needs to be wired to that snapshot before changes apply without remount.

## 2026-05-07

- `data_extents` is now the native extent storage table, and `persist_file_extents_native(...)` writes extent rows directly instead of re-expanding them into `PersistBlockRow`; the read path now prefers extents first and falls back to block storage only when no matching extent rows are found.
- The current extent path is still intentionally narrow: `choose_persist_plan(...)` only selects extents for full-file sequential coverage so the PoC stays clear of merge/split overlap handling.
- `statfs` and file-block counting now include extent-backed data, so extent-backed files no longer disappear from size/block visibility just because they skipped `data_blocks`.
- The PG-backed flock smoke must expire `lock_range_leases`, not `lock_leases`; `getlk` reads the range-lease blob source, and `F_GETLK` on the same fd does not prove ownership of a lock, so reacquire checks need to observe a different mount or process.
- `persist_file_extents_native(...)` removes stale `data_blocks`, `data_extents`, and copy-CRC rows for the affected data object before inserting native extent rows, so the extent path isolates the new storage representation instead of leaving old block rows behind.
- `schema_status` on an existing database now reports migration `0012_data_extents.sql` and schema version `12`, so migration 12 is wired through `mkfs` status output as well as the schema upgrader.
- The extents-enabled runtime still passes the `statfs/use_ino` smoke, so the FUSE statfs path remains healthy after the migration 12 extent storage change.
- The fio smoke matrix after migration 12 still passes for sequential, mixed sequential, and random mixed workloads, which confirms the new extent storage path does not break the existing block path or the opt-in extent path on the current runtime.
- `test-postgresql-requirements` now has explicit `autocommit=off` and `autocommit=on` Makefile targets, and both pass against the local PostgreSQL smoke; the test itself now validates the psycopg2 autocommit mode it is asked to exercise.
- The fio smoke targets already present in `Makefile` (`test-fio-sequential-io`, `test-fio-mixed-io`, `test-fio-random-mixed-io`) still pass, so the current benchmark coverage is intact while the PostgreSQL autocommit smoke was expanded.
- Block writes now explicitly delete stale `data_extents` rows before persisting `data_blocks`, so a file that switches from extent-backed to block-backed storage no longer leaves extent-first reads shadowing the fresh block data.
- After the block-write extent cleanup, the sequential and mixed fio smoke tests still pass, which means both the block and extent write paths remain healthy under the current runtime.
- The `data_extents` cleanup can stay keyed by `data_object_id` only; the existing `(data_object_id, start_block)` and `data_object_id` indexes are enough, so `id_file` was a redundant predicate for the write-path cleanup.
- Extent writes are symmetric with block writes now: `persist_file_extents_native(...)` clears `data_blocks` before inserting extent rows, and `persist_file_blocks_with_crc_flag(...)` clears stale `data_extents` before block persistence, so representation switches do not leave the other storage form behind.
- `rust_hotpath/tests/pg_query.rs` now has a regression for both `extent-backed -> block-backed` and `block-backed -> extent-backed` writes, and it checks both block ordering on readback and cleanup of stale rows in the opposite storage table.
- The Rust PostgreSQL integration tests need access to the local Postgres host port; inside the sandbox they fail at connection setup, so `cargo test -p fod-rust-hotpath --test pg_query` has to run with elevated network access in this environment.
- Boundary profiling is now available on the FUSE↔DB edge: `fuse_read_total_us`, `fuse_write_total_us`, `repo_fetch_block_range_us`, `repo_persist_blocks_us`, `read_cache_lock_us`, `write_state_lock_us`, `reply_data_us`, and `reply_write_us` are accumulated and printed on FUSE shutdown, and `test_runtime_profile` still passes after wiring them in.
- The profiler report is now easier to read because `fod_test_cleanup` prints a multi-line boundary summary. On a short `make test-fio-sequential-io` run with `FOD_PROFILE_IO=1`, the block path reported `fuse_read_total_us=52623`, `fuse_write_total_us=40812`, `repo_fetch_block_range_us=3640`, `repo_persist_blocks_us=0`, `read_cache_lock_us=2`, `write_state_lock_us=3`, `reply_data_us=82`, and `reply_write_us=277`; the extent path reported `fuse_read_total_us=37505`, `fuse_write_total_us=71111`, `repo_fetch_block_range_us=5860`, `repo_persist_blocks_us=0`, `read_cache_lock_us=0`, `write_state_lock_us=1`, `reply_data_us=91`, and `reply_write_us=318`. This suggests a larger write-heavy smoke is still needed if we want to isolate DB persist cost more clearly.
- Live config changes should start from a clearly defined reloadable subset, not from mount semantics. The first-pass helper in `rust_runtime` now isolates tuning knobs such as `profile`, `log_level`, cache TTLs, read/write worker thresholds, and copy-dedupe switches; mount-only settings such as `read_only`, `default_permissions`, `lazytime`, `sync`, `dirsync`, `use_fuse_context`, `fopen_direct_io`, `fuse_writeback_cache`, SELinux/ACL labels, and lock backend selection should remain startup-only until a separate, explicit reload path exists.
- `fod.change --list` and `fod.change --get KEY` now read the effective live reload snapshot, meaning the baseline INI and the persisted `runtime_overrides` row are merged before inspection; `--set` composes from that same effective snapshot, and the command stays quiet on read-only paths because `runtime_overrides` is only created when a write actually needs it.
- The live reload transport is now wired into the running FUSE process. A mounted smoke confirmed that `fod.change --set read_ahead_blocks=5` produced the `FOD runtime reload applied` log line in the mount log, and the next `fod.change --get read_ahead_blocks` observed the updated effective value before restoring it to `4`. Mount-only settings still remain startup-only, but the reloadable subset is now actually consumed without remounting.
- `make test-runtime-profile` still passes after the live reload wiring, and the SELinux mount-label branch remains host-dependent and skips cleanly on this machine when FUSE rejects the label options with `EINVAL`.
- `FOD_VERSION_LABEL` is now sourced from `fod_version.txt` through `rust_runtime/build.rs`, and `Makefile` reporting uses the same file so version bumps no longer need to be duplicated between build metadata and runtime output.
- `fod_version.txt` is the sole source of FOD numbering; Cargo package versions remain build metadata for the Rust workspace and should not be treated as the release-number source of truth.
- `fopen_direct_io=1` is a large regression on both read and write in the current fio sequential smoke; the extent path suffers much more than the block path, so direct I/O should stay a diagnostic/compatibility toggle rather than a performance baseline.
- The next useful performance step is finer-grained profiling under `FOD_FOPEN_DIRECT_IO=1`, because it exposes the real hot path once kernel/page-cache masking is removed. The new timers now split read/write/cache/extent costs more tightly, so further tuning should be guided by those internal timings rather than by direct I/O as a production target.
- The new direct-I/O timers are intentionally inclusive per stage, not additive. They are useful for hotspot localization and for comparing block vs extent behavior, but they should not be summed as if they were disjoint sub-costs.
- After the first direct-I/O cleanup pass, the 4 MiB `FOD_FOPEN_DIRECT_IO=1` sequential smoke still shows the same shape but with a clearer split: block path printed `fuse_read_total_us=5623155`, `fuse_write_total_us=1385764`, `read_block_map_us=1589113`, `assemble_read_slice_us=292614`, and `repo_fetch_block_range_us=232754`; extent path printed `fuse_read_total_us=34858233`, `fuse_write_total_us=171943630`, `read_block_map_us=25370750`, `assemble_read_slice_us=651871`, and `repo_fetch_block_range_us=22561833`. The take-away is unchanged: direct I/O is a microscope that exposes where the current path hurts, especially on extent writes, and the new exact-size read assembly plus pre-reserved extent payloads already trimmed some obvious overhead, but they did not change the fundamental shape of the bottleneck.
- After the read-cache `Arc<[u8]>` cut, the same 4 MiB `FOD_FOPEN_DIRECT_IO=1` sequential smoke improved to block `fuse_write_total_us=1753480`, `fuse_read_total_us=4786009`, `repo_fetch_block_range_us=227342`, and extent `fuse_write_total_us=111350791`, `fuse_read_total_us=17885048`, `repo_fetch_block_range_us=12281426`. The direct-I/O mode is still far slower than the normal path, but sharing cached read blocks and keeping the read slice assembly zero-copy on cache hits clearly helped the block and extent read paths, while extent writes remain the main bottleneck.
- After moving the ordered-read work out of `assemble_read_slice()` and returning ordered block vectors from `read_block_map()`, the same 4 MiB `FOD_FOPEN_DIRECT_IO=1` sequential smoke improved again to block `fuse_write_total_us=1426788`, `fuse_read_total_us=4012986`, `read_block_map_us=680311`, `assemble_read_slice_us=20055`, `repo_fetch_block_range_us=188679`, and extent `fuse_write_total_us=114083764`, `fuse_read_total_us=16870253`, `read_block_map_us=13013347`, `assemble_read_slice_us=20115`, `repo_fetch_block_range_us=12525504`. The block path still shows a clear read-side win from removing the extra ordering work, but extent writes are still the dominant wall-clock bottleneck and still need a different optimization hypothesis than further read-path sorting cleanup.
- After the full-block overwrite fast path removed the stale-block clone on writes that replace an entire block, the latest 4 MiB `FOD_FOPEN_DIRECT_IO=1` smoke stayed in the same overall shape: block `fuse_write_total_us=1383550`, `fuse_read_total_us=4978730`, `read_block_map_us=831036`, `assemble_read_slice_us=26213`, `repo_fetch_block_range_us=190994`, and extent `fuse_write_total_us=109133128`, `fuse_read_total_us=16721936`, `read_block_map_us=12755055`, `assemble_read_slice_us=22612`, `repo_fetch_block_range_us=12201486`. The new fast path removes a useless copy on full overwrites, but the direct-I/O smoke still says the same thing: extent writes dominate wall-clock time, and the read side still has enough leverage to matter when we chase total throughput.
- After making `flush_write_state()` move the buffered block map into `recent_write_blocks` instead of cloning it, the latest 4 MiB `FOD_FOPEN_DIRECT_IO=1` smoke settled at block `fuse_write_total_us=1445256`, `fuse_read_total_us=5528339`, `read_block_map_us=876342`, `assemble_read_slice_us=26761`, `repo_fetch_block_range_us=208573`, and extent `fuse_write_total_us=109133128`, `fuse_read_total_us=16721936`, `read_block_map_us=12755055`, `assemble_read_slice_us=22612`, `repo_fetch_block_range_us=12201486`. This is a small but real write-path cleanup, yet the main story is unchanged: direct I/O still exposes a very expensive extent-write path, while the read side remains important enough that reducing its overhead changes the overall profile measurably.
- After adding a single-block cache-hit fast path in `read()` and throttling default atime touches so repeated reads do not hammer PostgreSQL, the latest 4 MiB `FOD_FOPEN_DIRECT_IO=1` smoke moved block read to `2.2 MiB/s` and dropped the block-side read hot path to `fuse_read_total_us=1789189`, `read_block_map_us=281729`, and `repo_fetch_block_range_us=271743`. Extent reads are still much slower at `141 KiB/s` and remain dominated by `read_block_map_us=27491422` / `repo_fetch_block_range_us=27480547`. The important takeaway is that read-side metadata updates were a real bottleneck in direct I/O, and throttling them helped the block path a lot, but extent writes and extent-backed reads still need deeper work.
- The shared repo-fetch `Arc<[u8]>` path kept the block-side read fast path without needing to widen the recent-write retention window. The latest 4 MiB direct-I/O smoke after reverting that extra retention tweak settled at block `fuse_write_total_us=4165896`, `fuse_read_total_us=2475601`, `read_block_map_us=480617`, `repo_fetch_block_range_us=467440`, and extent `fuse_write_total_us=283457371`, `fuse_read_total_us=32594382`, `read_block_map_us=31031761`, `repo_fetch_block_range_us=31020472`. The key conclusion is that sharing repo fetch blocks by `Arc` was the useful optimization to keep, while widening the recent-write retention window was not a stable win and was not worth keeping.
- Switching extent reads to a binary `bytea` result format removed the base64 roundtrip and turned out to be a major win. The latest 4 MiB smoke with `FOD_FOPEN_DIRECT_IO=1` now lands at block `fuse_write_total_us=1535135`, `fuse_read_total_us=955559`, `read_block_map_us=183453`, `repo_fetch_block_range_us=175134`, and extent `fuse_write_total_us=15609659`, `fuse_read_total_us=1519041`, `read_block_map_us=364902`, `repo_fetch_block_range_us=357071`. This is the first direct-I/O change that made extent reads look genuinely healthy instead of merely less bad; the remaining extent bottleneck is now predominantly write-side, not read-side.
- Keeping extent block slices zero-copy in the helper layer preserved that gain and made the helper path itself cheaper. The latest 4 MiB smoke with `FOD_FOPEN_DIRECT_IO=1` now lands at block `fuse_write_total_us=1550407`, `fuse_read_total_us=884268`, `read_block_map_us=132663`, `repo_fetch_block_range_us=125743`, and extent `fuse_write_total_us=14576426`, `fuse_read_total_us=1461691`, `read_block_map_us=365770`, `repo_fetch_block_range_us=357677`. The important part is that extent reads are now close to the normal run, while extent writes remain the real throughput problem to solve next.
- The single-extent direct path and small-extent staging fast path did not materially change the direct-I/O shape on this host. The latest smoke still lands around block write `2.1 MiB/s`, block read `3.2 MiB/s`, extent write `260 KiB/s`, and extent read `1.0 MiB/s`, so the remaining write-side cost is deeper than the stage-table boundary and needs a different hotspot than just data_extents staging.
- The new `strace -f -c` smoke confirms the same picture at the syscall level. In block mode the hot syscalls are `wait4`, `futex`, `restart_syscall`, `sendto`, `poll`, and `recvfrom`; in extent mode `recvfrom` becomes even more dominant, which points to PostgreSQL round-trips and synchronization as the remaining bottleneck rather than buffer copying. This is a good complement to the FUSE timers, but it does not replace them.

## 2026-05-08

- Hot-path validation should now prefer `FOD_PROFILE_IO=1` plus `make test-fio-sequential-io-strace` for timing and syscall shape checks; that pair makes it easier to see whether a change helped the FUSE callback, copying, locks, or the PostgreSQL round-trip path.
- Removing the single-extent direct CRC fast path did not materially change the direct-I/O shape on the current host; the extent path still shows much heavier `recvfrom` pressure than the block path, so the bottleneck remains in DB round-trips and synchronization rather than in the special-case extent CRC branch.
- The latest strace smoke keeps reinforcing the same rule of thumb: if extent mode is still `recvfrom`-heavy, chasing more payload micro-optimizations is less promising than reducing SQL / sync work on the write side.
- The combined extent-cleanup and metadata-update SQL trimmed a bit of syscall pressure from the 1 MiB strace smoke: the block path reported `fuse_read_total_us=423896`, `fuse_write_total_us=582237`, `repo_fetch_block_range_us=63934`, and `recvfrom=3681`/`total=19096`; the extent path reported `fuse_read_total_us=475883`, `fuse_write_total_us=1349577`, `repo_fetch_block_range_us=164738`, and `recvfrom=15715`/`total=31443`. The shape is still the same, but the extent path is now a little less noisy in the syscall trace.
- Switching extent CRC maintenance from a staging-table merge to a direct COPY into `copy_block_crc`, and batching the COPY payload into one buffer, gave a small but real improvement on the same 1 MiB smoke: block stayed around `fuse_write_total_us=379272` / `fuse_read_total_us=543106`, while extent settled around `fuse_write_total_us=1358718` / `fuse_read_total_us=534982`, with `recvfrom` dropping a bit as well. The extent path is still the bottleneck, but this confirms the write-side round-trip path is the right place to keep trimming.

- VS Code debug launches should reset the FUSE mountpoint with `fusermount -uz` before and after a session, because stale transport endpoints can make the next debug run fail with `Transport endpoint is not connected`.
- Makefile now exposes `change-runtime-list`, `change-runtime-get`, and `change-runtime-set` as direct `fod.change` helpers for the reloadable runtime snapshot. They intentionally depend on `up wait` instead of `init`, because `init` can create or skip schema state with a random schema-admin password and would be the wrong prerequisite for a live configuration edit.

- Makefile now exposes `reload-runtime` (with `change-runtime-sync` as an alias) for pushing reloadable config values into a running mount through `fod.change --sync-config`; the sync path no longer needs the schema-admin password because it only replays the reloadable snapshot from `fod_config.ini`, so reloadable tuning no longer needs a remount when the change is already represented in `fod_config.ini`.

- `persist_copy_block_crc_extent_rows_on_conn(...)` now reuses the `data_object_id` already returned by `detach_shared_data_object_on_conn(...)`, so extent CRC maintenance no longer pays an extra lookup round-trip during extent writes. The normal `FOD_PROFILE_IO=1` sequential smoke still passes after that cut, but `make test-fio-sequential-io-strace` currently still needs a test-side adjustment because the extent run under strace is treated as a block-storage case and fails on the expected extent PoC log.

- `make test-fio-sequential-io-strace` now keeps the extent expectation active for `FOD_STRACE=1`; the earlier failure was only the test-side branch that incorrectly treated the extent run like block-storage mode. After fixing that condition, the strace smoke passes for both block and extent cases on the current host.

## 2026-05-20

- The Docker compose stack now takes container names from `CONTAINER_*` environment variables with the current `fod-postgres` and `fod-selinux-acl` values preserved as fallbacks. That keeps the repo compatible with the shared port/name registry in `/home/wojtek/git/config` without forcing a behavior change on the existing smoke targets.

## 2026-05-22

- `mount.fod` and `rust_fuse/tests/support.rs` now prefer workspace `target/debug` and `target/release` before the legacy `rust_mkfs/target/...` paths, so fresh workspace builds are no longer shadowed by an older `/usr/local/bin/fod-bootstrap`.
- The installed `mount.fod` wrapper also needs to discover the checkout root by walking up from `PWD` first and then from `SCRIPT_DIR`; otherwise a copy installed under `/usr/local/sbin` can miss the workspace `target/...` tree and fall back to `PATH` even when the local build is present.
- The installed sequential smoke passed with both `FOD_PROFILE_IO=1` and `FOD_STRACE=1` against `/usr/local/bin/fod-bootstrap` and `/usr/local/bin/mkfs.fod`. On this host the hot syscalls stayed in the expected FUSE/PostgreSQL shape (`wait4`, `futex`, `restart_syscall`, `sendto`, `poll`, `recvfrom`), and the measured block/extents timings did not expose a wrapper-specific regression.
- The biggest FUSE optimization cost is still the kernel/userspace boundary: the kernel docs and recent FAST papers keep pointing to context switches, request copying, and userspace round-trips as the main tax. Optimizations that win in this area are the ones that reduce crossings, batch work, or bypass the daemon on the hot path.
- Metadata and locking are usually the next bottleneck after the boundary tax. The manycore filesystem papers consistently show directory serialization, inode/file locks, journaling, and metadata updates as scalability limiters, often more than raw data transfer.
- Caching and passthrough help, but they are not free. Writeback cache can make writes look fast, yet it changes consistency assumptions; passthrough reduces userspace traffic, but it raises privilege, resource-accounting, and stacking concerns; io_uring helps with transport, but it does not remove all `/dev/fuse` traffic and is still incomplete.
- Benchmark results on FUSE are workload-sensitive enough that a single throughput number is not useful. The same framework can look close to native on some workloads and dramatically worse on metadata-heavy or highly concurrent ones, so any optimization needs separate data-path, metadata-path, and contention-focused measurements.
FOD checklist for the common FUSE/FS bottlenecks:

| Problem | `strace` signal | `FOD_PROFILE_IO` signal | `fio` signal |
| --- | --- | --- | --- |
| Kernel/userspace boundary tax | Lots of `sendto`, `recvfrom`, `poll`, `wait4`, `futex`, and `restart_syscall` around the hot path | High `fuse_read_total_us`, `fuse_write_total_us`, `repo_fetch_block_range_us`, and reply-time totals | Throughput drops most on small I/O, `O_DIRECT`, and high-round-trip workloads versus native |
| Metadata and locking | Syscall bursts around create/unlink/readdir plus heavy `futex`/wait activity | High `read_block_map_us`, `write_state_lock_us`, `read_cache_lock_us`, and `repo_persist_blocks_us` | Poor scaling with threads; small-file and directory-heavy workloads degrade first |
| Cache / writeback / passthrough tradeoffs | Fewer daemon round-trips on the fast path, but extra `READ` on writes or more `fsync` / `close` pressure when cache semantics are involved | Write totals improve, but hidden read or sync costs can reappear when cache assumptions are violated | Cached writes may look much faster, while correctness-sensitive or direct-I/O cases can regress sharply |
| Workload sensitivity | Different syscall shapes across sequential, mixed, random, and threaded runs | Compare the same counters across workloads, not just one run | A win in one benchmark class can hide a regression in another, so keep separate sequential, mixed, random, and contention tests |

## 2026-06-13

- The optional `test-admpanch-trace` Makefile helper needs both an exported `ADMP_INI` and an explicit `ADMP_TRACE_ENV` passthrough for targets that run through `sudo env`; otherwise the tracer config disappears before the test process starts. The local `admpanch_trace.fod.local.ini` profile is the safe default, while the DB-backed profile stays opt-in.
- `tests/integration/test_root_owned_permissions.sh` also needed its own `sudo -n env` line to accept `ADMP_TRACE_ENV`, otherwise the root-mounted FOD process would not see the tracer config even when the caller exported it. That script is now trace-friendly without changing the actual root-owned permission checks.
- `ADMP_TRACE_ENV` is now exported from `Makefile` and consumed by the shared shell and Rust test helpers, so `fod-rust-mkfs`, `fod-bootstrap`, `test-mount-root-permissions`, and the root-owned permission smokes inherit the trace config without per-target special cases. That keeps the optional trace profile focused on FOD binaries while still letting the permission tests run through their normal `sudo` paths.
- The shared shell helper needed an explicit `env` prefix for `ADMP_TRACE_ENV`; otherwise `test-admpanch-trace` could treat `ADMP_INI=...` like a command in the `fod_testlib.sh` init path. After that fix, `make test-mount-root-permissions`, `make test-root-owned-permissions`, `make test-admpanch-trace ADMP_TRACE_TARGET=test-mount-root-permissions`, and `make test-admpanch-trace ADMP_TRACE_TARGET=test-root-owned-permissions` all passed on this host.
- The DB-backed `admpanch_trace` profile does produce real rows when it is loaded into a non-`sudo` FOD path: `make test-fio-sequential-io` with `LD_PRELOAD=/media/wojtek/virtdata/home/wojtek/git/admpanch_trace/libadmpanch_trace.so` and `ADMP_INI=/media/wojtek/virtdata/home/wojtek/git/fod/admpanch_trace.fod.db.ini` wrote 657 rows into `public.trace` with `close`, `fclose`, `fopen`, `read`, and `write` events. The PostgreSQL container logs did not show a dedicated `admpanch_trace` banner; the visible noise was mainly FOD-side SQL errors in the main database and the earlier auth failures before the trace role/database were provisioned.
## 2026-06-23

- Partial-block writes now fail closed when PostgreSQL cannot load the existing block. `load_write_block_from_repo()` maps repo errors to `EIO` instead of returning zero-filled data, and both `write()` and `copy_file_range()` propagate the failure without publishing the mutated write state.
- The new regression coverage is intentionally small: one unit test checks the helper's error mapping, and `cargo test -p fod-rust-fuse multi_open_unique_handles -- --nocapture` still passes to confirm the normal partial-write path remains intact on a live mount.
- Zero-length writes are already treated as a no-op before `update_write_buffer()` mutates `file_size`; `rust_fuse/tests/mount_smoke.rs::zero_length_write_is_noop` now pins that contract, and `cargo test -p fod-rust-fuse zero_length_write_is_noop -- --nocapture` plus `cargo test -p fod-rust-fuse write_noop -- --nocapture` both pass.
- A live smoke on this host showed that `fcntl.ioctl(..., FICLONE, ...)` still returns `EOPNOTSUPP` on the FOD mount before the daemon sees a `FICLONE` request, so the reflink path is not end-to-end usable yet. The `FICLONE` / `FICLONERANGE` handler code is in place, but the current FUSE stack does not forward that request on this setup; the ioctl follow-up is now the conservative `FS_IOC_SETFLAGS` no-op policy until a real persisted flag model exists.
- A comparative `go-fuse` loopback mount on the same host behaved the same way: `fcntl.ioctl(..., FICLONE, ...)` returned `ENOTSUP` and the tracing `Ioctl()` wrapper never logged a `FICLONE` call. That means the problem is not limited to the current FOD daemon implementation; on this stack and host, the request is short-circuited before userspace even on a minimal userspace filesystem.
- A second comparative mount built directly on `libfuse3` behaved the same way: the test process logged `open` for both source and destination files, but the filesystem `ioctl()` callback never ran for `FICLONE`, and the caller still got `ENOTSUP`. That makes the current conclusion stronger: on this host and kernel/FUSE stack, `FICLONE` is blocked before userspace regardless of whether the daemon is `go-fuse` or `libfuse3`.
- Kernel tracing now pins the cut point more precisely: `do_vfs_ioctl()` sees the real `FICLONE` request code (`0x40049409`) and returns `-ENOTSUP` without ever entering `fuse_priv_ioctl_prepare()`. By contrast, `FS_IOC_GETFLAGS` (`0x80086601`) does reach `fuse_priv_ioctl_prepare()` and then the libfuse callback. So `FICLONE` is rejected in the generic ioctl dispatch path before the FUSE-specific prep stage, not inside the daemon.
- Binary block persistence now participates in the bounded replay path too. `INSERT INTO data_blocks ... ON CONFLICT DO UPDATE` and the matching `copy_block_crc` upsert in the staging merge are replay-safe after a disconnect, and the idempotent file-size updates that keep `data_objects.file_size` and `files.size` aligned now retry once as well. That closes one more gap in the hot-path SQL replay coverage without enabling arbitrary non-idempotent write replay.
- The narrowed replay classifier still passes the hot-path smoke stack: `cargo test -p fod-rust-hotpath`, `FOD_PROFILE_IO=1 cargo test -p fod-rust-hotpath`, and `make test-fio-sequential-io-strace` all passed after the replay coverage expansion.
- `FIGETBSZ` is now answered directly by the FOD daemon from the current mount `blksize`, and a manual smoke confirmed that the mount returns the expected block size while `FIONREAD` still reports the pending read length correctly.
- `FS_IOC_GETFLAGS` is now also handled in the FOD daemon. The current implementation returns a neutral zero flag set because inode flags are not persisted yet, which is enough to keep the ioctl path compatible while the flag policy remains undecided.
- `FS_IOC_FSGETXATTR` now returns a zeroed 28-byte `fsxattr` payload, and `FS_IOC_FSSETXATTR` accepts only an all-zero no-op for now. That keeps the XFS-style ioctl family from falling through to `ENOTTY` while FOD still lacks a persisted xflag model.
- The smoke also showed that the caller can surface the 32-bit `FS_IOC32_GETFLAGS` / `FS_IOC32_SETFLAGS` encodings on this host, so FOD now accepts both the native and compat flag variants instead of assuming a single numeric ioctl spelling.
- `tests/integration/test_ioctl.py` now exercises both `FIONREAD` and the current `FICLONE` path. On this host, the ioctl smoke accepts the unsupported clone result that the kernel/FUSE stack returns before userspace, but it still verifies that a successful clone would preserve the source payload and leave the destination unchanged on the fallback path.
- `FodFuse` already overrides the core path, metadata, xattr, access, lock, copy, and file lifecycle callbacks: `lookup`, `getattr`, `readdir`, `readlink`, `statfs`, xattr CRUD, `access`, `ioctl`, `poll`, `open`, `getlk`, `setlk`, `bmap`, `flush`, `read`, `release`, `setattr`, `mkdir`, `unlink`, `rmdir`, `rename`, `create`, `write`, `copy_file_range`, `mknod`, `symlink`, and `link`. The remaining trait defaults are mostly directory-stream or lifecycle helpers such as `opendir`, `releasedir`, `fsyncdir`, `readdirplus`, `forget`, `batch_forget`, and `destroy`.
- The bounded replay path now also covers the lock and client-session schema bootstrap DDL, including `CREATE TABLE IF NOT EXISTS`, `CREATE INDEX IF NOT EXISTS`, `CREATE OR REPLACE FUNCTION`, `DROP TRIGGER IF EXISTS`, and the safe `ALTER TABLE IF EXISTS` variants used there. That means an interrupted schema init can retry once instead of leaving the mount half-initialized, while the broader non-idempotent transaction replay problem remains open.
- `FS_IOC_SETFLAGS` now follows a conservative policy: zero is accepted as a no-op, and any non-zero flag request gets `EOPNOTSUPP` until FOD has a real persisted flag model. `readdirplus` and `fsyncdir` are still feasible if directory-stream semantics become important, but they are less urgent than the remaining ioctl gap above.
- `recent_write_blocks` now evicts in deterministic sorted-key order instead of relying on `HashMap` iteration order. That keeps the cache behavior reproducible across runs without changing the cache size limit, and the existing mount smokes still pass after the change.
- `write_state_clone_us` now separates the cost of cloning `WriteState` from the lock timings in the direct-I/O profiler. The clone path is measured in `write()`, `copy_file_range()`, `write_state_for_handle()`, and `flush_pending_write_states_for_file_except()`, so `FOD_FOPEN_DIRECT_IO=1` can show whether the pressure is in lock contention or in copying the buffered state itself.
- `fod-indexer` now exists as a dedicated Rust binary with local-source registration, filesystem scanning, staged duplicate detection, and a dry-run import plan over PostgreSQL-backed index tables. The shared `DbRepo::query_rows_text()` helper in `rust_hotpath` is the common query path for it, so the new crate does not need its own libpq wrapper.
- The schema bump to version `13` is wired through `mkfs.fod`, `base_schema.sql`, and `migrations/0013_indexer.sql`, and `cargo test -p fod-rust-mkfs schema_upgrade_non_destructive_password_protected -- --nocapture` passed against the live database after the change.
- `fod-indexer` now also has a real `materialize --source <name>` phase that revalidates size, mtime, inode/device, and full hash before creating a per-run import root in FOD. Canonical payloads are written once and duplicate references reuse the canonical data object when the payload is non-empty; the same flow is exposed through a simple `make indexer` / `make indexer-import` wrapper.
- Materialization cannot assume the database already has a stored `full_hash` for every candidate. The safe path is to recompute the full hash during validation, compare it only when an existing hash is present, and then use that verified hash for the import step.
- The new `test-fod-indexer-materialize` smoke test covers the full source-add/scan/hash/report/plan/materialize pipeline on `/tmp/fod-indexer-src` with duplicate, unique, and zero-length files. It verifies that the source tree stays untouched, `a.txt` and `b.txt` share the same data object, `c.txt` stays unique, and zero-size materialization does not break import.
- The smoke test needed an explicit cleanup of stale `index-source-*` materialization roots before each run, because reruns can otherwise collide with an existing imported directory tree and fail on `uniq_files_parent_name`. The DB assertions also need `search_path TO fod, public` before querying `index_*` tables.
- `fod-indexer plan-import` now requires an explicit scope. `--source <name>` limits the dry-run plan to one registered source, while `--all-sources` keeps the global view. The dry-run summary now prints the chosen scope, and the new scope test pins both the single-source and all-sources paths.
- `ReadBlockCache` now exposes an explicit FIFO/LRU eviction choice through `FOD_READ_CACHE_EVICTION_POLICY`. Re-running the fio comparisons with six runs per policy/workload changed the picture on this host: sequential reads favored LRU on the repeat series, mixed workloads still favored FIFO, and random mixed was effectively a tie. The earlier one-run FIFO-wide win was not stable.

## 2026-06-26

- `fod-indexer` now has a top-level `--output json` mode for `source list`, `scan`, `hash`, `report duplicates`, `plan-import`, `clean`, `materialize`, and `cleanup-failed`, plus read-only snapshot exports for `plan show --id <id>` and `report duplicates --id <id>`. The JSON mode keeps the same underlying contract as the text mode, but exposes structured source views, stored plan snapshots, and duplicate-set snapshots for `msfind`.
- The retry boundary is now documented explicitly in `docs/fod-indexer.md`: `scan`, `hash`, `plan-import`, and `cleanup-failed` stay inside the bounded idempotent envelope, while full transactional replay remains a separate project. `materialize` still uses best-effort rollback plus `cleanup-failed` as fallback.
- `cargo check -p fod-rust-indexer` passed after the output/snapshot refactor.
- `make test-fod-indexer-json-output` passed against the live PostgreSQL-backed integration stack. The new smoke covers JSON source inspection, JSON scan/hash summaries, duplicate-set export by id, and stored import-plan export by id, and it confirmed that the source tree stayed unchanged.

## 2026-07-01

- There is no local project rule in `AGENTS.md` or the checked Markdown files that requires every change to be minimal. Mentions of `minimal` or `small` in other documents are descriptive and context-specific, not a general policy. For future work, scope should be chosen to close the actual task cleanly, without artificially minimizing changes, while still avoiding unrelated refactors.
- SQL payload persistence in the 64 MiB `test-large-copy-benchmark` is still dominated by `COPY fod_persist_block_stage` plus the set-based `data_blocks` merge. Batching binary COPY rows into 1 MiB client-side sends is safe because it does not alter the staging table, transaction scope, replay-safe merge, or durability settings, but the local before/after numbers did not prove a stable large throughput improvement.
- The 2026-07-01 local measurements around commit `024547a` showed before `elapsed_s=3.523786` / `throughput_mib_s=18.16`, after repeats around `3.818472`, `3.557921`, `3.542274`, and a sudo `perf stat` after run at `3.494957` / `18.31 MiB/s`. Treat the change as transport hardening and keep the next SQL optimization focused on server-side `COPY` and `INSERT INTO data_blocks ... ON CONFLICT` cost.
- `FOD_PROFILE_IO=1` during Rust FUSE cargo benchmarks can hide useful aggregate log lines because the successful test removes the temporary mount log. A future profiling harness improvement should preserve or print those logs when profiling is explicitly enabled.
- The `FOD_DATA_BLOCKS_MERGE_DO_NOTHING=1` probe is rejected. It did not preserve `copy_block_crc_table`: the second copy produced CRC rows `[1796809269, 3908932036, 1508472627, 1835173265]` instead of `[377268347, 3908932036, 1508472627, 1835173265]`, so the current `ON CONFLICT DO UPDATE` merge remains required for correctness.
- The first safe `data_blocks` merge EXPLAIN reproducer now runs entirely on temporary tables and leaves real `fod.data_blocks` unchanged. On the 2026-07-01 local run, the temp reproducer measured about `235.6 ms` for fresh 16k-row insert, `307.7 ms` for identical conflict update, and `359.1 ms` for changed conflict update, while the live large-copy path still spent `1218.576 ms` in server-side COPY and `935.805 ms` across the two real `data_blocks` merges. The next optimization should inspect real table/index bloat, WAL/write amplification, and batch sensitivity before changing runtime SQL.
- The first real `data_blocks` size/churn snapshot after the current merge run showed `data_blocks` at `143 MB` heap / `206 MB` total, `n_dead_tup=84`, and `n_mod_since_analyze=32768`. That does not point to immediate large dead-tuple bloat on this local database, but `idx_data_blocks_data_object_id` has high tuple read/fetch counters and should be understood before index-level changes.
- The first clean real WAL delta for the current 64 MiB large-copy workload showed `wal_bytes=12857415`, `wal_records=165625`, `wal_buffers_full=179`, `wal_write=199`, and `wal_sync=20`, with no `buffers_checkpoint` or `buffers_backend` delta. That makes WAL generation/write/sync visible as a production-path metric, while checkpoint pressure was not triggered in this short local run.
- The `FOD_PERSIST_COPY_SEND_BUFFER_BYTES` sensitivity run on the 64 MiB large-copy workload changed client-side `PQputCopyData` fragmentation from 1024 sends at `64 KiB`, through 65 sends at the `1 MiB` default, down to 17 sends at `4 MiB`, but local elapsed time stayed in a narrow `3.75-3.91s` band and WAL stayed around `12.87-13.01 MB`. Treat the knob as diagnostic only for now; the dominant cost remains server-side `COPY fod_persist_block_stage` plus the two `data_blocks` merges.
- The matching QNAP run on commit `85b3ee0` showed the same SQL/WAL shape but a much slower remote path: the 64 MiB large-copy workload landed between `2.60` and `3.14 MiB/s`, with WAL still around `12.85-13.35 MB`. `4 MiB` was fastest in the single QNAP pass, but the warm default was slower than the cold default, so this remains a variance signal rather than a reason to change the default `1 MiB` COPY send buffer.
- The real-path local matrix on commit `ac47828` compared `FOD_PERSIST_COPY_SEND_BUFFER_BYTES=262144`, `1048576`, `4194304`, and `16777216`. The `1 MiB` default was fastest in that single run (`3.616664s`, `17.70 MiB/s`), but all variants stayed in a narrow `16.73-17.70 MiB/s` band and WAL remained stable at about `12.85-12.88 MB`. The practical next candidate is server-side `COPY fod_persist_block_stage` plus `data_blocks` conflict merge analysis, not changing the default client COPY send buffer.

## 2026-07-03

- The new `profile-pg-table-dml-snapshot` / `profile-pg-table-dml-delta` path captures real `pg_stat_user_tables`, `pg_stat_user_indexes`, and relation-size deltas for `data_blocks`. The first local 64 MiB large-copy run on commit `c5d7f24` inserted `32768` rows, performed `0` `data_blocks` updates, produced `0` HOT updates, and grew `n_dead_tup` by `0`; therefore this workload measures insert-heavy COPY plus conflict lookup cost, not heap rewrite behavior under conflict updates.
- The overwrite-only conflict profile on commit `1969674` closes that gap: after seeding the file before the snapshot, the 64 MiB overwrite produced `16384` `data_blocks` updates, `0` HOT updates, `16384` non-HOT updates, and `16384` new dead tuples. The current conflict update path is therefore a full heap rewrite path on this local database, and future SQL work should target avoiding unnecessary updates or replacing full-overwrite row rewrites with a safer data-object-level strategy.
- The unchanged-block conflict filter on commit `76867aa` removes same-payload row rewrites from `data_blocks`: the 64 MiB same-payload overwrite produced `0` inserts, `0` updates, `0` dead tuples, and only `1266` WAL bytes in the profiled window. The temp-table merge reproducer also showed `Rows Removed by Conflict Filter: 16384`, so the SQL guard itself works when a same-payload conflict reaches the merge. The remaining write-amplification problem is the changed-payload/full-overwrite case, not unchanged-block rewrites.
- The VS Code `rust-analyzer` panic at `span/src/lib.rs:403` was tied to the cache priming phase: the log showed `PrimeCaches(... crates_total: 114, crates_done: 114)` immediately before the crash, while `checkOnSave` and `sysrootSrc` in the local workspace settings were valid. The local workaround is to set `"rust-analyzer.cachePriming.enable": false` in ignored `.vscode/settings.json`; keep `checkOnSave`, proc macros, and build scripts enabled so normal diagnostics still run.
- Full-overwrite data-object swap on commit `0eb2d0e` removes changed-payload `data_blocks` non-HOT updates from the profiled 64 MiB overwrite path: the local run produced `16384` inserts, `16384` deletes, `0` updates, and `0` non-HOT updates. A post-run consistency query returned `0` unreferenced data objects, `0` blocks without objects, and `0` files without objects. The remaining write-amplification problem is now insert/delete churn and dead-tuple cleanup, so the next useful benchmark is repeated full-overwrite bloat/WAL behavior and possible delayed cleanup or object-GC policy.
- Block and extent reads now resolve `files.data_object_id` inside the same SQL statement that reads payload rows. That avoids the stale object-id window that a data-object swap could otherwise expose between a separate object lookup and the block/extent read.

## 2026-07-04

- Repeated full-overwrite data-object swap profiling on commit `60658e8` shows that `data_object_swap_cleanup=deferred` is a valid opt-in maintenance tradeoff, not an obvious new default. In the local five-run 64 MiB overwrite smoke, immediate cleanup generated `43069493` WAL bytes, `81920` inserts, `81920` deletes, and `49152` observed new `data_blocks` dead tuples in the hot window. Deferred cleanup generated `37976180` hot-path WAL bytes, `81920` inserts, `0` deletes, and `0` hot-path dead tuples, but the explicit GC phase then deleted `81920` rows and added `4429524` WAL bytes plus `81920` dead tuples. Keep `immediate` as the default; use `deferred` only when shorter write transactions are worth scheduled object-GC maintenance.
- The storage DML snapshot now covers `data_blocks`, `copy_block_crc`, `files`, `data_objects`, their relevant indexes, and relation sizes, so future block-path benchmarks can distinguish table insert/update/delete churn from index lookup and table-growth effects. The separate `profile-pg-top-io-wal` report confirms that PostgreSQL exposes per-statement `wal_records`, `wal_fpi`, and `wal_bytes`, while `wal_buffers_full` remains a cluster-level `pg_stat_wal` metric rather than a per-query `pg_stat_statements` field.
- The local COPY send-buffer matrix on commit `adeaa35` did not show a reason to change `FOD_PERSIST_COPY_SEND_BUFFER_BYTES`: throughput stayed in a narrow `16.48-16.93 MiB/s` band and each run inserted `32768` `data_blocks` rows with `0` updates and `0` dead tuples. QNAP repeats are still needed, but the current host cannot reach `192.168.1.11` on Docker TLS `2376` or PostgreSQL `5432`.
- The safe fillfactor clone profile shows that lower heap fillfactor can make changed-conflict updates partially HOT-capable on temporary `data_blocks` clones (`0`, `1800`, and `5380` HOT updates for fillfactor `100`, `90`, and `75`), but it also increases temp relation size. Since the current runtime avoids full-overwrite conflict updates through data-object swap, real-table fillfactor should remain a measured future option, not a runtime change.
- The metadata prepared-statement review found that the main high-call read paths already use prepared statements on reused PostgreSQL connections: path resolution, child lookup, attr fetch, block/extent reads, hardlink lookup, symlink target, and special-file metadata are covered. The new `profile-pg-metadata-top` target should be used before any future metadata caching or query-shape change, because metadata lookup was visible but secondary compared with payload persistence in the existing baselines.
- The first `profile-pg-metadata-top` run on commit `48d132a` showed `path_walk=92.234 ms`, `child_lookup=85.318 ms`, `file_attrs=20.072 ms`, `special_file_metadata=13.350 ms`, and `xattr_value=13.174 ms` after the local COPY-buffer matrix. These numbers confirm that metadata lookup is not the current dominant bottleneck for the large-copy workload.
- `profile-indexer-alloc` now provides a repeatable allocation profiling entry point for `fod-indexer` through `heaptrack`, `valgrind massif`, or `/usr/bin/time -v`. The 2026-07-04 `--help` smoke on commit `deabdf6` captured status, output, environment, and `8504 kB` max RSS, but real allocation decisions still need representative `scan` and `hash` runs.
- The first synthetic `fod-indexer` allocation baseline on commit `8d90a6e` processed `250` indexed files through `source add`, `scan`, and `hash --candidates-only`; max RSS stayed between `12820 kB` and `13628 kB`. The code review found bounded hash buffers and staged scan rows, so there is no evidence-based reason to change `rust_indexer` buffer reuse until a larger real-source `heaptrack` or `massif` capture shows pressure.
- `profile-fuse-sequential-io` plus `profile-fuse-sudo-perf-stat` now gives a repeatable FUSE-specific profiling path before touching cache, timeout, or `max_background` knobs. The first local 2026-07-05 run on commit `d55b555` captured `FOD_PROFILE_IO`, strace, and sudo `perf stat` successfully; the short 64 KiB smoke stayed dominated by `wait4`, `futex`, and `restart_syscall`, so no FUSE tuning change is justified yet. The perf-wrapped run also exposed a test-side follow-up: the extent marker warning can be printed while the workload still exits with status `0`.
- The 2026-07-05 fio marker follow-up on commit `7ec2b84` found the real test-harness bug: `fod_test_cleanup()` disabled `errexit` and left it off for subsequent cases. Restoring the previous `errexit` state made the missing extent marker fail correctly; the fio extent checks now assert `enable_extents=true` from startup logs rather than requiring the optional extent execution debug marker for every write pattern. `make test-fio-sequential-io-strace`, `make test-fio-mixed-io`, and `make test-fio-random-mixed-io` passed after the fix.
- The 2026-07-05 extended-suite cleanup on commit `79fa073` wires `test-fod-indexer-parallel-smoke` into `test-all-full` and removes the duplicate sequential `plan-import-scope` / `cleanup-failed` prerequisites from that suite. The extended indexer coverage now checks the source-scoped paths under concurrency instead of only as independent sequential smokes.
- The 2026-07-05 copy-buffer follow-up on commit `ef0e782` adds a single compare entry point for the remaining local/QNAP COPY-buffer baseline work. `profile-data-blocks-copy-buffer-matrix-compare` always runs the local matrix and then either skips, requires, or auto-probes QNAP through `PROFILE_COPY_BUFFER_INCLUDE_QNAP=0/1/auto`, so the benchmark procedure no longer depends on remembering separate local and remote commands. The local-only `64 KiB` smoke passed with `32` `data_blocks` inserts, `0` updates/deletes/dead tuples, and `223978` WAL bytes; keep it as target validation, not a production baseline.
- The full 2026-07-05 COPY-buffer matrix on commit `a3076e1` completed locally and on QNAP. Local throughput stayed effectively flat across buffers (`18.01-18.38 MiB/s`), so there is no local default-change signal. QNAP improved from `2.46 MiB/s` at default to `3.18 MiB/s` at `4194304`, but this is one run and should be repeated before changing `FOD_PERSIST_COPY_SEND_BUFFER_BYTES`. All eight runs were insert-only in `data_blocks` (`32768` inserts, `0` updates/deletes/dead tuples), so the current large-copy path remains COPY plus insert/merge cost rather than update churn.
- The 2026-07-09 local repeatability smoke on commit `bad53cc` did not produce a stable local winner between `default` and `4194304`. The three repeats came out as `14.55/18.17`, `18.43/17.48`, and `17.42/17.76 MiB/s`, so the range still overlaps and the local side alone is not enough to choose a new default. The QNAP repetition remains the deciding signal, but `192.168.1.11:5432` was unreachable in this follow-up attempt.
- Storage Engine v2 is now an explicit gated project rather than a runtime rewrite. It preserves 4 KiB logical blocks and the default block path, first bounds opt-in extent payloads, then requires repeated correctness and performance evidence before sequential segment state, append-only new-object persistence, or a segment manifest can proceed.
- On 2026-07-10, the Storage Engine v2 planning worktree based on commit `3fe5590` introduced bounded extent ranges and `extent_target_bytes=1048576` as a validated startup setting. `cargo check --workspace`, `cargo test -p fod-rust-runtime`, and the full `cargo test -p fod-rust-hotpath` passed against local PostgreSQL. The FUSE suite passed with its four root-only lock tests executed separately through the already-built test binary under `sudo -n`; `make test-runtime-profile-extents` also passed and logged `enable_extents=true extent_target_bytes=1048576`.
- The same 2026-07-10 test pass exposed two harness issues rather than storage regressions. The data-block conflict benchmark depended on parallel test order and now serializes its default mode while self-seeding only when no explicit benchmark ID was supplied. The unprivileged root-permissions smoke left a FUSE mount active after reporting success; the mount was removed manually and the cleanup behavior remains a follow-up in `TODO.md`.
- On 2026-07-10, the bounded-execution worktree based on commit `93f1ab9` persisted a 4 MiB sequential extent-backed file as exactly four 1 MiB `data_extents` rows. `FOD_PROFILE_IO` reported `repo_persist_extents_us=30843`, `prepare_persist_extent_rows_from_extent_ranges_us=1916`, and `prepare_persist_extent_rows_peak_payload_bytes=1048576`; the block control stayed at zero for all three extent counters. Payload preparation now returns `EIO` instead of silently omitting a row when a planned buffered block is missing or when a payload exceeds the configured bound.
- The 2026-07-10 bounded-execution gate based on `93f1ab9` passed workspace check, the full hot-path suite, the ordinary FUSE suite, the four sudo-only multi-mount lock tests, CRC, persist chunking, unlink-after-write, remount durability, copy dedupe, sequential fio, mixed fio, random-mixed fio, and the required `FOD_PROFILE_IO=1` strace run. The root-permissions harness now performs and verifies privileged unmount cleanup, and a repeated full FUSE run left no active test mount.
- The 2026-07-10 three-run local bounded-extent matrix from the worktree based on commit `38af786` passed the Phase A local gate. On the 64 MiB large-file workload, block storage averaged `52.82 MiB/s`; 64 KiB extents averaged `87.67 MiB/s`, 256 KiB `94.66 MiB/s`, 1 MiB `94.51 MiB/s`, and 4 MiB `91.70 MiB/s`. Physical inserts dropped from `16384 data_blocks` rows to `1024/256/64/16 data_extents` rows, while mean maximum RSS stayed around `137.6-137.8 MiB`, proving that row reduction is solved but the block-based `WriteState` memory shape is not.
- The all-workload local matrix smoke on the same 2026-07-10 worktree confirms that 256 KiB and 1 MiB are the strongest balanced candidates. The 16 MiB large-copy sample improved from `22.69 MiB/s` on blocks to `26.64/27.39 MiB/s`; 64 KiB regressed to `17.14 MiB/s`, and 4 MiB reached `25.12 MiB/s` with a larger large-copy RSS sample. Sequential fio reads stayed healthy, but extent-mode fio writes and mixed/random workloads remained slower. Decision: keep 1 MiB as the opt-in PoC default, do not enable extents globally, and use Phase B to identify truly sequential writes and avoid forcing block-overlay workloads through extent persistence.
- The 2026-07-10 three-run QNAP matrix from the same worktree completed the Phase A gate. Block storage averaged `10.56 MiB/s`; 64 KiB extents averaged `19.31 MiB/s`, 256 KiB `24.70 MiB/s`, 1 MiB `21.20 MiB/s`, and 4 MiB `23.13 MiB/s`. Every extent sample exceeded every block sample, physical inserts again fell from `16384` rows to `1024/256/64/16`, and mean WAL fell from about `7.32 MB` to `0.91-1.16 MB` without a meaningful RSS increase. Decision: Phase A is complete, 1 MiB remains the balanced opt-in default, and Phase B can begin; the QNAP-only 256 KiB throughput peak is not enough to create a backend-specific default.
- Final validation on 2026-07-11 from the worktree based on commit `38af786` passed `cargo fmt --all -- --check`, `cargo check --workspace`, shell syntax checks, summary regeneration for all three recorded matrices, and the default `make test-fio-sequential-io` run. The final fio command exercised both block and extent cases successfully, confirming that the new `FIO_CASES` selector preserves the existing default behavior.
- On 2026-07-11, the Phase B1 worktree based on commit `1246b83` introduced `WritePayloadState` with block-overlay and bounded sequential-segment variants. Only opt-in extent mode for a new empty file written from offset zero enters segment mode; gaps, backward writes, existing-file writes, and cross-handle merges downgrade before using the unchanged persistence path. The first integration attempt exposed a stale fixed-name fio file that triggered unchanged-write skipping, so the sequential fio test now uses a per-process file and removes it after verification. The corrected run confirmed the segment-entry marker, extent persistence, and a `65536`-byte maximum payload.
- The same 2026-07-11 Phase B1 worktree passed `cargo check --workspace`, all `fod-rust-hotpath` tests, the ordinary FUSE suite, sequential block and extent fio cases, mixed and random-mixed fio, CRC-table, persist-buffer chunking, unlink-after-write, remount durability, and `FOD_PROFILE_IO=1 make test-fio-sequential-io-strace`. Phase B1 intentionally does not claim a performance improvement because flush still converts segments to 4 KiB block vectors; the next measured change is direct segment persistence.
- On 2026-07-11, the Phase B2 worktree based on commit `f0e0a1c` moved eligible sequential segment payloads directly into native extent rows. Segment ownership is restored to `SequentialSegmentState` when repository persistence fails, while successful writes clear stale recent-write blocks before reads return to PostgreSQL. The default block path and the opt-in extent gate are unchanged.
- The repeated local 64 MiB large-file profile on that worktree averaged `98.81 MiB/s` with 1 MiB direct segments, compared with `94.51 MiB/s` in the earlier bounded-extent baseline. Segment-row preparation averaged `12.33 us` instead of roughly `32 ms`, with one segment-mode entry, no downgrade, 64 extent rows, and about `0.89 MB` WAL per run.
- The repeated 64 MiB large-copy profile is an explicit negative result: extents averaged `14.07 MiB/s` versus `18.50 MiB/s` for blocks even though direct segment preparation took only `18 us`. The remaining cost is repeated extent reads during copy, not segment payload assembly. Keep extents opt-in and avoid broad large-copy classification until range-oriented reads or direct data-object adoption remove that amplification.
- The final 2026-07-11 Phase B2 gate based on `f0e0a1c` passed workspace check, the full hot-path suite, the ordinary FUSE suite, CRC-table, persist chunking, unlink-after-write, remount durability, copy dedupe, sequential fio, mixed fio, random-mixed fio, and `FOD_PROFILE_IO=1 make test-fio-sequential-io-strace`. The profiled strace smoke reported one segment-mode entry, zero downgrades, one 64 KiB segment, `prepare_persist_segment_rows_us=4`, and `repo_persist_extents_us=6346` for the extent case.
- The final sequential test exposed a diagnostics race: checking the log immediately after buffered fio writes could happen before FUSE `release` persisted the state. `sync -f` was not sufficient because it synchronizes the filesystem rather than the specific file handle. `fio --fsync_on_close=1` now forces the FOD `fsync` boundary before mode assertions, and the corrected assertion matches the structured `write_state_mode=sequential_segment` field regardless of its position in the message.
- On 2026-07-11, the Phase C classification worktree based on commit `a7fcd5a` added one shared `PersistWriteClass` contract for `NewObjectSequential`, `ExistingObjectPatch`, and `TruncateOnly`. `PersistExecutionPlan` no longer exposes a redundant `truncate_only` field, direct segments and ordinary execution plans use the same classifier, and the sequential fio smoke confirms the selected classes on the real FUSE paths. Repository transactions remain unchanged until the append-only Phase C step.
- The same 2026-07-11 classification worktree passed workspace check, the full hot-path and ordinary FUSE suites, CRC-table, persist chunking, unlink-after-write, remount durability, copy dedupe, sequential, mixed, random-mixed, and profiled strace fio gates. The extent strace case retained one segment entry, zero downgrades, `prepare_persist_segment_rows_us=4`, and `repo_persist_extents_us=5289`, so adding the semantic class did not alter the Phase B execution shape.
- On 2026-07-11, the Phase C append-only worktree based on commit `42c5edf` routed complete sequential segment payloads through one `transactional_replay_confirmed()` data-object replacement. New extent and optional CRC rows are copied before `files.data_object_id` is swapped; old reference counts and immediate/deferred cleanup are handled in the same transaction, and a durable request token confirms a commit whose acknowledgement was lost.
- The append-only correctness gate passed body and commit disconnect replay, durable confirmation, shared-object detach, hardlink preservation, full overwrite, immediate/deferred cleanup, CRC, block/extent remount durability, unlink-after-write, copy dedupe, sequential/mixed/random fio, and the required profiled strace run. No default-path or extent opt-in setting changed.
- The repeated local 64 MiB append-only large-file profile averaged `94.55 MiB/s` versus `46.39 MiB/s` for blocks, with 64 extent rows, `14.33 us` mean segment preparation, and about `1.03 MB` mean WAL. The corresponding large-copy profile remained slower (`12.14 MiB/s` extents versus `16.07 MiB/s` blocks) because repeated source extent reads dominate. Phase C is complete, but large-copy selection must remain conservative and Phase D must distinguish read-query amplification from a genuine need for a segment manifest.
- On 2026-07-11, inspection of fuser 0.14 dispatch behavior showed that FOD's implemented `copy_file_range` callback was unreachable while the crate enabled only ABI 7.17. Enabling ABI 7.31 made the kernel request reach userspace; exact clean full-file copies into empty destinations now adopt the source data object independently of optional changed-block dedupe.
- Whole-object adoption averaged `1219.23 MiB/s` for block-backed sources and `1282.86 MiB/s` for 1 MiB extent-backed sources across three local 64 MiB runs. The destination created no payload rows. Chunked 4 MiB requests averaged `26.68 MiB/s` on extents versus `17.74 MiB/s` on blocks and used about `42%` less WAL in this sample.
- The first corrected chunked extent run exposed a correctness bug: deleting all destination extent rows before persisting only the current dirty block range lost earlier data. Existing extents are now expanded to block rows by one server-side PostgreSQL statement inside the same transaction before the patch is merged and the extents are removed. The regression passes for staging, binary BYTEA, legacy hex, and a partial final block.
- Partial final extent reads also exposed inconsistent padding: `block_bytes()` returned a 4 KiB zero-padded block while `block_bytes_arc()` returned a short slice. The shared variant now follows the same logical-block contract.
- Phase D is closed by `docs/adr/storage-object-segment-manifest.md`: do not add a manifest now. Reopen only when measured partial-clone reuse, patch conversion, chunk dedupe, compression, snapshots, or GC justify the additional schema and replay complexity.
- Payload reads already use `data_object_id`; `id_file` remains only a representative schema pointer that forces row rewrites during purge and failed-materialization cleanup. `docs/storage-payload-ownership-inventory.md` defines the next migration to object-owned payload rows.
- On 2026-07-11, the payload-ownership worktree based on commit `a23bfbb` completed schema version 17. Fresh initialization and upgrades from versions 1 and 16 now converge on object-owned `data_blocks`, `data_extents`, and `copy_block_crc`; the version-16 fixture retained its block, extent, and CRC payloads. A missing version row is restored only after a strict latest-shape verification instead of replaying incompatible historical migrations against a current schema.
- Runtime and indexer cleanup no longer carry or rewrite representative payload `id_file` values. `files.data_object_id` prevents deletion of an attached object, while deleting an unreferenced `data_objects` row cascades to blocks, extents, and CRC rows. Shared detach, source/destination purge order, immediate/deferred cleanup, hardlinks, failed materialization cleanup, and whole-object adoption all passed against the final CRC primary key; the integration test no longer removes that production constraint.
- The 2026-07-11 local gate based on `a23bfbb` passed workspace check, all hot-path tests, the ordinary FUSE suite, all three schema tests, transactional body/commit disconnect replay, indexer materialization, early/late materialization rollback, failed-materialization cleanup, CRC, persist chunking, unlink, hardlink, copy-file-range, 64 MiB whole-object adoption, remount durability, sequential/mixed/random fio, and the required profiled strace run. No FUSE test mounts or root-owned build artifacts remained.
- The post-gate PostgreSQL diagnostic on the same base commit reported zero orphan files/payload rows, zero reference-count mismatches, and zero hybrid block/extent objects. Two objects with zero references were expected products of the deferred-cleanup tests and validate that the existing object-GC path still has work to collect rather than indicating an ownership mismatch.
- On 2026-07-11, the compatibility inventory based on commit `54668b1` confirmed that FOD builds with `fuser 0.14` and ABI 7.31, dynamically links `libfuse3.so.4` and `libpq.so.5`, and has 31 explicit FUSE callbacks. `init`, `ioctl`, and `bmap` remain weakly verified because negotiated capability details, most ioctl variants, and a mounted bmap consumer are not asserted.
- The same inventory found 116 exported `fod_*` symbols and 20 C-layout structures in `libfod_rust_hotpath.so`, but no repository header, dynamic loader, linker, or Python consumer of that shared library. The active runtime uses the Rust library interface, so the current `cdylib` is not classified as a public ABI. Before any public ABI decision, the byte-output allocation/free contract must stop assuming that a forgotten `Vec<u8>` has capacity equal to its length, and `DbfsPgRepo` must remain opaque.
- The current PostgreSQL boundary manually declares 19 `libpq` functions in the hotpath and a 10-function subset in mkfs. The inventory host uses client 17.10 against the local PostgreSQL 16.14 server; these are observed versions pending explicit runtime reporting, not a declared support range.
- On 2026-07-11, the Rust baseline worktree based on commit `f4cfa87` set `rust-version = 1.85` for all five workspace packages while retaining Edition 2021. This matches the minimum declared by `fuser 0.17.0`; the host and the verified `rust:1.85-bookworm` image both provide Rust and Cargo 1.85.1.
- The same baseline passed `cargo fmt --all -- --check`, `cargo check --workspace`, every non-privileged workspace test, all four separately executed root-only lock-backend tests, and complete release and profiling workspace builds. The root-only tests ran from the already-built test executable through `sudo -n`, and no non-user-owned file appeared under `target/`.
- The SELinux/ACL Docker image built successfully from the pinned Rust 1.85 base, and a read-only checkout mounted into that image passed `cargo check --workspace --locked`. The stored CI definition now selects Rust 1.85 and separates caches by compiler baseline, but `.github/workflows/ci.yml_` remains undiscoverable by GitHub Actions until activation is explicitly decided.
- The current FUSE migration baseline was measured locally on 2026-07-11 against production commit `7d9ed837bec69670501c78262c08723fde5d5f48`, `fuser 0.14`, ABI 7.31, schema v17, and PostgreSQL 16.14. Three-run 64 MiB exact copies issued exactly one `copy_file_range` request and averaged `8050.38 MiB/s` for blocks and `9979.55 MiB/s` for 1 MiB extents; the timer covers only the copy operation.
- Exact-copy destinations created no payload rows. The six retained measurement objects each had two file references, a stored `reference_count` of two, and exactly one physical layout. The final database diagnostic reported zero orphan files/payload rows, unreferenced objects, reference-count mismatches, and hybrid block/extent objects.
- Chunked 4 MiB copies issued 16 `copy_file_range` requests and averaged `19.05 MiB/s` for blocks versus `31.66 MiB/s` for opt-in extents. PostgreSQL evidence retained the expected safe conversion shape: the extent destination was expanded to blocks before subsequent partial patches instead of becoming a hybrid object.
- The dedicated 64 MiB sequential workload favored 1 MiB extents (`113.01 MiB/s` versus `54.66 MiB/s`) and extents consistently reduced payload rows and WAL. The 64 MiB fio runs showed the opposite for several practical paths: sequential read fell from about `116053` to `40209 KiB/s`, mixed read/write from `1480/1491` to `643/648 KiB/s`, and random mixed from `1043/1051` to `537/541 KiB/s`. This confirms that extents must remain opt-in through the `fuser` migration.
- The first mixed-fio repetition was invalid because all repeats reused a fixed filename; its first run issued 24608 writes and later runs only 8224. The harness now gives each case a process-specific filename and removes it afterward. The accepted replacement series issued 24608 writes in every sample, so the invalid series is explicitly excluded from the baseline.
- Both block and extent remount durability passed in all three samples. The repository-level partial-patch regression also preserved the unchanged blocks, changed only the requested block, converted an existing three-block extent to three block rows, and left no extent rows. These correctness shapes, callback counts, and database invariants are now the acceptance contract in `docs/fuse-abi-7-31-current-baseline.md` for the later `fuser 0.17` comparison.
- The final 2026-07-11 validation on the same worktree passed formatting, shell syntax, Python compilation, Markdown table-width checks, deterministic report regeneration, FUSE test compilation, the exact partial extent-patch regression, and a mounted 64 KiB whole-object adoption smoke. The mounted smoke observed one read, one write, and one `copy_file_range` request with a shared destination object. The follow-up PostgreSQL diagnostic remained at zero orphan rows, unreferenced objects, reference-count mismatches, and hybrid objects; no test mount or root-owned build artifact remained.
- On 2026-07-12, the worktree based on commit `0c48865` migrated FOD from `fuser 0.14` / ABI 7.31 to `fuser 0.17.0` with `abi-7-40` and `libfuse3` selected explicitly. The adaptation keeps FOD's internal inode, handle, lock-owner, offset, and libc errno model behind typed fuser boundary conversions and keeps the default single-threaded session behavior; no storage, cache, copy, schema, extent, or direct-I/O policy changed.
- The migration gate passed workspace check and all non-root workspace tests, the two tests excluded by the root-test name filter, all four independently executed sudo-only two-mount lock tests, the mounted suite, ioctl, poll, access groups, copy-file-range, hardlinks, lseek, CRC, persist chunking, unlink-after-write, remount, exact 64 MiB object adoption, chunked 4 MiB copy, partial extent conversion, sequential/mixed/random fio, and the required profiled strace run. The exact 64 MiB copy still used one request, shared the data object, and reached `8594.48 MiB/s` in the validation sample; the chunked copy reached `18.94 MiB/s`.
- The post-gate schema-v17 diagnostic reported zero orphan files or payload rows, zero unreferenced objects, zero reference-count mismatches, and zero hybrid block/extent objects. No FUSE test mount or non-user-owned target artifact remained.
- `fuser 0.17` logs `Failed to umount filesystem: Invalid argument` after a successful external `fusermount3 -u` because session drop attempts a second libfuse3 unmount. Runtime checks prove the mount is already gone and data tests pass. The public blocking `mount2` API does not expose the mount guard needed to consume it after an external unmount, so this is documented as an upstream follow-up rather than hidden or patched locally.
- The 2026-07-12 mounted compatibility probe on base commit `dd67984` observed kernel FUSE protocol 7.44 and effective protocol 7.40, with both FOD-requested lock capabilities available and enabled. FOD now reports all kernel-advertised capabilities and explicitly marks `max_write`, `max_readahead`, `max_background`, and `congestion_threshold` unavailable because fuser 0.17 exposes no public getters for their final values; no private-state inference or local fuser fork was introduced.
- The compatibility diagnostics gate passed all 27 FUSE binary unit tests, all 9 mounted Rust smoke tests, and 10 host-supported cases from the 11-case system mount suite; the SELinux xattr case remained an expected host-dependent skip. No temporary FOD mount or non-user-owned target artifact remained after the gate.
- The 2026-07-12 fuser 0.17 migration matrix on commit `522b1b5` repeated the exact, chunked, sequential, fio sequential/mixed/random, and remount workloads three times for block and 1 MiB extent layouts. Callback counts and physical DML shapes matched the ABI 7.31 baseline exactly. The initially weak exact-block and sequential-block series were repeated; their replacement means were `8012.34 MiB/s` and `58.18 MiB/s`, so neither indicated a persistent regression.
- The migrated fio means were at or above the ABI 7.31 baseline in every measured layout/workload pair. The required strace smoke retained 16 reads and 16 writes, while total traced calls rose by 3.4% for blocks and 2.6% for extents; this small one-run traced-call delta did not reproduce as a 64 MiB throughput regression.
- The final post-matrix schema-v17 gate found zero orphan payload rows, unreferenced objects, reference-count mismatches, and hybrid objects. All 12 measured exact-copy data objects had two real file references and one physical payload layout. The fuser 0.17 migration is therefore retained, while extents and post-7.31 capabilities remain opt-in or disabled pending separate decisions.
- The 2026-07-12 protocol 7.32-7.40 inventory found no capability that should be enabled immediately. fuser 0.17 internally handles `INIT_EXT/flags2` and exposes open/init flags, but does not parse or dispatch `SYNCFS`, `TMPFILE`, or `STATX`. `FOPEN_NOFLUSH` would remove FOD's only close path that can return persistence errors, parallel direct writes conflict with the current single-worker and clone/update write-state design, direct-I/O mmap lacks a coherence contract, and passthrough is incompatible with PostgreSQL-owned payloads.
- The per-crate Cargo lockfile inventory confirmed that all five Rust manifests resolve the repository workspace root, including builds invoked through `--manifest-path`. The removed nested lockfiles described obsolete FOD 3.0.4 dependencies and, in `rust_fuse`, fuser 0.14.0. The repository now retains only the authoritative workspace `Cargo.lock`.
- The 2026-07-13 mounted post-7.31 fallback probe on commit `aa77738` and kernel `6.17.0-40-generic` enabled no new capability. `syncfs()` returned success, `O_TMPFILE` returned `ENOTSUP`, and `statx()` returned inode, size, mode, uid, gid, and link count consistent with `os.stat()`; the namespace and known file contents remained intact. These results preserve the current client-visible behavior but do not claim `FUSE_SYNCFS`, `FUSE_TMPFILE`, or `FUSE_STATX` callback support or a global durability contract.
- The 2026-07-13 hotpath ABI audit on commit `e32853b` found no real dynamic consumer: FUSE and indexer use the Cargo `rlib`, no FOD binary needed `libfod-2.so`, no process mapped it, and no public C header existed. The apparent loader references were libc `CDLL(None)` calls in the mounted FUSE fallback test, while the apparent symbol consumers were Rust tests. `fod-rust-hotpath` is therefore an internal `rlib`; the unused `cdylib` crate type and root-install shared-library step are removed. `ffi.rs` remains internal source/test code and does not define a supported ABI.
- Runtime PostgreSQL diagnostics now use `PQlibVersion()`, `PQserverVersion()`, and `SHOW server_version` in both manual `libpq` boundaries. `mkfs.fod status` exposes the values directly, while FUSE logs them once through the existing pooled repository connection. The major relation is informational and `compatibility=connected` records only that the connection succeeded; this change does not define a PostgreSQL support range or alter connection architecture.

## 2026-07-18

- The space-accounting review found that the POSIX 512-byte unit alone did not make `st_blocks` correct: deriving the count from logical file length still over-reported sparse files. File and hardlink attributes now derive allocation from their referenced block/extent payload rows.
- `statfs` must not convert a PostgreSQL failure into a successful empty-filesystem response. Accounting failures are propagated as `EIO`, symlinks are included in inode usage, and an explicit `pg_visible_path` also limits free blocks through the host's available-block count.
- PostgreSQL `octet_length()` is an application payload-column length, not a physical storage measurement. Runtime accounting avoids reading payload values by using block-row count times block size plus extent `used_bytes`; physical payload-relation diagnostics use `pg_total_relation_size()` and remain distinct from both values.
- `max_fs_size_bytes` is not currently a hard quota. A correct limit must be checked after payload mutation but before commit under PostgreSQL-side serialization shared by every mount; a process-local check would be unsafe.
- Commits `b85717a`, `754921a`, `d83e328`, and `597ed2e` used versions not sourced from `fod_version.txt`. The published `main` history was not rewritten; subsequent commits must return to the authoritative version from `fod_version.txt`.
- Validation on the worktree based on `597ed2e` passed workspace check, 80 hotpath unit tests, 27 FUSE binary unit tests, metadata-cache, the expanded mounted `df` regression, sequential block/extent I/O profiling, and the required strace run. The diagnostic helper reported `359133184` payload-column bytes versus `493273088` payload-relation bytes on the local database, confirming that these metrics must remain distinct.
- Versioning now has a durable repository contract in `docs/versioning.md`: every commit increments the patch component, synchronizes `fod_version.txt` and Cargo workspace metadata, and uses that new version in its subject. The stale `3.2.1` source was aligned to `3.2.6`, one greater than the highest `3.2.5` version already published on `main`.
- Version validation for `3.2.6` passed all seven `fod_config` tests, including equality of `fod-config`, `fod-bootstrap`, and `mkfs.fod` output. Cargo metadata reports `3.2.6` for all five workspace packages.
- The transactional payload quota uses `config.max_fs_size_bytes` as the database-wide source of truth. Block and extent payload transactions serialize through one PostgreSQL transaction-scoped advisory lock, calculate post-write payload usage before commit, and roll back the complete mutation when the limit is exceeded.
- FUSE maps a quota rejection to `ENOSPC` rather than hiding it behind `EIO`. Existing block and opt-in extent paths passed the local `FOD_PROFILE_IO=1` and strace gates on the worktree based on commit `81eec1a`. A reversible one-byte-limit smoke returned `ENOSPC`, retained exactly `359133184` persisted payload bytes before and after the rejected write, and restored the configured 10 GiB limit. A dedicated concurrent two-mount over-limit regression remains recommended before treating cross-process contention behavior as fully covered.
- `make test-locking` must not run Cargo through `sudo`. The target now compiles into the isolated `target/test-locking` directory as the invoking user, extracts the exact `lock_backend_smoke` executable from Cargo's machine-readable output, and grants root privileges only to that finished binary.
- The 2026-07-18 privileged locking validation on the worktree based on commit `0f95348` passed all four two-mount lock tests. After the run, all `1571` entries under `target/test-locking` belonged to user `wojtek`, and the complete workspace `target/` contained zero files owned by another user.
- Whole-object `copy_file_range` adoption changes the destination's data object without passing through the normal buffered flush path. It must therefore invalidate the destination read cache, recent-write cache, and `statfs` cache explicitly; otherwise readers can observe pre-adoption bytes and `df` can retain the old payload total until cache expiry.
- The 2026-07-18 64 KiB whole-object adoption smoke on the worktree based on commit `f8d83c5` passed with `shared_object=true` and `7.13 MiB/s`. This small run validates the corrected adoption path and cache invalidation placement; it is not a production performance baseline.
- Copy admission must reserve capacity in the shared PostgreSQL database before source assembly or destination mutation. Schema version 18 adds expiring payload-capacity reservations protected by the same advisory quota lock; active reservations count against normal writes and other copies, while exact whole-object adoption remains reservation-free because it creates no payload.
- On 2026-07-18, the worktree based on commit `a35d301` passed the two-connection reservation serialization regression and the mounted `copy_file_range` capacity regression. The rejected copy returned `ENOSPC`, preserved the one-byte destination, and left the reservation count unchanged.
- `statfs` must count active PostgreSQL copy reservations as temporarily used capacity while keeping them separate from persisted payload bytes in diagnostics. Otherwise admission control is safe but `df` can still advertise space already claimed by another mount until the reservation is consumed or released.
- On 2026-07-18, the mounted `statfs` reservation probe on the worktree based on commit `1550e9b` inserted a `32768`-byte active reservation and observed `f_bavail` decrease from `2621176` to `2621168`, exactly eight 4096-byte blocks. Removing the reservation restored the normal accounting path.
- `statfs` and payload admission must read the same canonical PostgreSQL `max_fs_size_bytes`. Keeping a startup-only copy in `FodFuse` lets `df` disagree with writes after a live database limit change, so the refreshed snapshot now carries the current database limit with usage and reservations.
- On 2026-07-18, the mounted live-limit probe on the worktree based on commit `e3fb13e` reduced the PostgreSQL limit by one 4096-byte block without remounting. With the `statfs` cache disabled for the probe, `f_blocks` changed immediately from `2621440` to `2621439`, and the original limit was restored afterward.
- A fixed one-hour copy reservation is only crash safety, not a maximum supported copy duration. Persistence must renew the token under the shared quota lock and recheck its reserved bytes against current usage first; this preserves long-running copies without allowing an expired token to take capacity already granted elsewhere.
- The 2026-07-18 validation on the worktree based on commit `0b789ce` passed the mounted `copy_file_range` path, 80 hotpath tests, 27 FUSE tests, and the required block/extent profile plus strace gates after adding transactional reservation renewal.
- Expired copy reservations need two distinct regression cases: renewal must succeed while their capacity is still available, but persistence must fail with `ENOSPC` before changing the destination once another transaction has committed into that capacity. Exercising the real block persistence transaction verifies the quota lock, expiry filtering, renewal, and rollback boundary together.
- The forced two-mount quota regression must prove actual PostgreSQL serialization, not only simultaneous client start. Holding the production quota advisory lock until both independent FUSE daemons appear as advisory-lock waiters makes the race deterministic; after release, exactly one 4 KiB payload committed and the rejected file retained zero payload rows.
- Reservation renewal and reclaim rejection now exercise both block and extent persistence through the same PostgreSQL contract. This closes the earlier test asymmetry without introducing a second production implementation.
- Mounted space accounting is stable across remount when measured according to its declared semantics: `df` uses unique persisted payload plus active reservations, while per-file `du` attributes a shared object's allocation to every independent file that references it. The local regression retained `1163264` unique payload bytes, one shared object, and `65536` attributed bytes per shared file before and after remount.
- The inactive `.github/workflows/ci.yml_` was documentation, not automation. Removing it also requires removing the CI badge and job claims; local Makefile gates remain authoritative until a new workflow is explicitly designed and enabled.
- Storage compatibility has three separate domains: release version, PostgreSQL schema version, and semantic payload format. Schema version 18 remains the only persisted compatibility marker; a separate format marker is justified only when schema shape cannot safely describe payload interpretation and must come with conversion, crash, replay, upgrade, and remount contracts.

## 2026-07-23

- The opt-in four-lane PostgreSQL mount failure was not caused by lane isolation. Mounted `CREATE` completed, while the first payload write failed because the local schema-19 test database had an empty `fod.config` table.
- Treating the resulting empty allocated-byte field as zero would hide malformed database state. An initialized schema now requires a valid positive `config.block_size`, a valid `config.max_fs_size_bytes` value, and a schema-version row during the startup snapshot; PostgreSQL and parsing errors remain fatal before mount.
- After adding only the two missing local test configuration rows, `pg_lanes_mount` passed create, write, sync, read, rename, stat, remove, and cleanup with read/write/control/lease limits `2/6/1/1`. Automatic endpoint routing remains disabled.
- Validation on the worktree based on commit `2c04860`, targeting FOD `3.2.28`, passed warning-free locked workspace checking, 25 runtime tests, the complete hotpath suite, 31 FUSE binary tests, all three `data_blocks` conflict paths, large-copy and large-file tests, the dedicated-lane mounted smoke, all four sudo-only locking tests, and the seven-test version gate. The broad FUSE package command stopped only at the four tests that intentionally require the dedicated sudo workflow; `make test-locking` then passed all four.
- Rust package tests that rebuild the shared `fod` schema must run sequentially. Running `rust_mkfs`, hotpath, and FUSE package tests concurrently caused expected schema replacement to interfere with active database tests; after `make init`, the same hotpath and FUSE paths passed sequentially.
- Reversible malformed-schema probes on the same worktree removed each required config row separately, observed an explicit pre-mount error for the missing key, and restored `block_size=4096` plus `max_fs_size_bytes=10737418240`.
- The required 64 KiB FUSE profile smoke passed for blocks and opt-in extents. The ordinary profile reported block write/read `2667/8000 KiB/s` and extent write/read `2462/7111 KiB/s`; the strace-wrapped profile reported block `5333/1600 KiB/s` and extent `6400/1524 KiB/s`. These one-run micro-smokes validate execution shape only and are not performance baselines.
- Final cleanup removed the one stale test mount left by the invalid parallel run. No FOD mount, FOD daemon, or non-user-owned target artifact remained.
- Phase 4 observability must distinguish connection-pool pressure from future logical task pressure. The first stage-2 implementation therefore records current and peak pool waiters, active/live/idle connections, checkout and connection-creation latency, generic repository-operation latency/errors, and replay count without claiming that these are operation-classified queues or transaction timings. The opt-in lane path logs these cumulative values with process RSS at startup failure, after successful startup, and after mount completion; routing and pool limits remain unchanged.
- The 2026-07-23 mounted lane diagnostic on the worktree based on `4499dd7`, targeting FOD `3.2.29`, observed six successful control-repository operations on one cached connection after startup. The completed mounted smoke observed 137 write-repository operations, one closure-level error, zero replays, no pool waiters, and no use of the read or lease repositories, which matches the still-disabled routing contract. Process RSS changed from `15921152` bytes after startup to `18259968` bytes after mount completion. These values validate field collection for one short smoke and are not a throughput or memory baseline.
- Validation on the same worktree passed warning-free locked workspace checking, all 80 hotpath unit tests and its complete integration suite, two mounted four-lane smoke runs, the block/extent sequential FUSE profile, the required block/extent strace profile, formatting, diff checks, and all seven version tests. No automatic endpoint routing, pool tuning, or logical task queue was introduced.
- PostgreSQL correctness checks must use a small, explicit `pg_settings` contract rather than promoting all of `SHOW ALL` to fixed requirements. FOD now owns its per-connection UTC, `read committed`, timeout, and string-literal state, while instance-level `max_connections`, `fsync`, and `full_page_writes` discrepancies produce actionable startup messages containing the setting context and pending-restart state. Performance knobs remain benchmark-driven and are not silently rewritten.
- The 2026-07-23 runtime-requirements validation on the worktree based on commit `8481705`, targeting FOD `3.2.31`, passed 25 runtime tests, the complete hotpath suite including all 15 transactional replay cases, the complete mkfs suite, 33 indexer tests, the mounted four-lane smoke, the existing PostgreSQL requirements smoke, warning-free workspace checking, formatting, diff checks, and all seven version tests.
- A controlled local indexer startup with `FOD_POOL_MAX_CONNECTIONS=100` against PostgreSQL `max_connections=100` reported `required=>=102`, `context=postmaster`, `pending_restart=false`, and an explicit restart action. Normal `mkfs.fod status` remained ready against PostgreSQL `16.14`.
- The same worktree passed the required 64 KiB profiled sequential smoke and strace smoke for both block and extent layouts. The ordinary run reported block write/read `1561/5333 KiB/s` and extent write/read `2909/4571 KiB/s`; the strace run reported block `7111/2133 KiB/s` and extent `7111/1829 KiB/s`. These short runs validate the connection/session setup path and are not performance baselines.
- Session setup must not create extra PostgreSQL round trips or collide with replay-test query markers. The required settings and `search_path` are sent in one initialization query per physical connection, and the isolation GUC uses the lowercase value `read committed`, so a proxy marker for the actual uppercase `COMMIT` statement remains unambiguous.
- Phase 4 Stage 2 payload observability must measure one logical persistence attempt across pool wait, bounded replay, and completion, while remaining distinct from process RSS or an enforced memory permit. Per-repository trackers provide lane values and one shared tracker provides process-wide values; scope guards release current bytes on success, error, or unwind, and accounting overflow/underflow is exposed explicitly. Streaming imports attribute the logical file size while retaining their existing bounded application buffers.
- The 2026-07-23 final mounted probe on the worktree based on `02089d3`, targeting FOD `3.2.32`, wrote and removed one 4096-byte file through dedicated lanes. Post-mount diagnostics reported global/write-lane `payload_peak_in_flight_bytes=4096`, `persist_input_bytes_total=4096`, one input row, two persistence calls, zero persistence failures, zero current in-flight bytes, and zero accounting errors; read, control, and lease payload counters remained zero.
- The same probe exposed fuser's `[Not Implemented] fsync` warning even though the normal write/flush path persisted the payload. Explicit per-file `fsync` semantics, PostgreSQL-error propagation, and remount durability are therefore a separate open FUSE task rather than an implied guarantee of the lane test.
- Validation for the FOD `3.2.32` worktree passed warning-free locked workspace checking, 25 runtime tests, the complete hotpath suite including all 15 replay cases, 31 FUSE binary tests, the mounted dedicated-lane smoke, all three 64 MiB data-block conflict scenarios, the seven-test version gate, and required block/extent sequential profile plus strace profile.
- Phase 4 Stage 2 observability is complete for the current non-queued architecture. Exact transaction attempts are timed inside the `BEGIN`/`COMMIT` or rollback boundary, heartbeat cycles report scheduling delay separately from execution latency, and the opt-in lane path samples current/peak process RSS plus portable PostgreSQL activity, temporary-file, deadlock, memory-setting, and current diagnostics-backend memory indicators. Queue classification, physical batch size, and completed-file throughput remain Stage 3 work because no logical task boundary exists yet.
- The 2026-07-23 mounted observability probe on the worktree based on `8dc5e88`, targeting FOD `3.2.33`, used a 100 ms PostgreSQL sample interval and a 0.2 s lock heartbeat. A periodic write-lane snapshot observed `7` transaction attempts, `4` heartbeat cycles, zero transaction and heartbeat failures, maximum heartbeat scheduling delay of `166 us`, and maximum heartbeat execution time of `9292 us`. PostgreSQL 16 reported `8294056` bytes for the diagnostics backend, while the process RSS peak reached `18563072` bytes. These values validate collection only and are not a performance or sizing baseline.
- Validation for the same FOD `3.2.33` worktree passed warning-free workspace checking, the complete hotpath suite including all `15` transactional replay cases, `31` FUSE binary tests, two dedicated-lane mounted smokes, all four independent-mount PostgreSQL lock/lease tests, and all seven version checks. The required 64 KiB profile smoke reported block write/read `3048/8000 KiB/s` and extent write/read `2909/7111 KiB/s`; the strace smoke reported block `5818/1829 KiB/s` and extent `6400/1684 KiB/s`. These one-run values validate execution shape only and are not performance baselines; the known post-unmount `EINVAL` cleanup warning remained non-fatal.

## 2026-07-28

- Planning documentation was stale relative to the implemented FOD `3.2.33` tree and schema version `19`. The refresh moved roadmap and compatibility text to the current schema contract, separated historical schema-17/18 migration facts from current schema-19 state, and kept release versioning tied to `fod_version.txt` instead of making older documentation numbers authoritative.
- `TODO.md` now treats the one-capability-per-commit requirement as a durable rule rather than an open implementation task. The remaining actionable Compatibility/FUSE work is the actual follow-up work such as explicit `fsync`, upstream double-unmount handling, and aggregated diagnostics after the underlying boundaries are trustworthy.
- Per-file FUSE `fsync` is now explicit in FOD `3.2.35`. It reuses the same write-state persistence path as `flush`, treats `datasync` as the same PostgreSQL transaction boundary because data and metadata are committed together, and returns PostgreSQL persistence errors through the FUSE reply. `release` no longer ignores final flush errors, but close-time visibility still depends on kernel/FUSE behavior, so caller-visible durability should use `fsync`/`sync_all`.
- `release` now cleans lock-owner state even when its final flush fails. This preserves the close/release lock cleanup contract while still reporting the persistence error through the FUSE reply.
- `make test-db-restore-local` now provides the deterministic local database restore path needed after `cargo test -p fod-rust-mkfs`. It is intentionally narrower than `make reset`: it refuses QNAP/remote endpoints, refuses non-default local DB settings, refuses active FOD mounts or daemons, and then recreates only the local Docker Compose test database before running `mkfs init`.
- Dependency discovery on 2026-07-28 found newer crates including `fuser 0.18.0`, `clap 4.6.4`, `serde 1.0.229`, and `serde_json 1.0.150`, while `cargo update --workspace --dry-run --verbose` changed no packages under the current constraints. The fuser upgrade is a separate compatibility-maintenance task because FOD currently depends on `0.17.0` with explicit `abi-7-40` and `libfuse3`.
- FOD `3.2.37` upgrades the current Rust dependency pins to `fuser 0.18.0`, `clap 4.6.4`, `serde 1.0.229`, and `serde_json 1.0.150`. `fuser 0.18.0` removes the earlier public `mount2` helper and the `abi-7-*` feature switches; FOD now calls `fuser::mount` and relies on runtime protocol negotiation while keeping explicit `libfuse3`.
- The requested `postgres 0.19.14` upgrade is not applicable to the current workspace because FOD does not depend on the Rust `postgres` crate. PostgreSQL access remains implemented through the existing libpq boundary, so adding the crate would only introduce unused dependency surface.
- Validation on 2026-07-28 for the dependency bump worktree based on `1e652a2` passed warning-free locked workspace checking, format checking, version tests, 31 FUSE binary tests, 33 indexer tests, the mounted smoke test with diagnostic `fuser=0.18.0`, the dedicated PostgreSQL-lane mounted smoke, and the remount durability benchmark.
- FOD `3.2.38` introduces the first `fod-rust-monitor` crate. It now owns the lane observability sampler, process RSS sampling, PostgreSQL pressure logging, per-lane pool/payload log formatting, and global payload log formatting. `pg_lanes.rs` now keeps lane construction and mount orchestration instead of carrying monitoring implementation details. The remaining Stage 3 monitoring work is still tied to the future logical task queue and throughput counters.
- FOD `3.2.39` moves the stable observability data models and payload persist tracker into `fod-rust-monitor`. `rust_hotpath` now depends on the monitor crate for shared metric types and implements `LaneObservabilitySource` for `DbRepo`; `fod-rust-monitor` no longer depends on the concrete hotpath crate. This makes the monitoring boundary usable for later Stage 3 queue and throughput metrics without creating a dependency cycle.
- FOD `3.2.40` adds the first Stage 3 queue and throughput contracts to `fod-rust-monitor`. The new types describe logical task lanes, operations, queue state, active-transaction and payload headroom, database batch sizing, and completed-file/byte throughput without changing FUSE scheduling, routing, concurrency limits, or backpressure behavior.
- FOD `3.2.41` adds `LogicalTaskQueueObservability` as an in-memory Stage 3 aggregate in `fod-rust-monitor`. It records queue/task lifecycle counters, active-transaction and payload-in-flight state, database batch totals, throughput totals, backpressure/fairness counters, and accounting errors without changing FUSE scheduling or routing.
- FOD `3.2.42` turns `fod-rust-monitor` into an installable binary as well as a library. The initial `fod-monitor status` command reports local FOD processes and RSS memory from `/proc`, while `make install-root-scripts` and `make install-on-root` now build, install, and strip `/usr/local/bin/fod-monitor`.
- FOD `3.2.43` makes `fod-monitor` useful as an operator-facing runtime viewer, not only a presence check. `status` reports system load, uptime, memory, and per-process RSS, virtual memory, threads, file descriptors, context switches, and command line; `top`/`watch` can refresh the same view continuously; `report` emits a one-shot text report that can be redirected into an incident or benchmark artifact.

## 2026-08-03 — FOD 3.2.50 logical task admission

- Base commit: `5cf1051`.
- The existing logical-task counters already separated admitted, queued, active, completed, and failed work, so the smallest safe Stage 3 step was an admission gate rather than a second queue model.
- Read operations use their own process-local gate. File writes and `copy_file_range` intentionally share one write-lane gate so copy pressure cannot bypass the write admission budget.
- A zero limit preserves the previous single-lock observation path. Positive limits use a condition variable and an RAII permit; waiting tasks remain visible in queue counters and capacity is released on normal completion, explicit error, or early return.
- This stage does not claim PostgreSQL transaction limiting, payload-memory enforcement, fairness, or multi-endpoint routing. Those remain independent acceptance steps.

## 2026-08-03 — FOD 3.2.51 unused FUSE constructor cleanup

- Base commit: `a38381a`.
- `FodFuse::new` was compiled only for tests but no test called it; the production mount already used `FodFuse::new_with_task_settings`.
- Removing the unused wrapper is preferable to `#[allow(dead_code)]` because it preserves strict warning checks and leaves one explicit constructor contract.
- The change does not alter task admission, PostgreSQL behavior, FUSE callbacks, configuration parsing, or routing.

## 2026-08-03 — FOD 3.2.52 FIFO logical task admission

- Base commit: `0fe5dd1`.
- The 3.2.50 condition-variable gate limited concurrency but did not define which waiter should acquire the next slot.
- Each positive-limit gate now assigns tickets under the gate mutex and serves them in FIFO order. Reads keep their own queue; writes and `copy_file_range` keep sharing one queue.
- `notify_all` is required for correctness with ticket order: `notify_one` may wake a later ticket, which would go back to sleep while the eligible ticket remains blocked.
- The zero-limit fast path is unchanged. This stage does not add PostgreSQL transaction permits, payload byte budgets, cross-process fairness, or endpoint routing.

## 2026-08-03 — FOD 3.2.53 deterministic FIFO admission validation

- Base commit: `8d33c48`.
- The previous fio runs used `iodepth=1`, so they exercised admission but did not prove ordering among multiple waiters.
- The new unit test queues eight threads behind a held permit, waits until every ticket is assigned, then verifies exact FIFO acquisition and balanced observability counters.
- The ignored 500-waiter benchmark measures the complete local handoff path and reports elapsed microseconds plus nanoseconds per waiter. It intentionally has no timing assertion, avoiding hardware-dependent CI failures.
- The benchmark includes scheduler, channel, accounting, and wake-all costs. It must not be interpreted as isolated `Condvar::notify_all` latency.
- Production admission behavior is unchanged; this version strengthens validation only.

## 2026-08-04 — FOD 3.2.54 FIFO admission profiling baseline

- Base commit: `bbae407`.
- Five successful 500-waiter runs:
  - elapsed microseconds: `96528`, `65831`, `79442`, `86502`, `96265`;
  - nanoseconds per waiter: `193057`, `131663`, `158885`, `173005`, `192531`.
- Aggregate elapsed result:
  - minimum `65831 us`;
  - maximum `96528 us`;
  - mean `84913.6 us`;
  - median `86502 us`;
  - population coefficient of variation `13.53%`.
- Aggregate per-waiter result:
  - minimum `131663 ns`;
  - maximum `193057 ns`;
  - mean `169828.2 ns`;
  - median `173005 ns`.
- All five runs preserved exact FIFO order and completed without deadlock.
- The measurement includes thread scheduling, mutex reacquisition, channels, observability accounting, permits, and thread shutdown; it is not isolated `Condvar::notify_all` latency.
- The new profiling helper builds the test binary once before measurement and provides baseline, syscall-summary, and resource/counter profiles without altering production admission behavior.

## 2026-08-04 — FOD 3.2.55 targeted FIFO wakeups

- Base commit: `722bc9e`.
- The FOD 3.2.54 profile showed a wake storm: `126763` futex calls, approximately `59000` to `65000` voluntary context switches per run, and `99.62%` of aggregated strace syscall time in futex.
- One private condition variable and ready flag are now allocated per queued ticket.
- The gate reserves capacity for the oldest `VecDeque` entry under the gate mutex, then signals only that waiter after unlocking.
- Reservation before signalling preserves FIFO and prevents later arrivals from stealing a released slot.
- The ready flag prevents a lost wakeup when signalling happens before the target thread starts waiting.
- The zero-limit path, public API, queue counters, backpressure semantics, and fairness semantics remain unchanged.
- Apply-time profiling results follow below.

## 2026-08-04 — FOD 3.2.55 apply-time targeted-wake profile

Artifact directory: `/tmp/fod-fifo-admission-profile/fod-3.2.55-670987-1785844997`

- Five-run elapsed values: `115439,63625,73995,68803,65213` us.
- Elapsed minimum: `63625` us.
- Elapsed maximum: `115439` us.
- Elapsed mean: `77415.0` us.
- Elapsed median: `68803.0` us.
- Elapsed population CV: `24.99%`.
- Per-waiter median: `137607.0` ns.
- Median change versus the FOD 3.2.54 profiling run (`174667 us`): `60.61%`.
- `strace -f -c` futex calls: `2255`; errors: `78`; reduction versus `126763`: `98.22%`.
- `strace -f -c` sched_yield calls: `6254`; errors: `0`.
- Median voluntary context switches: `1020.0`.
- Median involuntary context switches: `758.0`.
- Median maximum RSS: `9660.0` kB.
- The profile remains an end-to-end 500-thread handoff measurement, including scheduler and test-harness overhead.

## 2026-08-04 — FOD 3.2.56 resource-profile summary correction

- The FOD 3.2.55 implementation and measurements were valid, but the apply-time parser did not accept the leading whitespace emitted by `/usr/bin/time -v`; three values were therefore written as `n/a`.
- Corrected FOD 3.2.55 medians:
  - voluntary context switches: `1020.0`;
  - involuntary context switches: `758.0`;
  - maximum resident set size: `9660.0 kB`.
- Compared with FOD 3.2.54, voluntary context switches fell from `63615` to `1020` (`98.40%`), involuntary context switches fell from `1015` to `758` (`25.32%`), and maximum RSS fell from `9940 kB` to `9660 kB` (`2.82%`).
- The profiling helper now creates `fifo-admission-resource-summary.txt` directly from `/usr/bin/time -v` output and fails when metrics are missing or run counts disagree.

## 2026-08-04 — FOD 3.2.57 real FUSE FIFO fairness benchmark

- Adds `make test-fifo-fuse-fairness`.
- Default comparison covers write admission limits `0`, `1`, `2`, and `4`.
- Each limit runs three times by default.
- One paced 4 MiB sequential writer overlaps with 24 independent 4 KiB small-file writers.
- Positive admission limits require at least 90% of small writes to finish before the large writer ends.
- Every run verifies the large file size plus every small file size and payload.
- Per-run JSON and aggregate JSON/Markdown summaries are written to `/tmp/fod-fifo-fuse-fairness/...`.
- The test measures client-visible latency through a real FUSE mount. It does not claim exact client launch order equals kernel callback ticket order.
- Production admission behavior is unchanged.

## 2026-08-04 — FOD 3.2.57 initial real-FUSE fairness result

- All 12 runs completed without worker, timeout, accounting, size, or payload-integrity errors.
- Every limit completed 100% of small writes before the paced large writer ended.
- Median results:
  - limit `0`: large `1258.430 ms`, small `299.380 ms`, small p95 `500.949 ms`;
  - limit `1`: large `1273.380 ms`, small `306.571 ms`, small p95 `528.321 ms`;
  - limit `2`: large `1134.528 ms`, small `229.194 ms`, small p95 `398.831 ms`;
  - limit `4`: large `1155.903 ms`, small `229.085 ms`, small p95 `424.440 ms`.
- The result is not sufficient to select a default limit: every shutdown snapshot reported `peak_queued_tasks=1` and `peak_active_tasks=1`, including limits `2` and `4`.
- The debug callback grep also reported zero because the mount used info-level logging; shutdown observability reported `3144` admitted and completed file-write callbacks per limit.
- FOD 3.2.58 must require measured queue saturation instead of accepting overlap alone.

## 2026-08-07 — FOD 3.2.58 saturated real-FUSE fairness benchmark

- Replaces the paced single large writer with eight independent writer processes.
- Each large writer performs four 256 KiB write calls without intentional pacing.
- Twenty-four prepared 4 KiB writers are released into the continuing large stream.
- Every limit/run pair uses a fresh mount; order rotates between repeats.
- The shutdown log is archived after unmount and parsed into each run JSON.
- Unlimited mode must demonstrate at least two concurrent callbacks.
- Positive limits must demonstrate `peak_queued_tasks >= 2` and `peak_active_tasks == configured limit`.
- Completion balance, empty final queue, zero failures, zero accounting errors, tail operations, small-write overlap, sizes, and full payload digests are mandatory.
- The generic integration cleanup now falls back to shutdown observability when debug request lines are unavailable.
- Production admission behavior is unchanged.

## 2026-08-07 — FOD 3.2.58 single-threaded FUSE root cause

- The first saturated run stopped correctly before commit.
- Unlimited admission completed `56` write callbacks without failures, but
  reported `peak_queued_tasks=1` and `peak_active_tasks=1`.
- FOD did not set `fuser::Config::n_threads`; the default single dispatcher
  serialized callbacks before the admission gate.
- FOD 3.2.58 keeps one thread as the compatibility default, adds validated
  explicit configuration, and uses eight event threads in the saturation test.

## 2026-08-07 — FOD 3.2.58 write-boundary criterion correction

- The threaded baseline proves the event-loop change works:
  `peak_active_tasks=8` with admission disabled.
- Limit `1` proves the admission gate is saturated:
  `peak_queued_tasks=8`, `peak_active_tasks=1`, `56` admitted/completed writes,
  zero failed tasks, and zero accounting errors.
- The first threaded run failed only because the harness measured each small
  operation through `fsync()`. Logical-task admission ends when the FUSE write
  callback replies, so post-write `fsync` delay is outside that gate.
- The benchmark now records `write()` latency and total write-plus-fsync latency
  separately. The 90% progress criterion uses only `write()` completion.
- The large competing stream is extended from four to eight 256 KiB writes per
  large worker to keep a stable post-injection workload.
- The AWK observability fallback now uses `field_index` rather than shadowing
  AWK's built-in `index()` function.

## 2026-08-07 — FOD 3.2.58 threaded saturation result

Artifact directory: `/tmp/fod-fifo-fuse-fairness/fod-3.2.58-write-boundary-2923036-1786102688`

| limit | runs | large median ms | small write median ms | small write p95 ms | worst small write ms | minimum write overlap % | minimum tail ops | peak queued | peak active | callbacks median |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 3 | 180.399 | 25.608 | 28.891 | 42.808 | 100.00 | 57 | 1-1 | 8-8 | 88 |
| 1 | 3 | 231.631 | 25.492 | 41.706 | 122.560 | 100.00 | 61 | 8-8 | 1-1 | 88 |
| 2 | 3 | 186.197 | 11.762 | 17.781 | 20.718 | 100.00 | 60 | 6-7 | 2-2 | 88 |
| 4 | 3 | 185.709 | 13.650 | 37.251 | 72.191 | 100.00 | 57 | 5-5 | 4-4 | 88 |

- Positive limits passed only after shutdown observability proved a real backlog and exact active-limit use.
- The progress threshold refers to completion of `write()`, not the later `fsync()` tail.
- All runs drained the queue and passed failure, accounting, size, and full-payload-digest checks.

## 2026-08-07 — FOD 3.2.59 FUSE admission tuning matrix

- Adds `make test-fuse-admission-matrix`.
- Default event-thread axis: `2`, `4`, `8`, `16`.
- Default write-admission axis: `0`, `1`, `2`, `4`, `8`.
- Each of the 20 cells runs three times with a fresh mount and rotated order.
- Workload uses 16 large writer processes, eight 256 KiB writes per large
  writer, and 32 prepared 4 KiB small writers.
- Binding cells must prove queue pressure and exact active-limit use.
- Non-binding cells (`limit >= threads`) are retained because they show when
  FUSE dispatch, not logical admission, is the actual concurrency ceiling.
- Ranking weights are deliberately explicit: throughput 35%, small-write
  median 25%, small-write p95 25%, run-to-run stability 15%.
- The score is diagnostic only. FOD 3.2.59 does not alter production defaults.

## 2026-08-07 — FOD 3.2.59 apply-time matrix result

Artifact directory: `/tmp/fod-fuse-admission-matrix/fod-3.2.59-3025784-1786104660`

Overall diagnostic leader: threads `8`, limit `4`, score `90.530`.

Best binding candidate: threads `8`, limit `4`, score `90.530`.

| rank | threads | limit | score | throughput MiB/s | small median ms | small p95 ms | stability CV % |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 8 | 4 | 90.530 | 127.501 | 16.577 | 22.867 | 18.36 |
| 2 | 4 | 8 | 81.920 | 71.715 | 14.325 | 19.440 | 22.61 |
| 3 | 16 | 4 | 81.390 | 95.174 | 15.902 | 23.153 | 27.63 |
| 4 | 2 | 8 | 71.520 | 92.307 | 20.690 | 29.080 | 23.36 |
| 5 | 2 | 4 | 69.938 | 112.145 | 27.604 | 37.508 | 13.44 |

The ranking remains diagnostic; this commit does not change runtime defaults.

## 2026-08-07 — FOD 3.2.60 admission confirmation suite

- Confirms only the four decision-relevant configurations from FOD 3.2.59:
  `8/4`, `16/4`, `4/8`, and `8/0`.
- Uses ten runs per candidate, forty fresh mounts total.
- Rotates candidate order between repeats.
- Reuses the validated FOD 3.2.58 fairness workload and FOD 3.2.59 matrix
  annotation/ranking logic.
- Executes three sequential fio runs and three strace runs under `8/4`, all with
  `FOD_PROFILE_IO=1`.
- Default confirmation thresholds for `8/4`:
  - rank 1 among the four candidates;
  - at least 10% throughput gain versus `8/0`;
  - at least 10% improvement in small-write median versus `8/0`;
  - at least 10% improvement in small-write p95 versus `8/0`;
  - aggregate stability CV no higher than 25%.
- Performance verdict is recorded and does not by default prevent committing the
  measurement infrastructure. Correctness or incomplete-data errors do fail.
- Production defaults remain unchanged.

## 2026-08-07 — FOD 3.2.60 apply-time confirmation result

Artifact directory: `/tmp/fod-fuse-admission-confirmation/fod-3.2.60-3092631-1786105616`

Verdict: `confirmed`.

- target rank: `1`;
- throughput gain vs `8/0`: `54.02%`;
- small-write median improvement vs `8/0`: `31.44%`;
- small-write p95 improvement vs `8/0`: `29.95%`;
- target stability CV: `18.63%`.

| threads | limit | rank | score | throughput MiB/s | small median ms | small p95 ms | stability CV % |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 8 | 4 | 1 | 96.666 | 121.677 | 16.214 | 22.403 | 18.63 |
| 16 | 4 | 2 | 93.654 | 112.734 | 15.675 | 22.271 | 33.61 |
| 4 | 8 | 3 | 78.57 | 85.821 | 19.120 | 25.354 | 31.25 |
| 8 | 0 | 4 | 67.236 | 78.999 | 23.649 | 31.983 | 42.41 |

Captured fio/strace logs: `6`.

Production defaults remain unchanged in FOD 3.2.60.

## 2026-08-08 — FOD 3.2.61 production validation and INI parity

- Active in both base INI files: `fuse_event_threads=8`,
  `fuse_clone_fd=false`, `task_read_active_limit=0`,
  `task_write_active_limit=4`.
- Explicit environment variables keep precedence over INI.
- Bootstrap now propagates startup-only FUSE controls from INI to the Rust FUSE
  child; previously equivalent INI keys would have been ignored.
- `allow_other` is active and false; FUSE cache timeouts are documented but
  commented to preserve established defaults.
- `scripts/audit_runtime_env_ini.py` generates the production Rust env-vs-INI
  inventory.
- `test-fuse-production-validation` compares `8/4` with `8/0` on real
  PostgreSQL-backed sequential, mixed and random-mixed FUSE workloads, plus
  strace and the mount suite.
- Hard-coded runtime fallbacks are not changed in this stage.

## 2026-08-08 — FOD 3.2.61 mount-suite cleanup race

The first production-validation attempt reached `test-mount-suite` after the
compile, configuration, and environment-audit checks passed.
`test_selinux_runtime_feature_on` correctly took the host-dependent skip path,
but `TemporaryDirectory.cleanup()` received `EBUSY` because the threaded FUSE
mount had not fully detached yet.

The failure was in the integration-test mount helper, not in FOD data
correctness. The helper previously issued one unmount command, ignored its
return code, terminated the FUSE process, and immediately removed the
directory without verifying kernel mount state.

The helper now retries normal detach after process shutdown, uses a bounded
lazy-detach fallback only for test cleanup, verifies that the mountpoint is
actually detached, and raises an explicit error if it remains mounted.
Production FUSE unmount semantics are unchanged.

## 2026-08-08 — FOD 3.2.61 apply-time production validation result

Artifact directory: `/tmp/fod-fuse-production-validation/fod-3.2.61-v8-1289068-1786152459`

Verdict: `production_candidate_supported`.

| workload | 8/4 median s | 8/0 median s | 8/4 delta % |
| --- | ---: | ---: | ---: |
| sequential | 3.735 | 3.739 | -0.11 |
| mixed | 8.306 | 10.108 | -17.83 |
| random-mixed | 11.289 | 10.363 | 8.93 |

Endurance 8/4: `158.6s`, `16` iterations.
Endurance 8/0: `155.0s`, `15` iterations.
PostgreSQL snapshot warnings: `18`.

Correctness/accounting checks passed. Hard-coded runtime fallbacks remain unchanged.

## 2026-08-08 — FOD 3.2.62 PostgreSQL telemetry diagnosis

The 3.2.61 production-validation harness selected `FOD_PG_HOST` and sometimes
`FOD_PG_PORT`, but selected database/user/password from `POSTGRES_*`. The
Makefile exports `FOD_PG_HOST`, `FOD_PG_PORT`, `FOD_PG_DBNAME`, `FOD_PG_USER`
and `FOD_PG_PASSWORD` to child processes; the `POSTGRES_*` Make variables are
used to configure Docker and are not the canonical exported runtime identity.

That mismatch explains why FUSE workloads could succeed while the auxiliary
`psql` snapshots were unavailable. FOD 3.2.62 aligns the telemetry connection
resolver with the exported FOD variables, keeps `POSTGRES_*` as compatibility
fallbacks, and makes required telemetry failures blocking for the production
validation. Production Rust/FUSE/hot-path behavior is unchanged.

## 2026-08-08 — FOD 3.2.62 telemetry smoke result

- status: `ok`
- endpoint: `127.0.0.1:5432`
- database: `foddbname`
- user: `foduser`
- source variables: `{"database": "FOD_PG_DBNAME", "host": "FOD_PG_HOST", "password": "FOD_PG_PASSWORD", "port": "FOD_PG_PORT", "user": "FOD_PG_USER"}`
- transaction delta: `3470`
- artifact directory: `/tmp/fod-postgres-telemetry/fod-3.2.62-2535463-1786177014`
- password value was not stored in the report.

## 2026-08-08 — FOD 3.2.63 transaction admission

Transaction concurrency is now bounded at the actual PostgreSQL
`BEGIN`/`COMMIT`/`ROLLBACK` boundary rather than inferred from FUSE callback
concurrency. The transaction gate is process-local and independent from the
connection-pool limit and FUSE task admission.

Write transactions and control/lease transactions use separate FIFO gates.
RAII release covers normal commit, rollback, replayable disconnects and error
unwinding. The base preset uses write limit `4` and control/lease limit `2`
with connection pool `10`; code fallback remains `0` when startup configuration
is absent.

## 2026-08-08 — FOD 3.2.64 payload budget and logging security

The existing payload observability measured bytes in flight but did not apply
backpressure. FOD 3.2.64 turns the global cross-lane payload tracker into an
actual byte-admission boundary. Transaction count and payload size remain
independent controls: transaction admission bounds active PostgreSQL
transactions, while the payload budget bounds the amount of logical persist
data admitted concurrently.

The base payload budget is `64MiB`. Strict FIFO prevents smaller requests from
bypassing an older large request. A request larger than the configured budget
is permitted only when the budget is empty, which prevents deadlock without
allowing several oversized requests to amplify memory pressure concurrently.

Security/logging conclusion: previous Makefile execution could echo expanded
test passwords such as `POSTGRES_PASSWORD` and `FOD_SCHEMA_ADMIN_PASSWORD` into
captured logs. This did not block FOD 3.2.63 correctness, but logs containing
those command lines must be treated as sensitive. FOD 3.2.64 silences
password-bearing Makefile recipe commands and adds an audit that blocks future
echo regressions. This protects Makefile command echo; child tools must still
avoid printing secrets themselves.

## 2026-08-08 — FOD 3.2.65 role-aware startup routing

Multi-endpoint configuration now changes the actual PostgreSQL endpoint used by
the mount instead of remaining parser/health-only metadata. Candidate endpoints
are role-probed before repository pools are created. Endpoint-routing enablement
itself enters the `pg_lanes` startup path, so routing does not depend on the
separate dedicated-pool-lanes opt-in.

The safety policy remains deliberately conservative. A writable mount pins all
current repository pools to one verified writable primary, preserving
read-after-write consistency. `auto` can fall back to a healthy read-only
endpoint, but the startup snapshot then makes the mount read-only.

This stage provides startup failover only. Transparent endpoint replacement
after a mounted path fails must preserve transaction replay confirmation,
lock/lease ownership and read consistency, so it remains the next stage.

## 2026-08-08 — FOD 3.2.66 runtime primary failover

Startup failover alone is insufficient because `DbRepo` previously stored one
immutable conninfo and replay reopened that same DSN. Runtime routing therefore
has to exist inside the connection-recovery boundary.

FOD 3.2.66 introduces a shared target generation. A replayable connection
failure invalidates the generation and rotates to the next configured primary
entrypoint. Cached connections are generation-tagged: stale entries are closed
on acquisition, and in-flight stale connections are closed instead of being
returned to cache. This prevents an old endpoint from silently re-entering the
pool after failover.

Every newly opened routed connection is revalidated against its required role.
For writable mounts the target must remain a non-recovery, non-read-only
primary. This is not a general multi-primary protocol: configured primary
entrypoints must still refer to the same authoritative PostgreSQL primary/HA
cluster. Replica read routing remains deferred until WAL/replay-LSN consistency
is available.

## 2026-08-08 — FOD 3.2.67 WAL-gated replica reads

Replica routing cannot safely be implemented by only renaming the existing
read pool lane. The mounted `DbRepo` now owns a separate replica target set and
read pool.

Each successful primary write-lane operation refreshes a process-local
`pg_current_wal_lsn()` barrier. A stale-sensitive read uses a replica only when
its replay LSN reaches that barrier. Any uncertainty falls back to primary.

This gives process/session-local read-after-write behavior. It does not claim
global linearizability for writes performed by other FOD processes. Multiple
replica scoring and stronger cross-process consistency remain later work.

## 2026-08-09 — FOD 3.2.68 explicit INI per mount

Implicit configuration discovery is inappropriate for a mount helper because
the filesystem-to-database relationship must be deterministic. A host may mount
`fod.db01` and `fod.db02` simultaneously and each must remain bound to its own
PostgreSQL settings.

The mount helper therefore requires an absolute `ini=` path and passes that
selection to `fod-bootstrap --config`. An inherited `FOD_CONFIG` cannot silently
redirect the mount, and cwd-based `./fod_config.ini` fallback is removed from
the mount path. General administrative tools may continue using the existing
configuration resolver independently of this stricter mount contract.

## 2026-08-12 — FOD 3.2.69 adaptive replica scoring

Replica routing now separates eligibility from preference. WAL replay position
is an eligibility requirement; score is only a preference among candidates that
may be tried.

The adaptive score uses replay lag, connection/validation latency, successful
operation latency and recent failures. Hysteresis prevents small differences
from invalidating cached connections, while a short circuit-breaker cooldown
keeps repeatedly failing targets out of the hot path.

Pool pressure is evaluated at the route level because the current architecture
uses one replica connection pool shared across replica targets. Per-target pool
pressure would require per-target pools and is intentionally not fabricated.

A stale-generation failure is still counted globally, but is ignored for
per-target attribution. This prevents an old in-flight connection from
incorrectly replacing `last_failed_authority` or penalizing the current target
after another thread already advanced the generation.

## 2026-08-12 — FOD 3.2.70 primary promotion guard

Runtime failover can no longer treat any writable endpoint as an acceptable
replacement primary. The target must belong to the same PostgreSQL
`system_identifier`, and a guard scan must see only one concrete writable server
identity.

Multiple configured aliases are intentionally allowed when they resolve to the
same server fingerprint. This preserves HA/proxy entrypoint layouts while still
rejecting two distinct writable servers from the same cluster as split brain.

A backend fingerprint change behind an existing entrypoint is treated as a
primary identity transition and forces a guard scan. The existing routing
generation invalidates cached write/control/lease connections after the
transition.

This is a fail-closed detection/fencing gate inside FOD, not external fencing.
A process-local guard cannot revoke a transaction already executing on an old
primary. Real STONITH/consensus fencing and cross-process write epochs remain
required before FOD can claim full split-brain safety.

## 2026-08-12 — FOD 3.2.70 validation environment

`make test-postgresql-primary-promotion-safety` needs the repository-local
`target` directory because the Makefile uses a local build stamp. Moving Cargo
artifacts with `CARGO_TARGET_DIR=/tmp/fod-target` is valid for plain
`cargo check --workspace`, but it is not compatible with this Makefile target.

The full promotion-safety target passed after clearing stale local Cargo
artifacts with `cargo clean`; the earlier failure was caused by a full
`/media/wojtek/virtdata` filesystem during compilation, not by a test failure.

## 2026-08-14 — FOD 3.2.70 fio/strace profiling baseline

Metadata:

- Commit: `dca6688`.
- Run id: `fio-profile-20260814T094852Z`.
- Artifact directory:
  `artifacts/perf/dca6688/lt7300-fio-profile-20260814T094852Z`.
- Tools present: `fio`, `strace`, `perf`; unprivileged `perf stat` was blocked
  by `perf_event_paranoid=4`.

Representative fio results:

| Workload | Path | Size | Read BW | Write BW | Notes |
| --- | --- | ---: | ---: | ---: | --- |
| sequential + strace | block | 64KiB | 271KiB/s | 239KiB/s | smoke-size strace baseline |
| sequential + strace | extent | 64KiB | 132KiB/s | 508KiB/s | one 64KiB segment |
| sequential + strace | block | 4MiB | 275KiB/s | 1221KiB/s | 1024 read/write callbacks |
| sequential + strace | extent | 4MiB | 268KiB/s | 1857KiB/s | direct sequential path, 4 segments |
| mixed sequential rw | block | 8MiB | 216KiB/s | 229KiB/s | cache-hot reads; writes dominate wall time |
| mixed sequential rw | extent | 8MiB | 144KiB/s | 153KiB/s | no direct segment mode for mixed writes |
| random mixed rw | block | 8MiB | 147KiB/s | 156KiB/s | negative-control random workload |
| random mixed rw | extent | 8MiB | 116KiB/s | 123KiB/s | slower than block under random writes |

FOD profile highlights:

- The pure 4MiB sequential extent write used the intended bounded direct
  segment path: `segment_payload_bytes=4194304`, `segment_count=4`, and
  `prepare_persist_extent_rows_peak_payload_bytes=1048576`.
- The 4MiB sequential extent write had lower FUSE write time than block
  (`1.636s` vs `2.734s`) and lower persistence time (`repo_persist_extents_us`
  `211499` vs block `repo_persist_blocks_us` `524592`).
- Sequential reads remained similarly slow for block and extent at this size:
  fio read bandwidth was `275KiB/s` for block and `268KiB/s` for extent.
  Extent read spent `3.111s` in `repo_fetch_block_range_us`, so read-side
  extent lookup/reassembly remains a visible cost.
- Mixed and random-mixed workloads did not enter direct segment mode
  (`segment_count=0`). In those cases the extent path was slower than block,
  which matches the current opt-in contract: extents are a sequential write/read
  PoC, not the default random-write path.

Strace highlights:

- The 4MiB sequential block strace table was dominated by `read` (`51.97%`,
  `18.921s`) and `futex` (`35.94%`, `13.084s`), with `53,987` total syscalls.
- The 4MiB sequential extent strace table had the same shape: `read`
  (`51.86%`, `20.228s`) and `futex` (`35.91%`, `14.006s`), with `58,785`
  total syscalls.
- This points at synchronous 4KiB FUSE request/response overhead and waiting
  time as a major part of the measured wall time. A single throughput number is
  not enough to justify tuning; repeat runs and an iodepth/block-size matrix are
  needed before changing FUSE concurrency or cache policy.

PostgreSQL profiling limits:

- `profile-pg-top-io-wal` could not run because `pg_stat_statements` is not
  installed in the local PostgreSQL test database.
- `profile-pg-wal` captured WAL/checkpointer state successfully, but the
  counters are cumulative since `2026-08-08 01:27:41+00`, not isolated to this
  fio run. The snapshot reported `wal_bytes=105553788`, but it must not be used
  as a per-run WAL cost.

## 2026-08-14 — FOD 3.2.70 perf and pg_stat_statements enablement

`kernel.perf_event_paranoid=0` is sufficient for the current unprivileged
`perf stat -d` targets. The previous value `4` blocked all perf event access.
There is no need to lower it to `-1` for the current tests because
`perf stat -d -- true` and the fio `profile-perf-stat` workload both collected
counters at `0`.

The local Docker PostgreSQL server was already started with
`shared_preload_libraries=pg_stat_statements`, but the extension was missing
inside `foddbname`. After `make enable-pg-stat-statements`, PostgreSQL reports
`pg_stat_statements:1.10`, and both `profile-pg-top-io-wal` and
`profile-pg-metadata-top` return statement data.

The statement-profile Makefile targets now create
`pg_stat_statements` automatically before reading from it. This avoids stale
database-volume failures where the server preloads the library but the database
does not yet contain the extension. If preload itself is missing, the target
should still fail explicitly rather than hiding the server configuration error.

The first perf-backed fio profile after enabling counters used commit
`74e84d2`, run id `perf-pgstat-after-makefile-20260814T100648Z`, workload
`test-fio-sequential-io`, and `FIO_FILE_SIZE=4M`. Perf completed five repeats
with `21.253 +- 0.304 seconds` elapsed, `10,001,464,643` task-clock,
`6,704,631,624` instructions, `7,937,396,605` cycles, and `22,017` context
switches.

This run is a validation baseline for enabled instrumentation, not a tuning
decision. It differs from the strace/FOD-profile run because it does not set
`FOD_PROFILE_IO=1` and does not trace every syscall, so read timings are much
more cache-hot.

## 2026-08-14 — Persistent perf sysctl

The sysctl key must be written as `kernel.perf_event_paranoid`. Writing
`perf_event_paranoid=0` is invalid on this host because it targets the
nonexistent `/proc/sys/perf_event_paranoid` path.

The local host now has `/etc/sysctl.d/99-fod-perf.conf` with:

```text
kernel.perf_event_paranoid = 0
```

Loading that file with `sudo sysctl -p /etc/sysctl.d/99-fod-perf.conf`
sets `/proc/sys/kernel/perf_event_paranoid` to `0`, and `perf stat -d -- true`
collects counters successfully. This is still intentionally narrower than
`-1`; current FOD perf targets do not require the more permissive setting.

## 2026-08-15 — extent PoC retirement decision

The extent PoC is retired as a production storage direction. In the profiled
128 MiB sequential case, FOD split persistence into two 64 MiB operations. The
later block-path operation spent about 13.65 s in the SQL conversion from
`data_extents` to 4 KiB `data_blocks` via `generate_series()`, while block COPY,
set-based merge and COMMIT were substantially cheaper. Maintaining two physical
representations therefore adds conversion cost and branching without a stable
mixed/random workload win.

The performance baseline is now one canonical `data_blocks` representation.
FOD 3.2.71 first disables production extent execution without dropping legacy
data. Migration and physical code/schema removal follow only after integrity
checks. Repeated per-I/O hardlink-count SQL is the next measured optimization
candidate after the block-only baseline is restored.

## 2026-08-15 — FOD 3.2.72 controlled extent migration

Working tree based on commit `ad75fce` implements schema version 20 as the
controlled migration from legacy `data_extents` rows to canonical `data_blocks`.
The migration is administrative: it runs through `mkfs.fod upgrade`, locks the
payload tables, rejects hybrid objects, validates logical file size and
contiguous block coverage, inserts canonical block rows, clears migrated CRC
cache rows, and deletes `data_extents` only after validating the inserted block
data.

The FUSE write hot path no longer materializes extent rows through the previous
`generate_series()` SQL conversion. Partial block writes to an unmigrated
extent-backed object now fail with an explicit upgrade instruction. Full
block-backed replacement can still use a new data object and does not need
runtime extent conversion.

Validation from the FOD 3.2.72 candidate:

```bash
cargo fmt --all
git diff --check
cargo check --workspace
cargo check --workspace --locked
cargo test --locked -p fod-rust-hotpath
cargo test --locked -p fod-rust-mkfs -- --nocapture
make test-version
POSTGRES_DB=foddbname POSTGRES_USER=foduser POSTGRES_PASSWORD=cichosza target/debug/fod-rust-mkfs status | sed -n '1,24p'
```

Results:

- `cargo check --workspace --locked` passed with workspace packages at
  `3.2.72`.
- `cargo test --locked -p fod-rust-hotpath` passed after rerunning
  sequentially.
- `cargo test --locked -p fod-rust-mkfs -- --nocapture` passed, including the
  schema upgrade fixture that now verifies extent rows are migrated to blocks.
- `make test-version` passed.
- After the final `make reset`, `mkfs.fod status` reported FOD version
  `FOD 3.2.72`, schema version `20`, latest migration version `20`,
  `FOD ready: yes`, and no pending migrations.
- `FOD_PROFILE_IO=1 make test-fio-sequential-io-strace FIO_CASES=block
  FIO_FILE_SIZE=4M` passed on 2026-08-15 from the same `ad75fce`-based candidate.
  It reported `effective_extents=0`, 4 MiB block write bandwidth `1859 KiB/s`,
  4 MiB block read bandwidth `329 KiB/s`, `repo_persist_blocks_us=516507`, and
  `repo_persist_extents_us=0`.
- `make test-fio-sequential-io FIO_FILE_SIZE=4M` passed on 2026-08-15 from the
  same candidate. It reported `effective_extents=0`, 4 MiB block write bandwidth
  `665 KiB/s`, and cache-hot 4 MiB block read bandwidth `36.4 MiB/s`.

The first hotpath test attempt failed because it ran concurrently with the
`rust_mkfs` schema upgrade tests against the same local PostgreSQL schema. This
matches the existing rule that PostgreSQL schema-rebuilding suites must run
sequentially; it was not a migration regression.

## 2026-08-16 — FOD 3.2.73 physical extent removal

Working tree based on commit `834737e` removes the remaining runtime extent
paths after the controlled migration:

- schema version 21 drops `data_extents` after verifying migration 20 left no
  rows;
- production schema bootstrap no longer creates `data_extents`;
- runtime config, FUSE startup diagnostics, write buffering, hotpath persist
  planning, and PostgreSQL read/write code no longer expose extent knobs,
  segment state, extent persist APIs, or extent read fallbacks;
- fio, runtime-profile, PostgreSQL DML, space-accounting, and docs now use the
  block-only contract.

Validation from the FOD 3.2.73 candidate:

```bash
cargo check --workspace --locked
cargo test -p fod-rust-runtime
cargo test -p fod-rust-hotpath --test helper_parity
cargo test -p fod-rust-hotpath --test transactional_replay_smoke
make test-schema-upgrade
make test-schema-status
make reset && make test-rust-pg-query
make reset && make test-fio-sequential-io-strace
FOD_PROFILE_IO=1 make test-fio-mixed-io
FOD_PROFILE_IO=1 make test-fio-random-mixed-io
make test-runtime-profile
```

Results:

- `cargo check --workspace --locked` passed with workspace packages at `3.2.73`.
- Runtime, helper parity, transactional replay, schema upgrade/status, and
  PostgreSQL hotpath query tests passed. The first `make test-rust-pg-query`
  attempt failed only because a previous mkfs test left the shared local schema
  in a stale state without `config.block_size`; rerunning after `make reset`
  passed all 14 pg_query tests.
- `make test-fio-sequential-io-strace` passed on 2026-08-16 from the
  `834737e`-based candidate. With `FOD_PROFILE_IO=1`, 64 KiB block sequential
  fio reported write `566 KiB/s`, read `381 KiB/s`,
  `repo_persist_blocks_us=58037`, and strace total `4702` calls in `1.297626 s`.
- `FOD_PROFILE_IO=1 make test-fio-mixed-io` passed. The 4 MiB mixed block fio
  run reported read `453 KiB/s`, write `482 KiB/s`, and
  `repo_persist_blocks_us=282072`.
- `FOD_PROFILE_IO=1 make test-fio-random-mixed-io` passed. The 4 MiB random
  mixed block fio run reported read `301 KiB/s`, write `320 KiB/s`, and
  `repo_persist_blocks_us=292731`.
- `make test-runtime-profile` passed; SELinux label mounting was skipped as
  unsupported on this host, and auto-recovery profile coverage passed with
  schema version `21`.
- Final active-code scans found no `PersistExtentRow`, extent persist APIs,
  sequential segment state, `FOD_ENABLE_EXTENTS`, `FOD_EXTENT_TARGET_BYTES`, or
  `profile-storage-extent` targets outside historical documentation/migrations.
- The active Storage Engine v2 plan and TODO checklist now mark FOD 3.2.72 and
  FOD 3.2.73 as completed and describe the post-removal boundary as block-only.

## 2026-08-16 — FOD 3.2.73 performance comparison after extent removal

Compared current commit `a6a6a19` against immediate predecessor `834737e` on the
same host. Each side ran two local samples with Docker PostgreSQL reset before
the sequential strace smoke; mixed and random-mixed ran after that reset with
`FOD_PROFILE_IO=1`. The 3.2.72 tests still emitted a requested-extent case, but
the comparison below uses the explicit block case only.

| workload | 834737e samples | a6a6a19 samples | result |
| --- | --- | --- | --- |
| sequential 64 KiB strace write/read | `640/489`, `2065/1730 KiB/s` | `667/561`, `577/354 KiB/s` | inconclusive; this micro-smoke was dominated by variance |
| sequential strace total | `0.824293s / 4534 calls`, `0.357320s / 4556 calls` | `0.767739s / 4573 calls`, `0.970467s / 4586 calls` | no syscall-count improvement; total time too noisy |
| mixed 4 MiB read/write | `1223/1302`, `1261/1343 KiB/s` | `1418/1510`, `1418/1510 KiB/s` | improvement: about `+14%` read and `+14%` write |
| mixed `repo_persist_blocks_us` | `147925`, `101379` | `105186`, `101714` | improvement: average dropped from about `124.7 ms` to `103.5 ms` |
| random mixed 4 MiB read/write | `910/968`, `910/969 KiB/s` | `939/1000`, `923/982 KiB/s` | small improvement: about `+2%`, likely near noise floor |
| random mixed `repo_persist_blocks_us` | `110555`, `111089` | `109171`, `104328` | small improvement: average dropped from about `110.8 ms` to `106.7 ms` |

Conclusion: FOD 3.2.73 did bring a measurable improvement in the 4 MiB mixed
sequential rw smoke, probably because the active write/flush path no longer
carries retired extent branch checks and observability fields. The random mixed
workload shows only a small positive signal, and the 64 KiB strace smoke should
be treated as a correctness/syscall-shape gate rather than a throughput
baseline. There is no evidence yet that the removal improves large 64 MiB or
128 MiB sequential throughput; that still needs a dedicated repeated
`test-large-file-multiblock-benchmark` run.

## 2026-08-16 — FOD 3.2.73 block-only 4 MiB and 128 MiB profiles

Working tree based on commit `9616c50` fixed the Makefile follow-up from extent
target removal and re-profiled clean block-only workloads:

- the top-level `.PHONY` list now names `test-fod-indexer-parallel-smoke`
  instead of the accidental `test-fod-indexer-parallel-smoke-local-qnap`;
- the help output lines that inherited an extra tab during extent target
  removal were normalized;
- `make -qp test-fod-indexer-parallel-smoke` confirms the real target is phony,
  `make help` prints the affected entries without the extra indentation, and
  `make test-fod-indexer-parallel-smoke` passed after the cleanup.

Block-only profile results:

| workload | result | dominant PostgreSQL statements |
| --- | --- | --- |
| 4 MiB sequential fio, `FOD_PROFILE_IO=1` | write `3220 KiB/s`, read `54.1 MiB/s`, `repo_persist_blocks_us=111374` | COPY stage `54.785 ms`, child lookup `44.950 ms`, data_blocks merge `37.693 ms`; hardlink count `28` calls / `1.178 ms` |
| 128 MiB large-file multiblock, `4M x 32` | `46.06 MiB/s`, `repo_persist_blocks_us=2320189`, `prepare_persist_rows_from_block_plan_us=19666` | file attrs query with `COUNT(*)` over `data_blocks` `1031` calls / `3570.578 ms`, data_blocks merge `1237.777 ms`, COPY stage `1062.855 ms`; hardlink count `1032` calls / `17.737 ms` |

Conclusion: the repeated `COUNT(*) FROM hardlinks WHERE id_file = $1` query is
visible, but it is not the dominant cost on either clean block-only profile.
The 128 MiB profile instead points at the file-attribute allocation calculation,
which repeatedly counts `data_blocks` for the same data object and costs more
than COPY plus merge combined in this run. The next optimization should target
that attr/allocation path first; hardlink nlink batching should wait until a
metadata-heavy profile shows it dominating again.

## 2026-08-16 — FOD 3.2.73 fio, strace, and perf profiles for 4 MiB and 128 MiB

Measured commit: `54970e3` (`FOD 3.2.73: update block-only performance plan`).
The shell log directory is `/tmp/fod-bench-20260816-ff7fa68/`; that name came
from the first observed commit before a local fast-forward, while the actual
Makefile perf artifacts were written under
`artifacts/perf/54970e3/lt7300-20260816-fio-strace-perf-ff7fa68/`.

| profile | size | fio write | fio read | callbacks read/write | FOD timers |
| --- | --- | --- | --- | --- | --- |
| `FOD_PROFILE_IO=1 make test-fio-sequential-io` | 4 MiB | `3377 KiB/s` | `65.6 MiB/s` | `20 / 1024` | `fuse_read_total_us=873881`, `fuse_write_total_us=227958`, `repo_persist_blocks_us=103784` |
| `FOD_PROFILE_IO=1 make test-fio-sequential-io` | 128 MiB | `3602 KiB/s` | `109 MiB/s` | `516 / 32768` | `fuse_read_total_us=27565089`, `fuse_write_total_us=5377967`, `repo_persist_blocks_us=2411285` |
| `make test-fio-sequential-io-strace` | 4 MiB | `7923 KiB/s` | `1925 KiB/s` | `1024 / 1024` | `fuse_read_total_us=1996121`, `fuse_write_total_us=418261`, `repo_persist_blocks_us=101559` |
| `make test-fio-sequential-io-strace` | 128 MiB | `12.3 MiB/s` | `997 KiB/s` | `32768 / 32768` | `fuse_read_total_us=127249534`, `fuse_write_total_us=7580028`, `repo_persist_blocks_us=2257411` |
| `profile-sudo-perf-stat-system`, `FOD_PROFILE_IO=1` | 4 MiB | `3043 KiB/s` | `143 MiB/s` | `20 / 1024` | `fuse_read_total_us=944501`, `fuse_write_total_us=243419`, `repo_persist_blocks_us=112002` |
| `profile-sudo-perf-stat-system`, `FOD_PROFILE_IO=1` | 128 MiB | `3299 KiB/s` | `93.2 MiB/s` | `516 / 32768` | `fuse_read_total_us=30172549`, `fuse_write_total_us=5833192`, `repo_persist_blocks_us=2616746` |

`strace -f -c` summaries:

| size | total | dominant syscalls |
| --- | --- | --- |
| 4 MiB | `5.539111 s`, `54187` calls, `40` errors | `read=2.874814 s / 2221 calls`, `futex=1.672791 s / 6310 calls`, `wait4=0.441334 s / 3 calls` |
| 128 MiB | `151.946507 s`, `1594743` calls, `186` errors | `read=79.932722 s / 65938 calls`, `futex=56.538371 s / 197530 calls`, `wait4=11.434356 s / 3 calls` |

System-wide `perf stat -a -d -d -d` summaries:

| size | elapsed | cpu-clock | context switches | instructions | cycles | LLC load misses |
| --- | --- | --- | --- | --- | --- | --- |
| 4 MiB | `4.864275126 s` | `38897497426` | `119645` | `26484968615` | `30381588355` | `78887662` |
| 128 MiB | `44.947420015 s` | `359576572717` | `2211091` | `242658835743` | `293409324450` | `326918480` |

All six measured workloads finished with status 0. The `strace` target again
logged `Failed to umount filesystem: Invalid argument`; this matches the known
`fuser 0.17` double-unmount behavior already documented on 2026-07-12 and did
not fail the test.

Conclusion: clean block-only 128 MiB sequential write remains around
`3.2-3.5 MiB/s` in the normal/perf path, with cache-hot reads around
`93-109 MiB/s`. The write-side persistence timer stays near `2.3-2.6 s` for
128 MiB and about `0.10-0.11 s` for 4 MiB. The strace/direct-IO profile is a
diagnostic shape, not a throughput baseline: it changes read callback counts
from `516` to `32768` on 128 MiB and makes `read` plus `futex` dominate the
syscall wall time. The next optimization target remains the block-only file
attribute/allocation path that repeatedly counts `data_blocks`; these runs do
not add evidence that `COUNT(hardlinks)` is the current large-file bottleneck.

## 2026-08-16 — FOD 3.2.74 FileAttr allocation query optimization

Measured working tree: FOD 3.2.74 based on commit `5855293`. Raw tee logs were
kept under `/tmp/fod-file-read-metadata-20260816T071029Z/`; Makefile
`pg_stat_statements` artifacts were written under
`artifacts/perf/5855293/lt7300-file-read-metadata-20260816T071029Z/`.

The repeated allocation query call path was `read()` ->
`entry_attrs_for_ino()` -> `lookup_path()` -> `attrs_for_path()` ->
`DbRepo::fetch_path_attrs_blob()` -> `FetchPathAttrsBlobFile`, whose SQL
calculated `FileAttr.blocks` with `COUNT(data_blocks)`. FOD 3.2.74 keeps that
full attr SQL for real `lookup`/`getattr`, but uses a narrow
`file_read_metadata(file_id)` query in `read()` for file size and atime policy.

| workload | fio/result | callbacks read/write | key SQL result |
| --- | --- | --- | --- |
| 4 MiB sequential fio, `FOD_PROFILE_IO=1` | write `3287 KiB/s`, read `90.9 MiB/s` | `20 / 1024` | old attr `COUNT(data_blocks)` query `7` calls / `1.938 ms`; new metadata query `20` calls / `2.464 ms` |
| 128 MiB sequential fio, `FOD_PROFILE_IO=1` | write `3779 KiB/s`, read `481 MiB/s` | `516 / 32768` | old attr `COUNT(data_blocks)` query `8` calls / `24.136 ms`; new metadata query `516` calls / `14.523 ms` |
| 128 MiB large-file multiblock, `4M x 32` | `46.97 MiB/s` | `1024 / 128` | old attr `COUNT(data_blocks)` query `5` calls / `12.109 ms`; new metadata query `1024` calls / `27.215 ms` |

Compared with the FOD 3.2.73 128 MiB large-file profile, the attr allocation
query dropped from `1031` calls / about `3570.578 ms` to `5` calls / about
`12.109 ms`. The normal 128 MiB fio read improved from the previous recorded
`109 MiB/s` to `481 MiB/s` in this run. The 128 MiB large-file wall throughput
was nearly flat (`46.06` to `46.97 MiB/s`), which means the removed allocation
query was real PostgreSQL work but not the limiting wall-clock path for that
benchmark.

The 128 MiB profile still shows COPY BINARY staging and the `data_blocks`
merge as the next write-side bottleneck: large-file COPY took `1039.630 ms` and
the merge took `1198.296 ms`; sequential fio recorded two COPY calls totaling
`1174.739 ms` and two merge statements totaling about `1185.071 ms`. The
remaining `file_read_metadata` query is deliberately not cached in the file
handle because a handle-local file-size cache would need a proven invalidation
contract for truncate, writes from another mount, shared objects,
copy-on-write, remount, replay, and quota semantics.

Additional profiles passed:

| profile | size | result |
| --- | --- | --- |
| `make test-fio-sequential-io-strace` | 4 MiB | write `6861 KiB/s`, read `8533 KiB/s`, callbacks `1024 / 1024`, strace total `2.582191 s` / `20218` calls |
| `make test-fio-sequential-io-strace` | 128 MiB | write `12.1 MiB/s`, read `11.3 MiB/s`, callbacks `32768 / 32768`, strace total `44.959186 s` / `509472` calls |
| `perf stat -d -d -d` around fio | 4 MiB | elapsed `3.915978945 s`, task-clock `1569984944`, instructions `4677462637`, cycles `3874518131` |
| `perf stat -d -d -d` around fio | 128 MiB | elapsed `42.396279710 s`, task-clock `19163662957`, instructions `65374421213`, cycles `61644399091` |

All measured fio, strace, perf, and large-file workloads exited with status 0.
The strace target again logged `Failed to umount filesystem: Invalid argument`;
as before, the test still completed successfully.

Final code validation passed with `cargo fmt --all`,
`cargo check --workspace --locked`, FUSE atime tests `3/3`, and hotpath library
tests `80/80`. The hotpath test run still emits two existing
`unreachable pattern` warnings in `rust_hotpath/src/persist_plan.rs`.

## 2026-08-16 — FOD 3.2.74 concurrent block persist profiles

Measured commit: `6797299` (`FOD 3.2.74: reduce read attr allocation
queries`). The requested concurrent fio-through-FUSE matrix could not run in
this execution session: the first mount attempt failed with `fusermount3:
mount failed: Operation not permitted`, and the sudo retry failed before mount
with `sudo-rs: sudo must be owned by uid 0 and have the setuid bit set`.
Those are environment blockers for this session, not a successful FOD workload.

As a fallback, a temporary benchmark outside the repo called
`DbRepo::persist_file_blocks_with_crc_flag` directly, split the payload into
4 KiB `data_blocks`, and started 4 or 8 concurrent PostgreSQL persist workers.
The benchmark set `FOD_PG_WRITE_TRANSACTION_LIMIT` to the worker count and
disabled the PostgreSQL payload byte gate with
`FOD_PG_PAYLOAD_IN_FLIGHT_LIMIT_BYTES=0`, so the result isolates the hotpath
database persist contract rather than FUSE admission. Logs are under
`/tmp/fod-direct-concurrent-persist-6797299-20260816T080429Z/`; profiler
artifacts are under
`artifacts/perf/6797299/lt7300-direct-concurrent-persist-6797299-20260816T080429Z/`.

| workload | workers | blocks | elapsed | throughput | worker max |
| --- | --- | --- | --- | --- | --- |
| direct hotpath persist | 4, total 4 MiB | `1024` | `0.151831 s` | `26.345 MiB/s` | `150750 us` |
| direct hotpath persist | 8, total 4 MiB | `1024` | `0.231268 s` | `17.296 MiB/s` | `230099 us` |
| direct hotpath persist | 4, total 128 MiB | `32768` | `2.484365 s` | `51.522 MiB/s` | `2449177 us` |
| direct hotpath persist | 8, total 128 MiB | `32768` | `3.759582 s` | `34.046 MiB/s` | `3715045 us` |

`pg_stat_statements` shows that higher write parallelism does not improve the
current hotpath shape. The dominant statement is the quota advisory lock before
the actual COPY/merge work:

| workload | advisory lock | COPY staging | data_blocks merge shape |
| --- | --- | --- | --- |
| 4 MiB / 4 workers | `4` calls / `117.372 ms` | `4` calls / `62.861 ms` | four merges, about `9.4-10.8 ms` each |
| 4 MiB / 8 workers | `8` calls / `256.648 ms` | `8` calls / `73.312 ms` | eight merges, about `4.9-8.3 ms` each |
| 128 MiB / 4 workers | `4` calls / `3517.839 ms` | `4` calls / `1198.963 ms` | four merges, about `284-307 ms` each |
| 128 MiB / 8 workers | `8` calls / `12683.113 ms` | `8` calls / `1798.962 ms` | eight merges, about `152-349 ms` each |

Conclusion: the current `persist_file_blocks*` contract serializes concurrent
payload persists on `SELECT pg_advisory_xact_lock($1, $2)` for quota accounting,
even when the payload in-flight gate is disabled and the PostgreSQL transaction
limit is raised. For 128 MiB, 8 workers are slower than 4 workers on this host
because total lock wait rises from about `3.5 s` to about `12.7 s`. The next
write-side optimization should first narrow or remove the global quota lock
from the long COPY/merge section while preserving quota correctness across
multiple mounts/processes. Only after that does it make sense to tune worker
counts or the data_blocks merge itself.

Additional profiling for the 128 MiB / 8-worker direct benchmark:

| profile | result |
| --- | --- |
| `perf stat -d -d -d` | elapsed `2.997395838 s`, task-clock `1636084892`, instructions `10810655605`, cycles `5624560314`, context switches `2061` |
| `strace -f -c` | total `1.518181 s` / `37434` calls; dominant calls: `mprotect` `1.130065 s` / `32884`, `futex` `0.374142 s` / `250`, `read` `0.002344 s` / `543`, `sendto` `0.002331 s` / `554` |

## 2026-08-16 — FOD 3.2.74 block-only plan review

Reviewed commit `24a1cb5`, `docs/block-only-performance-plan.md`, and the
subordinate `docs/quota-lock-concurrency-plan.md`. The next executable
storage-performance work is not hardlink counting, COPY/merge tuning, worker
default tuning, block-size changes, or any extent-style payload representation.

The active order is:

1. instrument quota critical-section timing;
2. move ordinary block-persist advisory-lock serialization to the final quota
   validation section;
3. prove two-process/two-mount quota correctness and rollback behavior;
4. re-profile direct concurrent block persistence for 1/2/4/8 workers on 4 MiB
   and 128 MiB;
5. choose the next bottleneck from that new profile.

The current acceptance criterion is to remove artificial global serialization
around long COPY/merge work while preserving database-wide quota correctness.
It is not required that 8 workers beat 4 workers before the post-change profile
proves that is the real optimum.

## 2026-08-16 — FOD 3.2.75 quota critical-section observability

Measured working tree: FOD 3.2.75 based on commit `e11c81a`
(`FOD 3.2.74: record block-only plan review`).

FOD 3.2.75 adds diagnostic observability for the current quota-serialized
persist path without changing quota behavior. PostgreSQL lane and global
payload logs now expose:

- `persist_transaction_*` for total persist transaction time and failures;
- `persist_copy_stage_*` for COPY BINARY staging time;
- `persist_data_blocks_merge_*` for set-based `data_blocks` merge time;
- `quota_lock_wait_*` for advisory-lock wait time;
- `quota_lock_held_*` for time from successful quota-lock acquisition until the
  transaction-scoped lock guard is dropped;
- `quota_final_check_*` for the final persisted+reserved payload count.

Validation passed with `cargo fmt --all`, `cargo check --workspace --locked`,
the focused monitor test `payload_persist_quota_timings_are_reported`, and
hotpath library tests `80/80`. The hotpath test run still emits the existing
two `unreachable pattern` warnings in `rust_hotpath/src/persist_plan.rs`.

The required FUSE fio validation could not run in this execution session.
`FOD_PROFILE_IO=1 FIO_FILE_SIZE=4M make test-fio-sequential-io-strace` built
the Rust binaries and then failed before workload execution because `sudo-rs`
is not installed with uid-0 ownership and setuid. The non-strace retry
`FOD_PROFILE_IO=1 FIO_FILE_SIZE=4M make test-fio-sequential-io` reached FOD
mount startup and exported the new quota/persist observability fields in the
post-startup/post-mount logs, but `fusermount3` failed with `Operation not
permitted`. This is an environment blocker for FUSE end-to-end benchmarking,
not a successful fio measurement.

Next implementation step remains moving the ordinary block-persist advisory
lock from the start of the long COPY/merge transaction body to the final quota
validation gate while preserving cross-process quota correctness.

## 2026-08-16 — FOD 3.2.76 late quota gate for ordinary block persist

Measured working tree: FOD 3.2.76 based on commit `2f95155`
(`FOD 3.2.75: instrument quota persist timings`). Direct profile logs are under
`/tmp/fod-direct-concurrent-persist-2f95155-wt-3.2.76-20260816T103304Z/`.
Profiler artifacts are under
`artifacts/perf/2f95155/lt7300-direct-concurrent-persist-2f95155-wt-3.2.76-20260816T103304Z/`.

FOD 3.2.76 moves ordinary block persistence to a late quota gate:
COPY/merge now runs before `pg_advisory_xact_lock`, then the transaction takes
the advisory lock, rereads `max_fs_size_bytes`, performs the final
persisted+reserved quota count, and commits or rolls back. Reservation-token
paths intentionally remain conservative and still hold the quota lock across
reservation refresh and payload persistence.

The new direct regression
`ordinary_persist_copy_merge_completes_before_final_quota_gate` passed. It
starts two independent `DbRepo` instances, holds the quota advisory lock from a
third connection, verifies both writers reach advisory-lock wait only after
COPY/merge metrics have been recorded, releases the lock, and checks that one
writer commits while the other rolls back with `FOD_ENOSPC` leaving zero
payload rows.

Direct concurrent block-persist profile after the change:

| workload | workers | elapsed | throughput | quota advisory-lock total |
| --- | ---: | ---: | ---: | ---: |
| 4 MiB total | 1 | `0.134806 s` | `29.672 MiB/s` | `0.011 ms` |
| 4 MiB total | 2 | `0.086771 s` | `46.098 MiB/s` | `0.018 ms` |
| 4 MiB total | 4 | `0.088379 s` | `45.260 MiB/s` | `0.038 ms` |
| 4 MiB total | 8 | `0.168180 s` | `23.784 MiB/s` | `0.077 ms` |
| 128 MiB total | 1 | `2.466857 s` | `51.888 MiB/s` | `0.014 ms` |
| 128 MiB total | 2 | `1.490450 s` | `85.880 MiB/s` | `0.018 ms` |
| 128 MiB total | 4 | `1.041978 s` | `122.843 MiB/s` | `0.049 ms` |
| 128 MiB total | 8 | `0.974978 s` | `131.285 MiB/s` | `9.742 ms` |

Compared with the pre-change FOD 3.2.74 direct profile, the 128 MiB / 4-worker
case improved from `51.522 MiB/s` to `122.843 MiB/s`, and aggregate advisory
lock SQL time fell from `3517.839 ms` to `0.049 ms`. The 128 MiB / 8-worker
case improved from `34.046 MiB/s` to `131.285 MiB/s`, and aggregate advisory
lock SQL time fell from `12683.113 ms` to `9.742 ms`.

The new 128 MiB / 8-worker SQL profile is dominated by COPY and merge, not the
quota lock: COPY BINARY staging totaled `3240.165 ms`, the eight
`data_blocks` merge statements totaled about `2662.982 ms`, final quota checks
totaled `16.563 ms`, and advisory-lock SQL totaled `9.742 ms`.

Additional 128 MiB profiling:

| profile | result |
| --- | --- |
| `perf stat -d -d -d`, 1 worker | elapsed `2.437911013 s`, task-clock `1214271388`, instructions `10437846992`, cycles `4366561114`, context switches `94` |
| `perf stat -d -d -d`, 8 workers | elapsed `1.142070629 s`, task-clock `2329900405`, instructions `10777817667`, cycles `7299629263`, context switches `3985` |
| `strace -f -c`, 1 worker | total `0.275117 s` / `35827` calls; dominant calls: `futex` `0.180365 s` / `173`, `mprotect` `0.068900 s` / `32739` |
| `strace -f -c`, 8 workers | total `1.660953 s` / `37427` calls; dominant calls: `mprotect` `1.174470 s` / `32851`, `futex` `0.380897 s` / `243`, `poll` `0.073117 s` / `445` |

Validation passed with `cargo fmt --all`, `cargo check --workspace --locked`,
the focused quota regression, the focused monitor timing test, and hotpath
library tests `81/81`. The hotpath run still emits the existing two
`unreachable pattern` warnings in `rust_hotpath/src/persist_plan.rs`.

FUSE end-to-end validation remains blocked in this execution session:
`FOD_PROFILE_IO=1 FIO_FILE_SIZE=4M make test-fio-sequential-io-strace` failed
before workload execution because `sudo-rs` lacks uid-0 ownership/setuid, and
`make test-two-mount-quota` failed at mount startup with `fusermount3:
Operation not permitted`.

Conclusion: the artificial quota advisory-lock serialization has been removed
from ordinary block persist. Do not tune worker defaults from this single host
yet; the next measured bottleneck is COPY BINARY staging plus the set-based
`data_blocks` merge under concurrent writes.
