# C6 — audyt powierzchni zapisu i fencing FOD

Stan bazowy: FOD 3.4.43, gałąź `feature/forget-convergence-c1`, po testach C1–C5 (5 PASS, 0 FAIL, 2 ignored).

## Reguła architektoniczna

PostgreSQL jest centralnym arbitrem własności zasobów między klientami i hostami FOD. Każda produkcyjna transakcja **zmieniająca zawartość, mapowanie obiektu danych lub rozmiar pliku** musi być zsynchronizowana z przejęciem własności oraz przejść walidację aktualnego tokenu fencing w **tej samej transakcji**. Samo sprawdzenie ownership przed transakcją jest niewystarczające.

## Potwierdzone zabezpieczenia

- `rust_hotpath/src/pg.rs`: `persist_file_blocks_with_crc_flag_reservation_and_fence` przeprowadza `validate_write_persistence_fence_on_conn` wewnątrz `transactional_replayable`. Walidacja bierze advisory transaction lock `fod:write:file:{file_id}` i sprawdza `session_id`, `owner_key`, token oraz czas ważności sesji i lease.
- `rust_fuse/src/fs.rs`: `attach_write_persistence_fence_for_handle` odrzuca zwykły flush bez własności; `flush_write_state_for_handle` dołącza fence przed zapisem.
- `rust_fuse/src/write_buffer.rs`: `execute_persist_plan` przekazuje `state.persistence_fence` do `persist_file_blocks_profiled`.
- `rust_hotpath/src/pg.rs`: `punch_hole_keep_size_with_fence` waliduje fencing.
- Testy C4/C5 w `rust_hotpath/tests/write_ownership.rs` potwierdzają chroniony zapis bloków i odrzucenie starego tokenu, także w scenariuszu współbieżnego przejęcia.

## Miejsca wymagające zamknięcia

### C6-A — szybka ścieżka copy_file_range (wysoki priorytet)

`rust_fuse/src/fs.rs` w `copy_file_range` oraz `copy_range_from_states` wywołuje `repo.adopt_source_data_object(src_file_id, dst_file_id)`.

`rust_hotpath/src/pg.rs::adopt_source_data_object` zmienia `files.data_object_id`, `files.size` i liczniki referencji wewnątrz transakcji, **ale nie pobiera advisory lock dla file fencing i nie waliduje `WritePersistenceFence`**.

**Ryzyko:** szybkie przyłączenie obiektu może ominąć mechanizm broniący zwykłego `persist`. Brak kontroli jest potwierdzony w kodzie; exploit między mountami nie został jeszcze wykonany.

**Działanie:** dodać wariant `adopt_source_data_object_with_fence`, walidujący `WritePersistenceFence` wewnątrz transakcji modyfikującej docelowy plik; wywołania z obu ścieżek FUSE mają fail-closed wymagać ownership dla `fh_out`. Sprawdzić przypadki `src==dst`, puste źródło, dst dirty, hardlink i równoległy klient.

### C6-B — legacy API utrwalania i FFI (wysoki priorytet)

`rust_hotpath/src/pg.rs::persist_file_blocks`, `persist_file_blocks_with_crc_flag` oraz `persist_file_blocks_with_crc_flag_and_reservation` kierują zapisy do `..._and_fence(..., None)`. Wariant `..._and_fence` sprawdza token **tylko dla `Some(write_fence)`**.

`rust_hotpath/src/ffi.rs::fod_rust_pg_repo_persist_file_blocks` używa `persist_file_blocks_with_crc_flag`, więc nie dostarcza fencing — zarówno z pustą listą bloków, jak i z danymi.

**Działanie:** ustalić wszystkich konsumentów eksportu FFI oraz użycia legacy Rust API, rozdzielić jawny kanał administracyjny/migracyjny od produkcyjnych zapisów i wymagać fencing w każdym publicznym kanale mutacji używanym przez klientów FOD. Nie usuwać API bez audytu kompatybilności.

