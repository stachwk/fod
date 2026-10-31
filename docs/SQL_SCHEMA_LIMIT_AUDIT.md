# Audit limitów schematu SQL

Stan: 2026-10-31. Audyt obejmuje definicje w `migrations/base_schema.sql`,
migracje aktualizacyjne oraz limity wejściowe widoczne w implementacji FUSE.
Limit w kodzie klienta nie jest automatycznie ograniczeniem PostgreSQL: do
ochrony przed bezpośrednimi zapisami i wyścigami między mountami potrzebny jest
również warunek w bazie.

Audyt objął także użycie `TEXT` w schemacie świeżej bazy, migracjach
historycznych oraz dynamicznym SQL używanym przez Rust.

## Wyniki

| Dane | Definicja / limit SQL | Wniosek |
| --- | --- | --- |
| Nazwy wpisów katalogowych: `directories.name`, `files.name`, `hardlinks.name`, `symlinks.name` | `VARCHAR(255)` | Sam typ ograniczał liczbę znaków, a Linux `NAME_MAX` liczy bajty. Nazwa UTF-8 mogła więc przekroczyć 255 bajtów, mimo że miała mniej niż 255 znaków. Schema 33 dodaje `CHECK (octet_length(name) <= 255)`. |
| Nazwy xattr: `xattrs.name` | `VARCHAR(255)` | Ten sam rozdźwięk znaków i bajtów; schema 33 wymusza 255 bajtów także w PostgreSQL. |
| Nazwa zasobu dzierżawy: `destination_write_leases.name` | `TEXT` | SQL nie miał limitu długości. Schema 33 dodaje ten sam limit bajtowy 255, spójny z nazwą komponentu ścieżki używaną jako klucz dzierżawy. |
| Rozmiary plików i obiektów danych | `BIGINT NOT NULL` plus `CHECK (>= 0)` | Odpowiada signed 64-bit `off_t` w obsługiwanych systemach 64-bitowych; SQL odrzuca wartości ujemne. Maksimum to `2^63-1` bajtów. |
| UID/GID, `journal.id_user` i numery urządzeń `rdev_*` | `BIGINT` z zakresem `0..=2^32-1` | FUSE przekazuje te wartości jako `u32`. PostgreSQL `INTEGER` był signed i nie mieścił górnej połowy domeny. Schema 34 zachowuje pełny zakres przez `BIGINT` i sprawdza granice `u32`. |
| Kolejność bloku `data_blocks._order` i `copy_block_crc._order` | `BIGINT NOT NULL` plus `CHECK (>= 0)` | Rust używa `u64` dla indeksu bloku, a plik może mieć rozmiar `BIGINT`; poprzedni signed `INTEGER` ograniczał indeks do `2^31-1`. Schema 34 rozszerza typ i odrzuca indeksy ujemne. |
| Liczniki referencji, grup i zadań indeksatora | `BIGINT` plus ograniczenia nieujemności | Rust przechowuje je jako `u64`; `INTEGER` ograniczał je do `2^31-1`. Schema 34 poszerza `reference_count`, `file_count` oraz liczniki planów importu. PostgreSQL `BIGINT` nadal ma maksimum `2^63-1`, więc wartości `u64` powyżej tej granicy nie są obsługiwane. |
| Klucze `SERIAL` i relacyjne kolumny `INTEGER` | `SERIAL`/`INTEGER` | Pozostają signed 32-bit; pojedyncza tabela może osiągnąć maksymalnie `2^31-1` automatycznych identyfikatorów. Konwersja wymaga wspólnej migracji kluczy głównych i wszystkich kluczy obcych, więc pozostaje osobną decyzją. |
| `mode` w plikach i katalogach | `VARCHAR(6)` oraz `CHECK` formatu oktalnego `0..07777` | Schema 36 normalizuje historyczne wartości pełnego `st_mode` po sprawdzeniu, czy bity typu pasują do tabeli, i dodaje warunki walidujące. `VARCHAR(6)` pozostaje dla kompatybilnej reprezentacji tekstowej; limit nie służy oszczędzaniu miejsca. |
| `symlinks.target`, ścieżki i korzenie źródeł indeksatora, ścieżka mountu, filtr źródeł, komunikaty błędów | `TEXT` | Zmienna długość jest częścią domeny tych danych. `TEXT` nie rezerwuje maksymalnej długości; PostgreSQL przechowuje rzeczywistą wartość i może kompresować duże wartości. Nie zastępować arbitralnym `VARCHAR(n)`. |
| `inode_seed`, tokeny żądań (także tabele lease/session tworzone przez dynamiczny SQL) | `TEXT` | Wartości są generowane przez FOD i/lub kod wywołujący; wymagają ewentualnie osobnego limitu kontraktu, ale `VARCHAR(n)` nie zmniejszy zajętości dla tych samych danych. |
| `password_hash`, `password_salt`, `minimum_client_version` | `TEXT` | Rozmiar zależy od wybranego formatu hasha/soli i wersjonowania klienta. Ustalanie limitu ma sens tylko razem z walidacją formatu i wspieranych algorytmów. |
| `status`, `kind`, `file_kind`, `scan_status`, `hash_status`, `hash_algorithm`, `action` | `TEXT` | To pola o małym lub kontrolowanym zbiorze wartości. Ograniczenie ich listy przez `CHECK` może poprawić walidację danych, ale `VARCHAR(n)` nie przyniesie oszczędności miejsca. Wartości SQL i Rust powinny być kontrolowane wspólnie. |
| Zmienne lokalne funkcji PL/pgSQL oraz staging tables indeksatora | `TEXT` | Nie są trwałymi kolumnami schematu. Typ stagingu powinien pozostać zgodny z docelową kolumną. |

