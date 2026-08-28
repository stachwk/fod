# FOD 3.3.24 - Rust 1.98.0 i domyslny profil release-lto

## Decyzja

Od FOD 3.3.24 kanoniczny toolchain uzywany do budowania artefaktow produkcyjnych jest przypiety do Rust `1.98.0`, a domyslny profil instalacyjny FOD to `release-lto`. Od FOD 3.3.26 ta sama para jest takze domyslna dla runtime i testow uruchamianych przez `Makefile`.

Repozytorium zawiera teraz `rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.98.0"
profile = "minimal"
components = ["clippy", "rustfmt"]
```

Profil `minimal` jest celowy. Do kompilacji i walidacji FOD potrzebne sa `rustc`, Cargo, `clippy` i `rustfmt`; dokumentacja Rust nie jest wymagana jako czesc lokalnego toolchainu projektu.

## Dlaczego release-lto

FOD 3.3.23 wprowadzil kontrolowany benchmark tego samego commita w trzech wariantach:

| wariant | sredni zapis 512 KiB | min | max | rozmiar fod-rust-fuse |
| --- | ---: | ---: | ---: | ---: |
| Rust 1.85.0 / `release` | 74.41 MiB/s | 71.99 | 77.38 | 5,208,336 B |
| Rust 1.98.0 / `release` | 71.80 MiB/s | 68.33 | 75.85 | 5,297,144 B |
| Rust 1.98.0 / `release-lto` | 75.30 MiB/s | 73.82 | 76.20 | 3,506,560 B |

Trzy probki nie sa wystarczajace, aby traktowac okolo 1.2% przewagi throughput jako statystycznie pewne przyspieszenie. Wynik pokazal jednak, ze zwykly Rust 1.98 `release` nie daje automatycznej przewagi, natomiast Rust 1.98 z istniejacym profilem ThinLTO zachowuje co najmniej porownywalna wydajnosc i jednoczesnie zmniejsza binarium FUSE o okolo jedna trzecia wzgledem bazowego Rust 1.85 `release`.

Dlatego decyzja 3.3.24 nie opiera sie na deklaracji, ze sam nowszy kompilator musi przyspieszac FOD. Kanoniczna para jest traktowana lacznie:

```text
Rust 1.98.0 + release-lto
```

## Profil produkcyjny

Profil zdefiniowany w `Cargo.toml` pozostaje:

```toml
[profile.release-lto]
inherits = "release"
lto = "thin"
codegen-units = 1
panic = "abort"
strip = "symbols"
```

Standardowe wywolanie `make`, ktore buduje lub uruchamia artefakty FOD z `Makefile`, uzywa teraz domyslnie:

```text
FOD_CARGO_PROFILE=release-lto
```

Jawny override nadal jest mozliwy, np. dla diagnostyki lub porownania historycznego:

```bash
make FOD_CARGO_PROFILE=release cargo-profile-show
```

Od FOD 3.3.26 ten sam domyslny profil obejmuje takze:

- `build-runtime`, czyli standardowy build binariow runtime z `Makefile`;
- `init`, `reset`, `mount`, `indexer`, `fod-change` i pokrewne targety runtime;
- testy Cargo uruchamiane przez zmienne `CARGO_TEST_*` w `Makefile`;
- pomocnicze testy integracyjne, ktore same wyszukuja lokalne binaria.

`FOD_CARGO_TEST_PROFILE` domyslnie dziedziczy `FOD_CARGO_PROFILE`, wiec standardowe targety testowe Makefile uruchamiaja Cargo z:

```text
--profile release-lto
```

`build-debug` i `build-debug-shm` pozostaja dostepne jako jawne targety diagnostyczne. Nie sa jednak zwykla sciezka runtime ani domyslny fallback dla testow.

## MSRV a kanoniczny toolchain

`rust-version` pozostaje:

```toml
rust-version = "1.85"
```

Jest to swiadome rozdzielenie dwoch kontraktow:

