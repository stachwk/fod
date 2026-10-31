# FOD compatibility status and contracts

This document is the short compatibility status page. The POSIX requirement
ledger, Linux/libfuse runtime contract, and errno mapping each have one detailed
owner linked below. The release version is sourced from `fod_version.txt` and
the root Cargo workspace.

Inventory date: `2026-10-31`

Current source version: FOD 3.4.514. POSIX evidence remains individually
versioned by regression in the [POSIX audit](POSIX_CONFORMANCE_AUDIT.md); the
current version does not imply that any Pending area is closed. Recent
PostgreSQL result hardening rejects malformed booleans at lease, fencing,
schema-admission and indexer-cleanup boundaries; see the
[primary-authority audit](FOD_PRIMARY_AUTHORITY_AUDIT.md). FOD 3.4.415 raised
the enforced Linux minimum to 6.17.0; FOD 3.4.422 raised the libfuse3 minimum
to 3.18.2, with mounted validation on 3.18.2 and 3.18.3. Kernel version
preflight rejects malformed numeric fields while retaining
distribution suffixes after the final numeric component. FOD 3.4.425 preserves
ACL evaluation errors from `open()` instead of mapping backend failures to
permission denials. FOD 3.4.426 adds a mounted regression verifying malformed
stored ACL data returns `EINVAL` from the callback. FOD 3.4.427 maps poisoned
per-handle write-state
locks to `EIO` instead of treating them as absent; a deterministic unit
regression covers this failure boundary. FOD 3.4.428 expands mounted timestamp
checks for invalid nanoseconds in both `utimensat()` fields and both mixed
`UTIME_NOW`/`UTIME_OMIT` orders. It improves P06 evidence without changing the
POSIX matrix status.
FOD 3.4.429 also records the mounted readdir snapshot regression on FOD 3.4.428,
including `EMFILE` from both `open()` and `opendir()` under the process
descriptor limit; P02 remains Pending. FOD 3.4.430 adds passing mounted
`rename()` tests for file/symlink replacement pairings and rollback preservation.
FOD 3.4.431 extends these tests with explicit cross-mount visibility checks for
committed changes and preservation after rejected changes. FOD 3.4.432 adds
FIFO-over-file and file-over-FIFO cases, checking type and cross-mount
visibility. FOD 3.4.433 verifies sticky-directory rename denial
returns `EPERM` and preserves both entries. FOD 3.4.434 verifies that Linux
checks source and destination ownership independently: owning only one entry
still returns `EPERM`, while owning both allows rename. P08 remains Pending for
remaining POSIX requirements and error cases. The preceding release
implements and verifies Linux FUSE killpriv handling for write, truncate,
`O_TRUNC` and ACL updates; these changes do not close a broader POSIX matrix
area.
FOD 3.4.435 fixes ACL-enabled create mode handling: ACL mounts require negotiated
`FUSE_POSIX_ACL` and `FUSE_DONT_MASK`, so Linux passes the original mode and
umask to FOD. A default ACL filters the original requested mode; without one,
FOD applies umask in the primary transaction. Mounted checks cover capability
negotiation, file and directory creation, and recursive inheritance. FOD
3.4.436 adds mounted checks that umask is applied to file, directory and FIFO
creation when no default ACL exists. P07 remains Pending for the rest of the
permissions and identity contract.
The POSIX audit remains open; the preceding copy-source fencing fix improves
multi-host data correctness but does not close a POSIX audit area. The latest
POSIX-specific mounted evidence remains the individually versioned cases
recorded in the [POSIX audit](POSIX_CONFORMANCE_AUDIT.md). The mounted
`test_readdir_snapshot_multimount.py` regression passed on FOD 3.4.411 and
verifies `EMFILE` from `open()` and `opendir()` plus the valid `closedir()`
success path. An earlier probe that closed a stream's `dirfd()` and observed
`EBADF` was removed from the POSIX test because POSIX defines that use as
undefined behavior. No new POSIX area is marked Pass.

## Overall filesystem compatibility status

### Wniosek w skrócie

POSIX.1-2024 określa wymagane zachowanie interfejsów; Linux realizuje syscalle i
VFS, a FUSE przenosi żądania między jądrem i procesem filesystemu. FOD dostarcza
callbacki po stronie userspace. Wynik widziany przez program zależy od całego
łańcucha, więc test udanego syscalla sam nie wskazuje, która warstwa wykonała
operację. Audyt obejmuje wybrane zachowania, nie certyfikuje całego Linuxa,
protokołu FUSE ani zgodności FOD z całym POSIX.