Migracje 0033–0034 i świeży schemat sprawdzają zakresy nazw i wartości
numerycznych. Test migracji
wykonuje bezpośredni `INSERT` do `fod.xattrs`: wartość 255 bajtów jest
akceptowana, a 128 znaków `é` (256 bajtów UTF-8) odrzucane. Test montowanego
FUSE sprawdza również `ENAMETOOLONG` dla nazwy, której długość znakowa mieści
się w 255, lecz długość bajtowa przekracza `PC_NAME_MAX`.
Test SQL aktualizuje właściciela katalogu do `u32::MAX`, sprawdza zachowaną
wartość i potwierdza odrzucenie `u32::MAX + 1`.
Przed migracją wykonano tylko odczytowe sprawdzenie aktualnej bazy FOD;
znaleziono zero ujemnych rozmiarów, indeksów bloków i liczników oraz zero
wartości UID/GID, `id_user` i `rdev` poza zakresem `u32`.

Odczytowe sprawdzenie pól `mode` w aktywnej bazie schema 31 wykazało 800
plików z historycznymi pełnymi bitami typu (np. `100644`, ósemkowo), żadnej
wartości o błędnym zapisie oktalnym i żadnego takiego katalogu. Bieżący parser
FOD słusznie odrzuca pełne `st_mode`, bo typ inode jest przechowywany osobno;
bez migracji te stare wiersze mogły zwracać `EIO`. Schema 36 usuwa typ inode
tylko wtedy, gdy jego bity odpowiadają tabeli (`S_IFREG` dla `files`,
`S_IFDIR` dla `directories`), po czym ogranicza obie kolumny do maksymalnie
czterech cyfr oktalnych. Test upgrade z wersji 1 zasiewa oba historyczne
przypadki i sprawdza wynik `644`/`755`. Baza aktywna pozostała nietknięta.

## Granice i dalsze decyzje

Weryfikacja `SERIAL`/`INTEGER` dla identyfikatorów inode/obiektów oraz potrzeby
obsługi liczników `u64` większych niż `i64::MAX` pozostają otwarte. Nie należy
zmieniać typów identyfikatorów bez wspólnej migracji wszystkich
kluczy głównych i obcych oraz sprawdzenia serializacji i istniejących danych.
Identyfikatory SQL pozostają do audytu. Reprezentacja `mode` ma już walidację
oraz migrację danych historycznych. Te pozycje są zapisane w
[`TODO.md`](../TODO.md); audyt nie certyfikuje pełnej zgodności POSIX ani
wszystkich wejść SQL.

## Wniosek dla `TEXT` i `VARCHAR`

