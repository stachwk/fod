> **Status:** transaction review snapshot dated 2026-10-13/14. Test-pending statements and source locations are historical evidence; they are not a current validation report. Use the current implementation queue for active work and rerun the relevant checks before relying on closure status.

# C10-B2 — ręczny przegląd transakcji o najwyższym ryzyku

Data przeglądu: 2026-10-13; aktualizacja TX-02B: 2026-10-14. Zakres: pierwsza transza z 1045 kandydatów skanera (149 plików).
Stan: **TX-02A odtworzony; TX-02B potwierdzony lokalnie: formatowanie PASS, kompilacja PASS, 2/2 testów sesji i rollback PASS, 33/33 testów indexera PASS. Nowa regresja braku replay oczekuje na test lokalny.**
Nie zmieniono danych PostgreSQL, schematu, kont ani uprawnień. Zmiany TX-02B obejmują implementację API transakcyjnego i cleanup indexera; do przeprowadzenia pozostają lokalne testy kompilacji i izolacji.

## Interpretacja wyników automatycznego skanera

Potwierdzone lokalnie: 5/5 testów, `compile=0 test=0 scan=0 diff=0`.
149 plików, 1045 wskazań (700 SQL, 345 punktów wykonania).
Spośród komponentów: `rust_hotpath` 614, `rust_indexer` 69,
`rust_mkfs` 57, `migrations` 253. Pozostałe komponenty i skrypty: 52.
Wynik skanera nie jest liczbą luk. Bez analizy wywołań nie można
wnioskować z samego wystąpienia `validate_write_persistence_fence_on_conn`
o poprawnym uwierzytelnieniu lub granicy transakcyjnej.

## Przegląd źródeł — priorytet P0/P1

