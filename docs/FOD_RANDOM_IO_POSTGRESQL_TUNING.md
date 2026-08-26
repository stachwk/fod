# FOD random I/O i tuning PostgreSQL

## Cel

Ten dokument zapisuje wnioski z pierwszych testow losowego I/O FOD oraz zasade,
ze obserwowane ograniczenie wydajnosci nie moze byc automatycznie przypisywane
samej warstwie FOD/FUSE.

Dla kazdej regresji lub ograniczenia nalezy rozdzielic koszt na co najmniej:

1. FOD/FUSE i liczbe callbackow,
2. zapytania i transakcje PostgreSQL,
3. cache PostgreSQL (`shared_buffers`),
4. page cache i scheduler I/O systemu operacyjnego,
5. fizyczny storage,
6. WAL, checkpointy i autovacuum przy obciazeniu zapisem.

Czesc parametrow PostgreSQL moze byc zmieniana na poziomie sesji i wykorzystana
przez FOD bez restartu serwera. Inne parametry wymagaja reloadu konfiguracji
albo restartu. FOD nie powinien kodowac tej klasyfikacji na stale dla wszystkich
wersji PostgreSQL; stan autorytatywny nalezy odczytywac z `pg_settings`.

## 1. Baseline random I/O FOD 3.3.14

Warunki diagnostyczne:

```text
FOD 3.3.14
plik testowy: 1 GiB
rzeczywisty losowy I/O na przypadek: 64 MiB
noatime
FOD direct I/O
FOD read cache = 0
FOD read-ahead = 0
FOD prefetch = 0
fio iodepth = 1
fio numjobs = 1
```

Powtarzany test wykonal trzy przebiegi dla `randread` i `randrw50`.

### Mediany

| workload | block size | read MiB/s | write MiB/s | read IOPS | uwaga |
|---|---:|---:|---:|---:|---|
| randread | 4 KiB | 7.505 | - | 1921 | bardzo duzy koszt per operation |
| randread | 16 KiB | 28.725 | - | 1838 | podobny IOPS jak 4 KiB |
| randread | 256 KiB | 202.532 | - | 810 | najwyzszy stabilny throughput |
| randread | 1 MiB | 206.452 | - | 206 | praktycznie plateau vs 256 KiB |
| randrw50 | 4 KiB | 1.183 | 1.165 | 303 read | read silnie degraduje przy zapisie |
| randrw50 | 16 KiB | 4.100 | 4.141 | 262 read | jw. |
| randrw50 | 256 KiB | 22.422 | 21.160 | 90 read | jw. |
| randrw50 | 1 MiB | 23.579 | 18.409 | 24 read | jw. |

W pierwszej pelnej macierzy pojedynczy `randwrite` byl wzglednie plaski
przepustowosciowo: od `15.725 MiB/s` dla 4 KiB do `18.524 MiB/s` dla 1 MiB.
Ten wynik wymaga osobnego powtorzenia 3x przed uznaniem go za stabilny baseline.

W `randrw50 256 KiB`, run 2, parser profilu zwrocil
`read_tasks=0 write_tasks=0` mimo poprawnie wykonanego workloadu fio.
Te dwa zera sa traktowane jako brak telemetrii shutdown, a nie jako rzeczywista
liczba operacji. Metryki fio tego przebiegu pozostaja wazne, ale task counters
nie moga byc uzyte do wnioskowania o FUSE/FOD.

### Spadek read przy `randrw50`

W porownaniu do czystego `randread` mediana read throughput w `randrw50`
spadla o:

```text
4 KiB:   -84.2%
16 KiB:  -85.7%
256 KiB: -88.9%
1 MiB:   -88.6%
```

Ta regularnosc wymaga osobnego rozbicia kosztu na:

- flush/persist poprzedzajace read-after-write,
- PostgreSQL transaction/WAL,
- blokady i serializacje,
- prepared statements,
- FUSE callback cost.

Nie nalezy zakladac, ze jest to wyłącznie problem warstwy FOD.

