# TX03J — spojnosc odczytow miedzy montowaniami FOD

## Punkt wyjscia (2026-10-16)

TX03I.3 potwierdzil poprawne przejecie write ownership (stary token 3205,
nowy 3206), odrzucenie stale renew/release i zgodnosc danych w montowaniu B
oraz w nowym montowaniu C. Wczesniej uzyte montowanie A odczytalo poprzednie
dane (`stale_read_on_a=1`). To jest osobny problem spojnosci odczytow,
nie dowod niepoprawnego fencing.

## Wymagany kontrakt

PostgreSQL pozostaje autorytatywnym stanem zawartosci i ownership.
Po zakonczonym `fsync` na montowaniu B nowe otwarcie pliku w A ma
odczytac zatwierdzone dane zgodnie z przyjetym modelem spojnosci.
Stare uchwyty i semantyka niezatwierdzonych zapisow sa testowane oddzielnie.
FOD nie powinien obiecywac natychmiastowej koherencji dla juz
zbuforowanych stron jadra bez mechanizmu ich uniewazniania.

## Etapy

1. **TX03J.1 — diagnoza**: powtorzyc A -> B na dwoch montowaniach,
   rozdzielic natychmiastowy i opozniony odczyt A oraz swiezy mount C.
   Porownac zwykle FUSE, tryb `FOD_FOPEN_DIRECT_IO=1` oraz odczyt
   przy wylaczonym wewnetrznym read cache (`FOD_READ_CACHE_BLOCKS=0`).
   Nie zmieniac zachowania produkcyjnego przed uzyskaniem wynikow.
2. **TX03J.2 — implementacja**: zidentyfikowac warstwe odpowiedzialna
   (kernel page cache vs read block cache vs metadata cache) i przygotowac
   bezpieczny tryb koherencji z wersjonowaniem w PostgreSQL lub
   odpowiednia walidacja/inwalidacja przy otwarciu/odczycie.
   Rozwazyc kompromis wydajnosciowy. Nie stosowac globalnego
   `direct_io` bez decyzji o zgodnosci i kosztach.
3. **TX03J.3 — testy**: macierz dwoch mountow, mount C, ponowne open,
   dotychczasowy fd, fsync i rozne rozmiary pliku; potwierdzic brak
   regresji TX03G / TX03I.1 / TX03I.2 / TX03I.3 oraz pomiar wydajnosci.
4. **TX03J.4 — multi-host**: powtorzyc po obu stronach oddzielnych hostow,
   sprawdzic TTL, recovery po rozlaczeniu oraz obciazenie PostgreSQL.

## Kryterium zamkniecia

Nie zamykamy TX03J po samym poprawnym fencing. Nowe odczyty z A musza
zwracac dane B zgodnie z jawnie udokumentowanym kontraktem i testy
regresyjne musza przechodzic. Wynik diagnostyczny z niespojnym A
pozostaje FAIL/OPEN nawet gdy testy ownership sa PASS.

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

Szesc konfiguracji (domyslna, direct I/O, brak read cache, kombinacja,
TTL metadanych zero oraz wszystkie przelaczniki jednoczesnie)
odczytalo stare dane w A, nawet po 2 sekundach. B i swiezy C
odczytaly zatwierdzone dane. Jest to FAIL spojnosc odczytu;
`TX03J_DIAGNOSTIC=PASS` oznacza jedynie wykonanie pomiaru.

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
