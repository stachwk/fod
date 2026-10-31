# FOD Decisions, Follow-ups, Unresolved Findings & Archive

This document preserves historical decisions and completed follow-ups, and is
the durable record for unresolved findings that must not be lost. It is not an
active implementation backlog. The only ordered queue is
[`docs/plans/CURRENT.md`](docs/plans/CURRENT.md). The legacy path
[`docs/FOD_CURRENT_ACTION_PLAN.md`](docs/FOD_CURRENT_ACTION_PLAN.md) is a
compatibility pointer.

Reading guide:
- `Historical note:` means closed-item context, benchmark results, or archived implementation detail.
- `Decision:` means durable guidance that still applies.
- The old extent-engine proposal is superseded; `Target Architecture` is historical context, not the current runtime design.

## Unresolved findings

- [ ] **Bezpiecznie odzyskiwać import pozostawiony w `materialize_running`.**
  `cleanup-failed` odmawia teraz usunięcia drzewa, gdy proces importera może
  nadal je tworzyć; blokada wiersza planu nie zatrzymuje operacji importu
  wykonywanych w osobnych transakcjach. Awaria procesu może więc pozostawić
  częściowe drzewo i plan w stanie `materialize_running`. Zaprojektować trwały
  lease/właściciela planu z wygaśnięciem i kontrolowanym przejęciem po
  potwierdzeniu braku aktywnego importera. Do tego czasu nie zmieniać ręcznie
  statusu planu bez potwierdzenia, że żaden importer go nie wykonuje. Kod:
  `rust_indexer/src/cleanup.rs`; dodać i uruchomić regresję odmowy cleanup dla
  aktywnego planu. Szczegóły ryzyka TX-03C w
  `docs/FOD_C10B2_HIGH_RISK_TRANSACTION_REVIEW.md`.

- [ ] **Dodać regresję współbieżnej adopcji i cleanup licznika obiektu.**
  FOD 3.4.514 zastępuje zapis starej liczby referencji atomowym odjęciem liczby
  usuwanych plików. PostgreSQL serializuje tę arytmetykę na wierszu obiektu, więc
  równoległy przyrost nie jest nadpisywany. Usunięcie obiektu, który uznano za
  wyłączny, jest warunkowe względem licznika oczekiwanego; zmiana powoduje
  rollback zamiast skasowania potencjalnie używanego obiektu. Źródłowy
  interleaving jest zidentyfikowany, ale regresji dwóch transakcji jeszcze nie
  uruchomiono. Test ma sprawdzać dokładny licznik oraz zachowanie payloadu dla
  każdego żywego pliku. Szczegóły TX-03D:
  `docs/FOD_C10B2_HIGH_RISK_TRANSACTION_REVIEW.md`.

- [ ] **Dodać regresję zewnętrznego hardlinku dla cleanup importu.**
  FOD 3.4.513 odmawia cleanup, jeśli plik z importowanego drzewa ma alias
  hardlink poza tym drzewem. Bez tej kontroli `DELETE files` uruchamiał
  `ON DELETE CASCADE` i usuwał również zewnętrzny wpis. Sprawdzenie wykonuje
  się po zdobyciu blokad cyklu życia plików, które współdzieli z `link()`.
  Dodać test potwierdzający odmowę oraz zachowanie aliasu i payloadu; testu
  jeszcze nie uruchomiono. Szczegóły TX-03E w
  `docs/FOD_C10B2_HIGH_RISK_TRANSACTION_REVIEW.md`.

- [ ] **Uruchomić montowany test dwóch mountów dla wyścigu źródła `link()`.**
  FOD 3.4.475 przekazuje do transakcji PostgreSQL dokładny wpis źródłowy
  `(kind, entry_id, parent_id, name)` i odrzuca go, jeśli inny host usunął lub
  zastąpił nazwę przed utworzeniem aliasu. Regresja `pg_query` przeszła na
  PostgreSQL 16.15 z `--with-blocksize=32` i `SHOW block_size=32768`, obejmując
  hardlink pliku oraz hardlink symlinka, gdy backing inode nadal istnieje.
  Pozostaje test montowany z deterministycznym wyścigiem na dwóch hostach.
  Callback skompilowano profilem `release-lto` w builderze z libfuse 3.18.3.
  Lokalny host ma libfuse 3.14.0, poniżej wymaganego 3.18.2; montowany test
  wymaga hosta z odpowiednią wersją libfuse i działającymi uprawnieniami FUSE.
  Szczegóły: [`docs/FOD_PRIMARY_AUTHORITY_AUDIT.md`](docs/FOD_PRIMARY_AUTHORITY_AUDIT.md).

- [x] **Domyślne stosy SELinux/ACL i HA-chain muszą używać PostgreSQL 32 KiB.**
  `docker-compose.selinux-acl.yml` używa teraz obrazu FOD 16.15 z nowym
  wolumenem i healthcheckiem `SHOW block_size`; `docker-compose.ha-chain.yml`
  buduje ten sam obraz dla mastera i replik, sprawdza rozmiar stron w
  healthcheckach i ma osobne wolumeny 32 KiB. Test polityki obejmuje oba pliki.
  Oba stosy uruchomiono 2026-10-31; SELinux/ACL potwierdził
  `--with-blocksize=32` oraz `32768`, a master i obie repliki HA-chain
  przeszły healthcheck i zwróciły te same wartości. Testowe kontenery i ich
  wolumeny usunięto po weryfikacji.

- [ ] **Uruchomić test końcowego purge po zamknięciu ostatniego uchwytu przy opóźnionej replice.**
  `finalize_deferred_unlink_after_handle_removal()` sprawdza teraz stan
  `unlinked`, lease i purge przez widok primary. Test dwóch mountów
  `last_close_purge_uses_primary_during_replica_lag` jest dodany do
  `tests/integration/test_unlink_primary_authority_docker.py`; należy go
  ponowić na hoście z libfuse >= 3.18.2 i uprawnieniami FUSE. Próba 2026-10-31
  przeszła sprawdzenie obu baz (`--with-blocksize=32`, `SHOW block_size=32768`),
  ale mount przerwał start, bo host ma libfuse 3.17. Artefakt weryfikacji PG:
  `artifacts/integration/unlink-primary-authority-20261031T160742Z/`.

- [ ] **Lokalne endpointy PostgreSQL mają starsze schematy niż aktualny kod FOD.**
  Kontener `fod-postgres` na porcie 5432 ma `fod.schema_version=31`, a
  kontenery deploymentu mają wersję 23; brakuje w nich m.in. tabeli
  `symlink_hardlinks` wymaganej przez bieżący kod. Początkowo klient kończył
  połączenie błędem `42P01` podczas przygotowania statementów; FOD 3.4.470
  sprawdza wersję przed tym krokiem i zwraca instrukcję `mkfs.fod upgrade`.
  Obraz PostgreSQL jest poprawnie zbudowany z `--with-blocksize=32` i zwraca
  `block_size=32768`. Endpointy nadal wymagają uaktualnienia do schematu 38,
  zanim będą mogły obsłużyć bieżące testy. Do testów na odizolowanej bazie
  wskazuj pustą bazę administracyjną, z której harness tworzy tymczasową bazę;
  przed pełnym uruchomieniem `fod-rust-hotpath` potwierdź schemat endpointu.

- [ ] **`link()` do symlinka różni się od zachowania Linuksa.**
  Na lokalnym Linuksie 6.17 wywołanie `os.link(source, alias,
  follow_symlinks=False)` dla `source` będącego symlinkiem tworzy drugi wpis
  tego samego inode (`lstat()` pokazuje `S_IFLNK`, identyczne inode i `nlink=2`).
  FOD 3.4.461 dodaje tabelę `symlink_hardlinks` (migracja 38) i wstępną
  obsługę PostgreSQL/FUSE: aliasy współdzielą inode, target, metadane i xattry;
  działają lookup, readdir, `st_nlink`, `link()`, `unlink()` z promocją nazwy
  oraz rename aliasu i zastępowanie symlinka z promocją jego aliasu. Test
  integracyjny warstwy PostgreSQL przeszedł na odizolowanej bazie PostgreSQL
  16.15 z `BLCKSZ=32KiB`. Dodano test montowany
  `tests/integration/test_symlink_hardlink.py` oraz target `make
  test-symlink-hardlink`; obejmuje Linux `link()` z no-follow, typowe błędy,
  wspólny inode/target, znaczniki czasu, readdir, rename, promocję po unlink i
  replacement oraz alias w roocie. Runtime zbudowany i uruchomiony w teście
  zgłosił Linux 6.17.0-41 oraz libfuse 3.18.3, ale mount zwrócił `EPERM`
  przed asercjami (środowisko nie ma efektywnych capabilities); izolowana baza
  testowa została usunięta. Test wymaga ponownego uruchomienia na hoście, który
  dopuszcza mount FUSE. Nadal wymagane są
  pełna weryfikacja errno (w tym read-only i odmowy uprawnień), testy wyścigów
  na dwóch mountach i kontrola obsługi zapamiętanych ścieżek inode. Zachowanie
  `link()` dla symlinka pozostaje
  implementacyjnie zdefiniowane przez POSIX; to zgodność z Linuksem, nie
  stwierdzone naruszenie POSIX. Szczegóły i akceptacja są w
  [`docs/SYMLINK_HARDLINK_DESIGN.md`](docs/SYMLINK_HARDLINK_DESIGN.md).

- [x] **`opendir()` nie może zwracać inode roota jako `d_ino` wpisu `..` po utracie mapy rodzica.**
  Callback brał inode rodzica wyłącznie z lokalnego cache ścieżek i w razie
  braku mapowania podstawiał `ROOT_INO`. Regresja montowana celowo usuwa wpis
  rodzica z cache przed `opendir()` zagnieżdżonego katalogu i sprawdza inode
  `..` z `getdents64()`. `opendir()` pobiera teraz ID rodzica z rekordu
  katalogu w PostgreSQL, a inode rodzica po tym ID z primary. Pełny
  `test_readdir_snapshot_multimount.py` przeszedł na FOD 3.4.458, PostgreSQL
  16.15 z `BLCKSZ=32KiB`, Linux 6.17.0-41-generic i libfuse3 3.18.3; baza
  tymczasowa została usunięta. Sprawdzono też kompilację `release-lto` z
  `integration-test-hooks` i `cargo check` dla obu zmienionych pakietów.

- [x] **`rename()` nie może kontynuować po błędzie odczytu inode źródła.**
  Callback ignorował błąd odczytu atrybutów źródła i podstawiał `ROOT_INO` do
  aktualizacji lokalnej mapy ścieżek po commicie `rename`. Mogło to usunąć
  mapowanie inode źródła i przypisać ścieżkę docelową do inode roota. Atrybuty
  są teraz pobierane po ID wpisu z wcześniejszego odczytu PostgreSQL, a błąd
  przerywa mutację przed transakcją. Deterministyczny test wstrzykuje błąd
  odczytu, wymaga `EIO` oraz niezmienionych nazw i inode. Pełny
  `test_create_write_ownership.py` przeszedł na FOD 3.4.457, PostgreSQL 16.15
  z `BLCKSZ=32KiB`, Linux 6.17.0-41-generic i libfuse3 3.18.3; baza testowa
  została usunięta. Pierwsza próba użyła domyślnego hasła zamiast poświadczeń
  kontenera i nie przeszła inicjalizacji; powtórka z poświadczeniami serwera
  zakończyła się powodzeniem.

- [x] **`unlink` nie może przypisać wpisu dziennika do zastępczego pliku.**
  Po commicie `unlink` callback ponownie rozwiązywał ścieżkę, aby pobrać
  `file_id`; inny mount mógł wcześniej utworzyć pod tą nazwą nowy plik. Test
  dwóch mountów zatrzymuje callback po transakcji usunięcia i przed zapisem
  dziennika, tworzy zastępcę, a następnie sprawdza, że dokładnie jeden wpis
  `unlink` ma `id_file=NULL`, zamiast ID zastępcy. Callback używa teraz wyniku
  transakcji: zachowuje ID tylko gdy oryginalny wiersz nadal istnieje, a nie
  rozwiązuje ponownie ścieżki. `test_create_write_ownership.py` przeszedł na
  FOD 3.4.456, PostgreSQL 16.15 z `BLCKSZ=32KiB`, Linux 6.17.0-41 i libfuse3
  3.18.3; baza testowa została usunięta.

- [x] **Udokumentować precyzję znaczników czasu przechowywanych przez PostgreSQL.**
  `TIMESTAMP` w PostgreSQL zapisuje ułamki sekund z rozdzielczością 1 µs.
  Test symlinku podaje `tv_nsec=123456789` i sprawdza po zapisie oraz `stat()`
  wartość `123457000`; wejście nanosekundowe jest zaokrąglane do najbliższej
  mikrosekundy. `test_symlink.sh` przeszedł na FOD 3.4.455, PostgreSQL 16.15
  z `BLCKSZ=32KiB`, Linux 6.17.0-41 i libfuse3 3.18.3. Pierwsze uruchomienie
  `make test-symlink` zatrzymało się na współdzielonej bazie ze starszym stanem
  schematu; test powtórzono na izolowanej bazie. Runtime wymagał jawnego
  `LD_LIBRARY_PATH`, aby załadować libfuse3 3.18.3 zamiast systemowej 3.17.0.
  Pełny obszar P06 pozostaje otwarty dla pozostałych pól `stat` i błędów.

- [x] **Walidacja inode rodzica i wybór jego ID korzystają z tego samego odczytu PostgreSQL.**
  `parent_entry_id_for_inode()` pobierało atrybuty i potwierdzało inode, po
  czym osobnym `resolve_path()` odczytywało ID katalogu. Podmiana katalogu
  pomiędzy zapytaniami mogła skierować `mkdir()`/inne mutacje do katalogu
  zastępczego pod tą samą ścieżką. `lookup()` również pobierało dziecko po
  ścieżce, więc żądanie ze starym inode rodzica mogło zwrócić wpis z nowego
  katalogu. Walidacja bierze teraz ID z tego samego wiersza co inode, a
  `lookup()` odczytuje wpis po `directory_id + name` i atrybuty po ID obiektu.
  Barierowe testy dwóch mountów podmieniają katalog podczas `mkdir()` i
  `lookup()`; oba żądania starego inode zwracają `ENOENT`, a katalog zastępczy
  pozostaje niezmieniony. Pełny
  `test_create_write_ownership.py` przeszedł na FOD 3.4.455, PostgreSQL 16.15
  `BLCKSZ=32KiB`, Linux 6.17.0-41 i libfuse3 3.18.3; baza testowa została
  usunięta.

- [x] **`poll()` zachowuje tożsamość deskryptora i zgłasza EOF jako gotowość do odczytu.**
  Callback używał `ino`/ścieżki mimo otrzymania `fh`; po zdalnym `unlink`
  otwartego pliku regresja `test_poll.py` dostała `POLLERR` (mask `8`) zamiast
  `POLLIN`, choć `pread()` nadal zwracał oryginalny payload. `poll()` pobiera
  teraz atrybuty po `FileHandleState.file_id`, a zwykły plik jest czytelny
  także na EOF. Test obejmuje pusty plik i otwarty, zdalnie odłączony deskryptor.
  `test_poll.py` przeszedł po poprawce na PostgreSQL 16.15 z
  `BLCKSZ=32KiB`, libfuse3 3.18.3; w teście działały dwa mounty zapisu i mount
  tylko do odczytu.

- [x] **`create()` nie może zwrócić inode zastępcy po wyścigu unlink/recreate.**
  Po zatwierdzeniu nowego pliku FUSE pobierało atrybuty ponownie po ścieżce,
  mimo że uchwyt już wskazywał konkretne `file_id`. Drugi mount mógł w tym
  oknie usunąć nazwę i utworzyć pod nią inny plik, przez co odpowiedź `CREATE`
  łączyła inode zastępcy z uchwytem pierwotnego pliku. `create()` pobiera teraz
  atrybuty po zapisanym w uchwycie `file_id`. Dwumountowy test z barierą po
  commicie odtworzył błąd przed poprawką (oba deskryptory miały inode
  zastępcy), a po zmianie wykazał różne inode i niezależne payloady. Cały
  `test_create_write_ownership.py` przeszedł 2026-10-31 na Linux 6.17.0-41,
  libfuse3 3.18.3 i PostgreSQL 16.15; użyto usuniętej po teście bazy
  tymczasowej.

- [x] **Create callback nie może pomylić obiektu z jego zastępcą ani aliasem hardlinku.**
  `mkdir`, `mknod` i `symlink` pobierają teraz metadane odpowiedzi po ID zwróconym
  z zatwierdzonej transakcji; `link` pobiera je po ID pliku źródłowego, zachowując
  tożsamość hardlinku. Dwumountowy test z barierą usuwa utworzony obiekt i tworzy
  inny pod tą samą nazwą; callbacki oryginalnych obiektów zwracają `EIO`, zamiast
  atrybutów zastępcy. Pierwsza próba ujawniła, że parser pustą ścieżkę metadanych
  po ID normalizował do `/` i przypisywał katalogom inode roota; root jest teraz
  rozpoznawany jawnie. Test hardlinku wykrył też nadpisywanie odwrotnej mapy
  inode→ścieżka przez alias; rejestracja zachowuje istniejącą ścieżkę dla
  współdzielonego inode, a jej usunięcie wybiera pozostały alias. Pełny
  `tests/integration/test_create_write_ownership.py` przeszedł 2026-10-31 na FOD
  3.4.452, Linux 6.17.0-41, libfuse3 3.18.3 i PostgreSQL 16.15 z
  `BLCKSZ=32KiB`; test używał i usunął tymczasową bazę.

- [x] **`open()` musi zachować tożsamość zweryfikowanego inode przy równoległej podmianie ścieżki.**
  Callback sprawdzał inode i uprawnienia, a następnie ponownie rozwiązywał
  ścieżkę, by znaleźć `file_id`. Drugi mount mógł w tym oknie usunąć plik i
  utworzyć zastępcę; żądanie starego inode otwierało wtedy nowy plik. `open()`
  bierze teraz ID właściciela z tych samych atrybutów, które zweryfikowały
  inode, a lease w PostgreSQL odrzuca ID już usunięte z `ENOENT`. Test dwóch
  mountów zatrzymuje callback po walidacji inode, podmienia nazwę i sprawdza,
  że `open()` zwraca `ENOENT`, a bajty zastępcy pozostają niezmienione. Pełny
  `test_create_write_ownership.py` przeszedł na FOD 3.4.453, Linux 6.17.0-41,
  libfuse3 3.18.3 i PostgreSQL 16.15. Bieżący serwer sprawdzono przez
  `/opt/postgresql-custom/bin/pg_config` (`--with-blocksize=32`) i `SHOW
  block_size=32768`; test użył tymczasowej bazy usuniętej po zakończeniu.

- [x] **Weryfikacja `BLCKSZ` nie może polegać na `PATH` z powłoki logowania.**
  `sh -lc` resetowało `PATH` i uruchamiało standardowe binaria PostgreSQL z
  `/usr/local/bin`, przez co `pg_config` raportował wariant 8 KiB mimo obrazu
  i serwera FOD 32 KiB. Sprawdzenie przez `/opt/postgresql-custom/bin/pg_config`
  oraz `SHOW block_size` wykonane rolą skonfigurowaną w kontenerze potwierdziło
  `--with-blocksize=32` i `32768`. `AGENTS.md` wymaga teraz jawnego używania
  niestandardowych binariów lub powłoki bez trybu logowania oraz sprawdzenia
  działającego serwera.

- [x] **Obsłużyć i zweryfikować długie ścieżki w indeksach indeksatora.**
  Źródło problemu było potwierdzone w kodzie: `WalkDir` zapisuje ścieżkę
  względną bez limitu aplikacyjnego, podczas gdy B-tree pełnego tekstu może
  odrzucić wpis większy niż około 1/3 strony. Implementacja w migracji 0035
  zastępuje takie klucze indeksami MD5 zawężającymi kandydatów; transakcyjny
  kod porównuje pełne wartości z `COLLATE "C"` i serializuje zapisy blokadami
  advisory w PostgreSQL. Dodano regresję `tests/integration/test_fod_indexer_long_paths.py`
  dla 3000-bajtowej nazwy źródła oraz ścieżki przekraczającej limit wpisu
  typowej strony 8 KiB. FOD domyślnie używa PostgreSQL kompilowanego z
  `BLCKSZ=32KiB` (`--with-blocksize=32`); uruchomiony obraz potwierdził
  `SHOW block_size=32768`. Obraz PG8 jest wariantem zgodnościowym. Bezpośrednia
  próba indeksowania niekompresowalnego wpisu 3216-bajtowego na PG8 odrzuciła go
  przy limicie 2704 bajtów; regresja indeksatora z digestem przeszła na obu
  wariantach z `source_name_bytes=3000 path_bytes=3041`, tworzeniem snapshotu
  i pełnym odczytem ścieżki. Test Make działa na dedykowanej tymczasowej bazie,
  sprzątanej po uruchomieniu, i nie buduje ani nie montuje frontendu FUSE.
  `make test-schema-upgrade` również przeszedł na izolowanej bazie, potwierdzając
  upgrade do schema 35. Początkowe próby na głównej bazie nie mogły wykonać
  upgrade, bo były w niej aktywne sesje FOD; nie zmieniano tej bazy. Zmiana
  `TEXT` na `VARCHAR(n)` nie rozwiązuje problemu.

- [x] **Poszerzyć graf identyfikatorów SQL.** Rust/FUSE i indeksator używały
  `u64`, podczas gdy `SERIAL`/`INTEGER` zatrzymywały auto-ID na `2^31-1`.
  Schema 37 zmienia 45 kolumn identyfikatorów/referencji na `BIGINT` i 17
  sekwencji na zakres signed 64-bit; zachowuje 29 FK oraz 7 triggerów.
  Regresja upgrade z wersji 1 weryfikuje typy, maksimum sekwencji i zachowane
  ograniczenia. Odczyt schema 31 z 2026-10-31 wykazał 24 FK w grafie i
  dodatkowe referencje bez FK (`xattrs.owner_id`, snapshot indeksatora); stan
  lokalny był daleko od limitu, ale nie wyznacza limitu innych wdrożeń.
  Produkcyjnego upgrade nie wykonywałem. Wymagane jest okno serwisowe, bo
  zmiana typów może blokować i przepisywać indeksy. Szczegóły i wyniki są w
  `docs/SQL_SCHEMA_LIMIT_AUDIT.md`.

- [x] **Ocenić potrzebę liczników poza signed `BIGINT`.** Decyzja: pozostawić
  `BIGINT`; obecny kontrakt nie wymaga wartości powyżej `i64::MAX`. Rozmiary
  plików są ograniczone przez 64-bitowy `off_t`, ID tworzy PostgreSQL, indeksy
  bloków wynikają z limitu rozmiaru pliku, a `st_nlink` podlega limitowi ABI
  FUSE i zwraca `EOVERFLOW` przy jego przekroczeniu. Liczniki indeksatora i
  referencji nie deklarują pełnej domeny Rust `u64`. Sam typ Rust nie uzasadnia
  `NUMERIC(20,0)`; wrócić do sprawy, jeśli wymaganie konkretnego pola obejmie
  większy zakres. Szczegóły w `docs/SQL_SCHEMA_LIMIT_AUDIT.md`.

- [x] **Znormalizować tryby POSIX i walidować je w SQL.** Odczytowy audyt bazy
  schema 31 znalazł 800 plików z historycznie zapisanym pełnym `st_mode`;
  aktualny parser wymaga samych bitów uprawnień, bo typ inode jest osobnym
  polem. Migracja 0036 sprawdza bity typu, normalizuje `files`/`directories`
  do wartości `0..07777` i dodaje warunki `CHECK`; świeży schemat ma te same
  warunki. Regresja upgrade z wersji 1 sprawdza normalizację pliku `100644` do
  `644` i katalogu `40755` do `755`. Aktywna baza nie była modyfikowana.
  Szczegóły oraz wynik audytu są w `docs/SQL_SCHEMA_LIMIT_AUDIT.md`.

- [x] **Egzekwować w PostgreSQL bajtowy limit 255 dla nazw FOD.** `VARCHAR(255)`
  liczyło znaki, a nazwy komponentów Linuxa są limitowane bajtami; ponadto
  `destination_write_leases.name` miało typ `TEXT`. Schema 33 dodała
  `octet_length(name) <= 255` do nazw wpisów katalogowych, nazw xattr i nazwy
  dzierżawy. Migracja i świeży schemat mają te same warunki. Test schematu
  sprawdza granicę 255 bajtów oraz odrzucenie 256-bajtowej nazwy UTF-8 przez
  bezpośredni SQL.

- [x] **Dopasować numeryczne kolumny SQL do domen używanych przez FOD/Linux.**
  Schema 34 używa `BIGINT` z zakresem `u32` dla UID/GID, `journal.id_user` i
  numerów urządzeń, `BIGINT` dla indeksów bloków i liczników używanych jako
  `u64`, oraz odrzuca ujemne rozmiary, indeksy i liczniki. Bezpośrednie testy
  SQL sprawdzają `u32::MAX` oraz odrzucenie wartości większej. Limity kluczy
  `SERIAL`/`INTEGER` pozostają osobnym punktem audytu. Odczytowe sprawdzenie
  istniejącej bazy wykazało zero wartości naruszających nowe nieujemne zakresy
  i zakres `u32`; baza nie była modyfikowana.

- [x] **Rename replacement must preserve an open destination inode across mounts.**
  A mounted two-mount regression opened the destination on mount B, replaced its
  name from mount A, then received `ENOENT` reading the still-open descriptor.
  Root cause is confirmed in `remove_primary_file_or_promote_hardlink_on_conn()`:
  the regular-file target branch deletes the `files` row without checking
  PostgreSQL `file_open_leases`, unlike unlink's deferred-purge path. The
  replacement transaction now checks active leases under the PostgreSQL
  inode lifecycle lock and marks the target unlinked instead of deleting it;
  final handle release purges it after the last lease. The regression verifies
  the old descriptor's payload and inode, `st_nlink == 0`, and replacement
  visibility on both mounts. `test_rename_directory_transactional.py` passed
  on Linux 6.17.0-41-generic/libfuse3 3.18.2/PostgreSQL 16.15 against an
  isolated database, which was dropped after the run.

- [x] **Nie-UTF-8 w celu symlinka i nazwie xattr nie może aliasować U+FFFD.**
  PostgreSQL `text` nie zachowuje dowolnych bajtów wejścia. Montowane regresje
  sprawdzają, że `symlink()` z celem `target-\xff` oraz
  `getxattr()`/`setxattr()`/`removexattr()` z nazwą `user.fod.\xff` zwracają
  dokładnie `EILSEQ`, nie tworzą linku i nie zmieniają poprawnych obiektów o
  nazwach zawierających U+FFFD. Testy `make test-symlink test-xattr` przeszły
  2026-10-31 na Linux 6.17/libfuse3 3.17.4 i izolowanej bazie PostgreSQL 16.15;
  bazę testową usunięto po przebiegu. To zamyka regresję aliasowania, nie
  zapewnia pełnej obsługi dowolnych bajtów ścieżek; do tego potrzebna byłaby
  reprezentacja bajtowa w schemacie.

- [x] **`expand_user()` zachowuje bajty ścieżki i rozwija tylko `~/`.**
  Test `expand_user_only_replaces_tilde_prefix_and_preserves_path_bytes`
  sprawdza ścieżki `~`, `~/`, względną, bezwzględną i sufiks zawierający
  nie-UTF-8 zarówno pod `~/`, jak i w zwykłej ścieżce. Test jednostkowy
  `cargo test --offline -p fod-rust-runtime
  expand_user_only_replaces_tilde_prefix_and_preserves_path_bytes` przeszedł
  2026-10-31. Nie rekonstruujemy ścieżki przez konwersję tekstową.

- [x] **Nazwy zakończone spacją lub tabulatorem nie mogą aliasować krótszej ścieżki.**
  Montowany test odtworzył kolizję trzech nazw: `get_file_id()`,
  `get_file_mode_value()`, `get_hardlink_id()`, `get_symlink_id()`,
  `resolve_path()`, `list_directory_entry_metadata()` i
  `list_directory_entries_blob()` usuwały spacje/tabulatory przez `path.trim()`.
  Teraz lookupi przekazują ścieżkę bez zmian; test rozróżnia `trailing`,
  `trailing ` i `trailing\t`, sprawdza odczyt, rename i unlink oraz pliki w
  katalogach `parent ` i `parent\t`. Haki wyścigowe testów porównują też
  ścieżki dokładnie, bez `trim()`. `test_path_trailing_slash.py` przeszedł
  2026-10-31 na Linux 6.17.0-41/libfuse3 3.17.4 z PostgreSQL 16.15;
  test używał izolowanej bazy, którą usunięto po przebiegu.

- [x] **Do not treat external `dirfd()` closure as a POSIX `closedir()` test.**
  The directory-stream regression had closed the descriptor returned by
  `dirfd()` and then required `closedir()` to return `EBADF`, incorrectly
  attributing that result to POSIX. POSIX `dirfd()` says closing the descriptor
  outside the directory-stream operations has undefined behavior. The
  Linux/glibc probe happened to return `EBADF`, but that is not portable
  conformance evidence. Removed the assertion from the gating mounted test and
  reclassified it in the POSIX audit as an implementation-specific diagnostic.
  The regular `closedir()` success path remains covered. The corrected
  `test_readdir_snapshot_multimount.py` passed on FOD 3.4.398 and was rerun on
  FOD 3.4.411 in `release-lto` against a disposable PostgreSQL 16.15 database
  on Linux 6.17.0-41-generic/libfuse3 3.17.4. Both runs exercised mounted
  `open()`/`opendir()` `EMFILE`; the disposable database was dropped. POSIX
  source: `dirfd()`; root cause confidence: high.

- [x] **Whole-object `copy_file_range` adoption did not serialize with source writes.**
  `adopt_source_data_object_optional_fence()` validated and locked the
  destination lease but read the source object and incremented its reference
  count without taking the source inode's `fod:write:file:<id>` advisory lock.
  FOD block writers use that key to serialize their reference-count/COW
  decision. A source write could therefore decide the data object was unique
  before adoption committed, then modify it after the destination began
  sharing it. The Rust transaction now acquires source and destination write
  locks in sorted resource-key order before re-reading metadata and validating the
  destination fence. Regression `test_copy_file_range_source_fencing.py`
  blocks the source key, requires adoption to wait on that PostgreSQL lock, then
  modifies the source and checks the destination retained the original bytes
  and data object. Root cause confidence: high from the shared advisory-lock
  paths and transaction code. Impact: a concurrent source write could alter a
  copied destination. The mounted regression passed on FOD 3.4.397 in
  `release-lto` against disposable PostgreSQL 16.15 on Linux
  6.17.0-41-generic/libfuse3 3.17.4. The existing C7 destination ownership and
  C8 stale-fencing regressions passed on the same stack and database. The test
  database was removed after the run. No pre-fix byte-corruption reproduction
  was recorded; the race was identified from the transaction/lock paths, and
  the regression verifies source-lock serialization plus post-copy COW
  isolation. See [primary-authority audit](docs/FOD_PRIMARY_AUTHORITY_AUDIT.md).