## 2. Bardzo wazny efekt warm/cold

Pierwszy pojedynczy przebieg przed powtorkami dawal:

```text
randread 4 KiB:   0.866 MiB/s
randread 256 KiB: 119.850 MiB/s
randread 1 MiB:   122.605 MiB/s
```

Powtarzane przebiegi daly odpowiednio mediany:

```text
randread 4 KiB:   7.505 MiB/s
randread 256 KiB: 202.532 MiB/s
randread 1 MiB:   206.452 MiB/s
```

Roznica jest zbyt duza, aby traktowac pierwszy pomiar jako stabilny baseline.

Wniosek:

> Stan `shared_buffers`, page cache OS, checkpointow i innych procesow
> PostgreSQL musi byc rejestrowany przy benchmarkach FOD.

Dalsze testy powinny jawnie rozdzielac:

```text
cold-ish PostgreSQL/cache
warm PostgreSQL/cache
```

i nie porownywac tych dwoch stanow jako rownowaznych wynikow.

## 3. PostgreSQL jako czesc silnika storage FOD

Dla FOD PostgreSQL nie jest zwykla zewnetrzna baza aplikacyjna. Jest czescia
sciezki storage.

Dlatego konfiguracja PostgreSQL powinna byc traktowana jako czesc profilu
wydajnosci FOD.

FOD powinien docelowo:

1. wykrywac wersje i mozliwosci PostgreSQL,
2. odczytywac `pg_settings`,
3. rozpoznawac parametry bezpieczne do ustawienia per-session,
4. miec osobne profile read/write/control/lease,
5. nie zmieniac globalnej konfiguracji serwera bez jawnej zgody administratora,
6. raportowac, gdy konfiguracja PostgreSQL jest istotnym ograniczeniem
   wydajnosci.

## 4. Klasyfikacja parametrow PostgreSQL

Zamiast utrzymywac recznie liste "restart/reload/live", FOD/test powinien
sprawdzac autorytatywny kontekst aktualnego serwera:

```sql
SELECT
    name,
    setting,
    unit,
    context,
    source,
    pending_restart
FROM pg_settings
WHERE name IN (
    'shared_buffers',
    'work_mem',
    'maintenance_work_mem',
    'effective_cache_size',
    'random_page_cost',
    'effective_io_concurrency',
    'plan_cache_mode',
    'synchronous_commit',
    'wal_compression',
    'wal_buffers',
    'max_wal_size',
    'checkpoint_timeout',
    'checkpoint_completion_target',
    'wal_writer_delay',
    'wal_writer_flush_after',
    'autovacuum_max_workers',
    'autovacuum_work_mem',
    'track_io_timing',
    'track_wal_io_timing',
    'max_connections'
)
ORDER BY name;
```

Najwazniejsze wartosci `context`:

```text
user / superuser
    parametr moze byc kandydatem do SET na sesji

sighup
    wymaga zmiany konfiguracji i reloadu

postmaster
    wymaga restartu PostgreSQL

backend / superuser-backend
    zwykle ustalany przy starcie backendu/polaczenia
```

Przed kazdym eksperymentem nalezy odczytac faktyczny `context` z docelowej
wersji PostgreSQL.

## 5. Parametry kandydackie - read path

### 5.1 `plan_cache_mode`

FOD intensywnie wykorzystuje prepared statements.

Do sprawdzenia:

```sql
SET plan_cache_mode = 'auto';
SET plan_cache_mode = 'force_generic_plan';
SET plan_cache_mode = 'force_custom_plan';
```

To jest szczegolnie interesujace dla zapytan, ktorych zakres blokow i
selektywnosc zmieniaja sie pomiedzy callbackami.

Nie nalezy zakladac, ze custom plan bedzie szybszy. Nalezy wykonac A/B.

### 5.2 `random_page_cost`

Random workload powinien byc jednym z glownych testow tego parametru.

Kandydaci diagnostyczni:

```text
4.0
2.0
1.5
1.1
```

