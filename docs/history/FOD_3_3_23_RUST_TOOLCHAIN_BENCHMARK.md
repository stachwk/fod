# FOD 3.3.23 - kontrolowane porownanie toolchainu Rust

## Cel

FOD 3.3.23 dodaje powtarzalny sposob sprawdzenia, czy nowszy kompilator Rust
realnie poprawia wykonywany kod FOD. Ta wersja **nie podnosi MSRV** i nie
zmienia domyslnego toolchainu repozytorium.

Kanoniczne:

```toml
rust-version = "1.85"
```

pozostaje deklaracja minimalnej wspieranej wersji Rust. Samo podniesienie
`rust-version` nie zmienia kodu maszynowego. Kod maszynowy zmienia dopiero
faktyczne uzycie innego `rustc` oraz profilu kompilacji.

Kandydatem porownawczym jest Rust `1.98.0`, oficjalnie wydany 2026-08-20.
Bazowym toolchainem pozostaje minimalny Rust `1.85.0`.

## Macierz

Nowy benchmark buduje i mierzy trzy warianty tego samego commita:

| Wariant | Toolchain | Profil |
| --- | --- | --- |
| `baseline-release` | Rust 1.85.0 | `release` |
| `candidate-release` | Rust 1.98.0 | `release` |
| `candidate-release-lto` | Rust 1.98.0 | `release-lto` |

Pozwala to osobno odpowiedziec na dwa pytania:

1. czy nowszy `rustc` daje roznice przy tym samym profilu `release`;
2. czy dodatkowa roznice daje istniejacy profil FOD `release-lto`.

Nie laczymy tego eksperymentu z Edition 2024, PGO, `target-cpu=native` ani
kolejna aktualizacja zaleznosci.

## Izolacja buildow

Kazdy wariant ma osobny `CARGO_TARGET_DIR` pod:

```text
target/toolchain-benchmark/
```

Domyslnie:

```text
target/toolchain-benchmark/baseline-release/
target/toolchain-benchmark/candidate-release/
target/toolchain-benchmark/candidate-release-lto/
```

Helper:

- nie uruchamia `cargo clean`;
- nie wykonuje `rm -rf` na targetach;
- nie czysci `~/.cargo/registry` ani `~/.cargo/git`;
- nie instaluje toolchainow przez `rustup`;
- zachowuje buildy po pomiarze, aby nie wymuszac ponownej kompilacji przy
  kolejnych uruchomieniach.

Brakujacy toolchain powoduje czytelny blad z poleceniem, ktore operator moze
wykonac jawnie.

## Weryfikacja MSRV

Przed porownaniem mozna osobno sprawdzic minimalny i nowy toolchain:

```bash
make rust-msrv-check
make rust-candidate-check
make rust-candidate-clippy
```

`rust-msrv-check` jest szczegolnie wazny po zmianach `Cargo.lock`: obecna
deklaracja `rust-version = "1.85"` jest kontraktem i odswiezenie zaleznosci nie
moze go po cichu uniewaznic.

## Benchmark wykonywanego kodu

Plan bez budowania i bez uruchamiania FOD:

```bash
make rust-toolchain-benchmark-plan
```

Wlasciwy pomiar:

```bash
make rust-toolchain-benchmark
```

Domyslny workload jest powiazany z aktualnym kierunkiem I/O FOD:

```text
write block size = 512 KiB
count            = 64
source           = pattern
repetitions      = 3
```

`pattern` jest przygotowywany poza mierzonym odcinkiem zapisu, dlatego wynik
nie mierzy generatora losowych danych ani specjalnej sciezki samych zer.

Dla kazdego wariantu budowane sa:

- `fod-bootstrap`;
- `fod-rust-mkfs`;
- `fod-rust-fuse`.

`fod-bootstrap` dostal jawny override `FOD_RUST_FUSE_BIN`, aby benchmark mogl
uruchomic dokladnie binarium z badanego katalogu target. Bez tego bootstrap
moglby znalezc starszy `target/debug/fod-rust-fuse` albo `target/release` i
pomiar nie bylby wiarygodny.

## Ograniczenie biasu kolejnosci

Wszystkie trzy warianty sa najpierw budowane. Dopiero potem uruchamiany jest
runtime benchmark.

Kolejnosc wariantow zmienia sie miedzy powtorzeniami:

```text
1: baseline -> candidate -> candidate-lto
2: candidate-lto -> candidate -> baseline
3: baseline -> candidate -> candidate-lto
```

Nie eliminuje to wszystkich efektow cache systemu i PostgreSQL, ale ogranicza
staly bias wynikajacy z uruchamiania zawsze jednego toolchainu jako pierwszego.

## Raport

Kazdy run zapisuje raport pod:

```text
target/toolchain-benchmark/reports/<UTC timestamp>/
```

Zapisywane sa m.in.:

- commit Git;
- system/kernel i model CPU;
- `rustc -vV`, w tym wersja LLVM;
- profil Cargo;
- czas budowania;
- rozmiary binariow;
- SHA-256 `fod-rust-fuse`;
- wszystkie surowe logi throughput;
- probki MiB/s;
- srednia, minimum i maksimum throughput.

Czas budowania jest informacyjny. Poniewaz katalogi target sa zachowywane,
pierwszy run moze byc zimny, a kolejne cieple. Do decyzji o jakosci
wykonywanego kodu podstawowa metryka jest runtime throughput, nie czas builda.

## Interpretacja

Podniesienie MSRV bedzie uzasadnione dopiero wtedy, gdy istnieje potrzeba
uzycia nowych elementow jezyka/biblioteki albo utrzymanie Rust 1.85 zacznie
blokowac zaleznosci lub bezpieczenstwo projektu.

Sam lepszy wynik kompilatora 1.98 nie wymaga automatycznie podniesienia MSRV.
Mozliwy jest model:

```text
MSRV:       1.85
produkcyjny build: nowszy stable Rust
```

jezeli projekt nadal regularnie weryfikuje `cargo +1.85 check --locked`.

Analogicznie brak poprawy throughput nie oznacza, ze nowszy Rust jest bez
wartosci: nowszy kompilator moze nadal dawac poprawki codegen, diagnostyki,
linty i poprawki bledow kompilatora. Te korzysci trzeba jednak oddzielac od
pomiaru wydajnosci FOD.

## Poza zakresem 3.3.23

Ta zmiana nie:

- zmienia formatu storage;
- zmienia schematu PostgreSQL;
- zmienia rozmiaru bloku FOD;
- wlacza Edition 2024;
- wlacza PGO;
- ustawia `target-cpu=native`;
- podnosi `rust-version` ponad 1.85;
- aktualizuje zaleznosci Cargo;
- zmienia standardowych targetow build/test;
- czysci istniejacych artefaktow Cargo.

Celem 3.3.23 jest dostarczenie wiarygodnych danych do pozniejszej decyzji, a
nie zalozenie z gory, ze Rust 1.98 musi byc szybszy.
