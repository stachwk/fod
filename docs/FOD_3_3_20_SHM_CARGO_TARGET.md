# FOD 3.3.20 - opcjonalny Cargo target w tmpfs

## Cel

FOD 3.3.20 dodaje bezpieczny, opcjonalny tryb lokalnych buildow i testow,
w ktorym artefakty Cargo trafiaja do tmpfs zamiast do repozytoryjnego
`./target`.

Domyslne zachowanie pozostaje bez zmian:

```text
make ...
=> ./target
```

Tryb tmpfs:

```text
make build-debug-shm
QNAP=0 make test-all-shm
```

uzywa domyslnie:

```text
/dev/shm/fod-target-<uid>-<repo-key>
```

## Dlaczego nie wystarczy samo CARGO_TARGET_DIR

Makefile FOD korzysta z gotowych binariow podczas testow FUSE i integracji.
Przed 3.3.20 sciezki do tych binariow zakladaly `target/debug` lub
`target/release`. Samo ustawienie `CARGO_TARGET_DIR` moglo zbudowac artefakty
poprawnie, ale kolejne etapy mogly szukac `fod-bootstrap` albo `fod-rust-mkfs`
w starym katalogu.

3.3.20 wprowadza jeden efektywny target dir dla Makefile oraz uczy helper testow
FUSE respektowania `CARGO_TARGET_DIR`.

## Preflight

Przed wysokopoziomowym buildem/testem w trybie `shm` sprawdzane sa:

- istnienie i zapis do tmpfs root;
- typ filesystemu `tmpfs`;
- polozenie targetu pod wskazanym root;
- minimum 2 GiB wolnego miejsca (konfigurowalne);
- ownership katalogu;
- domyslna sciezka zawiera UID oraz klucz wyliczony z katalogu repo;
- marker FOD jest walidowany przy `check`, `status` i `clean`, wiec drugi
  checkout nie moze przypadkowo przejac tego samego targetu;
- mozliwosc wykonania pliku, aby wykryc mount `noexec`.

Przyklad:

```bash
make FOD_SHM_MIN_FREE_BYTES=4294967296 build-debug-shm
```

Jesli `/dev/shm` ma `noexec` albo jest za male, mozna wskazac inny tmpfs:

```bash
make \
  FOD_SHM_TARGET_ROOT=/mnt/fod-build-tmpfs \
  FOD_SHM_TARGET_DIR=/mnt/fod-build-tmpfs/fod-target-$UID \
  build-debug-shm
```

## Polecenia

```bash
make cargo-target-info
make shm-target-status
make build-debug-shm
QNAP=0 make test-all-shm
QNAP=0 make test-all-full-shm
make shm-target-clean
```

Preferowane sa jawne wrappery `build-debug-shm`, `test-all-shm` i
`test-all-full-shm`, poniewaz wykonuja preflight w osobnym kroku przed
uruchomieniem builda lub testow.

Dla dowolnego innego targetu nalezy najpierw wykonac:

```bash
make FOD_CARGO_TARGET_MODE=shm cargo-target-preflight
make FOD_CARGO_TARGET_MODE=shm <target>
```

Nie nalezy laczyc preflightu z targetem jako zwyklym/order-only prerequisite:
GNU make moze rozpoczac inne prerequisites wczesniej i Cargo zdazy wtedy
utworzyc nieoznaczony katalog target.

## Czyszczenie

`make shm-target-clean` usuwa tylko katalog z markerem nalezacym do aktualnego
repo i UID. Nie usuwa zwyklego `./target`.

Automatyczna polityka wieku/rozmiaru dla dyskowego `./target` pozostaje osobnym
etapem po pomiarach. 3.3.20 nie wykonuje automatycznego `cargo clean`.

## Walidacja

Walidacja funkcjonalna 3.3.20 przeszla dla:

```bash
cargo fmt --all -- --check
cargo check --workspace --locked
make cargo-target-info
make shm-target-status
make build-debug-shm
QNAP=0 make test-all-shm
```

Po poprawce kolejnosci wrappery wykonuja preflight w osobnym `make` przed
wlasciwym buildem/testem. Regresja markera potwierdzila tez odrzucenie targetu
nalezanego do innego checkoutu.

## Wynik A/B target vs tmpfs

Pomiar na commicie `af26b0d`, ten sam `make build-debug`, osobne targety:

| wariant | elapsed | max RSS KiB | fs inputs | fs outputs |
| --- | ---: | ---: | ---: | ---: |
| disk cold | 12.17 s | 490572 | 13160 | 3008000 |
| shm cold | 15.29 s | 495728 | 16160 | 0 |
| disk warm | 1.05 s | 169340 | 4672 | 48504 |
| shm warm | 0.85 s | 166740 | 0 | 0 |

Cold build w tmpfs byl o ok. 25.6% wolniejszy. Warm incremental skrocil czas
z 1.05 s do 0.85 s, czyli o ok. 19.0% (ok. 1.24x).

Rozmiar targetow:

```text
disk: 1386041440 B
shm:  1386036632 B
```

To ok. 1.29 GiB na pojedynczy target. Po pomiarze `/dev/shm` mialo
1045381120 B wolnego miejsca i bylo zajete w 88%.

Artefakt:

```text
artifacts/perf/af26b0d/build-target-ab-20260826T210118Z
```

## Decyzja 3.3.20

`/dev/shm` pozostaje opcjonalnym trybem lokalnego developmentu i testow.
Nie staje sie domyslnym `CARGO_TARGET_DIR`.

Powody:

- cold build nie przyspieszyl;
- warm incremental uzyskal umiarkowany zysk ok. 19%;
- pojedynczy target zuzywa ok. 1.29 GiB tmpfs;
- kilka targetow szybko wyczerpuje `/dev/shm`;
- dyskowy `./target` zachowuje cache po restarcie.

Domyslny preflight 2 GiB wolnego miejsca pozostaje jako konserwatywny limit.
Automatyczna polityka czyszczenia dyskowego `./target` pozostaje osobnym etapem.

Status: zakonczone w FOD 3.3.20.
