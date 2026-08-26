# Wymagania PostgreSQL dla FOD

## Cel

Ten dokument opisuje parametry PostgreSQL wymagane lub sprawdzane przez FOD,
aby montowanie, zapis, blokady, replikacja i centralna telemetria dzialaly
poprawnie.

Dokument rozroznia:

1. wymagania twarde - ich niespelnienie powoduje odrzucenie startu FOD,
2. wymagania serwerowe dla bezpiecznej pracy i odpowiedniego budzetu polaczen,
3. tuning wydajnosciowy - zalecany, ale nie jest warunkiem poprawnosci.

Kod zrodlowy walidatora znajduje sie w
`rust_runtime/src/postgres_requirements.rs`.

## 1. Minimalna wersja PostgreSQL

FOD wymaga:

```text
PostgreSQL >= 9.5
server_version_num >= 90500
```

Jest to minimalna wersja zakodowana w runtime FOD. W praktyce do nowych
instalacji nalezy uzywac wspieranej wersji PostgreSQL.

Sprawdzenie:

```sql
SHOW server_version;
SHOW server_version_num;
```

## 2. Wymagane ustawienia sesji FOD

Kazde polaczenie FOD ustawia i nastepnie waliduje nastepujace wartosci:

| Parametr | Wymagana wartosc | Uwagi |
|---|---:|---|
| `TimeZone` | `UTC` | FOD ujednolica znaczniki czasu miedzy hostami |
| `transaction_isolation` | `read committed` | wymagany poziom izolacji sesji |
| `statement_timeout` | `0` | zapytania FOD nie moga byc globalnie ucinane timeoutem |
| `lock_timeout` | `0` | blokady FOD maja wlasna semantyke i timeouty |
| `standard_conforming_strings` | `on` | wymagane bezpieczne znaczenie literałów SQL |
| `idle_in_transaction_session_timeout` | `0` | wymagane od PostgreSQL 9.6 |

FOD wykonuje te ustawienia przez `SET SESSION`, dlatego nie trzeba ustawiac ich
globalnie w `postgresql.conf`, o ile uzytkownik FOD moze zmieniac te parametry
na poziomie sesji.

Odpowiada to w przyblizeniu:

```sql
SET TIME ZONE 'UTC';
SET SESSION default_transaction_isolation TO 'read committed';
SET SESSION statement_timeout TO 0;
SET SESSION lock_timeout TO 0;
SET SESSION standard_conforming_strings TO on;
SET SESSION idle_in_transaction_session_timeout TO 0;
```

Ostatnie polecenie dotyczy PostgreSQL 9.6 i nowszych.

Niespelnienie tych wartosci po konfiguracji sesji jest traktowane jako blad
startowy FOD.

## 3. `max_connections`

FOD wymaga odpowiedniego budzetu polaczen:

```text
max_connections >= FOD_pool_max_connections + 2
```

Dodatkowe `2` polaczenia sa rezerwa administracyjna FOD.

Przy typowym limicie puli:

```text
FOD pool = 10
```

minimalna wartosc widziana przez walidator wynosi:

```text
max_connections >= 12
```

Nie oznacza to jednak, ze `12` jest dobra wartoscia dla calego serwera.
`max_connections` musi uwzgledniac lacznie:

- wszystkie jednoczesne instancje FOD,
- pozostale aplikacje korzystajace z tej samej bazy/instancji,
- polaczenia administracyjne,
- narzedzia monitorujace,
- replikacje i procesy utrzymaniowe zalezne od konfiguracji PostgreSQL.

Przy wielu hostach FOD laczacych sie do tej samej instancji nalezy policzyc
budzet dla wszystkich aktywnych pul.

Sprawdzenie:

```sql
SHOW max_connections;
```

Zmiana `max_connections` zwykle wymaga restartu PostgreSQL.

## 4. Trwalosc danych: `fsync` i `full_page_writes`