Zmiana ma sens tylko wtedy, gdy rzeczywiscie zmienia plan zapytania FOD.

Przed i po zmianie nalezy porownac `EXPLAIN`.

### 5.3 `effective_cache_size`

Parametr nie przydziela pamieci. Informuje planner, ile cache jest
prawdopodobnie dostepne.

Nie wolno interpretowac go jako wielkosci rzeczywistego cache.

Nalezy dobrac go do:

```text
shared_buffers
+ uzyteczna czesc page cache OS
- pamiec potrzebna innym procesom/workloadom
```

### 5.4 `effective_io_concurrency`

Moze poprawic wybrane plany wykorzystujace rownolegle/asynchroniczne I/O.

Nie kazde zapytanie FOD z niego skorzysta. Nalezy najpierw sprawdzic plan.

### 5.5 `work_mem`

Kandydat tylko dla zapytan, ktore rzeczywiscie sortuja, hashuja albo buduja
struktury wymagajace `work_mem`.

Podnoszenie `work_mem` bez planu wykorzystujacego dodatkowa pamiec nie jest
optymalizacja.

## 6. Prepared statements - obowiazkowy test planu

Dla krytycznych statementow FOD nalezy zapisac:

```sql
EXPLAIN (ANALYZE, BUFFERS, WAL, SETTINGS, TIMING OFF)
...
```

Dla prepared statements nalezy testowac reprezentatywne zestawy parametrow
i porownac generic/custom plan.

Wynik powinien trafiaz do artifactu benchmarku razem z fio i profilem FOD.

## 7. Parametry kandydackie - write i mixed path

### 7.1 `synchronous_commit`

Jest parametrem istotnym dla latency zapisu, ale zmienia kontrakt durability.

Domysl produkcyjny FOD powinien zachowywac bezpieczna semantyke.

Warianty inne niz bezpieczny default mozna testowac tylko jako jawny benchmark
i nie wolno ich wlaczac automatycznie tylko dlatego, ze zwiekszaja throughput.

### 7.2 `wal_compression`

Moze zmniejszyc objetosc WAL kosztem CPU.

Do testu szczegolnie dla:

```text
randwrite
randrw50
duze overwrite
duzo full-page images po checkpoint
```

Wartosc musi byc dobrana do metod kompresji dostepnych w danej kompilacji PG.

### 7.3 `wal_buffers`

Kandydat przy intensywnym zapisie i wielu commitach.

Jest to parametr innej klasy niz ustawienia per-session; przed eksperymentem
nalezy sprawdzic `pg_settings.context`.

### 7.4 `max_wal_size`, `checkpoint_timeout`, `checkpoint_completion_target`

Zbyt czeste checkpointy moga:

- zwiekszac chwilowe I/O,
- zwiekszac WAL przez full-page writes,
- zaklocac latency losowego read/write,
- powodowac duza zmiennosc miedzy kolejnymi benchmarkami.

Dla FOD nalezy mierzyc checkpointy przed i po workloadzie.

### 7.5 `wal_writer_delay`, `wal_writer_flush_after`

Kandydaci do testow, jezeli profile wykaza, ze WAL flush jest ograniczeniem.

Nie nalezy ich zmieniac przed potwierdzeniem tego w statystykach PostgreSQL.

## 8. `shared_buffers`

`shared_buffers` jest jednym z najwazniejszych kandydatow do wyjasnienia
roznicy miedzy pierwszym i kolejnymi random-read runs.

Test powinien porownac kilka rozmiarow, np. zależnie od RAM hosta:

```text
128 MB
512 MB
1 GB
2 GB
```

Nie jest to parametr do adaptacji per-session.

Zmiana wymaga sprawdzenia `pg_settings.context` i w typowej konfiguracji
restartu serwera.

Wieksze `shared_buffers` powinno byc testowane razem z odpowiednim
`max_wal_size`, aby checkpointy nie staly sie nowym bottleneckiem.

## 9. Autovacuum

Random write i mixed I/O zmieniaja te same relacje wielokrotnie.

