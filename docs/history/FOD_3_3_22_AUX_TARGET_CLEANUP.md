# FOD 3.3.22 - selektywne czyszczenie pomocniczego Cargo target

## Cel

FOD 3.3.22 rozszerza polityke z 3.3.21 o selektywne czyszczenie znanych
pomocniczych targetow Cargo bez usuwania swiezego glownego cache kompilacji.

Punktem wyjscia byl rzeczywisty pomiar repozytorium po FOD 3.3.21:

```text
./target              9.5 GiB
target/debug          ok. 8.10 GB
target/test-locking   ok. 1.70 GB
target/release        ok. 0.32 GB
```

Caly target byl swiezy, dlatego pelne `cargo clean` nie mialoby sensu. Osobny
`target/test-locking` jest jednak niezaleznym `CARGO_TARGET_DIR` tworzonym przez
`test-locking` i moze byc oceniany oraz czyszczony osobno.

## Zakres 3.3.22

Pierwsza allowlista zawiera tylko:

```text
target/test-locking
```

Nazwa targetu nie jest przeliczana z dowolnej sciezki operatora. Helper sam
wyprowadza dokladnie `<repo>/target/test-locking` i odrzuca kazda inna nazwe,
w tym `debug` i `release`.

## Domyslna polityka

Pomocniczy target ma `eligible=yes` tylko wtedy, gdy lacznie:

```text
rozmiar >= 1 GiB
brak zmian >= 7 dni
```

Oba progi musza byc spelnione. `FORCE` moze ominac tylko progi, nie allowliste
ani token potwierdzenia.

## Polecenia

Status:

```bash
make target-aux-status
```

Plan bez kasowania:

```bash
make target-aux-clean-plan
```

Plan zawsze uzywa:

```text
cargo clean --manifest-path <repo>/Cargo.toml \
  --target-dir <repo>/target/test-locking --dry-run
```

Wlasciwe czyszczenie:

```bash
make target-aux-clean \
  FOD_TARGET_AUX_CLEAN_CONFIRM=clean-test-locking-target
```

Jawne ominiecie progow:

```bash
make target-aux-clean \
  FOD_TARGET_AUX_CLEAN_FORCE=1 \
  FOD_TARGET_AUX_CLEAN_CONFIRM=clean-test-locking-target
```

## Czego helper nie moze wyczyscic

- `target/debug`;
- `target/release`;
- calego `./target`;
- `~/.cargo/registry`;
- `~/.cargo/git`;
- zadnego innego `CARGO_HOME`;
- `/dev/shm`;
- dowolnej sciezki podanej przez operatora.

Glowny cache `debug`/`release` i pobrane zaleznosci pozostaja dzieki temu
nienaruszone.

## Zabezpieczenia

- allowlista tylko `test-locking`;
- zakaz symlinka dla repozytoryjnego `target` i targetu pomocniczego;
- zakaz `tmpfs` dla tej dyskowej polityki;
- obowiazkowy `cargo clean --dry-run` przed decyzja;
- osobny token `clean-test-locking-target`;
- `FOD_TARGET_AUX_CLEAN_FORCE=1` nie omija allowlisty ani tokenu.

## Test regresyjny

`test-target-aux-clean-policy` sprawdza negatywne przypadki oraz pozytywny
end-to-end clean w izolowanym zagniezdzonym repo. Test umieszcza sentinele w:

- `target/test-locking` - ma zostac usuniety;
- `target/debug` - ma pozostac;
- `target/release` - ma pozostac;
- testowym `CARGO_HOME` - ma pozostac.

Test jest dopiety jako dodatkowy prerequisite `test-all` przez `GNUmakefile`.
`GNUmakefile` wlacza istniejacy `Makefile`, wiec dotychczasowe cele pozostaja
bez zmian.

## Oczekiwane zachowanie na obecnym repo

Przy pomiarze ok. 1.70 GB `test-locking` przekracza prog 1 GiB, ale poniewaz
cache jest swiezy, prog 7 dni nie jest spelniony. Oczekiwany wynik po aktualnym
buildzie to nadal:

```text
eligible=no
```

Nie nalezy uzywac `FORCE` tylko po to, aby odzyskac miejsce ze swiezego cache.

Status: implementacja FOD 3.3.22 gotowa do lokalnej walidacji po pobraniu
commita z GitHub.
