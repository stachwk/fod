# Audit limitów schematu SQL

Stan: 2026-10-31. Audyt obejmuje definicje w `migrations/base_schema.sql`,
migracje aktualizacyjne oraz limity wejściowe widoczne w implementacji FUSE.
Limit w kodzie klienta nie jest automatycznie ograniczeniem PostgreSQL: do
ochrony przed bezpośrednimi zapisami i wyścigami między mountami potrzebny jest
również warunek w bazie.

## Wyniki

| Dane | Definicja / limit SQL | Wniosek |
| --- | --- | --- |
| Nazwy wpisów katalogowych: `directories.name`, `files.name`, `hardlinks.name`, `symlinks.name` | `VARCHAR(255)` | Sam typ ograniczał liczbę znaków, a Linux `NAME_MAX` liczy bajty. Nazwa UTF-8 mogła więc przekroczyć 255 bajtów, mimo że miała mniej niż 255 znaków. Schema 33 dodaje `CHECK (octet_length(name) <= 255)`. |
| Nazwy xattr: `xattrs.name` | `VARCHAR(255)` | Ten sam rozdźwięk znaków i bajtów; schema 33 wymusza 255 bajtów także w PostgreSQL. |
| Nazwa zasobu dzierżawy: `destination_write_leases.name` | `TEXT` | SQL nie miał limitu długości. Schema 33 dodaje ten sam limit bajtowy 255, spójny z nazwą komponentu ścieżki używaną jako klucz dzierżawy. |
| Rozmiary plików i obiektów danych | `BIGINT NOT NULL` | Typ dopuszcza wartości ujemne i ogranicza dodatnią wartość do `2^63-1`. Schemat nie ma ogólnego `CHECK`, który odrzuca wartości ujemne. |
| UID/GID i wiele identyfikatorów relacji | `INTEGER` | To signed 32-bit, więc kolumny dopuszczają liczby ujemne i ograniczają dodatnie do `2^31-1`. Wymaga to osobnej weryfikacji względem `uid_t`/`gid_t`, inode oraz sposobu przydzielania ID. |
| Klucze `SERIAL` | sekwencje `INTEGER` | Automatyczny identyfikator kończy się na zakresie signed 32-bit; to limit pojemności tabeli, a nie limit ścieżki POSIX. |
| Kolejność bloku `data_blocks._order` i `copy_block_crc._order` | `INTEGER NOT NULL` | Typ nie zabrania indeksu ujemnego i ogranicza go do signed 32-bit. Wartość ujemna nie jest wykluczona samą definicją SQL. |
| `mode` w plikach i katalogach | `VARCHAR(6)` | Ogranicza wyłącznie liczbę znaków; nie waliduje składni ani semantyki trybu. |
| `symlinks.target` i większość tekstowych metadanych | `TEXT` | Brak limitu długości w schemacie; wymóg długości wejściowej musi być egzekwowany w ścieżce operacji, jeśli dotyczy danej wartości. |

Migracja 0033 i świeży schemat sprawdzają sześć ograniczeń nazw. Test migracji
wykonuje bezpośredni `INSERT` do `fod.xattrs`: wartość 255 bajtów jest
akceptowana, a 128 znaków `é` (256 bajtów UTF-8) odrzucane. Test montowanego
FUSE sprawdza również `ENAMETOOLONG` dla nazwy, której długość znakowa mieści
się w 255, lecz długość bajtowa przekracza `PC_NAME_MAX`.

## Granice i dalsze decyzje

Weryfikacja `INTEGER` względem `uid_t`, `gid_t`, przestrzeni inode i ID jest
otwarta. Nie należy zmieniać typów bez sprawdzenia kluczy obcych, kodu
serializacji, migracji i istniejących danych. Osobno trzeba ustalić, które
nieujemne zakresy (`size`, `_order`, identyfikatory) powinny być wymaganiami
SQL, a które są już walidowane w transakcjach FOD. Te pozycje są zapisane w
[`TODO.md`](../TODO.md); audyt nie certyfikuje pełnej zgodności POSIX ani
wszystkich wejść SQL.