Dlugotrwaly benchmark moze wiec zmieniac swoje zachowanie w czasie przez:

```text
dead tuples
vacuum
analyze
freeze
index bloat
```

Nalezy monitorowac:

```sql
SELECT
    schemaname,
    relname,
    n_live_tup,
    n_dead_tup,
    vacuum_count,
    autovacuum_count,
    analyze_count,
    autoanalyze_count
FROM pg_stat_user_tables
WHERE schemaname = 'fod'
ORDER BY relname;
```

Dla tabel FOD mozna w przyszlosci rozwazyc per-table storage parameters dla
autovacuum zamiast zmieniac globalna konfiguracje calego klastra.

## 10. `pg_stat_io` jako obowiazkowe zrodlo od PostgreSQL 16

Dla PostgreSQL 16+ benchmark FOD powinien wykonywac snapshot `pg_stat_io`
przed i po workloadzie.

Przykladowo:

```sql
SELECT
    backend_type,
    object,
    context,
    reads,
    read_time,
    writes,
    write_time,
    writebacks,
    writeback_time,
    extends,
    extend_time,
    fsyncs,
    fsync_time,
    hits,
    evictions
FROM pg_stat_io
ORDER BY backend_type, object, context;
```

`pg_stat_io` pozwala m.in. wykryc:

- nadmierne evictions,
- zapisy wykonywane przez client backend zamiast background writer/checkpointer,
- wzrost fizycznych read/write requests,
- potencjalny niedobor `shared_buffers`.

PostgreSQL nie rozroznia tu zawsze dysku od trafienia w kernel page cache,
dlatego `pg_stat_io` trzeba laczyc z `iostat`, `vmstat` lub odpowiednikiem OS.

## 11. `pg_stat_wal`

Przed i po `randwrite` / `randrw` nalezy zapisac:

```sql
SELECT *
FROM pg_stat_wal;
```

Interesuja nas co najmniej delty:

```text
wal_records
wal_fpi
wal_bytes
wal_buffers_full
wal_write
wal_sync
wal_write_time
wal_sync_time
```

To pozwoli ustalic, czy stale ~18 MiB/s random write wynika z:

- WAL,
- commit/fsync,
- modyfikacji `data_blocks`,
- fizycznego storage,
- czy z FOD przed wyslaniem danych do PostgreSQL.

## 12. `pg_stat_database`

Przed i po benchmarku:

```sql
SELECT
    datname,
    blks_read,
    blks_hit,
    temp_files,
    temp_bytes,
    blk_read_time,
    blk_write_time,
    deadlocks
FROM pg_stat_database
WHERE datname = current_database();
```

Daje to prosty sygnal:

```text
cache-hit vs PostgreSQL read
temporary spills
read/write timing
deadlocks
```

## 13. Kontrolowany protokol A/B PostgreSQL

Kazdy kandydat tuningu powinien byc testowany osobno.

Przyklad:

```text
baseline
parameter=A
baseline
parameter=B
baseline
parameter=C
```

Nie nalezy wykonywac:

```text
zmien 8 parametrow -> fio szybsze -> zaakceptuj wszystko
```

bo nie wiadomo wtedy, ktory parametr pomogl i ktory wprowadzil koszt.

Dla kazdego wariantu trzeba zapisac:

```text
fio JSON
FOD profile
pg_settings snapshot
pg_stat_io delta
pg_stat_wal delta
pg_stat_database delta
pg_stat_user_tables delta
EXPLAIN krytycznych statementow
stan AC/CPU
parametry kernela/FUSE
```

## 14. Dynamiczny tuning przez FOD - kierunek

Po zebraniu A/B mozna rozwazyc jawny profil PostgreSQL po stronie FOD.

Przyklad koncepcyjny:

```text
FOD_PG_TUNING_PROFILE=default
FOD_PG_TUNING_PROFILE=random-read
FOD_PG_TUNING_PROFILE=sequential-read
FOD_PG_TUNING_PROFILE=write-heavy
FOD_PG_TUNING_PROFILE=mixed
```