Dla bezpiecznej pracy FOD powinny byc wlaczone:

```conf
fsync = on
full_page_writes = on
```

FOD wykrywa ich wylaczenie i raportuje ostrzezenie konfiguracji serwera.

### `fsync = on`

Zapewnia, ze PostgreSQL wymusza trwały zapis WAL i danych zgodnie z wlasna
semantyka durability.

Wylaczenie `fsync` moze po awarii systemu lub zasilania doprowadzic do
uszkodzenia klastra PostgreSQL, dlatego nie jest akceptowalnym ustawieniem
produkcyjnym dla storage FOD.

### `full_page_writes = on`

Chroni strony PostgreSQL przed czesciowym zapisem po awarii w czasie
checkpointu/WAL.

Dla systemu plikow, ktorego stan logiczny znajduje sie w PostgreSQL, ta ochrona
jest szczegolnie istotna.

Sprawdzenie:

```sql
SHOW fsync;
SHOW full_page_writes;
```

## 5. Primary i replica

### Writable FOD

Mount zapisujacy musi miec dostep do writable primary PostgreSQL.

Serwer primary powinien zwracac:

```sql
SELECT pg_is_in_recovery();
```

wynik:

```text
false
```

oraz:

```sql
SHOW transaction_read_only;
```

wynik:

```text
off
```

FOD posiada walidacje roli endpointow i nie powinien kierowac operacji zapisu
na fizyczna replike read-only.

### Replica

Fizyczna replika moze sluzyc jako zrodlo odczytu, jezeli routing replica jest
wlaczony i FOD potwierdzi, ze endpoint jest rzeczywista replika read-only.

Dla repliki typowy stan to:

```text
pg_is_in_recovery() = true
transaction_read_only = on
```

FOD dodatkowo kontroluje spojnosc i replay WAL przed wykorzystaniem repliki w
scenariuszach, w ktorych wymagany jest aktualny stan danych.

### Telemetria FOD 3.3.1

Centralna telemetria `fod-monitor` zapisuje snapshoty do primary PostgreSQL.
Mount dzialajacy wylacznie na fizycznej read-only replice nie moze sam zapisac
rekordu telemetrii do tej repliki.

## 6. Ustawienia, ktorych nie nalezy narzucac globalnie

FOD zarzadza transakcjami i timeoutami na poziomie swoich polaczen. Nie nalezy
zmieniac globalnych parametrow tylko dlatego, ze FOD wymaga okreslonej wartosci
w swojej sesji.

W szczegolnosci nie jest konieczne ustawianie globalnie:

```conf
timezone = 'UTC'
default_transaction_isolation = 'read committed'
statement_timeout = 0
lock_timeout = 0
idle_in_transaction_session_timeout = 0
```

jezeli FOD moze skutecznie wykonac odpowiadajace im `SET SESSION`.

Pozwala to korzystac z tej samej instancji PostgreSQL innym aplikacjom o
odmiennych wymaganiach sesyjnych.

## 7. Parametry wydajnosciowe - nie sa wymaganiami poprawnosci

Ponizsze parametry maja duzy wplyw na wydajnosc, ale ich konkretna wartosc
zalezy od pamieci, typu dyskow, liczby hostow FOD i charakteru obciazenia:

```text
shared_buffers
effective_cache_size
work_mem
maintenance_work_mem
wal_compression
max_wal_size
checkpoint_timeout
checkpoint_completion_target
random_page_cost
autovacuum_max_workers
autovacuum_work_mem
```

Nie nalezy wpisywac jednej stalej wartosci jako "wymaganej przez FOD".
Powinny byc dobierane na podstawie pomiarow PostgreSQL i benchmarkow FOD.

