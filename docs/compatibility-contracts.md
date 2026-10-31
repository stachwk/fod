# FOD compatibility status and contracts

This document records the current verified compatibility boundaries of FOD. The
release version is sourced from `fod_version.txt` and the root Cargo workspace.

Inventory date: `2026-10-31`

Reviewed base: commit `6e2d936` (`FOD 3.4.393`); test/documentation review
updated for FOD 3.4.394. The latest mounted
`statvfs()` signal-interruption regression cited below passed on FOD 3.4.387;
3.4.391 fixes `st_nlink` saturation at the FUSE ABI limit; its conversion
boundary has a unit test but no mounted test with an unrepresentably large link
count. The 3.4.392 review classifies native `stat()` overflow for the audited
x86_64 ABI. FOD 3.4.393 added a `closedir()` `EBADF` assertion. FOD 3.4.394
adds an `open()` `EMFILE` assertion at the process descriptor limit. The local
helper passed; the two-mount test could not run without PostgreSQL. No new POSIX area
is marked Pass.

## Overall filesystem compatibility status

FOD exposes filesystem operations through Linux FUSE. The application-visible
result is produced by Linux VFS, FUSE protocol/libfuse and FOD callbacks
together; success at the syscall boundary does not identify which layer
implemented or rejected an operation. The [POSIX audit](POSIX_CONFORMANCE_AUDIT.md)
uses POSIX.1-2024 Issue 8 as its baseline and makes no general POSIX-conformance
claim.

| Area | Implemented boundary | Verification status |
| --- | --- | --- |
| POSIX filesystem API | Broad Linux filesystem operations, metadata, permissions, namespace changes, regular-file locks and FIFO behavior | Issue 8 audit remains open: 2 `lseek()` case groups are Pass and 12 areas remain Pending. No overall POSIX conformance claim is made. P07 includes paused-replica checks for `open()` denial and `access(X_OK)` refresh with kernel `default_permissions`; `statvfs()` has a mounted `EINTR` regression. The detailed P01-P12 ledger owns remaining work. |
| Linux kernel/VFS | Linux-only runtime; startup rejects kernels older than 4.18.0. Linux VFS performs pathname, descriptor, permission and syscall work before or after FUSE callbacks. | Version-gate unit tests cover the declared floor; source-level Linux v6.17 review classifies selected VFS behavior and errno precedence. Recorded mounted evidence is concentrated on Linux 6.17; no end-to-end run on Linux 4.18 is recorded. Linux-specific differences and callback short-circuits are attributed in the errno matrix. |
| FUSE/libfuse | `fuser 0.18.0`, explicit libfuse3 backend, userspace protocol ceiling 7.40, runtime protocol/capability negotiation | Mounted evidence uses Linux 6.17/libfuse 3.17.4. The declared libfuse3 3.0.0 floor has preflight coverage but no minimum-version mount run. Writable mounts require negotiated `FUSE_ATOMIC_O_TRUNC`; POSIX/flock locking capabilities are requested and ACL capability is conditional. `FUSE_INTERRUPT` is tested for blocked `statfs` and POSIX lock requests. Callback coverage and omissions are listed below. |
| Product platform | 64-bit and wider userspace only | 32-bit binaries and ABI targets are outside the support matrix. |

