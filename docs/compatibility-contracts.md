# FOD Compatibility Contracts

This document records the current compatibility boundaries after the
`fuser 0.17` migration. It describes verified repository and runtime state,
not the historical migration path. The release version remains sourced from
`fod_version.txt` rather than duplicated here.

Inventory date: `2026-07-12`

Last clean base before the migration: `0c48865`

## Rust Toolchain Boundary

- Every workspace package declares a minimum Rust version of 1.85 through
  `[workspace.package]` inheritance.
- All crates remain on Edition 2021. The compiler baseline does not require an
  Edition 2024 migration.
- `fuser 0.17.0` declares Rust 1.85 as its minimum
  supported version.
- The inventory host uses `rustc 1.85.1` and Cargo 1.85.1.
- The SELinux/ACL lab image is based on `rust:1.85-bookworm`.
- The repository currently has no active GitHub Actions definition. The
  compiler minimum is enforced by Cargo package metadata and local/container
  validation; a future automated workflow must select Rust 1.85 or newer
  explicitly.
- No `rust-toolchain.toml` is required. Cargo package metadata enforces the
  minimum for development, debug, release, and profiling builds while allowing
  newer compatible toolchains.
- Distribution packages are acceptable only when both `rustc` and Cargo satisfy
  the 1.85 minimum; installation documentation must not imply that every distro
  package version is sufficient.

## FUSE Boundary

### Build and Runtime

- `rust_fuse/Cargo.toml` selects `fuser = 0.17` with `abi-7-40` and an explicit
  `libfuse3` feature.
- The built `fod-rust-fuse` executable dynamically links `libfuse3.so.4` on the
  inventory host; no libfuse2 or pure-Rust mount fallback is selected.
- The compiled userspace protocol maximum is FUSE ABI 7.40. The FOD init log
  reports the fuser version, userspace maximum, kernel protocol, effective
  negotiated protocol, kernel-available capabilities, and the capabilities
  requested and enabled by FOD.
- `FodFuse::init()` requests `FUSE_POSIX_LOCKS` and `FUSE_FLOCK_LOCKS`. It does
  report whether each requested capability was available and enabled.
- Native `copy_file_range` reaches the FOD callback under ABI 7.31.
- An exact clean whole-file copy into an empty destination adopts the source
  `data_object_id`; it does not duplicate payload rows.
- A chunked copy uses the payload-copy path. The current correctness path also
  converts an extent-backed destination before applying a partial block patch,
  preventing a hybrid block/extent object.
- The measured correctness, callback, SQL, WAL, and throughput reference for
  the migration is frozen in `docs/fuse-abi-7-31-current-baseline.md`.
- After the dependency migration, mounted exact and chunked copies, remount,
  locking, ioctl, poll, metadata, namespace, and fio gates retain their prior
  behavior. The repeated performance comparison is recorded separately.
- Post-7.31 protocol surfaces, API gaps, risks, and decisions are classified in
  `docs/fuse-protocol-7-32-7-40-capabilities.md`; none is enabled merely because
  the runtime negotiated protocol 7.40.

### Negotiation Diagnostics

- The dependency is pinned to `fuser 0.17.0`, and the reported userspace
  protocol maximum is 7.40.
- `KernelConfig::kernel_abi()` and `KernelConfig::capabilities()` provide the
  kernel protocol and kernel-advertised capability set. FOD reports the
  effective protocol as the lower of the kernel protocol and the compiled
  userspace maximum.
- FOD requests `FUSE_POSIX_LOCKS` and `FUSE_FLOCK_LOCKS` independently from the
  available set and reports any unsupported subset instead of discarding the
  result from `add_capabilities()`.
- The public fuser API does not expose getters for the final `max_write`,
  `max_readahead`, `max_background`, or `congestion_threshold` values. FOD
  reports these fields as `unavailable`; it does not infer private state or
  carry a local fuser fork only for diagnostics.
- On the 2026-07-12 validation host, the kernel advertised protocol 7.44, FOD
  negotiated 7.40, and both requested lock capabilities were available and
  enabled. The mounted test verifies the structured fields on every run.