| ID | Miejsce (ścieżka:linie) | Zweryfikowany mechanizm | Otwarte ryzyko / test |
|---|---|---|---|
| TX-01 | `rust_indexer/src/cleanup.rs:173-305` | `cleanup_failed_materialization` weryfikuje stan planu przed `repo.exec("BEGIN")`, w transakcji zbiera identyfikatory, wykonuje `DELETE files`, `DELETE data_objects`, `DELETE directories`, `UPDATE data_objects.reference_count` i `UPDATE index_import_plans`; kończy `COMMIT` albo `ROLLBACK`. | Brak jawnego fencing/lease/advisory na usuwanych plikach w tej funkcji. Stan planu odczytywany przed `BEGIN`. Wymagany negatywny test współbieżności aktywnego mounta, pisarza, współdzielonego obiektu i importu. |
| TX-02 | `rust_hotpath/src/pg.rs:7886-7904,5704-5808,5000-5082` | `DbRepo::exec` pobiera połączenie przez `with_cached_connection`; `with_connection` oddaje je do wspólnej puli po zakończeniu **każdego wywołania**. `query_rows_text` także pobiera połączenie (linie 7819-7822). | **Hipoteza wysokiego ryzyka:** sekwencja `BEGIN` / wiele metod repo / `COMMIT` z TX-01 nie ma jawnego przypięcia do jednego backendu PostgreSQL. Przy konkurencyjnych operacjach możliwe użycie różnych sesji albo wypożyczenie sesji z otwartą transakcją. Nie twierdzimy, że błąd wystąpił. Potrzebny deterministyczny test dwóch połączeń i `pg_backend_pid()` z kontrolowaną blokadą. |
| OWN-01 | `rust_hotpath/src/pg.rs:9596-9873` | `acquire_write_ownership` wymaga zarejestrowanej sesji, porządkuje klucze advisory, w jednej transakcji sprawdza ważność sesji i konflikty, po czym aktualizuje `destination_write_leases` i `file_write_leases`. | Kontrola jest skuteczna, o ile mutacje przechodzą przez tę metodę; obecne konto `foduser` może wykonać bezpośrednie DML na lease (C10-A). W docelowym modelu nie przyznawać takich praw kontu runtime. |
| OWN-02 | `rust_hotpath/src/pg.rs:13557-13611` | `validate_write_persistence_fence_on_conn` bierze file-level `pg_try_advisory_xact_lock`, sprawdza token, sesję, owner key i wygaśnięcie obu lease przy czasie serwera. | Chroni wywołania używające API oraz tej samej transakcji; samo sprawdzenie tokenu przekazanego przez klienta nie jest kontrolą SQL uprawnień. |
| LEG-01 | `rust_hotpath/src/pg.rs:13613-13653,13255-13297` | Import `persist_file_blocks_from_path` bierze ten sam klucz advisory co pisarze i odmawia przy aktywnym lease. | Bez tokenu dozwolony jest zapis, kiedy nie ma aktywnego lease. Docelowo wymaga odrębnego uprawnienia importowego, by nie tworzyć obejścia granicy autoryzacji. |
| LEG-02 | `rust_hotpath/src/pg.rs:13533-13555,13656-13723` | Wrapper `persist_file_blocks_with_crc_flag_and_reservation` przekazuje `None` jako fencing; docelowa funkcja sprawdza aktywny lease przed zapisem. | Przejrzeć wszystkie wywołania wrappera w FFI i indexerze; zapisy bez lease to osobna, jawna polityka, nie równoważnik posiadania praw do pliku. |
| ADOPT-01 | `rust_hotpath/src/pg.rs:13952-14135` | Fenced i legacy adopcja korzystają z tej samej ścieżki; z tokenem sprawdzany jest fence, bez tokenu — odmowa przy aktywnym lease; modyfikowane są `files` i `data_objects`. | Tokenless adopcja pozostaje dostępna poza aktywnym lease; ograniczyć przyszłym osobnym uprawnieniem importowym. C9 potwierdza współbieżność **fenced** adopcji, ale nie autoryzację SQL. |
| LEG-03 | `rust_hotpath/src/pg.rs:14139-14187` | `set_file_size` odmawia zapisu bez tokenu przy aktywnym lease. | Ta metoda nadal pozwala na mutację bez fence przy braku lease. Wymaga przeglądu wywołań FFI oraz polityki uprawnienia do zmiany rozmiaru. |
| DDL-01 | `rust_hotpath/src/pg.rs:8543-8612` | `ensure_client_session_schema` może wywołać `CREATE TABLE`, `CREATE INDEX`, `ALTER TABLE` przy starcie. | Ograniczony runtime nie może samodzielnie prowadzić DDL; bootstrap i migration muszą dostarczać kompletnego schematu, runtime ma wyłącznie sprawdzać gotowość. |
| GRANT-01 | `rust_mkfs/src/main.rs:611-619,786-791,876-881` | `ensure_fod_privileges` nadaje `SELECT, INSERT, UPDATE, DELETE` do wszystkich tabel FOD na `init` i `upgrade`, oraz prawa do sekwencji. | Dowolna migracja do konta restrykcyjnego będzie cofnięta przez obecny `GRANT`. Ten punkt jest blokadą wdrożenia C10-B3. |

### TX-02A — izolowana próba współdzielenia transakcji (do uruchomienia)

Dodano test `rust_hotpath/tests/tx02_pool_transaction_affinity.rs`.
Cel: w **izolowanej bazie testowej** otworzyć transakcję przez
`DbRepo::exec("BEGIN")`, uzyskać blokadę `pg_try_advisory_xact_lock`,
sprawdzić, czy odrębne połączenie widzi zajętą blokadę, oraz czy
inny wątek współdzielący repo otrzymuje tę samą sesję `pg_backend_pid()`.
Po wykonaniu próby wywoływany jest `ROLLBACK`, po czym sprawdzane
jest zwolnienie blokady transakcyjnej. Test **nie zapisuje danych FOD**.

