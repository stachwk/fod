# FOD - aktualny plan dzialania

Stan planu: 2026-08-23. Ten dokument jest aktywna kolejnoscia prac. Historyczne
decyzje i zakonczone zadania pozostaja w `TODO.md`, a kierunek dlugoterminowy
w `ROADMAP.md`.

## 1. FOD 3.3.2 - spojnosc schematu i migracji

Status: zakonczone w `fd9d781`.

Priorytet: P1.

- podniesc kanoniczny numer schematu z 21 do 22;
- wlaczyc `0022_monitor_session_stats.sql` do manifestu `fod-mkfs`;
- zapewnic realny upgrade 21 -> 22 bez uruchamiania FUSE/runtime DDL;
- wymagac `monitor_session_stats` i `idx_monitor_session_stats_sampled` w
  najnowszym ksztalcie schematu;
- `fod-mkfs status` ma rozrozniac zgodny numer wersji od kompletnego ksztaltu
  i nie moze raportowac `FOD ready: yes` przy brakujacym obiekcie;
- test ma porownywac pliki `migrations/NNNN_*.sql` z manifestem, aby nie dalo
  sie ponownie dodac migracji bez podniesienia `SCHEMA_VERSION`.

Kryterium zakonczenia: test upgrade 21 -> 22, status kompletnego i uszkodzonego
schematu, test manifestu, test wersji i mount smoke przechodza.

## 2. FOD 3.3.3 - lifecycle sesji i identity hosta

Status: zakonczone w `f8b4b2e`.

Priorytet: P1/P2.

- oddzielic maintenance `client_sessions` od heartbeatow lock managera;
- zapewnic okresowy prune wygaslych sesji takze dla `lock_backend=memory`;
- nie przywracac heartbeatow lockow dla backendu memory;
- uzyc wspolnego `current_hostname()` jako fallbacku, gdy `HOSTNAME` nie jest
  ustawione;
- dodac regresje crash/expiry dla writable mounta z backendem memory.

## 3. FOD 3.3.4 - hardening lifecycle/TTL sesji

Status: zakonczone w commicie FOD 3.3.4.

Priorytet: P1.

- dac `client_sessions` jeden autorytatywny heartbeat sesji, niezalezny od TTL
  lockow PostgreSQL;
- rejestrowac poczatkowy `lease_expires_at` sesji tym samym TTL, ktory potem
  odnawia heartbeat sesji;
- usunac odnawianie `client_sessions` z heartbeatow lock managera;
- lock heartbeat ma odnawiac tylko owner state i lease lockow;
- publisher `fod-monitor` ma publikowac tylko `monitor_session_stats`, bez
  skracania lub odnawiania TTL sesji;
- maintenance sesji ma pozostac tylko sprzataniem wygaslych rekordow i
  historycznych lock rows z `session_id=0`;
- przy `FOD_MONITOR_PUBLISH_INTERVAL_MS=60000` TTL sesji ma pozostac 180 s i
  nie moze byc skrocony przez domyslny `lock_lease_ttl=30 s`.

## 4. FOD 3.3.5 - czytelnosc i API fod-monitor

Status: zakonczone w commicie FOD 3.3.5.

Priorytet: P2.

- poprawic format source authority bez maski CIDR;
- uporzadkowac szerokosc i nazwy kolumn `top`;
- dodac stabilny `cluster --json` i `report --json`;
- rozdzielic jednoznacznie dane centralne i lokalne;
- dodac wskazniki efektywnosci, m.in. DB ops na read/write task i sredni
  rozmiar callbacku.

## 5. FOD 3.3.6 - centralna telemetria read-only replica

Status: zakonczone w commicie FOD 3.3.6.

Priorytet: P1 funkcjonalny.

- rozdzielic read endpoint repliki od writable telemetry/control endpointu primary;
- read-only mount ma publikowac identity i statystyki centralnie;
- awaria telemetry primary ma byc fail-soft i nie moze zatrzymywac odczytu repliki;
- zachowac jawne informacje o roli zrodla i mozliwym opoznieniu WAL.

## 6. Read-path performance

Priorytet: po zamknieciu powyzszych bledow poprawnosci/obserwowalnosci.