### C6-C — pozostałe mutacje bez fencing (wysoki priorytet do weryfikacji)

- `rust_hotpath/src/pg.rs::persist_file_blocks_from_path` utrwala zawartość w transakcji bez fencing.
- `rust_hotpath/src/pg.rs::set_file_size` modyfikuje `files.size` bez fencing.
- `rust_hotpath/src/ffi.rs::fod_rust_pg_repo_set_file_size` eksportuje tę metodę.
- `rust_fuse/src/fs.rs::copy_range_from_states`, dla pustego źródła i `truncate_destination`, ma bezpośrednie `flush_write_state` — należy wykazać, że zawsze posiada właściwy fence, również poza typową ścieżką uchwytu.

**Działanie:** określić wszystkich wywołujących, dopuścić wyjątki administracyjne wyłącznie z jawną, testowalną polityką; dołączyć regresję multi-client z aktywną dzierżawą innego właściciela.

## Plan testów i kryteria akceptacji

1. **C6-A1:** A utrzymuje lease na pliku docelowym; B próbuje `copy_file_range` pełnego obiektu z innego mounta. Oczekiwane: odmowa, brak zmian `data_object_id`, `size` i bajtów.
2. **C6-A2:** A ma poprawny fence; pełne przyłączenie danych przechodzi, a nieaktualny fence A zostaje odrzucony po przejęciu B.
3. **C6-B1:** próba zapisu bloków przez niechroniony FFI/legacy API podczas aktywnej obcej dzierżawy nie może zmienić zawartości.
4. **C6-C1:** ścieżki importu, zmiany rozmiaru i skrócenia pliku podlegają analogicznemu testowi.
5. **C6-R:** pełna regresja `write_ownership` C1–C5, `cargo fmt --all --check`, `cargo check --workspace --locked`, testy `copy_file_range`, hardlink, path-based truncate, unlink/open-writer i multimount.

Wykonywać testy destrukcyjne wyłącznie w `foddbname_hotpath_test` lub osobnej izolowanej bazie. Nie modyfikować głównego `foddbname`.

## Status i ograniczenia

To **audyt statyczny**, nie potwierdzenie wykorzystania luki w działającym FUSE. Stan zabezpieczenia C4/C5 nie dowodzi bezpieczeństwa alternatywnych ścieżek.

**C6-A — implementacja do walidacji:** `rust_hotpath/src/pg.rs` otrzymał `adopt_source_data_object_with_fence`, z walidacją w transakcji mutującej; oba wywołania adoption z `rust_fuse/src/fs.rs` wymagają aktualnego fencing pliku docelowego. Ścieżka pustego źródła i truncate uzyskuje fence przed `flush`. Dodano test `c6_adopt_requires_current_destination_fence`; uruchomiony na izolowanej bazie `foddbname_hotpath_test` z wynikiem PASS (`token=51->52`, `fmt=0 check=0 test=0 diff=0`). Publiczny legacy `adopt_source_data_object` nadal istnieje bez wymuszonego fencing i wymaga osobnej polityki oraz przeglądu konsumentów.

**C6-B — implementacja do walidacji:** `persist_file_blocks_with_crc_flag_reservation_and_fence` przy `None` oraz legacy `adopt_source_data_object` walidują brak aktywnej dzierżawy pliku przy tej samej blokadzie advisory co operacje z fencing. Zachowano możliwość użycia starszych API bez aktywnej dzierżawy, istotną dla narzędzi indeksujących i zgodności FFI. Dodano test `c6_legacy_payload_requires_no_active_file_lease`, obejmujący zarówno Rust API, jak i bezpośrednie eksporty FFI (adoption, payload persist i pusty truncate); walidacja C6-B zakończyła się wynikiem PASS (`bytes=32768`, `deny_adopt=1`, `deny_persist=1`, `ffi_deny=3`, `fmt=0 check=0 test=0 diff=0`). **Nie jest to zastępstwo pełnego obowiązkowego fencing** — gdy dzierżawa wygasła, ścieżka bez tokenu może nadal modyfikować plik, dlatego wymaga dalszej polityki administracyjnej.

