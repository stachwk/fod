# Zasady sprawdzeń

Ten plik opisuje profile sprawdzeń dla FOD. Profil oznacza tu uporządkowaną, krok po kroku listę komend, które trzeba wykonać, żeby dany zestaw testów przeszedł.

## Zasady ogólne

- Najpierw używaj `make`, jeśli dla danego scenariusza istnieje odpowiedni target.
- Jeśli target `make` już istnieje, nie przepisywaj go ręcznie w innym miejscu.
- Jeśli musisz uruchomić skrypt bez `make`, rób to tylko wtedy, gdy jest to prostsze albo wymagane przez sam test.
- Porty, nazwy kontenerów i inne wspólne parametry bierz z `/home/wojtek/git/config`.
- Profile `admpanch_trace` są opcjonalnym pomocnikiem do testów, nie są wymaganym wariantem dla normalnych uruchomień.
- Domyślny lokalny profil trace to `admpanch_trace.fod.local.ini`.
- Profil `admpanch_trace.fod.db.ini` używaj tylko wtedy, gdy chcesz zapisywać trace do PostgreSQL.
- Jeśli polecenie idzie przez `sudo env`, a ma widzieć `ADMP_INI`, przekaż też `ADMP_TRACE_ENV="ADMP_INI=..."`.
- Jeśli helper testowy sam uruchamia `mkfs` albo `mount.fod`, niech też czyta `ADMP_TRACE_ENV`, żeby trace nie urywał się na poziomie wspólnych helperów.
- `strace` i `perf` są traktowane jako narzędzia diagnostyczne, więc nie powinny trafiać do trace jako monitorowane programy.
- Nie zmieniaj `.gitignore` w ramach profili sprawdzeń.
- `test-integration`, `test-all` i `test-all-full` współdzielą lokalną bazę
  Docker/PostgreSQL oraz zasoby FUSE i są celowo wykonywane sekwencyjnie,
  również przy `make -j`.
- Pełny zestaw mkfs celowo modyfikuje lokalny stan schematu. Przed testami
  mounta używaj `make test-rust-mkfs-suite-local-restored` albo po ręcznym
  uruchomieniu mkfs wykonaj `make test-db-restore-local`.
- `cargo test --workspace --locked` nie zastępuje `make test-locking`, ponieważ
  `lock_backend_smoke` wymaga uruchomienia przez `sudo`.

## Profil bazowy bazy i konfiguracji

Cel: sprawdzić, czy baza, schemat i runtime config są spójne.

1. Uruchom bazę i inicjalizację schematu.

```bash
make init
```

2. Sprawdź wymagania PostgreSQL.

```bash
make test-postgresql-requirements
```

3. Sprawdź stan schematu i ścieżkę upgrade.

```bash
make test-schema-status
make test-schema-upgrade
```

Oczekiwany wynik: targety kończą się bez błędów, a `make init` nie tworzy zbędnie nowego stanu, jeśli schemat już istnieje.

## Profil mkfs i runtime

Cel: sprawdzić helpery `mkfs`, profile runtime i ścieżki TLS / wersji bez
pozostawienia lokalnej bazy w stanie używanym przez testy niepełnego schematu.

1. Sprawdź zestaw konfiguracji `mkfs`.

```bash
make test-mkfs-config-suite
```

2. Uruchom pełny lokalny zestaw mkfs z automatycznym odtworzeniem bazy.

```bash
make test-rust-mkfs-suite-local-restored
```

Ten target najpierw wykonuje pełny `test-rust-mkfs-suite`, a następnie zawsze
próbuje uruchomić zabezpieczone `test-db-restore-local`, również wtedy, gdy
testy mkfs zgłoszą błąd.

3. Sprawdź runtime config.

```bash
make test-runtime-config
```

4. Sprawdź nazwane profile runtime.

```bash
make test-runtime-profile
make test-runtime-profile-extents
```

5. Sprawdź ścieżkę TLS dla `mkfs`.

```bash
make test-mkfs-pg-tls
```

6. Sprawdź publikowaną wersję.

```bash
make test-version
```

Samodzielne targety `make test-runtime-validation` i
`make test-rust-hotpath-runtime-size-limits` zachowują surowy pełny zestaw
mkfs. Tak samo działa bezpośrednie:

```bash
cargo test --locked -p fod-rust-mkfs
```

Po każdym z tych trzech wariantów, przed testami FUSE, wykonaj:

```bash
make test-db-restore-local
```

Oczekiwany wynik: profile runtime, helpery `mkfs` i wersja są spójne z
aktualnym drzewem źródłowym, wariant extents pozostaje opt-in, a lokalna baza
jest ponownie gotowa do testów mounta.

