# FOD current implementation plan

Status: 2026-09-13.

This file contains only work that is current enough to direct the next change.

For current implemented behavior use [`../CURRENT_STATE.md`](../CURRENT_STATE.md).
For long-term direction use [`../../ROADMAP.md`](../../ROADMAP.md).
`TODO.md` is a mixed follow-up/archive record rather than the implementation
queue.

The completed FOD 3.4.16-3.4.20 read/write optimization sequence is archived in
[`../history/FOD_CURRENT_PLAN_2026-09-08_READ_WRITE_TUNING.md`](../history/FOD_CURRENT_PLAN_2026-09-08_READ_WRITE_TUNING.md).

The completed FOD 3.4.23 cross-mount write ownership, stale-writer fencing and
FUSE hang-guard sequence is archived in
[`../history/FOD_CURRENT_PLAN_2026-09-12_WRITE_OWNERSHIP.md`](../history/FOD_CURRENT_PLAN_2026-09-12_WRITE_OWNERSHIP.md).

The completed P2 QNAP COPY-buffer repeatability follow-up is archived in
[`../history/FOD_CURRENT_PLAN_2026-09-12_QNAP_COPY_BUFFER.md`](../history/FOD_CURRENT_PLAN_2026-09-12_QNAP_COPY_BUFFER.md).
It produced no runtime tuning change: the current
`FOD_PERSIST_COPY_SEND_BUFFER_BYTES` default remains unchanged.

The completed P3 external-unmount/session-teardown validation is archived in
[`../history/FOD_CURRENT_PLAN_2026-09-13_EXTERNAL_UNMOUNT.md`](../history/FOD_CURRENT_PLAN_2026-09-13_EXTERNAL_UNMOUNT.md).
On the current `fuser 0.18.0` / libfuse3 stack the historical benign teardown
`EINVAL` was not reproduced, so no runtime teardown change was made.

The completed P4 compatibility-diagnostics aggregation sequence is archived in
[`../history/FOD_CURRENT_PLAN_2026-09-13_COMPATIBILITY_DIAGNOSTICS.md`](../history/FOD_CURRENT_PLAN_2026-09-13_COMPATIBILITY_DIAGNOSTICS.md).
It completed in FOD 3.4.27 with versioned PostgreSQL/libpq/schema/storage
status, versioned negotiated FUSE telemetry, aggregated `fod-monitor report
--json` sources and an explicit coverage-only compatibility summary.

The post-P4 roadmap review selected one concrete next correctness task rather
than inventing a generic P5 sequence.

## F1 — Mounted `fallocate` contract audit

The current Rust FUSE frontend does not override the public
`fuser::Filesystem::fallocate` callback. Older TODO/archive text containing
legacy "already in place" wording must not be used as evidence of current
mounted Rust behavior.

### F1.1 — Baseline and semantic contract — completed

The mounted diagnostic baseline was captured on FOD 3.4.27 with:

```text
kernel                    6.17.0-41-generic
libfuse3 / fusermount3    3.17.4
fuser                     0.18.0
kernel FUSE protocol      7.44
userspace protocol max    7.40
negotiated protocol       7.40
requested/enabled caps    POSIX_LOCKS, ATOMIC_O_TRUNC, FLOCK_LOCKS
unsupported requested     none
```

The raw `libc.fallocate()` matrix used a fresh mount per mode so kernel caching
of unsupported operations could not hide callback behavior. Results:

- `mode=0` -> `ENOTSUP`, with the default fuser `fallocate` callback logged;
- `KEEP_SIZE` -> `ENOTSUP`, with the default callback logged;
- `PUNCH_HOLE|KEEP_SIZE` -> `ENOTSUP`, with the default callback logged;
- `ZERO_RANGE` -> `ENOTSUP`, with the default callback logged;
- `PUNCH_HOLE` without `KEEP_SIZE` -> `ENOTSUP` before the default callback;
- failed raw calls did not mutate file contents, logical size, `st_blocks`,
  `fod.data_blocks` or persisted payload bytes;
- state was verified again after unmount/remount.

The legacy `os.posix_fallocate()` control is not evidence of FOD preallocation.
It returned success only through fallback behavior: a 64 KiB file grew
logically to 128 KiB while `st_blocks` stayed at 128 512-byte units,
`fod.data_blocks` stayed at two rows and PostgreSQL payload stayed at 64 KiB.
The newly exposed range therefore read as zeroes but no durable payload/capacity
was allocated for it.

The source audit explains that result and fixes the semantic boundary for F1.2:

- `assemble_read_slice()` initializes the requested output with zeroes, so a
  missing canonical block is already a logical sparse zero range;
- PostgreSQL persistence intentionally keeps fully zero blocks sparse;
- `st_blocks` is derived from actually allocated payload bytes, not logical
  file size;
- existing truncate persistence can delete tail blocks and the direct block
  path can delete individual block rows, but there is no current mounted
  fallocate range contract;
- canonical block-only storage has no durable state that distinguishes an
  allocated all-zero block from a sparse/missing block.

Consequences:

- true `mode=0` preallocation cannot be represented faithfully by merely
  extending `file_size` or materializing zeroes;