```bash
cd ~/git/fod
FOD_PG_DBNAME=foddbname_hotpath_test \
POSTGRES_DB=foddbname_hotpath_test \
cargo test --locked -p fod-rust-hotpath \
  --test tx02_pool_transaction_affinity -- --nocapture
```

Jeżeli test wypisze `TX02_POOL_TRANSACTION_SHARING=OBSERVED
same_backend=1 xact_lock_held=1 xact_lock_released=1`,
potwierdzi niebezpieczne ponowne użycie **otwartej** transakcji przy
rozbiciu na niezależne wywołania `DbRepo`. Jest to test diagnostyczny,
nie test PASS bezpieczeństwa; samo zaliczenie odtwarza antywzorzec.

Próba TX-02A nie dowodzi jeszcze, że rzeczywisty cleanup utracił
atomowość lub uszkodził dane. Następny warunek: dedykowane API
przypiętej transakcji i test pozytywny izolacji wraz z rollback
i konkurencyjnym writerem.

### TX-02B — jednosesyjne API i migracja cleanup (walidacja oczekuje)

Zmiany w gałęzi `feature/forget-convergence-c1`:

- `DbRepo::with_transaction` otwiera, wykonuje i zamyka transakcję na
  **tym samym** wypożyczonym `PGconn`. W callbacku używa się
  `DbRepoTransaction::exec/query_rows_text/query_scalar_text`.
- `rust_indexer/src/cleanup.rs::cleanup_failed_materialization` przekazuje
  wszystkie odczyty i mutacje do `DbRepoTransaction`, w tym odczyt stanu
  planu. Usunięto ręczne `repo.exec("BEGIN"/"COMMIT"/"ROLLBACK")`.
- Przypadek niejednoznacznego wyniku `COMMIT` nie jest automatycznie
  ponawiany: arbitralny callback cleanup nie ma jeszcze zatwierdzonego
  wzorca idempotentnego potwierdzenia; taki błąd należy rozstrzygnąć
  przez odczyt trwałego stanu planu przed ewentualną powtórką.
- `rust_hotpath/tests/tx02_pinned_transaction.rs` sprawdza stały backend
  transakcji, oddzielną sesję równoległego użytkownika puli, lock
  transakcyjny i zwolnienie przy sukcesie/błędzie. Testy nie zapisują
  tabel FOD.

Walidacja:

```bash
cd ~/git/fod
cargo fmt --all
cargo fmt --all --check
cargo check --workspace --locked
FOD_PG_DBNAME=foddbname_hotpath_test POSTGRES_DB=foddbname_hotpath_test \
  cargo test --locked -p fod-rust-hotpath --test tx02_pinned_transaction -- --nocapture
FOD_PG_DBNAME=foddbname_hotpath_test POSTGRES_DB=foddbname_hotpath_test \
  cargo test --locked -p fod-rust-hotpath --test tx02_pool_transaction_affinity -- --nocapture
cargo test --locked -p fod-rust-indexer
git diff --check
git diff --stat
```

**Zakres poprawki:** usuwa zagrożenie dziedziczenia sesji PostgreSQL
przez losowego użytkownika puli w **tym** cleanup. Nie zapewnia jeszcze
poprawnych blokad / fencing dla usuwanych plików, powiązania
planu importu z aktualnym właścicielem ani ochrony przed modyfikacją
tabel przez superużytkownika `foduser` (C10-A).

Test TX-02A nadal jest testem **charakteryzującym wcześniejszy
antywzorzec** i pozostaje testem pozytywnej reprodukcji, nie
niezaliczoną regresją nowego API. Test rzeczywistej, konkurencyjnej
operacji cleanup z błędem po częściowych DML pozostaje do wykonania.

### TX-02B — wyniki lokalne i bezpieczeństwo ponawiania