- [x] **Callback FUSE `access(X_OK)` używał bitu wykonywania z opóźnionej repliki.**
  Callback `FodFuse::access` pobierał początkowe atrybuty przez zwykły read
  routing, a następnie stosował wyjątek Linuxa wymagający co najmniej jednego
  bitu wykonywania dla root. `acl_allows()` odświeżał metadane z primary, lecz
  rootowy bypass zwracał zgodę; po cofnięciu `x` na primary stary bit z repliki
  mógł więc dać błędne `access(X_OK) == true`. Callback pobiera teraz atrybuty
  żądanego inode przez primary-backed `authoritative_entry_attrs_for_ino()`.
  `test_unlink_primary_authority_docker.py` zatrzymuje replay, sprawdza rozbieżne
  tryby w PostgreSQL i wymaga odmowy `os.access(X_OK)` na mouncie z
  `FOD_DEFAULT_PERMISSIONS=0`. Cały test primary/replica przeszedł na FOD
  3.4.383 dnia 2026-10-30; artefakty: `artifacts/integration/unlink-primary-authority-20261030T225902Z/`.

- [x] **`default_permissions` omijało primary-authoritative sprawdzenie prawa przejścia rodzica.**
  Dwumountowa regresja w `test_unlink_primary_authority_docker.py` odtwarza
  katalog `0711` dla `nobody`, zatrzymuje replay, zmienia primary na `0700` i
  próbuje otworzyć czytelny plik potomny z mountu `FOD_DEFAULT_PERMISSIONS=1`.
  Przed poprawką open zwrócił sukces (`errno=0`): `lookup()`/`opendir()` oraz
  wspólny `check_path_search_permissions()` polegały na kernelowym prechecku,
  który widział opóźniony tryb. Sprawdzenie FOD korzysta teraz zawsze z
  autorytatywnej ACL/mode, niezależnie od kernelowej opcji. Test primary/replica
  przeszedł na FOD 3.4.384 dnia 2026-10-31; artefakty:
  `artifacts/integration/unlink-primary-authority-20261030T231016Z/`.

- [x] **`access(X_OK)` zwracał stary tryb przy kernelowym `default_permissions`.**
  Linux wykonuje lokalny precheck na atrybutach inode, a po wygaśnięciu ich TTL
  odświeża je przez FUSE `GETATTR`; callback `FUSE_ACCESS` nie jest źródłem
  decyzji przy `default_permissions`. `getattr()` pobiera teraz atrybuty ścieżki
  z autorytatywnego źródła zamiast z read-routingu repliki, a mount z
  `default_permissions` otrzymuje zerowy TTL atrybutów. Regresja
  `test_unlink_primary_authority_docker.py` używa read-only mountu z dodatnim
  skonfigurowanym TTL (3 s), zapamiętuje tryb `0100`, zatrzymuje replay, zmienia
  primary na `0000` i wymaga odmowy `os.access(X_OK)`. Pełny test primary/replica
  przeszedł na FOD 3.4.385 dnia 2026-10-31; artefakty:
  `artifacts/integration/unlink-primary-authority-20261030T232728Z/`.

- [x] **Ustalono i zamontowanie zweryfikowano minimalną wersję libfuse3.**
  Runtime preflight wymaga Linux 6.17.0; runtime i build wymagają libfuse3
  3.18.2 lub nowszego.
  Pełna wersja jest parsowana z `fuse_pkgversion()`, bo `fuse_version()` nie
  zawiera patch level; niespójne i nieparsowalne metadane są odrzucane.
  libfuse 3.18.2 jest pierwszym wydaniem poprawiającym upstreamowy UAF
  GHSA-qxv7-xrc2-qmfx; zweryfikowano go oraz 3.18.3 przez izolowane buildy
  upstream. Na Linux 6.17.0-41 z obiema wersjami przeszło `test_destroy.py`,
  a `test_permissions.py` przeszło na 3.18.2 i 3.18.3. Testy obejmują mount,
  unmount/remount, odczyt/zapis, uprawnienia, truncate i killpriv. Biblioteka
  systemowa hosta pozostała 3.17.4; testy uruchomiono na izolowanych
  bibliotekach i helperach, bez podmiany instalacji systemowej. Pełna macierz
  dystrybucji i innych kombinacji Linux/libfuse pozostaje poza tym zadaniem.

- [x] **Zapis i skracanie pliku respektują `RLIMIT_FSIZE` na mountcie FUSE.**
  `test_lseek.py` ustawia w osobnym procesie limit 4096 bajtów, żąda zapisu
  8192 bajtów i wymaga krótkiego zapisu dokładnie do granicy oraz dostarczenia
  `SIGXFSZ`. Kolejne `write()`/`pwrite()` i przekroczenie limitu przez
  `truncate()`/`ftruncate()` zwracają `EFBIG`; oba pliki użyte do prób
  rozszerzenia ponad limit pozostają puste. Test przeszedł
  2026-10-30 na FOD 3.4.372, Linux 6.17,
  libfuse 3.17.4 i izolowanej bazie PostgreSQL 16.15, którą usunięto po
  przebiegu. Wynik potwierdza zachowanie wywołań systemowych na mountcie FUSE;
  test nie przypisuje odpowiedzi do callbacku FOD, bo kernel może ograniczyć
  zapis przed dispatch do userspace. Pozostają przerwanie I/O przez `EINTR`
  oraz kontrolowany błąd backendu `EIO`.