Unlike `fuser 0.14`, `fuser 0.17` has no default mount feature. FOD therefore
selects `libfuse3` explicitly so dependency updates cannot silently switch the
session backend.

### Callback Inventory

The classification below concerns explicit `Filesystem` overrides in
`rust_fuse/src/fs.rs`. A syscall test can pass through kernel behavior even when
FOD does not override the corresponding high-level callback, so only explicit
handlers are listed as implemented.

| Status | Callbacks | Evidence and limits |
| --- | --- | --- |
| Implemented and exercised | `init`, `lookup`, `getattr`, `readdir`, `readlink`, `statfs`, `setxattr`, `getxattr`, `listxattr`, `removexattr`, `access`, `poll`, `open`, `getlk`, `setlk`, `flush`, `read`, `release`, `setattr`, `mkdir`, `unlink`, `rmdir`, `rename`, `create`, `write`, `copy_file_range`, `mknod`, `symlink`, `link` | Mount, negotiated compatibility, metadata, permission, xattr, poll, multi-mount locking, persistence, namespace, copy, special-node, and link suites exercise these handlers. Exact and chunked copy paths have dedicated mounted tests. |
| Implemented but weakly tested | `ioctl`, `bmap` | The mounted `ioctl` test covers `FIONREAD` and the host-dependent `FICLONE` result; `FIGETBSZ`, inode flags, `fsxattr`, `FICLONERANGE`, and all error shapes are not covered end to end. No mounted `bmap` consumer was found. |
| Not explicitly implemented | `destroy`, `forget`, `batch_forget`, `fsync`, `opendir`, `readdirplus`, `releasedir`, `fsyncdir`, `fallocate`, `lseek`; macOS-only `setvolname`, `exchange`, `getxtimes` | FOD inherits the `fuser` defaults. Existing `fsync`, `fallocate`, and `lseek` syscall-level tests do not make these explicit FOD callback implementations. |
| Available in the upgraded protocol surface but not enabled by FOD | No additional high-level `Filesystem` method was found in the `fuser 0.17` trait compared with 0.14. Post-7.31 protocol flags include surfaces such as `FOPEN_NOFLUSH`, `FOPEN_PARALLEL_DIRECT_WRITES`, `FUSE_INIT_EXT`, `FUSE_DIRECT_IO_ALLOW_MMAP`, and `FUSE_PASSTHROUGH`. | Availability in the crate or protocol is not evidence that the kernel negotiated a flag or that FOD can enable it safely. `SYNCFS`, `STATX`, and `TMPFILE` are not exposed as new methods in the inspected `fuser 0.17` high-level trait and require a separate API inventory. |

## Rust and C Boundary

### Decision

The `2026-07-13` host and repository audit on commit `e32853b` found no real
dynamic consumer of the hotpath shared object:

- `fod-rust-fuse` and `fod-rust-indexer` depend on `fod-rust-hotpath` through
  Cargo and use its Rust `rlib` interface.
- No inspected FOD binary had a dynamic dependency on `libfod-2.so` or
  `libfod_rust_hotpath.so`.
- No running process mapped either library name.
- No public C header existed in the repository or `/usr/local/include`.
- The two apparent `ctypes.CDLL` references loaded libc with `CDLL(None)` for
  `syncfs()` and `statx()` probes; they did not load the hotpath library.
- The direct calls to `fod_copy_dedupe()` and `fod_free_ranges()` came from a
  Rust test linked against the crate, not from a dynamic ABI consumer.

`fod-rust-hotpath` therefore builds only as an internal `rlib`. The root install
workflow installs the FOD Rust executables and mount helper but no longer builds
or installs `/usr/local/lib/libfod-2.so`.

A copy left in `/usr/local/lib/libfod-2.so` by an older installation is a legacy
orphan, not a supported runtime dependency. It may be removed after confirming
that no out-of-tree local software depends on it; the FOD install target does
not delete system files implicitly.

### Internal FFI Source