- `rust-version = 1.85` oznacza minimalna wersje Rust, z ktora kod zrodlowy i zablokowane zaleznosci maja pozostawac kompatybilne;
- `rust-toolchain.toml = 1.98.0` okresla kompilator uzywany do kanonicznych buildow projektu.

Dzieki temu FOD nie podnosi sztucznie MSRV tylko dlatego, ze nowszy kompilator daje lepsze diagnostyki lub korzystniejszy wynik z ThinLTO.

Kontrola MSRV pozostaje dostepna przez:

```bash
make rust-msrv-check
```

Natomiast zwykle `cargo`, `rustc`, `cargo fmt` i `cargo clippy` uruchomione wewnatrz checkoutu z rustup automatycznie wybieraja Rust 1.98.0 z `rust-toolchain.toml`.

## Ochrona artefaktow runtime

Targety `build-runtime`, `build-libfod` i `install-root-scripts` wymagaja aktywnego `rustc 1.98.0` poprzez `rust-production-toolchain-check`.

Ma to zapobiegac sytuacji, w ktorej system posiada starszy distro `cargo/rustc`, a operator nieswiadomie tworzy oficjalny artefakt innym kompilatorem niz ustalony dla FOD.

Polecenie kontrolne:

```bash
make rust-production-toolchain-check
```

`rust-toolchain.toml` jest zaleznoscia stempli `build-runtime` i `build-debug`. Zmiana przypietego kompilatora wymusza ponowne zbudowanie binariow zamiast pozostawienia starego stempla jako aktualnego.

## Dobor binariow przy montowaniu

Wczesniej lokalne wyszukiwanie rozpoznawalo przede wszystkim:

```text
target/debug
target/release
```

Po przejsciu na `release-lto` mogloby to spowodowac uruchomienie starszego `target/release/fod-bootstrap` albo starszego `fod-rust-fuse`, mimo ze nowy build produkcyjny znajdowal sie w `target/release-lto`.

FOD 3.3.24 dodal `release-lto` do sciezek rozpoznawanych przez `mount.fod` i `fod-bootstrap`. FOD 3.3.26 zaostrza te zasade: normalna sciezka checkoutu nie spada juz automatycznie do lokalnego `target/release` ani `target/debug`.

Dla normalnego montowania checkout preferuje:

```text
target/release-lto
```

W trybie debug `target/debug` pozostaje pierwszym wyborem, ale jest to jawny tryb diagnostyczny.

Dodatkowo wybrany `fod-bootstrap` preferuje `fod-rust-fuse` znajdujacy sie obok niego. Jezeli wrapper wybierze:

```text
target/release-lto/fod-bootstrap
```

to odpowiadajacy daemon ma byc:

```text
target/release-lto/fod-rust-fuse
```

Zapobiega to mieszaniu artefaktow z roznych profili kompilacji.

Jezeli operator chce uruchomic inny artefakt, powinien ustawic jawnie odpowiednia zmienna, np. `FOD_BOOTSTRAP_BIN`, `FOD_MKFS_BIN`, `FOD_RUST_FUSE_BIN` albo `FOD_RUNTIME_PROFILE=debug`.

## Rust 1.98 i granica FFI

Walidacja 3.3.23 przez Clippy 1.98 wykryla, ze publiczna funkcja Rust:

```text
fod_program_find
```

przyjmuje surowy wskaznik C i bezposrednio go dereferencjonuje przez `CStr::from_ptr`, ale jej kontrakt Rust nie byl oznaczony jako `unsafe`.

W 3.3.24 funkcja jest jawnie:

```rust
pub unsafe extern "C" fn fod_program_find(...)
```

i posiada sekcje `# Safety` opisujaca wymaganie poprawnego wskaznika do zakonczonego NUL-em lancucha C.

Nie jest to zmiana ABI C. Naglowek `libfod.h` i symbol eksportowany przez `cdylib` zachowuja ten sam interfejs binarny. Zmiana doprecyzowuje kontrakt tylko dla wywolan z kodu Rust i usuwa deny-level diagnostyke Clippy 1.98.