W PostgreSQL `TEXT` oraz `VARCHAR` przechowują tę samą treść w ten sam sposób;
`VARCHAR(n)` nie przydziela z góry `n` znaków ani nie zmniejsza zajętości
wiersza czy indeksu. Limit długości dodaje kontrolę przy zapisie i może
odrzucić prawidłowe dane, jeśli `n` nie wynika z kontraktu. Dokumentacja
PostgreSQL stwierdza brak różnicy wydajności między tymi typami, poza kilkoma
dodatkowymi cyklami CPU dla sprawdzenia limitu: [Character Types — PostgreSQL](https://www.postgresql.org/docs/current/datatype-character.html).

W przejrzanych schematach nie znaleziono kolumny `TEXT`, której zamiana na
`VARCHAR(n)` uzasadniałaby się oszczędnością zasobów. Ograniczenia długości
należy dodawać tam, gdzie wynikają z Linux/POSIX lub formatu konkretnego pola;
powinny wtedy być wyrażone właściwą walidacją, zwykle bajtową dla ścieżek, a
nie arbitralnym limitem znaków. Osobnym problemem są duże wartości `TEXT`
używane jako klucze indeksów B-tree: wymagają analizy limitu wielkości wpisu
indeksu i sposobu indeksowania, a nie zmiany typu na `VARCHAR`.

### Potwierdzony przypadek: ścieżki indeksatora

`rust_indexer/src/scan.rs` przechodzi po źródle przez `WalkDir`, zapisuje
ścieżkę względną i nie nakłada na nią limitu krótszego od możliwości systemu
plików. Ścieżka jest kluczem unikalnym `(id_index_source, path)` w
`index_files`; ta sama wartość jest kopiowana do snapshotu z B-tree
`(id_catalog_snapshot, path)`. PostgreSQL ogranicza rozmiar pojedynczego wpisu
B-tree do około jednej trzeciej strony, po ewentualnej kompresji TOAST:
[B-Tree Indexes — PostgreSQL](https://www.postgresql.org/docs/current/btree.html).
Domyślny obraz PostgreSQL FOD jest kompilowany z `BLCKSZ=32KiB`; wariant
`BLCKSZ=8KiB` pozostaje dostępny dla zgodności i porównań. Budowę opisują
[`docker/postgres-blocksize/Dockerfile`](../docker/postgres-blocksize/Dockerfile)
i [`scripts/publish_postgres_fod_32k.sh`](../scripts/publish_postgres_fod_32k.sh):
Dockerfile przekazuje `--with-blocksize=32`, inicjalizuje tymczasowy klaster i
sprawdza `postgres -C block_size`; skrypt publikacji powtarza weryfikację
wyniku. Publikacja domyślnie wybiera 32 KiB przez
`FOD_POSTGRES_BLOCK_SIZE_KB=32`, nazwę obrazu `postgres-16-fod-32k` i bazowy
tag PostgreSQL 16.15; wariant 8 KiB jest jawnie wybierany jako zgodnościowy.
Uruchomiony lokalnie obraz `ghcr.io/stachwk/postgres-16-fod-32k:16` zwrócił
`32768` dla `SHOW block_size`, a `pg_config --configure` zawiera
`--with-blocksize=32`. Sam tag obrazu nie jest dowodem rozmiaru strony —
sprawdzaj parametr kompilacji i `SHOW block_size` dla obrazu, którego używa
wdrożenie.

Ścieżka plikowa ok. 3 KiB nie przekracza limitu wpisu PG32, ale na wariancie
PG8 bezpośredni test B-tree odrzucił wpis 3216-bajtowy przy maksimum 2704
bajtów. Długie nazwy źródeł nie mają takiego limitu wynikającego z `PATH_MAX`,
więc mogą przekroczyć limit także na większej stronie. Migracja 0035
zastępuje pełne klucze tekstowe indeksami kandydatów opartymi o `md5()`; kod
transakcyjny nadal porównuje pełną nazwę/ścieżkę bajtowo i serializuje
konkurencyjne zapisy blokadami advisory w PostgreSQL, więc skrót nie jest
tożsamością ani jedynym warunkiem unikalności. Indeks ścieżek snapshotów,
którego nie używały zapytania filtrujące substring i stronicowane po ID,
usunięto; indeks źródła snapshotu również nie przechowuje już pełnego tekstu.
Test migracji upgrade oraz regresja indeksatora przechodzą. Regresja zapisuje
źródło 3000-bajtowe, skanuje ścieżkę 3041-bajtową, tworzy snapshot i odczytuje
pełną ścieżkę zarówno na domyślnym PG32, jak i na PG8. Działa na osobnej,
tymczasowej bazie i nie wymaga montowania FUSE. Szczegóły implementacji i
wyniki weryfikacji są w [`TODO.md`](../TODO.md).
