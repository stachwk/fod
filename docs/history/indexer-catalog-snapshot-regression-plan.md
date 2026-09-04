# Catalogue snapshot regression plan

Status: implemented for FOD 3.2.22.

## Goal

Protect the immutable catalogue snapshot contract with a PostgreSQL-backed integration regression that uses real, non-empty index data.

## Scope

The regression is executed through the existing `test-fod-indexer-plan-import-scope` path, which is already part of `test-fod-indexer-parallel-smoke` and therefore `make test-all-full`.

The test must verify:

1. a local source with three files can be scanned and hashed;
2. `snapshot create --source` stores exactly those three catalogue rows;
3. `snapshot list` and `snapshot show` expose the stored header;
4. `file list`, `file search`, and `file show` can read the snapshot by `--snapshot-id`;
5. changing, deleting, and adding live source files followed by `scan` and `clean` changes the live catalogue;
6. the stored snapshot keeps the original paths and sizes;
7. deleting the snapshot removes its copied rows;
8. reading the deleted snapshot fails with `catalog_snapshot_not_found`;
9. all temporary source, index, and snapshot state is removed even when the test fails.

## Implementation

- Add `tests/integration/test_fod_indexer_catalog_snapshot.py`.
- Invoke its regression from `tests/integration/test_fod_indexer_plan_import_scope.py`.
- Keep production snapshot code and database schema unchanged.
- Keep source names and paths unique so the test remains safe during parallel indexer smokes.
- Synchronize the project version and lockfile to FOD 3.2.22.

## Validation

Run:

```bash
make test-fod-indexer-plan-import-scope
RUSTFLAGS="-D warnings" cargo check --workspace --locked
RUSTFLAGS="-D warnings" cargo test --locked -p fod-rust-indexer
make test-version
```

For the full local gate, run:

```bash
make test-all-full
```
