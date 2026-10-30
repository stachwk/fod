# Mapa błędów POSIX dla operacji plikowych

To jest roboczy, ograniczony wycinek audytu P12, a nie deklaracja zgodności.
Rozróżnia błędy, dla których opis POSIX mówi `shall fail`, od błędów
`may fail`. Wymienione błędy dotyczą tylko warunków wejściowych i typów
obiektów objętych daną pozycją. Zachowanie Linux VFS może zakończyć operację
przed wywołaniem callbacku FUSE; testy zamontowane w FOD potwierdzają wynik
wywołania systemowego, ale same nie dowodzą, która warstwa zwróciła errno.

Wiersze `open`, `read`/`pread`, `write`/`pwrite`, podstawowe operacje namespace,
metadane, blokady rekordowe oraz `statvfs`/`fstatvfs` sprawdzono względem tekstu
POSIX Issue 8. Warunki oznaczone XSI są oddzielone od wymagań bazowego zakresu
FOD. Pełne przypisanie errno do warstwy VFS, libfuse lub FOD i regresje dla
pozostałych warunków pozostają do sprawdzenia w P12.

| Interfejs | POSIX: wymagane błędy w zakresie FOD | POSIX: warunkowe, `may fail` lub XSI | Dowody FOD i otwarte sprawdzenia |
| --- | --- | --- | --- |
| `open()` | `EACCES`, `EEXIST`, `EILSEQ` gdy `O_CREAT` nie może utworzyć nazwy nieprzenośnej, `EINTR`, `EISDIR`, `ELOOP`, `EMFILE`, `ENAMETOOLONG` dla komponentu dłuższego niż `NAME_MAX`, `ENFILE`, `ENOENT`, `ENOSPC`, `ENOTDIR`, `ENXIO` (nieblokujący zapis do FIFO bez czytelnika), `EOVERFLOW`, `EROFS`; `EINVAL` jeśli podano flagi synchronizowanego I/O, których implementacja nie obsługuje | `EAGAIN` dla zablokowanego PTY (XSI); `EINVAL` dla nieprawidłowej opcji; `ELOOP` przy przekroczeniu `SYMLOOP_MAX`; `ENAMETOOLONG` dla `PATH_MAX`; `EOPNOTSUPP` dla socketu; `ETXTBSY` dla wykonywanego pliku | `O_EXCL`, tryby deskryptora i błędy ścieżek mają regresje w [`test_mount_suite.py`](../tests/integration/test_mount_suite.py), [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_symlink.sh`](../tests/integration/test_symlink.sh), [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py), [`test_df.sh`](../tests/integration/test_df.sh) i [`test_mknod.py`](../tests/integration/test_mknod.py). Test w `test_path_trailing_slash.py` sprawdza `EILSEQ` i brak aliasowania nie-UTF-8 nazwy. Limit deskryptorów, `EINTR` i `EOVERFLOW` nie mają osobnych regresji. |
| `read()` / `pread()` | `EBADF`; `EINTR` gdy sygnał przerwie odczyt przed przekazaniem danych; `EOVERFLOW` dla pozycji poza limitem offsetu; `EINVAL` dla ujemnego offsetu `pread()`; `ESPIPE` dla `pread()` na nieprzeszukiwalnym obiekcie; `EAGAIN` dla nieblokującego FIFO, gdy zapisujący pozostaje otwarty | `EISDIR` dla katalogu jest oznaczone XSI; `EIO` w warunkach terminala albo błędu fizycznego; `ENXIO` dla nieistniejącego urządzenia; błędy STREAMS/socket dotyczą zakresu spoza FOD | [`test_lseek.py`](../tests/integration/test_lseek.py) sprawdza EOF, offsety, niezgodny tryb deskryptora i ujemny offset `pread()`; [`test_mknod.py`](../tests/integration/test_mknod.py) sprawdza FIFO. Sygnałowe przerwanie, `EOVERFLOW` i błąd odczytu `EIO` wymagają osobnych testów albo potwierdzonego zachowania Linux VFS; `test_mknod.py` asercjami montowanymi sprawdza `ESPIPE` dla `pread()` FIFO. |
| `write()` / `pwrite()` | `EBADF`, `EFBIG` (limit rozmiaru pliku lub offsetu), `EINTR` gdy nie zapisano danych, `ENOSPC`; `EINVAL` dla ujemnego offsetu `pwrite()`; `ESPIPE` dla `pwrite()` na nieprzeszukiwalnym obiekcie; `EPIPE` dla `write()` do FIFO bez czytelników; `EAGAIN` dla nieblokującego FIFO, gdy zapis opóźniłby wątek | `EIO` przy fizycznym błędzie I/O; błędy terminala, STREAMS/socket/urządzeń są poza zakresem FOD | [`test_lseek.py`](../tests/integration/test_lseek.py) sprawdza tryby deskryptora, zapis pozycyjny, rozszerzanie pliku i ujemny offset `pwrite()`; [`test_two_mount_quota.py`](../tests/integration/test_two_mount_quota.py) i [`test_df.sh`](../tests/integration/test_df.sh) sprawdzają `ENOSPC`; [`test_mknod.py`](../tests/integration/test_mknod.py) sprawdza zapis do FIFO i dokładne `EPIPE` po zamknięciu ostatniego czytelnika. [`test-postgres-enospc`](../rust_fuse/tests/postgres_error_mapping.rs) wymusza PostgreSQL SQLSTATE `53100` podczas zapisu bloku i potwierdza `ENOSPC` przy `fsync()`. Sygnałowe przerwanie i limity `off_t` wymagają dedykowanych regresji albo potwierdzonego zachowania Linux VFS; `test_mknod.py` sprawdza montowane `ESPIPE` dla `pwrite()` FIFO, nieblokujące `EAGAIN` odczytu i `EPIPE` zapisu bez czytelników. |
| `mkdir()` | `EACCES`, `EEXIST`, `EILSEQ` dla nieprzenośnej nazwy, której nie można utworzyć, `ELOOP`, `EMLINK`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOSPC`, `ENOTDIR`, `EROFS` | `EIO`, przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` | [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza `EEXIST`, brakującego rodzica `ENOENT`, rodzica będącego plikiem `ENOTDIR`, `ELOOP` dla pętli symlinków w rodzicu, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX` i zachowanie pliku-rodzica po błędzie. [`test_path_search_permissions.py`](../tests/integration/test_path_search_permissions.py) sprawdza dokładne `EACCES` przy braku praw rodzica; `test_mount_suite.py::test_replica_read_only` sprawdza dokładne `EROFS` na replice przy dostępnym do zapisu rodzicu. `EILSEQ` komponentu sprawdza [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py). [`test-postgres-enospc`](../rust_fuse/tests/postgres_error_mapping.rs) wymusza PostgreSQL SQLSTATE `53100` podczas `mkdir()` i zapisu bloku, sprawdzając widoczne `ENOSPC`, oraz `58030` podczas `mkdir()`/`rmdir()`, sprawdzając `EIO` i rollback namespace; pozostałe warunki wymagają regresji lub potwierdzenia wcześniejszej obsługi przez Linux VFS. |
| `rmdir()` | `EACCES`, `EBUSY`, `EEXIST` lub `ENOTEMPTY` dla niepustego katalogu, `EINVAL` dla końcowego `.`, `EIO`, `ELOOP`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOTDIR`, `EROFS` | `EPERM`/`EACCES` dla ochrony sticky directory są XSI; przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` jest `may fail` | [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza `EBUSY` dla korzenia zamontowanego systemu plików, `EINVAL` dla końcowego komponentu `.`, `ENOENT`, `ENOTDIR` dla pliku zamiast katalogu i dla symlinku wskazującego katalog, `ELOOP` dla pętli symlinków w rodzicu, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX` oraz `ENOTEMPTY`. [`test_path_search_permissions.py`](../tests/integration/test_path_search_permissions.py) sprawdza dokładne `EACCES` przy braku praw rodzica; `test_mount_suite.py::test_replica_read_only` sprawdza dokładne `EROFS` na replice dla pustego katalogu w dostępnym do zapisu rodzicu. Niepusty katalog może zgodnie z POSIX zwrócić `EEXIST` albo `ENOTEMPTY`; FOD zwraca `ENOTEMPTY`; `test-postgres-enospc` wymusza SQLSTATE `58030` i potwierdza `EIO` z zachowaniem katalogu. `test_permissions.py` sprawdza dokładne `EPERM` dla obcego użytkownika usuwającego podkatalog w katalogu sticky. |
| `opendir()` / `readdir()` / `closedir()` | `opendir()`: `EACCES`, `ELOOP`, `EMFILE`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENFILE`, `ENOENT`, `ENOTDIR`; `readdir()`: `EOVERFLOW` gdy pole `struct dirent` nie mieści wartości | `opendir()`: `ELOOP` ponad `SYMLOOP_MAX`, `ENAMETOOLONG` ponad `PATH_MAX`; `readdir()`: `EBADF` dla nieprawidłowego strumienia, `ENOENT` dla nieprawidłowej pozycji; `closedir()`: `EBADF`, `EINTR` | [`test_readdir_small_buffers.py`](../tests/integration/test_readdir_small_buffers.py) sprawdza kontynuację `getdents64()` na małych buforach i brak pominięć/duplikatów; [`test_readdir_snapshot_multimount.py`](../tests/integration/test_readdir_snapshot_multimount.py) iteruje po 512 początkowych wpisach, zmienia zawartość przez drugi mount podczas odczytu i sprawdza stabilne wpisy, brak powtórzeń, odświeżenie po rewind oraz libc `opendir()`/`readdir()`/`closedir()` (EOF bez errno, `ENOENT` dla brakującej ścieżki i `EMFILE` po wyczerpaniu procesu). [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza `ENOTDIR` przez `getdents` na pliku i `EBADF` po zamknięciu deskryptora. FOD przypina uporządkowaną migawkę metadanych (domyślnie) lub nazw (`FOD_READDIR_BATCH_METADATA=0`) wraz z ID katalogu zapamiętanym przez `opendir()`; kontynuacja, rewind, pobieranie metadanych dzieci w trybie names i atime używają tego ID, aby stary uchwyt nie przełączył się na katalog utworzony ponownie pod tą samą ścieżką. Migawka jest zwalniana w `releasedir()`. Źródła mapują `EOVERFLOW` do konwersji ABI (Linux legacy compat `getdents` lub libc), a zamknięty fd zwraca `EBADF` w jądrze przed FUSE; callback FOD zwraca `EBADF` tylko dla nieznanego uchwytu. Nieprawidłowy cookie kończy strumień bez błędu, co jest dopuszczalne dla POSIX `ENOENT` oznaczonego `may fail`. Nadal wymagane są testy `EOVERFLOW` na obsługiwanych ABI, systemowego `ENFILE`, nieudanego `closedir()` i pozostałych błędów. |
| `unlink()` | `EACCES`, `EBUSY`, `ELOOP`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOTDIR`, `EROFS`; `EPERM` gdy wpis wskazuje katalog, a implementacja zabrania jego usunięcia przez `unlink()` | `EPERM`/`EACCES` dla ochrony sticky directory są XSI; przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` oraz `ETXTBSY` są `may fail` | [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza brakujący wpis `ENOENT` oraz linuxowe `EISDIR` przy `unlink()` katalogu. [`test_permissions.py`](../tests/integration/test_permissions.py) sprawdza dokładne `EPERM` dla obcego użytkownika usuwającego plik z katalogu sticky. POSIX Issue 8 wymienia `EPERM` dla implementacji, która zabrania takiego usunięcia; [Linux man-pages](https://man7.org/linux/man-pages/man2/unlink.2.html) potwierdza, że Linux zwraca `EISDIR` jako nie-POSIX errno. FOD zachowuje to zachowanie Linux FUSE; różnica jest świadoma i odnotowana w [`TODO.md`](../TODO.md). Test symlinków sprawdza usunięcie linku bez usunięcia celu. |
| `rename()` | `EACCES`, `EBUSY`, `EEXIST` lub `ENOTEMPTY` dla niepustego celu-katalogu, `EILSEQ` dla nieprzenośnej nazwy celu, `EINVAL` dla katalogu przenoszonego do własnego potomka lub końcowego `.`/`..`, `EIO`, `EISDIR`, `ELOOP`, `EMLINK`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOSPC`, `ENOTDIR`, `EROFS`, `EXDEV` | `EPERM`/`EACCES` dla sticky directory są XSI; przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` jest `may fail` | [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza `ENOTDIR` i brak zmian źródła/celu; [`test_rename_directory_transactional.py`](../tests/integration/test_rename_directory_transactional.py) sprawdza `ENOENT`, `ENOTEMPTY`, `ENOTDIR`, `EISDIR` i `EINVAL`/`EBUSY`; [`test_rename_root_conflict.sh`](../tests/integration/test_rename_root_conflict.sh) sprawdza `EBUSY`/`EXDEV`. Uprawnienia, `EILSEQ`, `EIO`, limity i `ENOSPC` wymagają dalszego mapowania. |
| `link()` | `EACCES`, `EEXIST`, `EILSEQ` dla nieprzenośnej nazwy celu, `ELOOP`, `EMLINK`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOSPC`, `ENOTDIR`, `EPERM` dla katalogu jako źródła, `EROFS`, `EXDEV` | Przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` jest `may fail`; reguły dostępu do źródła zależą od implementacji | [`test_hardlink.sh`](../tests/integration/test_hardlink.sh) sprawdza `EEXIST`, `ENOENT` i `EPERM`; [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza `ENOTDIR` dla celu pod plikiem. Pozostałe warunki wymagają osobnych regresji albo potwierdzenia obsługi przez VFS. |
| `symlink()` / `readlink()` | `symlink()`: `EACCES`, `EEXIST`, `EILSEQ` dla nieprzenośnej nazwy linku, `EIO`, `ELOOP`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOSPC`, `ENOTDIR`, `EROFS`. `readlink()`: `EACCES`, `EINVAL` dla obiektu niebędącego symlinkiem, `EIO`, `ELOOP`, `ENAMETOOLONG`, `ENOENT`, `ENOTDIR` | Przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` jest `may fail`; `symlink()` może zwrócić `ENAMETOOLONG`, gdy treść celu przekracza `SYMLINK_MAX` | [`test_symlink.sh`](../tests/integration/test_symlink.sh) sprawdza `EINVAL`, `ENOENT`, `ELOOP`, zachowanie dangling linków, `O_CREAT|O_EXCL` oraz usuwanie samego linku; [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza `ENOTDIR` dla rodzica będącego plikiem. Pozostałe warunki wymagają osobnych regresji albo potwierdzenia wcześniejszej obsługi przez Linux VFS. |
| `stat()` / `fstat()` | `EACCES`, `EBADF` dla nieprawidłowego deskryptora `fstat()`, `EIO`, `ELOOP`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOTDIR`, `EOVERFLOW` gdy rozmiar, liczba bloków lub numer pliku nie mieści się w `struct stat` | Przekroczenie `SYMLOOP_MAX` lub `PATH_MAX`; `EOVERFLOW` dla innego pola struktury jest `may fail` | [`test_path_search_permissions.py`](../tests/integration/test_path_search_permissions.py) sprawdza odmowę `stat()` przez katalog bez prawa wyszukiwania; [`test_mount_suite.py`](../tests/integration/test_mount_suite.py) sprawdza podstawowe pola metadanych; testy ścieżek sprawdzają `ENOENT`, `ENOTDIR` i `ELOOP`. Brak dedykowanej awarii `EIO` i testów `EOVERFLOW` dla wspieranych ABI. `fstat()` nie rozwiązuje ścieżki, więc błędy ścieżkowe nie dotyczą tej odmiany. |
| `chmod()` / `fchmod()` | `EACCES`, `EBADF` dla nieprawidłowego deskryptora `fchmod()`, `ELOOP`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOTDIR`, `EPERM` gdy wywołujący nie jest właścicielem i nie ma wymaganych uprawnień, `EROFS` | `EINTR`, `EINVAL` dla nieprawidłowego trybu, przekroczenie `SYMLOOP_MAX` lub `PATH_MAX`; `EOPNOTSUPP` dla nieobsługiwanego `AT_SYMLINK_NOFOLLOW` | [`test_path_search_permissions.py`](../tests/integration/test_path_search_permissions.py) sprawdza `EPERM` przy zmianie trybu pliku innego właściciela oraz `EACCES` przy braku prawa wyszukiwania; [`test_metadata.sh`](../tests/integration/test_metadata.sh) i [`test_permissions.py`](../tests/integration/test_permissions.py) sprawdzają poprawne zmiany trybu. `fchmod()` z nieprawidłowym deskryptorem nadal wymaga osobnego sprawdzenia. POSIX klasyfikuje `EINVAL` dla nieprawidłowego `mode` jako `may fail`; Linux VFS maskuje wejście do `S_IALLUGO`. Montowany probe FOD z bitem typu `S_IFIFO` (`0o10000`) zakończył się sukcesem z trybem `0o000`, więc test błędu `EINVAL` nie jest wymagany dla tego przypadku. `test_mount_suite.py::test_replica_read_only` sprawdza dokładne `EROFS` przy zmianie trybu repliki. |
| `chown()` / `fchown()` | `EACCES`, `EBADF` dla nieprawidłowego deskryptora `fchown()`, `ELOOP`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOTDIR`, `EPERM` gdy wywołujący nie ma wymaganych uprawnień, `EROFS` | `EIO`, `EINTR`, `EINVAL` dla UID/GID nieobsługiwanego przez implementację, przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` | [`test_path_search_permissions.py`](../tests/integration/test_path_search_permissions.py) sprawdza dokładne `EPERM` przy próbie zmiany właściciela cudzego pliku; [`test_permissions.py`](../tests/integration/test_permissions.py) sprawdza zmianę grupy i zachowanie bitów setuid/setgid. `EINVAL` i deskryptor `fchown()` nadal wymagają osobnych sprawdzeń; `test_mount_suite.py::test_replica_read_only` sprawdza dokładne `EROFS` dla replikowej zmiany właściciela/grupy. |
| `utimensat()` / `futimens()` | `EINVAL` gdy `tv_nsec` jest mniejsze od zera lub co najmniej miliard, z wyjątkiem znaczników `UTIME_NOW` i `UTIME_OMIT`, albo gdy czas nie jest obsługiwany przez system plików; `EPERM` dla jawnego czasu ustawianego przez nie-właściciela bez wymaganych uprawnień; `EROFS`; `EBADF` dla nieprawidłowego deskryptora `futimens()` | `EACCES` przy odmowie zapisu dla `UTIME_NOW`/braku tablicy czasów oraz przy braku prawa wyszukiwania w prefiksie; błędy ścieżki, przekroczenie `SYMLOOP_MAX`/`PATH_MAX`; `EINVAL` dla nieprawidłowych flag `utimensat()`; gdy oba czasy to `UTIME_OMIT`, część błędów jest `may fail` | [`test_acl_mount_option.py`](../tests/integration/test_acl_mount_option.py) sprawdza `EINVAL` dla nanosekund równych miliardowi, `EPERM` dla jawnego czasu ustawianego przez nie-właściciela z prawem zapisu ACL, `UTIME_OMIT` i `ENOENT` pominiętej ścieżki; [`test_path_search_permissions.py`](../tests/integration/test_path_search_permissions.py) sprawdza odmowę przejścia przez katalog. Pełny zestaw `UTIME_NOW`/`UTIME_OMIT` i nieprawidłowe flagi nadal wymaga sprawdzenia; `test_mount_suite.py::test_replica_read_only` sprawdza dokładne `EROFS` dla próby zmiany czasu na replice. |
| `fcntl(F_GETLK/F_SETLK/F_SETLKW)` | `EACCES` lub Linux `EWOULDBLOCK`/`EAGAIN` dla kolidującej blokady nieblokującej; `EBADF` dla nieprawidłowego deskryptora lub trybu otwarcia niezgodnego z blokadą; `EINTR` gdy sygnał przerwie oczekujące `F_SETLKW`; `EINVAL` dla nieprawidłowego polecenia/struktury albo obiektu bez blokad; `EDEADLK` gdy `F_SETLKW` spowodowałoby zakleszczenie; `ENOLCK` po przekroczeniu systemowego limitu blokad; `EOVERFLOW` dla zakresu, którego nie da się przedstawić jako `off_t` | — | POSIX Issue 8 wymaga blokad rekordowych dla plików regularnych; obsługa innych typów jest opcjonalna ([`fcntl()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/fcntl.html)). [`lock_backend_smoke.rs`](../rust_fuse/tests/lock_backend_smoke.rs) sprawdza kolizję `F_SETLK`, `EBADF` dla niezgodnego trybu deskryptora, `F_GETLK`, zakresy, przekazanie `F_SETLKW` po zwolnieniu oraz `EINTR` po `SIGUSR1`, wraz z brakiem późniejszego przyznania blokady. Linux 6.17 dla FIFO w FUSE używa lokalnych `pipefifo_fops` bez `.lock`, więc FOD callbacks nie są wywoływane. Probe z dwoma mountami primary i osobnymi procesami potwierdził niezależne przyznanie blokady FIFO; FOD nie gwarantuje blokad FIFO między mountami. Deadlock `EDEADLK`, limit `ENOLCK` i przepełnienie `off_t` nadal wymagają testów lub analizy źródeł. |
| `statvfs()` / `fstatvfs()` | `EIO`, `EINTR` dla przechwyconego sygnału, `EOVERFLOW` gdy wynik nie mieści się w strukturze; `fstatvfs()` dodatkowo `EBADF` dla nieprawidłowego deskryptora; `statvfs()` dodatkowo `EACCES`, `ELOOP`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOTDIR` | `ELOOP` po przekroczeniu `SYMLOOP_MAX` i `ENAMETOOLONG` dla ścieżki ponad `PATH_MAX` są `may fail` | [`test_df.sh`](../tests/integration/test_df.sh) sprawdza `EBADF`, ścieżkowe błędy oraz kontrakt FOD `f_ffree == f_favail == 1,000,000`; [`test_statfs_eio.py`](../tests/integration/test_statfs_eio.py) wymusza odpowiedź `EIO` callbacku i potwierdza jej dostarczenie przez `statvfs()` (nie symuluje awarii PostgreSQL). Brak testu `EINTR` i `EOVERFLOW` na wspieranych ABI. |

## Potwierdzone skróty Linux VFS (v6.17)

Te wyniki opisują warstwę Linux i nie dowodzą, że odpowiadający im callback
FOD został wywołany:

- `read()` i `write()` odrzucają nieprawidłowy deskryptor przed wejściem do VFS;
  `vfs_read()` i `vfs_write()` zwracają `EBADF`, gdy plik nie został otwarty do
  wymaganego trybu, zanim wywołają operację odczytu/zapisu systemu plików.
- `pread()` i `pwrite()` sprawdzają ujemny offset przed deskryptorem i przed
  przekazaniem operacji do `vfs_read()`/`vfs_write()`; Linux zwraca wtedy
  `EINVAL`. Test montowany w `test_lseek.py` potwierdza errno, zachowanie
  offsetu deskryptora i brak modyfikacji zawartości.
- Przy pobieraniu argumentu ścieżki Linux zwraca `ENOENT` dla pustej ścieżki
  oraz `ENAMETOOLONG`, gdy ścieżka osiąga `PATH_MAX`; te kontrole zachodzą przed
  rozwiązywaniem ścieżki i callbackiem FUSE.
- Linux v6.17 `chmod_common()` ogranicza argument `mode` przez `S_IALLUGO` i
  zachowuje istniejące bity typu inode. Montowany probe FOD z `mode=0o10000`
  (bit `S_IFIFO`) zakończył się sukcesem i ustawił prawa na `0o000`. POSIX
  wymienia `EINVAL` dla nieprawidłowego `mode` jako `may fail`, więc Linux nie
  ma obowiązku zwracać tego błędu dla dodatkowych bitów. [Źródło Linux
  v6.17 `fs/open.c`](https://github.com/torvalds/linux/blob/v6.17/fs/open.c).
- Linux v6.17 sprawdza część warunków `mkdir()` i `rmdir()` przed FUSE:
  `may_create()` zwraca `EEXIST` dla zajętej nazwy, a sprawdzenie uprawnień
  rodzica może zakończyć się `EACCES`; `do_mkdirat()` dopiero potem wywołuje
  `vfs_mkdir()`. `do_rmdir()` zwraca `EINVAL` dla końcowego `.`, `ENOTEMPTY`
  dla `..` i `EBUSY` dla korzenia ścieżki przed lookupem wpisu. `may_delete()`
  odrzuca końcowy obiekt inny niż katalog przez `ENOTDIR` (w tym symlink),
  a korzeń wpisu przez `EBUSY`; `vfs_rmdir()` zwraca `EBUSY` dla mountpointu.
  Kontrola `mnt_want_write()` może też zwrócić błąd read-only mount przed
  callbackiem. Zatem regresja syscalla może potwierdzić errno aplikacyjne,
  lecz nie przypisuje go automatycznie do callbacku FOD. Źródło: Linux v6.17
  [`fs/namei.c`](https://kernel.googlesource.com/pub/scm/linux/kernel/git/torvalds/linux/+/refs/tags/v6.17/fs/namei.c#L3167).
- W callbacku FOD `mkdir()` zwraca `EROFS` na montowaniu tylko do odczytu,
  `EILSEQ` dla nazwy spoza UTF-8 i `ENAMETOOLONG` ponad 255 bajtów. Transakcja
  PostgreSQL serializuje utworzenie nazwy; kontrola uprawnień rodzica mapuje
  się na `EACCES`, konflikt na `EEXIST`, a brak rodzica na `ENOENT`. `rmdir()`
  zwraca z callbacku `ENOENT` dla brakującego wpisu, `ENOTDIR` dla wpisu
  niebędącego katalogiem, `ENOTEMPTY` dla katalogu z dziećmi, a kontrola ACL i
  sticky bitu daje odpowiednio `EACCES`/`EPERM`. `namespace_mutation_errno()`
  mapuje PostgreSQL SQLSTATE [`53100` (`disk_full`)](https://www.postgresql.org/docs/current/errcodes-appendix.html#ERRCODES-TABLE)
  na `ENOSPC`, a pozostałe
  nierozpoznane błędy backendu na `EIO`. Odpowiedzi SQL dla tworzenia katalogu,
  pliku, symlinku, pliku specjalnego i hardlinku zachowują SQLSTATE przez
  `result_error()`; komendy w ścieżkach `rmdir()` i promocji hardlinku korzystają
  z `exec_command_params()`, które również je zachowuje. Źródłowy przegląd
  gałęzi błędnych statusów `PGresult` wykrył kilka miejsc odczytujących
  `conn_error()` po zwolnieniu wyniku; teraz pobierają `result_error()` przed
  `PQclear()`. [`test-postgres-enospc`](../rust_fuse/tests/postgres_error_mapping.rs)
  wymusza SQLSTATE `53100` i potwierdza `ENOSPC` dla `mkdir()` oraz `fsync()`;
  wymusza też SQLSTATE `58030` i potwierdza `EIO` dla `mkdir()`/`rmdir()` wraz
  z wycofaniem nieudanej zmiany namespace.
- Linux v6.17 obsługuje `FIONREAD` dla zwykłego pliku w `do_vfs_ioctl()` i
  zwraca `i_size - f_pos` przed przekazaniem polecenia do callbacku FUSE.
  Pozycja deskryptora nie występuje w żądaniu ioctl FUSE, więc callback
  userspace nie może sam odtworzyć tej wartości dla dowolnego współdzielonego
  deskryptora. Gałąź FOD `FIONREAD` nie stanowi zatem implementacji semantyki
  zwykłego pliku na tym jądrze; istniejący test zamontowany sprawdza wynik
  syscalla przy pozycji zero. Źródło: Linux v6.17
  [`fs/ioctl.c`](https://github.com/torvalds/linux/blob/v6.17/fs/ioctl.c#L2439-L2446).
- Baza FOD przechowuje nazwy w PostgreSQL `text`, więc nazwa z bajtami spoza
  UTF-8 nie może być zachowana. `validate_component_name()` zwraca dla niej
  `EILSEQ` przed użyciem `to_string_lossy()`, co zapobiega utworzeniu lub
  odczytaniu innego pliku pod nazwą zawierającą znak zastępczy. Regresja
  `non_utf8_create` sprawdza `EILSEQ` i brak takiego aliasu.

Podstawa źródłowa: Linux [`read_write.c`](https://github.com/torvalds/linux/blob/v6.17/fs/read_write.c#L3120-L3158)
(`vfs_read`), [`vfs_write`](https://github.com/torvalds/linux/blob/v6.17/fs/read_write.c#L3314-L3354),
[`pread64`/`pwrite64`](https://github.com/torvalds/linux/blob/v6.17/fs/read_write.c#L3460-L3525)
oraz [`namei.c`](https://github.com/torvalds/linux/blob/v6.17/fs/namei.c#L2390-L2524)
(`getname_flags`).

## Źródła i granice

- POSIX [`open()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/open.html)
  i [POSIX Programmer's Manual `open(3p)`](https://man7.org/linux/man-pages/man3/open.3p.html).
- POSIX [`read()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/read.html)
  i [POSIX Programmer's Manual `read(3p)`](https://man7.org/linux/man-pages/man3/read.3p.html).
- POSIX [`write()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/write.html)
  i [POSIX Programmer's Manual `write(3p)`](https://man7.org/linux/man-pages/man3/write.3p.html).
- POSIX [`mkdir()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/mkdir.html),
  [`rmdir()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rmdir.html),
  [`unlink()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/unlink.html),
  [`rename()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html),
  [`link()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/link.html),
  [`symlink()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/symlink.html)
  i [`readlink()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/readlink.html).
- POSIX [`opendir()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/opendir.html),
  [`readdir()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/readdir.html)
  i [`closedir()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/closedir.html).
- POSIX [`stat()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/stat.html),
  [`chmod()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/chmod.html),
  [`chown()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/chown.html)
  i [`utimensat()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/utimensat.html).
- POSIX [`fcntl()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/fcntl.html)
  oraz [`statvfs()`/`fstatvfs()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/statvfs.html).
- [Linux FUSE API](https://docs.kernel.org/filesystems/fuse/fuse.html) oraz
  callbacki w [`rust_fuse/src/fs.rs`](../rust_fuse/src/fs.rs).

Tabela celowo nie traktuje każdego errno wymienionego przez POSIX jako
wymagania wobec FOD: część dotyczy socketów, STREAMS, urządzeń terminalowych
lub opcji nieobjętych interfejsem FOD. Pozostaje zamknąć wskazane luki testowe
i przypisać niezweryfikowane błędy do Linux VFS, libfuse lub callbacku FOD.