Profil moglby ustawiac tylko te parametry, dla ktorych aktualny serwer
potwierdza bezpieczny context sesyjny.

Przyklad:

```sql
SET plan_cache_mode = ...;
SET random_page_cost = ...;
SET effective_cache_size = ...;
SET work_mem = ...;
```

Nie oznacza to, ze wszystkie te ustawienia powinny wejsc do FOD. Kazde musi
najpierw wykazac powtarzalna korzysc.

## 15. Osobne profile lane

FOD ma rozne typy polaczen i nie kazde musi miec ten sam profil PostgreSQL.

Docelowo mozna testowac:

```text
read lane:
  ustawienia planera i read I/O

write lane:
  ustawienia istotne dla transakcji/WAL

control lane:
  konserwatywny profil

lease lane:
  niski latency, brak agresywnego tuningu
```

To jest bezpieczniejsze niz zmiana globalnych parametrow calego klastra.

## 16. Czego FOD nie powinien robic automatycznie

Bez jawnej konfiguracji administratora FOD nie powinien:

- restartowac PostgreSQL,
- wykonywac globalnego `ALTER SYSTEM`,
- obnizac durability,
- zmieniac `fsync`,
- zmieniac `full_page_writes`,
- wylaczac autovacuum,
- modyfikowac globalnie checkpoint/WAL tylko na podstawie jednego benchmarku,
- zakladac, ze parametr ma ten sam `context` we wszystkich wersjach PG.

## 17. Priorytet eksperymentow po obecnym baseline

### P1 - ustalic warm/cold

Najpierw mierzyc:

```text
pg_stat_io
pg_stat_database
pg_statio_user_tables
```

dla pierwszego oraz kolejnych `randread`.

Cel:

> wyjasnic skok 4 KiB `0.866 -> ~7.5 MiB/s` oraz
> 256 KiB/1 MiB `~120 -> ~200 MiB/s`.

### P2 - prepared statement / planner

A/B:

```text
plan_cache_mode=auto
force_generic_plan
force_custom_plan
```

oraz sprawdzenie `EXPLAIN`.

### P3 - planner cost/cache assumptions

A/B:

```text
random_page_cost
effective_cache_size
```

tylko jesli zmieniaja rzeczywisty plan.

### P4 - `shared_buffers`

Osobny restartowy A/B.

### P5 - random write / mixed

Rozbic ~18 MiB/s write na:

```text
WAL
commit/fsync
checkpoint
FOD persist
COPY/merge
storage
```

### P6 - autovacuum i dlugi workload

Dopiero po podstawowym I/O wykonac testy dlugotrwale, aby zobaczyc stabilnosc
wydajnosci przy dead tuples i autovacuum.

## 18. Zasada projektowa

Od tego etapu w projekcie FOD obowiazuje wniosek:

> Wynik benchmarku FOD jest wynikiem calego stosu:
> FOD + FUSE + PostgreSQL + kernel + cache + storage.
>
> Przed zmiana kodu FOD nalezy sprawdzic, czy bottleneck nie wynika z
> konfiguracji PostgreSQL. Parametry PostgreSQL, ktore moga byc bezpiecznie
> ustawiane per-session, powinny byc rozwazane jako potencjalna czesc profilu
> runtime FOD, ale dopiero po powtarzalnym A/B i bez naruszania durability.

## 19. Powiazanie z dokumentacja

Ten dokument uzupelnia:

```text
docs/POSTGRESQL_REQUIREMENTS.md
docs/FUSE_REQUIREMENTS.md
BENCHMARKS.md
docs/FOD_CURRENT_ACTION_PLAN.md
```

`POSTGRESQL_REQUIREMENTS.md` pozostaje dokumentem kontraktu i wymagan.

Ten dokument jest roboczym planem pomiarow i tuningu wydajnosci random I/O.

## 20. Diagnostyka write-state przed tuningiem planera

Powtorzony benchmark na poprawnym profilu `noatime + direct_io` zebral
kontrolowane pary po restarcie PostgreSQL i kolejny warm run:

