# FOD - aktualny plan dzialania

Stan planu: 2026-08-21. Ten dokument jest aktywna kolejnoscia prac. Historyczne
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
dostaje.

- profilowac jeden callback FUSE 512 KiB na fizycznej replice;
- policzyc dokladne metadata/map/payload round-trip PostgreSQL;
- redukowac `read_block_map -> repo_fetch_block_range` w kierunku jednej
  operacji range-oriented, jesli zachowuje to strict read-only i WAL gate;
- mierzyc sukces przez spadek `repo_fetch_block_range_us` i `read_block_map_us`,
  a nie przez sama liczbe callbackow FUSE;
- po kazdej zmianie powtarzac macierz 4 KiB / 64 KiB / 512 KiB primary-write
  -> WAL replay -> replica-read.

## 7. HA miedzy hostami

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
