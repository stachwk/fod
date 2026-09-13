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

The current Rust FUSE frontend does not expose a repository-visible
`fallocate` callback, while older TODO/archive text still contains legacy
"already in place" wording from previous runtime stages. That historical text
must not be used as evidence of current mounted Rust behavior.

### F1.1 — Baseline and semantic contract — active slice

Before changing runtime behavior:

1. record the current kernel FUSE protocol, negotiated protocol, `fuser` and
   libfuse3 versions used by the mounted test;
2. verify the public `fuser 0.18.0` callback/API surface available to FOD;
3. run a mounted syscall matrix for at least normal allocation/extension,
   `KEEP_SIZE`, `PUNCH_HOLE|KEEP_SIZE`, and unsupported flag combinations;
4. probe additional modes such as `ZERO_RANGE` only where the host/kernel and
   public API expose them meaningfully;
5. capture return codes/errno, file size, byte contents, `st_blocks`, `statfs`,
   `mtime`/`ctime`, payload-row/storage effects and state after unmount/remount;
6. keep this slice diagnostic-only: no runtime semantics, database schema or
   storage-format change.

The baseline must distinguish kernel/libfuse fallback behavior from an actual
FOD callback. A syscall returning success or a particular errno is not evidence
that FOD implements the operation unless the request reaches the Rust frontend
and the resulting storage semantics are verified.

### F1.2 — Implementation gate

Implement `fallocate` only after F1.1 defines an explicit FOD contract and only
for modes the current `fuser` surface can represent safely.

Any implementation must preserve:

- read-only mounts returning `EROFS` for mutation;
- unsupported flags/combinations returning `EOPNOTSUPP` rather than being
  silently approximated;
- PostgreSQL-authoritative cross-mount write ownership and fencing;
- stale-writer rejection and `FUSE_ATOMIC_O_TRUNC` safety assumptions;
- transactional payload quota and capacity-reservation accounting;
- canonical block-only storage, sparse-range semantics and `st_blocks`/`statfs`
  accounting;
- hardlink/data-object ownership and copy-on-write behavior;
- read/recent-write/metadata/statfs cache invalidation;
- `mtime`/`ctime` semantics, replay safety, persistence errors and remount
  durability.

No database schema migration is justified merely to add the callback. Add one
only if the selected semantic contract proves that current canonical storage
cannot represent the required state safely.

### F1 acceptance

- mounted integration coverage exists for every supported mode and for rejected
  combinations;
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
