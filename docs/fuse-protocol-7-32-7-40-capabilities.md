# FUSE Protocol 7.32-7.40 Capability Inventory

Inventory date: `2026-07-12`

Inventory base commit: `cbee716`

This document classifies the FUSE protocol surface added after the frozen ABI
7.31 baseline. Protocol availability, kernel advertisement, fuser API
exposure, and safe FOD semantics are separate facts. No capability is enabled
only because the negotiated protocol is 7.40.

## Verified Runtime

The validation host kernel advertised FUSE protocol 7.44. FOD uses
`fuser 0.17.0`, compiles protocol maximum 7.40, and negotiates 7.40. The
structured init diagnostic confirmed that the host advertises, among other
flags, `HANDLE_KILLPRIV_V2`, `SETXATTR_EXT`, `SECURITY_CTX`,
`CREATE_SUPP_GROUP`, `HAS_EXPIRE_ONLY`, `DIRECT_IO_ALLOW_MMAP`, `PASSTHROUGH`,
`NO_EXPORT_SUPPORT`, and `HAS_RESEND`.

Open-response flags such as `FOPEN_NOFLUSH` and
`FOPEN_PARALLEL_DIRECT_WRITES` are selected per open and are not init
capabilities. Their protocol availability follows the negotiated minor
version, but FOD must still provide safe semantics before returning them.

## Primary Candidates

| Capability | Protocol | Kernel/runtime | fuser 0.17 API | Relevance to FOD | Correctness risk | Benchmark or gate | Decision |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| `FUSE_SYNCFS` | 7.34 | Kernel protocol supports the opcode | No opcode parser and no `Filesystem::syncfs` callback | A filesystem-wide durability boundary could be useful | High if success is returned without flushing every dirty handle and PostgreSQL transaction | Dirty handles across multiple files, injected persistence failure, `syncfs`, remount | Blocked on upstream/high-level API and a defined global durability contract |
| `FOPEN_NOFLUSH` | 7.35 | Available through negotiated protocol | Exposed as `FopenFlags::FOPEN_NOFLUSH` | Could remove duplicate close-time flush callbacks | High: FOD `flush` returns persistence errors to `close`, while `release` cannot return them and currently treats a failed final persist as non-fatal | Clean close, dirty close, duplicated descriptors, lock-owner cleanup, injected DB error, remount, callback benchmark | Do not enable until final persistence and error propagation no longer depend on `flush` |
| `FUSE_INIT_EXT` and `flags2` | 7.36 | Used by the 7.44 kernel handshake | Parsed and emitted internally by fuser; intentionally hidden from `KernelConfig::capabilities()` | Required transport for capability bits above 31 | Low as internal protocol framing; it is not an application feature | Mounted init diagnostic with high capability bits | Already handled by fuser; no FOD switch or experiment |
| `FUSE_TMPFILE` | 7.37 | Kernel protocol supports the opcode | No opcode parser and no `Filesystem::tmpfile` callback | Could support `O_TMPFILE` workflows | High: unnamed inode lifetime, linking, cleanup, ownership, and transaction recovery need an explicit SQL model | Create unnamed file, write, `linkat(AT_EMPTY_PATH)`, crash/remount, orphan checks | Blocked on API and missing namespace/lifetime contract |
| `FOPEN_PARALLEL_DIRECT_WRITES` | 7.38 | Available through negotiated protocol | Exposed as `FopenFlags::FOPEN_PARALLEL_DIRECT_WRITES` | Potential direct-I/O throughput experiment | High: FOD defaults to one fuser worker; overlapping clone/update of per-handle `WriteState` is not a concurrency contract | Multi-thread session first; disjoint and overlapping same-file writes, two handles, transaction ordering, replay, fio direct I/O | Do not enable; revisit only after an independently verified multi-thread write design |
| `FUSE_STATX` | 7.39 | Kernel protocol supports the opcode | No opcode parser, request/reply types, or `Filesystem::statx` callback | Useful only if FOD can answer requested masks and birth time precisely | Medium/high if masks or attributes are invented; current `getattr` remains the truthful surface | Mask-specific statx calls, fallback behavior, timestamp/attribute comparison | Blocked on API; do not emulate unsupported fields |
| `FUSE_DIRECT_IO_ALLOW_MMAP` | 7.39 | Advertised by this host | Exposed as `InitFlags::FUSE_DIRECT_IO_ALLOW_MMAP` | Applies only to diagnostic/opt-in `FOPEN_DIRECT_IO` mounts | High page-cache, dirty-page, mmap/write, truncate, and multi-mount coherence risk | Shared/private mmap, dirty writeback, truncate, concurrent writer, remount, two mounts | Do not request without a separate mmap coherence project |
| `FUSE_PASSTHROUGH` and `FOPEN_PASSTHROUGH` | 7.40 | `PASSTHROUGH` advertised by this host | Init flag, stack-depth configuration, backing-fd registration, and passthrough replies are exposed | Not aligned with PostgreSQL-owned payloads; FOD has no backing file descriptor per data object | Architectural: kernel I/O would bypass FOD storage, locking, replay, cache, and journal logic | Only meaningful for a new backing-file storage architecture | Reject for the current PostgreSQL storage model |