## Rust 1.98 i Clippy po przejsciu na release-lto

FOD 3.3.27 porzadkuje ostrzezenia Clippy pokazane przez walidacje Rust 1.98 w profilu `release-lto`. Proste, mechaniczne ostrzezenia zostaly poprawione w miejscu: `div_ceil`, `checked_div`, `contains`, `is_some_and`, `is_none_or`, `inspect_err`, `append` zamiast `extend(drain(..))`, zbedne casty na celu Linuxowym oraz drobne uproszczenia testow.

Ostrzezenia typu `too_many_arguments` i `large_enum_variant` w goracej sciezce FUSE/PostgreSQL oraz w CLI indexera sa traktowane inaczej. Tam, gdzie funkcja jest swiadomym wewnetrznym kontraktem miedzy warstwami FOD, FOD 3.3.27 dodaje punktowe `#[allow(clippy::...)]` zamiast duzego refaktoru bez regresji runtime. Dotyczy to zwlaszcza sciezek persist/read/copy, startu FUSE, shared monitoringu i enumow utrzymywanych przez Clap.

FOD 3.3.28 ponownie przeglada te punktowe wyjatki przy konkretnych kontraktach. Tam, gdzie poprawa jest lokalna i lepiej opisuje granice API, `allow` zostal zastapiony jawna struktura wejscia: planowanie zakresow read-ahead, planowanie slice read, read-only `setattr`, publish shared monitoringu, start shared monitor publisher, log statusu mounta, persist blokow FUSE, copy range ze stanow write buffer oraz snapshotowe filtrowanie katalogu indexera. Duzy wariant storage lane PostgreSQL w FUSE zostal opakowany w `Box`, wiec rozmiar enumu nie wymaga juz wyjatku.

Po przegladzie FOD 3.3.28 nadal zostawia wyjatki tam, gdzie refaktor oznaczalby szersza zmiane stabilnego kontraktu albo mniej czytelny model domenowy: publiczny enum komend Clap w indexerze, istniejacy kontrakt `read_api::search_files` oraz kilka szerokich operacji PostgreSQL w `DbRepo` zwiazanych z tuningiem, specjalnymi plikami i persist/storage. Te miejsca wymagaja osobnej zmiany API i osobnych testow integracyjnych, a nie kosmetycznego opakowania argumentow.

Ta decyzja nie zmienia zachowania FUSE, SELinux, ACL, storage ani schematu PostgreSQL. Celem jest obnizenie szumu walidacji Rust 1.98, zeby przyszle ostrzezenia latwiej odroznic od swiadomych ksztaltow API.

## Poza zakresem

FOD 3.3.24 nie zmienia:

- formatu danych ani bloku storage 4 KiB;
- schematu PostgreSQL ani migracji;
- zaleznosci w `Cargo.lock` poza numerem wersji crate'ow workspace;
- Edition 2021;
- ustawien FUSE I/O;
- ustawien SELinux lub ACL;
- GitHub Actions;
- cache Cargo ani zasad selektywnego czyszczenia targetow.

## Walidacja po aktualizacji

Po `git pull` zalecany zestaw kontroli:

```bash
rustc --version
cargo --version
make cargo-profile-show
make rust-production-toolchain-check
make test-rust-release-defaults-policy
make rust-msrv-check
make rust-candidate-check
make rust-candidate-clippy
cargo fmt --all -- --check
cargo check --workspace --locked --profile release-lto
cargo test --workspace --locked --profile release-lto --lib --bins
QNAP=0 make test-all
```

Oczekiwane podstawowe wartosci:

```text
rustc 1.98.0
FOD_CARGO_PROFILE=release-lto
FOD_CARGO_TEST_PROFILE=release-lto
FOD_RUNTIME_PROFILE=release-lto
rust-version=1.85
```

Dopiero po przejsciu regresji produkcyjne artefakty powinny byc budowane lub instalowane przez standardowe targety FOD.
