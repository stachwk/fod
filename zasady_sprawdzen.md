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

Cel: sprawdzić helpery `mkfs`, profile runtime i ścieżki TLS / wersji.

1. Sprawdź zestaw konfiguracji `mkfs`.

```bash
make test-mkfs-config-suite
```

2. Sprawdź runtime config i jego walidację.

```bash
make test-runtime-config
make test-runtime-validation
```

3. Sprawdź nazwane profile runtime.

```bash
make test-runtime-profile
make test-runtime-profile-extents
```

4. Sprawdź ścieżkę TLS dla `mkfs`.

```bash
make test-mkfs-pg-tls
```

5. Sprawdź publikowaną wersję.

```bash
make test-version
```

Oczekiwany wynik: profile runtime, helpery `mkfs` i wersja są spójne z aktualnym drzewem źródłowym, a wariant extents pozostaje opt-in.

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

Oczekiwany wynik: block path i extent path przechodzą, a strace pokazuje oczekiwany kształt syscalli, bez łapania `strace` i `perf` jako monitorowanych programów.

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

1. Uruchom główny zestaw regresyjny.

```bash
make test-all
```

2. Uruchom rozszerzony zestaw regresyjny.

```bash
make test-all-full
```

3. Jeśli chcesz jeszcze szersze pokrycie integracyjne, dodaj:

```bash
make test-integration
```

Oczekiwany wynik: pełny zestaw przechodzi albo jasno pokazuje, który profil trzeba zawęzić do izolacji problemu.

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