This summary deliberately distinguishes a declared runtime floor from tested
platform combinations and from POSIX requirements. The detailed current Linux
and FUSE requirements, including the remaining minimum-stack validation gap,
are in [`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md).

## Supported Build Architectures

FOD binaries and packages are built for 64-bit and wider userspace
architectures only. 32-bit binaries, packages and build targets are not part of
the supported product matrix and must not be produced or published. Platform
and ABI checks in the release process must follow this boundary; 32-bit-only
POSIX behavior is documented as outside supported-platform coverage.

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

- FOD pins `fuser 0.18.0` with an explicit `libfuse3` backend.
- FUSE protocol availability is reported from runtime negotiation; `fuser 0.18.0`
  no longer exposes the previous `abi-7-*` Cargo feature flags used by the
  earlier `0.17.0` build.
- Startup diagnostics report the fuser version, kernel protocol, negotiated
  protocol, available capabilities, and the capabilities requested by FOD.
- FOD requests POSIX and flock locking capabilities independently.
- Native `copy_file_range` reaches the FOD callback. Exact clean whole-file copy
  can adopt the source data object without duplicating payload.
- A system libfuse upgrade does not automatically add high-level Rust callbacks;
  missing operations must first appear in the public `fuser` API.
- Protocol availability alone never enables a feature. Every enabled feature
  needs a truthful FOD contract and a regression test.
- A writable mount requires the negotiated `FUSE_ATOMIC_O_TRUNC` capability;
  startup rejects the mount if Linux does not offer it. `FUSE_MAX_PAGES` affects
  request sizing but is not required for basic correctness.
- The runtime preflight requires Linux 4.18.0 and libfuse3 3.0.0. These are
  enforced minimums, not a claim that every supported kernel/libfuse pair has
  passed mounted tests. The latest recorded mounted stack is Linux 6.17 with
  libfuse 3.17.4.

### Callback status

| Status | Operations |
| --- | --- |
| Explicit and tested for selected behavior | init, lookup, forget, getattr, opendir/readdir/releasedir, readlink, statfs, xattr operations, access, poll, open/create, POSIX/flock locking, flush, fsync, read/write/release, setattr, namespace operations, copy_file_range, mknod, symlink, link |
| Explicit but not comprehensively exercised | ioctl, bmap, fallocate (supported mode subset), lseek (`SEEK_DATA`/`SEEK_HOLE`) |
| Not explicitly implemented | destroy, batch_forget, readdirplus, fsyncdir |
| Not exposed by the inspected high-level API | native SYNCFS, TMPFILE, STATX callbacks |

`opendir()` checks the directory and read permission and creates FOD directory
handle state; `releasedir()` drops that state. These callbacks exist in
`rust_fuse/src/fs.rs`; their presence alone does not imply complete POSIX
directory-stream conformance. `readdir()` has separate mounted coverage.

Syscall success through a kernel or library fallback is not proof that FOD
implements the corresponding callback.

Linux FUSE sends the `lseek` callback for `SEEK_DATA` and `SEEK_HOLE`; the
kernel handles ordinary `SEEK_SET`, `SEEK_CUR` and `SEEK_END` positioning.
POSIX.1-2024 now specifies `SEEK_DATA` and `SEEK_HOLE` in Issue 8, although
they were Linux/BSD-style extensions in earlier editions. FOD's mounted tests
cover both the kernel-handled positioning cases and its sparse-seek callback.

### Priority follow-ups

- add explicit failure-injection coverage for `fsync` once a narrow mounted
  PostgreSQL error-injection harness exists;
- measure inode/path cache retention and add `forget` plus `batch_forget` when
  the lookup-reference lifecycle is defined;
- connect selected `fallocate` modes to existing resize and sparse-storage
  machinery and reject unsupported combinations;
- benchmark `readdirplus` before enabling it;
- repeat the callback/capability inventory after every `fuser` upgrade.

## Rust and C boundary

- FOD binaries consume `fod-rust-hotpath` through its Rust library interface.
- FOD binaries remain self-contained executables; `fod-monitor`,
  `fod-indexer`, `fod-rust-fuse`, and the mkfs/config commands do not load or
  depend on `libfod.so`.
- `libfod.so` is an external command-surface aggregate over installed FOD
  commands. It exposes command metadata and launcher entrypoints for the
  `fod-*`/`mkfs.fod` tools through `rust_libfod/include/fod/libfod.h`, but it
  is not the data hot path and is not a replacement dependency for the tools
  themselves.
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

- Current schema version: 32 (the migration manifest in
  `rust_mkfs/src/main.rs` is authoritative). Schema versions 20 and 21
  introduced the legacy extent conversion and removal described below.
- Fresh installs use `migrations/base_schema.sql`; numbered files preserve the
  upgrade path.
- Payload rows are owned by `data_object_id`.
- Existing payload rows must be canonical `data_blocks`; `data_extents` is
  removed by schema migration 21 after migration 20 converts any legacy rows.
- Exact whole-object adoption may share one data object between independent file
  rows and update reference counts instead of copying payload.
- Missing block positions represent sparse or zero logical ranges; they are not
  block-content deduplication.
- Orphan payload rows and reference-count mismatches are invalid.

### Capacity contract

- `config.max_fs_size_bytes` is the canonical payload limit and the `statfs`
  capacity ceiling. Zero disables the configured quota.
- Block persistence uses one PostgreSQL transaction-scoped advisory lock for the
  capacity decision.
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
- `df` reports persisted block payload plus active reservations against
  canonical capacity.
- PostgreSQL relation size is a separate physical metric affected by indexes,
  tuple overhead, TOAST, compression, dead tuples, and free space.
- Mounted tests verify this contract before and after remount.

## Storage-format evolution

- Current physical layout is canonical `data_blocks`; schema version 21 finalized the legacy extent removal.
  Legacy `data_extents` rows are migrated administratively and the table is
  dropped before normal writes.
- Incompatible structure or interpretation changes require a schema migration.
- Per-object format markers are appropriate only when multiple layouts must
  coexist and readers can dispatch safely.
- The durable decision is recorded in
  `docs/adr/storage-format-versioning.md`.

## Summary

| Boundary | Current contract |
| --- | --- |
| Filesystem API | POSIX.1-2024 audit open; no overall conformance claim |
| Linux/FUSE | Linux >= 4.18.0 and libfuse3 >= 3.0.0 enforced; fuser 0.18.0, protocol negotiated at runtime, latest recorded mount validation Linux 6.17/libfuse3 3.17.4 |
| Rust | minimum 1.85, Edition 2021 |
| Automation | no active GitHub Actions workflow |
| Hotpath ABI | internal Rust interface; `libfod.so` is command-surface only; no supported hotpath C ABI |
| Database schema | version 32; authoritative manifest in `rust_mkfs/src/main.rs` |
| Capacity | canonical DB quota, advisory-lock admission, expiring reservations |
| Layout | canonical block payload; legacy extents only before migration 20 |