Status: FOD 3.3.8 ustawia domyslny `direct_io_read_prefetch_blocks=512` po
pomiarze wariantu 512 na slave/replica read. FOD 3.3.7 zakonczyl mechanizm
prefetch i per-handle `FileReadMetadata` cache dla read-only direct-I/O, bez
zmiany formatu storage i bez zakladania, ze kernel przestanie wysylac 4 KiB
callbacki. Nastepny kandydat to koszt samego `repo_fetch_block_range` dla
wiekszych zakresow. Walidacja na commicie `715cf22` potwierdzila domyslny
wariant 512 w fizycznym master-write / slave-read: 4 MiB read `21.9 MiB/s`
oraz 128 MiB read `29.0 MiB/s`, bez bledow operacji i z
`primary_reachable_before_read=0`.

FOD 3.3.9 uzupelnia montowanie opcji atime: `mount.fod -o noatime` i
`mount.fod -o nodiratime` wybieraja teraz odpowiednia polityke FOD zamiast byc
cicho ignorowanymi opcjami passthrough. Dodany smoke `test-atime-nodiratime`
sprawdza, ze katalogi nie dostaja atime update, ale odczyt pliku nadal go
dostaje. Walidacja na `d20c98d` przeszla dla wrappera, `test-atime-noatime`,
`test-atime-nodiratime` oraz krotkich benchmarkow atime dla obu polityk.

Kolejny krok read path zmniejszyl koszt `repo_fetch_block_range` bez zmiany
formatu storage: `load_block` i `fetch_block_range` pobieraja teraz binarne
`BYTEA` zamiast `encode(..., 'base64')`, jednowatkowa sciezka FUSE uzywa
`fetch_block_range_shared` bez kopii `Arc -> Vec -> Arc`, a wielo-blokowy
callback `read()` potrafi odpowiedziec z per-mount read cache, gdy caly zadany
zakres jest juz po prefetchu w cache.

Walidacja 128 MiB primary-write -> WAL replay -> primary stopped ->
replica-read na tej zmianie:

| fio bs | read throughput | `read_block_map_us` | `repo_fetch_block_range_us` | DB `operation_count` |
| --- | ---: | ---: | ---: | ---: |
| 4 KiB | 85.6 MiB/s | 356122 | 282451 | 200 |
| 64 KiB | 234 MiB/s | 347436 | 270469 | 199 |
| 512 KiB | 283 MiB/s | 326535 | 252586 | 199 |

Wazny wniosek z pomiaru: przed dopieciem wielo-blokowego cache hit wariant
128 MiB / 64 KiB wykonywal `operation_count=2152` i mial tylko 62.6 MiB/s.
Problemem nie byl sam rozmiar callbacku, tylko to, ze kazdy wielo-blokowy
callback dociagal mala koncowke zakresu z PostgreSQL mimo istniejacego
prefetchu.

FOD 3.3.9 dopina statement-level SQL profiling dla tego miejsca bez zmiany
storage ani read contractu. Przy `FOD_PROFILE_IO=1` boundary profile wypisuje
teraz agregaty `pg_prepared_statement` i `pg_sql_statement`: liczbe wywolan,
czas laczny/maksymalny/sredni, parametry, rozmiar zwroconego wyniku i bledy.
Skanowanie `PGresult` do policzenia payloadu jest wykonywane tylko, gdy
`FOD_PROFILE_IO` jest wlaczone, a sama flaga jest cache'owana per proces.

Walidacja 2026-08-21 na drzewie kodu zapisanym pozniej jako `cf96eb1`:

| Workload | Read result | FOD callbacks | `read_block_map_us` | `repo_fetch_block_range_us` | `reply_data_us` | DB `operation_count` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 128 MiB, fio bs 4 KiB | 64.4 MiB/s | 32768 | 421051 | 331078 | 220113 | 200 |

Najwazniejszy rozklad SQL z tego profilu:

| Statement | Count | Total us | Rows | Bytes |
| --- | ---: | ---: | ---: | ---: |
| `fod_fetch_block_range` | 65 | 261186 | 32768 | 134479872 |
| `fod_fetch_path_attrs_blob_file` | 15 | 110667 | 15 | 2280 |
| `SELECT COUNT(*) FROM directories WHERE id_parent IS NULL AND name != '/'` | 17 | 10782 | 17 | 17 |
| `SELECT 1 + COUNT(*) FROM hardlinks WHERE id_file = $1` | 15 | 7263 | 15 | 15 |
| `fod_file_read_metadata` | 1 | 444 | 1 | 87 |