| workload | cold read MiB/s | warm read MiB/s | zmiana |
|---|---:|---:|---:|
| `randread 4k` | 11.617 | 11.887 | +2.3% |
| `randread 256k` | 258.065 | 251.969 | -2.4% |
| `randrw50 4k` | 1.233 | 1.253 | +1.6% |
| `randrw50 256k` | 23.673 | 21.902 | -7.5% |

W tym pomiarze nie ma sygnalu >=15%, ktory uzasadnialby traktowanie
`shared_buffers` jako pierwszego bottlenecku.

Profil FOD pokazal natomiast dominujacy koszt read-after-write:

```text
randrw50 4k:
  write_state_clone_us / fuse_read_total_us = 75.0% / 74.8%

randrw50 256k:
  write_state_clone_us / fuse_read_total_us = 27.9% / 31.9%
```

Dla 4 KiB dwa przebiegi zuzyly ok. `21.9 s` i `21.7 s` tylko na
`write_state_clone`, przy ok. 8 tys. operacji read.

Przyczyna w kodzie jest bezposrednia:

```text
WritePayloadState
  -> BlockWriteState
  -> BTreeMap<u64, Vec<u8>>

write_state_for_handle()
  -> state.clone()
```

`WriteState` jest klonowany do snapshotu przed odczytem. Poniewaz `Clone`
dla `BTreeMap<u64, Vec<u8>>` jest gleboki, kazdy read kopiuje wszystkie
aktualnie dirty bloki oraz ich payload. Przy narastajacym random-write overlay
daje to koszt rosnacy z liczba dirty blocks.

### Decyzja 3.3.16

Pierwszym kandydatem nie jest tuning planera PostgreSQL, lecz zmiana
reprezentacji dirty-block overlay na copy-on-write:

```text
Arc<BTreeMap<u64, Vec<u8>>>
```

Zasady:

- snapshot read ma klonowac tylko `Arc`, nie caly payload;
- lock `write_states` ma pozostac krotki;
- odczyt nie moze trzymac globalnego mutexu podczas SQL;
- writer uzywa `Arc::make_mut()`, aby zachowac izolacje snapshotu;
- przy braku aktywnego snapshotu `Arc::make_mut()` nie kopiuje mapy;
- przy rzeczywistej rownoleglosci read/write kopia jest wykonywana tylko przy
  pierwszej modyfikacji wspoldzielonego snapshotu;
- flush musi zachowac dotychczasowa semantyke i poprawnie obslugiwac aktywny
  snapshot.

Walidacja A/B zostala wykonana na kandydacie 3.3.16
`5849d7caaf67` dla:

```text
randrw50 4k
randrw50 256k
randread 4k
randread 256k
```

Pierwsza seria po kontrolowanym restarcie PostgreSQL (2 przebiegi) pokazala
dla `randrw50 4k`:

```text
read  = 15.509 / 15.123 MiB/s
write = 15.589 / 14.867 MiB/s
write_state_clone_us = 5 / 5
write_state_lock_us  = 41 / 35
```

Dla porownania baseline 3.3.15 mial:

```text
read  = 1.233 / 1.253 MiB/s
write = 1.239 / 1.232 MiB/s
write_state_clone_us ~= 21.9 s / 21.7 s
```

Druga seria stabilizujaca bez restartow PostgreSQL (3 przebiegi) potwierdzila
wynik:

| workload | read MiB/s | write MiB/s | mediana read | mediana write |
|---|---|---|---:|---:|
| `randrw50 4k` | 15.330 / 15.636 / 15.983 | 15.409 / 15.371 / 16.275 | 15.636 | 15.409 |
| `randrw50 256k` | 29.730 / 27.728 / 30.017 | 27.928 / 29.982 / 24.871 | 29.730 | 27.928 |

Wzgledem median baseline 3.3.15 oznacza to dla `randrw50 4k` ok. `12.58x`
read i `12.47x` write. Dla 256 KiB wzrost mediany wynosi ok. `30.5%` read i
`21.6%` write.