Użytkownik uruchomił kod na bazie `foddbname_hotpath_test`:
`TX02_PINNED_COMMIT=PASS` (oddzielny backend drugiego wątku, blokada
zwolniona), `TX02_PINNED_ROLLBACK=PASS` (blokada zwolniona, `SET LOCAL`
cofnięte), 2/2 testów TX-02B oraz 33/33 testów jednostkowych indexera.
Podsumowanie: `TX02B fmt=0 check=0 pinned=0 indexer=0 diff=0`.

Po `cargo fmt --all` lokalnie pozostały wyłącznie zmiany stylu w
`rust_hotpath/tests/tx02_pinned_transaction.rs`,
`rust_hotpath/tests/tx02_pool_transaction_affinity.rs` i
`rust_indexer/src/cleanup.rs`. Te zmiany należy przejrzeć i
zapisać jako osobny commit przed kolejnym etapem; na tym etapie nie
oznaczono ich jako committed.

Dodatkowy przegląd ujawnił możliwość **automatycznego powtórzenia całego
callbacku po rozłączeniu w jego środku**. `with_transaction` usuwa
teraz wewnętrzny znacznik replay z błędu przed wyjściem do wspólnej
puli; dzięki temu callback nie jest uruchamiany drugi raz na nowym
połączeniu. Nie zmienia to zachowania innych istniejących transakcji
FOD. Dodano diagnostyczny test
`rust_hotpath/tests/tx02_pinned_no_replay.rs`: sztuczny znacznik
błędu, dokładnie jedna próba oraz działający odczyt po błędzie.
Test ten **nie symuluje fizycznego zerwania sieci** i nie zastępuje
testu utraty ACK COMMIT ani rollback po DML.

```bash
FOD_PG_DBNAME=foddbname_hotpath_test POSTGRES_DB=foddbname_hotpath_test \
  cargo test --locked -p fod-rust-hotpath --test tx02_pinned_no_replay -- --nocapture
```

Zagadnienia otwarte: wyścigi cleanup/aktywny mount, spójność
`data_objects.reference_count`, fencing i sesje właścicieli, retry
z trwałym receipt oraz docelowe odebranie DML kontu runtime.

## TX-01/TX-02 — wymagany test atomowości przed poprawianiem cleanup

1. Użyć **izolowanej bazy testowej**, nigdy `foddbname` ani produkcyjnego
   importu. Przygotować import nieukończony, pliki i wspólny `data_object`.
2. Uruchomić `cleanup_failed_materialization` na repo z pulą o minimum
   dwóch połączeniach. Zatrzymać wykonanie między `BEGIN` a pierwszą mutacją.
3. Na drugim wątku pobrać połączenie z tej samej puli. Sprawdzić rzeczywiste
   `pg_backend_pid()` i `pg_stat_activity.state` oraz czy transakcja cleanup
   pozostaje na **tej samej** sesji przez wszystkie zapytania.
4. Wstrzyknąć błąd po jednej mutacji; **wszystkie** zmiany w `files`,
   `data_objects`, `directories` i `index_import_plans` muszą wrócić
   do stanu początkowego. Sprawdzić również, że żaden inny wątek nie
   przejął transakcji cleanup ani otwartej sesji z puli.
5. Równolegle utrzymać aktywny zapis i drugi mount FUSE. Cleanup musi
   odpowiednio odmówić lub poczekać; nie wolno usunąć obiektu
   potrzebnego aktywnemu uchwytowi ani zepsuć `reference_count`.
6. Dopiero po potwierdzeniu zachowania wdrożyć jednosesyjne API transakcyjne
   dla indexera z tą samą hierarchią blokad i PostgreSQL-fencing.
   Nie naprawiać tego przez sam `REVOKE` albo sprawdzenie stanu planu bez locka.

## TX-03A — atomowość cleanup przy blokadzie planu importu

Stan: **skrypt przygotowany; test na bazie lokalnej jeszcze niewykonany**.

Nowy `tests/integration/test_fod_indexer_cleanup_tx03_atomicity.py`:

