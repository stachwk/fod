# FOD Compatibility Contracts

This document records the current compatibility boundaries of FOD 3.2.1 before
the planned `fuser` upgrade. It describes verified repository and runtime state,
not the historical migration path.

Inventory date: `2026-07-11`

Inventory base commit: `54668b10a640aedb518bc9942a01700a65a45a44`

## FUSE Boundary

### Build and Runtime

- `rust_fuse/Cargo.toml` selects `fuser = 0.14` with `abi-7-31`.
- The current dependency still enables the `fuser 0.14` default `libfuse`
  feature. The built `fod-rust-fuse` executable dynamically links
  `libfuse3.so.4` on the inventory host.
- The compiled userspace protocol maximum is FUSE ABI 7.31. The current FOD
  startup path does not expose the kernel or negotiated protocol version as a
  structured diagnostic.
- `FodFuse::init()` requests `FUSE_POSIX_LOCKS` and `FUSE_FLOCK_LOCKS`. It does
  not currently report whether each requested capability was accepted.
- Native `copy_file_range` reaches the FOD callback under ABI 7.31.
- An exact clean whole-file copy into an empty destination adopts the source
  `data_object_id`; it does not duplicate payload rows.
- A chunked copy uses the payload-copy path. The current correctness path also
  converts an extent-backed destination before applying a partial block patch,
  preventing a hybrid block/extent object.

The move to `fuser 0.17` must explicitly choose its libfuse feature. Unlike
`fuser 0.14`, the selected `fuser 0.17` release has no default features, so a
dependency-only update would silently change the session backend if the feature
selection were not reviewed.

### Callback Inventory

The classification below concerns explicit `Filesystem` overrides in
`rust_fuse/src/fs.rs`. A syscall test can pass through kernel behavior even when
FOD does not override the corresponding high-level callback, so only explicit
handlers are listed as implemented.

| Status | Callbacks | Evidence and limits |
| --- | --- | --- |
| Implemented and exercised | `lookup`, `getattr`, `readdir`, `readlink`, `statfs`, `setxattr`, `getxattr`, `listxattr`, `removexattr`, `access`, `poll`, `open`, `getlk`, `setlk`, `flush`, `read`, `release`, `setattr`, `mkdir`, `unlink`, `rmdir`, `rename`, `create`, `write`, `copy_file_range`, `mknod`, `symlink`, `link` | Mount, metadata, permission, xattr, poll, multi-mount locking, persistence, namespace, copy, special-node, and link suites exercise these handlers. Exact and chunked copy paths have dedicated mounted tests. |
| Implemented but weakly tested | `init`, `ioctl`, `bmap` | Mount success exercises `init`, but accepted capabilities and negotiated limits are not asserted. The mounted `ioctl` test covers `FIONREAD` and the host-dependent `FICLONE` result; `FIGETBSZ`, inode flags, `fsxattr`, `FICLONERANGE`, and all error shapes are not covered end to end. No mounted `bmap` consumer was found. |
| Not explicitly implemented | `destroy`, `forget`, `batch_forget`, `fsync`, `opendir`, `readdirplus`, `releasedir`, `fsyncdir`, `fallocate`, `lseek`; macOS-only `setvolname`, `exchange`, `getxtimes` | FOD inherits the `fuser` defaults. Existing `fsync`, `fallocate`, and `lseek` syscall-level tests do not make these explicit FOD callback implementations. |
| Available only after the `fuser` upgrade | No additional high-level `Filesystem` method was found in the `fuser 0.17` trait compared with 0.14. Post-7.31 protocol flags include surfaces such as `FOPEN_NOFLUSH`, `FOPEN_PARALLEL_DIRECT_WRITES`, `FUSE_INIT_EXT`, `FUSE_DIRECT_IO_ALLOW_MMAP`, and `FUSE_PASSTHROUGH`. | Availability in the crate or protocol is not evidence that the kernel negotiated a flag or that FOD can enable it safely. `SYNCFS`, `STATX`, and `TMPFILE` are not exposed as new methods in the inspected `fuser 0.17` high-level trait and require a separate API inventory. |