Wniosek: pozostale ~200 operacji DB nie wynika z per-callback
`FileReadMetadata`; read-only direct-I/O ma tylko jedno `fod_file_read_metadata`
dla uchwytu. Dominujacy koszt pozostaje w pobraniu payloadu i mapy blokow:
65 wywolan `fod_fetch_block_range` przenosi cale 128 MiB, a metadata/path
lookupi sa drugim, znacznie mniejszym kosztem. `reply_data_us` jest juz
widoczne przy 32768 callbackach 4 KiB i trzeba je brac pod uwage, ale nie
zastepuje jeszcze kosztu DB payload/map path.

Nastepny kandydat po tym pomiarze:

- zoptymalizowac `read_block_map` / `repo_fetch_block_range` jako payload/map
  path, zanim wracamy do hardlink count albo ogolnego strojenia PostgreSQL;
- rozdzielic w kolejnym pomiarze koszt samego transferu `BYTEA` od budowania
  mapy `Vec<(index, payload)>` po stronie procesu;
- dopiero po tym ocenic, czy potrzebna jest osobna sciezka bulk read albo
  ograniczenie kosztu `reply_data_us` przy 4 KiB callbackach.

FOD 3.3.9 w commicie `346aaf8` ogranicza koszt budowania mapy payloadu po
stronie procesu: `fod_fetch_block_range` dekoduje teraz binarne `BYTEA`
bezposrednio do `Arc<[u8]>`, bez przejsciowego `Vec<u8>` i kolejnej konwersji
`Vec -> Arc`. Semantyka sparse blocks i paddingu ostatniego bloku pozostaje
bez zmian.

Walidacja AC 2026-08-21:

| Workload | Read result | `repo_fetch_block_range_us` | `pg_prepared_statement` | `pg_result_decode` | Artifact |
| --- | ---: | ---: | ---: | ---: | --- |
| 128 MiB, fio bs 4 KiB | 85.4 MiB/s | 267069 | 218222 | 42755 | `artifacts/perf/346aaf8/lt7300-docker-primary-write-replica-read-20260821T165506Z` |
| 128 MiB, fio bs 64 KiB | 246 MiB/s | 259271 | 221476 | 32478 | `artifacts/perf/346aaf8/lt7300-docker-primary-write-replica-read-20260821T165617Z` |

Porownanie do AC baseline `5a4e3db`: `pg_result_decode` spadlo z 48.535 ms do
42.755 ms dla 4 KiB i z 41.603 ms do 32.478 ms dla stabilniejszego 64 KiB
rerunu. End-to-end throughput pozostaje w tym samym zakresie i jest nadal
zalezny od `pg_prepared_statement`/transferu payloadu. Nastepny etap powinien
wiec isc w transport/query shape dla 65 wywolan `fod_fetch_block_range`, a nie
w dalsze mikrooptymalizacje dekodera.

FOD 3.3.9 w commicie `b7ffbd7` dodal lokalna specjalizacje dla pojedynczego
bloku: po wyliczeniu wiekszego prefetch window `read_block_map_target_block()`
zapisuje brakujace bloki do cache, ale nie zwraca i nie scala pelnej mapy
blokow, jezeli biezacy FUSE callback potrzebuje tylko jednego bloku. Semantyka
sparse blocks pozostaje bez zmian: brak rekordu po udanym fetchu daje zera,
a blad bazy nadal daje `EIO`.

Walidacja AC 2026-08-21:

| Workload | Read result | `read_block_map_us` | `repo_fetch_block_range_us` | `pg_prepared_statement` | Artifact |
| --- | ---: | ---: | ---: | ---: | --- |
| 128 MiB, fio bs 4 KiB | 89.2 MiB/s | 326232 | 267392 | 219695 | `artifacts/perf/b7ffbd7/lt7300-docker-primary-write-replica-read-20260821T172119Z` |
| 128 MiB, fio bs 4 KiB repeat | 84.7 MiB/s | 336052 | 273630 | 221956 | `artifacts/perf/b7ffbd7/lt7300-docker-primary-write-replica-read-20260821T172248Z` |
| 128 MiB, fio bs 64 KiB repeat | 245 MiB/s | 330846 | 268293 | 225901 | `artifacts/perf/b7ffbd7/lt7300-docker-primary-write-replica-read-20260821T172220Z` |