1. Wymaga jawnie `FOD_PG_DBNAME`/ `POSTGRES_DB`, nazwy kończącej się
   `_test`, i odmawia działania na domyślnym `foddbname`.
2. Używa **bezpośredniego** binarium `fod-indexer` z jawnie przekazanym
   `FOD_INDEXER_CONNINFO`, żeby reguły `make` nie zmieniły bazy docelowej.
   Przed mutacjami sprawdza `current_database()` oraz gotowość schematu.
3. Rejestruje unikalne źródło, skanuje i materializuje dwa różne pliki.
   Następnie oznacza plan jako `materialize_failed`.
4. Osobnym backendem blokuje wiersz `index_import_plans` przy użyciu
   `SELECT ... FOR UPDATE`. Równolegle uruchamia `cleanup-failed`.
   Oczekuje na faktyczne zablokowanie `UPDATE index_import_plans`,
   identyfikując je przez `pg_stat_activity` i `pg_blocking_pids`.
5. Przed zwolnieniem blokady **niezależny** obserwator SQL musi widzieć
   oba pliki i niezmieniony status planu. Po zwolnieniu blokady wymagane
   są status `materialize_cleaned` i zero plików w katalogu importu.
6. W razie błędu kończy proces cleanup **przed** zwolnieniem blokady,
   wycofuje transakcję blokującą i usuwa wyłącznie unikalne zasoby testu.

Tryb TX-03D `--concurrent-reference-increment` dodatkowo hashuje dwa identyczne
pliki, tworzy zewnętrzną referencję i zatrzymuje cleanup po odczycie listy
survivorów. Osobna zatwierdzona transakcja dodaje kolejną referencję do tego
samego obiektu; po wznowieniu cleanup test wymaga `reference_count=2`, dwóch
żywych plików i zachowanego payloadu. Ten tryb wymaga tak samo jawnej,
odizolowanej bazy zakończonej `_test`.

Przebieg:

```bash
cd ~/git/fod
cargo build --locked -p fod-rust-indexer
python3 -m py_compile tests/integration/test_fod_indexer_cleanup_tx03_atomicity.py
FOD_PG_DBNAME=foddbname_hotpath_test \
POSTGRES_DB=foddbname_hotpath_test \
.venv/bin/python tests/integration/test_fod_indexer_cleanup_tx03_atomicity.py
```

Oczekiwane:
`TX03_ATOMIC_PRECOMMIT=PASS` i `TX03_ATOMIC_COMMIT=PASS`.

**Zakres:** testuje widoczność commit/rollback w trakcie prawdziwego cleanup
indexera, ale **nie** uruchamia mounta FUSE ani aktywnego pisarza. Ujemna
walidacja ochrony lease, writer fencing, usuwania aktualnie otwartego pliku
i zachowania przy zerwaniu połączenia pozostaje do TX-03B/C. Nie wolno
interpretować TX-03A PASS jako zakończenia TX-01/C10-B2.

## TX-03B — ochrona aktywnych dzierżaw przed cleanup

Stan: **kod ochronny i testy negatywne dodane; lokalna walidacja oczekuje**
(2026-10-15). TX-03A wcześniej zaliczył `build=0 compile=0 test=0`,
z `TX03_ATOMIC_PRECOMMIT=PASS` i `TX03_ATOMIC_COMMIT=PASS`.

Przegląd schematu ujawnił, że `file_write_leases.file_id` wskazuje
na `files.id_file` przez `ON DELETE CASCADE`. Bez odrzucenia aktywnej
dzierżawy, usunięcie pliku usuwałoby także jego rekord lease.