`write_state_clone_us` w trzech stabilizujacych przebiegach 4 KiB wynioslo
`7 / 5 / 6 us`, wiec kryterium usuniecia dominujacego deep-clone kosztu jest
spelnione. `write_state_lock_us` pozostalo male (`66 / 26 / 10 us`), zatem
koszt nie zostal przesuniety na globalny mutex.

### Zaklocony czysty randread 4 KiB

W serii bez restartow pierwszy `randread 4k` byl skrajnym outlierem:

```text
0.255 MiB/s
```

W tym samym oknie statystyki PostgreSQL pokazaly aktywny autovacuum/checkpointer
oraz ok. `2.27e9` `tup_returned`. Kolejne dwa przebiegi wyniosly
`9.698` i `9.744 MiB/s`, a w poprzedniej serii po restarcie PostgreSQL
`16.268` i `16.297 MiB/s`.

Ten punkt nie jest uzywany jako dowod regresji ani zysku COW. Pokazuje, ze
czysty random read jest silnie wrazliwy na stan/background maintenance i musi
byc porownywany przy kontrolowanym stanie PostgreSQL i systemu.

### Nastepny bottleneck po COW

Dla `randrw50 256k` liczba `fod_load_block` nadal wynosi:

```text
run 1: 132 read callbacks * 64 blocks = 8448 calls
run 2: 123 read callbacks * 64 blocks = 7872 calls
run 3: 140 read callbacks * 64 blocks = 8960 calls
```

Po usunieciu deep clone dominuje wiec per-block pobieranie brakujacego payloadu
z PostgreSQL. Nastepny etap powinien pobrac brakujacy zakres hurtowo i dopiero
nalozyc dirty-block overlay, zachowujac semantyke read-after-write.

Dopiero po ograniczeniu tego kosztu nalezy wracac do `plan_cache_mode`,
`random_page_cost`, `effective_cache_size` i innych ustawien planera.

### Timing PostgreSQL

W lokalnym PostgreSQL:

```text
track_io_timing = off       context=superuser
track_wal_io_timing = off   context=superuser
```

Dlatego obecne delty licznikow sa uzyteczne, ale czasy I/O/WAL nie sa pelne.
Do pozniejszego A/B PostgreSQL nalezy jawnie wlaczyc te statystyki w
kontrolowanym srodowisku benchmarkowym. Nie jest to wymaganie produkcyjnego
runtime FOD i FOD nie powinien wymagac uprawnien superuser tylko dla tych
metryk.

## 21. FOD 3.3.17: bulk read-after-write range

Po COW z 3.3.16 profil `randrw50 256k` pokazal kolejny bottleneck:

```text
132 read callbacks * 64 blocks = 8448 fod_load_block calls
123 read callbacks * 64 blocks = 7872 fod_load_block calls
140 read callbacks * 64 blocks = 8960 fod_load_block calls
```

Dotychczas `read_from_write_state()` iteruje po kazdym bloku i dla bloku,
ktorego nie ma w dirty state ani recent-write cache, wywoluje
`load_write_block()`. To prowadzi do osobnego prepared statement
`fod_load_block` dla kazdego brakujacego 4 KiB bloku.

FOD ma juz range API:

```text
fetch_block_range_profiled(file_id, first_block, last_block, block_size)
  -> DbRepo::fetch_block_range_shared(...)
  -> fod_fetch_block_range
```

3.3.17 nie dodaje nowego SQL. Zmienia tylko sposob uzycia istniejacego range
query w read-after-write.

Algorytm kandydata po pierwszym A/B:

1. wyznaczyc bloki callbacku read;
2. jezeli callback obejmuje tylko jeden blok, zachowac dokladnie fast path
   3.3.16 przez `load_write_block()` / `fod_load_block`;
3. dla callbacku wieloblokowego zachowac priorytet:
   `dirty WriteState > recent_write_blocks > PostgreSQL > sparse zero`;
