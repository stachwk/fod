# FOD 3.3.21 - kontrolowane czyszczenie dyskowego target

## Cel

FOD 3.3.21 dodaje bezpieczna polityke dla rosnacego, trwalego
repozytoryjnego `./target`.

Zmiana nie wlacza automatycznego czyszczenia podczas buildow i nie zmienia
dotychczasowego `make clean`, ktory nadal dotyczy legacy `.venv`.

## Zasada bezpieczenstwa

FOD nie usuwa pojedynczych starych plikow z wewnetrznych katalogow Cargo na
podstawie `mtime`. Pelne czyszczenie wykonuje Cargo przez jawne
`cargo clean --target-dir`.

Plan korzysta z `cargo clean --dry-run`, wiec Cargo samo wylicza zestaw
artefaktow bez ich usuwania.

## Domyslna polityka

Target ma `eligible=yes` tylko gdy lacznie:

```text
rozmiar >= 10 GiB
brak zmian w target >= 14 dni
```

## Polecenia

```bash
make target-disk-status
make target-disk-clean-plan
make target-disk-clean FOD_TARGET_CLEAN_CONFIRM=clean-disk-target
```

## Force

```bash
make target-disk-clean \
  FOD_TARGET_CLEAN_FORCE=1 \
  FOD_TARGET_CLEAN_CONFIRM=clean-disk-target
```

`FORCE` omija tylko progi. Nie omija ograniczenia do `<repo>/target`, zakazu
symlinka, zakazu tmpfs, Cargo dry-run ani jawnego tokenu potwierdzenia.

## Czego 3.3.21 nie robi

- nie czysci targetu w tle;
- nie uruchamia clean podczas build/test;
- nie wykonuje `rm -rf ./target`;
- nie czysci `/dev/shm`;
- nie zmienia `make clean`;
- nie czysci `CARGO_HOME`.

## Test regresyjny

`test-target-disk-clean-policy` sprawdza status, Cargo dry-run, odmowe sciezki
poza repo, walidacje progow i obowiazkowe potwierdzenie. Dodatkowo wykonuje
pozytywny end-to-end `cargo clean` w izolowanym, zagniezdzonym repo testowym,
bez czyszczenia wlasciwego `./target` FOD. Test jest wlaczony do `make test-all`.

Status: zakonczone w FOD 3.3.21.