Zmiana w `rust_indexer/src/cleanup.rs`: przed pierwszą mutacją payloadu
zbierane są identyfikatory, katalogi nadrzędne i nazwy plików. Cleanup
sortuje i pobiera transakcyjne, nieblokujące advisory locks dla
`fod:write:file:<id>` oraz `fod:write:destination:<parent>:<bytes>:<name>`,
zgodnie z `DbRepo::acquire_write_ownership`. Jeśli którykolwiek jest
zajęty, cleanup odmawia operacji. Po uzyskaniu blokad odrzuca również
aktywne w PostgreSQL dzierżawy `file_write_leases` lub
`destination_write_leases`. Błąd ma prefiks
`FOD_INDEXER_CLEANUP_ACTIVE_WRITER` i powoduje rollback całej transakcji.

Nowy test `tests/integration/test_fod_indexer_cleanup_tx03_active_lease.py`
tworzy **unikalny** import w jawnej bazie `*_test`, a następnie
w dwóch niezależnych próbach wprowadza kontrolowane rekordy lease SQL:
1) lease pliku, 2) lease ścieżki docelowej. W obu przypadkach
wymaga odmowy cleanup oraz niezmienionego pliku, statusu planu i lease.
Po usunięciu wyłącznie własnej sesji testowej ponawia cleanup i
oczekuje jego powodzenia. Test nie montuje FUSE i nie symuluje
rzeczywiście zapisującego klienta.

```bash
cd ~/git/fod
cargo fmt --all --check
cargo check --workspace --locked
cargo build --locked -p fod-rust-indexer
.venv/bin/python -m py_compile tests/integration/test_fod_indexer_cleanup_tx03_active_lease.py
FOD_PG_DBNAME=foddbname_hotpath_test POSTGRES_DB=foddbname_hotpath_test \
  .venv/bin/python tests/integration/test_fod_indexer_cleanup_tx03_active_lease.py
FOD_PG_DBNAME=foddbname_hotpath_test POSTGRES_DB=foddbname_hotpath_test \
  .venv/bin/python tests/integration/test_fod_indexer_cleanup_tx03_atomicity.py
```

Oczekiwane komunikaty:
`TX03_ACTIVE_FILE_LEASE=DENIED`,
`TX03_ACTIVE_DESTINATION_LEASE=DENIED`,
`TX03_CLEANUP_AFTER_LEASE_RELEASE=PASS`, a także ponowny TX-03A PASS.

**Ograniczenia:** dotyczy współpracujących protokołów lease i
zaobserwowanych rekordów PostgreSQL. Nie chroni przed bezpośrednim SQL
superużytkownika (C10-A), wyścigiem tworzenia nowych plików w
zmieniającym się drzewie katalogów, operacjami open/unlink w innych
hostach ani błędnym `reference_count` przy współbieżnej adopcji.
Przed dopuszczeniem produkcyjnego cleanup potrzebne są TX-03C
(dwa mounty FUSE, aktywny uchwyt i writer, crash/rollback) oraz
pełna ocena lock order na namespace/data objects.

## Reguły przejścia do C10-B3

### TX-03C — cleanup aktywnego planu i recovery po crashu

Przegląd kodu 2026-10-31 wykazał, że `cleanup_failed_materialization()`
dopuszczał status `materialize_running`. Blokada `FOR UPDATE` na wierszu planu
nie zatrzymuje importera, który tworzy drzewo w osobnych transakcjach; ręczny
cleanup mógł więc skasować część importu, zanim importer dokończył pracę.
FOD 3.4.510 odmawia teraz cleanup dla `materialize_running` i zwraca
`FOD_INDEXER_CLEANUP_ACTIVE_MATERIALIZER`. Automatyczny rollback po błędzie
nadal działa, bo najpierw zapisuje `materialize_failed`. Odczyt i decyzja o
statusie pozostają w tej samej transakcji PostgreSQL.

Powyższa odmowa zamknęła wyścig ręcznego cleanup z importerem, który nadal
pracuje. W pierwotnym stanie nie rozwiązywała jednak odzyskiwania po awarii
procesu, który pozostawił plan w `materialize_running`; tę lukę adresuje
implementacja FOD 3.4.519 opisana niżej. Regresja bezpośrednio sprawdzająca
odmowę dla planu z nieodgasłym lease'em nadal pozostaje do uruchomienia.

