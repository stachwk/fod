# TX03J — PostgreSQL content generation audit (2026-10-23)

Document role: topic review/specification supporting the
[current implementation queue](plans/CURRENT.md). Implemented capabilities are
summarized in [CURRENT_STATE.md](CURRENT_STATE.md); see the
[documentation map](README.md#correctness-reviews-and-evidence) for related reviews.

## Authority and guarantees

PostgreSQL is the authoritative owner of file mapping, content generation,
write leases and commit ordering. An invalidation is scoped to one `file_id`.
A client may never drop local uncommitted write buffers in response to a
remote event, and no SQL fill started before invalidation may publish later.

## Implemented (merged into main)

- Schema v26 adds `files.content_generation`; migration 0027 changes its
  trigger to accept only explicit, single-step increments. Metadata timestamps
  no longer imply content changes, and overflow is rejected by PostgreSQL.
- Persist, streaming persist, copy-on-write replacement, hole-punch, size
  changes, and whole-object adoption advance only the affected file's
  generation in the same PostgreSQL transaction as the content mutation.
- The isolated generation fixture applies migrations 0026 and 0027 and covers
  rollback, concurrent updates, file isolation, metadata-only updates, invalid
  generation steps, and the BIGINT overflow boundary.
- `DbRepo::fetch_block_range_with_generation_shared` retrieves size,
  generation and blocks in one PostgreSQL statement snapshot; legacy reads
  are unchanged.
- `FileCacheGenerationGate` protects publication and now snapshots its
  generation before reading cached blocks. Local file-cache clearing now
  advances that file's epoch and removes its blocks under the same gate lock,
  so a fill started before the clear cannot republish its older snapshot.

## Remaining correctness gates — do NOT enable default multi-host cache yet

1. The generation-bearing read API is used by explicit degraded read-only
   FUSE reads, but is not integrated into the ordinary cached read path. It
   does not itself establish cross-mount cache coherence.
   FOD 3.4.522 fixes a separate source-level gap in the opt-in
   `FOD_CROSS_MOUNT_STRICT_READS` path: its uncached size-and-block snapshot now
   uses the primary/write authority instead of replica-routed `with_read_connection`.
   This prevents replica lag from defeating that diagnostic mode, but does not
   add generation validation or invalidate the kernel page cache.
   The local read-block cache also now advances its per-file epoch atomically
   with cache clearing after local write/persist operations. A source-level
   race review found that clearing the map without advancing the gate allowed
   an already-running SQL fill to publish afterward. The existing gate protocol
   now rejects that fill; runtime race validation is still pending.
2. Cache coherence also requires selective Linux FUSE kernel page-cache
   invalidation, and safe handling of open readers, local dirty buffers,
   and race ordering. Notifications are only hints; missing notifications
   require durable reconciliation. The accepted post-`fsync()` cross-mount
   contract also needs a PostgreSQL-backed ACK barrier: every mount active at
   the barrier must acknowledge targeted kernel invalidation before `fsync()`
   reports success. A timeout cannot be treated as success; the data may
   already be committed even if the syscall reports failure. The protocol is
   specified in [`TX03J_CROSS_MOUNT_CACHE_COHERENCE.md`](TX03J_CROSS_MOUNT_CACHE_COHERENCE.md).
3. When reading from replicas, a generation/content snapshot may lag a
   committed primary change; fencing reads or ensuring a sufficiently
   advanced replica snapshot is necessary.
4. Migration 0027, fresh bootstrap, generation transaction fixtures, the
   generated-block snapshot, mkfs schema-upgrade flow, and hotpath unit tests
   pass on the isolated PostgreSQL test database used on 2026-10-23. The full
   `pg_query` suite and actual two-host FUSE cache invalidation race tests still
   need to run before enabling default multi-host cache coherence.

## Test results on 2026-10-23

The isolated PostgreSQL fixture passed generation transaction tests (2/2),
the generated-block snapshot test, mkfs schema-upgrade and status tests, and
hotpath unit tests (82/82). `cargo fmt --all -- --check`,
`cargo check --workspace --locked`, and `git diff --check` also passed. The
destructive mkfs tests ran only against a dedicated test database, which was
removed after testing; the production database was not modified.

The full `pg_query` integration suite and two-mount FUSE cache invalidation
tests remain outstanding. PostgreSQL integration tests must use an isolated,
initialized database with `fod.config.block_size` and
`fod.config.max_fs_size_bytes` set.

## Next architectural gate

Next, wire a per-file event/reconcile ledger and selective FUSE kernel
invalidation. Then run two-mount tests for event loss, open readers, local dirty
buffers, and publication races before enabling generation-aware reads by
default. Keep `FOD_CROSS_MOUNT_STRICT_READS=1` as the interim correctness path.
