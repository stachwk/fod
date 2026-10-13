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

**C6-C — implementacja do walidacji:** import `persist_file_blocks_from_path` oraz `set_file_size` odmawiają mutacji podczas aktywnej dzierżawy, używając blokady advisory w tej samej transakcji. Dotyczy to również `fod_rust_pg_repo_set_file_size` przez delegowanie do zabezpieczonego Rust API. Test `c6_stream_import_and_size_require_no_active_file_lease` weryfikuje odmowę i niezmienność danych w trakcie lease oraz zgodność legacy po `release`. **Walidacja C6-C oczekuje na uruchomienie w izolowanej bazie.**

**Dalsze ryzyka i prace:** pozostałe mutacje bez fencing (w tym bezpośrednie SQL poza DbRepo oraz polityka uprawnień PostgreSQL), obsługa wygaśnięcia lease i brak obowiązkowego fencing w administracyjnym legacy API. Niezbędne są regresja C1–C6, testy FUSE/multimount, kompatybilność indexera i audyt uprawnień bazy. Po każdym commicie: przegląd diffu i testy.