FOD 3.4.515 przenosi `ensure_root_directory()` i `load_block_size()` do tej
samej obsługi błędu co tworzenie plików. Błąd tych kroków po zapisaniu planu
próbuje zapisać `materialize_failed` i wywołać cleanup.

FOD 3.4.519 dodaje migrację 0039 i trwały lease z monotonicznym tokenem
fencingowym. Plan importu dostaje token 1 i lease 60 sekund przy insercie.
Każda transakcja zapisująca katalog, plik, payload, adopcję payloadu lub
podsumowanie odświeża lease przez warunkowy `UPDATE` wiersza planu. Ten
`UPDATE` blokuje wiersz do końca transakcji, więc przejęcie nie może wyprzedzić
już rozpoczętego zapisu. Terminalna zmiana statusu również wymaga aktualnego
tokena i nieodgasłego lease'u. Po wygaśnięciu `cleanup-failed` podnosi token,
ustawia `materialize_failed` i usuwa drzewo w jednej transakcji; wznowiony
importer zostaje odrzucony przy następnym zapisie. Plan sprzed migracji z
`materialize_running`, tokenem 0 i brakiem lease'u pozostaje nieprzejmowalny,
żeby nie uznać starego aktywnego importera za martwy proces.

Kod i migracja przeszły kompilację Cargo dla hotpath, indexera i mkfs. Nie
uruchomiono regresji setup failure, aktywnego planu, wygasłego właściciela,
starego tokena ani współbieżnego przejęcia; TX-03C pozostaje otwarte do
walidacji runtime na izolowanej bazie. Test wymaga schematu 39 oraz PostgreSQL
FOD `BLCKSZ=32KiB`.

Źródłowa mapa mutacji do objęcia tokenem:

| Etap | Wywołania w `materialize.rs` | Granica, która musi sprawdzić fencing token |
| --- | --- | --- |
| Utworzenie planu i właściciela | `plan::insert_materialize_import_plan()` | Insert tworzy plan `materialize_running`, token 1 i lease razem |
| Katalog importu i podkatalogi | `ensure_root_directory()`, `ensure_directory_chain()` → `DbRepo::create_directory()` | Każda transakcja tworząca katalog odnawia i weryfikuje fence |
| Plik kanoniczny i referencje | `materialize_canonical_file()`, `materialize_reference_file()` → `DbRepo::create_file()` | Każda transakcja tworząca plik odnawia i weryfikuje fence |
| Zapis payloadu kanonicznego | `persist_file_blocks_from_path()` | Transakcja zapisująca bloki/rozmiar sprawdza fence; zapis payloadu jest transakcyjny |
| Adopcja payloadu dla duplikatu | `adopt_source_data_object()` | Transakcja zmieniająca referencję pliku i liczniki obiektu sprawdza fence |
| Zapis podsumowania planu | `upsert_index_import_plan_entries_staged()` | Transakcja stagingu sprawdza właściciela, a nie tylko istniejący `plan_id` |
| Zakończenie i obsługa błędu | `plan::update_import_plan()` oraz `cleanup_failed_materialization()` | Status terminalny wymaga aktualnego tokena; odzyskiwanie wygasłego planu inkrementuje token i sprząta w jednej transakcji |

Każdy `DbRepo` może pobierać kolejne połączenie z puli. Dlatego materializer
przekazuje fence w scoped `DbRepo`, ale autoryzacja nie opiera się na lokalnym
wrapperze: każda wymieniona metoda wykonuje warunkowy zapis wiersza planu w tej
samej transakcji co własną mutację. Procesowa blokada lub osobny heartbeat nie
zastąpiłyby tych kontroli.

### TX-03D — wyścig licznika współdzielonego obiektu (poprawka, test otwarty)