## Profil mount i uprawnień

Cel: sprawdzić podstawowe zachowanie mounta, locków i uprawnień.

1. Uruchom smoke mounta.

```bash
make test-mount-suite
```

2. Sprawdź osobny smoke root-permissions dla mounta.

```bash
make test-mount-root-permissions
```

3. Sprawdź locki produkcyjne i backend PostgreSQL.

```bash
make test-locking
make test-pg-lock-manager
```

4. Sprawdź opcje wrappera mounta.

```bash
make test-mount-wrapper-options
```

5. Sprawdź tworzenie inode typu `mknod`.

```bash
make test-mknod
```

6. Sprawdź zachowanie plików z właścicielem `root`.

```bash
make test-root-owned-permissions
```

7. Jeśli host wspiera `allow_other`, możesz dołożyć kontrolę widoczności.

```bash
make test-allow-other-visibility
```

Oczekiwany wynik: testy mounta przechodzą, a przypadki zależne od hosta mogą się pomijać tylko wtedy, gdy tak przewiduje sam test.

## Profil FIO i throughput

Cel: sprawdzić ścieżki odczytu, zapisu i pomiary throughput.

### Zasada pomiarów wydajnościowych primary/replica

- Dla miarodajnego benchmarku I/O zapis wykonuj na osobnej instancji PostgreSQL primary/master.
- Po zapisie odmontuj FOD zapisujący i poczekaj, aż replica/slave odtworzy WAL co najmniej do LSN zapisu.
- Przed pomiarem odczytu zatrzymaj primary, uruchom świeży mount FOD w roli replica i czytaj z instancji replica/slave. Preferowanym scenariuszem jest `make test-fio-primary-write-replica-read-docker`, który dodatkowo restartuje replikę przed odczytem i wyłącza cache/read-ahead FOD.
- Wyniku odczytu wykonanego bezpośrednio po zapisie tego samego pliku na tej samej instancji PostgreSQL nie traktuj jako benchmarku wydajnościowego. Taki przebieg może służyć diagnostycznie, ale cache systemu operacyjnego, PostgreSQL albo FOD może zafałszować wynik.
- Porównując rozmiary I/O, używaj tego samego rozmiaru pliku i tej samej konfiguracji. Każdy wariant 4K/64K/512K wykonuj jako niezależny pełny przebieg primary-write -> WAL replay -> replica-read.

1. Uruchom zwykły sequential smoke.

```bash
make test-fio-sequential-io
```

2. Uruchom wariant ze strace.

```bash
make test-fio-sequential-io-strace
```

3. Uruchom mixed I/O.

```bash
make test-fio-mixed-io
make test-fio-random-mixed-io
```

4. Uruchom throughput smoke.

```bash
make test-throughput
make test-throughput-sync
```

5. Jeśli chcesz dodatkowo sprawdzić hot path, użyj profilu io.

```bash
FOD_PROFILE_IO=1 make test-fio-sequential-io
```

Oczekiwany wynik: block path i extent path przechodzą, a strace pokazuje oczekiwany kształt syscalli, bez łapania `strace` i `perf` jako monitorowanych programów. Wyniki wydajnościowe odczytu uznawaj za porównywalne dopiero po przejściu scenariusza primary-write / replica-read zgodnie z powyższą zasadą.

## Profil `admpanch_trace`

Cel: uruchomić testy z opcjonalnym tracerem `admpanch_trace`, ale tylko dla binarek FOD.

### Lokalny profil trace

1. Użyj lokalnego profilu INI.

```bash
ADMP_INI="$PWD/admpanch_trace.fod.local.ini" make test-admpanch-trace
```

2. Jeśli chcesz konkretny target, nadpisz `ADMP_TRACE_TARGET`.

```bash
ADMP_INI="$PWD/admpanch_trace.fod.local.ini" make test-admpanch-trace ADMP_TRACE_TARGET=test-runtime-profile
ADMP_INI="$PWD/admpanch_trace.fod.local.ini" make test-admpanch-trace ADMP_TRACE_TARGET=test-locking
ADMP_INI="$PWD/admpanch_trace.fod.local.ini" make test-admpanch-trace ADMP_TRACE_TARGET=test-mount-root-permissions
ADMP_INI="$PWD/admpanch_trace.fod.local.ini" make test-admpanch-trace ADMP_TRACE_TARGET=test-mknod
ADMP_INI="$PWD/admpanch_trace.fod.local.ini" make test-admpanch-trace ADMP_TRACE_TARGET=test-root-owned-permissions
```

3. Dla strace-smoke użyj tego samego helpera.