Dla random I/O obowiazuje dodatkowa metodyka z
`docs/FOD_RANDOM_IO_POSTGRESQL_TUNING.md`. Przed zmiana kodu FOD nalezy
sprawdzic, czy ograniczenie nie pochodzi z planera, `shared_buffers`,
WAL/checkpointow, autovacuum albo page cache OS. Klase zmiany parametru
(sesja/reload/restart) nalezy odczytywac z `pg_settings.context` na
docelowej wersji PostgreSQL, zamiast utrzymywac stale zalozenie w FOD.

## 8. Autovacuum i ochrona przed wraparound

FOD intensywnie aktualizuje tabele PostgreSQL, dlatego autovacuum musi
pozostac aktywny.

Nie jest to obecnie pojedynczy parametr twardo walidowany podczas startu FOD,
ale wylaczenie autovacuum bez rownowaznego procesu utrzymaniowego jest
niebezpieczne dla dlugotrwalej pracy bazy.

Nalezy monitorowac co najmniej:

- wiek `relfrozenxid`,
- `datfrozenxid`,
- opoznione vacuum,
- martwe tuple,
- wykorzystanie i tempo generowania WAL.

## 9. Przykladowe minimum `postgresql.conf`

Ponizszy fragment pokazuje tylko parametry serwerowe istotne dla kontraktu FOD.
`max_connections` trzeba policzyc dla konkretnej instalacji.

```conf
# Trwalosc
fsync = on
full_page_writes = on

# Przyklad - wartosc musi pokrywac wszystkie pule FOD i inne aplikacje.
max_connections = 100
```

Ustawienia sesyjne FOD nie musza znajdowac sie w tym pliku.

## 10. Szybka kontrola SQL

```sql
SELECT version();

SHOW server_version_num;
SHOW max_connections;
SHOW fsync;
SHOW full_page_writes;

SHOW TimeZone;
SHOW transaction_isolation;
SHOW statement_timeout;
SHOW lock_timeout;
SHOW standard_conforming_strings;
SHOW idle_in_transaction_session_timeout;

SELECT
    pg_is_in_recovery() AS in_recovery,
    current_setting('transaction_read_only') AS transaction_read_only;
```

Uwaga: wartosci `TimeZone`, timeoutow i izolacji odczytane przez zwykle `psql`
moga roznic sie od sesji FOD. Wartosc autorytatywna dla FOD jest sprawdzana po
wykonaniu przez FOD jego `SET SESSION`.

## 11. Testy w repozytorium FOD

Po zmianach PostgreSQL nalezy uruchomic:

```bash
cd ~/git/fod

make test-postgresql-requirements
make test-postgresql-requirements-autocommit-off
make test-postgresql-requirements-autocommit-on
```

oraz dla pelnej kontroli montowania i lockow:

```bash
make test-mount-suite
make test-locking
```

W logu poprawnej walidacji FOD powinny pojawic sie wartosci podobne do:

```text
server_version_num=...
minimum_server_version_num=90500
pool_max_connections=...
max_connections=...
required_max_connections=...
session_time_zone=UTC
session_transaction_isolation=read committed
session_timeouts=disabled
standard_conforming_strings=on
```

## 12. Podsumowanie

Najwazniejszy kontrakt PostgreSQL dla FOD:

```text
PostgreSQL >= 9.5

sesja FOD:
  TimeZone = UTC
  transaction_isolation = read committed
  statement_timeout = 0
  lock_timeout = 0
  standard_conforming_strings = on
  idle_in_transaction_session_timeout = 0   # PostgreSQL >= 9.6

serwer:
  max_connections >= laczny wymagany budzet polaczen
  fsync = on
  full_page_writes = on

writable FOD:
  writable primary PostgreSQL
```

Parametry wydajnosciowe nalezy stroic osobno na podstawie realnego obciazenia,
a nie traktowac ich jako stale wymagania protokolu FOD. Metodyka random I/O,
w tym `pg_stat_io`, `pg_stat_wal`, `pg_settings.context` i kontrolowane A/B,
jest opisana w `docs/FOD_RANDOM_IO_POSTGRESQL_TUNING.md`.
