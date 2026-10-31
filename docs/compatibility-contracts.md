# FOD compatibility status and contracts

This document records the current verified compatibility boundaries of FOD. The
release version is sourced from `fod_version.txt` and the root Cargo workspace.

Inventory date: `2026-10-31`

Reviewed source baseline: commit `e4510f0` (`FOD 3.4.397`). The POSIX audit
remains open; the 3.4.397 copy-source fencing fix improves multi-host data
correctness but does not close a POSIX audit area. The latest POSIX-specific
mounted evidence remains the individually versioned cases recorded in the
[POSIX audit](POSIX_CONFORMANCE_AUDIT.md). Its two latest directory-stream
error assertions (`EMFILE` on `open()` and `EBADF` on `closedir()`) have only
passed their local helper so far, not the mounted regression. No new POSIX area
is marked Pass.

## Overall filesystem compatibility status

### Wniosek w skrócie

FOD jest filesystemem dostępnym przez Linux VFS i FUSE. Zgodność widziana przez
program zależy łącznie od jądra, uzgodnionego protokołu FUSE/libfuse i callbacków
FOD. Audyt obejmuje wybrane zachowania, nie certyfikuje całego Linuxa, FUSE ani
POSIX.

| Obszar | Etap | Co potwierdzono | Co pozostało |
| --- | --- | --- | --- |
| POSIX.1-2024 | Audyt wymagań w toku; brak ogólnej deklaracji zgodności | Dwie grupy przypadków `lseek()` mają status Pass: zwykłe pozycjonowanie oraz `SEEK_DATA`/`SEEK_HOLE`. | 12 obszarów P01–P12 pozostaje Pending, ponieważ ich pełny zakres wymagań, błędów lub dowodów nie został zamknięty. Szczegóły i kryteria zamknięcia są w [audycie POSIX](POSIX_CONFORMANCE_AUDIT.md). |
| Linux kernel/VFS | Zgodność sprawdzona dla wybranych zachowań Linux v6.17 | Testy montowane i przegląd źródeł v6.17 obejmują m.in. rozwiązywanie ścieżek, errno, uprawnienia, deskryptory i skróty VFS przed callbackiem. | Linux 4.18 jest egzekwowanym minimum, ale brak montowanego testu end-to-end na tej wersji. Nie ma pełnej macierzy wersji/dystrybucji ani audytu wszystkich zachowań jądra. |
| FUSE/libfuse | Frontend działa przez `fuser 0.18.0`/libfuse3; protokół i capability są negocjowane w runtime | Najnowszy zapisany zestaw montowany: Linux 6.17, libfuse3 3.17.4; testy wybranych callbacków, negocjacji i przerwań. | libfuse3 3.0.0 jest egzekwowanym minimum bez testu mounta na tej wersji. Nie wszystkie operacje mają callback FOD; właścicielem szczegółowej listy jest [przewodnik Linux/FUSE](FUSE_REQUIREMENTS.md). |

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
- The runtime preflight requires Linux 4.18.0 and libfuse3 3.0.0. These are
  enforced minimums, not a claim that every supported kernel/libfuse pair has
  passed mounted tests. The latest recorded mounted stack is Linux 6.17 with
  libfuse 3.17.4.

The FUSE callback inventory, including implemented-but-partially-tested and
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