Wniosek: ta zmiana jest poprawnym cleanupem lokalnego kosztu mapowania, ale
nie jest duza optymalizacja end-to-end. Dla 4 KiB throughput pozostaje w
poprzednim zakresie, a dominujacy koszt nadal pochodzi z 65 transferow
`fod_fetch_block_range` oraz z `reply_data_us` przy 32768 callbackach. Nastepny
etap nadal musi dotyczyc transport/query shape albo bezpiecznej reprezentacji
bulk-read/cache bez uzywania potencjalnie nieaktualnego `data_object_id`.

## 7. FOD 3.3.10 - hardening testow QNAP

Status: zakonczone w commicie FOD 3.3.10.

Priorytet: P1 test infrastructure.

- blokowac `QNAP=1 reset` przed `docker compose down -v`, dopoki operator nie
  poda jawnie `QNAP_ALLOW_DESTRUCTIVE_RESET=1`;
- po gotowosci PostgreSQL wewnatrz kontenera czekac tez na realny SQL endpoint
  widziany z hosta uruchamiajacego testy;
- live `fod-config endpoint-probe`, destrukcyjne testy `schema_upgrade.rs`,
  hotpath `pg_query.rs` oraz `lock_manager.rs` maja uzywac aktywnych
  `FOD_PG_*`, z fallbackiem do starszych `POSTGRES_*`; Makefile dodatkowo
  eksportuje kompletny legacy endpoint `POSTGRES_HOST/PORT/DB/USER/PASSWORD`
  zgodny z `FOD_PG_*`, aby starsze testy i program pod testem zawsze trafialy
  do tej samej bazy z tymi samymi danymi logowania;
- destrukcyjne testy mkfs/schema maja przywracac wybrany backend zamiast
  bezwarunkowo wolac lokalny restore;
- lokalny alias restore ma nadal wymuszac `QNAP=0`;
- test runtime-profile ma odzwierciedlac aktualny startup log cache, w tym
  `direct_io_read_prefetch_blocks=512`, bez zmiany samego runtime;
- test FUSE compatibility ma sprawdzac osobno `FOD FUSE compatibility:`
  i `FOD FUSE negotiated:` oraz relacje requested/effective, bez zalozenia
  ze uzgodnione limity sa `unavailable`;
- pelny destrukcyjny test QNAP pozostaje operacja jawna:
  `QNAP_ALLOW_DESTRUCTIVE_RESET=1 QNAP=1 make test-all`.

## 8. FOD 3.3.11 - QNAP primary/replica performance matrix

Status: implementacja i pierwszy pelny pomiar QNAP wykonane 2026-08-23; baseline zapisany.

Priorytet: P1 performance validation.

- uruchamiac primary i fizyczna replike w izolowanym projekcie Docker na QNAP,
  bez dotykania glownego wolumenu `fod-postgres`;
- publikowac tymczasowe porty primary/replica tylko na wskazanym adresie QNAP;
- jawnie ustawic kompletny `FOD_PG_HOST/PORT/DBNAME/USER/PASSWORD` z danych
  tymczasowego stosu, aby globalne legacy/local env nie zmienialy loginu;
- mierzyc dla kazdego rozmiaru bloku:
  - primary write,
  - swiezy primary read po restarcie PostgreSQL,
  - swiezy replica read po replay WAL i zatrzymaniu primary;
- przed replica read wymagac replay co najmniej do LSN z primary;
- wylaczyc FOD read cache/read-ahead/prefetch i wlaczyc FUSE direct I/O;
- potwierdzic strict read-only: zapis przez replica mount musi zostac odrzucony,
  `operation_failures=0`, brak SQLSTATE 25006;
- domyslna macierz QNAP: `256M`, `4k 16k 64k 256k 512k 1m`;
- zapisac `summary.tsv`, raw fio JSON, logi FOD/PostgreSQL i evidence WAL do
  `artifacts/perf/<commit>/`;
