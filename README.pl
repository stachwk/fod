<p align="center">
  <img src="assets/logo.png" alt="FOD logo" width="180">
</p>


# FOD

[![CI](https://github.com/stachwk/fod/actions/workflows/ci.yml/badge.svg)](https://github.com/stachwk/fod/actions/workflows/ci.yml) [Roadmap](ROADMAP.md) [Benchmarks](BENCHMARKS.md)

FOD (Filesystem On DataBaseEngine) to filesystem oparty o PostgreSQL, wystawiany przez FUSE. Ma zachowywać się jak praktyczny filesystem Linuksa: z przewidywalnymi metadanymi, sensowną semantyką katalogów, advisory locking, access checkami świadomymi ACL oraz testami, które sprawdzają realne ścieżki wykonania od końca do końca.

# FOD — Filesystem On DataBaseEngine

Słowa kluczowe:
- system plików
- FUSE
- PostgreSQL
- system plików oparty o bazę danych
- przechowywanie obiektów
- przechowywanie plików
- Rust
- Linux

Projekt skupia się na:

- stabilnych metadanych filesystemu
- sensownej zgodności z Linux/VFS
- jawnych opcjach runtime dla SELinux, ACL i polityki `atime`
- testach integracyjnych, które sprawdzają rzeczywiste zachowanie mounta, a nie tylko backend helpery

Aktualna uwaga runtime: FOD działa w pełni na Rustowym runtime. Wzmianki o Pythonie niżej są historycznymi baseline'ami migracyjnymi, a nie aktywną ścieżką fallback.

## O projekcie

FOD, czyli Filesystem On DataBaseEngine, to filesystem działający nad PostgreSQL. Projekt ma uprościć integrację aplikacji z przechowywaniem plików w bazie danych.

W wielu aplikacjach trzeba przechowywać dokumenty, obrazy, backupy, logi albo inne dane binarne w bazie. Zwykle oznacza to budowę i utrzymywanie dodatkowych warstw odpowiedzialnych za:

- upload i download plików
- zarządzanie katalogami
- synchronizację danych
- kontrolę wersji
- obsługę replikacji
- udostępnianie danych tylko do odczytu
- backup i restore

To zwiększa złożoność aplikacji i dokłada kolejny kod do utrzymania.

FOD nadal jest projektem na wczesnym etapie, więc API, benchmarki i charakterystyka wydajności nadal się zmieniają. Obecny cel to poprawność, architektura i praktyczne zachowanie filesystemu, a nie deklarowanie dojrzałej, maksymalnej przepustowości.

FOD usuwa ten problem, bo wystawia standardowy interfejs filesystemu. Dla użytkownika i aplikacji zachowuje się jak zwykły filesystem, taki jak ext4 czy xfs.

Aplikacje mogą korzystać ze standardowych operacji:

- `open`
- `read`
- `write`
- `mkdir`
- `rename`
- `cp`
- `rsync`
- `tar`

bez potrzeby wiedzy, że dane fizycznie trafiają do PostgreSQL.

### Główne zalety

- Prostota integracji: aplikacje zapisują pliki w zwykły sposób, bez własnej logiki binarnego storage.
- Centralizacja danych: pliki i metadane są w jednym spójnym systemie zarządzanym przez PostgreSQL.
- Wykorzystanie możliwości PostgreSQL: FOD korzysta naturalnie ze streaming replication, standby/read-only replicas, backup i restore, Point In Time Recovery (PITR), transakcyjności, kontroli integralności oraz pracy w wielu lokalizacjach.
- Replikacja i skalowanie odczytu: można uruchamiać wiele instancji read-only na replikach PostgreSQL, co pozwala budować rozproszone systemy dystrybucji plików, archiwa i środowiska HA/DR.
- Transparentność: użytkownik nie musi znać schematu bazy ani korzystać ze specjalnych API.

### Przykładowe zastosowania

- centralne repozytoria dokumentów
- systemy backupowe
- przechowywanie logów
- systemy HA/DR
- klastry read-only
- archiwizacja danych
- współdzielony storage dla aplikacji
- kontenery i środowiska chmurowe
- systemy edge z lokalnymi replikami

### Idea projektu

FOD łączy wygodę klasycznego filesystemu z możliwościami nowoczesnego silnika bazodanowego.

Zamiast budować kolejne warstwy pośrednie do obsługi plików, aplikacje mogą korzystać z jednego, spójnego interfejsu filesystemu, podczas gdy PostgreSQL odpowiada za trwałość, spójność, replikację i bezpieczeństwo danych.

## Licencjonowanie

FOD to oprogramowanie source-available licencjonowane na warunkach Business Source License 1.1 (BSL 1.1).

- Użycie niekomercyjne jest dozwolone.
- Użycie komercyjne wymaga odrębnej pisemnej umowy z właścicielem praw autorskich.
- Pełne warunki znajdują się w pliku [`LICENSE`](LICENSE), a kontakt do licencjonowania komercyjnego w [`LICENSE-COMMERCIAL`](LICENSE-COMMERCIAL).

## Aktualny Stan

- Główne operacje FUSE są zaimplementowane i pokryte testami integracyjnymi.
- `make test-all` przechodzi, a `make test-all-full` jest dostępne jako szerszy zestaw.
- Odczyty korzystają teraz z blokowego ładowania z małym cache i read-ahead zamiast pełnego ładowania pliku przy każdym dostępie.
- Test porównujący uprawnienia jest świadomie local-filesystem-vs-FOD, a nie tylko ext4-vs-FOD; porównuje hostowy zapisywalny local filesystem z FOD i sprawdza zgodność semantyki dla mode, ownership, access checków, sticky-bit unlink/rmdir oraz plików należących do root.
- Widoczność `allow_other` zależy od hosta: dedykowany test robi skip, jeśli host nie wystawia mounta dla `nobody`, więc jest to test diagnostyczny, a nie uniwersalna gwarancja pass/fail.
- Runtime jest teraz w całości oparty o Rust: frontend mounta żyje w `rust_fuse`, a bootstrap/schemat/mkfs w `rust_mkfs`.
- Lookup, CRUD namespace, metadane, permissions, xattr, locking, storage i journal handling żyją już w Rust zamiast w Pythonowych helperach.
- SELinux działa jako xattr z runtime gating; pełna polityka mount-label jest celowo poza zakresem.
- PostgreSQL TLS jest opcjonalny i konfigurowalny; FOD może też wygenerować lokalną parę certyfikat/klucz na żądanie.
- Przejściowe zerwania połączenia PostgreSQL w gorącej ścieżce odczytu/zapisu są ponawiane raz, z zachowaniem stanu po stronie procesu klienta, więc aktywny dirty write state i cache odczytu mogą przetrwać próbę reconnect.
- Migracja lock managera już się dokonała: PostgreSQL-backed leases są produkcyjną ścieżką dla zarówno `flock`, jak i range-locków `fcntl`, z TTL i heartbeat. `make test-locking` pozostaje zestawem semantyki locków, `make test-pg-lock-manager` pokrywa produkcyjny backend PostgreSQL, a `rust_fuse/tests/lock_backend_smoke.rs` sprawdza dwa niezależne primary mounty wobec tej samej bazy oraz repliki, która zostaje przy backendzie pamięciowym.
- Rustowy runtime trzyma osobne cache'owane połączenia dla zapisu i dla control plane, więc heartbeat i lease maintenance nie muszą czekać za długim flush.
- Wygasłe sesje writable primary są sprzątane przez sam PostgreSQL: usunięcie martwego `client_sessions` odpala trigger, który czyści jego lock leases i range leases po `session_id`.
- Świeże instalacje używają `migrations/base_schema.sql`, a `migrations/` trzyma numerowaną ścieżkę upgrade ze starszych stanów schematu i jawny eksport `mkfs.fod status`.
- Obecna wersja FOD pochodzi z wersji workspace `Cargo.toml` i jest wystawiana przez pomocnicze binarium Rust `fod-config`, a `fod-bootstrap --version` i `mkfs.fod --version` wypisują tę samą wartość. Wersje crate'ów Cargo dziedziczą ten workspace version.
- Kanoniczny schemat storage FOD nazywa się `fod` celowo: trzyma obiekty FOD poza `public`, żeby inne aplikacje w tej samej bazie nie kolidowały z tabelami FOD. Innymi słowy, `fod = canonical FOD storage schema`. W przyszłości można dodać parametr `fod.schema_name` dla wielu instancji FOD w jednej bazie, ale obecny runtime jest celowo przypięty do `fod`.
- Prace nad wydajnością są już w kodzie, a aktualne baseline'y benchmarków są zapisane w `BENCHMARKS.md`.
- Rustowy hot-path działa teraz w natywnym backendzie i współdzielonej bibliotece hot-path. Obejmuje planner, changed-run packing, padding bloków, składanie odczytu, logical resize planner dla `truncate()`/`fallocate()` oraz pierwsze lookupi/mutacje repo. Changed-copy dedupe zostaje opt-in, bo potrafi zauważalnie spowolnić workloady kopiujące.
- Lokalny stack Docker Compose preloaduje `pg_stat_statements`, a `make enable-pg-stat-statements` może utworzyć extension w lokalnej bazie, jeśli użytkownik DB ma do tego uprawnienia. Dzięki temu analiza zapytań i profilowanie runtime są dostępne w lokalnym stacku, ale inicjalizacja FOD nie zależy od uprawnień do tworzenia extension.
- `TODO.md` służy teraz jako log decyzji i notatek, a nie aktywny backlog implementacyjny.

## Pokrycie CI

Workflow GitHub Actions uruchamia krótki job kompilacyjny oraz wybrany matrix testów:

| Job | Co robi |
| --- | --- |
| `compile` | Byte-compiluje moduły core oraz obecne entry pointy testowe. |
| `workflow runtime` | Wymusza Node 24 dla akcji JavaScript przed domyślną zmianą GitHuba. |
| `test-runtime-config` | Sprawdza parsowanie runtime config i wynikowe wartości strojenia. |
| `test-runtime-validation` | Sprawdza, że błędne wartości runtime fail-fast odrzucają start. |
| `test-runtime-profile` | Sprawdza nazwane profile runtime. |
| `test-schema-upgrade` | Sprawdza bezpieczne `init` schematu, naprawę wersji i ochronę sekretu administracyjnego schematu. |
| `test-schema-status` | Sprawdza eksport statusu schematu i udokumentowany manifest migracji. |
| `test-postgresql-requirements` | Sprawdza minimalną wersję PostgreSQL i pojemność połączeń. |
| `test-metadata-cache` | Sprawdza krótki TTL cache metadanych i `statfs`. |
| `test-pg-lock-manager` | Sprawdza PostgreSQL-backed lock backend, zachowanie TTL / heartbeat i multi-host smoke coverage. |
| `test-read-ahead-sequence` | Sprawdza sekwencyjny read-ahead. |
| `test-block-read` | Sprawdza odczyt zakresowy bloków zamiast pełnego pliku. |
| `test-flush-release-profile` | Sprawdza zachowanie profilowania `flush/release`. |

Dla krok-po-kroku profili sprawdzeń lokalnych zobacz [zasady_sprawdzen.md](zasady_sprawdzen.md).

## Znane Ograniczenia

- Pełna polityka mount-label SELinux jest celowo poza zakresem; FOD trzyma SELinux jako metadane w xattr plus runtime gating.
- Obsługa `ioctl` jest celowo ograniczona na razie do `FIONREAD`.
- Metadane specjalnych urządzeń są zapisywane, ale pełna semantyka uruchamiania takich node'ów nie jest głównym celem projektu.
- `make test-all` jest głównym targetem regresji; workflow mounta są pokryte, ale CI skupia się na wybranym zestawie stabilnym w automatyzacji.
- Upgrade schematu jest na razie zachowawczy: `init` stosuje base schema dla świeżej instalacji, `upgrade` naprawia brakujący stan schematu i przywraca bieżącą wersję, a repo nadal trzyma numerowane pliki migracji dla starszych baz.
- FOD normalizuje timestampy przez sesję PostgreSQL ustawioną na UTC oraz konwersje w Rustowym runtime, więc lokalne różnice stref czasowych nie przesuwają metadanych. Ustawienie UTC jest inicjalizowane raz na fizyczne połączenie z puli, a nie przy każdej operacji filesystemu, i nie opiera się na domyślnych ustawieniach tworzenia bazy.
- Recovery jest ograniczone do ponawiania przejściowych disconnectów w gorącej ścieżce odczytu/zapisu; FOD trzyma stan dirty i cache w pamięci procesu, ale nie robi jeszcze pełnego replay dowolnych trwających operacji SQL.

## Wymagania

- Rust toolchain (`cargo`)
- PostgreSQL
- wsparcie FUSE na hoście
- `openssl`, jeśli FOD ma automatycznie generować parę certyfikat/klucz TLS dla PostgreSQL

## Pakiet Pip

FOD można zainstalować do virtualenv przez pip:

```bash
make install-on-root
```

To instaluje skrypty projektu do aktywnego venv:

- `fod-bootstrap`
- `mkfs.fod`
- `mount.fod`

W drzewie źródłowym nie ma już Pythonowych launcherów runtime. FOD dostarcza bezpośrednio binaria Rust: `fod-bootstrap`, `fod-config` i `mkfs.fod`. Zainstalowany `mount.fod` najpierw wybiera `target/debug/fod-bootstrap` i `target/release/fod-bootstrap` z bieżącego checkoutu, potem stare ścieżki `rust_mkfs/target/debug/fod-bootstrap` i `rust_mkfs/target/release/fod-bootstrap`, a dopiero później `fod-bootstrap` z `PATH` oraz `/usr/local/bin/fod-bootstrap`. Sam `fod-bootstrap` najpierw wybiera `rust_fuse/target/debug/fod-rust-fuse`, potem `fod-rust-fuse` z `PATH`, a na końcu `/usr/local/bin/fod-rust-fuse`. Jeśli `FOD_CONFIG` nie jest ustawione, a w bieżącym katalogu istnieje lokalny `./fod_config.ini`, wrapper eksportuje go automatycznie. Nieznane opcje wyglądające na FOD-owe wypisują ostrzeżenie na stderr, więc literówki typu `rool=primary` nie przechodzą po cichu; typowe opcje systemowe typu `_netdev`, `nofail` i `x-systemd.*` nadal są ignorowane. Jeśli nie znajdzie żadnego poprawnego bootstrappera ani sensownego pliku konfiguracyjnego, kończy się jasnym komunikatem zamiast zgadywać interpreter Pythona.

Przykład:

```bash
mount.fod /mnt/fod
```

Jeśli chcesz nazwany profil runtime, ustaw `FOD_PROFILE` jawnie albo podaj `--profile` / `-o profile=...` wtedy, gdy naprawdę potrzebujesz strojenia pod konkretny workload.

Wymagania PostgreSQL dla obecnego zestawu funkcji:

- PostgreSQL 9.5 lub nowszy
- `max_connections` powinno być wyraźnie większe niż `pool_max_connections`; jako praktyczne minimum zostaw co najmniej dwa dodatkowe połączenia dla administracji i równoległych klientów FOD
- nie są potrzebne specjalne parametry lock managera; domyślne `read committed` wystarcza
- FOD oczekuje transakcyjnych połączeń PostgreSQL z wyłączonym `autocommit`
- FOD inicjalizuje stan sesji UTC raz na cache'owane fizyczne połączenie i w stanie ustalonym zostaje tylko tani `rollback()`. Zapis i control plane używają osobnych cache'owanych połączeń, więc długi flush nie blokuje heartbeatów ani maintenance lease'ów.
- `sslmode=require` wystarcza do szyfrowania połączenia, a `verify-full` jest właściwe, jeśli chcesz też weryfikację certyfikatu

| Wymaganie | Wartość |
| --- | --- |
| Wersja PostgreSQL | `9.5+` |
| Tryb transakcyjny | `autocommit = off` |
| Poziom izolacji | `read committed` |
| `max_connections` | `pool_max_connections + 2` lub więcej |
| TLS | `sslmode=require` do szyfrowania, `verify-full` do weryfikacji certyfikatu |

## Przykładowy `fod_config.example.ini`

To jest minimalny punkt startowy. Repo-root `fod_config.ini` zostaje lokalnym configiem do dev/test, a `fod_config.example.ini` jest szablonem do kopiowania. Jeśli chcesz zainstalować konfigurację na współdzielonym hoście, skopiuj przykład i zmień hasło przed uruchomieniem `make install-config`.

```ini
[database]
host = 127.0.0.1
port = 5432
dbname = foddbname
user = foduser
password = cichosza

[fod]
pool_max_connections = 10
synchronous_commit = on
write_flush_threshold_bytes = 67108864
read_cache_blocks = 1024
read_ahead_blocks = 4
sequential_read_ahead_blocks = 8
small_file_read_threshold_blocks = 8
workers_read = 4
workers_read_min_blocks = 8
workers_write = 4
workers_write_min_blocks = 8
metadata_cache_ttl_seconds = 1
statfs_cache_ttl_seconds = 2

[fod.profile.bulk_write]
write_flush_threshold_bytes = 268435456
read_cache_blocks = 512
read_ahead_blocks = 2
sequential_read_ahead_blocks = 4
small_file_read_threshold_blocks = 4
workers_read = 4
workers_read_min_blocks = 8
workers_write = 8
workers_write_min_blocks = 8
metadata_cache_ttl_seconds = 2
statfs_cache_ttl_seconds = 2

[fod.profile.metadata_heavy]
write_flush_threshold_bytes = 67108864
read_cache_blocks = 1024
read_ahead_blocks = 4
sequential_read_ahead_blocks = 8
small_file_read_threshold_blocks = 8
workers_read = 4
workers_read_min_blocks = 8
workers_write = 4
workers_write_min_blocks = 8
metadata_cache_ttl_seconds = 5
statfs_cache_ttl_seconds = 5

[fod.profile.pg_locking]
lock_backend = postgres_lease
lock_lease_ttl_seconds = 30
lock_heartbeat_interval_seconds = 10
lock_poll_interval_seconds = 0.05

[fod.profile.extents]
# Opt-in sequential-only extent PoC preset.
enable_extents = true
```

## Pierwsze uruchomienie

Jeżeli uruchamiasz FOD pierwszy raz, zrób to w takiej kolejności:

1. Zainstaluj zależności wymienione wyżej.
1. Przygotuj PostgreSQL i upewnij się, że użytkownik oraz hasło w `fod_config.ini` są poprawne.
1. Wybierz, skąd FOD ma czytać konfigurację:
   - `/etc/fod/fod_config.ini`
   - albo lokalny plik `./fod_config.ini`
1. Jeśli config źródłowy nadal ma `password = cichosza`, `make install-config` wypisze ostrzeżenie przed skopiowaniem.
1. Utwórz schemat:

   ```bash
   mkfs.fod init
   ```

1. Zamontuj filesystem:

   ```bash
   fod-bootstrap -f /ścieżka/do/mountpointu
   ```

1. Zapisz plik do montażu, odczytaj go ponownie i sprawdź, czy dane przeżywają ponowne zamontowanie.
1. Po zakończeniu odmontuj filesystem:

   ```bash
   fusermount3 -u /ścieżka/do/mountpointu
   ```

## Minimalny start

Jeśli chcesz najszybszą drogę od zera do zamontowanego filesystemu, uruchom:

```bash
make up
make init
make mount
```

Jeśli chcesz użyć user-level pliku konfiguracyjnego zamiast `/etc/fod/fod_config.ini`, użyj:

```bash
make install-config-user
make mount-user
```

`make install-on-root` łączy `install-config`, `install-root-scripts`, `install-rust-hotpath` i `install-mount-helper` w jeden krok dla instalacji typu root-style. To instaluje config, Rustowe binarki, współdzieloną bibliotekę hot-path oraz helper mounta.

Jeśli chcesz finalny, bardziej zoptymalizowany build z ThinLTO i stripem symboli, użyj `make install-on-root FOD_CARGO_PROFILE=release-lto`.

`make install-on-root-venv` to odpowiednik `make venv` + `make install-on-root`.

Oba targety instalacyjne ostrzegają, jeśli config źródłowy nadal używa domyślnego hasła developerskiego `cichosza`.

## Szybki start

1. Skonfiguruj `/etc/fod/fod_config.ini` albo lokalny `fod_config.ini`.
1. Opcjonalnie uruchom `make install-config`, żeby skopiować wybrany plik konfiguracyjny do `/etc/fod/fod_config.ini`.
1. Dla instalacji typu root-style uruchom `make install-on-root`, żeby zainstalować config, Rustowe binarki, współdzieloną bibliotekę hot-path i helper mounta jednym krokiem.
1. Dla lokalnego developmentu możesz uruchomić `make install-config-user`, żeby zainstalować wybrany plik konfiguracyjny do `~/.config/fod/fod_config.ini` bez `sudo`.
1. `make config-show` pokazuje, którego pliku konfiguracyjnego FOD użyje, a `make mount-user` preferuje user-level `~/.config/fod/fod_config.ini` i wraca do lokalnego `fod_config.ini`, jeśli plik użytkownika nie istnieje.
1. Zainicjalizuj schemat:

   ```bash
   mkfs.fod init
   ```

   Jeśli chcesz, żeby FOD wygenerował lokalną parę certyfikat/klucz TLS PostgreSQL podczas tworzenia schematu, użyj:

   ```bash
   mkfs.fod init --generate-client-tls-pair 1
   ```

   Ta sama opcja działa też z `upgrade`:

   ```bash
   mkfs.fod upgrade --generate-client-tls-pair 1
   ```

   Wartość `--tls-common-name` jest walidowana przed zbudowaniem `openssl -subj`. Dozwolone są litery ASCII, cyfry, kropka, podkreślenie i myślnik.

1. Zamontuj filesystem:

   ```bash
   fod-bootstrap -f /ścieżka/do/mountpointu
   ```

## Obsługiwane parametry

FOD jest sterowany przez flagi CLI, zmienne środowiskowe oraz wartości z pliku konfiguracyjnego.

### Główne parametry runtime FOD

| Parametr | Typ | Domyślnie | Efekt |
| --- | --- | --- | --- |
| `-f`, `--mountpoint` | CLI | wymagane | Punkt montowania filesystemu FUSE. |
| `--role auto|primary|replica` | CLI / `FOD_ROLE` | `auto` | Steruje wykrywaniem repliki i wyborem backendu locków. `-o ro` daje mount tylko do odczytu bez zmiany roli. |
| `--selinux auto|on|off` | CLI / `FOD_SELINUX` | `off` | Włącza lub wyłącza obsługę `security.selinux`. |
| `--acl on|off` | CLI / `FOD_ACL` | `off` | Włącza lub wyłącza egzekwowanie POSIX ACL. |
| `--default-permissions` / `--no-default-permissions` | CLI / `FOD_DEFAULT_PERMISSIONS` | on | Steruje tym, czy kernelowe sprawdzanie uprawnień jest aktywne. |
| `--atime-policy default|noatime|nodiratime|relatime|strictatime` | CLI / `FOD_ATIME_POLICY` | `default` | Wybiera wewnętrzne zachowanie `atime` FOD. |
| `--lazytime` | CLI / `FOD_LAZYTIME` | off | Włącza opcję montowania `lazytime`. |
| `--sync` | CLI / `FOD_SYNC` | off | Włącza opcję montowania `sync`. |
| `--dirsync` | CLI / `FOD_DIRSYNC` | off | Włącza opcję montowania `dirsync`. |
| `FOD_ALLOW_OTHER=1` | Zmienna środowiskowa | off | Włącza `allow_other`, jeśli FUSE na to pozwala. |
| `FOD_USE_FUSE_CONTEXT=1` | Zmienna środowiskowa | on | Używa per-request uid/gid/pid dla access, ACL, sticky-bit i operacji zależnych od właściciela zamiast credentiali procesu demona. |
| `FOD_DEBUG=1` | Zmienna środowiskowa | off | Włącza debugowy tryb montowania jako domyślny. |
| `FOD_LOG_LEVEL=DEBUG|INFO|...` | Zmienna środowiskowa | `INFO` | Steruje poziomem logowania. |
| `FOD_CONFIG` | Zmienna środowiskowa | auto-detekcja | Wymusza konkretną ścieżkę do pliku konfiguracyjnego. Jeśli wskazuje na brakujący albo nieczytelny plik, FOD kończy pracę błędem zamiast robić fallback. |
| `FOD_SELINUX_CONTEXT` | Zmienna środowiskowa | nieustawione | Ustawia opcję mount `context=` dla SELinux. |
| `FOD_SELINUX_FSCONTEXT` | Zmienna środowiskowa | nieustawione | Ustawia opcję mount `fscontext=` dla SELinux. |
| `FOD_SELINUX_DEFCONTEXT` | Zmienna środowiskowa | nieustawione | Ustawia opcję mount `defcontext=` dla SELinux. |
| `FOD_SELINUX_ROOTCONTEXT` | Zmienna środowiskowa | nieustawione | Ustawia opcję mount `rootcontext=` dla SELinux. |
| `FOD_DEFAULT_PERMISSIONS` | Zmienna środowiskowa | `1` | Steruje tym, czy domyślne checki uprawnień są przekazywane do FUSE. |
| `FOD_ENTRY_TIMEOUT_SECONDS` | Zmienna środowiskowa | `0` | Steruje TTL cache wpisów katalogu w FUSE. |
| `FOD_ATTR_TIMEOUT_SECONDS` | Zmienna środowiskowa | `0` | Steruje TTL cache atrybutów w FUSE. |
| `FOD_NEGATIVE_TIMEOUT_SECONDS` | Zmienna środowiskowa | `0` | Steruje TTL cache negatywnych wpisów w FUSE. |
| `FOD_SYNCHRONOUS_COMMIT` | Zmienna środowiskowa | `on` | Steruje `synchronous_commit` PostgreSQL dla każdego połączenia. |
| `FOD_PG_VISIBLE_PATH` | Zmienna środowiskowa | nieustawione | Nadpisuje ścieżkę używaną do pomiaru widocznej dla PostgreSQL pojemności filesystemu dla `statfs()`. |
| `FOD_PERSIST_BUFFER_CHUNK_BLOCKS` | Zmienna środowiskowa | `128` | Steruje liczbą dirty bloków pakowanych do jednego zapytania `persist_buffer()`. |
| `FOD_PG_SSLMODE`, `FOD_PG_SSLROOTCERT`, `FOD_PG_SSLCERT`, `FOD_PG_SSLKEY` | Zmienna środowiskowa | nieustawione | Nadpisuje parametry TLS połączenia do PostgreSQL. |

### Plik konfiguracyjny

`fod_config.ini` powinien zawierać sekcję `[database]` z parametrami połączenia do PostgreSQL:

- `host`
- `port`
- `dbname`
- `user`
- `password`
- `sslmode` dla szyfrowanego połączenia PostgreSQL, na przykład `require` albo `verify-full`
- `sslrootcert` dla certyfikatu CA używanego do weryfikacji serwera
- `sslcert` i `sslkey` dla opcjonalnej autoryzacji certyfikatem klienta

Może też zawierać sekcję `[fod]` z:

- `pool_max_connections`
- `write_flush_threshold_bytes`
- `read_cache_blocks`
- `read_ahead_blocks`
- `sequential_read_ahead_blocks`
- `small_file_read_threshold_blocks`
- `workers_read`
- `workers_read_min_blocks`
- `workers_write`
- `workers_write_min_blocks`
- `persist_buffer_chunk_blocks`
- `max_fs_size_bytes`
- `pg_visible_path`
- `copy_dedupe_enabled`
- `copy_dedupe_min_blocks`
- `copy_dedupe_max_blocks`
- `copy_dedupe_crc_table`
- `metadata_cache_ttl_seconds`
- `statfs_cache_ttl_seconds`
- `lock_lease_ttl_seconds`
- `lock_heartbeat_interval_seconds`
- `lock_poll_interval_seconds`
- `synchronous_commit`

Kanoniczne reguły zakresów dla wartości runtime są w [`rust_runtime/src/lib.rs`](/media/wojtek/virtdata/home/wojtek/git/fod/rust_runtime/src/lib.rs); ta lista jest tylko skrótem dla czytelnika. `pool_max_connections` musi być większe od zera, bo ustawia limit połączeń w puli PostgreSQL.

### Narzędzie do tworzenia schematu

`mkfs.fod` obsługuje:

`init` stosuje świeży bootstrap z `migrations/base_schema.sql` do dedykowanego schematu `fod` i odmawia działania, jeśli obiekty FOD już istnieją; `upgrade` najpierw weryfikuje hasło schema-admin, a potem stosuje brakujące migracje do istniejącego schematu `fod`; `clean` usuwa cały schemat `fod` i zostawia obce obiekty w `public` nietknięte. `clean` weryfikuje istniejący sekret schema-admin i zamiast go odtwarzać kończy się błędem, jeśli tabela lub wpis sekretu zniknęły. Narzędzie schematu używa jednego jawnego źródła hasła administracyjnego schematu: `--schema-admin-password`. Jeśli hasła brakuje, `init`, `upgrade` i `clean` kończą się natychmiast, bez promptu i bez ukrytej generacji sekretu. `mkfs.fod status` pokazuje `FOD version`, `FOD schema name`, `FOD schema version`, aktywny schemat, to czy obiekty FOD istnieją, to czy schemat jest gotowy, oraz zaległe migracje, bez ujawniania samego sekretu.

| Parametr | Typ | Domyślnie | Efekt |
| --- | --- | --- | --- |
| `init` | akcja | wymagane | Stosuje `migrations/base_schema.sql`, żeby utworzyć świeży schemat FOD w `fod`; odmawia działania, jeśli obiekty FOD już istnieją. |
| `upgrade` | akcja | wymagane | Najpierw weryfikuje hasło schema-admin, a potem stosuje brakujące migracje do istniejącego schematu `fod` i przywraca `schema_version` do wersji kodu. |
| `clean` | akcja | wymagane | Usuwa cały schemat `fod`; obce obiekty w `public` zostają nietknięte. |
| `--block-size N` | CLI | `4096` | Ustawia domyślny rozmiar bloku używany przy inicjalizacji schematu. |
| `--schema-admin-password PASS` | CLI | generowane przy pierwszym `init`/`upgrade` | Sekret narzędzia schematu zapisany w bazie i wymagany przy późniejszych wywołaniach `init` / `upgrade` / `clean` na istniejącej bazie. |
| `--generate-client-tls-pair 1` | CLI | wyłączone | Generuje lokalną parę certyfikat/klucz TLS PostgreSQL podczas `init` lub `upgrade`. Użyj `0`, żeby wyłączyć jawnie. |
| `--tls-material-dir PATH` | CLI | `.fod/tls` | Ustawia katalog dla wygenerowanych materiałów TLS PostgreSQL. |
| `--tls-common-name NAME` | CLI | `fod` | Ustawia common name dla wygenerowanych materiałów TLS. Dozwolone znaki: litery ASCII, cyfry, kropka, podkreślenie i myślnik. |
| `--tls-cert-days N` | CLI | `365` | Ustawia czas ważności wygenerowanych materiałów TLS. |

## Docker Lab

Dla lokalnego backendu PostgreSQL:

```bash
make up
make init
make smoke
make mount
# w drugim terminalu:
make unmount

# demo w jednym kroku:
make demo

# test integracyjny:
make test-integration

# autodetekcja roli:
make test-role-autodetect

# pełny lokalny check:
make test-all

# rozszerzony pełny lokalny check:
make test-all-full
```

Osobne targety są rozdzielone tak, żeby można było odpalać tylko interesujący obszar:

- `make test-files`
- `make test-block-read`
- `make test-directories`
- `make test-metadata`
- `make test-symlink`
- `make test-destroy`
- `make test-locking`
- `make test-permissions`
- `make test-hardlink`
- `make test-fallocate`
- `make test-copy-file-range`
- `make test-ioctl`
- `make test-mknod`
- `make test-lseek`
- `make test-poll`
- `make test-utimens-noop`
- `make test-timestamp-touch-once`
- `make test-read-ahead-sequence`
- `make test-read-cache-benchmark`
- `make test-runtime-config`
- `make test-runtime-validation`
- `make test-mkfs-pg-tls`
- `make test-metadata-cache`
- `make test-runtime-profile`
- `make test-schema-upgrade`
- `make test-schema-status`
- `make test-access-groups`
- `make test-inode-model`
- `make test-ownership-inheritance`
- `make test-statfs-use-ino`
- `make test-atime-noatime`
- `make test-atime-relatime`
- `make test-pool-connections`
- `make test-mount-suite`
- `make test-all-full`

## Helper montowania

Jeśli chcesz, żeby FOD działał jak helper `mount.fod`, zainstaluj skrypt do katalogu z `PATH`:

```bash
sudo install -m 755 mount.fod /usr/local/sbin/mount.fod
```

To samo możesz zrobić przez:

```bash
make install-mount-helper
```

Potem możesz montować FOD tak:

```bash
mount.fod /mnt/fod
```

Opcje specyficzne dla FOD możesz przekazać przez `-o`, na przykład:

```bash
mount.fod /mnt/fod -o role=auto,selinux=off,acl=off,default_permissions
```

Jeśli chcesz, żeby mount był widoczny także dla innych użytkowników niż właściciel mounta, dodaj `allow_other` i upewnij się, że `/etc/fuse.conf` zawiera `user_allow_other`. Bez tego FUSE nie pozwoli FOD wystawić mounta innym użytkownikom, nawet jeśli sam filesystem jest zapisywalny.

Jeśli potrzebujesz własnego pliku konfiguracyjnego, ustaw `FOD_CONFIG` przed uruchomieniem helpera:

```bash
FOD_CONFIG=/ścieżka/do/fod_config.ini mount.fod /mnt/fod
```

Co sprawdzają testy:

- `make test-files` sprawdza create/write/truncate/rename/unlink.
- `make test-directories` sprawdza mkdir/rmdir/rename/stat/ls na drzewach katalogów oraz potwierdza, że `unlink()` na katalogu kończy się `EPERM`.
- `make test-metadata` sprawdza stat, chmod, chown, read, write, touch, truncate, access, stabilne raportowanie `st_dev` oraz aktualizacje `ctime`/`mtime`/`atime` przy zmianach metadanych, w tym jawne semantyki `touch -a` i `touch -m` oraz no-op `truncate` dla niezmienionego rozmiaru.
- `make test-write-noop` sprawdza, że zero-length `write()` jest no-op i nie podbija `ctime`, `mtime` ani rozmiaru pliku.
- `make test-symlink` sprawdza `ln -s`, `readlink`, `cat` przez symlink, `mv` na samym symlinku oraz przypadek osieroconego symlinka po usunięciu targetu. Test pokazuje też uszkodzony link przez `ls -al` na samej ścieżce symlinka.
- `make test-destroy` sprawdza, że `destroy()` flushuje bufory i zostawia dane trwałe dla nowej instancji FOD.
- `make test-journal` sprawdza, że journal zapisuje główne operacje mutujące w kolejności i przechowuje aktualny uid procesu.
- `make test-locking` sprawdza semantykę locków i zachowanie własności, w tym konflikty zakresów, współistnienie shared locków i czyszczenie po unlock.
- `make test-pg-lock-manager` sprawdza produkcyjny backend locków oparty o PostgreSQL z TTL i heartbeat, w tym regresję dla dwóch klientów piszących do tego samego pliku oraz Rust smoke coverage dla dwóch primary mountów i repliki.
- `make test-permissions` sprawdza egzekwowanie sticky bit przy `unlink`/`rmdir`, odrzucanie `chmod` na symlinkach, root-only `chown` na symlinkach, sprawdzanie właściciela/roota plus `chown` z uwzględnieniem grup dodatkowych, traktowanie `chown(-1, -1)` jako no-op, traktowanie `chown` z niezmienioną własnością jako no-op zarówno na plikach, jak i katalogach, traktowanie `chmod` z niezmienionym trybem jako no-op zarówno na plikach, jak i katalogach, zdejmowanie `setuid`/`setgid` przy zmianie własności zwykłych plików oraz zachowanie `setgid` na katalogach przy jednoczesnym zdejmowaniu `setuid` po zmianie własności.
- `make test-utimens-noop` sprawdza, że `utimens` z niezmienionymi timestampami jest no-op i nie podbija `ctime` zarówno na zwykłych plikach, jak i katalogach.
- Uwagi zgodności z `pjdfstest`: FOD zostawia `unlink()` na katalogach jako `EPERM`, zachowuje bit `setgid` katalogów przy zmianach własności i traktuje przypadki brzegowe `utimens` oraz zmian własności zgodnie z zachowaniem Linux/POSIX widocznym w tym zestawie testów.
- `make test-hardlink` sprawdza tworzenie hardlinków, rename i zachowanie link count przez backend.
- `make test-fallocate` sprawdza preallocation i wzrost wypełniony zerami przez backend.
- `make test-copy-file-range` sprawdza kopiowanie danych z offsetami przez backend.
- `make test-ioctl` sprawdza wsparcie `FIONREAD` przez backend.
- `make test-mknod` sprawdza tworzenie FIFO i char-device oraz raportowanie `stat` typu i `rdev`. `open` dla special node'ów nadal jest unsupported.
- `make test-lseek` sprawdza backendowy seek helper dla `SEEK_SET`, `SEEK_CUR` i `SEEK_END`.
- `make test-poll` sprawdza backendowy poll helper dla plików regularnych.
- `make test-access-groups` sprawdza `access()` dla właściciela, grupy podstawowej i grup dodatkowych.
- `make test-inode-model` sprawdza, że `st_ino` przeżywa rename i restart FOD dla katalogów, plików, hardlinków i symlinków.
- Model inode używa trwałych `inode_seed`, a hot-path query są oparte o `UNION ALL` oraz indeksy na `hardlinks.id_file` i `data_blocks(id_file, _order)`.
- `make test-ownership-inheritance` sprawdza, że `chmod`/`chown` na katalogu z `setgid` powoduje dziedziczenie `gid` przez nowe dzieci, a `rename` zachowuje metadane źródła i `mkdir` propaguje `setgid` do nowych podkatalogów.
- `make test-rename-root-conflict` sprawdza replace semantics dla plików i katalogów oraz edge-case'y dla `rename` na root.
- `make test-statfs-use-ino` sprawdza, przez mały shell smoke, że inode widoczne na mountcie zgadzają się z backendem, a `statvfs()` zwraca te same wartości filesystemowe co backendowy helper `statfs()`.
- `make test-mount-root-permissions` sprawdza świeży mount root oraz zachowanie chmod/chown/write dla katalogu na nowo zamontowanym filesystemie.
- `make test-atime-noatime` sprawdza zachowanie `atime` FOD w trybie `noatime` i potwierdza, że odczyt nie podnosi `atime`.
- `make test-atime-relatime` sprawdza zachowanie `atime` FOD w trybie `relatime` i potwierdza, że stary `atime` aktualizuje się po odczycie.
- `make test-timestamp-touch-once` sprawdza relatime-style one-touch dla pliku i katalogu, potwierdzając, że pierwszy stary odczyt/listing podnosi `atime`, a drugi już nie.
- `make test-atime-benchmark` wypisuje krótki baseline wall-time dla zachowania `atime` FOD na odczytach plików i listowaniu katalogów, żeby porównać uruchomienia `default`, `noatime` i `nodiratime` bez długiej pętli smoke.
- `make test-pool-connections` sprawdza, że FOD startuje pulę PostgreSQL z ustawionym limitem połączeń.
- `make test-mount-suite` to główny Pythonowy mount smoke suite; obejmuje pliki, katalogi, metadane, access modes, symlinki, `ioctl/FIONREAD`, `read`-driven `atime` dla plików, runtime-off dla ACL/SELinux, SELinux-on gdy jest włączony, `df` i tryb read-only dla repliki.
- `make test-throughput` uruchamia prosty benchmark `dd if=/dev/zero` na zamontowanym FOD i wypisuje czas oraz MiB/s.
- `make test-throughput-sync` to wariant z `conv=fsync`.
- `make test-large-copy-benchmark` mierzy duży transfer `copy_file_range()` przez backend i wypisuje czas oraz MiB/s.
- `make test-large-file-multiblock-benchmark` mierzy duży zapis wieloblokowego pliku i wypisuje czasy write/persist/flush.
- `make test-remount-durability-benchmark` sprawdza, że dane przeżywają cykl stop/remount/reopen i wypisuje czas round-trip.
- `make test-tree-scale` benchmarkuje `getattr` i `readdir` na większym, zasilonym drzewie i pokazuje czasy `ls`/`find`.
- `make test-flush-release-profile` sprawdza, że czyste `flush()` / `release()` są tanie, a dirty flush persystuje dane dokładnie raz.
- `make test-write-flush-threshold` sprawdza, że niski próg auto-flush potrafi wypchnąć dirty dane przed zamknięciem i że bufor nie zostaje dirty po zapisie.
- `make test-all-full` rozszerza `make test-all` o workflow dla files/directories/metadata/symlink, shellowy smoke `statfs/use_ino`, mount workflow, oba smoke profile `atime` i benchmark throughput.

`make test-all` zawiera check xattr/SELinux/trusted/ACL oraz złożony mount smoke suite.
Mount repliki można wymusić przez `--role replica`. Domyślne `--role auto` wykrywa replikę przez `pg_is_in_recovery()` i montuje filesystem jako read-only. Jeśli chcesz tylko mount read-only bez zmiany roli na replikę, użyj `-o ro`.

Aktualne baseline'y porównawcze dla throughput, dużego copy, dużych wieloblokowych plików, durability po remount, read cache i zachowania `atime` są zapisane w [BENCHMARKS.md](BENCHMARKS.md). Na tym hoście uruchomienie `THROUGHPUT_SYNC=1` pozostało w podobnym zakresie wydajności jak wariant bez `fsync`, a największy batch był minimalnie lepszy, więc `synchronous_commit` zostaje knobem strojenia, a nie domyślną rekomendacją dla wszystkich workloadów. Porównanie `bulk_write` vs `metadata_heavy` dla dużego copy jest już baseline'em, a Rust POC w `rust_hotpath/` obejmuje teraz copy planner, helper changed-copy dedupe i changed-run packer; historyczne wzmianki o Pythonie w tych porównaniach są zachowane wyłącznie jako baseline'e migracyjne.

## Opcje runtime

Jeśli potrzebujesz `allow_other`, uruchom mount z `FOD_ALLOW_OTHER=1`, ale tylko wtedy, gdy `/etc/fuse.conf` na to pozwala.
W `/etc/fod/fod_config.ini` można też dodać sekcję `[fod]` z `pool_max_connections = N`, żeby ograniczyć budżet połączeń PostgreSQL używany przez cache'owane połączenia runtime. Ta sama sekcja może także ustawiać domyślne parametry storage/read, takie jak `write_flush_threshold_bytes`, `max_fs_size_bytes`, `read_cache_blocks`, `read_ahead_blocks`, `sequential_read_ahead_blocks`, `small_file_read_threshold_blocks`, `metadata_cache_ttl_seconds` i `statfs_cache_ttl_seconds`. `max_fs_size_bytes` przyjmuje zwykłe bajty albo binarne rozmiary typu `50GiB` czy `1TiB`, a `pg_visible_path` pozwala wskazać ścieżkę, którą PostgreSQL faktycznie widzi na dysku, żeby `statfs()` mógł ograniczyć raportowany rozmiar do rzeczywistego. Jeśli tego pliku nie ma, FOD użyje `fod_config.ini` z katalogu projektu.
Ta sama sekcja może też ustawiać parametry wielowątkowości dla większych odczytów i kopiowania, takie jak `workers_read`, `workers_read_min_blocks`, `workers_write` i `workers_write_min_blocks`, oraz `persist_buffer_chunk_blocks`, które decyduje o wielkości paczek flushu. `persist_block_transport` wybiera sposób zapisu bloków: `copy_binary_staging` (domyślnie), `binary_bytea` albo `legacy_hex`. `workers_read` jest używane tylko wtedy, gdy brakujące bloki w odczycie dzielą się na kilka rozłącznych zakresów, a `workers_write` tylko wtedy, gdy kopiowanie można podzielić na kilka segmentów źródłowych. `block_size` nadal ma znaczenie, bo heurystyki workerów działają na blokach, a nie na surowych bajtach, więc mniejszy albo większy blok zmienia moment, w którym wielowątkowość zaczyna mieć sens, ale nie oznacza automatycznie "4 KiB = jeden wątek". Dla powtarzanych kopii typu rsync można też włączyć `copy_dedupe_enabled`, żeby porównywać bloki docelowe i pomijać niezmienione zakresy podczas `copy_file_range()`. `copy_dedupe_min_blocks` jest dolną bramką, `copy_dedupe_max_blocks` opcjonalnym górnym limitem dla bardzo dużych plików, a `copy_dedupe_crc_table` może przy tym utrzymywać tabelę CRC w PostgreSQL i uzupełniać ją lazy podczas porównań. Knoby dedupe zostają domyślnie wyłączone, jeśli nie wiesz, że workload faktycznie na tym korzysta. `lock_heartbeat_interval_seconds` steruje zarówno odświeżaniem lease'ów locków PostgreSQL, jak i heartbeatem `client_sessions` na writable primary mountach. Dzięki temu backend może wykrywać martwe mounty i zwalniać ich stan po wygaśnięciu TTL. Gdy martwy `client_sessions` zostanie usunięty, trigger w PostgreSQL czyści lock leases i range leases dla jego `owner_key`'ów. Może też ustawiać `synchronous_commit`, żeby sterować trwałością sesji PostgreSQL dla każdego połączenia; dozwolone wartości to `on`, `off`, `local`, `remote_write` i `remote_apply`.
O ile nie zaznaczono inaczej, numeryczne parametry runtime są nieujemne; `0` wyłącza odpowiedni cache albo limit tam, gdzie kod to obsługuje. `lock_lease_ttl_seconds`, `lock_heartbeat_interval_seconds`, `lock_poll_interval_seconds` i `persist_buffer_chunk_blocks` muszą być większe od zera. `max_fs_size_bytes` przyjmuje dodatni rozmiar albo może zostać pominięty, jeśli filesystem ma działać bez limitu.
Przy starcie mounta FOD loguje aktywny profil runtime, `FOD version`, `FOD schema name`, `FOD schema version`, ustawienia PostgreSQL TLS, trwałość sesji PostgreSQL (`synchronous_commit`), strojenie storage, opcje mounta i backend locków, żebyś mógł sprawdzić aktywną konfigurację bez zgadywania, które domyślne wartości zostały użyte.
Jeśli chcesz gotowy preset produkcyjny, ustaw `FOD_PROFILE=bulk_write`, `FOD_PROFILE=metadata_heavy` albo `FOD_PROFILE=pg_locking` przed mountem. Jeśli chcesz opt-in preset dla sekwencyjnego PoC extentów, ustaw `FOD_PROFILE=extents`. Wybrany profil nadpisuje bazowe wartości z `[fod]` w `fod_config.ini`.
Profil możesz też podać jawnie jako `--profile bulk_write` do `fod-bootstrap` albo jako `-o profile=bulk_write` do `mount.fod`.
Ta sama zmienna `FOD_PROFILE` działa też z `make mount`, `make mount-user` i `make demo`.
Do dynamicznego strojenia użyj `make change-runtime-list`, `make change-runtime-get` i `make change-runtime-set`, które korzystają z `fod.change`; target `change-runtime-set` oczekuje `FOD_CHANGE_KEY`, `FOD_CHANGE_VALUE` i `FOD_CHANGE_PASSWORD`.
Jeśli zmieniłeś tylko reloadowalne parametry w `fod_config.ini`, użyj `make reload-runtime` (albo aliasu `make change-runtime-sync`), aby przepchnąć bieżący config do działającego mounta przez `fod.change` bez remountu; target sync nie potrzebuje hasła schema-admin, bo tylko odtwarza reloadowalny snapshot z bieżącego configu.
Na writable primary FOD używa backendu locków PostgreSQL lease, a każdy mount read-only, także `--role replica` albo `-o ro`, przełącza się na backend pamięciowy, bo mount i tak jest tylko do odczytu. Testy locków sprawdzają zarówno konflikt między dwoma primary mountami, jak i rozdzielenie primary/replica na tej samej bazie.

Wsparcie xattr dla SELinux jest sterowane przez `--selinux auto|on|off` albo `FOD_SELINUX=auto|on|off`.
Domyślnie jest `off`. `on` wymusza aktywację, a `auto` używa wykrywania po stronie hosta.
Wsparcie POSIX ACL jest sterowane przez `--acl on|off` albo `FOD_ACL=on|off`.
Domyślnie jest `off`.
Przy starcie FOD loguje efektywny profil runtime, wersję schematu, ustawienia TLS PostgreSQL, trwałość sesji PostgreSQL (`synchronous_commit`), tuning storage, opcje mounta i backend locków, więc można łatwo sprawdzić, jakie wartości faktycznie zostały zastosowane.
`FOD_WRITE_FLUSH_THRESHOLD_BYTES` steruje tym, ile dirty danych może się zebrać, zanim FOD auto-persystuje duży bufor podczas `write()`, `truncate()`, `fallocate()` albo `copy_file_range()`. Domyślna wartość to `67108864` bajtów.
`metadata_cache_ttl_seconds` steruje krótkim cache TTL dla odczytów metadanych `getattr()` i `readdir()`. Domyślna wartość to `1` sekunda.
`statfs_cache_ttl_seconds` steruje krótkim cache TTL dla `statfs()`. Domyślna wartość to `2` sekundy.
`FOD_METADATA_CACHE_TTL_SECONDS` i `FOD_STATFS_CACHE_TTL_SECONDS` nadpisują odpowiednie wartości z `fod_config.ini`, jeśli chcesz stroić te cache per środowisko.
`FOD_PROFILE` wybiera nazwany profil runtime z `fod_config.ini`, na przykład `bulk_write`, `metadata_heavy` albo `extents`.
`FOD_ATIME_POLICY` jest wewnętrznym przełącznikiem FOD, a nie surową opcją mounta FUSE. Steruje tym, kiedy FOD aktualizuje `atime` w swoim własnym read path; `noatime`, `nodiratime`, `relatime` i `strictatime` są obsługiwane wewnętrznie i nie są przekazywane do frontendu mounta.
Dla jednego uchwytu FOD zapisuje `access_date` tylko raz, aby nie przepisywać ciągle tego samego rekordu podczas pojedynczej sekwencji open/read lub open/readdir. Kolejne dotknięcia są pomijane aż do zwolnienia uchwytu.
Ten sam model dotyczy też zapisu `mtime`/`ctime`: wiele zapisów na tym samym otwartym pliku aktualizuje te znaczniki dopiero przy persystencji dirty bufora, a nie przy każdym pośrednim wywołaniu `write()`.
Cache odczytu można ustawić przez `FOD_READ_CACHE_EVICTION_POLICY`; obecny domyślny wariant to FIFO, a sekwencyjne odczyty automatycznie zwiększają read-ahead, dzięki czemu sąsiednie odczyty częściej trafiają w prefetche zamiast ponownie walić w PostgreSQL.

### Dockerowy lab SELinux/ACL

Do pracy z uprawnieniami i xattrami, gdy potrzebujesz razem `FOD_USE_FUSE_CONTEXT=1`, `--acl on` i `--selinux on`, użyj opcjonalnego stacka `docker-compose.selinux-acl.yml` oraz serwisu `fod-selinux-acl`. `make docker-selinux-acl-up` uruchamia lab, a `make docker-selinux-acl-shell` otwiera shell w kontenerze, żeby można było tam uruchamiać mounty i smoke testy.
`make docker-selinux-acl-smoke` uruchamia w tym kontenerze zestaw smoke: identyfikację kontekstu FUSE, xattr SELinux/ACL oraz test root-owned permissions. Lab montuje `docker/selinux-acl/fod_config.ini` nad repo root configiem, żeby testy w kontenerze łączyły się do compose'owego `postgres`, a nie do localhosta hosta.
Ten lab nadal wymaga hosta/runtime, które wspierają etykiety SELinux; Docker nie włączy SELinux, jeśli kernel ma go wyłączonego.

## Backup i restore

Backup i restore FOD to w praktyce backup i restore PostgreSQL.

1. Użyj `pg_dump` / `pg_dumpall` albo standardowych narzędzi backupu PostgreSQL.
1. Odtwarzaj do instancji PostgreSQL zgodnej z wersją schematu FOD.
1. Po restore możesz uruchomić `make test-schema-upgrade`, żeby szybko sprawdzić bezpieczeństwo `init` i naprawę wersji schematu.
1. Trzymaj dump bazy i użyty profil `fod_config.ini` razem, żeby restore wrócił do tego samego baseline strojenia.

Opcje widoczne w mount:

- `--default-permissions` jest włączone domyślnie; wyłącz przez `--no-default-permissions`, jeśli chcesz tylko checks FUSE.
- Zachowanie `atime` FOD można wybrać przez `--atime-policy default|noatime|nodiratime|relatime|strictatime`.
- `noatime` wyłącza aktualizację `atime` dla odczytów plików i listowania katalogów; `nodiratime` wyłącza aktualizację `atime` katalogów, ale zostawia aktualizację `atime` plików.
- Dostępne są też `--lazytime`, `--sync` i `--dirsync`.
- Label SELinux można podać przez `FOD_SELINUX_CONTEXT`, `FOD_SELINUX_FSCONTEXT`, `FOD_SELINUX_DEFCONTEXT` i `FOD_SELINUX_ROOTCONTEXT`.
- Ustaw `FOD_LOG_LEVEL=DEBUG`, jeśli chcesz pełne diagnostyczne tracebacki; domyślnie jest `INFO`, więc oczekiwane przypadki `ENODATA` nie będą zaśmiecały logów.
- `--acl on` jest wymagane, jeśli chcesz egzekwować ACL podczas runtime; inaczej xattr ACL pozostają nieaktywne.
- `--selinux on` lub `--selinux auto` jest wymagane, jeśli chcesz, żeby `security.selinux` było aktywne podczas runtime; inaczej xattr SELinux pozostają nieaktywne.
- `make test-mount-suite` zawiera zarówno smoke dla SELinux-off, jak i SELinux-on; przypadek SELinux-on jest pomijany automatycznie, jeśli mount nie startuje z `FOD_SELINUX=on|auto`.
- FOD przechowuje etykiety SELinux jako xattr i steruje nimi w runtime; nie implementuje samodzielnie pełnej polityki label mount.
- To zachowanie jest celowe: w tym repo pełna polityka mount-label jest poza zakresem, a zachowanie SELinux opiera się na host policy plus przechowywaniu xattr.
- `mknod` tworzy FIFO i char device metadata; `st_rdev` i `st_dev` są raportowane, ale `open` dla special node'ów nadal jest unsupported.
- `system.posix_acl_*` działa dla access ACL i default ACL inheritance; backend zapisuje, propaguje i egzekwuje ACL.
- `poll` działa przez Rustowy frontend mounta dla zwykłych plików.

## Troubleshooting

- Zacznij od `mkfs.fod status`, żeby zobaczyć, czy sekret administracyjny schematu jest obecny i czy FOD jest gotowy.
- Jeśli `mkfs.fod init` kończy się błędem, sprawdź czy PostgreSQL działa i czy dane w `fod_config.ini` zgadzają się z serwerem.
- Jeśli montowanie kończy się `fod schema is not initialized`, uruchom najpierw `make init`; dla operacji `mkfs.fod` zawsze podawaj `--schema-admin-password`.
- Jeśli montowanie kończy się `fod schema version mismatch`, uruchom `mkfs.fod upgrade` z sekretem administracyjnym schematu, żeby schemat `fod` zgadzał się z kodem.
- Przy udanym starcie mounta FOD loguje `FOD version=<release> FOD schema name=fod FOD schema version=<db> initialized=<bool>`, więc możesz od razu potwierdzić zgodność wersji przed użyciem mounta.
- Jeśli montowanie kończy się `ENOTCONN` albo błędem połączenia, uruchom najpierw `make smoke`, żeby potwierdzić łączność z bazą.
- Jeśli brakuje `fusermount3`, spróbuj `fusermount` albo doinstaluj narzędzia userspace FUSE dla swojej dystrybucji.
- Jeśli `allow_other` jest ignorowane albo inni użytkownicy nie widzą mounta, sprawdź `/etc/fuse.conf` i upewnij się, że `user_allow_other` jest włączone.
- Jeśli ACL albo SELinux wyglądają na nieaktywne, upewnij się, że mount został uruchomiony z `--acl on` albo `--selinux on|auto`.

## Rekomendowane profile mounta

| Profil | Zastosowanie | Kluczowe opcje |
| --- | --- | --- |
| `fod-relaxed` | Lokalny dev i smoke testy | `--no-default-permissions`, `FOD_ACL=off`, `FOD_SELINUX=off`, `--atime-policy default` |
| `fod-linux-default` | Najbliżej typowego mounta Linuksa | `--default-permissions`, `FOD_ACL=off`, `FOD_SELINUX=off`, `--atime-policy relatime` |
| `fod-selinux` | Środowiska z SELinux | `--default-permissions`, `FOD_ACL=on`, `FOD_SELINUX=auto` albo `on`, `FOD_SELINUX_CONTEXT` według potrzeb |

## Rekomendowane workloady

| Profil runtime | Dobry dla | Dlaczego |
| --- | --- | --- |
| `fod-relaxed` | Lokalny development, smoke runy i szybkie testy ręczne | Najmniej restrykcyjna polityka i najluźniejsza semantyka mounta. |
| `fod-linux-default` | Mieszane workloady z zachowaniem zbliżonym do typowego mounta Linuksa | Zbalansowane ustawienia dla ACL-off, SELinux-off i zachowania podobnego do relatime. |
| `bulk_write` | Duży ingest sekwencyjny, `copy_file_range()`, testy throughputu, durability po remount | Większe batchowanie flush i bardziej agresywne strojenie strony zapisu. |
| `metadata_heavy` | `ls`, `find`, `stat`, przeglądanie głębokich drzew, operacje tylko na metadanych | Dłuższy TTL cache metadanych i bardziej zachowawcza presja na write path. |
| `pg_locking` | Koordynacja wielu klientów i testy regresji locków | Strojenie backendu locków z krótszym poll interval do sprawdzania lease'ów. |
| `extents` | Opt-in sekwencyjny PoC extentów i smoke porównawczy | Trzyma `enable_extents = true` jawnie, bez zmiany reszty baseline'u. |

## Antywzorce

- Nie używaj `bulk_write` do nawigacji po metadanych albo pracy na wielu małych plikach; ten profil jest pod throughput, nie pod niską latencję namespace.
- Nie używaj `metadata_heavy` do dużego sekwencyjnego ingestu albo `copy_file_range()`; ten profil jest świadomie bardziej zachowawczy po stronie zapisu.
- Nie używaj `fod-relaxed` dla wieloużytkowych albo produkcyjnych mountów, gdzie potrzebujesz bardziej linuksowej semantyki uprawnień.
- Nie traktuj `synchronous_commit=off` jako domyślnego ustawienia trwałości; stosuj je tylko wtedy, gdy workload akceptuje kompromis i benchmark pokazuje sens.
- Nie oczekuj, że `pg_locking` sam poprawi throughput zapisu; ten profil dotyczy koordynacji i semantyki, a nie przyspieszania data path.
- Nie używaj `extents` jako domyślnego presetu produkcyjnego; to jawny profil PoC dla sekwencyjnych extentów.

## Historyczna Notatka Architektury

Aktualny runtime jest w pełni Rustowy. Notatki poniżej są zachowane wyłącznie jako kontekst migracyjny i nie opisują aktywnej ścieżki fallback w Pythonie.

- W erze Pythona bootstrap, `mkfs`, ładowanie configów i profili, callbacki FUSE, logika administracyjna, migracje schematu, testy integracyjne oraz warstwy polityk typu ACL/permissions/journal/runtime validation żyły w Pythonie.
- Rust teraz odpowiada za runtime hot-path i core storage opisane wyżej.