`rust_hotpath/src/ffi.rs` remains internal implementation and test code. Its
`extern "C"` declarations, `#[repr(C)]` structures, symbol names, status values,
layouts, and allocation helpers are not a versioned or supported public ABI.

The pre-decision audit observed 116 `fod_*` dynamic exports and 20 C-layout
structures when the crate was built as a `cdylib`. It also confirmed why that
accidental surface must not be frozen:

- `DbfsPgRepo` embeds the internal Rust `DbRepo`; any future public handle must
  be opaque.
- byte outputs can originate from a forgotten `Vec<u8>`, while
  `fod_free_bytes()` reconstructs allocation metadata from pointer and length;
  a future ABI must preserve the real allocation metadata.
- there is no ABI version query, generated C header, SONAME compatibility
  policy, or cross-version consumer test.

If a real external consumer appears later, it should receive a deliberately
versioned ABI boundary rather than re-enabling the current crate-wide `cdylib`
surface.

## libpq Boundary

Both PostgreSQL implementations use manually declared, dynamically linked
`libpq` functions.

`rust_hotpath/src/pg.rs` declares:

```text
PQconnectdb
PQstatus
PQerrorMessage
PQlibVersion
PQserverVersion
PQexec
PQprepare
PQexecPrepared
PQexecParams
PQputCopyData
PQputCopyEnd
PQgetResult
PQresultStatus
PQresultErrorMessage
PQresultErrorField
PQntuples
PQnfields
PQgetvalue
PQgetlength
PQclear
PQfinish
```

`rust_mkfs/src/pg.rs` declares the subset:

```text
PQconnectdb
PQstatus
PQerrorMessage
PQlibVersion
PQserverVersion
PQexec
PQresultStatus
PQntuples
PQnfields
PQgetvalue
PQclear
PQfinish
```

Both bindings now declare `PQlibVersion()` and `PQserverVersion()`.

`mkfs.fod status` reports:

- the runtime `libpq` version number and normalized version label;
- the runtime PostgreSQL server version number;
- the server's `SHOW server_version` string;
- the client/server major-version relation;
- compatibility as `connected`, meaning that the current connection succeeded.

`fod-rust-fuse` records the same diagnostic once during startup through the
existing `DbRepo` connection pool. A diagnostic lookup failure is logged as a
warning and does not itself block the mount; the normal startup snapshot remains
the operational gate.

The earlier inventory host linked `libpq.so.5` from PostgreSQL client 17.10
against PostgreSQL server 16.14. Those values remain observations, not a
declared support range. `same-major`, `client-newer`, and `client-older` are
descriptive labels only and do not accept or reject a connection.

## PostgreSQL and Storage Contract

- Schema version is 17.
- `data_blocks`, `data_extents`, and `copy_block_crc` payload rows are owned by
  `data_object_id`.
- Physical payload layout is either blocks or extents.
- Exact whole-object adoption is supported by sharing the source data object and
  updating reference counts.
- A data object containing both block and extent rows is invalid.
- Orphan payload rows and data-object reference-count mismatches are invalid.
- This modernization project does not add a new storage format or change the
  schema. Future incompatible storage changes must use the normal schema
  migration contract and first justify whether schema-level or per-object layout
  versioning is needed.

## Compatibility Summary

| Boundary | Current | Planned |
| --- | --- | --- |
| FUSE | `fuser 0.17` / protocol maximum 7.40 / explicit libfuse3 | Verified negotiated-protocol and capability diagnostics, then isolated capability experiments |
| Rust toolchain | Minimum 1.85, Edition 2021 | Keep explicit minimum aligned with dependencies and build environments |
| Hotpath C ABI | Unclassified exports; no external consumer found | Inventory-based internal/public decision before changing or freezing symbols |
| libpq runtime | Dynamically linked; runtime client/server diagnostics exposed | Keep observed versions separate from any future tested support range |
| DB schema | Version 17 | Normal migration contract; no compatibility-only schema change |
| Physical layout | Blocks plus opt-in extents | No new format in the FUSE modernization phase |
