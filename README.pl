<p align="center">
  <img src="assets/logo.png" alt="FOD logo" width="180">
</p>

# FOD

FOD (Filesystem On DataBaseEngine) to system plikow oparty o PostgreSQL i udostepniany przez FUSE. Runtime jest napisany w Rust, trwaly stan systemu plikow znajduje sie w PostgreSQL, a aplikacje korzystaja ze standardowych operacji filesystemu Linux.

Autorytatywna wersja projektu znajduje sie w [`fod_version.txt`](fod_version.txt).

## Co zapewnia FOD

- semantyke filesystemu Linux/FUSE dla operacji na plikach i katalogach,
- trwale metadane i payload w PostgreSQL,
- blokady i koordynacje sesji przez PostgreSQL dla zapisywalnych mountow,
- obsluge deploymentu primary/replica PostgreSQL,
- buforowanie zapisu, cache odczytu, read-ahead i kontrolowane limity rownoleglosci,
- deployment Docker z jednym zapisywalnym primary PostgreSQL, opcjonalnymi replikami streaming i stalym klientem FOD/FUSE,
- integracje systemd do startu po reboot oraz aktualizacji klienta FOD bez niepotrzebnego restartu PostgreSQL,
- narzedzia Rust do schematu, monitoringu i indeksowania zewnetrznych zrodel.

## Aktualna architektura

| Warstwa | Rola |
| --- | --- |
| `rust_fuse` | frontend FUSE i callbacki filesystemu |
| `rust_runtime` | runtime PostgreSQL, konfiguracja i uslugi wspolne |
| `rust_hotpath` | gorace sciezki storage/read/write |
| `rust_mkfs` | schemat, bootstrap i narzedzia konfiguracyjne |
| `rust_monitor` | diagnostyka runtime i klastra |
| `rust_indexer` | rejestracja zrodel, scan/hash/import |
| PostgreSQL | trwale metadane, payload, locki, sesje i replikacja |

Referencyjny deployment Docker uzywa PostgreSQL 16 z serwerowym block size 32 KiB. Ten rozmiar strony PostgreSQL jest niezalezny od bloku storage FOD i od rozmiaru requestow FUSE.

Aktualne defaulty i lifecycle sa opisane w [`docs/CURRENT_STATE.md`](docs/CURRENT_STATE.md).

## Dokumentacja wedlug zadania

Glownym indeksem jest [`docs/README.md`](docs/README.md). Dokumentacja jest tam pogrupowana wedlug tego, co chcesz zrobic, a nie wedlug kolejnosci historycznych zmian projektu.

| Chce... | Dokument |
| --- | --- |
| poznac aktualna architekture i defaulty | [`docs/CURRENT_STATE.md`](docs/CURRENT_STATE.md) |
| wdrozyc PostgreSQL + FOD w Docker | [`docs/DOCKER_DEPLOYMENT.md`](docs/DOCKER_DEPLOYMENT.md) |
| obslugiwac lub aktualizowac deployment | [`docs/OPERATIONS.md`](docs/OPERATIONS.md) |
| zarzadzac tylko kontenerem FOD/FUSE | [`docs/DOCKER_FOD_INSTALL.md`](docs/DOCKER_FOD_INSTALL.md) |
| skonfigurowac start po reboot przez systemd | [`docs/DOCKER_SYSTEMD.md`](docs/DOCKER_SYSTEMD.md) |
| konfigurowac runtime lub mount | [`docs/runtime-configuration.md`](docs/runtime-configuration.md) |
| sprawdzic wymagania PostgreSQL | [`docs/POSTGRESQL_REQUIREMENTS.md`](docs/POSTGRESQL_REQUIREMENTS.md) |
| sprawdzic wymagania FUSE/kernela | [`docs/FUSE_REQUIREMENTS.md`](docs/FUSE_REQUIREMENTS.md) |
| sprawdzic bezpieczenstwo, uprawnienia i polityke hosta | [`docs/SECURITY.md`](docs/SECURITY.md) |
| profilowac lub optymalizowac wydajnosc | [`docs/performance.md`](docs/performance.md) |
| indeksowac/importowac zewnetrzne zrodla | [`docs/fod-indexer.md`](docs/fod-indexer.md) |
| rozwijac FOD lub zmieniac schemat/kontrakty | [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) |
| przejrzec historie benchmarkow | [`BENCHMARKS.md`](BENCHMARKS.md), [`docs/HISTORY.md`](docs/HISTORY.md), [`docs/history/`](docs/history/) |
| sprawdzic plan prac | [`ROADMAP.md`](ROADMAP.md), [`TODO.md`](TODO.md), [`docs/plans/`](docs/plans/) |
| wykonac procedury testowe | [`zasady_sprawdzen.md`](zasady_sprawdzen.md) |