- [x] **Blokady rekordów POSIX nie są dziedziczone po `fork()`.**
  `lock_backend_smoke::posix_record_locks_are_not_inherited_across_fork` ustawia
  blokadę `F_WRLCK` w rodzicu, forkuje proces z odziedziczonym deskryptorem i
  sprawdza, że `F_GETLK` dziecka nadal zwraca blokadę rodzica oraz jego PID.
  Test przeszedł 2026-10-30 na FOD 3.4.370 i jednorazowej bazie schematu 32.
  To spełnia wymóg [POSIX `fcntl()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/fcntl.html).

- [x] **Blokada rekordowa POSIX przetrwa `exec()` z otwartym deskryptorem.**
  `lock_backend_smoke::posix_record_locks_survive_exec_with_open_fd` uruchamia
  osobny proces, zakłada `F_WRLCK`, czyści `FD_CLOEXEC` i wykonuje `exec()`.
  Po execie test potwierdza, że ten sam deskryptor nadal jest otwarty, a drugi
  proces widzi blokadę z PID procesu wykonującego exec. Przeszło 2026-10-30 na
  FOD 3.4.370 i jednorazowej bazie schematu 32.

- [x] **Wielowątkowe usuwanie i odtwarzanie nazw zachowuje otwarte inode’y.**
  `mount_smoke::concurrent_threads_unlink_and_replace_many_open_file_tasks`
  wykonuje 32 zadania w 8 wątkach na 8 wątkach event loop FUSE. Każde zadanie
  otwiera plik, usuwa jego nazwę, tworzy nowy plik pod tą samą ścieżką i
  potwierdza, że stary deskryptor nadal czyta stare bajty, a nowa ścieżka
  wskazuje inny inode i nową zawartość. Test przeszedł 2026-10-30 na FOD
  3.4.369 i jednorazowej bazie schematu 32.

- [x] **POSIX blokady rekordów są związane z otwartym inode’em po usunięciu nazwy.**
  `getlk()` i `setlk()` ignorowały uchwyt FUSE i wybierały zasób blokady przez
  ponowne rozwiązanie bieżącej ścieżki. Po zdalnym `unlink()` i utworzeniu
  nowego pliku pod tą samą nazwą blokada ze starego deskryptora mogła dotyczyć
  nowego pliku. Callbacki korzystają teraz z `file_id` zweryfikowanego dla
  otwartego uchwytu. `file_locks_remain_bound_to_open_inode_after_path_replacement`
  sprawdza na dwóch mountach, że blokada starego inode’a nie blokuje nowego.
  Test przeszedł 2026-10-30 na FOD 3.4.368 i świeżej bazie schematu 32.

- [x] **Zatrucie lokalnego mutexa blokad nie może udawać braku blokad.**
  Odczyt lokalnej mapy blokad zwraca teraz `EIO`, gdy mutex jest zatruty,
  zamiast podawać pustą listę i dopuszczać dostęp bez sprawdzenia konfliktu.
  Heartbeat odzyskuje snapshot blokad i zbiór właścicieli przez `PoisonError`,
  odnawia dostępne lease w PostgreSQL i rejestruje błąd stanu. Rejestracja i
  usuwanie właściciela również odzyskują zatruty zbiór zamiast go pomijać.
  Przy zamykaniu uchwytu ścieżka PostgreSQL usuwa wszystkie lease danego
  właściciela bezpośrednio z bazy, bo zatruta mapa lokalna nie jest wiarygodną
  listą zasobów do zwolnienia.
  Deterministyczny test zatruwa oba mutexy przez panic pod guardem, wymaga `EIO`
  przy odczycie mapy, potwierdza odzyskanie wpisu do heartbeat i poprawną
  rejestrację/usunięcie właściciela. PostgreSQL pozostaje autorytatywnym źródłem
  blokad między mountami.

- [x] **`readlink()` nie może zwrócić celu nowego symlinka przez ścieżkę starego inode.**
  Callback używa ścieżki zapamiętanej dla inode, ale wcześniej sprawdzał tylko,
  czy ta ścieżka wskazuje obecnie symlink. Po zdalnym usunięciu i ponownym
  utworzeniu tej samej nazwy mógł więc odczytać cel obiektu zastępczego.
  `readlink()` weryfikuje teraz inode przez primary i pobiera cel oraz aktualizuje
  atime po niezmiennym ID symlinka. `test_stale_inode_readlink.py` na dwóch
  mountach potwierdza `ENOENT` dla starego inode i poprawny cel po świeżym
  mountcie; test przeszedł 2026-10-30 na jednorazowej bazie schematu 32.
  Test ma target `make test-stale-inode-readlink`.

- [x] **`ftruncate()` na zamkniętym deskryptorze zwraca `EBADF` bez zmiany pliku.**
  `test_lseek.py` sprawdza dokładny errno i porównuje rozmiar, mtime, ctime oraz
  payload przed i po wywołaniu. Potwierdza to walidację deskryptora po stronie
  Linux przed dispatch do FUSE. Test przeszedł 2026-10-30 na FOD 3.4.355 na
  jednorazowej bazie PostgreSQL, którą usunięto po przebiegu.

- [x] **`truncate()` odrzuca brak uprawnień z dokładnym `EACCES`.**
  `test_path_search_permissions.py` sprawdza, że użytkownik `nobody` dostaje
  dokładne `EACCES`, gdy plik nie ma prawa zapisu lub katalog nadrzędny nie
  pozwala na wyszukiwanie. `test_acl_mount_option.py` potwierdza ten sam errno
  dla ACL odmawiającego prawa zapisu. Pierwszy przebieg obu testów na FOD
  3.4.353 potwierdził exact `EACCES`. Przy dodaniu asercji zachowania payloadu
  kolejny przebieg zatrzymał się w setupie: test próbował zapisać jako root do
  pliku należącego do `nobody`, a aktywne FUSE ACL zwróciło `EACCES` przed
  właściwą asercją. Payload przygotowuje teraz `nobody`; drugi przebieg
  potwierdził `EACCES`, ale kontrola payloadu przez ścieżkę FUSE jest zabroniona
  przez testowaną ACL. W test harness poprawiono dwa błędy setupu/kontroli:
  payload właściciela `nobody` tworzy teraz `nobody`, a test bez ACL przywraca
  tryb katalogu tylko na czas kontroli bajtów; test ACL porównuje stan payloadu
  bezpośrednio w PostgreSQL, bo FUSE słusznie odmawia odczytu pod tą ACL.
  `test_path_search_permissions.py` i `test_acl_mount_option.py` przeszły
  2026-10-30 na FOD 3.4.353 w osobnych jednorazowych bazach PostgreSQL;
  asercje potwierdziły dokładne `EACCES`, niezmienione bajty i rozmiar.
  Obie bazy zostały usunięte. Pozostają testy `EINTR`, limitu rozmiaru i błędu I/O.

- [x] **Równoczesny zapis i `truncate(path)` z różnych mountów są serializowane przez lease.**
  `test_stale_writer_fencing.py` synchronizuje barierą start `pwrite()` na
  mouncie A i `os.truncate()` na mouncie B. Zapis kończy się pełnymi 4096 bajtami,
  `truncate()` zwraca dokładne `EBUSY`, lease zachowuje właściciela i token,
  a bezpośredni odczyt z PostgreSQL potwierdza rozmiar oraz payload `R`.
  Test przeszedł 2026-10-30 na FOD 3.4.354 na jednorazowej bazie PostgreSQL,
  którą usunięto po przebiegu. Pierwsza wersja asercji sprawdzała payload przez
  cache drugiego mounta i zobaczyła stare bajty; finalny test korzysta z
  PostgreSQL jako źródła prawdy. Obserwacja lokalnego cache jest objęta
  osobnym workstreamem TX03J i nie jest tu traktowana jako wynik wyścigu.
  Test obejmuje też późniejsze przejęcie lease'a z wyższym fencing tokenem i
  odrzucenie starego `ftruncate()`, zapisu oraz `fsync()`. Testy `EINTR`, limity
  rozmiaru i błędy I/O operacji `truncate()` pozostają w P05.
- [x] **Równoczesne odczyty nie uszkadzają sparse pliku przy równoległym pobieraniu bloków.**
  `mount_smoke::concurrent_read_tasks_on_sparse_file_preserve_parallel_block_ranges`
  tworzy 16 zapisanych bloków oddzielonych dziurami, ustawia 4 workerów
  odczytu i uruchamia 8 czytelników jednocześnie. Każdy sprawdza pełną
  zawartość, w tym wyzerowane luki. Test przeszedł 2026-10-30 na FOD 3.4.350
  w osobnej bazie ze świeżym schematem; tymczasowa baza została usunięta.
- [x] **Wielowątkowy admission gate odzyskuje slot po panicu zadania.**
  Testy `rust_monitor` uruchamiają zadanie trzymające jedyny slot, kolejkowany
  worker czeka na admission, a kontrolowany unwind pierwszego zadania musi
  zwolnić slot i obudzić kolejny wątek. Asercje sprawdzają końcowe liczniki
  success/failure, kolejki, transakcji i bajtów payloadu. Osobny test potwierdza,
  że drop niedokończonej obserwacji liczy zadanie jako nieudane, a RAII permitu
  zwalnia aktywny slot.

- [x] **Granica bloku przy rozszerzeniu truncate używała rozmiaru z opóźnionej repliki.**
  `prepare_truncate_boundary_blocks()` pobierało poprzedni EOF przez zwykły
  read lane. Jeśli primary rozszerzył plik, a replica nie odtworzyła jeszcze
  WAL, truncate z mountu czytającego z repliki mogło potraktować aktualne dane
  jako przestrzeń do wyzerowania. Odczyt planujący granicę korzysta teraz z
  `file_size_authoritative()`. Regresja w
  `test_unlink_primary_authority_docker.py` zatrzymuje replay, rozszerza plik na
  primary, wykonuje dalsze rozszerzenie z drugiego mountu i sprawdza wynik
  przez FUSE oraz payload bloku w PostgreSQL primary. Przeszła 2026-10-30;
  artefakt: `artifacts/integration/unlink-primary-authority-20261030T184010Z/`.

- [x] **Operacje metadanych odrzucają zamknięty deskryptor.**
  `test_lseek.py` sprawdza dokładne `EBADF` dla `fstat()`, `fchmod()`, `fchown()`
  i `futimens()` oraz potwierdza,
  że tryb, właściciel, grupa, znaczniki czasu i zawartość pliku nie ulegają zmianie. Linux
  wykonuje tę walidację przed callbackiem FUSE.

- [x] **Wątki tworzące różne typy pod tą samą nazwą mają jednego zwycięzcę.**
  Nowy test `concurrent_file_and_directory_create_at_same_name_have_one_winner`
  uruchamia 16 wyścigów między `create_new()` pliku i `mkdir()`. Sprawdza dokładnie
  jedną udaną operację, `EEXIST` dla konkurenta oraz typ i zawartość zwycięskiego
  obiektu.

- [x] **Sprawdzono `pwrite()` przy przepełnieniu maksymalnego `off_t`.**
  `test_lseek.py` próbuje dwubajtowego `pwrite()` od `OFF_T_MAX`; na zamontowanym
  FOD Linux zwraca `EINVAL`, zachowuje offset deskryptora i nie zmienia pliku.
  Źródło Linux v6.17 `rw_verify_area()` potwierdza, że ten overflow jest
  odrzucany przed callbackiem systemu plików, więc FOD nie może zmienić errno.
  POSIX matrix rejestruje różnicę wobec `EFBIG` opisanego dla limitu rozmiaru;
  to zachowanie należy traktować jako warstwę Linux, nie FOD. Test
  `make test-lseek` przeszedł na FOD 3.4.317 (2026-10-30).
- [x] **`truncate()` i `ftruncate()` aktualizują `mtime` oraz `ctime`.**
  `test_truncate_shrink_block_boundary.py` sprawdza oba znaczniki po
  skróceniu ścieżką oraz po skróceniu i rozszerzeniu przez deskryptor. Przed
  operacjami deskryptorowymi test czeka 10 ms, aby rozróżnić znaczniki przy
  mikrosekundowej rozdzielczości PostgreSQL. `make test-truncate-shrink-block-boundary`
  przeszedł na FOD 3.4.316 (2026-10-30).
- [x] **Stary uchwyt nie może wykonać `ftruncate()` po cross-mount takeover.**
  `test_stale_writer_fencing.py` odbiera lease mountowi A, pozwala mountowi B
  przejąć plik z wyższym fencing tokenem i zapisuje zwycięski payload. Następnie
  `ftruncate()` ze starego deskryptora A otrzymuje dokładnie `EBUSY`; kontrola
  przez B potwierdza, że payload i rozmiar 4096 bajtów pozostały bez zmian.
  Test `make test-stale-writer-fencing` przeszedł na FOD 3.4.315 (2026-10-30).
- [x] **Concurrent FUSE flush and `ftruncate()` could fail with false `EBUSY`.**
  A threaded mounted regression raced positioned `write()` with `ftruncate()`
  on cloned descriptors for 24 rounds. The trace showed FLUSH and SETATTR
  validating the same active write lease in separate PostgreSQL transactions;
  `pg_try_advisory_xact_lock` rejected one despite both operations having the
  valid owner token. The same race could persist a stale per-handle buffer over
  the truncate result. Persistence now waits on the per-file advisory
  transaction lock, and write/flush/fsync/ftruncate/release callbacks serialize
  access to one handle's buffered state. The 24-round test and complete
  `mount_smoke` suite passed on 2026-10-30. Cross-host write/truncate ordering
  remains in the POSIX audit.
- [x] **An unchanged-write check could trust stale replica bytes.** The write
  callback compared a candidate overwrite through the replica-routed slice
  reader and returned success without persistence if those old bytes matched.
  It now compares through `assemble_file_slice_authoritative()`. The
  two-mount regression pauses replay, verifies primary has a same-length newer
  value, writes the stale replica value from the other mount, and checks the
  resulting `data_blocks` bytes directly on primary. It passed on 2026-10-30
  in `test_unlink_primary_authority_docker.py` (artifact:
  `artifacts/integration/unlink-primary-authority-20261030T141413Z/results.json`).
- [x] **Client-version gate is enforced on a physical PostgreSQL standby, even
  before WAL replay catches up.**
  The first test attempts exposed two harness defects: Compose did not receive
  the selected `55621`/`55622` ports and fell back to `55441`, already allocated
  by `fod-deploy`; after fixing that, cleanup read `launcher.config` after
  `FODMount.stop()` cleared it. Both were fixed, and the disposable cluster
  left no containers behind. The passing `make test-client-version-gate-replica`
  run initialized schema 32 on a physical primary, then paused standby WAL
  replay before raising the primary's `minimum_client_version`. The replica
  retained the old local floor, but FOD checked the authoritative primary and
  rejected both primary-role and replica-role mounts. Replica reads still use
  the standby; normal startup requires a reachable writable primary for the
  gate, while explicit degraded read-only mode falls back to the last replayed
  replica floor. The test resumed replay and restored the original floor before
  destroying the cluster. It also verifies normal rejection and degraded mount
  when the primary is unavailable.
  Client-session creation rechecks the version floor under the same shared lock
  as the schema-upgrade gate, closing the stale-snapshot race. The passing
  regression artifact is
  `artifacts/integration/client-version-gate-replica-20261030T183039Z/`.
  Earlier harness-repair runs:
  `artifacts/integration/client-version-gate-replica-20261030T163605Z/`,
  `artifacts/integration/client-version-gate-replica-20261030T163703Z/`,
  `artifacts/integration/client-version-gate-replica-20261030T163722Z/`, and
  `artifacts/integration/client-version-gate-replica-20261030T163817Z/`.
- [x] **Schema upgrades gate new mounts and refuse active client sessions.**
  `mkfs.fod upgrade` takes a PostgreSQL session advisory lock, checks active
  client sessions, installs a database trigger against legacy session inserts,
  raises `minimum_client_version`, and sets `schema_version.upgrade_in_progress`
  in one transaction before the migration loop. Mount session registration
  takes the corresponding shared transaction lock and rechecks the marker.
  Runtime builds enforce their required schema number, so a lagging physical
  standby also rejects the new release. The final schema number, client floor,
  and cleared marker publish atomically. A failure injected into the final
  schema-version write leaves the marker set; a later upgrade clears it. The
  migration test also covers schema 31 upgrade, active-session refusal, old
  session insert rejection, and concurrent migrator refusal. `make
  test-schema-upgrade` passed; all 44 `pg_query` tests passed on an isolated
  database; `make test-client-version-gate-replica` passed for upgrade marker,
  required schema and client-floor gates on primary and physical standby
  (artifact: `artifacts/integration/client-version-gate-replica-20261030T165941Z/`).
  Operators must still unmount FOD on every host before upgrading; existing
  processes cannot be terminated by the admission gate.
- [x] **The non-adoption `copy_file_range()` path read source bytes from a
  lagging replica.** Source inspection found that its block-copy branch still
  called the ordinary replica-routed slice reader even though copy input must
  be authoritative. It now uses `assemble_file_slice_authoritative()` through
  the profiled write-input helper. A two-mount test pauses replay, changes the
  source on primary, copies a partial range from the replica-routed mount, and
  checks destination `data_blocks` on primary. It passed on 2026-10-30 in
  FOD 3.4.313 (artifact:
  `artifacts/integration/unlink-primary-authority-20261030T142313Z/results.json`).
- [x] **The sticky-bit integration test stopped before reaching FOD.** Its
  `nobody` subprocess could not traverse the mode-0700 temporary mountpoint and
  received `EACCES` before testing the intended `EPERM`. The test now makes the
  parent traversable and starts an allow-other FUSE session; the complete
  primary-authority integration script, including its permission suite, passed
  on 2026-10-30 (same artifact directory as above).
- [x] **Partial `copy_file_range()` fallback omitted the write-persistence fence.**
  On 2026-10-29, `make test-copy-file-range` returned `EBUSY`; the mounted log
  showed `__FOD_WRITE_OWNERSHIP_FENCE_REJECTED__` because the partial-copy path
  created a new destination `WriteState` and flushed it without attaching the
  destination handle's active PostgreSQL write fence. The callback now attaches
  the fence before changing the destination buffer. The dedicated mounted test
  passed after the fix on FOD 3.4.206.
- [x] **`getattr` returned stale attributes while a buffered write was flushing.**
  On 2026-10-29, isolated C7 showed PostgreSQL reporting a 65,536-byte source
  while `fstat()` on the mounted handle reported size 0. The FUSE trace showed
  `GETATTR` overlapping the final `FLUSH`; the callback read old PostgreSQL
  attributes without overlaying the mount's pending `WriteState`. The kernel
  then returned a zero-byte `copy_file_range` before reaching FOD's callback.
  `getattr` now overlays the pending local size and, when PostgreSQL reports no
  allocated blocks yet, derives `st_blocks` from nonzero pending blocks. The
  isolated two-mount C7 test and `test-df` passed with this regression covered.
- [x] **`F_SETLKW` signal interruption is supported through the fuser frontend.**
  A mounted probe held a conflicting lock on one mount, waited in `F_SETLKW` on
  a second mount, then sent `SIGUSR1` to the waiting thread. The syscall did not
  return `EINTR`; FOD continued logging the lock wait, and test cleanup found
  the mount busy. The root cause was pinned fuser 0.18.0's high-level
  dispatcher returning `ENOSYS` for `FUSE_INTERRUPT`. FOD now carries a narrow
  local fuser patch that exposes the target `RequestId`, registers blocking
  lock requests before path/lease work, and wakes only a matching pending lock
  wait. A cancelled wait returns `EINTR`; PostgreSQL still arbitrates lock
  acquisition, and an operation that acquired its lock before cancellation
  completes successfully. Mounted regression tests passed for both signal
  interruption/no-late-acquisition and ordinary cross-mount unlock handoff on
  2026-10-29. Upstream source evidence: [fuser request dispatcher](https://github.com/cberner/fuser/blob/master/src/request.rs#L136-L139).
- [x] **Writable-mount `O_APPEND` used another mount's cached inode size.**
  On 2026-10-30, FOD 3.4.309 reproduced corruption with a positive FUSE
  attribute TTL: after mount B cached size 1 and mount A appended `-A`, mount
  B's FUSE write callback received offset 1 and overwrote the result as `A-B`.
  This reproduced at `FOD_ATTR_TIMEOUT_SECONDS=1` and `=120`; separate probes
  reproduced it with buffered and direct-I/O opens. Linux v6.17 resolves
  `IOCB_APPEND` from local `inode->i_size` in `generic_write_checks_count()` before dispatching FUSE
  WRITE; the FUSE write reply cannot return a replacement offset.
  FOD 3.4.310 makes the effective FUSE attribute TTL zero on writable mounts,
  including when a positive TTL is requested; read-only mounts retain their
  configured TTL. The deterministic mounted regression
  `mount_smoke::cross_mount_append_uses_fresh_kernel_file_size` requests 120 s
  in both buffered and direct-I/O modes, checks the effective TTL, primes mount
  B's size at 1, then verifies callback offset 3 and final bytes `A-A-B` after
  both mounts append. It passed together with all 13 `mount_smoke` tests and
  `make test-create-write-ownership`, which
  confirms PostgreSQL's exclusive write lease still returns `EBUSY` to a
  competing mount. Writable metadata accesses now incur attribute
  revalidation; the entry-name TTL remains configurable. The earlier anomalous
  result came from an older 3.4.304 runtime artifact, not the current binary.
  Details: [`POSIX conformance audit`](docs/POSIX_CONFORMANCE_AUDIT.md#cross-mount-append-finding).
- [ ] **P02 POSIX directory-error evidence is incomplete.** Source/test review
  on 2026-10-30 found that `test_path_search_permissions.py` only checked the
  Python exception class for denied `mkdir()` and `rmdir()`, not exact `EACCES`.
  `test_mount_suite.py::test_replica_read_only` permitted either `EROFS` or
  `EPERM` for `mkdir()` and did not check `rmdir()`. These gaps are now covered:
  mode-000 parent operations assert exact `EACCES`, and a writable parent on a
  read-only replica asserts exact `EROFS` for both `mkdir()` and `rmdir()`.
  `test_directories.sh` also checks exact `ELOOP` when either operation
  traverses a symlink loop in its parent and exact `ENAMETOOLONG` when the final
  component exceeds `NAME_MAX`; neither operation may create or remove an
  entry in these cases.
  It now also verifies Linux VFS returns `EBUSY` when asked to remove the
  mounted FUSE root, and exact `ENOTEMPTY` for `rmdir(directory/..)`, which
  Linux v6.17 rejects before the FUSE callback. `test_permissions.py` now calls
  `unlink()` and `rmdir()`
  directly as `nobody` in a sticky directory and asserts exact `EPERM`; the
  former shell-command checks accepted any failure and did not establish errno.
  The POSIX errno matrix no longer lists sticky-directory errors for `mkdir()`,
  where that restriction does not apply.
  `test_path_search_permissions.py` also calls libc `opendir()` as `nobody` on
  a mode-000 directory and checks exact `EACCES`; this isolates the directory
  stream API boundary from `os.listdir()`/`readdir()` behavior. An earlier run
  against the shared database could not upgrade while three FOD client
  sessions were active. On 2026-10-30 the complete test passed on FOD 3.4.374
  in `release-lto` against a fresh disposable PostgreSQL 16.15 database on
  Linux 6.17/libfuse 3.17.4, including exact `EACCES` from libc `opendir()`,
  `mkdir()` and `rmdir()`. The temporary database was dropped; existing mounts
  were left untouched. The two-mount `test_readdir_snapshot_multimount.py` was
  also run on FOD 3.4.428/Linux 6.17.0-41-generic/libfuse3 3.18.2 against an
  isolated PostgreSQL 16.15 database; both `open()` and libc `opendir()` return
  exact `EMFILE` after the process descriptor limit is exhausted. The database
  was dropped and the pre-existing mounts were not touched. `ENFILE` and other
  remaining directory-operation cases keep P02 open.
  Linux v6.17 source review confirms that VFS handles `rmdir(.)` as `EINVAL`,
  `rmdir(..)` as `ENOTEMPTY`, root/mountpoint removal as `EBUSY`, and a final
  symlink as `ENOTDIR` before calling FUSE; VFS also checks destination
  existence and parent permissions. Distinguish these syscall results from
  callback-owned errors in the remaining map. Keep P02 Pending until required
  `opendir()`/`mkdir()`/`rmdir()` errors and their layer attribution are mapped.
  Source and audit:
  [`POSIX errno matrix`](docs/POSIX_ERRNO_MATRIX.md#potwierdzone-skroty-linux-vfs-v617),
  [`POSIX conformance audit`](docs/POSIX_CONFORMANCE_AUDIT.md#pending-requirement-ledger).
- [x] **P07 `access()` identity and root execute regressions.**
  Linux v6.17 `do_faccessat()` switches fsuid/fsgid to the caller's real
  uid/gid for ordinary `access()`; FUSE request headers carry those fs IDs, so
  FOD's `req.uid()`/`req.gid()` already have the required identity. Source
  review found that FOD's root bypass granted `X_OK` on regular files with no
  execute bit. The callback now returns `EACCES` for that case, matching Linux
  v6.17. `test_path_search_permissions.py` now uses a root process with real UID
  `nobody` and effective UID 0 to check that `access()` uses the real ID, and
  checks root `X_OK` both with and without execute bits. It passed on mounted
  FUSE with `default_permissions` disabled against an isolated PostgreSQL
  17.11 instance. Broader P07 gaps remain in the conformance ledger. See the
  [`permissions and identity audit`](docs/POSIX_CONFORMANCE_AUDIT.md#pending-requirement-ledger).
- [x] **P07 directory set-ID clearing is distinct from regular-file chown.**
  Linux v6.17 `chown_common()` requests set-ID clearing only for
  non-directories. FOD already preserved both directory bits, but the
  integration test allowed either `02755` or `06755`; it now requires
  exact mode `06755` after changing the group. This records Linux
  compatibility behavior, not a claim that POSIX requires it for
  directories. Broader P07 gaps remain in the conformance ledger.
  See [`permissions and identity audit`](docs/POSIX_CONFORMANCE_AUDIT.md#pending-requirement-ledger)
  and [`errno matrix`](docs/POSIX_ERRNO_MATRIX.md).
- [x] **P07 Linux file-capability clearing on chown.** `test_permissions.py`
  now installs `security.capability` with `setcap`, changes the file's
  group through FOD, and requires the xattr to be absent. The mounted
  test passed on 2026-10-30 with a disposable PostgreSQL database.
  Linux v6.17 `chown_common()` requests `ATTR_KILL_PRIV`;
  `cap_inode_killpriv()` removes the capability xattr and the FUSE
  xattr operation reaches FOD. This Linux extension is distinct from
  POSIX ACL and default-ACL behavior. Details are in
  [`SECURITY.md`](docs/SECURITY.md).
- [x] **Verify stale-inode xattr behavior on mounted FUSE.** Source review
  found that xattr callbacks resolved the request's inode through a cached path
  without confirming that the path still resolved to that inode. After a
  remote remove/recreate, this could expose or mutate the replacement object's
  xattrs. The callbacks now verify the inode against metadata resolved from
  the primary and use that xattr owner ID; if the old inode remains open after
  unlink/replacement, they use the open handle's primary-resolved `file_id`.
  Confidence is high for the code-path fix. A two-mount regression covers
  get/list, set/remove and `fgetxattr(user.*)` on an open-unlinked inode with
  ACLs enabled. It also primes a path entry on mount B, replaces that path from
  mount A, and verifies stale-path `setxattr` returns `ENOENT` without changing
  the replacement. The test passed against a temporary PostgreSQL 17.11
  instance. See [`xattr stale-path review`](docs/FOD_PRIMARY_AUTHORITY_AUDIT.md#xattr-callbacks-and-stale-inode-to-path-mappings).
- [x] **FUSE entry, attribute and negative-cache timeouts were passed as invalid mount options.**
  On 2026-10-30, setting `FOD_ENTRY_TIMEOUT_SECONDS=60` made the mount fail
  before initialization. The captured FUSE/libfuse diagnostic was
  `fuse: unknown option(s): '-o entry_timeout=60'`. Source review confirmed
  that low-level FUSE accepts the timeout values in entry/create/attribute
  replies, not as these mount options. FOD now removes the invalid mount
  options, sends entry and attribute TTLs separately, and sends a negative
  entry reply for a missing lookup when its timeout is positive. The mounted
  xattr stale-inode regression also sets all three timeout variables, verifies
  the negative lookup cache, and passed on 2026-10-30. See
  [`runtime configuration`](docs/runtime-configuration.md#entry_timeout_seconds)
  and [`xattr stale-path review`](docs/FOD_PRIMARY_AUTHORITY_AUDIT.md#xattr-callbacks-and-stale-inode-to-path-mappings).
- [x] **Align the `utimens` same-value test with POSIX ctime semantics.** On
  2026-10-30, `profile_smoke::utimens_noop` failed because it expected ctime to
  stay unchanged when `utimensat()` was given the existing atime and mtime.
  POSIX requires a successful `utimensat()` to mark ctime for update, and Linux
  v6.17 sets `ATTR_CTIME` for this operation. FOD's PostgreSQL update was
  correct; the test's “no-op” expectation was wrong. It now verifies that the
  requested atime/mtime remain unchanged and ctime does not move backwards for
  files and directories. The POSIX rule is documented in the
  [Open Group futimens specification](https://pubs.opengroup.org/onlinepubs/9799919799/functions/futimens.html);
  Linux implementation evidence is in
  [`fs/utimes.c`](https://kernel.googlesource.com/pub/scm/linux/kernel/git/torvalds/linux.git/+/refs/tags/v6.17/fs/utimes.c#44)
  and [`fs/attr.c`](https://kernel.googlesource.com/pub/scm/linux/kernel/git/torvalds/linux.git/+/refs/tags/v6.17/fs/attr.c).
- [x] **Mounted stale-inode directory replacement regression.** On
  2026-10-30, source review found that `opendir()` fetched attributes through
  the replica-routed repository without checking whether the locally cached
  path still resolved to the requested directory inode on primary. After a
  remote remove/recreate at that path, the old FUSE inode could open the
  replacement directory and snapshot its names. `opendir()` now validates the
  inode/path identity through `authoritative_entry_attrs_for_ino()` before
  taking its configured read snapshot. Review and the mounted regression then
  exposed the underlying generation alias: FUSE-created directories used their
  path as `inode_seed`, so a replacement reused the deleted object's inode
  number and passed the identity check. FUSE-created files, directories,
  symlinks and special files now use persisted generation seeds built from
  the PostgreSQL-assigned mount session ID and mount-local monotonic counter.
  Directory handles retain
  that PostgreSQL ID for initial and rewind snapshots, names-mode child
  lookups and atime updates. The two-mount regression verifies that reopening
  cannot bind a replacement to the old inode, rewind on an old open handle
  never lists replacement entries, and a fresh mount sees the replacement
  under a different inode. It passed against an isolated PostgreSQL 17.11
  instance. Snapshot reads remain on the configured read route, consistent
  with the mount's read-routing policy. Concurrent membership changes and
  large-directory cookie behavior remain in the P02 work below.
- [x] **Create callbacks use primary during replica lag.** `mkdir`, `create`,
  `mknod`, `symlink` and `link` let the serialized primary transaction decide
  destination vacancy and fetch committed reply metadata from the authority.
  The two-mount `test_unlink_primary_authority_docker.py` now pauses replica
  replay, performs each create through the replica-routed mount, confirms the
  primary committed exactly one entry while the replica still has none, and
  verifies the returned inode type. Existing paused-replica cross-type cases
  confirm competing creates return `EEXIST`. The full isolated primary/replica
  suite passed on 2026-10-30; results are in
  `artifacts/integration/unlink-primary-authority-20261030T104143Z/results.json`
  (`routed_create_success_before_replica_replay: 5`,
  `cross_type_namespace_cases: 40`).
- [x] **Finish PostgreSQL SQLSTATE-to-errno coverage for writes and namespace
  mutations.** `errno.rs` maps PostgreSQL `53100` (`disk_full`) to `ENOSPC` in
  both persistence and namespace error mappers. File-block persistence also
  maps the configured FOD quota to `ENOSPC`. `mkdir()` and regular-file,
  symlink, special-file and hardlink inserts preserve SQLSTATE; `rmdir()` and
  hardlink-promotion commands use `exec_command_params()`. Parent timestamp
  updates in those five creation transactions now also check command status and
  preserve SQLSTATE. Source review found
  that several non-success `PGresult` branches discarded the result before
  reading `conn_error()`; these now capture `result_error()` first. The
  `touch_data_object()` helper also now propagates the original command error
  instead of replacing it with a generic string. `errno.rs` unit regressions
  now verify SQLSTATE `53100` maps to `ENOSPC` in both error mappers. The mounted
  fault-injection regression and `make test-postgres-enospc` create temporary
  triggers on `data_blocks` and `directories`. SQLSTATE `53100` must return
  `ENOSPC` from `fsync()` and `mkdir()`, and `58030` must return `EIO` from
  `mkdir()` and `rmdir()`; failed namespace mutations must leave the original
  state intact. The test passed against temporary PostgreSQL 17.11. Do not parse
  localized PostgreSQL error text. See
  [`POSIX errno matrix`](docs/POSIX_ERRNO_MATRIX.md#potwierdzone-skroty-linux-vfs-v617).
- [x] **Reject malformed xattr names during list decoding.** `listxattr()` now
  returns an error instead of exposing a SQL `NULL` name as `""`, and it also
  rejects empty names. A focused regression verified both malformed rows
  against PostgreSQL on 2026-10-30.
- [ ] **PostgreSQL read failures must not appear as empty metadata.** Source
  review found that `count_file_links()`, `count_file_blocks()`,
  `path_has_children()`, startup config reads and xattr value/name reads could
  inspect `PQntuples()` without checking `PGresultStatus()`. A failed query
  could therefore appear as zero links/blocks, an empty directory, or missing
  xattrs/config. The lock-range snapshot query also treated a failed result as
  an empty lock list, which could hide conflicts; it now returns the PostgreSQL
  error instead. The affected read paths now check result status, and shared
  helpers and block-read decoders capture `result_error()` (including SQLSTATE)
  before clearing the result. FOD 3.4.463 also hardens the common scalar decoders:
  required scalar reads reject no rows, NULL, multiple rows or columns; optional
  reads accept zero rows only and reject malformed result shapes. The
  `scalar_text_decoders_distinguish_absent_null_and_malformed_results`
  PostgreSQL regression covers valid, empty, NULL, multirow and multicolumn
  results. Namespace ID lookups and path resolution now
  return an error for NULL or malformed IDs rather than treating them as a
  missing entry; xattr path lookup no longer substitutes ID 0 for a missing
  directory ID. Special-file metadata decoding now rejects NULL/malformed rows
  and frees every `PGresult` on parse failures. The
  `special_file_metadata_decoder_rejects_malformed_values` regression verifies
  valid decoding and rejects NULL, empty type or nonnumeric major/minor values.
  Broader PostgreSQL fault-injection and malformed-row regressions remain
  pending. A related
  copy-on-write reference-count parse fallback was removed: malformed counts
  now fail the transaction instead of defaulting to zero and deleting a data
  object.
  On 2026-10-31 the focused scalar-decoder regression passed on PostgreSQL
  16.15 built with `--with-blocksize=32`; runtime `SHOW block_size` returned
  `32768`. The first broad run against the configured `foddbname` found an old
  schema without `symlink_hardlinks` (`SQLSTATE 42P01`), so it was not used for
  verification and was left unchanged. A fresh disposable schema exposed
  outdated minimal test fixtures, corruption probes blocked by current SQL
  constraints, a generic missing-reference-count error, and cached libpq
  sessions not closed when `DbRepo` was dropped. FOD 3.4.464 fixes the pool
  lifecycle and missing-count handling; tests now include the alias table,
  temporarily remove and restore constraints for malformed-row probes, and
  check that dropping a repository closes its cached PostgreSQL sessions. The
  full 66-test `pg_query` suite passed serially against a fresh disposable
  schema on the verified 32KiB build. Broader PostgreSQL fault-injection and
  malformed-row regressions remain pending. FOD 3.4.465 also makes the shared
  optional namespace decoders reject multiple rows and unexpected column
  counts. `resolve_path` now fetches two candidates so it can detect rather
  than hide cross-table namespace collisions. The new
  `duplicate_symlink_namespace_rows_are_rejected` regression first reproduced
  the defect, then passed for both direct symlink lookup and `resolve_path`;
  the full 67-test `pg_query` suite passed on the verified disposable 32KiB
  database. FOD 3.4.466 also fixes a connection-pool leak when a callback
  panics while holding a PostgreSQL connection: the operation slot is released
  and the raw connection is closed before the original panic resumes. The new
  regression observed `pg_stat_activity` grow from zero to one session before
  the fix and verifies it returns to baseline afterward. Panic cleanup is
  applied to both primary and replica read callbacks. All 68 `pg_query` tests
  passed on a fresh disposable PostgreSQL 16.15 database with the verified
  `BLCKSZ=32KiB` build. FOD 3.4.467 corrects callers exposed by the stricter
  scalar decoder: a no-row flock conflict query now returns an explicit empty
  scalar, and write-lease heartbeats treat no `RETURNING` row as stale/lost
  ownership instead of propagating a missing-row error. The lock-manager tests
  also assert the serialized `lock_pid` field. Full `cargo test -p
  fod-rust-hotpath -- --test-threads=1` passed against a newly initialized
  disposable PostgreSQL 16.15 database; the custom build configuration and
  live `SHOW block_size=32768` were checked, and the database was dropped.
  FOD 3.4.477 removes `fetch_first_row_texts()` and migrates its call sites to
  `fetch_optional_single_row_texts()`, which distinguishes no row from
  malformed shape and rejects NULL fields, zero-column results and multiple
  rows. `fetch_first_column_texts()` now requires exactly one column and
  rejects NULL values. The
  `optional_row_decoder_rejects_null_and_multiple_rows` unit regression covers
  valid/absent rows, NULL, extra rows, zero columns, valid multirow
  single-column results and malformed first-column results; it passed against
  the verified PostgreSQL 16.15 `BLCKSZ=32KiB` endpoint. The generic multirow
  FOD 3.4.478 changes `fetch_rows_text()` to reject zero-column results and SQL
  NULL values while preserving valid empty strings and multirow results. Its
  consumers were audited: nullable query outputs use `COALESCE`, schema-backed
  metadata is non-NULL, and test cleanup explicitly filters nullable
  `data_object_id`. `postgres_row_decoders_reject_null_and_malformed_shapes`
  covers valid multirow/empty results, empty text, NULL and zero-column results.
  It passed against the verified PostgreSQL 16.15 build after checking its
  Docker image, `pg_config --configure` (`--with-blocksize=32`) and live
  `SHOW block_size=32768`. This closes the generic multirow NULL-conversion
  question, not the broader PostgreSQL fault-injection work.
  FOD 3.4.479 also validates exact result-column counts in the specialized
  optional file-size, single-block, block-range and xattr readers before
  returning their no-row/empty state. This prevents an unexpected zero-column
  result from masquerading as a missing file, empty block set or absent xattr.
  The first xattr test attempt against the configured admin endpoint was
  correctly refused because that database is schema 31. A new disposable
  admin database was initialized at schema 38; after adjusting older malformed
  row assertions to accept the stricter NULL error and changing a whole-row
  test comparison to `to_jsonb` (because ordinary text-row decoding now rejects
  nullable columns), all 72 `pg_query` tests passed serially on PostgreSQL
  16.15 with the verified `BLCKSZ=32KiB` build. The disposable admin database
  was dropped after the run.
  FOD 3.4.480 adds `fetch_optional_single_row_texts_exact()` and uses it for
  file-read metadata, file data-object information, link counts, path/file
  attributes, hardlink promotion and statfs snapshots. It checks the exact
  result-column contract before returning `None` for zero rows. File mode,
  symlink target, special-file metadata and file-unlinked decoders now apply
  the same check. The PostgreSQL row-decoder regression also verifies
  mismatched column counts on empty results.
  FOD 3.4.481 moves every remaining production optional-row callsite to the
  exact-column decoder, including namespace permission checks, parent ACL
  inheritance, xattr authorization and existence checks. The unqualified
  wrapper remains test-only; production callers cannot turn a zero-row result
  with the wrong column shape into an ordinary absence. The focused row decoder
  regression and all 72 `pg_query` tests passed on an isolated PostgreSQL 16.15
  database after verifying `--with-blocksize=32` and live
  `SHOW block_size=32768`; the disposable database was dropped afterward.
  FOD 3.4.482 also replaces the manual lock-range state decoder, which accepted
  extra columns and converted every SQL NULL to an empty field. It now requires
  exactly five columns and rejects NULLs; SQL explicitly coalesces the one
  nullable `range_end` field to preserve the open-ended-lock encoding. The
  decoder regression and all 10 `lock_manager` tests passed on verified
  PostgreSQL 16.15 `BLCKSZ=32KiB`; the isolated test database was dropped.
  FOD 3.4.483 replaces six namespace-insert `RETURNING` decoders that read
  only the first cell with a required single-row/single-column positive-ID
  decoder. The PostgreSQL regression rejects zero, multiple rows and extra
  columns; all 72 `pg_query` tests passed on the verified 32 KiB build.
  FOD 3.4.484 applies the same exact one-row/one-column positive-ID check to
  source, scan-run and import-plan staging upserts. Its pure regression covers
  empty, duplicate, multicolumn, zero and malformed IDs.
  FOD 3.4.485 hardens post-conflict namespace confirmation lookups: they now
  reject duplicate rows and require exact result-column counts instead of
  trusting `LIMIT 1` and the first row. Startup probes likewise require one
  row with the exact expected shape. The focused decoder regression and all 72
  `pg_query` tests passed against an isolated PostgreSQL 16.15 database after
  verifying the custom build flag `--with-blocksize=32` and runtime
  `SHOW block_size=32768`; the disposable database was dropped afterward.
  FOD 3.4.486 applies exact row-count and column-count checks to the inode and
  access-ACL reads used by transactional `chmod` and timestamp authorization.
  Duplicate or malformed rows now abort the transaction instead of selecting
  an arbitrary first record. All 72 `pg_query` tests passed on an isolated
  current-schema PostgreSQL 16.15 database after checking the custom
  `--with-blocksize=32` build and live `SHOW block_size=32768`; the disposable
  database was dropped afterward.
  FOD 3.4.487 removes `LIMIT 1` from directory-entry metadata lookup and
  returns an error if several namespace kinds claim the same parent/name.
  `duplicate_symlink_namespace_rows_are_rejected` now checks this lookup in
  addition to path and symlink resolution. All 72 `pg_query` tests passed on
  the disposable current-schema database after verifying PostgreSQL 16.15's
  `--with-blocksize=32` build and `SHOW block_size=32768`; the database was
  dropped afterward.
  FOD 3.4.488 makes hash-candidate and duplicate/import-plan report readers
  reject malformed row widths. They previously skipped short rows, which could
  silently omit data from a hash report or import-plan response. A pure
  regression verifies both short and extra-column rows fail validation.
  FOD 3.4.489 extends exact projection-width checks to the remaining indexer
  row decoders and validates single-row query cardinality for counts, source
  lookups, snapshot metadata and cleanup plan reads. The first-row helper is
  no longer used to decode SQL results in the indexer. All 35
  `fod-indexer` unit tests passed; `cargo check`, formatting and
  `make test-version` passed as well.
  Names-only directory snapshots now return `EIO` when a child metadata query
  fails instead of silently omitting that name. On 2026-10-30, the two-mount
  `test_unlink_primary_authority_docker.py` paused-replica suite opened a
  names-only snapshot, renamed `files.inode_seed` after `opendir`, and verified
  that the failing per-child PostgreSQL query surfaces as `EIO` from directory
  iteration; restoring the column and the rest of the suite passed. Evidence:
  `artifacts/integration/unlink-primary-authority-20261030T105137Z/results.json`
  (`names_snapshot_child_query_failure_returns_eio: true`). The same suite now
  injects failures into `xattrs.value` and `xattrs.name` reads, confirming
  `getxattr()` and `listxattr()` return `EIO`; evidence:
  `artifacts/integration/unlink-primary-authority-20261030T105549Z/results.json`
  (`xattr_read_query_failures_return_eio: true`). The suite now also renames
  `config.value` while starting a separate mount. Its config-dependent startup
  query reports SQLSTATE `42703` and mount startup aborts; evidence from the full
  two-mount suite on 2026-10-30:
  `artifacts/integration/unlink-primary-authority-20261030T110515Z/results.json`
  (`startup_config_query_failure_aborts_mount: true`). FOD 3.4.506 also routes
  startup recovery, schema readiness, required-config existence and upgrade
  state through the strict PostgreSQL boolean decoder, so malformed bool text
  aborts startup instead of becoming false. Formatting and hotpath compilation
  pass; no database fault injection was run for this decoder change. The
  hardlink-count query
  has a separate fault that breaks only `hardlinks.id_file` after a new file is
  visible on the replica, then checks that `stat()` returns `EIO`; evidence:
  `artifacts/integration/unlink-primary-authority-20261030T110115Z/results.json`
  (`link_count_query_failure_returns_eio: true`). `count_file_blocks()` now has
  a PostgreSQL regression in Rust using a temporary database and minimal schema:
  it verifies the valid count, renames `data_blocks.data_object_id`, and requires
  SQLSTATE `42703` instead of a zero count. The full
  `make test-rust-pg-query` suite passed on 2026-10-30 (40 tests).
  `path_has_children()` and `count_directory_children()` now also have isolated
  regressions: they verify empty and non-empty directory results, then inject an
  undefined-column error and require SQLSTATE `42703` instead of `false` or zero.
  The expanded `make test-rust-pg-query` suite passed on 2026-10-30 (41 tests).
  `count_directory_subdirs()` and `count_root_directory_children()` now also
  verify valid counts and reject an undefined-column query with SQLSTATE
  `42703`; `make test-rust-pg-query` passed on 2026-10-30 (43 tests).
  `get_file_mode_value()` previously conflated a NULL mode or malformed query
  result with a missing path. It now returns `None` only for zero rows and
  rejects NULL, empty, non-octal, or malformed result shapes. A Rust
  PostgreSQL regression checks missing/valid cases and injects empty and
  non-octal values into a file row, requiring errors to propagate. Broader
  malformed-row fault injection remains open. The isolated PostgreSQL
  regression `file_mode_lookup_rejects_null_mode_and_query_failure` now feeds
  a NULL mode and then renames the queried column; the exact lookup returns an
  error in both cases instead of reporting a missing file. The test uses a
  disposable database and passed on FOD 3.4.416.
  Namespace ID decoders also reject zero: the logical root uses a separate
  sentinel, so zero is never a valid directory, file, hardlink or symlink row
  ID. `namespace_lookups_reject_zero_inode_ids` corrupts a symlink primary key
  and verifies both direct lookup and resolved-path lookup fail instead of
  returning inode 0.
  Directory metadata decoding now rejects zero entry IDs in addition to empty
  names, unsupported entry kinds and empty inode seeds. A temporary-database
  regression feeds zero IDs and NULL name/inode-seed fields through the real
  query decoder; single-entry lookup rejects `Some(0)` instead of aliasing it
  to the logical root sentinel. A mounted regression also confirms `stat()`
  returns `EIO` when persisted `files.inode_seed` is empty.
  The Rust query suite passed on 2026-10-30 (42 tests); mounted evidence is
  `artifacts/integration/unlink-primary-authority-20261030T112245Z/results.json`
  (`malformed_metadata_fields_return_eio` includes `inode_seed`).
  Names-only directory snapshots now reject empty names instead of encoding an
  invalid empty `readdir` entry. The repository regression checks NULL names,
  and the mounted test injects an empty child name with triggers disabled only
  for that corruption transaction, then confirms `os.listdir()` returns `EIO`.
  Full mounted evidence:
  `artifacts/integration/unlink-primary-authority-20261030T112944Z/results.json`
  (`names_snapshot_rejects_empty_child_name: true`).
  FOD 3.4.333 also fixed one row-decoding edge: a root-level hardlink
  legitimately stores `id_directory = NULL`, but primary-hardlink selection
  treated that NULL as if the row were absent. Promotion could then skip the
  surviving root alias. The shared decoder now preserves the nullable parent
  while rejecting NULL/invalid IDs and empty names instead of returning
  `None`. `pg_query::root_hardlink_promotion_accepts_null_parent_and_rejects_malformed_name`
  verifies root alias selection and promotion, then injects an empty persisted
  name and confirms selection/promotion fail without changing the primary
  namespace. Broader malformed-row fault injection remains open.
  FOD 3.4.334 also changes copy-on-write source adoption to distinguish a
  missing file row from malformed size or data-object fields. Invalid values
  now fail the transaction instead of silently disabling adoption. The
  `pg_query::source_adoption_rejects_malformed_file_info_instead_of_falling_back`
  regression injects a negative persisted size and confirms that adoption
  returns an error while the destination object and size stay unchanged.
  A follow-up fault-injection test found that `fetch_xattr_value_for_owner()`
  treated a persisted SQL `NULL` payload as an absent xattr: the new regression
  initially failed with `None` instead of an error. The query decoder now
  distinguishes zero rows from a NULL payload and rejects malformed row shape;
  `pg_query::xattr_null_value_is_not_reported_as_missing_attribute` passes on a
  temporary database. Query-failure injection for the xattr remains covered by
  the mounted primary-authority suite.
  File-block read decoders had the analogous issue: a `NULL` payload for an
  existing block was skipped, making corrupt data look like a sparse zero block.
  `load_block()`, both block-range readers, and the generation snapshot now fail
  closed on NULL payloads and malformed row shapes. The
  `block_read_decoders_reject_null_payloads` regression reproduced the old
  behavior and now verifies all four query paths against PostgreSQL.
  Rename replacement also treated a missing data-object reference-count row as
  `1`, allowing it to remove a target whose `data_object_id` was orphaned. The
  operation now errors instead; `rename_replace_rejects_missing_target_data_object`
  reproduced the old deletion and verifies the source and target bindings stay
  unchanged.
  Symlink target decoding previously trimmed valid target whitespace and
  conflated SQL `NULL` or an empty target with a missing link. `readlink` now
  preserves target bytes and rejects NULL/empty targets; symlink metadata uses
  the same fail-closed validation. `symlink_null_target_is_not_reported_as_missing_or_empty`
  checks the exact whitespace case and both malformed target values.
  `file_is_unlinked()` also used to treat a SQL `NULL` flag as a nonexistent
  file. It now distinguishes zero rows from a malformed NULL or boolean value;
  `null_file_unlinked_flag_is_not_reported_as_missing` verifies the distinction
  and confirms a genuinely purged file still reports as absent. The related
  `file_link_count_for_stat()` query also treated NULL as `false`, which could
  report `st_nlink=1` for a corrupted row; it now validates the persisted flag
  before returning the link count. The same regression reproduced `Some(1)`
  for NULL before the fix and verifies that the decoder now errors.
  `count_file_links()` also used to return `1` for an absent file because its
  aggregate did not join `files`; it now returns no row and fails the count
  decode. `count_file_links_does_not_report_link_for_missing_file` reproduced
  and verifies this case. `file_data_object_id()` now also rejects a NULL
  foreign key instead of treating the row as missing, and sparse seek reports
  an error for a vanished file rather than fabricating a hole result. The
  `null_data_object_id_is_not_reported_as_missing_file` and
  `sparse_seek_does_not_treat_missing_file_as_hole` regressions reproduced
  both old results and verify fail-closed handling. `file_read_metadata()` also
  used to return a successful snapshot with an empty timestamp when PostgreSQL
  supplied NULL; it now requires exactly four columns and rejects an empty
  timestamp field. `file_read_metadata_rejects_null_timestamp` reproduced the
  empty `access_date` and verifies rejection.
  Copy-on-write persistence also used to treat a file whose data-object row was
  missing as a nonexistent file and commit a successful no-op. The lookup now
  uses a left join so an orphaned reference is an error; the regression
  `persist_rejects_orphaned_file_data_object` reproduced the false success and
  verifies that the write fails without adding blocks. `file_size()` and
  `file_size_authoritative()` also used to map NULL sizes to missing files;
  both now distinguish a corrupt value from zero rows. The regression
  `null_file_size_is_not_reported_as_missing_file` covers both query routes and
  confirms that a purged file still returns `None`.
  Other malformed/missing rows remain pending. The `open()` callback also used
  to collapse every `acl_allows()` error into `EACCES`; it now preserves backend
  `EIO` and malformed-ACL `EINVAL`, while a genuine ACL denial remains
  `EACCES`. The focused unit regression
  `open_acl_check_preserves_backend_errors_and_maps_denial_to_eacces` covers
  all outcomes. `test_acl_mount_option.py` also corrupts only the target file's
  stored access ACL in PostgreSQL, then verifies `open()` returns `EINVAL` and
  the FOD callback log reports `acl_check` with that errno. The mounted test
  passed on FOD 3.4.426, Linux 6.17.0-41-generic, libfuse3 3.18.2 and PostgreSQL
  16.15 on 2026-10-31; its disposable database was dropped. Broader PostgreSQL
  fault-injection and malformed-row regressions remain pending.
- [x] **Verify invalid PostgreSQL `flock` lease types fail closed.** Lease
  acquisition rejects requested and persisted types outside the supported
  shared (`1`) and exclusive (`2`) values. A backend regression now seeds an
  active lease with type `0` and confirms that a competing read-lock request
  returns the explicit unsupported-type error instead of granting the lease.
  The test cleans up its row after the assertion.
- [x] **Verify `F_GETLK` reports locks extending to EOF.** An unbounded
  PostgreSQL range was represented internally as `None` but the FUSE reply
  converted it to end offset `0`, truncating the reported conflict. It now uses
  the maximum `loff_t` offset supported by Linux FUSE. The mounted two-primary
  regression queried a conflicting zero-length lock through EOF and confirmed
  the returned `l_start` and `l_len` against temporary PostgreSQL 17.11.
- [x] **Verify malformed POSIX range-lease rows fail closed.** The decoder
  rejects unknown lock types, offsets outside the PostgreSQL `BIGINT`/Linux
  `loff_t` range, reversed ranges and malformed fields; a unit regression
  covers these corrupt blobs. A mounted backend regression inserts an active
  PostgreSQL range lease with an unsupported lock type and confirms `F_GETLK`
  returns exactly `EIO` rather than reporting the corrupt row as absent. See
  `rust_fuse/tests/lock_backend_smoke.rs::malformed_active_range_lease_returns_eio_from_getlk`.
- [x] **Detect cross-mount POSIX lock deadlocks.** `F_SETLKW` previously
  retried a PostgreSQL range conflict indefinitely and did not report
  `EDEADLK`. Migrations 0029 and 0030 add a PostgreSQL wait-for graph whose
  nodes use host and caller PID, because FUSE `lock_owner` values are local to a
  mount. Cycle registration and lock-range changes share one transaction
  advisory lock so independent mounts use the same ordering authority. A mounted
  regression starts two processes on separate mounts, grants them exclusive
  locks on separate files, then requests each other's file and requires one
  request to return `EDEADLK`; the other
  process is stopped by test cleanup. The regression exposed the mount-local
  FUSE lock-owner identity; `(host_name, lock_pid)` fixes that mismatch.
  `make test-locking` passed 12/12 and `make test-schema-upgrade` passed
  (2026-10-30). This global graph
  lock serializes PostgreSQL-backed POSIX range-lock updates; measure that cost
  before tuning it. `fork()`/`exec()` lock lifecycle and `ENOLCK` limits remain
  outside this fix. See P10 in the [POSIX audit](docs/POSIX_CONFORMANCE_AUDIT.md).
- [x] **Metadata projection preserves restrictive modes and rejects malformed rows.**
  The mounted `test_unlink_primary_authority_docker.py` regression creates and
  chmods a directory to `0644`, sets pre-epoch file timestamps through
  `utimensat` and checks them through `stat`, then corrupts persisted `size`,
  `mode`, `uid`, `gid` and `modification_date` one field at a time. Each
  malformed metadata row returns `EIO` from the FUSE metadata path; the test
  restores the original row after each injection. It also injects octal `10000`
  and requires `EIO`; PostgreSQL mode lookup and path/open FUSE attribute
  parsing enforce the permission-bit range `0o0000–0o7777`. Rust regressions
  cover both decoders. A follow-up source audit found metadata/ACL authorization
  decoders with separate unbounded octal parsing; all now share one POSIX-mode
  parser, and the PostgreSQL regression verifies xattr authorization rejects
  `10000` without writing the xattr. Unit and PostgreSQL tests passed. The
  mounted `10000` case is added but awaits execution
  against an isolated/up-to-date database because current schema upgrade is
  blocked by active FOD sessions. The existing unit regression verifies
  unknown special-file types fail closed. The full isolated
  primary/replica suite passed on 2026-10-30; results are in
  `artifacts/integration/unlink-primary-authority-20261030T104648Z/results.json`.
  Malformed PostgreSQL query results and missing hardlink/link-count rows remain
  tracked separately under PostgreSQL read-failure handling.
- [x] **Verify Linux FUSE device-number round trips for `mknod`.** The callback
  decodes the kernel's 12-bit-major/20-bit-minor `new_encode_dev` value and FUSE
  attrs encode it in the same format; unrepresentable database values fail
  with `EIO`. Mounted char/block-node checks covered minor values above 255 and
  the ABI boundary and compared `stat().st_rdev` major/minor; both special-node
  cases passed against temporary PostgreSQL 17.11.
- [ ] **P02 directory-stream errno needs remaining boundary verification.** Source review
  maps POSIX `readdir()` errors to the libc/ABI boundary: `EOVERFLOW` can occur
  when a wide FUSE inode number is converted to a narrower `ino_t`; Linux's
  legacy compat `getdents` has its own inode-width check, while `getdents64`
  carries a 64-bit inode. `EBADF` for a closed descriptor is rejected by the
  kernel before FUSE; FOD's `EBADF` handles only an invalid FUSE directory
  handle. POSIX `ENOENT` for an invalid stream position is conditional; FOD
  treats an out-of-range cookie as end-of-stream with a saturating conversion.
  FOD supports userspace architectures with pointer width 64 bits or greater,
  so 32-bit `ino_t` conversion coverage is outside the supported matrix. The
  mounted snapshot regression now traverses 512 initial entries and checks
  direct libc `opendir()`/`readdir()` EOF, successful `closedir()`, and `ENOENT`
  from `opendir()` for a missing path, plus `EMFILE` for `opendir()` and `open()`
  after exhausting the process descriptor limit. The mounted
  `test_readdir_snapshot_multimount.py` regression passed on FOD 3.4.398 and was
  rerun on FOD 3.4.411 in `release-lto` against a disposable PostgreSQL 16.15
  database on Linux 6.17.0-41-generic/libfuse3 3.17.4; that database was
  dropped. A former assertion required `closedir()` to return `EBADF` after
  external closure of `dirfd()`; it was removed because POSIX defines that use
  as undefined behavior (see the resolved finding above). System-wide `ENFILE` and remaining
  namespace errno checks are unverified on supported 64-bit-and-wider systems;
  do not build a 32-bit FOD target for the narrow-inode `EOVERFLOW` case.
  See [`POSIX errno matrix`](docs/POSIX_ERRNO_MATRIX.md)
  and [`POSIX conformance audit`](docs/POSIX_CONFORMANCE_AUDIT.md#pending-requirement-ledger).
- [ ] **P02 directory-stream snapshot behavior still needs ABI and descriptor checks.** FOD
  now pins an ordered PostgreSQL snapshot to each `opendir()` handle (full
  metadata by default, names when `FOD_READDIR_BATCH_METADATA=0`), uses it for
  continuation requests, refreshes it on `rewinddir()` (cookie zero
  after stream progress), binds each snapshot to the PostgreSQL directory ID
  captured at `opendir()`, and releases it in `releasedir()`. Names-mode child
  lookups and atime updates use the same ID. This prevents positional cookies
  from shifting when another mount changes directory membership and prevents
  an old handle from switching to a replacement directory at the same path.
  A two-mount remove/recreate regression now verifies an old open handle does
  not expose the replacement after rewind and that fresh lookup sees the new
  inode. `test_readdir_snapshot_multimount.py` reads in small chunks while a
  second mount removes and adds names, checks stable unaffected entries and
  duplicate-free continuation, then confirms rewind refreshes the snapshot.
  POSIX leaves visibility of additions/removals unspecified, so the test does
  not require either membership outcome before rewind. The regression uses 512
  initial child entries and checks the libc stream boundary, including `EMFILE`.
  32-bit narrow-ABI `EOVERFLOW` is outside the supported architecture matrix;
  system-wide `ENFILE` remains to be covered on supported systems. The test
  source's invalid external-`dirfd()` `closedir()` assertion was removed; the
  mounted test passed on FOD 3.4.398 against an isolated database, covering
  `open()`/`opendir()` `EMFILE` and successful `closedir()`. The first attempt
  used a stale runtime binary and was rerun after rebuilding; this was a test
  setup mismatch, not a filesystem failure. System-wide `ENFILE` and remaining
  namespace errno checks are still open.
  See [`POSIX conformance audit`](docs/POSIX_CONFORMANCE_AUDIT.md#directory-iteration-cookie-finding).
- [x] **Podniesiono wspierany baseline libfuse3 do poprawionego wydania.**
  Runtime, build `vendor/fuser`, pakiety DEB i RPM wymagają co najmniej 3.18.2;
  środowisko testowe sprawdziło także najnowsze upstreamowe 3.18.3. Zmiana
  nie dodaje brakujących callbacków `fuser`, takich jak `remap_file_range`;
  dystrybucje i pozostałe kombinacje kernel/libfuse wymagają osobnej macierzy.
- [ ] **Intermittent whole-object sharing failure in `test-df`.** On
  2026-10-29, one mounted run completed the 65,536-byte `copy_file_range()`
  with correct file contents but found different PostgreSQL
  `data_object_id` values for the source and copy (`695` and `696`). Two later
  runs passed, including one with debug logging, so the cause is not established
  and the issue is not considered fixed. Source review shows that whole-object
  adoption is skipped when either open write state has pending changes; the
  current successful-run log archive does not establish which copy branch ran.
  ASM/ELF review of the `release-lto` x86-64 PIE confirmed that symbols are
  stripped and found the copy completion/adoption log call sites, but did not
  identify a causal branch or optimization defect. `copy_file_range` now logs
  source/destination state presence and dirtiness, sizes, offsets, copy bounds,
  whole-object decision, and a declined adoption result at debug level. Re-run
  `test-df` in `release-lto` with debug logs archived; retain the failed-run log
  and PostgreSQL rows if the mismatch returns. A stale kernel-visible size was
  reproduced in isolated C7 and fixed; it could make `copy_file_range()` return
  zero before FOD handles it. The test previously archived only the second
  mount's log, after remount verification; it now also saves the first mount's
  log as `<FOD_TEST_LOG_ARCHIVE>.before-remount`, where the copy callback runs.
  Four traceable `test-df` runs on 3.4.207 (including three repeats) recorded
  `whole_object_adoption=true` and successful source-object adoption. The
  2026-10-30 repeat on FOD 3.4.322 ran ten `release-lto` debug passes; all ten
  recorded no pending source/destination write state,
  `whole_object_adoption=true`, successful adoption, and matching PostgreSQL
  source/copy object IDs. First-mount traces are retained under `/tmp/` as
  `fod-test-df-3.4.322-20261030-{debug,run2,...,run10}.log.before-remount`.
  These successful runs still do not reproduce the original mismatch; keep
  the finding open until a failure is captured with its first-mount trace and
  PostgreSQL rows.

## Previously tracked follow-ups (closed or deferred)

- [x] Document and test FOD's virtual free-inode headroom.
  - Closure note (2026-10-29, FOD 3.4.203): `statfs` reports a synthetic
    `f_ffree`/`f_favail` headroom of 1,000,000 because PostgreSQL-backed FOD
    has no fixed inode pool or separate non-privileged inode quota.
    `test_df.sh` now asserts the value and equality of both fields.

- [x] Add a mounted `statfs` failure regression for `EIO`.
  - Closure note (2026-10-29, FOD 3.4.201): the mounted integration probe
    injects an `EIO` reply in the FOD statfs callback with
    `FOD_TEST_STATFS_FORCE_EIO` and asserts `os.statvfs()` returns exactly
    `EIO`. This verifies callback-to-syscall errno delivery; it does not inject
    a real PostgreSQL failure. The injection is confined to the test-named
    environment variable.

- [x] **Classify `statvfs()` `EOVERFLOW` on the audited x86_64 ABI.**
  - Closure note (2026-10-31): Linux FUSE statfs counters are `uint64_t`; the
    mounted x86_64 ABI has 64-bit `fsblkcnt_t`/`fsfilcnt_t`, and the native
    Linux v6.17 `statfs` conversion checks count overflow only for a 32-bit
    destination field. The 32-bit compatibility ABI is outside FOD support, so
    no mounted overflow test is applicable on the audited ABI. Repeat the field
    width review before adding another ABI. Sources: [FUSE statfs ABI](https://github.com/torvalds/linux/blob/v6.17/include/uapi/linux/fuse.h#L2772-L2794), [Linux conversion](https://github.com/torvalds/linux/blob/v6.17/fs/statfs.c#L1179-L1200). See the [POSIX errno matrix](docs/POSIX_ERRNO_MATRIX.md).

- [x] **`statvfs()` did not return `EINTR` when a caught signal arrived during a blocked FUSE request.**
  - Closure note (2026-10-31, FOD 3.4.387): a mounted regression in
    `test_unlink_primary_authority_docker.py` holds an exclusive PostgreSQL
    table lock until the FUSE `statfs` query waits on it, then sends `SIGUSR1`
    to a C helper calling `statvfs()`. Before the fix the call remained
    blocked. FOD now tracks pending statfs replies and answers the interrupted
    request with `EINTR`; the integration run passed and records
    `statvfs_signal_returns_eintr=true`. This cancels the caller's wait and
    discards a later reply; it does not cancel the already-running PostgreSQL
    query. See the [POSIX audit](docs/POSIX_CONFORMANCE_AUDIT.md) and
    [FUSE requirements](docs/FUSE_REQUIREMENTS.md).

- [x] Add a mounted stale-read `ESTALE` regression for degraded read-only mode.
  - Closure note (2026-10-29): `test_pg_three_node_failover.sh` holds a file
    descriptor open on a standby, deletes only that dedicated test file row on
    primary, waits until the deletion LSN is replayed, then checks that reading
    the already-open descriptor returns `ESTALE`. The complete three-node HA
    test passed with `degraded_open_file_missing_snapshot_errno=ESTALE`.

- [x] Set the POSIX record-lock scope for FIFO objects.
  - Decision (2026-10-29): POSIX Issue 8 requires `fcntl()` record locks for regular files and makes support on other file types optional. FOD's cross-mount PostgreSQL lock guarantee covers regular files only; FIFO record locks are outside that guarantee. A Linux v6.17 probe showed the kernel accepts FIFO locks locally on each mount without calling FOD's FUSE lock callbacks: FUSE assigns FIFO `pipefifo_fops`, which has no `.lock` operation. Two FOD mounts therefore each obtained an exclusive lock on the same FIFO. This is an explicit Linux FUSE limitation, not a POSIX conformance failure; do not claim shared FIFO lock semantics. Sources: [POSIX `fcntl()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/fcntl.html), [FUSE inode setup](https://github.com/torvalds/linux/blob/v6.17/fs/fuse/inode.c), [special inode setup](https://github.com/torvalds/linux/blob/v6.17/fs/inode.c), [FIFO operations](https://github.com/torvalds/linux/blob/v6.17/fs/pipe.c).

- [x] Decide the errno for `unlink(directory)`.
  - Decision (2026-10-29): retain Linux-compatible `EISDIR` for FOD's Linux FUSE backend; POSIX Issue 8 prescribes `EPERM` when directory unlink is prohibited. The test asserts `EISDIR`, and both errno references state the platform difference. Do not claim this case is identical to the POSIX prescribed errno.

- [x] Protect one destination path against concurrent copies from independent FOD mounts or machines.
  - Closure note (2026-09-29, FOD 3.4.30-3.4.31): direct writable `create`/copy and temporary-file plus `rename`/replace are PostgreSQL-authoritative first-writer-wins paths.
  - Direct `O_CREAT`/`O_TRUNC` creation acquires destination ownership before namespace mutation. Protected rename/replace acquires namespace resources in deterministic order, re-resolves under the transaction, checks conflicting destination/file ownership and commits target replacement plus source move atomically.
  - Competing writers fail promptly with deterministic `EBUSY` and do not wait for the winner. A losing rename preserves its source and the destination; injected post-target-removal failure rolls the whole PostgreSQL transaction back.
  - Deterministic two-mount coverage includes absent and existing same-destination races, active source/destination writers, hardlink-alias ownership, symlink and directory rename/replace, same-file and literal same-path no-op behavior, root/type/descendant errors and zero ownership leaks.
  - The legacy non-transactional rename fallback and its unused direct `DbRepo::rename_*_entry()`/FFI helper API were removed after the blocking rename gates passed.
  - A crashed/disconnected owner still relies on the existing lease expiry and stale-writer fencing model; temporary-file workflows retain atomic replacement semantics.

- [x] Reconcile the QNAP FOD test database schema-admin secret or deliberately reset that dedicated QNAP database before running mounted FUSE/ACL tests there.
  - 2026-08-22, commit `43724e8`: QNAP PostgreSQL smoke and PostgreSQL-only checks passed, but the FOD schema on QNAP was version `16` with pending migrations `0017..0022`; the guarded upgrade correctly rejected the unrelated local schema-admin secret.
  - 2026-08-23: the dedicated QNAP test volume was deliberately recreated by the destructive test reset and current-schema initialization completed successfully. FOD 3.3.10 makes that operation an explicit opt-in with `QNAP_ALLOW_DESTRUCTIVE_RESET=1` instead of allowing `QNAP=1 reset` to remove the remote volume implicitly.

- [x] Optimize the PostgreSQL-backed replica read path after closing the FOD 3.2.84 strict read-only correctness fix.
  - Closure note (2026-09-08): FOD 3.4.16 through 3.4.20 removed the measured metadata, duplicate range-fetch and repeated xattr/path bottlenecks. The final read matrices no longer justify reopening this broad task. Reopen only for a new measured regression.
  - Baseline evidence (2026-08-19, Docker physical replica, 256 MiB, caches/read-ahead disabled, primary stopped before read): 4 KiB = ~8.4 MiB/s with 65,536 FOD read callbacks and 131,226 PostgreSQL operations; 64 KiB = 29.1 MiB/s with 4,096 callbacks and 8,332 operations; 512 KiB = 38.5 MiB/s with 512 callbacks and 1,162 operations; fio 1 MiB = 41.9 MiB/s but still 512 FOD callbacks and 905 PostgreSQL operations.
  - Decision: treat 512 KiB as the current effective FUSE read-callback ceiling. fio 1 MiB issues 256 userspace requests but FOD still receives 512 callbacks, so increasing the fio block above 512 KiB does not currently produce larger FUSE reads.
  - Decision: do not use one universal I/O size for read and write tuning. The same 256 MiB matrix measured writes at 36.7 MiB/s (4 KiB), 51.4 MiB/s (64 KiB), 54.3 MiB/s (512 KiB), and 18.2 MiB/s (1 MiB). 512 KiB is the best measured common point, while 1 MiB materially regresses write throughput.
  - Next measurement task: trace/profile the exact repository and SQL operations performed by one 512 KiB FUSE read callback. Determine why the final PostgreSQL `operation_count` remains roughly 1.8-2.3 operations per callback and why the 512 KiB and fio-1 MiB layouts produce different operation counts despite the same 512 callback count.
  - Next implementation task: reduce avoidable per-callback PostgreSQL round trips in `read_block_map -> repo_fetch_block_range`; prefer one range-oriented database operation when correctness permits instead of repeated metadata/map/payload operations.
  - Preserve the Docker strict-read-only regression as a mandatory gate: `operation_failures=0`, no SQLSTATE 25006/read-only DML errors, two stable reusable connections rather than per-read reconnect churn, primary unreachable before the measured read, and replica WAL replay at or beyond the captured primary flush LSN.
  - Re-run the 4 KiB / 64 KiB / 512 KiB matrix after each read-path optimization. Use the current results as the post-correctness/pre-optimization baseline and do not compare against the invalid pre-fix 209 KiB/s result as a storage baseline.

- [x] Investigate and document the mounted FOD space-accounting discrepancy where `df` reports `638M` total and `23M` used, `du -hx .` reports `45M`, while the logical sizes shown by `ls -al *` are much larger.
  - Reproduce the case before and after `sync`, flush, unmount/remount, and cache expiry. Capture byte-precise `df`, allocated and apparent `du`, `stat`, `find -printf '%s %b'`, FOD startup/runtime diagnostics, and PostgreSQL totals for logical file sizes, payload rows, data objects, extents/blocks, and metadata.
  - Determine separately what each command is expected to measure: `ls` logical file length, `du` allocated FOD storage for reachable file data, and `df` filesystem-wide used/free capacity. Check sparse ranges, shared/deduplicated data objects, extent payload sizing, deferred cleanup, dirty data not yet persisted, and stale `statfs` cache invalidation before treating the difference as corruption.
  - Define one durable accounting contract. Shared data objects must not be double-counted in filesystem-wide usage; per-file reporting must state whether it is logical or exclusive/shared physical usage; writes, truncate, unlink, rename, dedupe/object swap, GC, and remount must invalidate or refresh the relevant counters.
  - Add an integration regression using the reported shape (several large logical files with much smaller database-backed allocation) and assert the documented relationship between `ls`, apparent `du`, allocated `du`, and `df` rather than requiring all values to be identical.
  - Implementation note (2026-07-18): `FileAttr.blocks` uses the mandatory POSIX 512-byte unit and counts payload rows referenced by the file's data object, so sparse ranges are no longer reported as allocated. `df` reports unique persisted payload bytes over `data_blocks` and `data_extents`, not the physical PostgreSQL relation footprint.
  - Implementation note (2026-07-18): successful whole-object adoption through `copy_file_range` now invalidates destination read/recent-write caches and `statfs`, so replacing a destination object cannot expose stale destination bytes or retain the pre-swap payload total until TTL expiry.
  - Implementation note (2026-07-18): the mounted `test-df` regression now verifies the documented contract against PostgreSQL before and after remount: unique payload plus active reservations drives `df`, sparse ranges do not inflate `st_blocks`, and two files sharing one data object each expose attributed `du` allocation without duplicating filesystem-wide usage.
- [x] Implement a transactional PostgreSQL payload quota for `max_fs_size_bytes`.
  - Implementation note (2026-07-18): payload-producing block and extent transactions use one PostgreSQL transaction-scoped advisory lock, calculate post-write persisted payload usage against the canonical database config value, and roll back with `ENOSPC` when the limit is exceeded. This serializes the decision across independent mounts without a racy process-local preflight check.
- [x] Add a dedicated concurrent two-mount quota regression that places both writes near `max_fs_size_bytes`, proves that only one transaction commits, and verifies `ENOSPC` plus unchanged payload totals for the rejected writer.
  - Historical note: the 2026-07-18 single-mount smoke verified `ENOSPC` and complete payload rollback before the later independent two-mount regression forced advisory-lock contention.
  - Implementation note (2026-07-18): copy paths now reserve rounded target capacity in PostgreSQL before loading or mutating the destination. Two independent repository connections verify that an active reservation prevents a second reservation from consuming the same free capacity, and the mounted `copy_file_range` regression verifies pre-write `ENOSPC`, unchanged destination data, and no leaked reservation.
  - Implementation note (2026-07-18): active copy reservations are also included in the PostgreSQL `statfs` snapshot, so `df` stops advertising reserved capacity as free after the normal cross-mount cache refresh.
  - Implementation note (2026-07-18): the same snapshot now reads the canonical database `max_fs_size_bytes`; quota enforcement and `df` no longer diverge after the configured limit changes while a mount is running.
  - Implementation note (2026-07-18): persistence now revalidates and renews its reservation under the quota advisory lock. A copy running longer than the original one-hour lease can continue if capacity remains available, while an expired reservation cannot reclaim capacity already granted to another operation.
  - Implementation note (2026-07-18): forced-expiry PostgreSQL regressions now verify both successful renewal with available capacity and `ENOSPC` without payload mutation after another operation commits into the released capacity.
  - Implementation note (2026-07-18): `test-two-mount-quota` starts two independent FUSE daemons, queues both payload transactions behind the shared PostgreSQL advisory lock, and verifies one committed block, one `ENOSPC`, zero payload rows for the rejected file, and an exact one-block payload delta.
- [x] Design and implement role-aware multi-endpoint PostgreSQL routing for one FOD cluster, with several writable entrypoints and several read-only replicas, without encoding endpoint roles through list order.
  - Closure note (2026-09-08): startup role selection, runtime primary failover, WAL-gated replica reads, adaptive replica scoring and fail-closed promotion validation are delivered. Future work must be a concrete hardening gap rather than reopening this broad project.
  - Implementation note (2026-07-23): phase 4 stage 1 is complete. The opt-in four-lane mount passes create, write, sync, read, rename, stat, remove, and cleanup while all lanes still use the legacy DSN and automatic routing remains disabled. Startup now rejects initialized schemas with missing or invalid `config.block_size`, missing or invalid `config.max_fs_size_bytes`, or a missing schema-version row instead of failing on the first write.
  - Implementation note (2026-07-23): phase 4 stage 2 observability is complete for the current non-queued architecture. Shared `DbRepo` state records connection-pool pressure, operation and exact transaction timing/errors, bounded replay, payload flow, and heartbeat scheduling/execution delay. The opt-in lane mount samples current/peak process RSS and portable PostgreSQL activity, temporary-file, deadlock, memory-setting, and current diagnostics-backend memory indicators. Logical queue classification plus physical batch and completed-file throughput move to stage 3 because they require the queue/task boundary; automatic routing and concurrency tuning remain disabled.
  - Implementation note (2026-07-28): the first `fod-rust-monitor` crate now owns the PostgreSQL lane observability sampler, process RSS reading, PostgreSQL pressure logging, lane/payload metrics logging, and global payload metrics logging. `pg_lanes.rs` keeps pool orchestration and mount startup decisions; Stage 3 still needs the logical task queue, task classification, active-transaction limits, and completed-file throughput counters.
  - Implementation note (2026-07-28): `fod-rust-monitor` now also owns the public pool, payload, repository-observability, and PostgreSQL-pressure snapshot models plus the payload persist tracker. `rust_hotpath` implements `LaneObservabilitySource` for `DbRepo`, so monitor logging depends on a narrow contract instead of a concrete hotpath type. The remaining split is queue/task metrics once Stage 3 introduces that boundary.
  - Implementation note (2026-07-28): Stage 3 queue work now has neutral `fod-rust-monitor` contracts for logical task lane, operation, class, queue state, active transaction headroom, payload headroom, database batch size, and completed-file/byte throughput. These are data contracts only; no FUSE scheduling, routing, concurrency, or backpressure behavior changes yet.
  - Implementation note (2026-07-28): `LogicalTaskQueueObservability` now aggregates admitted, queued, active, completed, failed, backpressure, fairness, active-transaction, payload-in-flight, batch, throughput, and accounting-error counters in `fod-rust-monitor`. FOD 3.2.45 wires these counters into real FUSE execution while keeping scheduling observation-only.
  - Implementation note (2026-08-02): FOD 3.2.45 adds a cancellation-safe logical-task observation guard and wires real FUSE `read`, `write`, and `copy_file_range` requests into read/write/copy class counters. This first runtime slice is observation-only: no semaphore, byte-budget enforcement, backpressure, fairness policy, or endpoint routing is enabled yet.
  - Implementation note (2026-08-02): FOD 3.2.46 moves periodic logical-task reporting into `fod-rust-monitor`, starts the sampler for every FUSE mount, records a final shutdown snapshot, and combines immediate task admission plus start accounting under one mutex acquisition. Runtime limits and routing remain disabled.
  - Implementation note (2026-08-02): FOD 3.2.47 makes each logged logical-task report internally consistent by reading queue and throughput counters under one mutex acquisition. The sampler interval is configurable with `FOD_TASK_OBSERVABILITY_INTERVAL_MS` from 100 to 3600000 milliseconds, while the existing 30-second default remains unchanged. Tests trigger an immediate sample deterministically without timing sleeps.
  - Implementation note (2026-10-30): FOD 3.4.322 adds a barrier-controlled eight-thread monitor regression. All tasks remain active together while the test checks peak concurrency and in-flight payload/transaction totals, then it verifies balanced completion and exact throughput counters.
  - Historical note (2026-10-30; resolved below): the first mounted runs of `concurrent_threads_run_multiple_copy_file_range_tasks` and `concurrent_threads_create_write_and_read_independent_files` returned `EIO` on `CREATE`. Investigation identified the invalid persisted FUSE file-type bits, then the rename stress case exposed malformed recursive SQL. Both causes are fixed and the tests pass; see the completed item below for evidence and remaining test-owned mount artifacts.
- [x] Fix writable FUSE `CREATE` and threaded directory-rename regressions.
  - FOD 3.4.365 passed `mode=0o100664` from the FUSE `CREATE` request directly into the database. The shared persisted-mode decoder correctly rejects values above `0o7777`, so the post-create authoritative lookup returned `EIO` and rolled the file row back. `create()` now strips the FUSE file-type bits before applying `umask` and persisting permissions.
  - Once creation worked, the eight-thread namespace workload exposed a second defect: Rust's escaped line continuations removed SQL whitespace in the recursive directory-cycle query, producing `$1UNIONSELECT` and PostgreSQL SQLSTATE `42601`. The query now preserves newline separators.
  - The FUSE `CREATE` phase now records the failure phase; path-attribute SQL errors retain their PostgreSQL cause in logs. `FOD_TEST_CONFIG` lets Rust FUSE integration tests use a disposable PostgreSQL database. The test launcher and its FUSE daemon now share a process group so teardown does not orphan the test mount.
  - Validated in `release-lto` on Linux 6.17/libfuse 3.17.4 against fresh schema-32 databases: `concurrent_threads_create_write_and_read_independent_files` passes, including threaded create/write/read and directory mkdir/rename/rmdir; `concurrent_threads_run_multiple_copy_file_range_tasks` passes all 32 copy tasks across eight FUSE event threads.
  - Added `concurrent_parent_directory_create_and_child_entry_create_are_serializable`: 24 barrier-started races each for a child file and directory verify that child creation either succeeds after its parent exists or returns `ENOENT` before it exists, with no mismatched path state. The regression and all eight other `mount_smoke` tests matching `concurrent_` passed on FOD 3.4.373 in `release-lto` against a fresh disposable PostgreSQL database on 2026-10-30. The new test is included in `make test-fuse-multithreaded-tasks`.
  - FOD 3.4.378 adds `concurrent_threads_write_through_cloned_shared_offset_handles`: eight threads share a cloned open-file description and each append 64 distinct fixed-size records through ordinary `write()`, then the test verifies exact length and every record exactly once. `make test-fuse-multithreaded-tasks` now runs all `mount_smoke` tests matching `concurrent_` serially, including this regression. All 10 matching mounted tests passed in `release-lto` against a fresh disposable PostgreSQL database on Linux 6.17/libfuse 3.17.4 on 2026-10-30.
  - Older attempts before process-group cleanup left test-owned FUSE daemons at PIDs 558414 and 559623 under `/tmp/fod-rust-fuse-558410-1793392029505667116-concurrent-threaded-file-tasks/mount` and `/tmp/fod-rust-fuse-559619-1793392046642418526-concurrent-threaded-file-tasks/mount`; the earlier detached-but-attached path `/tmp/fod-rust-fuse-491369-1793391070227018754-concurrent-threaded-file-tasks/mount` also remains. These were not unmounted or killed.
  - Implementation note (2026-08-03): FOD 3.2.50 adds opt-in process-local active-task admission. Reads have a dedicated limit; writes and `copy_file_range` share a write-lane limit. Zero preserves observation-only behavior, while positive limits keep excess callbacks queued, count backpressure, and release capacity through a cancellation-safe RAII permit. PostgreSQL transaction permits, payload byte budgets, fairness, and routing remain separate follow-ups.
  - Implementation note (2026-08-03): FOD 3.2.51 removes the unused test-only `FodFuse::new` compatibility constructor. Production and tests now compile against the single explicit `new_with_task_settings` path, so `fod-rust-fuse` test builds are clean under `-D warnings` without suppressing dead-code diagnostics.
  - Implementation note (2026-08-03): FOD 3.2.52 makes positive logical-task limits FIFO within each process-local admission gate. Ticket order prevents later callbacks from repeatedly overtaking earlier waiters, `fairness_yields` records the first deferral behind an older ticket, and the zero-limit fast path remains unchanged. PostgreSQL transaction permits, payload byte budgets, cross-process fairness, and routing remain separate follow-ups.
  - Implementation note (2026-08-03): FOD 3.2.53 validates FIFO admission deterministically. A unit test queues eight waiters behind one held permit and verifies exact ticket order plus balanced queue, backpressure, fairness, and accounting counters. An explicitly ignored 500-waiter diagnostic benchmark reports end-to-end `notify_all` handoff cost without imposing a timing threshold on normal tests. Mixed-file FUSE benchmarking remains a separate follow-up.
  - Implementation note (2026-08-04): FOD 3.2.54 records the first five-run 500-waiter baseline and adds repeatable profiling targets. `make test-fifo-admission-analysis` runs the prebuilt monitor test binary through the baseline loop, `strace -f -c`, and `perf stat`; systems without usable perf counters fall back to `/usr/bin/time -v`. Artifacts are written outside the repository by default. The next functional step remains mixed small/large-file FUSE fairness testing.
  - Implementation note (2026-08-04): FOD 3.2.55 replaces broadcast `notify_all` handoffs with one private condition variable per queued ticket. Capacity is reserved under the gate mutex for the oldest waiter before that waiter is signalled, preventing lost wakeups and preserving strict FIFO while eliminating the 500-thread wake storm measured in 3.2.54. The apply-time analysis records the new baseline, syscall, context-switch, and RSS results for direct comparison.
  - Implementation note (2026-08-04): FOD 3.2.56 fixes resource-profile summary extraction for the leading whitespace emitted by `/usr/bin/time -v`. The profiler now writes a validated `fifo-admission-resource-summary.txt` with run lists, means, and medians for context switches, RSS, CPU, user/system time, and wall-clock time. Production admission behavior is unchanged.
  - Implementation note (2026-08-04): FOD 3.2.57 adds a real FUSE mixed-size fairness benchmark. A paced large sequential writer overlaps with many independent small-file writers under write admission limits `0`, `1`, `2`, and `4`. Positive limits must complete at least 90% of small writes before the large writer finishes, while every run verifies file size and payload integrity. JSON and Markdown summaries are written outside the repository. This validates starvation resistance at the FUSE boundary; PostgreSQL transaction and payload-byte limits remain separate.
  - Implementation note (2026-08-07): FOD 3.2.58 strengthens that benchmark into a saturation test. Eight independent large writer processes issue repeated 256 KiB writes without client pacing; prepared small writers are released into the continuing large stream. Every run uses a fresh mount, limit order rotates between repeats, and archived shutdown observability must prove concurrent baseline callbacks, `peak_queued_tasks >= 2`, and exact exercise of positive active limits. The test fails when queue pressure was not actually created.
  - Prefer explicit configuration such as `primary_hosts = 127.0.0.1:15432,127.0.0.1:15433` and `replica_hosts = 127.0.0.1:15442,127.0.0.1:15443`. Keep the existing `host` plus `port` format for single-node backward compatibility. A generic `hosts` field may be accepted only as a transitional form whose roles are discovered through `pg_is_in_recovery()` and `transaction_read_only`, never by assuming that the first two entries are primaries.
  - Reserve and consume the cluster ports from `/home/wojtek/git/config/ports/fod.env`: primaries `15432`/`15433`, replicas `15442`/`15443`. Treat multiple writable endpoints as HA/proxy entrypoints to the same authoritative primary unless the selected PostgreSQL technology provides real multi-primary conflict handling; do not silently present independent PostgreSQL primaries as one safe write cluster.
  - Split connection management into write/control/lease pools and read pools. Writes, schema changes, lock leases, session heartbeats, replay confirmation, and other authoritative operations must use a verified writable primary. Eligible read-only metadata and payload queries may use replicas, with primary fallback.
  - Define read-consistency rules before routing reads to replicas. After a local write, transaction, rename, truncate, lock transition, or object swap, either pin dependent reads to the primary or require the selected replica to reach the required WAL/replay LSN. Never trade correctness for a lower load score.
  - Add per-endpoint health and load instrumentation using measurements available to FOD: connection success/failure, connect and query latency, pool checkout wait, active/idle pool usage, server read-only/recovery role, replica replay lag/LSN, backend saturation, and recent error rate. Use bounded EWMA scoring, hysteresis, cooldown/circuit-breaker state, and structured selection logs so traffic does not oscillate between hosts.
  - Keep endpoint selection operation-aware: writes choose only a healthy verified primary; stale-sensitive reads choose the primary or a caught-up replica; stale-tolerant reads prefer the best healthy replica; maintenance and lock/session operations remain primary-only. Re-evaluate roles after reconnect/failover and preserve the existing lost-commit replay-confirmation guarantees.
  - Add a repeatable local integration cluster and fault matrix covering primary endpoint loss, replica lag, replica promotion, connection refusal, slow endpoint, pool saturation, read-after-write, lock/lease safety, and recovery without split-brain. Expose endpoint state and routing decisions through status/diagnostic commands before making balancing automatic by default.
  - Implementation note (2026-08-08): FOD 3.2.65 activates role-aware startup routing. Bootstrap propagates configured endpoint lists to Rust FUSE, all candidates are probed before pool construction, and the selected endpoint must match the requested mount role. Writable mounts stay pinned to one verified primary for read-after-write correctness. A deterministic integration test refuses the first primary entrypoint and proves successful mount/write/read through the second. Dynamic runtime failover and replica read routing remain follow-ups pending an explicit WAL/replay-LSN consistency policy.
  - Implementation note (2026-08-08): FOD 3.2.66 adds runtime primary failover for HA/proxy entrypoints that represent the same authoritative PostgreSQL primary/cluster. Replayable connection failure advances a shared target generation; stale cached connections are discarded, the next target is live-validated as writable, and the existing replay-confirmation rules remain unchanged. Replica reads remain primary-pinned until WAL/replay-LSN consistency is implemented.
  - Implementation note (2026-08-08): FOD 3.2.67 adds opt-in WAL-gated replica reads. Healthy observed replicas use a separate target set/pool. Successful primary write-lane work captures `pg_current_wal_lsn()`; stale-sensitive reads use a replica only after `pg_last_wal_replay_lsn()` satisfies the process-local barrier, otherwise they fall back to primary. Writes, control/lease and diagnostics remain primary-only; the base setting stays disabled until a real standby topology is explicitly validated.
  - Implementation note (2026-08-09): FOD 3.2.68 makes the mount-to-configuration relationship explicit. Every mount must provide `ini=/absolute/path.ini`; mount.fod validates the file and passes it to `fod-bootstrap --config`, overriding any inherited FOD_CONFIG selector. This supports independent mounts such as `/mnt/fod.db01` and `/mnt/fod.db02` backed by different PostgreSQL configurations without cwd/system-config ambiguity.
  - Implementation note (2026-08-12): FOD 3.2.69 adds adaptive replica scoring behind the existing WAL gate. Multiple replicas are ranked by replay lag plus EWMA connection/operation latency and failure penalties; lag can trigger trying another replica, hysteresis avoids flapping, two failures open a 5-second circuit-breaker cooldown, and replica-vs-primary pool pressure can cause an early primary fallback. Stale-generation failures no longer overwrite current target attribution.
  - Implementation note (2026-08-12): FOD 3.2.70 adds a fail-closed runtime primary promotion guard. Before accepting an initial guarded primary or a primary identity transition, configured entrypoints are scanned for one PostgreSQL system_identifier and one concrete writable server fingerprint. Same-server aliases are deduplicated; no-primary, foreign-cluster and multiple-writable-server states are rejected. External fencing/STONITH and in-flight operation fencing remain a later stage.
- [x] Validate PostgreSQL correctness requirements at startup and enforce session-adjustable values on every physical connection.
  - Implementation note (2026-07-23): the shared Rust runtime contract enforces UTC, `read committed`, disabled session timeouts, and standard-conforming strings. FUSE, indexer, and mkfs inspect selected `pg_settings` values and report actionable instance warnings for insufficient `max_connections`, disabled `fsync`, or disabled `full_page_writes`; unsupported PostgreSQL versions and unapplied session requirements remain fatal.
- [x] Add a deterministic local test-database restore target for the state left by `cargo test -p fod-rust-mkfs`.
  - Implementation note (2026-07-28): `make test-db-restore-local` resets only the local Docker Compose test database after strict guards reject QNAP/remote endpoints, non-default local DB parameters, and active FOD mounts or daemons.
  - Implementation note (2026-08-02): FOD 3.2.48 runs the complete mkfs suite through a local restored wrapper inside `test-integration`, attempts the guarded database restore even when the mkfs suite fails, and keeps `test-integration`, `test-all`, and `test-all-full` serial under `make -j`. A source-level regression test locks the ordering contract while standalone mkfs aliases retain their previous scope.
- [x] Make Python test venv setup incremental with `requirements-test.txt` and `$(VENV_DIR)/.fod-venv.stamp`.
- [x] Isolate fod-indexer integration smokes for parallel execution with per-test source names, temporary paths, scoped cleanup, and `test-fod-indexer-parallel-smoke`.
- [x] Consider adding `test-fod-indexer-parallel-smoke` to extended CI after it is stable on more than one host.
  - Historical note: `test-all-full` now uses `test-fod-indexer-parallel-smoke` instead of running the two component smokes only sequentially, so the extended suite exercises source-scoped indexer cleanup under concurrency without duplicating the same coverage.
- [x] Add repeatable performance profiling capture infrastructure for environment fingerprints, PostgreSQL statistics, `perf`, and optional bpftrace helpers.
- [x] Benchmark and optimize the SQL payload persistence path identified by the first local baseline: `COPY fod_persist_block_stage` plus `INSERT INTO data_blocks ... SELECT ... ON CONFLICT ...` dominated the 64 MiB `test-large-copy-benchmark` run.
  - Historical note: the first safe implementation batches binary COPY payload sends into 1 MiB client buffers for the `fod_persist_block_stage` block path. It keeps the same staging table, transaction, and replay-safe merge SQL. Local results on 2026-07-01 were mixed rather than a clear throughput win, so this should be treated as low-risk transport hardening; the server-side `COPY` plus `data_blocks` merge remains the dominant area for future SQL work.
- [x] Add a safe privileged profiling workflow or Makefile helper that uses sudo only for `perf`/bpftrace attach or system-wide capture, without running FOD workloads as root; direct `sudo perf stat -- make ...` leaves root-owned build artifacts and should remain diagnostic only. Covered by `profile-sudo-perf-stat-system` and `profile-sudo-bpftrace-syscalls-workload`.
- [x] Reassess FUSE cache, timeout, and `max_background` tuning after a FUSE-specific profile can collect `perf` or bpftrace counters on a host that allows them.
  - Historical note: `profile-fuse-sequential-io`, `profile-fuse-sudo-perf-stat`, and `profile-fuse-sudo-bpftrace-syscalls` now provide a repeatable FUSE profiling path. The first 2026-07-05 local sequential profile on commit `d55b555` captured `FOD_PROFILE_IO`, strace syscall summaries, and sudo `perf stat` system counters successfully. This short 64 KiB smoke does not justify changing FUSE cache, timeout, or `max_background`; use the same targets with larger sequential/mixed workloads before tuning.
- [x] Revisit `tests/integration/test_fio_sequential_io.sh` extent marker handling after the 2026-07-05 perf-wrapped FUSE profile printed `missing 'FOD extent PoC execution file_id='` while the overall workload still exited with status `0`.
  - Historical note: `fod_test_cleanup()` temporarily disabled `errexit` and did not restore it, so the missing marker was able to print without failing the full script. Cleanup now restores the previous `errexit` state. The fio extent assertions now verify `enable_extents=true` in the startup log instead of requiring the optional `FOD extent PoC execution` debug marker for every fio write pattern. The same pass also made the generated schema-admin password safe under `pipefail`.
- [x] Review prepared statement and connection reuse in high-call metadata lookup paths after the payload persistence path has before/after numbers; the first baseline showed path and child-entry lookups as visible but secondary.
  - Historical note: `rust_hotpath/src/pg.rs` already prepares the main read-only lookup statements per connection, including `ResolvePathRoot/Nested`, `FetchPathAttrsBlob*`, `GetHardlinkFileId`, `GetSpecialFileMetadata`, `LoadBlock`, `FetchBlockRange`, `LoadExtentBlock`, and `FetchExtentRange`; the shared connection pool reuses prepared connections across write/control lanes. The follow-up is now measurement-only through `profile-pg-metadata-top`, not a new prepared-statement refactor.
- [x] Review `rust_indexer` allocation and buffer reuse after heap or allocation profiling.
  - Historical note: `profile-indexer-alloc` captures `fod-indexer` runs through `heaptrack`, `valgrind massif`, or `/usr/bin/time -v`. The first local synthetic `source add` / `scan` / `hash --candidates-only` profile on 2026-07-04 from commit `8d90a6e` stayed in the `12.8-13.6 MiB` max RSS range, so there is no evidence-based reason to change `rust_indexer` buffers yet. Reopen this only after a larger real-source `heaptrack` or `massif` run shows allocation pressure.
- [x] Preserve optional `FOD_PROFILE_IO` summaries from Rust FUSE benchmark tests when profiling is enabled, because the current test support could remove the temporary mount log before the aggregate was visible. Covered by `FOD 3.2.1: preserve FUSE profile IO summaries` and documented in `docs/performance-profile-io-visibility-2026-07-01.md`.
- [x] Use `profile-pg-data-blocks-merge-explain`, `profile-pg-data-blocks-bloat`, `profile-pg-wal-delta`, and `profile-pg-table-dml-delta` evidence before any runtime `data_blocks` merge optimization. Current diagnostics show the live large-copy path still spending materially in server-side `COPY` and real `data_blocks` merges. The 2026-07-03 insert-heavy DML delta on commit `c5d7f24` showed `32768` inserts and `0` updates for the 64 MiB large-copy workload. The follow-up overwrite-only conflict profile on commit `1969674` showed `16384` updates, `0` HOT updates, `16384` non-HOT updates, and `16384` new dead tuples. The first safe reduction, unchanged-block conflict filtering, is implemented on commit `76867aa`: same-payload overwrite produced `0` `data_blocks` updates/dead tuples, and the temp-table reproducer removed `16384` identical conflicts through the `ON CONFLICT ... WHERE` filter. Changed-payload full-overwrite now uses data-object swap on commit `0eb2d0e`: the profiled overwrite produced `16384` inserts, `16384` deletes, `0` `data_blocks` updates, and `0` non-HOT updates.
- [x] Measure repeated full-overwrite data-object swap runs for WAL, relation growth, and dead-tuple cleanup behavior. The immediate swap path removes non-HOT updates, but still creates insert/delete churn; if this becomes the next bottleneck, evaluate delayed cleanup or object-GC policy without reintroducing changed-payload conflict updates.
  - Historical note: repeated local five-run profiles on 2026-07-04 from commit `60658e8` compared `data_object_swap_cleanup=immediate` against opt-in `deferred`. Deferred reduced hot-path WAL and avoided hot-path deletes/dead tuples, but explicit object GC then deleted `81920` block rows and generated `81920` dead tuples plus about `4.43 MB` WAL. Decision: keep `immediate` as the default and treat `deferred` as an opt-in policy for workloads that prefer shorter write transactions and can schedule object-GC maintenance.
- [x] Extend real storage-table DML diagnostics before any further `data_blocks` runtime SQL change. Covered by `profile-pg-table-dml-snapshot` / `profile-pg-table-dml-delta` for `data_blocks`, `copy_block_crc`, `files`, `data_objects`, relevant indexes, relation sizes, and by `profile-pg-top-io-wal` for local buffers plus per-statement WAL.
- [x] Use the extended DML and IO/WAL reports to collect a fresh local/QNAP baseline plus COPY buffer matrix before changing server-side `COPY` or the target-table merge.
  - Historical note: the local 2026-07-04 matrix on commit `adeaa35` covered `default`, `262144`, `1048576`, and `4194304` send buffers with DML/WAL/top-io-wal/bloat captures. QNAP was not reachable from this host (`No route to host` for `192.168.1.11:2376` and `192.168.1.11:5432`), so repeat the same `QNAP=1` matrix after network/LAN connectivity is restored.
  - Implementation note: `profile-data-blocks-copy-buffer-matrix-compare` now runs the local matrix first and then runs the QNAP matrix when `PROFILE_COPY_BUFFER_INCLUDE_QNAP=1`, or when the QNAP smoke probe passes in the default `auto` mode. A local-only `64 KiB` target smoke on 2026-07-05 from commit `ef0e782` passed and recorded DML/WAL artifacts, but it is not a full baseline.
  - Historical note: the fresh 2026-07-05 local/QNAP matrix on commit `a3076e1` completed all eight runs. Local stayed in a narrow `18.01-18.38 MiB/s` band, while QNAP improved from `2.46 MiB/s` at default to `3.18 MiB/s` at `4194304`. All runs were insert-only for `data_blocks` (`32768` inserts, `0` updates/deletes/dead tuples), so the run does not point to non-HOT update churn.
- [x] QNAP COPY-buffer retuning deferred. The current default remains unchanged; repeat the matrix only after a new measured regression or a materially changed environment, as recorded in the current plan and benchmark evidence.
  - Historical note: the 2026-07-09 local repeat sample on commit `bad53cc` stayed mixed (`14.55/18.17`, `18.43/17.48`, `17.42/17.76` MiB/s for `default` vs `4194304`). The QNAP repetition still needs a clean run; in this session `192.168.1.11:5432` returned `No route to host`.
- [x] Isolate the Cargo target directory used by `make test-locking` under `sudo`, then recheck the privileged test workflow.
  - Implementation note (2026-07-18): Cargo now compiles `lock_backend_smoke` as the invoking user in `target/test-locking`, records the exact executable from Cargo JSON output, and only runs that finished test binary through `sudo`. The privileged process no longer builds or writes Cargo artifacts.
- [x] External-unmount/session teardown checked on the current fuser/libfuse3 stack and closed without a runtime change; historical evidence is indexed in `docs/HISTORY.md`. Reopen only if a future stack reproduces a correctness or warning regression.
- [x] Implement and verify an explicit fuser `fsync` callback for per-file durability.
  - Implementation note (2026-07-28): `fsync` now reuses the existing write-state persistence path, treats `datasync` as the same PostgreSQL durability boundary because file data and metadata are committed together, and returns PostgreSQL persistence errors instead of relying on fuser's default `ENOSYS` path. `release` also stops hiding final flush failures in its FUSE reply, although close-time propagation still depends on kernel/FUSE behavior.
- [x] Remove the stale per-crate `rust_*/Cargo.lock` files after confirming that every crate resolves the repository workspace root and all supported `--manifest-path` builds use the root `Cargo.lock`.
  - Historical note: the removed lockfiles described FOD 3.0.4 and, for `rust_fuse`, fuser 0.14.0. The retained workspace lockfile is `./Cargo.lock`; `rust_indexer` already had no nested lockfile.
- [x] Compatibility diagnostics aggregation completed in FOD 3.4.27/P4; reopen only for a concrete missing consumer field, as recorded in `ROADMAP.md` and `docs/HISTORY.md`.
  - [x] Inventory current FUSE behavior, the Rust/C boundary, actual FFI consumers, direct `libpq` symbols, and the PostgreSQL storage contract in `docs/compatibility-contracts.md`; the refreshed contract now points at schema version 19, while the original migration evidence remains historical.
  - [x] Define and enforce Rust 1.85 as the verified minimum toolchain while retaining Edition 2021; workspace packages inherit the minimum, the SELinux/ACL image is pinned to `rust:1.85-bookworm`, and the stored CI definition selects the same version.
  - [x] Remove the intentionally inactive `.github/workflows/ci.yml_`; automated GitHub Actions coverage is not claimed until a new workflow is designed and enabled explicitly.
  - [x] Record the current `fuser 0.14` / ABI 7.31 correctness and performance baseline.
    - Historical note: `docs/fuse-abi-7-31-current-baseline.md` records three-run local block/extent measurements for exact and chunked copy, sequential storage, sequential/mixed/random fio, and remount durability on commit `7d9ed83`. It also freezes callback counts, SQL/WAL shape, object-sharing checks, partial extent conversion, and zero-orphan/reference/hybrid invariants for the later `fuser 0.17` comparison.
  - [x] Upgrade `fuser` from 0.14 to 0.17 for functional parity before enabling any new capabilities.
    - Historical note: the 2026-07-12 migration selects `abi-7-40` and `libfuse3` explicitly, adapts typed inode/handle/flag/error boundaries, and preserves the single-threaded session default. Workspace, mounted callback, two-mount locking, exact/chunked copy, remount, replay, schema-v17 ownership, fio, and profiled strace gates passed without enabling a new capability.
  - [x] Report the available subset of negotiated FUSE protocol limits and capabilities without forking `fuser` only for diagnostics.
    - Historical note: FOD now reports fuser 0.17.0, userspace maximum 7.40, kernel and effective negotiated protocol, kernel-available capabilities, and FOD-requested/enabled/unsupported capabilities. Publicly unreadable final request limits are explicitly reported as unavailable; the 2026-07-12 host negotiated 7.40 from kernel protocol 7.44.
  - [x] Repeat the ABI 7.31 workload matrix after the upgrade and retain the migration only if correctness and storage ownership invariants remain intact.
    - Historical note: `docs/fuser-0-17-migration-baseline.md` compares three-run exact/chunked/sequential/fio/remount block and extent profiles from commit `522b1b5` with the frozen `7d9ed83` baseline. Callback, DML, copy, durability, and schema-v17 ownership shapes stayed intact; repeated noisy exact/sequential samples did not confirm a material regression, so the migration is retained.
  - [x] Inventory protocol 7.32-7.40 capabilities with separate protocol, kernel, fuser API, FOD relevance, risk, and benchmark decisions.
    - Historical note: `docs/fuse-protocol-7-32-7-40-capabilities.md` records that fuser handles `INIT_EXT/flags2` internally, exposes the open/init flags, but has no high-level `SYNCFS`, `TMPFILE`, or `STATX` callbacks. `FOPEN_NOFLUSH` is unsafe while close-time errors depend on `flush`; parallel direct writes require a separate multi-thread ordering design; mmap and passthrough do not fit the current coherence/storage model.
  - [x] Probe mounted `SYNCFS`/`TMPFILE`/`STATX` syscall behavior without enabling a production flag.
    - Historical note: on 2026-07-13, commit `aa77738`, kernel `6.17.0-40-generic`, and negotiated protocol 7.40, `syncfs()` returned success, `O_TMPFILE` returned `ENOTSUP`, and `statx()` returned basic metadata matching `os.stat()`. The namespace remained clean and the known file remained readable. These are syscall-visible fallback observations, not evidence that FOD implements the missing fuser callbacks or a filesystem-wide durability contract.
  - [x] Keep every future production capability experiment isolated to one capability per commit with an explicit semantic contract, correctness gates, and a workload-specific benchmark.
    - Decision: this is a durable project rule, not an implementation backlog item. Future capability work still needs its own TODO entry and one-capability commit scope.
  - [x] Classify `fod-rust-hotpath` as an internal Rust `rlib` and remove the unused installed `cdylib`.
    - Historical note: the 2026-07-13 audit on commit `e32853b` found that `fod-rust-fuse` and `fod-rust-indexer` consume the crate through Cargo, while no FOD binary linked `libfod-2.so`, no running process mapped it, no public C header existed, and the only apparent loader references were `ctypes.CDLL(None)` calls for libc in a FUSE syscall test. The broad FFI module remains internal source/test code and is not a public ABI. Reopen a versioned C ABI only for an identified external consumer, with opaque handles and an allocation contract that does not reconstruct forgotten vectors from pointer/length alone.
    - Current note: the new `libfod.so` plus `libfod.h` is a separate command-surface aggregate over installed FOD commands. It does not revive the old hotpath shared library, and FOD tools do not depend on it at runtime.
  - [x] Add `libpq` client/server runtime version diagnostics without replacing the current PostgreSQL architecture.
    - Historical note: both manual `libpq` bindings now declare `PQlibVersion()` and `PQserverVersion()`. `mkfs.fod status` reports the runtime client and server numbers plus the server's `SHOW server_version` string; `fod-rust-fuse` records the same data once during startup through the existing pooled repository connection. Major-version relation is informational only, and compatibility is reported as `connected`; no new version gate or support-range claim is introduced.
  - [x] Write the storage-format versioning ADR before adding any new format marker or schema column. Accepted in `docs/adr/storage-format-versioning.md`; the decision distinguishes schema shape, minimum client release, and payload format.
  - [x] Inventory the existing compatibility diagnostics and keep their individual trust boundaries explicit before aggregation.
    - Tracking note (2026-09-08): the remaining aggregation work is represented by the parent task above; this nested duplicate is closed.
  - [x] Revisit `fuser 0.18.0` compatibility as a separate capability-maintenance pass.
    - Implementation note (2026-07-28): FOD now depends on `fuser 0.18.0` with explicit `libfuse3`. The `0.18.0` crate no longer exposes the old `abi-7-*` Cargo feature switches, so protocol support is treated as a runtime negotiation fact rather than a compile-time feature flag.
- [x] Deliver the `Storage Engine v2` project described in `docs/storage-engine-v2-plan.md`, preserving 4 KiB logical blocks and the default block path while making large sequential physical persistence bounded and opt-in.
  - Phase A: add bounded extent planning and payload rows, then pass the local and QNAP benchmark gate.
    - Completed: bounded planning, bounded payload rows, the startup-only `extent_target_bytes` setting, and peak-payload diagnostics are implemented with a 1 MiB default. Three-run local and QNAP 64 MiB matrices passed, as did the all-workload local smoke. Extents remain opt-in because mixed/random and fio write still regress.
  - Phase B: add a sequential segment builder and direct segment persistence only after Phase A proves useful.
    - Gate status: local and QNAP evidence is positive for bounded extents, so Phase B may start without changing the default storage path. Keep 1 MiB as the balanced opt-in default; QNAP's best isolated throughput at 256 KiB does not justify a backend-specific default.
    - Completed implementation: `WritePayloadState`, `BlockOverlay`, and bounded `SequentialSegmentState` handle new empty-file writes from offset zero. Gaps, backward writes, existing-file writes, and merged handle state downgrade to the block overlay. Eligible segment payloads now move directly into `PersistExtentRow` without rebuilding 4 KiB block vectors; failed repository calls restore the owned segment state for retry.
    - Measured gate: the repeated local 64 MiB large-file run improved the 1 MiB extent path from the earlier `94.51 MiB/s` bounded baseline to `98.81 MiB/s`, while payload preparation fell from roughly `32 ms` to `12.33 us` and physical storage remained `64 data_extents` rows. This is sufficient to close the direct-persistence preparation bottleneck without changing the default path.
    - Historical constraint: the pre-ABI-7.31 repeated 64 MiB large-copy run was slower on extents (`14.07 MiB/s`) than blocks (`18.50 MiB/s`) because the kernel bypassed the implemented `copy_file_range` callback. After enabling ABI 7.31, exact whole copies adopt the data object and the corrected chunked matrix averages `26.68 MiB/s` on extents versus `17.74 MiB/s` on blocks.
  - Phase C: classify persistence semantics and add replay-confirmed, append-only persistence for new sequential objects.
    - Completed: `PersistWriteClass` separates `NewObjectSequential`, `ExistingObjectPatch`, and `TruncateOnly`, and eligible direct segments now use one replay-confirmed append-only transaction. The transaction creates a replacement data object, copies bounded extent rows directly, atomically swaps file ownership, adjusts reference counts, follows immediate/deferred cleanup policy, and persists a durable request token for lost-commit confirmation.
    - Correctness gate: body-disconnect and commit-disconnect replay, shared-object detach, hardlink preservation, full overwrite, CRC rows, immediate/deferred cleanup, remount durability, and the complete storage/FUSE regression gate passed locally on 2026-07-11.
    - Measured gate: the repeated 64 MiB large-file run averaged `94.55 MiB/s` on append-only extents versus `46.39 MiB/s` on blocks, with 64 physical extent rows and about `1.03 MB` mean WAL. The later ABI 7.31 copy measurements supersede the earlier fallback-path copy result. Extents remain opt-in because mixed/random workloads still regress.
  - Phase D: completed. `docs/adr/storage-object-segment-manifest.md` defers a manifest because exact whole-file copies now adopt the source object and corrected chunked extent copies outperform blocks locally. Reopen only for a measured partial-clone, patch-amplification, chunk-dedupe, snapshot, compression, or GC requirement.
  - Ownership cleanup: completed in schema version 17. `data_blocks`, `data_extents`, and `copy_block_crc` are owned only by `data_object_id`, object deletion cascades to payload rows, and runtime/indexer cleanup no longer rewrites representative file IDs. The base schema and upgrades from versions 1 and 16 reach the same final shape without a dual-path fallback.
- [x] Revisit cleanup in `rust_fuse/tests/root_permissions_smoke.rs` when the full Cargo suite is run without root.
  - Historical note: root-mounted test filesystems now use an explicit `sudo -n umount` from an unprivileged test process, verify that the mount disappeared, and retain the same operation as a `Drop` fallback. The full unprivileged FUSE suite no longer leaves this mount active.
- [x] Build a safe EXPLAIN/fillfactor clone experiment for `data_blocks` merge variants without touching real `fod.data_blocks` runtime data.
  - Historical note: `profile-pg-data-blocks-merge-fillfactor-explain` uses temporary clone tables and confirmed real `fod.data_blocks` row count stayed unchanged. On the local 2026-07-04 run, heap fillfactor `100` produced `0` HOT updates for changed conflicts, `90` produced `1800`, and `75` produced `5380`, with larger temp relation sizes.
- [x] Detect single-node vs read-only replica mode early and let runtime choose the appropriate lock strategy before mount. Handled in `rust_fuse/src/startup.rs` via `effective_read_only()` and `lock_settings(read_only)`.
- [x] Keep `workers_read` and `workers_write` constrained to the cases where they really help: disjoint read gaps and segmented copy operations, not small contiguous fetches. Read-side gating now goes through `rust_hotpath::read_missing_range_worker_count`; write-side gating already goes through the shared hot-path worker planner.
- [x] Primary mounts keep the PostgreSQL lease backend. Covered by `rust_fuse/tests/lock_backend_smoke.rs::primary_uses_pg_leases_and_replica_stays_memory_backed`.
- [x] Replica mounts stay on the in-memory lock backend. Covered by the same smoke.
- [x] `auto` resolves to the PostgreSQL lease backend on a writable primary DB. Covered by `rust_runtime/src/lib.rs::resolves_auto_and_replica_lock_roles`.
- [x] Test auto role on a recovery/read-only DB, if easy to simulate. Covered by `tests/integration/test_runtime_profile.py` using a temporary standby container created from `pg_basebackup`, plus the existing `FOD_RUST_FUSE_READONLY=1` read-only mount branch coverage.
- [x] Repository-level lease expiry is already covered by `rust_hotpath/tests/lock_manager.rs::expired_flock_lease_is_pruned_on_reacquire`; keep the remaining follow-up focused on mount/process end-to-end expiry.
- [x] Add an end-to-end lease-expiry regression between two independent mounts or processes so stale PostgreSQL leases are validated at the mount layer, not only in repository tests. Covered by `rust_fuse/tests/lock_backend_smoke.rs::primary_lease_expiry_allows_second_mount_reacquire`.
- [x] `FOD_ATIME_POLICY` is covered by `tests/integration/test_atime_policy.sh`.
- [x] `FOD_SYNCHRONOUS_COMMIT`, `FOD_PG_VISIBLE_PATH`, and cache TTL overrides are covered by `tests/integration/test_runtime_profile.py`.
- [x] SELinux mount label contexts (`FOD_SELINUX_CONTEXT`, `FOD_SELINUX_FSCONTEXT`, `FOD_SELINUX_DEFCONTEXT`, `FOD_SELINUX_ROOTCONTEXT`) are covered by `tests/integration/test_runtime_profile.py`.

### FOD Runtime / FUSE Refactor Follow-ups

- [x] Confirm a fully green CI after the `DbfsFuse` -> `FodFuse` rename.
- [x] Audit tests and user-facing messages for remaining `FodFuse` / `fod-*` vs legacy `DbfsFuse` / `dbfs-*` names where the legacy name is not intentional.
- [x] Decide whether `AGENTS.md` needs any extra clarification after the extent PoC started.
- [x] Verify the benchmark runner covers the full benchmark target set and keeps regression tests separate from benchmarks.
- [x] Add fio-based coverage for sequential read/write and extent PoC paths, keeping those cases separate from the Rust unit and integration suites.
- [x] Add a compatibility check for POSIX and FUSE semantics so the filesystem behavior stays aligned with the expected mount contract.
- [x] Verify that the `extents` runtime profile only flips `enable_extents` and leaves the rest of the baseline unchanged.

### Live Runtime Change

- [x] Add a runtime snapshot helper in `rust_runtime` for the live-reloadable tuning subset so future change commands can diff current vs requested state.
- [x] Define the live-reloadable subset explicitly: profile, logging, cache TTLs, read/write worker tuning, and other pure knobs that do not require remount.
- [x] Keep mount-only semantics startup-only for now: `read_only`, `default_permissions`, `lazytime`, `sync`, `dirsync`, `use_fuse_context`, `fopen_direct_io`, `fuse_writeback_cache`, SELinux/ACL mount labels, and lock backend selection.
- [x] Decide the command surface and auth for `fod.change --password`, and persist the canonical reloadable snapshot for the next runtime stage. The command surface now also includes `--get KEY` and `--list` inspection helpers over the effective reloadable snapshot.
- [x] Teach the running FUSE process to consume the stored reloadable snapshot without remount.
- [x] Apply live config updates without remount for the safe subset and reject unsafe keys with a clear error.
- [x] Add end-to-end tests once the reload transport exists, covering both accepted live updates and rejected mount-only changes. Covered by `tests/integration/test_runtime_reload.py` and `make test-runtime-reload`.

### Obszary do rozwoju

- [x] Dodać pełniejszy replay in-flight SQL po błędach.
  - Historical note: `transactional_replay_confirmed()` now centralizes the durable probe pattern for request-token-backed and natural-key-confirmed transactions, and it now also short-circuits `set_file_size()` and `adopt_source_data_object()` on durable row state. `lock_lease_request_tokens` makes flock-lease grant/deny outcomes durable, and `sql_is_replayable_command()` now covers the remaining create-entry inserts plus the `data_objects` reference-count updates that were still missing from the replayable set. The replay smoke and lock-manager suites passed after the refactor.
- [x] Review `fod-indexer` CLI ergonomics after manual use; keep the explicit `--source` contract if that remains the intended API, but consider clearer examples or a positional alias if users keep trying the old style. Added positional source shorthand for `scan`, `hash`, `plan-import`, and `materialize` while preserving `--source`.
- [x] Usprawnić automatyczne sugerowanie nazw źródłom w `fod-indexer source add`, bez usuwania `--name`.
  - Historical note: local sources default to the current hostname; SMB/QNAP try to infer the remote host or IP from the mounted share; ADB prefers the device serial from `ANDROID_SERIAL`, `ADB_SERIAL`, `ADB_DEVICE_SERIAL`, or `adb devices`; GitHub sources try the git remote slug or repository name. Explicit `--name` still overrides the suggestion.
- [x] Rozdzielić adaptery path-backed od przyszłych crawlerów SMB/QNAP/ADB/GitHub.
  - Historical note: the source kinds and naming heuristics are in place, scan/hash/plan/materialize still operate on mounted or mirrored filesystem roots, and the adapter boundary is now explicit in `docs/fod-indexer.md`.
  - Decision: current kinds stay path-backed/mirrored/export-backed through filesystem roots; none of them gets a direct remote crawler in the shared core. Future non-path-backed kinds should arrive as separate adapter projects.
- [x] Plan implementacji ioctl:
  - [x] Najpierw `FIGETBSZ`. Zaimplementowane w `rust_fuse/src/fs.rs` jako odpowiedź oparta o bieżący `blksize`.
  - [x] Potem `FS_IOC_GETFLAGS`. Na razie zwracane jest neutralne `0`, bo flags nie są jeszcze trwale przechowywane.
  - [x] `FS_IOC_SETFLAGS` przyjmuje teraz tylko `0` jako bezpieczny no-op; inne flagi dostają `EOPNOTSUPP` do czasu decyzji o trwałej polityce.
  - [x] `FS_IOC_FSGETXATTR` zwraca teraz wyzerowany `fsxattr`, a `FS_IOC_FSSETXATTR` przyjmuje tylko zero/no-op do czasu decyzji o trwałej polityce xflags.
  - [x] Ustalić status `FICLONE` i `FICLONERANGE` dla standardowego Linux FUSE.
    - Historical note (2026-10-29): Linux v6.17 dispatches these VFS remap ioctls through `remap_file_range`, which FUSE does not expose. FOD's unreachable userspace branches were removed; `test_ioctl.py` now checks that the kernel rejects `FICLONE` before userspace and leaves the destination unchanged. Clone ioctls remain unsupported unless a kernel-supported FUSE route is added. See [`FOD_PRIMARY_AUTHORITY_AUDIT.md`](docs/FOD_PRIMARY_AUTHORITY_AUDIT.md#remaining-source-level-review-items).
- [x] Zaprojektować pełną politykę mount-label SELinux. Celowo zamknięte jako non-goal: FOD używa hostowego modelu SELinux; na Rocky Linux 10.2 wspierany kontrakt to `fusefs_t` plus polityka domen, a nie per-inode `security.selinux` relabeling na zwykłym FUSE.

### PostgreSQL tuning and ingest hardening

- [x] Persist the reference QNAP PostgreSQL server profile in the repository.
  - Implementation note (2026-09-08, commit `f2b3d7d`): `QNAP=1` now selects
    the 8 GB RAM / 2 CPU / HDD profile through Make/Compose. All 15 tuned
    PostgreSQL parameters were live-validated with `source = command line`;
    the earlier `ALTER SYSTEM` duplicates were removed and
    `postgresql.auto.conf` contained no remaining profile entries.
  - Decision: the QNAP preset is a deployment profile, not a universal
    PostgreSQL requirement for FOD. Explicit command-line benchmark overrides
    remain supported, while the local `QNAP=0` path retains PostgreSQL defaults.

- [x] Expose server-side WAL/checkpoint/planner/autovacuum knobs in the Docker Compose and Makefile presets so local and QNAP benchmark runs can A/B the same config without editing container images.
  - Historical note: the compose stack now accepts optional `POSTGRES_*` tuning env vars and `make qnap-config-show` / `make postgres-config-show` print the resolved preset; `make postgres-benchmarks-wal-preset` and `make postgres-benchmarks-planner-preset` now apply the shared tuning profiles to both local Docker and QNAP without editing container images, and the benchmark A/B baseline is recorded in `BENCHMARKS.md`.
  - Historical note: the 2026-06-27 short local/QNAP smoke on commit `a7504c6` confirmed the knobs can be A/B-tested without editing images, but it stayed checkpoint-free (`checkpoints_req=0` everywhere), so `max_wal_size` and `checkpoint_timeout` still need a longer pressure test before they can be judged.
  - Historical note: the 2026-06-28 local-only long smoke on commit `e66e66c` finally crossed the checkpoint boundary at `PG_WAL_PRESSURE_COUNT=10000`. The baseline and `max_wal_size=8GB` runs both produced a timed checkpoint, while `checkpoint_timeout=15min` and `checkpoint_timeout=30min` moved the checkpoint to the requested path instead. That makes the 5-minute timeout the first knob to address on this workload, and it suggests a separate size-bound pass is still useful if we want to isolate `max_wal_size` further.
  - Historical note: the 2026-06-28 local-only max-WAL sweep on commit `be642a6` isolated that size cap. With `POSTGRES_CHECKPOINT_TIMEOUT=30min`, the current 1 GB-style `max_wal_size` still forced two requested checkpoints at `1.29 GB` of WAL, while `POSTGRES_MAX_WAL_SIZE=8GB` removed both requested and timed checkpoints on the same workload. That makes the default size cap the remaining checkpoint trigger once timeout has already been relaxed.
- [x] Move indexer metadata writes onto staging + `COPY` + set-based merge, starting with `index_files`, `index_file_hashes`, `index_duplicate_sets`, and import plan entries.
  - Historical note: `index_sources`, `index_scan_runs`, `index_import_plans`, `index_files`, `index_file_hashes`, `index_duplicate_sets`, and `index_import_plan_entries` now write through staged temp tables with `COPY` plus set-based merge helpers; `materialize` plan-entry writes use the same path too, while the streaming payload collection remains a separate follow-up.
- [x] Stream `materialize` payload collection instead of buffering whole files in memory; keep fully replay-safe batch import as a separate design.

### Direct I/O Microscope

- [x] Add fine-grained hot-path profiling around `read_block_map()`, `fetch_block_range_chunk()`, `fetch_block_range_parallel()`, `assemble_read_slice()`, `update_write_buffer()`, `flush_write_state()`, `prepare_persist_rows_from_block_plan()`, `prepare_persist_extent_rows_from_extent_ranges()`, `clear_read_cache_for_file()`, and the cache / write-state clone paths so `FOD_FOPEN_DIRECT_IO=1` can be used as a stress microscope instead of a production mode.

## FOD 3.0.9 — fod-indexer MVP

- [x] Add `fod-indexer` crate/binary.
- [x] Add indexer metadata schema.
- [x] Implement local source registration.
- [x] Implement local filesystem scan.
- [x] Implement staged duplicate detection.
- [x] Implement dry-run import plan.
- [x] Document import/materialization phase.

## FOD 3.0.9 — fod-indexer follow-up

- [x] Add materialization/import outside dry-run.
- [x] Wire `fod-indexer` into `Makefile`.
- [x] Add a smoke test for `fod-indexer materialize` that covers duplicate payload reuse, unique payloads, and zero-size imports.
- [x] Add `make test-fod-indexer-smoke` as the canonical Makefile entrypoint for the materialize smoke.
- [x] Scope `fod-indexer plan-import` by source with explicit `--source` and `--all-sources` flags, and cover both modes in smoke tests.
- [x] Add cleanup for failed indexer materialization via `fod-indexer cleanup-failed --plan <id>`, with smoke coverage for duplicate payload reuse and zero-size files.
- [x] Align the remaining `fod-indexer` integration smokes with the zero-length skip contract so stale `empty.txt` assertions do not keep documenting the old pipeline shape.

## FOD indexer: dalszy plan dla Codex

- [x] Utrwal granice miedzy core engine a adapterami zrodel. `fod-indexer` ma zostac wspolnym silnikiem indeksowania, a nie zbiorem osobnych crawlerow; wszystko, co da sie przedstawic jako lokalny katalog, mount albo mirror, powinno przechodzic przez jeden path-backed flow.
  - Historical note: source registration, browsing, list/remove, and source discovery now live in `rust_indexer/src/source_registry.rs`, while `rust_indexer/src/scan.rs` keeps the scan path focused on walking and persisting indexed files.
- [x] Wydziel model zdolnosci zrodla i trzymaj go osobno od samego skanu. Dla kazdego `source kind` doprecyzuj metadane takie jak `path_backed`, `readonly`, `mirror_required`, `needs_export` i `direct_crawler_possible`, zeby sposob pobrania danych byl deklaratywny.
  - Historical note: implemented in `rust_indexer/src/capabilities.rs` and surfaced through `SourceKind::capabilities()`, with `source add` / `source list` now printing the profile.
- [x] Ustal polityke dla `local`, `qnap`, `smb`, `adb` i `github`. Domyslnie maja byc path-backed albo mirrored, a direct crawler tylko wtedy, gdy naprawde nie da sie tego sensownie sprowadzic do katalogu.
  - Historical note: `local` is path-backed; `smb` / `qnap` are mirrored; `adb` / `github` are export-backed. The CLI now prints the policy alongside the capability profile.
- [x] Dopnij nazewnictwo i rejestracje zrodel do modelu capabilities. Heurystyki nazw maja pozostac pomocnicze, ale `--name` musi zostac jawna nadpiska; nazwa nie moze ukrywac, czy zrodlo jest mounted, mirrored, czy tylko importowane do katalogu roboczego.
  - Historical note: `--name` remains the explicit override, and the capability profile is now shown alongside source registration and listing so the kind stays visible.
- [x] Rozszerz testy integracyjne o scenariusze miedzyzrodlowe i adapterowe. Sprawdzaj osobno lokalny mount, mirror/backed source, cleanup po zniknieciu zrodla, ignorowanie hidden/cache paths oraz stabilnosc import/materialize przy kilku `source kind`ach.
  - Historical note: `tests/integration/test_fod_indexer_source_kinds.py` now covers local, smb, and github roots, browse filtering for hidden/cache trees, source-scoped scan/hash/materialize flows, and cleanup after a source root disappears.
- [x] Doprecyzuj dokumentacje FOD jako wspolnego silnika indeksowania. Wprost zapisz, ze `fod-indexer` ma byc wspolnym core dla `scan/hash/dedupe/plan/materialize/cleanup`, a `msfind` ma korzystac z tego rdzenia zamiast implementowac drugi, podobny pipeline.
  - Historical note: `docs/fod-indexer.md` now states that `fod-indexer` is the shared indexing core for registration, scan, hash, duplicate detection, planning, materialization, and cleanup, and `docs/msfind-fod-indexer-requests.md` now frames requests as gaps against that shared core instead of a parallel pipeline.
- [x] Zostaw w backlogu tylko to, czego realnie brakuje: decyzje, ktore `source kind`y beda kiedys potrzebowaly direct crawlerow, oraz czy maja byc mirror-only czy native API adapters. Nie rozszerzaj core o protokoly, jesli nie ma konkretnego, nieobslugiwnego jeszcze przypadku.
  - Historical note: the current kinds now have an explicit boundary in `docs/fod-indexer.md`, and none of them gets a direct remote crawler in the shared core. Future non-path-backed sources should come in as separate adapter projects.
- [x] Uporzadkuj safety i retry tylko tam, gdzie sa jeszcze luki. Read-only i idempotentne operacje maja zostac bounded-retry friendly, ale nie dokladaj pelnego replay nieidempotentnych transakcji bez osobnego projektu.
  - Historical note: the bounded replay envelope is now stabilized around read-only SQL, idempotent command SQL, and the replay-safe transactional wrappers already listed below. The remaining ambiguous-commit work stays in `docs/transactional-replay-project.md` instead of widening the current retry envelope in place.
- [x] Dopięcie `persist_file_extents_native(..., maintain_copy_crc_table = true)` do poprawnego binary COPY dla `copy_block_crc`.
  - Historical note: the original implementation encoded both representative `id_file` and owning `data_object_id`. Schema version 17 removed `id_file`; the current binary COPY encodes `data_object_id` and `_order` as PostgreSQL `INTEGER` fields and `crc32` as `BIGINT`. The extent replay smoke with `maintain_copy_crc_table = true` passes.
- [x] Nie wracaj do implementacji podstawowego pipeline jako nowego zadania. `scan`, `hash`, `duplicate detection`, `plan-import`, `materialize` i `cleanup` traktuj jako juz dostarczone; dalsza praca ma byc wokol granic, adapterow i hardeningu.
  - Historical note: the base indexer pipeline is already delivered, so the remaining work stays around adapter boundaries, hardening, and integration polish instead of re-implementing `scan` / `hash` / `plan-import` / `materialize`.

## Transactional Replay Project

- [x] Inventory transactional SQL call sites and split them into idempotent, replayable, and non-replayable groups.
- [x] Define the replay envelope and outcome confirmation for lost commit acknowledgements.
- [x] Add disconnect smoke tests for body failure, commit failure, and reconnect recovery.
- [x] Keep the current bounded replay envelope unchanged until the project proves a safe expansion.
  - Historical note: the replay envelope is now intentionally frozen at the verified safe scope, and any wider in-flight SQL replay work must be handled as a separate project.

## FOD 3.0.9 — Cleanup and recovery safety

- [x] Guard `fod-indexer cleanup-failed` against deleting shared data objects.
- [x] Report preserved shared data objects in the cleanup summary.
- [x] Extend cleanup smoke coverage for shared-object safety.
- [x] Document that recovery after errors is currently limited to one retry and does not yet replay in-flight SQL.
- [x] Document that ioctl support is currently limited to `FIONREAD`.
- [x] Document that full SELinux mount-label policy remains out of scope.
- [x] Document that FOD is still early-stage and APIs, benchmarks, and performance defaults may still evolve.

### Storage Hot-Path Prep

- [x] Split storage hot-path planning from SQL execution now that `fs.rs` has been broken up.
- [x] Define minimal planner interfaces before extending the extent engine.
- [x] Keep the current storage schema unchanged for now.
- [x] Preserve the existing helper parity tests while the hot-path refactor moves forward.
- [x] Route the opt-in extent PoC through a dedicated execution branch while keeping the block-storage fallback intact.
- [x] Define a narrow `PersistPlan` boundary for the remaining storage hot-path work so Rust returns the plan and SQL only executes the transaction, without changing the storage schema or the default block-storage path. Implemented as `PersistPlan::Blocks(...)` / `PersistPlan::Extents(...)` in `rust_hotpath/src/persist_plan.rs` with `choose_persist_plan(...)` as the selection boundary.
- [x] Split persist execution into explicit stages: plan persist, prepare rows, execute block storage, and execute extent storage, so the current extent PoC stops being a block-row adapter and becomes a real storage-engine boundary. Implemented in `rust_fuse/src/write_buffer.rs` with `prepare_persist_rows(...)`, `execute_block_storage_persist(...)`, and `execute_extent_storage_persist(...)`; the extent execution path still delegates to block-row persistence for now.

### Extent Engine PoC Scope

The PoC stays intentionally narrow:

- it is opt-in only through `enable_extents = true`,
- it starts with sequential write and read only,
- it accepts a single contiguous extent plan and falls back to block storage otherwise,
- it does not add merge/split/compaction logic yet,
- it does not replace the existing default block-storage path.

- [x] Describe the exact PoC scope so the extent work stays narrow and isolated.
- [x] Keep the PoC behind an explicit `enable_extents = true` gate and out of the default block-storage path.
- [x] Add the smallest possible feature/config gate if one is still missing.
- [x] Start with sequential write and read only.
- [x] Do not add merge/split extent logic at the start.
- [x] Benchmark the PoC on `4 KiB`, `64 KiB`, `1 MiB`, and `4 MiB`.

## Superseded extent-engine proposal

The extent-engine direction below was retired in FOD 3.2.71–3.2.73. Production
payload is block-only in `fod.data_blocks`; do not use the following proposal
as current architecture or implementation guidance. See
[`docs/CURRENT_STATE.md`](docs/CURRENT_STATE.md#storage-payload-model) for the
current model and [`docs/history/block-only-performance-plan.md`](docs/history/block-only-performance-plan.md)
for the retirement evidence.

## Target Architecture

### Warstwa 1 - Rust runtime

Rust is the runtime and control plane for FOD:

- mount bootstrap
- schema tooling
- config and profile loading
- FUSE callbacks
- administrative logic
- schema migrations
- ACL / permissions / journal / runtime validation policy layers

### Warstwa 2 - Rust storage hot-path

Rust owns the CPU/memory-heavy hot path:

- block engine
- read block assembly
- range slicing
- overlay plus `block_map` merging
- read buffer preparation
- write overlay engine
- `write_into_state()`
- `truncate_to_size()`
- dirty block management
- block payload assembly for flush
- copy engine
- `copy_file_range_into_state()`
- copy segmentation
- worker coordination for read/write copy paths
- segment ordering
- persist preparation
- list preparation for `(file_id, block_index, data)`
- block padding
- deleting blocks beyond EOF
- dirty range accounting

Why this matters:

- the benchmarks show that write path and finalization are the main bottleneck, not the project structure itself
- the highest-value Rust work is the part that reshapes data before SQL, not the SQL layer itself

## Archived Work

### FOD 3.0.0 Compatibility Cut

- [x] Remove `DBFS_*` env/config fallbacks.
- [x] Remove `copy_skip_*` transition alias shims.
- [x] Remove legacy `dbfs` / public schema migration handling from `rust_mkfs/src/main.rs`.

### Recent Architecture Cleanup

- [x] Split metadata cache payloads by purpose: attribute cache and directory-entry cache are now stored separately instead of sharing one mixed payload shape.
- [x] Fix the listing/getattr cache regression where `ls -al` on a directory could disagree with `ls file` because `readdir()` and `getattr()` interpreted the same cache key differently.
- [x] Remove runtime method rebinding from `fod_fuse.py` and replace it with explicit wrapper methods that delegate through `mod/repository/`.
- [x] Unify lookup helpers through the repository layer so `get_file_id`, `get_dir_id`, `get_entry_kind_and_id`, `entry_exists`, and related helpers no longer depend on hidden `__init__` rebinding.
- [x] Move the `getattr()` / `readdir()` query layer into `mod/repository/` so the main FUSE module no longer owns direct listing/attribute SQL for those hot paths.
- [x] Confirm the current refactor through `make test-all` plus an explicit regression check for create/list/remove consistency on a mounted FOD instance.

### FOD Runtime / FUSE Refactor

- [x] Centralize the version in the root `Cargo.toml`.
- [x] Migrate the main naming to FOD / `fod-*`.
- [x] Align CI with the workspace layout and FOD names.
- [x] Update `AGENTS.md` with the current working rules.
- [x] Add `FodFuseSettings` for mount initialization.
- [x] Centralize FUSE settings construction.
- [x] Centralize the `mount_options` helper.
- [x] Move FUSE mount startup out of `rust_fuse/src/main.rs`.
- [x] Keep `rust_fuse/src/main.rs` thin.
- [x] Split FUSE startup helpers into `rust_fuse/src/startup.rs`.
- [x] Split FUSE settings into grouped structs.
- [x] Split read cache helpers out of `rust_fuse/src/fs.rs`.
- [x] Split write buffer helpers out of `rust_fuse/src/fs.rs`.
- [x] Split copy range planner helpers out of `rust_fuse/src/fs.rs`.
- [x] Split runtime settings into grouped structs in `rust_runtime`.
- [x] Rename `DbfsFuse` to `FodFuse`.
- [x] Start the extent-engine PoC.
- [x] Add the benchmark runner to `Makefile`.

### Performance Plan

- Current numeric baselines and profile outputs live in [`BENCHMARKS.md`](BENCHMARKS.md). Keep this section focused on decisions, accepted changes, and rejected experiments.

- [x] Extract CLI parsing and mount startup into `fod_bootstrap.py` so `fod_fuse.py` only carries the filesystem operation layer.
- [x] Extract inode/path identity helpers into `fod_identity.py` so path normalization, inode generation, and ownership defaults are shared in one place.
- [x] Extract PostgreSQL connection pooling and config query helpers into `fod_backend.py` so the filesystem layer stops owning raw pool lifecycle.
- [x] Extract xattr and ACL policy helpers into `fod_xattr_acl.py` so xattr normalization, ACL encoding, and ACL checks live outside the filesystem core.
- [x] Remove the old in-class POSIX ACL constants and helpers from `fod_fuse.py` so `fod_xattr_acl.py` is the single source of truth for xattr/ACL policy.
- [x] Move xattrs from path-based storage to inode-based storage in `fod_xattr_store.py` so rename no longer has to rewrite xattr keys.
- [x] Extract lock state and lock conflict handling into `fod_locking.py` so advisory lock policy no longer lives in the filesystem core.
- [x] Extract data block loading, write cache management, dirty tracking, and buffer persistence into `fod_storage.py` so the write path is no longer embedded in the filesystem core.
- [x] Add an optional PostgreSQL-side CRC cache for unchanged-block copy detection. The cache is populated lazily on demand and refreshed during block persistence, so repeated copy-heavy workloads can reuse stored CRCs instead of rereading full destination blocks every time.
- [x] Remove the in-class write-buffer persistence implementation from `fod_fuse.py` so `fod_storage.py` is the single source of truth for data block persistence.
- [x] Move the read path to block-range loading with a small block cache and read-ahead so `read()` no longer has to load whole files on every access.
- [x] Benchmark the current write path on mounted FOD and record the baseline for large sequential writes. The live baseline is tracked in [`BENCHMARKS.md`](BENCHMARKS.md) and the profile entry points are `make test-throughput` and `make test-throughput-sync`.
- [x] Batch buffered writes in memory and persist them once per flush/release instead of rewriting the whole file repeatedly during a write burst.
- [x] Add block-delta persistence so unchanged blocks are not rewritten on every flush.
- [x] Profile the remaining hot paths (`getattr`, `readdir`, `persist_buffer`) and add only the indexes that move the benchmark. Added directory-parent indexes and confirmed the block-order index is used on `data_blocks`.
- [x] Capture a live throughput baseline for different write sizes on a mounted FOD instance. The measured values are recorded in [`BENCHMARKS.md`](BENCHMARKS.md).
- [x] Add schema versioning so the schema can be repaired in a controlled way instead of relying only on `init` to recreate the database. The historical first tracked version was `3`; the Rust `mkfs.fod status` command now exports the current migration manifest through version 19, and `make test-schema-upgrade` / `make test-schema-status` verify non-destructive init, upgrades, version-row recovery, and schema-admin protection.
- [x] Add named runtime profiles for production-style tuning in `fod_config.ini`. Current profiles include `bulk_write` and `metadata_heavy`, selected with `FOD_PROFILE`.
- [x] Add optional PostgreSQL TLS connection parameters (`sslmode`, `sslrootcert`, `sslcert`, `sslkey`) and client certificate/key generation to the Rust runtime and `mkfs.fod` implementation for `init` and `upgrade`.
- [x] Add a regression test for the flush/release dirty gate so clean closes stay cheap and dirty data is persisted exactly once. Added `make test-flush-release-profile`.
- [x] Try skipping the tail-delete optimization for normal growth writes. Rejected: the change regressed small-write throughput, so it was reverted.
- [x] Why it was rejected: the added bookkeeping outweighed the saved `DELETE`; see the historical benchmark notes in [`BENCHMARKS.md`](BENCHMARKS.md) if you need the exact comparison.
- [x] Record a live profile of the current write path so the next performance step starts from measured data instead of guesswork. The current profile split between write, persist, flush, and finalization is kept in [`BENCHMARKS.md`](BENCHMARKS.md) and surfaced by `make test-flush-release-profile`.
- [x] Reject a second tail-delete shortcut based on in-memory persisted-size tracking. Rejected for the same reason as the first shortcut: it hurt small-write throughput.
- [x] Do not retry tail-delete shortcuts or in-memory persisted-size tracking as a default performance strategy. Any future attempt must start from a different hypothesis and be benchmarked before merge.
- [x] Reject the per-block copy/fast-path rewrite in `persist_buffer`. Reverted after it regressed both small and medium writes.
- [x] Reject the shrink-marker variant for skipping tail `DELETE` on growth writes. Reverted because it was still worse than the stable baseline.
- [x] Gate `write()` profiling so it only runs when `FOD_PROFILE_IO=1`. This removed hot-path timing overhead.
- [x] Reject the memoryview-based `persist_buffer` copy path. Reverted after it regressed the write benchmark.
- [x] Cache `block_size` on the FOD instance instead of querying config during write-path operations. This removed a hot-path DB lookup.
- [x] Remove the extra `bytes()` copy from `write()` and assign the incoming buffer directly into the in-memory cache.
- [x] Skip `persist_buffer()` work in `flush()` / `release()` / `fsync()` when the file is not dirty.
- [x] Combine the tail `DELETE` and file-size `UPDATE` into one SQL round-trip inside `persist_buffer()`.
- [x] Avoid path-based lock cleanup work in `release()` and use the already-known file handle as the resource key.
- [x] Set `execute_values(..., page_size=len(blocks))` inside `persist_buffer()` so large block batches stay in one SQL round-trip.
- [x] Replace the `DELETE; UPDATE` multi-statement block in `persist_buffer()` with a single CTE-based statement.
- [x] Stop sorting dirty block indexes before persisting.
- [x] Stop copying the whole write buffer into `bytes()` before persisting and use `bytearray` slices directly for block payloads.
- [x] Split very large `persist_buffer()` batches into smaller SQL chunks instead of building one massive `execute_values()` payload.
- [x] Add a large-write auto-flush threshold so very large dirty buffers can be persisted before close instead of concentrating the entire cost in `release()`.
- [x] Expose `synchronous_commit` as a separate runtime knob for PostgreSQL sessions. Keep the default at `on` and treat `off` as an explicit tuning choice unless a future workload benchmark proves otherwise. The current local comparison is recorded in [`BENCHMARKS.md`](BENCHMARKS.md).
- [x] Extend the PostgreSQL WAL-pressure benchmark into a larger or checkpoint-forcing variant. Covered by `make postgres-benchmarks-compare` and the 2026-06-26 local/QNAP results in [`BENCHMARKS.md`](BENCHMARKS.md).
- [x] Expose `persist_buffer_chunk_blocks` as a separate runtime knob for flush batching. Keep the default conservative and let profiles override it when larger `execute_values()` batches help. The current comparison is recorded in [`BENCHMARKS.md`](BENCHMARKS.md).
- [x] Use the current benchmark baseline in [`BENCHMARKS.md`](BENCHMARKS.md) to decide whether the next performance work should focus on fewer SQL round-trips for small writes or on additional batching around flush/release. Decision: the current baseline is good enough; stop further tuning here unless a regression appears or a new benchmark target is introduced.
- [x] Confirm the large-write chunking fix on a real 1 GiB sequential write. The `dd if=/dev/zero of=test bs=1M count=1024` scenario completed successfully on `/mnt/fod`, and the file was visible afterward at the expected size.
- [x] Record the interpretation of the 1 GiB `dd` timing correctly: the data path itself finished in about `12 s`, while the remaining wall time was spent draining `flush()` / `release()` / `persist_buffer()` work. Future throughput work should measure `write` and finalization separately instead of treating `dd` wall time as pure copy speed.
- [x] Keep the benchmark suite expanded with explicit coverage for large `copy_file_range()` transfers, large multi-block file writes, and remount durability so write-path tuning stays comparable across releases.
- [x] Compare FOD atime behavior on a short wall-time benchmark. The current measured values live in [`BENCHMARKS.md`](BENCHMARKS.md).

### Finalized Performance Wins

These changes are already merged into the codebase and should be kept:

- `write()` profiling is opt-in via `FOD_PROFILE_IO=1`, so normal hot-path writes do not pay timing overhead.
- `block_size` is cached on the FOD instance instead of being queried from the database in the write path.
- `write()` no longer copies the incoming buffer through an extra `bytes()` conversion before writing into the cache.
- `flush()`, `release()`, and `fsync()` skip `persist_buffer()` work when the buffer is not dirty.
- `persist_buffer()` combines tail `DELETE` and file-size `UPDATE` into one SQL round-trip.
- `release()` uses the known file handle as the resource key for regular files instead of re-resolving the path.
- `persist_buffer()` splits very large block batches into smaller SQL chunks so large sequential writes do not exhaust the PostgreSQL client output buffer.
- Large sequential writes are now confirmed to work end-to-end on the mounted filesystem; the 1 GiB `dd` scenario completed successfully after chunking was added.
- Large dirty buffers can now auto-flush before close when they reach the configured threshold, which helps move finalization work out of `release()`.
- The read path now uses block-range loading with a small cache and read-ahead instead of loading whole files on every access.

### Must have

- [x] Fuller `xattr` family support:
  - `user.*`
  - `security.*`
  - `trusted.*`

#### Definition of done

- Each item in this section has:
  - an implementation in `fod_fuse.py` or a deliberate unsupported decision
  - at least one integration test or smoke test
  - passing coverage in `make test-all`

### Should have

- [x] Optional `mount --help`/README matrix with ready-made profiles:
  - `fod-relaxed`
  - `fod-linux-default`
  - `fod-selinux`

### Later

- [x] A short smoke profile for FOD atime behavior in `relatime` mode, plus a separate one for `noatime`.
- [x] A simple benchmark for FOD atime behavior that can compare `default`, `noatime`, and `nodiratime` runs on file reads and directory listings.
- [x] A target `make test-all-full` if symlinks/locks/journaling grow further.
- [x] Consider a separate test for `statfs` and `use_ino`.

## Already in place

- `getattr`, `readdir`, `open`, `read`, `write`, `truncate`, `rename`, `unlink`, `mkdir`, `rmdir`, `chmod`, `chown`, `utimens`, `statfs`
- `opendir`, `releasedir`, `fsyncdir`
- `destroy`
- `mknod` for FIFO and device nodes; `st_rdev` and `st_dev` are reported, and `open` for special nodes is still unsupported
- `flock`
- `fallocate`
- `copy_file_range`
- `ioctl` for `FIONREAD`
- `read_buf` / `write_buf`
- `poll` as a backend helper for regular files
- `lseek` as a backend helper for `SEEK_SET/SEEK_CUR/SEEK_END`
- `xattr` backend for `user.*`, `trusted.*`, `security.selinux`, and `system.posix_acl_*` with ACL enforcement and default inheritance
- `access()` smoke test for `R_OK`, `W_OK`, `X_OK`
- `access()` test for owner, primary group, and supplementary groups
- `access()` smoke test is part of `make test-all`
- mount suite covers end-to-end file and directory operations: `mkdir`, `rmdir`, `create`, `unlink`, `rename`, `read`, `write`, `truncate`, `chmod`, `chown`, `utimens`, `symlink`, `readlink`, `stat`, `statfs`, `df -Ph`, `df -Phi`, `access()`
- mount profile matrix is documented in README: `fod-relaxed`, `fod-linux-default`, `fod-selinux`
- smoke profiles for FOD atime behavior in `noatime` and `relatime` modes are exposed as `make test-atime-noatime` and `make test-atime-relatime`
- `make test-atime-benchmark` prints a simple wall-time baseline for file and directory atime behavior so `default`, `noatime`, and `nodiratime` runs can be compared directly.
- access-date writes are deduplicated per open handle, so a single `read()` / `readdir()` sequence only touches the timestamp once and does not rewrite it continuously.
- write-side `mtime` / `ctime` persistence is checked through a regression that confirms multiple writes on the same open file only advance metadata when the dirty buffer is flushed, not on every intermediate write call.
- sequential read-ahead now has a regression that verifies a second adjacent `read()` on the same handle preloads additional blocks into the cache instead of only fetching the requested byte range.
- the read cache defaults to a larger LRU size now, and sequential access stretches the read-ahead window so adjacent scans can reuse prefetched blocks more often.
- a dedicated read-cache benchmark can compare `FOD_READ_CACHE_BLOCKS=256` vs `1024` on sequential scans.
- metadata cache TTLs are configurable in `fod_config.ini` (`metadata_cache_ttl_seconds`, `statfs_cache_ttl_seconds`), and `getattr()` / `readdir()` / `statfs()` use short-lived caches with invalidation on mutating operations.
- metadata cache payloads are now split by type, so attribute and directory-entry state no longer share one cache payload layout.
- schema versioning is explicit: Rust `mkfs.fod` writes the current `schema_version`, `status` exports the version and migration manifest through version 19, and `make test-schema-upgrade` / `make test-schema-status` verify fresh initialization, supported upgrades, strict recovery of a missing version row, and schema-admin enforcement.
- runtime profiles are explicit: `FOD_PROFILE=bulk_write` and `FOD_PROFILE=metadata_heavy` override the base `[fod]` tuning values from `fod_config.ini`.
- `make test-all-full` extends `make test-all` with workflow checks for files/directories/metadata/symlink, shell statfs/use_ino, mount workflow, atime smoke, and throughput
- `make test-tree-scale` benchmarks `getattr`/`readdir` on a larger seeded tree
- stable inode model based on durable `inode_seed` values for directories, files, and symlinks
- `bmap` as a logical mapping for regular files and hardlinks
- hot-path indexes: `hardlinks.id_file`, `directories.id_parent`, `files.id_directory`, `hardlinks.id_directory`, `symlinks.id_parent`, and `data_blocks(data_object_id, _order)`
- `st_blocks` heuristic for directories and small files
- `st_nlink` for directories and root counted only from subdirectories
- `poll` as a backend helper for regular files
- `default_permissions` as the default mount option
- `use_ino` as the default mount option
- explicit wrapper/delegation model from `fod_fuse.py` into `mod/repository/`
- repository-owned query helpers for `getattr()` and `readdir()`
- SELinux options:
  - `--selinux auto|on|off`
  - default `off`
  - `auto` only when host-driven detection is desired
  - runtime SELinux is active only when `on` or `auto`
  - ACL activation uses `--acl on|off`
  - `context`
  - `fscontext`
  - `defcontext`
  - `rootcontext`
- atime / sync options:
  - `noatime`
  - `nodiratime`
  - `relatime`
  - `strictatime`
  - `lazytime`
  - `sync`
  - `dirsync`

## Documented Decisions

### SELinux

Status: intentionally closed as a non-goal for this repo.

- Decision: FOD keeps SELinux xattr handling gated by runtime configuration where the host forwards those requests, but it does not attempt to implement a full mount label policy or force ordinary FUSE into `fs_use_xattr`.
- Keep the existing coverage for:
  - mount with `FOD_SELINUX=on`
  - mount with `FOD_SELINUX=off`
  - mount with `FOD_SELINUX=auto`
  - reading and writing `security.selinux`
- Document explicitly that full SELinux correctness depends on host policy and mount-stack behavior. Rocky Linux 10.2 ordinary FUSE uses `genfscon fuse / ... fusefs_t`, so the supported model is operational enforcement through `fusefs_t` and domain policy, not per-inode relabeling.

### Full FUSE / Linux Compatibility

Based on verified `libfuse3` behavior and Linux VFS behavior, `libfuse3` stays the strategic baseline when compatibility, standardness, and easier upstream-aligned debugging matter most. This section records the current `ioctl` compatibility state:

#### Plan

1. Extend `ioctl` coverage beyond the current `FIONREAD` path if a real consumer appears in the mount suite or `pjdfstest`.
1. Add a dedicated smoke test for the consumer instead of the raw syscall wrapper, so the behavior is validated end-to-end through the mount.
1. Revisit whether any other libfuse hooks need explicit end-to-end coverage once `ioctl` has a real consumer.

Status: the mount suite now includes a real `ioctl/FIONREAD` consumer smoke test, so the first two items above are closed.
This section is now a documented compatibility note rather than active backlog.

#### Current status

- no known mount-smoke gap remains for `ioctl/FIONREAD`; keep the backend-only unit test as a low-level regression and the mount-suite smoke as the end-to-end check

### Repository / FUSE Boundary

Status: active design direction, but the main decisions below are already taken.

- Decision: `fod_fuse.py` should prefer explicit wrapper methods over runtime rebinding in `__init__`.
- Decision: lookup helpers and the `getattr()` / `readdir()` query layer should live in `mod/repository/`, not directly in the main FUSE module.
- Decision: metadata cache payloads should stay split by type instead of sharing one generic cache payload across attrs and directory listings.
- Follow-up: when `mod/repository/` grows enough, split it into lookup/query and mutation-oriented modules instead of moving SQL back into `fod_fuse.py`.

### Missing Filesystem Features

- No known open `file_metadata` gaps remain beyond the already-covered `change_date`/ctime tracking, `read`-driven `atime`, write/truncate/touch coverage, explicit `touch -a` / `touch -m` semantics, zero-length `write` handling, `truncate` no-op handling for unchanged sizes, and `utimens` no-op handling for unchanged timestamps on files and directories. Keep the regression tests in place.
- Permission behavior already covered includes sticky-bit enforcement, owner/root checks, supplementary-group-aware `chown`, symlink metadata handling, set-ID handling on ownership changes, unchanged-ownership `chown`/`chmod` no-op behavior, setgid-directory group/mode inheritance on creation, and `chmod()` synchronization of access ACL entries with the mode bits. The POSIX ledger remains authoritative for unreviewed privilege and default-ACL rules; keep the regression tests in place.

### POSIX Conformance Audit

- [x] **`stat()` nie może po cichu obcinać `st_nlink` poza limitem FUSE.**
  `fuse_attr.nlink` ma 32 bity (`include/uapi/linux/fuse.h`, Linux v6.17),
  podczas gdy FOD zlicza linki w `u64`. `attrs_for_path_from_repo()` i
  `attrs_for_open_file_id()` wcześniej zastępowały większą wartość przez
  `u32::MAX`; `root_attr()` robił to samo dla liczby podkatalogów. Wszystkie
  trzy ścieżki zwracają teraz `EOVERFLOW`, a sumowanie `2 + child_dirs` jest
  sprawdzane na przepełnienie. Test jednostkowy `fuse_nlink_reports_unrepresentable_link_counts`
  pokrywa `u32::MAX` i pierwszą nieprzedstawialną wartość. Test zamontowany
  z miliardami wpisów nie jest praktyczny. Zakres overflow pozostałych pól
  na x86_64 opisuje sąsiednie ustalenie; nowe ABI wymaga osobnego probe.
  Zob. [macierz errno](docs/POSIX_ERRNO_MATRIX.md).

- [x] **Sklasyfikować native Linux `stat()` `EOVERFLOW` dla x86_64.**
  ABI probe hosta wykazał 64-bitowe `ino_t`, `off_t`, `nlink_t`, `blkcnt_t`,
  `dev_t` i `time_t`. Linux v6.17 `cp_new_stat()` zgłasza overflow przy
  zwężaniu inode/link count/device oraz dla rozmiaru w 32-bitowym ABI;
  natywne x86_64 nie wykonuje tych zwężeń. FOD dodatkowo ogranicza rozmiary
  plików do PostgreSQL `BIGINT`, inode do `i64::MAX`, a `st_blocks` do
  `ceil(u64::MAX / 512)`, co mieści się w 64-bitowym `blkcnt_t`. Nie ma więc
  przypadku testowego native-stat overflow dla innych pól na audytowanym ABI.
  Zachowujemy test jednostkowy `st_nlink` ponad FUSE `u32`; test mounted
  wymagałby ponad 4 miliardów linków. Ponowić probe przy dodaniu ABI. Źródła:
  [Linux v6.17 `cp_new_stat()`](https://github.com/torvalds/linux/blob/v6.17/fs/stat.c#L2901-L2965),
  [FUSE `fuse_attr`](https://github.com/torvalds/linux/blob/v6.17/include/uapi/linux/fuse.h#L2674-L2708).

- [ ] Complete the [POSIX conformance audit](docs/POSIX_CONFORMANCE_AUDIT.md) for FOD's declared filesystem API. The audit defines POSIX.1-2024 Issue 8 as its baseline, identifies Linux/FUSE extensions separately, and starts a requirement matrix. Review exact normative sections and optional/XSI applicability; classify each requirement as pending, pass, fail, not implemented, or not applicable; map it to implementation evidence and a regression or `pjdfstest` case. Fix applicable gaps in sequenced changes and record the verified support boundary in `docs/compatibility-contracts.md`. Do not claim general POSIX compliance until every in-scope requirement has evidence and all exclusions are documented.
- [x] **Clear set-ID privilege bits after unprivileged writes and truncation.** FOD now requires `FUSE_HANDLE_KILLPRIV_V2` and `FUSE_SETXATTR_EXT` on writable mounts, applies write/truncate/`O_TRUNC` killpriv flags in the PostgreSQL content transaction, clears set-ID bits per Linux rules (including retaining SGID when group execute is absent), and removes remaining `security.capability`. ACL xattr updates apply the kernel's kill-SGID flag. Linux may issue an internal `REMOVEXATTR security.capability` before the content request; FOD allows that exact privilege-reducing removal for the requesting UID, while an explicit unprivileged `removexattr()` is rejected by Linux before FUSE. If this VFS path is used, xattr removal and content persistence are separate but PostgreSQL-serialized transactions, in kernel request order. Mounted regressions cover append, path truncate, `O_TRUNC`, capability removal and ACL-triggered SGID clearing; focused PostgreSQL and FUSE ABI tests pass. Verified on Linux `6.17.0-41-generic` with libfuse3 `3.17.4` against an isolated PostgreSQL database. This closes the specific write/truncate killpriv gap; P07 remains Pending for its broader permissions and identity matrix. Linux sources: [`fs/inode.c`](https://github.com/torvalds/linux/blob/v6.17/fs/inode.c), [`fs/fuse/file.c`](https://github.com/torvalds/linux/blob/v6.17/fs/fuse/file.c), [FUSE UAPI v7.33 flags](https://github.com/torvalds/linux/blob/v6.17/include/uapi/linux/fuse.h#L2490-L2502).
- [x] **Fail closed on malformed Linux kernel release fields.** The runtime preflight had parsed numeric prefixes from every `uname(2)` component, so malformed releases such as `6.17malformed.0` or `6.17-malformed.0` could be mistaken for kernel 6.17. It now requires numeric major/minor/patch fields and accepts distribution suffixes only after the final numeric component, introduced by `-` or `+`; known forms such as `6.17.0-41-generic`, `4.18.0-553.el8_10.x86_64`, `6.12-amd64` and `6.17.0+custom` remain parseable. Unit coverage rejects malformed minor/patch fields and suffixes before the patch field. This hardens the existing Linux 6.17.0 minimum; it does not change the minimum or claim support on older kernels.
- [x] **Keep `chmod()` and access ACL entries synchronized.** A mounted ACL regression first reproduced the defect on FOD 3.4.378: `chmod 0640` changed inode mode while leaving `ACL_USER_OBJ` and `ACL_MASK` stale. With `FUSE_POSIX_ACL` enabled, FOD now updates the owner entry plus the mask (or `ACL_GROUP_OBJ` when no mask exists) and other entry in the same PostgreSQL transaction as the inode mode change. `pg_query::chmod_updates_access_acl_and_mode_in_one_transaction` checks named-entry ACLs on files and directories and the no-mask form; `test_acl_mount_option.py` verifies the mounted file and directory behavior. Both tests passed on FOD 3.4.379 in `release-lto` against a fresh disposable PostgreSQL database on Linux 6.17/libfuse 3.17.4 on 2026-10-30. Linux v6.17's FUSE ACL code assigns inode-permission updates to userspace ([source](https://github.com/torvalds/linux/blob/v6.17/fs/fuse/acl.c#L116-L124)); its POSIX ACL chmod implementation defines the corresponding ACL entry changes ([source](https://github.com/torvalds/linux/blob/v6.17/fs/posix_acl.c#L506-L545)).
- [x] **Reject malformed stored POSIX ACLs before authorization.** The ACL decoder previously masked permission values with `0o7` and accepted duplicate named entries, so corrupt PostgreSQL ACL data could be interpreted as a valid permission list. It now rejects permission bits outside `ACL_READ|ACL_WRITE|ACL_EXECUTE`, duplicate named users or groups, and invalid ACL entry-class ordering before calculating access. Named user IDs are decoded as `u32` to preserve the full Linux UID range. `metadata_updates::tests` covers malformed permissions, duplicate IDs, accepted unsorted IDs, and a valid named user above `i32::MAX`; all 3 focused tests passed in `release-lto` on FOD 3.4.418. This fail-closed validation follows the permission-bit and entry-class checks in Linux v6.17 `posix_acl_valid()` ([source](https://github.com/torvalds/linux/blob/v6.17/fs/posix_acl.c#L2554-L2662)); duplicate-ID rejection is an additional FOD safety check. It does not claim that FOD implements every ACL namespace or idmapped-mount behavior.
- [x] **P07 inherit setgid parent ownership and directory mode.** Source review of Linux v6.17 `inode_init_owner()` confirms that children inherit the GID of a setgid parent and child directories always inherit `S_ISGID`; FOD previously assigned the requesting process GID and stripped setgid from every non-root mkdir. Creation now reads and locks the authoritative parent row in the same PostgreSQL namespace transaction, applies inherited GID to regular files, directories, symlinks and special files, and propagates setgid to child directories. `test_ownership_inheritance.py` passed on FOD 3.4.380 against a fresh disposable PostgreSQL 16.15 database on Linux 6.17/libfuse 3.17.4; it covers nested directories, files, symlinks, FIFOs, and umask-created objects. Linux source: [v6.17 `inode_init_owner()`](https://github.com/torvalds/linux/blob/v6.17/fs/inode.c#L2540-L2567).
- [x] **P09 `mknod()` stored inode type bits in permission metadata.** Mounted FIFO creation reached PostgreSQL but `getattr` returned `EIO`: `files.mode` contained octal `010644`, while FOD correctly expects inode type in `special_files.file_type` and permission metadata limited to `07777`. `mknod()` now strips `S_IFMT` before persisting both regular and special inode modes; the special type stays in the separate type field. `test_mknod.py` passed on FOD 3.4.380 against a fresh disposable PostgreSQL 16.15 database on Linux 6.17/libfuse 3.17.4, including FIFO open/read/write, blocking writer handoff and `_PC_PIPE_BUF` concurrent records. Character/block device attempts were rejected with `EPERM` and skipped as Linux-only extensions.
- [x] **Check overlong full paths across operations.** `test_path_trailing_slash.py` builds a pathname longer than the mounted filesystem's `PC_PATH_MAX`, using single-byte components below `NAME_MAX`, and asserts exact `ENAMETOOLONG` for 17 operations: `stat()`, `lstat()`, `open()`, `mkdir()`, `unlink()`, `rmdir()`, `truncate()`, `chmod()`, `chown()`, `utime()`, `statvfs()`, `readlink()`, hard-link source/destination, symlink destination and rename source/destination. Direct libc `access()` also returns `-1`/`ENAMETOOLONG`; the source and prospective destinations are checked unchanged. The mounted test passed on 2026-10-30. Linux rejects these overlong paths before dispatching a FUSE callback. Classify the conditional POSIX `PATH_MAX` requirements and continue P01's empty-component and per-operation final-component review.
- [x] **Verify empty pathname syscall errors.** `test_path_trailing_slash.py` checks that Linux returns exact `ENOENT` for empty-path `stat()`, `lstat()`, `open()`, `mkdir()`, `unlink()`, `rmdir()`, `readlink()`, `access()`, `chmod()`, `chown()`, `utime()`, `truncate()`, `statvfs()`, `link()`, `symlink()` and `rename()` calls before FUSE path lookup. libc `access()` is tested directly because Python `os.access()` converts the error to `False`. The expanded mounted regression passed on 2026-10-30. This records Linux syscall behavior and does not close P01's remaining empty-component and per-operation final-component review.
- [x] **Verify trailing slash on a regular-file path across operations.** The mounted `test_path_trailing_slash.py` now checks Linux `ENOTDIR` for `stat()`, `open()`, `lstat()`, `unlink()`, `rmdir()`, `truncate()`, `chmod()`, `chown()`, `utime()`, `readlink()`, `statvfs()`, hard-link source, and rename source/destination; `mkdir()` and link/symlink destinations return `EEXIST`, while libc `access()` returns `-1`/`ENOTDIR`. It also verifies the file and rename source remain unchanged; the test passed on 2026-10-30. This does not close the remaining symlink-final-component and path-length requirements in P01.
- [x] **Verify trailing-slash final-symlink operation rules.** `test_path_trailing_slash.py` confirms Linux follows a symlink to a directory for `stat()`/`open()`, returns `EEXIST` from `mkdir()` and `ENOTDIR` from `unlink()`/`rmdir()`, preserving both the symlink and target. P01 still needs equivalent review for other supported path operations.
- [x] **Verify trailing-slash symlink-to-file errors across callbacks.** `test_path_trailing_slash.py` checks Linux `ENOTDIR` for metadata, open, removal, attributes, `readlink()`, `statvfs()`, hard-link source and rename source/destination on a symlink-to-file path ending in `/`; `mkdir()` and link/symlink destinations return `EEXIST`, and libc `access()` returns `-1`/`ENOTDIR`. The mounted test confirms the symlink, target bytes and rename source remain intact. The expanded regression passed on 2026-10-30. P01 still needs equivalent review for other supported path operations and remaining path-length requirements.
- [x] **Verify trailing-slash symlink-to-directory operation rules.** `test_path_trailing_slash.py` checks that `stat()`/`lstat()`/`open()`/`chmod()`/`utime()`/`chown()`/`statvfs()`/`access()` resolve to the target directory; chmod/utime affect the target and preserve symlink timestamps. Linux returns `EPERM` for hard-linking the directory; `mkdir()` and link/symlink destinations return `EEXIST`; `unlink()`/`rmdir()` and rename source/destination return `ENOTDIR`; `truncate()` returns `EISDIR`; `readlink()` returns `EINVAL`. The mounted regression confirms the symlink, target and rename source remain intact and passed on 2026-10-30.
- [x] **Verify dangling symlink with trailing slash.** `test_path_trailing_slash.py` records exact Linux results for lookup, open, metadata, namespace mutation, both hard-link directions, symlink creation, rename source/destination, `statvfs()` and `access()` on a dangling symlink path ending in `/`. The calls return `ENOENT`, `EEXIST` or `ENOTDIR` by operation; the mounted test confirms the dangling symlink and rename source remain intact. It passed on 2026-10-30. Keep this case distinct from a plain dangling symlink.
- [x] **Check leading slash normalization.** `test_path_trailing_slash.py` verifies that this Linux/FOD mount resolves the same file through paths beginning `//` and `///`. POSIX requires more than two leading slashes to collapse to one; exactly two are implementation-defined.
- [x] **Verify `rmdir()` on a symlink to a directory.** `test_directories.sh`
  now requires exact `ENOTDIR` and confirms both the symlink and target remain.
  The mounted regression passed on 2026-10-30; references are recorded in
  [`POSIX_ERRNO_MATRIX.md`](docs/POSIX_ERRNO_MATRIX.md) and
  [`POSIX_CONFORMANCE_AUDIT.md`](docs/POSIX_CONFORMANCE_AUDIT.md).
- [x] **Verify `rmdir()` with a final `.` component.** `test_directories.sh`
  now requires exact `EINVAL` for `rmdir(directory/.)`; the mounted test passed
  on 2026-10-30. The POSIX errno matrix records the result.
- [x] **Verify directory link counts after cross-parent `rename()`.** The
  two-mount `test_rename_directory_transactional.py` regression moves a child
  directory and replaces an existing empty child directory across parents. It
  checks `st_nlink` changes from 3/2 to 2/3 on a move and from 3/4 to 2/4 on
  replacement, through both mounts. It sets
  `FOD_ATTR_TIMEOUT_SECONDS=0` because the default one-second FUSE attribute
  cache can otherwise return a valid pre-rename `st_nlink`. The full
  `release-lto` test passed on 2026-10-30 after building
  `fod-rust-fuse --features integration-test-hooks`; transactional target
  removal rollback and all existing rename-directory cases passed too.
- [x] **Parallel lock-test `ENOENT` came from unsafe mounted-workspace cleanup.** On 2026-10-30, a parallel run had 9/10 pass and the replica lock test failed with `ENOENT`; a repeat passed 10/10 but printed busy-unmount errors. `MountedFs::drop` ignored unmount failures and recursively removed `workspace/mount` even while it was a live FUSE mount. That traversed the shared FUSE root and could delete other mounts' PostgreSQL namespace entries. After the affected run, the local Docker test database had only 3 directories and 2 files; an earlier mount-startup snapshot in that run reported 21 directories. The running `/mnt/fod` service uses a `primary` hostname unavailable from this host, so its separate endpoint could not be checked. `MountedFs::drop` now preserves the workspace and log whenever unmount cannot be confirmed. `busy_mount_drop_preserves_shared_namespace_entries` deterministically holds a process cwd in the FUSE mount, checks the failed-unmount path preserves its workspace, then remounts and verifies a sentinel directory survived. On 2026-10-30, `make test-locking` passed 11/11 in parallel; all 21 namespace rows present before the run remained afterward, with 16 expected test rows added. The failed `release-lto` binary was ELF64 x86-64, Build ID `2f9de1e6a23f93aadce341ae8dc64c01232466d6`; `lookup_path`/`attrs_for_path_from_repo` were inspected in a symbol-preserving build whose `.text` bytes matched that binary exactly, with no code-generation discrepancy found. No timing or test serialization was changed.

### pjdfstest Observations

- FOD keeps directory `setgid` bits on ownership changes to match Linux-like behavior observed in `pjdfstest`, while still clearing special bits on regular files.
- FOD keeps `chown(-1, -1)` as an explicit no-op. The upstream `pjdfstest` cases note that POSIX allows timestamps to remain unchanged in that case, and FOD keeps that behavior explicit and tested.
- FOD keeps `unlink()` on directories as `EPERM`. The upstream `pjdfstest` coverage exercises this case, and FOD keeps it covered by a regression test.
- If a future change touches `chmod`, `chown`, `rename`, or `utimens` semantics again, extend compatibility coverage with additional `pjdfstest` subsets.

### Operational

- `LICENSE` is already set to MIT.
- PostgreSQL TLS is optional: `sslmode=require` gives encryption, and Rust `mkfs.fod --generate-client-tls-pair` can create a local client cert/key pair for certificate-auth setups during `init` or `upgrade`.
- FOD expects transactional PostgreSQL connections with `autocommit` disabled; `read committed` is sufficient for the current lock and metadata flows.
- Detect single-node vs read-only replica mode early and let the runtime pick an appropriate lock strategy for each case; `postgres_lease` stays the default production backend for writable primary mounts, and the Rust smoke suite already checks two primary mounts plus a replica against the same PostgreSQL database.
- Writable primary mounts now also maintain a `client_sessions` heartbeat row in PostgreSQL. If future cleanup or recovery logic grows around that session state, keep it TTL-driven, host-agnostic, and covered by multi-mount tests.
- Treat `session_id` as part of lease identity and cleanup scope for PostgreSQL-backed locks. `owner_key` alone is not enough once the same FOD dataset can be mounted from different hosts, so any future lock/session cleanup work should key off the session row and keep the semantics host-agnostic.
- Keep `workers_read` and `workers_write` constrained to the cases where they really help: disjoint read gaps and segmented copy operations, not small contiguous fetches.
- The most likely long-term direction for FOD is still userspace FUSE + PostgreSQL backend + a native Rust storage/hot-path engine, with Python kept as the orchestration layer until the native core is ready.

## Notes

- Storing `security.selinux` in xattr alone is not enough to make the filesystem fully SELinux-aware. On Rocky Linux 10.2 ordinary FUSE, SELinux/VFS rejects per-inode relabeling before FOD receives `FUSE_SETXATTR`; the supported model is `fusefs_t` plus domain policy.
- `mknod` creates FIFO and char-device metadata, but `open` for special nodes still needs separate semantics.
- `poll` is available through the Rust mount frontend for regular files.
- `fallocate`, `flock`, `copy_file_range`, `ioctl`, `read_buf`, `write_buf`, `opendir`, `releasedir`, `fsyncdir`, `destroy`, `access`, `bmap`, `lseek`, `rename`, `mknod`, `st_blocks`, `st_nlink`, `ownership inheritance`, advisory `locks`, sticky-bit enforcement on `unlink`/`rmdir`, `chown` special-bit clearing, journal UID tracking, `ctime`/`change_date` metadata tracking, and the stable inode model are already in `Already in place`.
- `statfs` and `use_ino` have a dedicated shell smoke test: `make test-statfs-use-ino`.
- metadata cache and statfs cache are TTL-backed and configurable via `fod_config.ini`; keep cache invalidation on mutating operations in sync with the read-side cache helpers.
- schema upgrades are intentionally conservative for now: `init` is idempotent and non-destructive, `upgrade` repairs missing schema objects and restores `schema_version`, the schema-admin secret is required once a database already exists, and the repo still does not ship multi-step migration files for future schema changes.
- production profiles should remain documented and tested when tuning defaults change, because they are part of the supported runtime surface.
- the public roadmap lives in `ROADMAP.md`, and the current comparison baselines live in `BENCHMARKS.md`; keep both in sync with changes to CI or runtime tuning.
- PostgreSQL-backed advisory locking is the supported production path for both `flock` and `fcntl` range locks; crash-recovery/TTL-expiry coverage is in place, and the remaining work is mostly operational hardening plus any edge-case cleanup.
- For Linux VFS, the main priority now is to keep metadata, permission checks, `statfs`, and the repository/FUSE boundary sane and consistent.


------->>2026.04.25 18:45<<----------------

Goal:
Add a mechanism for detecting identical regular files that may have different names and may be located in different directories, but have the same file size and the same content hash. In such cases, FOD should be able to reuse the same stored data object instead of storing duplicate content.

Scope:
- Applies only to regular files.
- File identity for deduplication should be based on:
  - file size,
  - full content hash.
- Preferred final hash: SHA256 or BLAKE3.
- A fast hash may be used only as a preliminary filter.
- In safe mode, FOD may additionally verify equality by byte-by-byte comparison.
- Different paths and filenames may reference the same stored data object.
- Removing one path must not remove the shared data while other references still exist.
- Modifying one file that shares data must either:
  - use copy-on-write, or
  - follow explicit hardlink semantics, depending on the selected FOD mode.

Preferred implementation model:
- Do not automatically create true POSIX hardlinks at the inode/file_id level.
- Start with data-object deduplication instead:

  path_a -> file_id_a -> data_object_id_x
  path_b -> file_id_b -> data_object_id_x

- When one of the files is modified, FOD should detach it from the shared data_object_id and create a new data object.
- Future idea: if `file_size + content_hash` dedupe comes back, implement it as block-level versioning with copy-on-write and immutable shared blocks, not as POSIX hardlinks. The goal is for later writes to diverge per file without mutating every path that started from the same content.

Design tasks:
- Review the current FOD metadata model.
- Identify where file_id/inode metadata is stored.
- Identify where chunks/blocks are stored.
- Add or design a data_objects/content_objects table.
- Store file_size, content_hash, reference_count, and data object metadata.
- Decide when the content hash is calculated:
  - on file release,
  - on flush,
  - after completed import,
  - or during a dedicated dedupe scan.
- Add protection against deduplicating files that are still open for writing.
- Add protection against deduplicating incomplete temporary files.
- Consider a dedicated command or maintenance mode:

  fod dedupe scan

Notes:
- File size alone is not sufficient.
- Weak checksums such as CRC32 should not be used as the final identity check.
- The minimum safe practical identity key should be:

  file_size + strong_content_hash

- Automatic deduplication should prefer copy-on-write semantics.
- True hardlink behavior should remain an explicit operation handled by link().

------->>2026.04.25 18:45<<----------------


[rename-fod-to-fod]
- Rename project branding from BDFS to FOD.
- FOD means FileSystem on Database Engine.
- The initial transition kept FOD-compatible wrappers and environment aliases.
- Do not rename PostgreSQL schema; canonical schema remains fod.
- Compatibility cut target: FOD 3.0.0


### Performance follow-up: Rust FUSE runtime tuning wiring

- [x] Pass read/write runtime tuning from `RuntimeConfig` into `FodFuse`.
- [x] Store `read_cache_blocks`, `read_ahead_blocks`, `sequential_read_ahead_blocks`, `small_file_read_threshold_blocks`, `workers_read`, `workers_read_min_blocks`, `workers_write`, and `workers_write_min_blocks` in the Rust FUSE state.
- [x] Add mount startup logging for these values.
- [x] Add a regression test proving env/config overrides are visible in the mounted Rust runtime.
- [x] Benchmark sequential read and metadata-heavy tree walk before and after the change. Covered by the read-cache benchmark and tree-scale / metadata-heavy entries in `BENCHMARKS.md`.


### Performance / Correctness Follow-ups

- [x] Make partial-block writes fail safely when the existing block cannot be loaded from PostgreSQL; do not replace a failed block read with zero-filled data.
- [x] Make `update_write_buffer()` treat zero-length writes as a no-op before changing the in-memory file size.
- [x] Add a deterministic eviction policy for `recent_write_blocks`.
- [x] Compare FIFO vs LRU behavior for `ReadBlockCache` on sequential, mixed, and random fio workloads. A six-run repeat series showed a mixed picture on the current host: sequential reads favored LRU, mixed workloads favored FIFO, and random mixed was effectively tied, so the cache policy choice is workload-dependent.
- [x] Keep extents opt-in until end-to-end mixed/random benchmarks show a stable win over the default block path. Fresh mixed and random mixed fio runs on this host still favor the block path, so the extent PoC stays an explicit opt-in.

  - Implementation correction (2026-08-07): the first saturation run
    proved that FOD still used fuser's default single event-loop thread.
    Therefore all limits reported `peak_active_tasks=1`. FOD 3.2.58 adds
    `FOD_FUSE_EVENT_THREADS` and optional `FOD_FUSE_CLONE_FD`; the saturation
    benchmark explicitly uses eight event threads and still requires a measured
    backlog plus exact active-limit use.

  - Implementation correction (2026-08-07): fairness now measures
    completion of the small `write()` call, which is the operation protected by
    logical-task admission. `fsync()` remains an end-to-end latency metric but
    no longer decides starvation. Large writers perform eight 256 KiB writes
    each to keep a longer competing tail, and failed runs are fully annotated
    instead of aborting before shutdown observability is parsed.

  - Implementation note (2026-08-07): FOD 3.2.59 adds a repeatable FUSE
    admission tuning matrix across event-thread counts `2/4/8/16` and write
    admission limits `0/1/2/4/8`. Each cell uses a fresh mount, matrix order is
    rotated between repeats, and non-binding combinations (`limit >= threads`)
    are measured rather than incorrectly rejected. The report ranks eligible
    cells using throughput, small-write median, small-write p95, and stability,
    while leaving production defaults unchanged until enough evidence exists.

  - Implementation note (2026-08-07): FOD 3.2.60 adds a focused
    confirmation suite for the four most relevant configurations from 3.2.59:
    `8/4`, `16/4`, `4/8`, and the `8/0` same-thread unlimited baseline. Each
    candidate runs ten times with rotated order and fresh mounts. The `8/4`
    candidate also receives three sequential-fio and three strace runs. The
    report marks `8/4` confirmed only when it remains rank 1, improves throughput
    and both small-write latency measures by at least 10% versus `8/0`, and
    keeps aggregate stability CV at or below 25%. Runtime defaults remain
    unchanged.

  - Implementation note (2026-08-08): FOD 3.2.61 moves the
    confirmed `8/4` FUSE concurrency/admission candidate into both base INI
    files, adds bootstrap propagation for startup-only FUSE controls while
    preserving explicit env overrides, documents the controls and their ranges,
    generates a production-Rust env-vs-INI audit, and adds a final production
    validation against `8/0` across sequential, mixed, random-mixed, strace, and
    mount-suite paths. Hard-coded fallback values remain unchanged.

## Execution roadmap after FOD 3.2.61

- [x] FOD 3.2.62 — fix and explain `pg_stat_database` telemetry. Use the same
  exported `FOD_PG_*` connection identity as the real FOD test/runtime path,
  add a before/after real-workload smoke, and make missing required snapshots
  fail validation instead of being warning-only.
- [x] FOD 3.2.63 — add PostgreSQL transaction-boundary admission limits.
- [x] FOD 3.2.64 — add payload-byte budgets and byte-aware backpressure.
- [x] FOD 3.2.65+ — continue role-aware multi-endpoint routing, consistency,
  endpoint health/load scoring, failover and cross-process fairness work.
  - Closure note (2026-09-08): startup routing, runtime primary failover,
    WAL-gated replica reads, adaptive scoring, promotion validation and
    process-local primary-generation fencing were delivered in later
    3.2.x releases. External/cross-process fencing remains a separate
    future hardening topic only when a concrete failure model requires it.
- [x] FOD 3.2.82 — add a process-local in-flight primary generation fence: operations using the guarded writable-primary target register against the current generation, primary generation transitions wait for the old generation to drain, and stale post-acquisition connections are not allowed to start a new operation. External cross-process fencing remains separate.

Detailed acceptance criteria are in `docs/fod-roadmap-3.2.62-plus.md`.

## FOD 3.2.71+ — retire extent PoC and simplify storage

- [x] Decide from measured 128 MiB profiling that extent PoC should not remain a production storage path.
- [x] FOD 3.2.71: force production FUSE persistence to the canonical `data_blocks` path while keeping legacy extent configuration parse-compatible.
- [x] FOD 3.2.71: update sequential fio/runtime-profile tests so `enable_extents=true` is a retirement guard, not an extent benchmark.
- [x] FOD 3.2.72: add explicit migration of existing `data_extents` to `data_blocks`, including integrity checks.
- [x] FOD 3.2.73: remove extent planner/payload/read/persist code and retired configuration after migration.
- [x] Re-profile block-only 4 MiB/128 MiB workloads and verify that repeated hardlink-count SQL is not the dominant cost on those profiles.
- [x] Optimize the file-attribute payload allocation query that repeatedly counts `data_blocks` for the same file object; FOD 3.2.74 moved normal `read()` away from the full `FileAttr.blocks` query, reducing the 128 MiB large-file profile from 1031 calls / about `3570 ms` to 5 calls / about `12 ms`.
- [x] FOD 3.2.75: instrument quota critical-section timing for persist transactions, COPY staging, `data_blocks` merge, advisory-lock wait/held time and final quota check without changing lock scope.
- [x] FOD 3.2.76: move ordinary block-persist quota advisory-lock serialization to the final quota validation gate, keeping reservation-token paths conservative until refresh/expiry semantics are reviewed.
- [x] FOD 3.2.76: prove direct two-repository quota safety and rollback behavior, then re-profile 4 MiB/128 MiB direct concurrent block persistence for 1/2/4/8 workers.
- [x] Re-run mounted two-FUSE-instance quota and fio/strace profiles using the temporary privileged Docker/chroot path that permits `fusermount3`, working `sudo`, strace and perf. Native Codex execution still has `NoNewPrivs: 1`, so future mounted profiles should use that same path or run outside this restricted process.
- [x] FOD 3.2.77: add mounted-FUSE write-stage instrumentation and decompose the 128 MiB direct-I/O write into admission, preflight, write-state, flush, persist and post-persist stages.
- [x] FOD 3.2.78: reduce mounted block persist overhead by avoiding per-block normalization allocation for full COPY BINARY blocks and by using append-only staging merge SQL when the target data object has no payload rows.
- [x] Revisit the remaining read metadata round trip only with a correctness-preserving design. The FOD 3.2.74 fix intentionally does not cache file size in `fh` without cross-mount/truncate invalidation; if this becomes material, fold size metadata into the block-read query or add a cache with a proven invalidation contract.
  - Closure note (2026-09-08): FOD 3.4.16 later reduced the measured 4 KiB read profile to one file-read metadata call while preserving the correctness boundary. Reopen only for a new measured regression.
- [x] Continue the remaining measured 128 MiB block-only persist bottleneck after FOD 3.2.78: the final profile still has two COPY calls totaling about `1020 ms`, one append-only `data_blocks` merge at about `142 ms`, and one remaining `ON CONFLICT` merge at about `355 ms`; next analyze whether the second 64 MiB flush can be proven append-only/disjoint before changing the merge shape.
  - Closure note (2026-09-08): this 3.2.78 profile is historical and was superseded by later block-only, 32 KiB storage-block and FOD 3.4.x read/write measurements. Any new persist optimization must start from a fresh current baseline rather than continue this old timing breakdown.

## FOD 3.4.427 — poisoned per-handle write-state map

- [x] Treat a poisoned write-state mutex as `EIO` in reads, writes, flushes,
  truncate, metadata-size overlays, `FIONREAD`, `poll` and `copy_file_range`.
  Previously, lock poisoning could be converted to `None` or silently ignored,
  allowing a callback to fall back to older PostgreSQL data or report success
  without publishing its updated local state. The shared `lock_or_eio()` helper
  now rejects poisoned access, all state update/removal results are propagated,
  and handle cleanup logs if it cannot remove state. `write()` now takes its
  target and sibling buffers under one lock and restores the prior merged state
  if preparing the requested block update fails. A deterministic unit test
  verifies the poisoned-lock to `EIO` mapping. This closes the local error
  mapping gap only; it does not claim recovery or cross-host persistence after a
  process panic.

## FOD 3.4.428 — timestamp sentinel and range coverage

- [x] Extend the mounted `utimensat()` regression to check negative and
  out-of-range `tv_nsec` values for both atime and mtime, and both mixed
  `UTIME_NOW`/`UTIME_OMIT` orders while asserting the omitted timestamp remains
  unchanged. This closes those specific test gaps only; invalid flags,
  precision boundaries, and the broader P06 metadata audit remain open.


## FOD 3.4.429 — refresh POSIX directory-stream evidence

- [x] Rerun the existing two-mount readdir snapshot test on FOD 3.4.428 and
  record the `open()`/`opendir()` `EMFILE` evidence in the POSIX audit and errno
  matrix. This closes the unrun-test gap only; P02 remains Pending for
  system-wide `ENFILE` and remaining directory errors.

## FOD 3.4.430 — cover file/symlink rename pairings

- [x] Extend `test_rename_directory_transactional.py` with successful file
  over symlink, symlink over file, symlink over symlink, and rejected
  symlink-to-directory over directory (`EISDIR`) and directory over symlink
  (`ENOTDIR`) cases. Verify source, target, and symlink target preservation.
  The full test passed on Linux 6.17.0-41-generic, libfuse3 3.18.2 and
  PostgreSQL 16.15; the fault-injected target-removal rollback also passed.
- [ ] P08 remains Pending: map remaining rename error precedence, permissions,
  limits and cross-mount observation after committed namespace mutations.

## FOD 3.4.431 — verify cross-mount rename visibility

- [x] Extend file/symlink rename-pair tests to observe successful destination
  changes and source removal through the other mount, and to observe both
  unchanged objects after type-mismatch failures. The full
  `test_rename_directory_transactional.py` suite passed on Linux
  6.17.0-41-generic, libfuse3 3.18.2 and PostgreSQL 16.15.
- [ ] P08 remains Pending for untested rename error precedence, permissions,
  limits and broader namespace cross-mount cases.


## FOD 3.4.432 — cover FIFO rename pairs

- [x] Extend `test_rename_directory_transactional.py` with FIFO-over-regular
  file and regular-file-over-FIFO replacement. Verify the resulting object type,
  payload where applicable, source removal, and committed visibility on both
  mounts. The full suite passed on Linux 6.17.0-41-generic, libfuse3 3.18.2 and
  PostgreSQL 16.15; its disposable test database was dropped afterward.
- [ ] P08 remains Pending for additional supported file-type pairs, rename
  permission/error-precedence cases, and broader cross-mount checks.

## FOD 3.4.433 — verify sticky-directory rename denial

- [x] Extend the mounted permission regression so an unprivileged rename in a
  sticky directory returns exact Linux `EPERM` and leaves both source and
  destination payloads unchanged. The focused permission test passed on Linux
  6.17.0-41-generic, libfuse3 3.18.2 and PostgreSQL 16.15.
- [ ] P08 remains Pending for other rename permission and error-precedence
  cases, limits, and broader cross-mount coverage.


## FOD 3.4.434 — verify sticky-directory rename owner exceptions

- [x] Verify Linux checks sticky-directory rename authority on both entries:
  owning only the source or only the destination returns `EPERM`, while owning
  both permits rename. The mounted permission suite passed on Linux
  6.17.0-41-generic, libfuse3 3.18.2 and PostgreSQL 16.15.
- [ ] P08 remains Pending for other rename permission/error-precedence cases,
  limits, and broader cross-mount coverage.

## FOD 3.4.435 — honor umask with inherited POSIX ACLs

- [x] A mounted regression with process umask `077`, requested create mode
  `0666`, and a default ACL reproduced a `0600` result. Linux v6.17 applies
  umask in the FUSE create path unless the daemon explicitly negotiates
  `FUSE_DONT_MASK`; `FUSE_POSIX_ACL` alone does not disable masking. FOD had
  neither requested that capability nor preserved the original mode through
  default-ACL lookup. ACL mounts now require and negotiate both
  `FUSE_POSIX_ACL` and `FUSE_DONT_MASK`. In the same PostgreSQL namespace
  transaction, the ACL inheritance helper filters original mode when a default
  ACL exists and applies umask when none exists. File, directory and
  special-file create paths pass the umask through this transaction. The
  mounted regression verifies capability negotiation, ignored umask,
  requested-mode filtering, child-directory preservation of the unfiltered
  default ACL and nested-file inheritance. It passed on Linux
  6.17.0-41-generic/libfuse3 3.18.2/PostgreSQL 16.15 with an isolated database,
  which was dropped afterward.
  Sources: [Linux FUSE initialization](https://github.com/torvalds/linux/blob/v6.17/fs/fuse/inode.c),
  [Linux FUSE create](https://github.com/torvalds/linux/blob/v6.17/fs/fuse/dir.c),
  [POSIX ACL create filtering](https://github.com/torvalds/linux/blob/v6.17/fs/posix_acl.c#L2921-L3019).
- [ ] P07 remains Pending for the rest of the permission and identity matrix.

## FOD 3.4.436 — check umask without a default ACL

- [x] Extend the mounted ACL regression to cover the other `FUSE_DONT_MASK`
  path: when a parent has no default ACL, FOD must apply the request umask
  itself. Under umask `077`, regular-file creation, `mkdir()` and `mkfifo()`
  must yield modes `0600`, `0700` and `0600`. The test confirms the parent has
  no default ACL before these operations. Validation runs against Linux
  6.17.0-41-generic, libfuse3 3.18.2 and an isolated PostgreSQL database.

## FOD 3.4.438 — check `rename()` with non-UTF-8 path components

- [x] Extend `test_path_trailing_slash.py` to require exact `EILSEQ` when a
  rename source or destination contains invalid UTF-8. Keep a valid U+FFFD name
  at each lossy-conversion alias and assert both payloads remain unchanged.
  The test passed on runtime FOD 3.4.437/Linux 6.17.0-41-generic/libfuse3
  3.18.2/PostgreSQL 16.15 using an isolated database, which was dropped after
  the run.
- [ ] P08 remains Pending for remaining rename permission/error precedence and
  limit evidence.

## FOD 3.4.439 — verify PostgreSQL errors during `rename()`

- [x] Extend the mounted PostgreSQL error-mapping regression so SQLSTATE
  `53100` from replacement of a regular-file target returns `ENOSPC`, and
  SQLSTATE `58030` returns `EIO`. In both cases the failed transaction leaves
  source and target payloads unchanged. The fault-injection trigger on
  `data_blocks` is installed after rename fixtures are persisted; installing it
  earlier caused their close-time flushes to fail and left legitimate active
  write leases, masking the intended rename SQLSTATE with `EBUSY`. The focused
  test passed on Linux 6.17.0-41-generic/libfuse3 3.18.2/PostgreSQL 16.15 with
  an isolated database, which was dropped after the run.
- [ ] P08 remains Pending for remaining rename permission/error-precedence,
  limit and broader operation coverage.

## FOD 3.4.441 — check `NAME_MAX` on both `rename()` components

- [x] Extend the mounted test to reject an overlong source as well as an
  overlong destination with `ENAMETOOLONG`. The existing exact-boundary rename
  still succeeds; failed destination/source calls preserve the source/target
  payload respectively. P01 and P08 remain Pending for their broader scopes.

## FOD 3.4.440 — verify `rename()` `NAME_MAX` handling

- [x] Extend the mounted pathname regression to rename a file to a component
  exactly `PC_NAME_MAX` bytes long and to reject a destination one byte longer
  with `ENAMETOOLONG`. The rejected call must preserve the source name and
  payload; the boundary-length rename must preserve the payload. This closes
  only this specific P01/P08 case; both audit areas remain Pending.

## FOD 3.4.490 — reject malformed namespace metadata rows

- [x] Require exact five-column directory-entry metadata rows and validate
  `rename()` resolver rows for cardinality, width, known entry kind, positive
  IDs and malformed nonempty file IDs. Added pure decoder regressions for
  short/extra rows, duplicate rows, invalid IDs and the valid empty file-ID
  representation for non-file entries. Both focused tests pass. The full
  `fod-rust-hotpath --lib` run passed 90 tests; four tests requiring a database
  could not run because the configured shared database is schema 31 while this
  client requires schema 38. The shared database was left untouched; no
  PostgreSQL integration claim is made for this change. This host lacks
  `/opt/postgresql-custom/bin/pg_config`; the system `pg_config` is Ubuntu
  PostgreSQL 17.10 without `--with-blocksize=32`, and no live server
  `SHOW block_size` value was verified.

## FOD 3.4.491 — reject malformed IDs in create confirmation

- [x] Make the six post-unique-violation namespace confirmation paths report
  malformed or non-positive object IDs as database-result errors rather than
  silently converting them to “not found”. Valid but nonmatching metadata
  still fails confirmation and retains the original unique-violation result.
  A pure positive-ID decoder regression covers valid, empty, malformed, zero
  and negative values. PostgreSQL integration remains pending because the
  configured database has schema 31 (the client requires 38) and this host
  lacks the FOD 32 KiB PostgreSQL binaries.

## FOD 3.4.492 — enforce exact hardlink and endpoint probe result shapes

- [x] Require the hardlink selection decoder's exact three-column projection
  and make PostgreSQL endpoint-role probing reject zero/multiple rows or
  columns. Added a pure endpoint-result shape regression. Formatting passes
  and the hotpath crate compiles. The FUSE test target could not compile because
  local libfuse3 is 3.14.0, below the project's 3.18.2 minimum. Live PostgreSQL
  integration is also unverified because the local server/build and schema do
  not meet the FOD integration requirements.

## FOD 3.4.493 — validate copy metadata and expired-session results

- [x] Require exactly two columns from the file-copy metadata query and decode
  every returned session ID from expired-session pruning through the existing
  one-column non-NULL validator. The row-decoder regression passed against the
  configured PostgreSQL 16.15 FOD container after verifying its
  `--with-blocksize=32` configure flag and runtime `SHOW block_size=32768`.
  `cargo check`, formatting and version checks also pass. This pure decoder
  query does not mutate the shared schema-31 database. In FOD 3.4.494, isolated
  schema-38 operation tests also pass for malformed copy metadata and expired
  session pruning; details are in the following item.

## FOD 3.4.494 — isolate PostgreSQL query tests from the shared FOD database

- [x] Create and drop temporary test databases through a `postgres`-database
  admin connection, so a shared FOD database below the current schema floor
  cannot block database setup or receive test DDL. Move the schema-corruption
  source-adoption regression off the shared database. Add an isolated operation
  test for expired-session pruning: it removes the expired session, preserves
  the active one, reports `true` after deletion and `false` when no rows remain.
  Both tests passed on PostgreSQL 16.15 after verifying the FOD
  `--with-blocksize=32` build and live `SHOW block_size=32768`; temporary
  databases were dropped and the shared schema-31 database was not modified.

## FOD 3.4.495 — reject malformed PostgreSQL boolean results

- [x] Route PostgreSQL text booleans through one strict decoder. Unknown values
  now return an error instead of being interpreted as `false` in connection
  role/snapshot checks, schema readiness and upgrade admission, advisory-lock
  results, write-lease and namespace-conflict checks, `rename()` cycle checks,
  and idempotency-token results. This matters for deferred-unlink cleanup and
  write fencing: malformed lease state must not imply that no lease exists.
  The pure decoder regression, hotpath crate check and formatting pass. These
  checks do not require a PostgreSQL query, so no database was changed.

## FOD 3.4.496 — make indexer PostgreSQL booleans fail closed

- [x] Reuse a strict PostgreSQL boolean decoder for indexer row projections,
  import-plan state, catalog snapshot creation/schema checks, and cleanup's
  advisory-lock, open-handle and write-lease checks. Previously, malformed
  `EXISTS` output in cleanup could be treated as “no active handles” and permit
  deletion. Malformed row cardinality in the snapshot schema check now reports
  a database-result error. All 36 indexer tests pass; no database was used.

## FOD 3.4.497 — validate scalar PostgreSQL booleans in mkfs and FUSE

- [x] Make `mkfs` boolean queries require exactly one non-NULL cell and accept
  only recognized PostgreSQL boolean values. The schema-upgrade advisory lock
  now uses this strict query path. FUSE runtime-override table discovery also
  rejects malformed boolean text instead of silently acting as if the table is
  absent. The hotpath boolean test and all `mkfs` unit-test targets pass. The
  `mkfs` content-generation integration tests refused the shared database name
  `foddbname` before making a connection. FUSE compilation remains unverified:
  this host has libfuse3 3.14.0 but FOD requires at least 3.18.2.

## FOD 3.4.498 — test the complete scalar-boolean result contract

- [x] Factor `mkfs` boolean decoding into a pure result-shape decoder and test
  valid values, zero/multiple rows, extra columns, SQL NULL, missing value
  pointers and malformed text. This covers the shape checks independently of
  a live server. Unit targets, hotpath compilation and version checks pass;
  no database was used.

## FOD 3.4.499 — reject non-positive data-object IDs in metadata reads

- [x] Both `file_data_object_id_on_conn()` and
  `file_data_object_info_on_conn()` now use the shared positive-ID decoder.
  Previously, a corrupt `data_object_id=0` could be accepted as a valid backing
  object. The existing decoder regression covers zero and negative IDs; the
  hotpath unit test and crate check pass without a database.
  Separately, the running PostgreSQL endpoint was checked with the custom
  `/opt/postgresql-custom/bin/pg_config` (`--with-blocksize=32`) and the
  configured FOD role reported `SHOW block_size=32768`; this change did not
  run a database test.

## FOD 3.4.500 — validate IDs returned by data-object creation

- [x] Route inserted, copied, hash-upserted and request-token-resolved data
  object IDs through the shared positive-ID decoder. File and special-file
  creation and rename replacement had separately parsed data-object IDs as
  arbitrary `u64` values, so a malformed zero result could be attached to a
  new inode or used in lifecycle cleanup. Verify the hotpath crate build and
  formatting pass; no PostgreSQL operation was run.
  PostgreSQL-backed operation coverage still requires the verified FOD 32 KiB
  server and a disposable schema-38 database.

## FOD 3.4.501 — fail closed on malformed file-lifecycle booleans

- [x] Decode the file-purge hardlink/open-lease flags and rename-replacement
  open-lease result through `parse_postgres_bool()`. Previously, unexpected
  result text was treated as `false`, which could bypass the hardlink-state
  guard or deferred unlink preservation. Hotpath compilation and formatting
  pass; no PostgreSQL operation was run. Integration coverage requires the
  verified FOD 32 KiB server and a disposable schema-38 database.

## FOD 3.4.502 — reject zero symlink IDs from hardlink lookup

- [x] Resolve the backing symlink ID through the shared positive-ID decoder
  in `symlink_id_for_hardlink_entry()`. The previous unsigned parse accepted
  zero, allowing malformed namespace metadata to escape as a valid target ID.
  The hotpath crate build and formatting pass; no database operation was run.

## FOD 3.4.503 — reject zero entity IDs in indexer results

- [x] Use shared positive-ID decoding for PostgreSQL entity IDs exposed in
  indexer models, snapshot/file catalogues, import-plan rows, duplicate sets,
  hash grouping and cleanup selections. Keep byte/count/inode/device values
  on unsigned parsing because zero is meaningful for those fields. Formatting
  and `cargo check --offline -p fod-rust-indexer` pass. No database operation
  was run, so malformed live SQL-result behavior still lacks integration
  coverage against an isolated schema-38 database.

## FOD 3.4.504 — reject malformed PostgreSQL booleans in filesystem decisions

- [x] Replace permissive `matches!(value, "true" | "t")` parsing in directory
  and file unlink, payload-empty, xattr and ACL decisions with the shared strict
  parser. Malformed values previously acted as false, risking incorrect
  namespace or privilege decisions. Formatting and hotpath crate compilation
  pass; database-backed fault injection still requires an isolated schema-38
  database on the verified FOD 32 KiB build.

## FOD 3.4.505 — validate remaining lifecycle and namespace IDs

- [x] Decode PostgreSQL-returned session, hardlink, symlink, parent-directory
  and deferred-unlink file IDs as positive identifiers before using them.
  Zero could otherwise flow into lease, promotion or namespace operations.
  Hotpath compilation and formatting pass; operation tests need the verified
  FOD 32 KiB server and an isolated schema-38 database.

## FOD 3.4.506 — fail closed on malformed PostgreSQL startup booleans

- [x] Route recovery state, schema readiness, required-config existence and
  upgrade-in-progress values through the shared PostgreSQL boolean decoder.
  Invalid text previously became `false`, potentially misclassifying a replica
  or admitting startup during malformed schema state. Formatting and hotpath
  compilation pass; no database operation was run.

## FOD 3.4.507 — validate returned write-lease fencing tokens

- [x] Distinguish a missing heartbeat `RETURNING` row from an empty or invalid
  token, require returned tokens to be positive and equal the requested
  fencing token. Previously empty text was treated as a stale lease and any
  nonempty token as a successful heartbeat. Formatting and hotpath compilation
  pass; no database operation was run.

## FOD 3.4.508 — require positive fencing tokens when acquiring write leases

- [x] Decode the destination and file fencing tokens returned by lease
  acquisition as positive IDs. Generic unsigned parsing previously accepted
  zero and could pass it into later heartbeat/write-fence operations.
  Formatting and hotpath compilation pass; no database operation was run.

## FOD 3.4.509 — reject invalid caller-provided write fences

- [x] Validate public lease and persistence-fence inputs before issuing
  heartbeat, release or persistence queries. A caller-constructed lease or
  fence with token zero could otherwise be reported as merely stale. Formatting
  and hotpath compilation pass; no database operation was run.

## FOD 3.4.510 — refuse cleanup of running imports

- [x] Refuse `cleanup-failed` while an import plan remains
  `materialize_running`. The plan-row lock did not stop an importer writing
  through separate transactions, so cleanup could delete a partially created
  tree while that importer continued. Automatic rollback sets
  `materialize_failed` first and remains supported. Formatting and
  `cargo check --offline -p fod-rust-indexer` pass. The direct regression for
  refusal remains pending; durable recovery for crashed importers is tracked
  in the unresolved finding at the top of this file.

## FOD 3.4.512 — record the unresolved shared-object cleanup race

- [x] Trace the interleaving where a concurrent object adoption increments
  `reference_count` after cleanup counts surviving files but before cleanup
  writes that count. The source-level review found a credible lost update, but
  no safe lock change without a complete file/object lock-order analysis.
  Runtime reproduction and a two-transaction regression remain pending; no
  code fix is claimed. See TX-03D in
  `docs/FOD_C10B2_HIGH_RISK_TRANSACTION_REVIEW.md`.

## FOD 3.4.513 — refuse cleanup that would delete external hardlinks

- [x] Detect hardlink aliases outside the failed import subtree before cleanup
  deletes imported `files` rows. The foreign-key cascade would otherwise erase
  those external namespace entries. The check runs after cleanup acquires the
  same per-file lifecycle advisory locks as hardlink creation, closing the
  check/delete race for new aliases. `cargo fmt --all --check`,
  `cargo check --offline -p fod-rust-indexer` and `git diff --check` pass;
  a direct database regression remains pending.

## FOD 3.4.514 — serialize cleanup reference-count changes

- [x] Replace stale absolute reference-count assignment with an atomic
  subtraction for imported references. Require the current count to cover the
  subtraction and fail closed on underflow. Delete exclusive data objects only
  if their count still equals the number of imported references being removed;
  a concurrent count change aborts the transaction. Formatting and indexer
  compilation pass; a deterministic two-transaction regression remains
  pending.