| Obszar | Etap | Co potwierdzono | Co pozostało |
| --- | --- | --- | --- |
| POSIX.1-2024 | Audyt wymagań w toku; brak ogólnej deklaracji zgodności | Dwie grupy przypadków `lseek()` mają status Pass: zwykłe pozycjonowanie oraz `SEEK_DATA`/`SEEK_HOLE`. | 12 obszarów P01–P12 pozostaje Pending, ponieważ ich pełny zakres wymagań, błędów lub dowodów nie został zamknięty. Szczegóły i kryteria zamknięcia są w [audycie POSIX](POSIX_CONFORMANCE_AUDIT.md). |
| Linux kernel/VFS | Wybrane zachowania przejrzane i sprawdzone, bez ogólnej deklaracji zgodności | Testy montowane i przegląd źródeł Linux v6.17 obejmują m.in. rozwiązywanie ścieżek, errno, uprawnienia, deskryptory i operacje kończone w VFS przed callbackiem. Preflight odrzuca jądra starsze niż 6.17, w tym 4.18. | Linux 6.17.0 jest egzekwowanym minimum i ma testy montowane na 6.17.0-41; wzmianka o 4.18 w historycznym raporcie strojenia nie oznacza wsparcia. Brak pełnej macierzy wersji/dystrybucji i przeglądu wszystkich zachowań jądra. |
| FUSE/libfuse | FOD używa `fuser 0.18.0` z backendem libfuse3; protokół i capability są negocjowane w runtime | Testy montowane na Linux 6.17/libfuse3 3.18.2 i 3.18.3 obejmują mount/unmount, zapis/odczyt, uprawnienia i killpriv. | libfuse3 3.18.2 jest egzekwowanym minimum; nie objęto testami wszystkich dystrybucji ani wszystkich callbacków. Szczegóły ma [przewodnik Linux/FUSE](FUSE_REQUIREMENTS.md). |

`Pass` oznacza wyłącznie wskazaną grupę przypadków, a nie cały obszar POSIX.
Linuxowe zachowanie syscalla nie dowodzi, że request dotarł do callbacku FOD.
Protokół oferowany przez jądro również nie oznacza, że FOD obsługuje każdą
operację lub capability.

### Jak czytać dokumentację zgodności

1. Ten dokument jest krótkim stanem i określa granice wsparcia.
2. [Audyt POSIX](POSIX_CONFORMANCE_AUDIT.md) zawiera macierz wymagań oraz
   rejestr P01–P12 z dowodami i pozostałymi krokami.
3. [Wymagania Linux/FUSE](FUSE_REQUIREMENTS.md) opisują minimum uruchomieniowe,
   negocjację protokołu/capability i testowane kombinacje.
4. [Macierz errno](POSIX_ERRNO_MATRIX.md) przypisuje przykładowe wyniki do
   POSIX, Linux VFS, FUSE lub FOD; [referencja errno](ERRNO_REFERENCE.md)
   definiuje kody używane w Rust.

Szczegółowy audyt jest właścicielem statusu P01–P12, a przewodnik FUSE
właścicielem minimalnych wersji i zakresu callbacków. Wnioski z innych
dokumentów powinny linkować do tych właścicieli, zamiast tworzyć równoległe
liczniki zgodności.

Wynik widziany przez aplikację tworzą wspólnie Linux VFS, uzgodniony protokół
FUSE/libfuse i callbacki FOD. Sukces syscalla nie wskazuje sam w sobie, która
warstwa wykonała lub odrzuciła operację. Powyższa tabela jest jedynym skróconym
podsumowaniem statusu. Wymagania POSIX opisuje
[`POSIX_CONFORMANCE_AUDIT.md`](POSIX_CONFORMANCE_AUDIT.md), a minimum Linux,
negocjację FUSE i listę callbacków — [`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md).

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
- Runtime preflight requires Linux 6.17.0; runtime and build checks require
  libfuse3 3.18.2. FOD verifies the full `fuse_pkgversion()` value because
  `fuse_version()` omits the patch component. Mounted evidence covers Linux
  6.17.0-41 with libfuse3
  3.18.2 and 3.18.3; other kernel/distribution pairs remain unvalidated.

FUSE is the Linux kernel/userspace request protocol; libfuse is a userspace
reference library and FOD uses the Rust `fuser` API over its libfuse3 backend.
The kernel, backend and FOD callback each impose separate compatibility limits.
The callback inventory, including implemented-but-partially-tested and
unavailable operations, has one owner in
[`FUSE_REQUIREMENTS.md`](FUSE_REQUIREMENTS.md). A successful syscall may be
handled by Linux VFS or a library fallback without invoking an FOD callback.

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

- Current schema version: 34 (the migration manifest in
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
| Linux/FUSE | Linux >= 6.17.0 and libfuse3 >= 3.18.2 enforced; fuser 0.18.0, protocol negotiated at runtime, mount validation Linux 6.17.0-41/libfuse3 3.18.2 and 3.18.3 |
| Rust | minimum 1.85, Edition 2021 |
| Automation | no active GitHub Actions workflow |
| Hotpath ABI | internal Rust interface; `libfod.so` is command-surface only; no supported hotpath C ABI |
| Database schema | version 34; authoritative manifest in `rust_mkfs/src/main.rs` |
| Capacity | canonical DB quota, advisory-lock admission, expiring reservations |
| Layout | canonical block payload; legacy extents only before migration 20 |