**C6-C — implementacja do walidacji:** import `persist_file_blocks_from_path` oraz `set_file_size` odmawiają mutacji podczas aktywnej dzierżawy, używając blokady advisory w tej samej transakcji. Dotyczy to również `fod_rust_pg_repo_set_file_size` przez delegowanie do zabezpieczonego Rust API. Test `c6_stream_import_and_size_require_no_active_file_lease` weryfikuje odmowę i niezmienność danych w trakcie lease oraz zgodność legacy po `release`. **C6-C PASS:** test wykonany w izolowanej bazie `foddbname_hotpath_test`, wynik `C6_STREAM_SIZE_FENCE=PASS token=55 bytes=32768 stream_denied=1 size_denied=1 ffi_denied=1`, `C6_C_TEST_RC=0`; kompilacja workspace oraz `cargo fmt --check` i `git diff --check` zakończone powodzeniem. Pełna regresja C1–C6-C została wykonana: `8 passed; 0 failed; 2 ignored`, `FINAL fmt=0 check=0 test=0 diff=0`. Testy FUSE/multimount pozostają do wykonania.

**Dalsze ryzyka i prace:** pozostałe mutacje bez fencing (w tym bezpośrednie SQL poza DbRepo oraz polityka uprawnień PostgreSQL), obsługa wygaśnięcia lease i brak obowiązkowego fencing w administracyjnym legacy API. Niezbędne są regresja C1–C6, testy FUSE/multimount, kompatybilność indexera i audyt uprawnień bazy. Po każdym commicie: przegląd diffu i testy.

## C7 — walidacja FUSE na dwóch niezależnych mountach

Dodano `tests/integration/test_copy_file_range_multimount_ownership.py` i target
`make test-copy-file-range-multimount-ownership`, wyłącznie dla izolowanej bazy
z nazwą zawierającą `test`.

Test uruchamia dwa procesy FOD na tej samej bazie PostgreSQL. A otwiera
plik docelowy do zapisu i utrzymuje lease; B próbuje otworzyć ten sam plik
w trybie zapisu, oczekując szybkiego `EBUSY` oraz niezmienności
`data_object_id`, rozmiaru i zawartości. Po zwolnieniu lease przez A
proces B wykonuje pełnoplikowe `copy_file_range` i sprawdza, czy
`data_object_id` źródła został przyłączony do pliku docelowego,
a zawartość i rozmiar są identyczne.

**C7 PASS (2026-10-13).** Wykonano dwa rzeczywiste mounty z runtime
`release-lto`: `C7_FUSE_COPY_ADOPTION=PASS loser_errno=16
loser_elapsed_ms=3.222 bytes=65536 shared_object=63 mounts=2`;
`C7_MULTIMOUNT_RC=0`. Wynik potwierdza odmowę otwarcia destination podczas
aktywnego lease oraz pełnoobiektowe przyłączenie po jego zwolnieniu.

C7 **nie zastępuje** testu nieaktualnego `copy_file_range` po wymuszonym
przejęciu lease ani audytu uprawnień PostgreSQL.

## C8 — stale copy_file_range po przejęciu lease

Dodano `tests/integration/test_copy_file_range_stale_multimount.py`
i `make test-copy-file-range-stale-multimount`.

W izolowanej bazie PostgreSQL test montuje FOD w dwóch procesach,
tworzy źródło z danymi i pusty plik docelowy, a następnie:

1. A otwiera `destination` do zapisu i otrzymuje fencing token.
2. Test usuwa tylko lease przypisany do sesji A i pliku testowego,
   pozostawiając uchwyt i daemon A aktywne.
