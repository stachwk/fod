# C10-B2 — ręczny przegląd transakcji o najwyższym ryzyku

Data: 2026-10-13. Zakres: pierwsza transza z 1045 kandydatów skanera (149 plików).
Stan: **przegląd kodu wykonany; hipotezy współbieżności i uprawnień oczekują na testy**.
Nie zmieniono danych PostgreSQL, schematu, kont, uprawnień ani implementacji FOD.

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

## Reguły przejścia do C10-B3

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