- `KEEP_SIZE` preallocation has the same representation gap;
- `ZERO_RANGE` would lose Linux allocated-zero/unwritten-range semantics if it
  were approximated as ordinary sparse zeroes;
- `PUNCH_HOLE|KEEP_SIZE` is the only currently selected implementation
  candidate because deallocated full blocks map naturally to missing
  `data_blocks` and partial boundary blocks can retain non-hole bytes while the
  punched bytes become zero.

No database schema, storage format or runtime version changed during F1.1.

### F1.2 — `PUNCH_HOLE|KEEP_SIZE` implementation — active slice

Add an explicit mounted `fallocate` callback only for the exact
`PUNCH_HOLE|KEEP_SIZE` mode. Keep `mode=0`, `KEEP_SIZE`, `ZERO_RANGE` and every
other unsupported flag combination on an explicit `EOPNOTSUPP` path until FOD
has a durable representation for allocation state distinct from payload data.

The implementation must reuse the current write-safety model rather than add a
parallel mutation path:

1. return `EROFS` before mutation on read-only mounts;
2. validate the file handle/file identity and range arithmetic without silent
   saturation of invalid user ranges;
3. preserve file size for every successful punch;
4. serialize with pending writes for the same file and carry the existing
   PostgreSQL-authoritative write ownership/fencing token through persistence;
5. zero only the requested bytes of partial first/last blocks and remove fully
   punched all-zero blocks from canonical storage;
6. preserve hardlink/data-object copy-on-write isolation;
7. keep payload-quota/statfs accounting based on real persisted blocks and
   release capacity when complete blocks disappear;
8. invalidate read/recent-write/metadata/statfs state so cross-handle and
   cross-mount reads cannot expose stale payload;
9. define and test `mtime`/`ctime` behavior as part of the mounted contract
   rather than inheriting accidental timestamp side effects;
10. preserve replay safety, persistence error mapping, stale-writer rejection
    and remount durability.

The first implementation tests must include:

- a full-block aligned punch across one and multiple blocks;
- unaligned start/end boundaries with surrounding bytes preserved;
- a range extending beyond EOF while file size remains unchanged;
- an entirely beyond-EOF no-op range;
- already sparse/missing blocks;
- hardlinks sharing the same data object before mutation;
- another writable mount holding ownership (`EBUSY`/fencing behavior);
- read-only mount (`EROFS`);
- unsupported mode/flag combinations (`EOPNOTSUPP`);
- `st_blocks`, `statfs`, PostgreSQL block rows/payload bytes and remount state.

Do not add allocation metadata merely to make `mode=0` appear supported in this
slice. A future preallocation design, if justified by a concrete workload,
requires a separate storage-format decision and migration/compatibility plan.

### F1 acceptance

- mounted integration coverage exists for every supported mode and for rejected
  combinations;
- `PUNCH_HOLE|KEEP_SIZE` changes only the requested byte range and never
  changes logical file size;
- block-only canonical storage remains the tested production path;
- old Python/extent-era TODO statements are treated as historical evidence, not
  as the runtime contract;
- no unrelated FUSE capability is enabled in the same implementation change;
- benchmark only after correctness tests pass, and keep the feature unchanged
  if measurements do not justify additional optimization.

## Deferred measured follow-ups

These remain candidates, not parallel active implementation projects:

- instrument inode/path cache lifetime and add `forget`/`batch_forget` only if
  large-tree measurements show retained-state pressure;
- benchmark `readdirplus` against `readdir` before enabling or relying on it;
- define and test sparse-file edge cases before implementing mounted
  `lseek(SEEK_DATA/SEEK_HOLE)` semantics;
- repeat QNAP COPY-buffer tuning only after a new measured regression or a
  materially changed QNAP/network/Docker environment;
- reopen external-unmount/session teardown only if a future fuser/libfuse3
  stack reproduces a correctness or warning regression;
- reopen broad read-path or compatibility-aggregation work only for a new
  measured gap.

## Architecture guardrails

The broad role-aware PostgreSQL routing project is complete: startup role
selection, runtime primary failover, WAL-gated replica reads, replica scoring,
promotion validation and process-local generation fencing are implemented.

Do not reopen that design as a generic project.

Further multi-endpoint work requires a concrete measured or correctness gap,
for example external/cross-process fencing with an explicit ownership and
failure model.

Independent writable PostgreSQL primaries must never be presented as one safe
multi-primary FOD filesystem.

The current production storage path is canonical block-only storage. Historical
extent-engine notes in `TODO.md` and `docs/history/` are evidence of how the
current architecture was reached, not alternate production runtime paths.

## Delivery rule

Each implementation change stays on `main`, updates the relevant
documentation/tests and follows the repository versioning policy.

After every commit compare it with its parent using `git diff HEAD~1..HEAD` or
`git show`, inspect the complete file set and run `git diff --check`.

Do not add or modify GitHub Actions workflows.

## Historical plans

Completed execution plans and measurement records belong under
[`../history/`](../history/). They explain how the current state was reached but
must not compete with this file for current priority.
