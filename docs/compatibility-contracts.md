# FOD Compatibility Contracts

This document records the current verified compatibility boundaries of FOD. The
release version is sourced from `fod_version.txt` and the root Cargo workspace.

Inventory date: `2026-07-19`

Reviewed base: commit `9422007` (`FOD 3.2.15`). This documentation update is
prepared for `FOD 3.2.16`.

## Rust Toolchain

- Minimum supported Rust version: 1.85.
- All workspace crates remain on Edition 2021.
- Newer compatible toolchains are allowed; an Edition 2024 migration is not
  required merely because a newer compiler exists.
- There is currently no active GitHub Actions workflow.
- Authoritative local gates are:

  ```bash
  cargo fmt --all -- --check
  cargo check --workspace --locked
  make test-version
  make test-all
  ```

- `make test-all-full` adds broader mounted and indexer coverage.

## FUSE and libfuse

- FOD pins `fuser 0.17.0` with an explicit `libfuse3` backend.
- The configured userspace protocol maximum is FUSE ABI 7.40.
- Startup diagnostics report the fuser version, kernel protocol, negotiated
  protocol, available capabilities, and the capabilities requested by FOD.
- FOD requests POSIX and flock locking capabilities independently.
- Native `copy_file_range` reaches the FOD callback. Exact clean whole-file copy
  can adopt the source data object without duplicating payload.
- A system libfuse upgrade does not automatically add high-level Rust callbacks;
  missing operations must first appear in the public `fuser` API.
- Protocol availability alone never enables a feature. Every enabled feature
  needs a truthful FOD contract and a regression test.

### Callback status

| Status | Operations |
| --- | --- |
| Explicit and exercised | init, lookup, getattr, readdir, readlink, statfs, xattr operations, access, poll, open, POSIX/flock locking, flush, read, release, setattr, namespace operations, write, copy_file_range, mknod, symlink, link |
| Explicit but not comprehensively exercised | ioctl, bmap |
| Not explicitly implemented | destroy, forget, batch_forget, fsync, opendir, readdirplus, releasedir, fsyncdir, fallocate, lseek |
| Not exposed by the inspected high-level API | native SYNCFS, TMPFILE, STATX callbacks |

Syscall success through a kernel or library fallback is not proof that FOD
implements the corresponding callback.

### Priority follow-ups

- implement `fsync` by reusing the current write-state persistence path;
- measure inode/path cache retention and add `forget` plus `batch_forget` when
  the lookup-reference lifecycle is defined;
- connect selected `fallocate` modes to existing resize and sparse-storage
  machinery and reject unsupported combinations;
- benchmark `readdirplus` before enabling it;
- implement `SEEK_DATA` and `SEEK_HOLE` over block and extent maps;
- repeat the callback/capability inventory after every `fuser` upgrade.

## Rust and C boundary

- FOD binaries consume `fod-rust-hotpath` through its Rust library interface.
- No supported external consumer of a hotpath shared library or C header is
  currently known.
- `rust_hotpath/src/ffi.rs` is internal implementation and test code, not a
  versioned public ABI.
- A future external consumer requires an intentionally designed opaque-handle
  ABI, generated headers, ownership rules, compatibility tests, and a support
  policy.

## libpq

- PostgreSQL access uses manually declared functions dynamically linked from
  libpq.
- `mkfs.fod status` and FUSE startup diagnostics report observed client and
  server versions.
- A successful connection is not a declaration that every client/server pair is
  supported. A support range must come from a repeatable compatibility matrix.

## PostgreSQL storage

- Current schema version: 18.
- Fresh installs use `migrations/base_schema.sql`; numbered files preserve the
  upgrade path.
- Payload rows are owned by `data_object_id`.
- One data object uses block payload or extent payload, never both.
- Exact whole-object adoption may share one data object between independent file
  rows and update reference counts instead of copying payload.
- Missing block positions represent sparse or zero logical ranges; they are not
  block-content deduplication.
- Orphan payload rows and reference-count mismatches are invalid.

### Capacity contract

- `config.max_fs_size_bytes` is the canonical payload limit and the `statfs`
  capacity ceiling. Zero disables the configured quota.
- Block and extent persistence use one PostgreSQL transaction-scoped advisory
  lock for the capacity decision.
- Exceeding the limit returns `ENOSPC` and rolls back the complete payload
  transaction.
- `payload_capacity_reservations` stores expiring copy admission tokens.
- Active reservations reduce `statfs` free space and participate in competing
  capacity decisions.
- Persistence revalidates and renews its reservation under the same quota lock.
- An expired reservation cannot reclaim capacity committed by another operation.
- Stale reservations expire after one hour; normal paths release them explicitly.
- Exact data-object adoption creates no payload and needs no capacity reservation.
- The two-mount regression forces independent writers behind the same advisory
  lock and verifies one commit, one `ENOSPC`, no rejected payload, and no leaked
  reservation.

## Space accounting

- `st_size` and apparent `du` report logical file length.
- `st_blocks` uses POSIX 512-byte units and reports allocation attributed to the
  file data object. Sparse positions without payload rows do not contribute.
- Independent files sharing one data object can each expose attributed
  allocation, so summed `du` may count shared payload more than once.
- `df` reports persisted block/extent payload plus active reservations against
  canonical capacity.
- PostgreSQL relation size is a separate physical metric affected by indexes,
  tuple overhead, TOAST, compression, dead tuples, and free space.
- Mounted tests verify this contract before and after remount.

## Storage-format evolution

- Current physical layouts are blocks and opt-in extents under schema version 18.
- Incompatible structure or interpretation changes require a schema migration.
- Per-object format markers are appropriate only when multiple layouts must
  coexist and readers can dispatch safely.
- The durable decision is recorded in
  `docs/adr/storage-format-versioning.md`.

## Summary

| Boundary | Current contract |
| --- | --- |
| FUSE | fuser 0.17.0, ABI maximum 7.40, explicit libfuse3 |
| Rust | minimum 1.85, Edition 2021 |
| Automation | no active GitHub Actions workflow |
| Hotpath ABI | internal Rust interface; no supported public C ABI |
| Database schema | version 18 |
| Capacity | canonical DB quota, advisory-lock admission, expiring reservations |
| Layout | block or extent payload per object |
