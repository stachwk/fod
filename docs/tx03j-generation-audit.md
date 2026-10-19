# TX03J — PostgreSQL content generation audit (2026-10-19)

## Authority and guarantees

PostgreSQL is the authoritative owner of file mapping, content generation,
write leases and commit ordering. An invalidation is scoped to one `file_id`.
A client may never drop local uncommitted write buffers in response to a
remote event, and no SQL fill started before invalidation may publish later.

## Implemented (feature/forget-convergence-c1)

- Schema v26 adds `files.content_generation` and a BEFORE UPDATE trigger.
- Two isolated PG transaction tests cover rollback, concurrent updates and
  independence across file IDs.
- `DbRepo::fetch_block_range_with_generation_shared` retrieves size,
  generation and blocks in one PostgreSQL statement snapshot; legacy reads
  are unchanged.
- `FileCacheGenerationGate` protects publication and now snapshots its
  generation before reading cached blocks.

## Critical correctness gaps — do NOT enable default multi-host cache yet

1. The v26 trigger increments on changes to `data_object_id`, `size`,
   or `modification_date`. In-place `data_blocks` upserts do not inherently
   update a `files` row. Verify **every** persist/copy/truncate/write path
   increments the authoritative file generation in the committing transaction.
2. `NOW()` in PostgreSQL is constant throughout a transaction. Multiple
   touches in one transaction can leave `modification_date` unchanged.
   A dedicated, explicit generation update protocol is needed; relying on
   timestamps alone is insufficient.
3. `touch_file_entry` advances generation through a timestamp change, which
   can over-invalidate metadata-only changes.
4. The generation-bearing read API is opt-in and is not called by the FUSE
   production path. It is not itself a cross-mount coherence mechanism.
5. Cache coherence also requires selective Linux FUSE kernel page-cache
   invalidation, and safe handling of open readers, local dirty buffers,
   and race ordering. Notifications are only hints; missing notifications
   require durable reconciliation.
6. When reading from replicas, a generation/content snapshot may lag a
   committed primary change; fencing reads or ensuring a sufficiently
   advanced replica snapshot is necessary.
7. Any future migration that corrects generation semantics must be added
   as a **new** numbered migration, rather than editing applied migration 26.

## Test gate on lt7300

Run `cargo fmt --all -- --check` and
`cargo check --workspace --locked` before database tests.

Create an **isolated initialized test database** for destructive mkfs and
quota tests. A database with schema_version=26 but no `config.block_size`
is not a valid hotpath quota-test fixture. Confirm `fod.config` contains
`block_size` and `max_fs_size_bytes` before starting tests.

Run:

```bash
cargo test --locked -p fod-rust-fuse read_cache::tests
cargo test --locked -p fod-rust-hotpath --test pg_query generated_block_snapshot_matches_committed_payload -- --exact --nocapture
cargo test --locked -p fod-rust-hotpath --lib pg::tests::ordinary_persist_copy_merge_completes_before_final_quota_gate -- --exact --nocapture
cargo test --locked -p fod-rust-mkfs --test content_generation
```

Only run PostgreSQL tests with `POSTGRES_DB` / `FOD_PG_DBNAME`
pointing at properly initialized, dedicated test databases.

## Next architectural gate

Implement transactionally explicit generation bumps in all content commit
paths, prove by write-path coverage tests, wire a per-file event/reconcile
ledger and only then enable selective read-cache and kernel invalidation.
Keep `FOD_CROSS_MOUNT_STRICT_READS=1` as the interim correctness path.