`fuser 0.17` maps unknown opcodes to `ENOSYS`, but it does not expose enough
request data for FOD to implement `SYNCFS`, `TMPFILE`, or `STATX` in the
high-level trait. FOD must not patch private parser structures merely to claim
support. A future upstream fuser release, a deliberate maintained fork, or a
libfuse3 low-level implementation would be a separate architectural decision.

## Supporting Flags By Protocol Version

| Protocol | Surface | Runtime observation | FOD decision |
| ---: | --- | --- | --- |
| 7.32 | `FUSE_SUBMOUNTS`, submount attributes | `SUBMOUNTS` was not advertised in the captured available set | Not relevant to the current flat PostgreSQL namespace; do not request |
| 7.33 | `FUSE_HANDLE_KILLPRIV_V2`, `FUSE_SETXATTR_EXT` | Both advertised; fuser exposes init flags and existing write/setxattr surfaces | Do not request until setuid/setgid/capability removal is verified across write, truncate, chown, ACL, and two mounts |
| 7.36 | `FUSE_SECURITY_CTX`, inode DAX | Both advertised | Security-context request extensions are not exposed as a complete FOD create contract; DAX does not match PostgreSQL payloads |
| 7.38 | supplementary create group and expire-only invalidation | `CREATE_SUPP_GROUP` and `HAS_EXPIRE_ONLY` advertised | FOD resolves process groups itself and does not use expire-only notifications; leave disabled |
| 7.40 | `FUSE_NO_EXPORT_SUPPORT`, `FUSE_HAS_RESEND` | Both advertised | Inventory as possible correctness signaling only; no performance value and no request-resend contract exposed to FOD |

## Mounted Fallback Probe

On `2026-07-13`, the mounted probe
`tests/integration/test_post_731_capability_fallbacks.py` ran against commit
`aa77738` on kernel `6.17.0-40-generic`. FOD negotiated protocol 7.40 and
requested only its existing `POSIX_LOCKS` and `FLOCK_LOCKS`; the probe enabled
no capability.

| Syscall surface | Mounted result | Interpretation |
| --- | --- | --- |
| `syncfs()` | Returned success | This records the client/kernel-visible result only. Because fuser 0.17 exposes no `Filesystem::syncfs` callback, it does not establish a FOD-wide durability boundary or prove how dirty handles and PostgreSQL failures are ordered. |
| `open(..., O_TMPFILE | O_RDWR)` | Failed with `ENOTSUP` (`errno 95`) | The unsupported result is the safe current behavior. No unnamed object was created and the directory namespace remained unchanged. |
| `statx()` | Returned success; inode, size, mode, uid, gid, and link count matched `os.stat()` | The host supplied truthful basic metadata through its available fallback path. This does not mean FOD implements `FUSE_STATX`, requested-mask semantics, birth time, or future statx-only attributes. |

The probe also confirmed that the known file contents remained intact and the
test directory contained only the named file. No result justifies requesting a
new init flag or returning a new open flag.

## Experiment Order

No new production capability is justified by this inventory. The controlled
follow-up order is:

1. Retain the mounted fallback probe as a regression check: `syncfs()` and
   `statx()` must remain truthful client-visible operations, while `O_TMPFILE`
   must remain clearly unsupported until an unnamed-object contract exists.
2. Keep `FOPEN_NOFLUSH` disabled until FOD can preserve close-time persistence
   errors without the flush callback; only then run the close/remount matrix.
3. Treat `FOPEN_PARALLEL_DIRECT_WRITES` as dependent on a separate multi-thread
   session and write-ordering design, not as a standalone flag benchmark.
4. Leave `FUSE_DIRECT_IO_ALLOW_MMAP` and passthrough out of the current
   architecture unless a concrete mmap or backing-file requirement appears.

Each implemented experiment must remain one capability per commit, include an
explicit semantic contract, and retain the storage, locking, replay, and
schema-v17 invariant gates.