4. dla blokow nieobecnych lokalnie wyznaczyc pierwszy i ostatni brakujacy blok;
5. pobrac ten span jednym `fod_fetch_block_range`;
6. zlozyc odpowiedz, nakladajac dirty/recent blocks nad wynikiem DB;
7. jezeli wszystkie bloki sa lokalne, nie wykonywac zapytania PostgreSQL.

Nie wolno trzymac `write_states` mutexu podczas query. Snapshot COW z 3.3.16
pozostaje bez zmian.

Pierwsze A/B kandydata bulk-only dalo mediany:

```text
3.3.16 baseline:
  randrw50 4k   read=15.636 MiB/s write=15.409 MiB/s
  randrw50 256k read=29.730 MiB/s write=27.928 MiB/s

3.3.17 bulk-only:
  randrw50 4k   read=13.358 MiB/s write=13.132 MiB/s
  randrw50 256k read=125.000 MiB/s write=117.424 MiB/s
```

To oznacza ok. `-14.6%/-14.8%` dla 4 KiB oraz ok. `4.2x` wzrost read i write
dla 256 KiB. Regresja 4 KiB wynika z uzycia ciezszego range statement nawet
dla pojedynczego bloku, dlatego kandydat zostal zmieniony na hybrydowy.

Kryteria powtornego A/B przed push:

- `randrw50 256k`: `fod_load_block` ma zniknac z wieloblokowego read path, a
  `fod_fetch_block_range` powinien byc bliski liczbie callbackow read;
- mediana read 256 KiB ma poprawic sie wzgledem 3.3.16 (`29.730 MiB/s`) albo
  przynajmniej wyraznie obnizyc `repo_fetch_block_range_us` bez regresji
  poprawnosci;
- `randrw50 4k` nie moze miec istotnej regresji, mimo ze pojedynczy blok nadal
  wymaga jednego zapytania;
- `write_state_clone_us` ma pozostac blisko zera;
- testy read-after-write, partial write, truncate i flush musza pozostac
  poprawne.

Dopiero po zamknieciu tego per-block kosztu wracamy do A/B
`plan_cache_mode`, `random_page_cost` i `effective_cache_size`.


## FOD 3.3.18: stale planner statistics and redundant data_blocks prefix index

The FOD 3.3.17 512 KiB read-after-write diagnostic exposed a PostgreSQL planner
failure mode after preparing a dense 1 GiB block set without refreshing
statistics. `fod_fetch_block_range` rose from about 1 ms to 32-40 ms and
PostgreSQL fetched about 257k tuples per query to return 128 blocks.

Refreshing statistics after dataset creation restored stable 512 KiB throughput
near 40-41 MiB/s. A controlled INDEX+ANALYZE versus NO-INDEX+ANALYZE comparison
showed no material throughput difference, so stale planner statistics are the
primary cause. The single-column
`idx_data_blocks_data_object_id(data_object_id)` remains redundant with the
left prefix of `idx_data_blocks_object_order(data_object_id, _order)` and is
removed in schema migration 0023 as a planner guard and maintenance
simplification.

Migration 0023 performs one `ANALYZE fod.data_blocks` after dropping the
redundant index. Normal FOD read/write/flush paths must not issue `ANALYZE`.
Synthetic large-data benchmarks should refresh statistics after preparing the
dataset and before collecting performance measurements.


### FOD 3.3.18 final validation

Final 1 GiB / 512 MiB randrw50 validation after post-prepare `ANALYZE`:

- 4 KiB median: read 13.642 MiB/s, write 13.637 MiB/s;
- 256 KiB median: read 42.908 MiB/s, write 44.341 MiB/s;
- 512 KiB median: read 41.390 MiB/s, write 43.431 MiB/s;
- 1 MiB median: read 44.487 MiB/s, write 47.090 MiB/s.

For 512 KiB, `fod_fetch_block_range` remained at 1.269-1.377 ms across all
three runs. The former 32-40 ms warm-plan pathology did not recur.

FOD 3.3.18 therefore closes the data_blocks planner/index issue.