3. B otwiera ten sam plik i otrzymuje **wyższy** fencing token.
4. A próbuje pełnoobiektowego `copy_file_range` ze starym uchwytem;
   oczekiwane szybkie `EIO` z warstwy FUSE, bez zmiany
   `data_object_id`, rozmiaru, bajtów ani aktywnego lease B.
5. B z poprawnym tokenem przyłącza obiekt źródłowy i weryfikuje dane.

Wymagana baza testowa: `FOD_PG_DBNAME=foddbname_hotpath_test`
i `POSTGRES_DB=foddbname_hotpath_test`.
**C8 PASS (2026-10-13).** Wykonano test z dwoma mountami: `C8_FUSE_STALE_COPY=PASS token=80->81 stale_errno=5 takeover_ms=20.913 stale_ms=14.672 bytes=65536 mounts=2`, `C8_MULTIMOUNT_RC=0`. Test potwierdza odmowę pełnoobiektowej adopcji przez nieaktualny uchwyt, poprawne przejęcie i integralność danych.

Scenariusz dowodzi ochrony przed *stale handle after takeover*,
ale nie wstrzymuje operacji wewnątrz transakcji adopcji: rzeczywisty
*in-flight takeover race* wymaga osobnego, deterministycznego hooka
oraz kontroli kolejności transakcji. Nie uznawać go za pokryty przez C8.

## C9 — in-flight adoption a współbieżne przejęcie lease

Dodano `c9_inflight_adoption_blocks_takeover_until_commit` w
`rust_hotpath/tests/write_ownership.rs`.

Test w izolowanej bazie PostgreSQL przygotowuje niepuste źródło i pusty
plik docelowy. A ma poprawny token fencing docelowego pliku.
Osobna transakcja blokuje rekord `files` przy użyciu
`FOR NO KEY UPDATE`. W tym czasie rzeczywista operacja
`adopt_source_data_object_with_fence` klienta A waliduje fencing,
uzyskuje advisory transaction lock i zatrzymuje się na modyfikacji
zablokowanego rekordu.

Po potwierdzeniu `pg_stat_activity.wait_event_type = 'Lock'`
test wymusza wygaśnięcie lease A w odizolowanej bazie. B musi
**odmówić przejęcia przed COMMIT A** pomimo wygaśnięcia lease.
Po zwolnieniu blokady rekordowej A zatwierdza przyłączenie
obiektu danych. Dopiero wtedy B zdobywa wyższy token, zmienia dane
i sprawdza odrzucenie ponownej adopcji na starym tokenie A.

**C9: pierwsza walidacja FAILED (test readiness timeout, `C5: backend ... did not reach Lock/*`).**
Przyczyna w kodzie testu: obie sesje zostały rejestrowane przez jeden `DbRepo`, którego
`current_lock_session_id` wskazywał już sesję B podczas próby uzyskania lease A.
Skorygowano na oddzielne `DbRepo` dla A i B oraz dodano asercję zgodności
`file_write_leases.session_id` z sesją `WritePersistenceFence`.
**C9 PASS (2026-10-13):** `C9_INFLIGHT_ADOPTION=PASS fencing=83->84 bytes=32768 early_takeover_denied=1 stale_adoption_denied=1`; `1 passed; 0 failed`. `cargo check --workspace` i `git diff --check` przeszły (`check=0 test=0 diff=0`). Pozostało formatowanie testu (`fmt=1`), które wymaga `cargo fmt --all` i weryfikacji nowego commita. Ostrzeżenie `there is no transaction in progress` pochodzi najprawdopodobniej z końcowego `ROLLBACK` po anulowaniu strażnika testowego.
C9 bada produkcyjną metodę PostgreSQL z kontrolowaną transakcją,
ale nie stanowi jeszcze testu zatrzymanego jądrowego
`copy_file_range` na poziomie dwóch rzeczywistych mountów FUSE.