Analiza kodu 2026-10-31 wykazała, że cleanup wylicza listę plików spoza
importowanego drzewa, a następnie zapisywał jej długość bezwarunkowo do
`reference_count`. Równoległa adopcja mogła zwiększyć licznik między odczytem
listy a zapisem cleanup, który nadpisywał przyrost.

FOD 3.4.514 rozwiązuje to przez atomowe odjęcie liczby referencji usuwanych z
importu (`reference_count = reference_count - N`) z warunkiem
`reference_count >= N` i `RETURNING`. Niespójność przerywa transakcję.
Usuwanie obiektu uznanego za wyłączny ma warunek `reference_count = N`; jeśli
równoległa operacja zmieni licznik, `DELETE` nie zwróci wiersza i cleanup
wycofuje całą transakcję. Nie wprowadzono dodatkowych blokad obiektu przed
usunięciem plików: wszystkie wiersze `files` są kasowane przed arytmetycznymi
aktualizacjami współdzielonych liczników, zgodnie z kolejnością unlink/write.

`tests/integration/test_fod_indexer_cleanup_tx03_atomicity.py
--concurrent-reference-increment` dodaje deterministyczną pauzę po klasyfikacji
referencji cleanupu, zatwierdza dodanie drugiego pliku w niezależnej
transakcji, a następnie wznawia cleanup. Test sprawdza, czy licznik odpowiada
obu żywym plikom i czy payload nadal istnieje. Test nie został jeszcze
uruchomiony; runtime reprodukcja pozostaje otwarta. Szczegóły są w `TODO.md`.

### TX-03E — cleanup importu usuwał zewnętrzne aliasy hardlinku (guard dodany)

Analiza `rust_indexer/src/cleanup.rs` wykazała, że cleanup kasuje wiersz
`files`, a FK `hardlinks.id_file REFERENCES files(id_file) ON DELETE CASCADE`
usuwa przy tym także hardlink umieszczony poza importowanym katalogiem. To
powodowało utratę zewnętrznego wpisu namespace przy czyszczeniu nieudanego
importu.

FOD 3.4.513 sprawdza aliasy po zdobyciu blokad `fod:open:file:<id>` dla plików
importu. Tworzenie hardlinku bierze ten sam klucz i blokadę wiersza pliku,
więc nowy alias nie może powstać pomiędzy sprawdzeniem i usunięciem. Jeśli
istnieje alias poza poddrzewem, cleanup kończy się błędem
`FOD_INDEXER_CLEANUP_EXTERNAL_HARDLINK`; transakcja nie kasuje pliku ani aliasu.
`tests/integration/test_fod_indexer_cleanup_failed.py` dodaje teraz przypadek,
który tworzy taki alias przez zamontowany FUSE, sprawdza odmowę cleanup oraz
zachowanie drzewa, aliasu i payloadu, a następnie usuwa alias i kontynuuje
cleanup. Test nie został jeszcze uruchomiony, więc potwierdzenie runtime nadal
pozostaje otwarte.

- Wszystkie mutacje `indexer cleanup` mają udokumentowany porządek blokad,
  spójność ze stanem lease i jedną transakcję na **jednym backendzie PG**.
- Zdefiniowano, które operacje importu i legacy wymagają zaufanej zdolności,
  a które muszą wymagać tokenu writer.
- `mkfs` nie nadaje runtime nieograniczonego DML po `upgrade`.
- Runtime nie wymaga praw `CREATE/ALTER` do schematu.
- Testy negatywne obejmują bezpośredni SQL, stale writer, tokenless import,
  podwójny mount i rollback podczas współbieżności.

C10-A jest zamknięty, ale stwierdzenie `C10_AUDIT=UNRESTRICTED` pozostaje
ważną, **nierozwiązaną** słabością uprawnień runtime. Niniejszy przegląd
nie zastępuje analizy pozostałych wskazań skanera.
