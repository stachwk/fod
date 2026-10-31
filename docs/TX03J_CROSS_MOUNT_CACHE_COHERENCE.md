# TX03J — spójność odczytów między montowaniami FOD

Document role: topic review/specification supporting the
[current implementation queue](plans/CURRENT.md). Implemented capabilities are
summarized in [CURRENT_STATE.md](CURRENT_STATE.md); see the
[documentation map](README.md#correctness-reviews-and-evidence) for related reviews.

## Status i diagnoza (aktualizacja 2026-10-31)

TX03I.3 potwierdził poprawne przejęcie write ownership (stary token 3205,
nowy 3206), odrzucenie nieaktualnych renew/release i zgodność danych w
montowaniu B oraz w nowym montowaniu C. Wcześniej używane montowanie A
odczytało poprzednie dane (`stale_read_on_a=1`). To osobny problem spójności
odczytów, a nie dowód niepoprawnego fencing.

Późniejsza macierz TX03J.1 potwierdziła stare dane w A w sześciu
konfiguracjach, także po dwóch sekundach; B i świeży mount C widziały nowe
dane. Macierz ustala objaw i odrzuca samo opóźnienie jako wyjaśnienie, ale nie
izoluje jeszcze warstwy źródłowej. Przegląd kodu wykazał, że ustawienie
`FOD_READ_CACHE_BLOCKS=0` było podnoszone do jednego bloku, a przy direct I/O
limit mógł być dodatkowo zwiększony przez read-ahead. Dlatego wcześniejsze
komórki „bez read cache” nie wyłączały cache bloków Rust i nie rozstrzygają,
czy po jego wyłączeniu pozostaje problem cache jądra lub innej warstwy.

Stan implementacji:

- PostgreSQL przechowuje `files.content_generation`; zmiany zawartości są
  wersjonowane w transakcji zmieniającej plik.
- Odczyt ze snapshotem generacji istnieje i jest używany w jawnym trybie
  degraded read-only. Nie jest jeszcze włączony w zwykłej ścieżce odczytu z
  cache.
- Lokalny cache bloków odrzuca teraz fill rozpoczęty przed lokalnym
  unieważnieniem; wymaga to jeszcze deterministycznej walidacji runtime.
- `FOD_READ_CACHE_BLOCKS=0` wyłącza teraz cache bloków Rust także w trybie
  direct I/O i omija wcześniej zapełnione wpisy także po przeładowaniu
  konfiguracji; poprzednia implementacja zachowywała pojemność co najmniej
  jednego bloku, a pojedynczy odczyt mógł nadal trafić do już zapełnionego
  cache. Macierz TX03J.1 trzeba powtórzyć dla poprawnie wyłączonego cache.
- Zdalny trwały dziennik zdarzeń, zwykły runtime listener FUSE i selektywne
  unieważnianie cache jądra nie są zaimplementowane. `fsync()` nie czeka dziś
  na potwierdzenia innych mountów.

Kod potwierdza tę granicę: standardowy start mounta używa `fuser_mount()`, a
obsługa `Notifier` jest obecnie tworzona tylko w specjalnej ścieżce
integration-test hook. Callback `fsync()` opróżnia lokalny stan zapisu i
odpowiada; nie wykonuje między-mount bariery.

## Wymagany kontrakt

PostgreSQL pozostaje autorytatywnym źródłem zawartości, generacji i kolejności
operacji. Po udanym `fsync()` na montowaniu B odczyt z innego aktywnego
montowania rozpoczęty po powrocie `fsync()` ma widzieć zatwierdzone dane B,
również gdy kernel cache jest włączony. Dotyczy to montowań na tym samym i na
różnych hostach. Zachowanie już trwających odczytów, otwartych mapowań i
niezatwierdzonych lokalnych zapisów wymaga jawnego określenia w implementacji
i testach; nie wolno odrzucać buforów zapisu w reakcji na zdalne zdarzenie.

## Kolejne bramy implementacji

1. **J2a — pełna lokalizacja źródła nieaktualnych danych.** Połączyć
   obserwację `file_id`, `data_object_id`, generacji i bloków z cache procesu,
   metadanymi FUSE i cache jądra. Nie zmieniać zachowania produkcyjnego na
   podstawie samej korelacji z `direct_io` lub TTL.
2. **J2b — ścieżka odczytu.** Zintegrować generację z normalnym odczytem;
   dla ścisłego kontraktu wybór i walidacja danych muszą opierać się na
   primary. Chronić także in-flight fills przed publikacją starszej generacji.
3. **J2c — trwałe zdarzenia i invalidacja.** Dodać PostgreSQL outbox,
   ustalony zestaw aktywnych mountów, selektywne invalidacje Rust/FUSE oraz
   replay po reconnect. Zanim kod uzna ACK, zdefiniować dokładnie, co
   potwierdza odpowiedź `Notifier` i jak bariera uwzględnia in-flight reads.
   `LISTEN/NOTIFY` może być wyłącznie wskazówką do pobrania trwałych zdarzeń.
4. **J3 — regresje i wydajność.** Macierz dwóch mountów: nowe `open`, ponowny
   odczyt starego fd, `fsync()`, różne rozmiary pliku, równoległe zapisy,
   utrata zdarzenia i reconnect. Nie zamykać obszaru na podstawie samej
   zgodności mountów B/C.
5. **J4 — wiele hostów.** Powtórzyć macierz na oddzielnych hostach, mierzyć
   opóźnienia ACK, obciążenie PostgreSQL, odzyskiwanie sesji i działanie przy
   utracie łączności.

### Granica sukcesu zapisu i invalidacji

Samo `LISTEN/NOTIFY` ani walidacja generacji w callbacku FUSE nie zapewnia
silnego kontraktu: kernel może obsłużyć odczyt z page cache bez wywołania
daemona, a asynchroniczne powiadomienie może dotrzeć do innego mountu po
powrocie `fsync()`. Zatwierdzona decyzja projektu wymaga więc bariery
potwierdzeń invalidacji:

1. `fsync()` odczytuje z primary zatwierdzoną generację pliku i granicę zdarzeń
   invalidacji objętą barierą.
2. Każdy aktywny mount zarejestrowany przed tą granicą musi odebrać trwałe
   zdarzenie dla `file_id`, wyczyścić tylko lokalne cache tego pliku, wysłać
   `FUSE_NOTIFY_INVAL_INODE` dla znanego inode i zapisać ACK dopiero po
   potwierdzonym zakończeniu invalidacji w kernelu oraz obsłużeniu starszych
   operacji odczytu.
3. `fsync()` może zwrócić sukces dopiero, gdy ACK obejmą wszystkie mounty
   uczestniczące w barierze. Nieosiągalny mount lub brak ACK oznacza brak
   potwierdzenia spójności i nie może zostać uznany za sukces `fsync()`.
   Dane mogą być już trwale zapisane w PostgreSQL mimo błędu odpowiedzi.
4. Mount uruchomiony po granicy nie ma starego page cache z poprzedniej sesji;
   mount odzyskujący połączenie musi odtworzyć trwałe zdarzenia przed
   udostępnieniem odczytów.

Zdarzenia i ACK musza byc autorytatywne w PostgreSQL. `NOTIFY` moze jedynie
przyspieszac ich dostarczenie. Invalidacja pozostaje per-file; globalne
czyszczenie cache jest zabronione. Ta bariera jest wymogiem do zaprojektowania
i wdrozenia, nie obecna gwarancja FOD.

## Kryterium zamkniecia

Nie zamykamy TX03J po samym poprawnym fencing. Odczyty z A rozpoczęte po
udanym `fsync()` w B muszą zwracać zatwierdzone dane B zgodnie z kontraktem,
a wymagane regresje muszą przechodzić. Wynik diagnostyczny z nieaktualnym A
pozostaje FAIL/OPEN nawet gdy testy ownership są PASS.

## Decyzja architektoniczna (zatwierdzona)

Wszystkie klienty FOD — rowniez na roznych hostach — musza obserwowac
autorytatywny stan PostgreSQL, niezaleznie od `direct_io=0/1`. Nie
wystarczy poprawny fencing zapisu. Aktualnosc tresci i metadanych jest
osobnym kontraktem.

Docelowa implementacja:

1. Dwie niezalezne monotoniczne generacje: `content_generation` dla
   zmian danych i `namespace_generation` (oraz wersjonowanie metadanych)
   dla zmian wpisow. `file_id` moze pozostac bez zmiany podczas zapisu.
2. Zmiana generacji w tej samej transakcji PostgreSQL co modyfikacja.
   Kazdy odczyt sprawdza wersje zgodnie z zadeklarowanym kontraktem
   synchronizacji i nie uzywa danych z niewlasciwej generacji.
3. Powiadomienia `LISTEN/NOTIFY` do szybkiego lokalnego uniewazniania,
   ale bez polegania wylacznie na dostarczeniu powiadomienia.
4. Inwalidacja cache Rust (w tym mapowania blokow, ostatnich zapisow
   i metadanych) oraz Linux/FUSE (data, attr, entry). Jest obowiazkowa
   rowniez przy `direct_io=0`.
5. Gdy walidacja generacji nie dziala, scisly tryb nie moze podawac
   nieweryfikowanego cache jako aktualnych danych.
6. Osobne testy: nowy `open`, ponowny `read` ze starego uchwytu,
   wspolbieznosc zapisu, przerwanie `LISTEN`, awaria i reconnect,
   unlink/recreate, rename i hardlink; testy wydajnosci po zmianach.

### Potwierdzone wyniki TX03J.1

Sześć konfiguracji (domyślna, direct I/O, wyłączony read cache, kombinacja,
TTL metadanych zero oraz wszystkie przełączniki jednocześnie) odczytało stare
dane w A, nawet po 2 sekundach; B i świeży C odczytały dane zatwierdzone.
Jest to FAIL spójności odczytu. Komórki z `FOD_READ_CACHE_BLOCKS=0` nie
wyłączały jednak cache Rust, więc ich wynik nie lokalizuje źródła problemu.
Macierz musi zostać powtórzona po poprawce. `TX03J_DIAGNOSTIC=PASS` oznacza
jedynie wykonanie pomiaru.

### Bramy wdrozenia

- **J2a**: instrumentacja metadanych `file_id/data_object_id` i mapy
  blokow, ustalenie zrodla stale A bez maskowania bledu.
- **J2b**: atomowe generacje PostgreSQL + walidacja przed odczytem
  i inwalidacja cache procesu oraz jadra FUSE.
- **J2c**: kanal powiadomien i uzgadnianie generacji po reconnect;
  powiadomienia nie stanowia zrodla prawdy.
- **J3/J4**: przejscie testow wielomontazowych, wielohostowych,
  crash recovery oraz testow wydajnosci. Stare A musi widziec nowe
  dane zgodnie z kontraktem; brak PASS dla samego poprawnego B/C.

Nie wlaczamy globalnie direct I/O jako obejscia.

## TX03J.2c — targeted invalidation and race-free publication (2026-10-16)

**Hard requirement:** a notification for one `file_id` MUST NOT flush
unrelated files or the entire cache. Notifications identify the affected
entity, not the entire mount. A single global cache flush is forbidden.

### Identity and sequence

Content event: `{file_id, content_generation, data_object_id, event_id}`.
Entry/namespace event: `{parent_directory_id, name, namespace_generation,
event_id}` (and old/new parents for rename). Use inode identity **and**
generation; a path alone is not safe for unlink/recreate. Hardlink aliases
share the same content identity. Never reuse a stale event to invalidate a
newer generation as though it were current.

The authoritative change and generation increment occur in the same
PostgreSQL transaction; publish a durable event/outbox entry atomically with
that change. `NOTIFY` is only a wake-up hint: it may be coalesced or lost.
A reconnecting listener replays the durable event stream or checks
authoritative generations, without falling back to a global cache flush.

### Concurrent writer/invalidation/cache-fill protocol

For each `file_id`, maintain observed_generation plus a monotonic local
invalidation epoch. Use file-scoped synchronization; holding a global lock
across PostgreSQL I/O or FUSE notification is prohibited.

1. A cache fill captures (file_id, observed_generation, local_epoch).
2. The reader loads data from an authoritative PostgreSQL snapshot and
   revalidates the generation before publishing data into local cache.
3. An event with `generation > observed_generation` advances the recorded
   version/epoch and selectively drops **only** keys belonging to its
   `file_id` from read blocks, recent-write snapshot entries (without
   discarding unsynced local writer buffers), read-metadata snapshots,
   read-ahead and file-scoped lookup state. Equal/older events are harmless.
4. In-flight fills may finish, but a fill carrying an outdated
   generation/epoch MUST NOT publish its result into the cache. It must
   retry against the current version or report a justified error.
5. Local writer buffers are not invalidated by a remote event: coordinate
   their state using PostgreSQL write ownership/fencing and let conflicting
   operations fail safely. Never delete pending local writes as a side
   effect of a reader's cache invalidation.
6. Invalidate the affected kernel inode's page/attribute cache and the
   precise directory entry for namespace mutations. For a hardlink's
   content event, invalidate the shared inode once, not the entire parent
   directory. Do not hold cache locks while invoking FUSE notifications.
7. Update the local `observed_generation` before enqueueing kernel
   invalidation and serialize/coalesce notifications per inode. A newer
   version must not be overwritten by delayed completion of an older event.
8. A successful, strictly coherent read following a completed remote
   commit requires ordered validation; async NOTIFY alone is insufficient.
   Choose and document the commit/read synchronization boundary before
   enabling strict semantics with kernel caching.

Example race:

```text
A reads generation 41 and starts DB cache fill
B commits generation 42 and publishes event(file_id=7, gen=42)
A advances epoch for file 7 and invalidates only file 7
A's fill for gen 41 completes: discard/retry, never republish gen 41
Unrelated file 8 remains cached and unaffected
```

### Mandatory adversarial regression gates

- Two unrelated files A and B: remote write to A must not evict B.
- 100+ overlapping writes, notifications and cache fills on one file:
  no stale resurrection, no lost update, no deadlock.
- Out-of-order/duplicate events (gen 43, then 42, then 43): no rollback.
- Hardlink aliases, rename, unlink/recreate and recycled path names.
- Held readers, pending local writers, disconnect/reconnect and missed NOTIFY.
- Both `direct_io=0` and `direct_io=1` with the strict-read override off.
- Instrument counters: targeted invalidations, skipped-old-events,
  rejected-stale-cache-fills, reconnect reconciliations, kernel invalidation
  errors; no metric named `global_flush` should increase on file writes.

**Implementation gate:** no default-on change before race tests and FUSE
kernel invalidation are proven. Current strict-read path is an opt-in
diagnostic/compatibility path, not the final protocol.