Pliki `docs/history/FOD_3_*` sa historycznymi zapisami implementacji i pomiarow. Nie nalezy traktowac ich jako glownego zrodla aktualnych defaultow. [`docs/HISTORY.md`](docs/HISTORY.md) grupuje te materialy wedlug tematu.

## Szybki start developerski

```bash
make up
make init
make smoke
make mount
```

W drugim terminalu:

```bash
make unmount
```

Glowna lokalna bramka regresji:

```bash
make test-all
```

Szerszy zestaw testow mount/indexer:

```bash
make test-all-full
```

Repozytorium celowo nie ma aktywnego workflow GitHub Actions. Walidacja jest wykonywana przez lokalne targety Make/Cargo.

## Referencyjny deployment Docker

Obslugiwany uklad:

```text
1 zapisywalny PostgreSQL primary
0..32 replik PostgreSQL streaming
1 staly klient FOD/FUSE
```

`MASTERS>1` jest odrzucane, poniewaz deployment nie implementuje bezpiecznej elekcji multi-primary PostgreSQL.

Instalacja:

```bash
make docker-deploy-plan MASTERS=1 SLAVES=2
make docker-deploy-fod-host-prepare MASTERS=1 SLAVES=2
make docker-deploy-install MASTERS=1 SLAVES=2
make docker-deploy-smoke MASTERS=1 SLAVES=2
```

Domyslnie deployment wybiera tag klienta FOD zgodny z `fod_version.txt`. Aby uruchomic juz opublikowany build niezaleznie od wersji checkoutu, podaj `FOD_CLIENT_VERSION`:

```bash
make docker-deploy-fod-install MASTERS=1 SLAVES=2 FOD_CLIENT_VERSION=3.4.5
```

Pelny `FOD_DOCKER_DEPLOY_CLIENT_IMAGE=registry/path:tag` ma wyzszy priorytet niz `FOD_CLIENT_VERSION`.

Staly start hosta:

```bash
sudo make docker-deploy-systemd-install MASTERS=1 SLAVES=2
```

Ten sam wybor buildu dziala dla systemd, a rozwiazany pelny image jest zapisywany w `/etc/fod/docker-deploy.env`:

```bash
sudo make docker-deploy-systemd-install MASTERS=1 SLAVES=2 FOD_CLIENT_VERSION=3.4.5
```

Jesli usluga systemd jest juz aktywna, reinstall/upgrade uzywa reload i reconcile zamiast pelnego restartu. Aktualizacja klienta FOD nie zatrzymuje ani nie odtwarza zdrowych kontenerow PostgreSQL primary/replica. Jawny target `docker-deploy-systemd-restart` nadal wykonuje pelny restart deploymentu.

Procedura weryfikacji upgrade jest w [`docs/OPERATIONS.md`](docs/OPERATIONS.md).

## Konfiguracja i rozmiary I/O

Glowne pliki:

- [`fod_config.ini`](fod_config.ini) - konfiguracja lokalna/testowa,
- [`fod_config.example.ini`](fod_config.example.ini) - szablon do udostepniania.

Warstwy rozmiarow sa niezalezne:

- blok storage FOD: 4 KiB,
- domyslny maksymalny request zapisu FUSE: 1 MiB,
- domyslny FUSE readahead: 512 KiB,
- bazowy persist chunk: 128 blokow FOD = 512 KiB,
- block size PostgreSQL w produkcyjnym obrazie Docker: 32 KiB.

Zmiana rozmiaru requestu FUSE nie zmienia formatu blokow storage FOD w bazie.

## Glowne programy

- `fod-bootstrap`
- `fod-config`
- `mkfs.fod`
- `mount.fod`
- `fod-monitor`
- `fod-indexer`

## Testy przed commitem

Minimalny zestaw release/policy:

```bash
make test-cargo-lock-integrity
make test-version
make test-docker-deploy-policy
make test-docker-fod-install-policy
make test-docker-deploy-systemd-policy
make test-docker-fod-client-policy
```

Szersze profile sa w [`zasady_sprawdzen.md`](zasady_sprawdzen.md).

Kazdy commit podnosi patch version. Zasada jest opisana w [`docs/versioning.md`](docs/versioning.md).

## Licencja

FOD jest oprogramowaniem source-available na licencji Business Source License 1.1.

- Uzycie niekomercyjne jest dozwolone.
- Uzycie komercyjne wymaga oddzielnej pisemnej umowy.
- Szczegoly: [`LICENSE`](LICENSE) i [`LICENSE-COMMERCIAL`](LICENSE-COMMERCIAL).