## Rust and C Boundary

### Current Surface

- `fod-rust-hotpath` builds both `rlib` and `cdylib` crate types.
- `rust_hotpath/src/ffi.rs` contains 116 `#[unsafe(no_mangle)] extern "C"`
  exports and 20 `#[repr(C)]` structures. The built shared object exposes the
  same 116 `fod_*` dynamic symbols.
- The shared object is installed as `/usr/local/lib/libfod-2.so`.
- No C header, C/C++ source consumer, `dlopen`/`dlsym` consumer, Python
  `ctypes.CDLL` consumer, or linker reference to `libfod-2.so` exists in this
  repository.
- Rust workspace code consumes the hotpath through the Rust `rlib`. A subset of
  exported functions is called directly by Rust unit tests, which does not test
  dynamic C ABI compatibility.

### Consumer Classification

| Class | Current finding |
| --- | --- |
| External public ABI | None found in the repository. No external consumer has been identified. |
| Internal compatibility API | The repository handle and planner exports are candidates, but no dynamic consumer was found. The installed shared object is therefore an internal compatibility artifact, not a proven public ABI. |
| Test-only or legacy exports | A subset of planner/free functions is invoked directly from Rust tests. The broad repository export set remains from the former FFI boundary, but the active runtime uses Rust APIs directly. |
| Unused exports | Source inspection cannot prove use outside the repository. Within this repository, most dynamic exports have no consumer other than their definitions. Runtime tracing would be required before removal if an out-of-tree consumer is later identified. |

### Ownership Contract and Risks

- Input byte ranges use pointer-plus-length pairs. A non-zero length requires a
  non-null pointer.
- Repository handles are created by `fod_rust_pg_repo_new()` and destroyed by
  `fod_rust_pg_repo_free()`.
- Output arrays are paired with `fod_free_copy_segments()`, `fod_free_ranges()`,
  `fod_free_persist_blocks()`, `fod_free_persist_crc_rows()`, or
  `fod_free_read_blocks()`.
- Byte outputs are paired with `fod_free_bytes()`.
- `DbfsPgRepo` is marked `#[repr(C)]` but embeds the internal Rust `DbRepo`
  layout. It must be treated as an opaque handle if a public ABI is ever
  defined; its layout is not a stable C contract.
- `bytes_to_raw()` forgets a `Vec<u8>` without preserving its capacity, while
  `fod_free_bytes()` reconstructs it with `capacity = len`. Rust does not
  guarantee that an arbitrary vector has `capacity == len`. This allocator
  contract must be corrected or replaced before the byte-output ABI can be
  called public and stable.

No `FOD_HOTPATH_ABI_VERSION` is defined in this phase. Freezing the current
accidental export set would preserve internal layouts and ownership risks
without evidence of an external consumer.

## libpq Boundary

Both PostgreSQL implementations use manually declared, dynamically linked
`libpq` functions.

`rust_hotpath/src/pg.rs` declares:

```text
PQconnectdb
PQstatus
PQerrorMessage
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
PQexec
PQresultStatus
PQntuples
PQnfields
PQgetvalue
PQclear
PQfinish
```

The inventory host links `libpq.so.5` from PostgreSQL client 17.10. The local
test server reports PostgreSQL 16.14. These are observed versions, not yet a
declared supported range. Runtime reporting of `PQlibVersion()` and
`PQserverVersion()` remains a later compatibility task.

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
| FUSE | `fuser 0.14` / ABI 7.31 | `fuser 0.17` / protocol up to 7.40, followed by verified negotiation diagnostics |
| Rust toolchain | Implicit | Explicit minimum compatible with `fuser 0.17` and supported build environments |
| Hotpath C ABI | Unclassified exports; no external consumer found | Inventory-based internal/public decision before changing or freezing symbols |
| libpq runtime | Dynamically linked | Runtime client/server version reporting and a tested-version contract |
| DB schema | Version 17 | Normal migration contract; no compatibility-only schema change |
| Physical layout | Blocks plus opt-in extents | No new format in the FUSE modernization phase |