- [x] rzeczywisty pomiar QNAP wykonany dla `4k 16k 64k 256k 512k 1m`,
  WAL replay i strict read-only potwierdzone, wyniki zapisane w `BENCHMARKS.md`;
- [x] powtorzyc `256k`, `512k` i `1m` na `1G` oraz kilku przebiegach:
  po 3.3.12 lokalny fused nie wykazuje istotnego zalamania przy `512k/1m`;
- [x] zidentyfikowac koszt `fod_file_read_metadata` na primary read: byl wykonywany per callback przed `fod_fetch_block_range`;
- [x] pierwszy lokalny A/B 256k wykryl regresje pierwszego fused SQL:
  legacy `215 MiB/s` vs `LEFT JOIN + ORDER BY` fused `78.6 MiB/s`; SQL fused
  kosztowal srednio `2835 us`/callback zamiast ok. `669 us` dla dwoch starych query;
- [x] drugi fused SQL odzyskal throughput: `218 MiB/s` vs legacy `215 MiB/s`,
  ale zysk `+1.4%` jest zbyt maly, a SQL nadal kosztuje `874 us/callback`
  vs ok. `669 us/callback` dla dwoch legacy statements;
- [x] trzeci ksztalt bez `MATERIALIZED CTE`, z prostym `UNION ALL`, zostal
  zwalidowany w 3 parach lokalnego A/B (`QNAP=0`, 256M, fio 256k):
  legacy `164/172/160 MiB/s`, fused `238/264/225 MiB/s`;
- [x] zaakceptowac direct-UNION jako optymalizacje 3.3.12: mediana
  `164 -> 238 MiB/s` (`+45.1%`), mediana `fuse_read_total_us`
  `1,463,130 -> 994,450` (`-32.0%`), mediana SQL `994,324 -> 798,491 us`
  (`-19.7%`);
- [x] koncowy pelny gate `QNAP=0 make test-all` na finalnym 3.3.12 zakonczony bez bledow;
- [x] zdiagnozowac fragmentacje duzych FUSE requestow: przy stronie 4096 i
  `fs.fuse.max_pages_limit=256` domyslne 512KiB dawalo 3072 callbacki/1GiB,
  1MiB dawalo 2048, a `max_pages_limit=512 + max_write=2MiB` dawalo 1024;
- [x] powtarzany A/B 1GiB/fio 1MiB: read `283 -> 322 -> 374 MiB/s`,
  write `60.8 -> 61.3 -> 61.3 MiB/s`; przyjac 1MiB jako bezpieczny domyslny
  limit FOD 3.3.13, a 2MiB/512 stron pozostawic jako jawny tuning hosta;
- [ ] po implementacji 3.3.13 wykonac testy targeted, koncowy
  `QNAP=0 make test-all` i review `git diff HEAD~1..HEAD`;
- [ ] dodac osobny benchmark kontrolowanej promocji replica -> primary i dopiero
  po promocji mierzyc write throughput dawnego slave;
- 3.3.12 jest zamkniete i wypchniete; pozostaje baseline'em dla 3.3.13.
  Biezacy krok to walidacja nowego domyslnego 1MiB FUSE request ceiling i
  diagnostyki limitu kernela, bez automatycznej zmiany globalnego sysctl.

## 9. HA miedzy hostami

Priorytet: wymagany przed deklarowaniem pelnego automatycznego HA.

- dodac zewnetrzny fencing/lease epoch dla cross-process/cross-host;
- zachowac obecny process-local primary generation fence jako warstwe lokalna;
- testowac rzeczywisty promotion/split-brain dopiero z autorytatywnym fencingiem.

## Zasady realizacji

- pracowac tylko na `main`;
- nie dodawac GitHub Actions;
- jedna wersja FOD na jeden logiczny commit kodu;
- po zmianach aktualizowac dokumentacje;
- uzywac lokalnych targetow `make` opisanych w `zasady_sprawdzen.md`;
- po kazdym commicie wykonac `git diff HEAD~1..HEAD` lub `git show` i sprawdzic
  bledy, przypadkowe zmiany, brakujace pliki i regresje wzgledem celu.