```bash
ADMP_INI="$PWD/admpanch_trace.fod.local.ini" make test-admpanch-trace ADMP_TRACE_TARGET=test-fio-sequential-io-strace
```

### Profil trace z PostgreSQL

1. Jeśli chcesz wysyłać trace do bazy, użyj pliku DB-backed.

```bash
ADMP_INI="$PWD/admpanch_trace.fod.db.ini" make test-admpanch-trace ADMP_TRACE_TARGET=test-runtime-profile
```

2. Profil DB-backed stosuj tylko wtedy, gdy masz uruchomiony i osiągalny backend trace.

Oczekiwany wynik: trace dotyczy tylko binarek FOD, a nie całego otoczenia testów.

## Profil pełny

Cel: uruchomić szeroki zestaw regresji.

1. Dla głównej lokalnej bramki uruchom:

```bash
make test-all 2>&1 | tee /tmp/fod-test-all.log
```

`test-all` zawiera `test-integration`, testy mounta, blokad, journala,
konfliktu rename i puli połączeń. W przebiegu integracyjnym pełny zestaw mkfs
jest wykonywany przez `test-rust-mkfs-suite-local-restored`, dlatego baza jest
odtwarzana przed dalszymi testami FUSE.

2. Dla najszerszego lokalnego zestawu uruchom zamiast tego:

```bash
make test-all-full 2>&1 | tee /tmp/fod-test-all-full.log
```

`test-all-full` zawiera całe `test-all` i dodaje między innymi szersze testy
plików, katalogów, metadanych, symlinków, polityk `atime` oraz indexera. Nie ma
potrzeby uruchamiać wcześniej osobno `make test-all`, chyba że świadomie chcesz
powtórzyć główną bramkę.

3. Nie zastępuj testu blokad surowym `cargo test --workspace`. Jeśli analizujesz
tylko backend blokad, użyj:

```bash
make test-locking
```

Oczekiwany wynik: pełny zestaw przechodzi albo zapisany log jasno pokazuje,
który profil trzeba zawęzić do izolacji problemu. Błąd
`lock backend smoke must be run via sudo` oznacza użycie niewłaściwego
polecenia, a nie regresję backendu blokad.

## Odtworzenie lokalnej bazy po ręcznych testach mkfs

Cel: przywrócić lokalny testowy PostgreSQL po teście, który celowo zostawił
uszkodzony albo niepełny schemat.

```bash
make test-db-restore-local
make test-timestamp-touch-once
```

Pierwszy target działa wyłącznie dla domyślnego lokalnego środowiska
`docker-compose.yml` i odmawia pracy dla QNAP, zdalnego endpointu,
niestandardowych parametrów bazy oraz aktywnego mounta lub demona FOD. Drugi
target potwierdza, że odtworzony schemat zawiera poprawne `config.block_size` i
jest gotowy do uruchomienia FUSE.

## Profil ręczny bez `make`

Cel: uruchomić wybrane skrypty bez warstwy `make`, gdy jest to wygodniejsze.

Założenie: baza jest już uruchomiona i zainicjalizowana przez `make init`, a interpreter Pythona ma dostępne zależności typu `psycopg2`.

1. Dla testu root-owned permissions przekaż tracer env jawnie, jeśli go używasz.

```bash
ADMP_INI="$PWD/admpanch_trace.fod.local.ini" \
ADMP_TRACE_ENV="ADMP_INI=$PWD/admpanch_trace.fod.local.ini" \
bash tests/integration/test_root_owned_permissions.sh
```

2. Dla FIO można uruchomić skrypt bezpośrednio, jeśli wcześniejsze kroki przygotowały bazę i mount.

```bash
bash tests/integration/test_fio_sequential_io.sh
```

3. Dla runtime profile można uruchomić skrypt testowy bezpośrednio, jeśli używasz tego samego środowiska co `make`.

```bash
python3 tests/integration/test_runtime_profile.py
```

Oczekiwany wynik: skrypty przechodzą tak samo jak przez `make`, ale odpowiedzialność za poprawne środowisko spoczywa wtedy na osobie uruchamiającej.

## Jak dodawać nowy profil

1. Najpierw sprawdź, czy istnieje odpowiedni target w `Makefile`.
2. Jeśli target istnieje, opisz go w tym pliku zamiast tworzyć nową ścieżkę ręcznie.
3. Jeśli test używa `sudo env`, upewnij się, że `ADMP_TRACE_ENV` przechodzi do procesu potomnego.
4. Jeśli profil dotyczy Dockerów lub portów, użyj wspólnej konfiguracji z `/home/wojtek/git/config`.
5. Dopisz krótki oczekiwany wynik, żeby profil był użyteczny przy analizie regresji.
