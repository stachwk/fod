# Mapa błędów POSIX dla operacji plikowych

To jest roboczy, ograniczony wycinek audytu P12, a nie deklaracja zgodności.
Rozróżnia błędy, dla których opis POSIX mówi `shall fail`, od błędów
`may fail`. Wymienione błędy dotyczą tylko warunków wejściowych i typów
obiektów objętych daną pozycją. Zachowanie Linux VFS może zakończyć operację
przed wywołaniem callbacku FUSE; testy zamontowane w FOD potwierdzają wynik
wywołania systemowego, ale same nie dowodzą, która warstwa zwróciła errno.

Wiersze `open`, `read`/`pread`, `write`/`pwrite` oraz podstawowe operacje
namespace sprawdzono względem tekstu POSIX Issue 8. Warunki oznaczone XSI są
oddzielone od wymagań bazowego zakresu FOD. Kolejne interfejsy oraz pełne
przypisanie errno do warstwy VFS, libfuse lub FOD pozostają do sprawdzenia w P12.

| Interfejs | POSIX: wymagane błędy w zakresie FOD | POSIX: warunkowe, `may fail` lub XSI | Dowody FOD i otwarte sprawdzenia |
| --- | --- | --- | --- |
| `open()` | `EACCES`, `EEXIST`, `EILSEQ` gdy `O_CREAT` nie może utworzyć nazwy nieprzenośnej, `EINTR`, `EISDIR`, `ELOOP`, `EMFILE`, `ENAMETOOLONG` dla komponentu dłuższego niż `NAME_MAX`, `ENFILE`, `ENOENT`, `ENOSPC`, `ENOTDIR`, `ENXIO` (nieblokujący zapis do FIFO bez czytelnika), `EOVERFLOW`, `EROFS`; `EINVAL` jeśli podano flagi synchronizowanego I/O, których implementacja nie obsługuje | `EAGAIN` dla zablokowanego PTY (XSI); `EINVAL` dla nieprawidłowej opcji; `ELOOP` przy przekroczeniu `SYMLOOP_MAX`; `ENAMETOOLONG` dla `PATH_MAX`; `EOPNOTSUPP` dla socketu; `ETXTBSY` dla wykonywanego pliku | `O_EXCL`, tryby deskryptora i błędy ścieżek mają regresje w [`test_mount_suite.py`](../tests/integration/test_mount_suite.py), [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_symlink.sh`](../tests/integration/test_symlink.sh), [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py), [`test_df.sh`](../tests/integration/test_df.sh) i [`test_mknod.py`](../tests/integration/test_mknod.py). Test w `test_path_trailing_slash.py` sprawdza `EILSEQ` i brak aliasowania nie-UTF-8 nazwy. Limit deskryptorów, `EINTR` i `EOVERFLOW` nie mają osobnych regresji. |
| `read()` / `pread()` | `EBADF`; `EINTR` gdy sygnał przerwie odczyt przed przekazaniem danych; `EOVERFLOW` dla pozycji poza limitem offsetu; `EINVAL` dla ujemnego offsetu `pread()`; `ESPIPE` dla `pread()` na nieprzeszukiwalnym obiekcie; `EAGAIN` dla nieblokującego FIFO, gdy zapisujący pozostaje otwarty | `EISDIR` dla katalogu jest oznaczone XSI; `EIO` w warunkach terminala albo błędu fizycznego; `ENXIO` dla nieistniejącego urządzenia; błędy STREAMS/socket dotyczą zakresu spoza FOD | [`test_lseek.py`](../tests/integration/test_lseek.py) sprawdza EOF, offsety, niezgodny tryb deskryptora i ujemny offset `pread()`; [`test_mknod.py`](../tests/integration/test_mknod.py) sprawdza FIFO. Sygnałowe przerwanie, `EOVERFLOW`, `ESPIPE` i błąd odczytu `EIO` wymagają osobnych testów albo potwierdzonego zachowania Linux VFS. |
| `write()` / `pwrite()` | `EBADF`, `EFBIG` (limit rozmiaru pliku lub offsetu), `EINTR` gdy nie zapisano danych, `ENOSPC`; `EINVAL` dla ujemnego offsetu `pwrite()`; `ESPIPE` dla `pwrite()` na nieprzeszukiwalnym obiekcie; `EPIPE` dla `write()` do FIFO bez czytelników; `EAGAIN` dla nieblokującego FIFO, gdy zapis opóźniłby wątek | `EIO` przy fizycznym błędzie I/O; błędy terminala, STREAMS/socket/urządzeń są poza zakresem FOD | [`test_lseek.py`](../tests/integration/test_lseek.py) sprawdza tryby deskryptora, zapis pozycyjny, rozszerzanie pliku i ujemny offset `pwrite()`; [`test_two_mount_quota.py`](../tests/integration/test_two_mount_quota.py) i [`test_df.sh`](../tests/integration/test_df.sh) sprawdzają `ENOSPC`; [`test_mknod.py`](../tests/integration/test_mknod.py) sprawdza zapis do FIFO. Sygnałowe przerwanie, limity `off_t`, `ESPIPE`, `EPIPE` i nieblokujące `EAGAIN` wymagają dedykowanych regresji albo potwierdzonego zachowania Linux VFS. |
| `mkdir()` | `EACCES`, `EEXIST`, `EILSEQ` dla nieprzenośnej nazwy, której nie można utworzyć, `ELOOP`, `EMLINK`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOSPC`, `ENOTDIR`, `EROFS` | `EIO`, przekroczenie `SYMLOOP_MAX` lub `PATH_MAX`; `EPERM`/`EACCES` dla ochrony sticky directory są XSI | [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza `EEXIST`, brakującego rodzica `ENOENT`, rodzica będącego plikiem `ENOTDIR` i zachowanie pliku-rodzica po błędzie. `EILSEQ` komponentu sprawdza [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py). Pozostałe warunki wymagają regresji lub potwierdzenia wcześniejszej obsługi przez Linux VFS. |
| `rmdir()` | `EACCES`, `EBUSY`, `EEXIST` lub `ENOTEMPTY` dla niepustego katalogu, `EINVAL` dla końcowego `.`, `EIO`, `ELOOP`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOTDIR`, `EROFS` | `EPERM`/`EACCES` dla ochrony sticky directory są XSI; przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` jest `may fail` | [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza `ENOENT`, `ENOTDIR` dla pliku zamiast katalogu i `ENOTEMPTY`. Niepusty katalog może zgodnie z POSIX zwrócić `EEXIST` albo `ENOTEMPTY`; FOD zwraca `ENOTEMPTY`. `EBUSY`, `EINVAL`, `EIO`, limity nazw i ochrona sticky wymagają osobnych sprawdzeń. |
| `unlink()` | `EACCES`, `EBUSY`, `ELOOP`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOTDIR`, `EROFS`; `EPERM` gdy wpis wskazuje katalog, a implementacja zabrania jego usunięcia przez `unlink()` | `EPERM`/`EACCES` dla ochrony sticky directory są XSI; przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` oraz `ETXTBSY` są `may fail` | [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza brakujący wpis `ENOENT`, a usunięcie katalogu przez `unlink()` otrzymuje `EISDIR` w callbacku FOD. Issue 8 wymienia `EPERM` dla zakazu `unlink()` katalogu; różnica Linux/FOD względem tego warunku jest zapisana jako nierozstrzygnięta w [`TODO.md`](../TODO.md). Test symlinków sprawdza usunięcie linku bez usunięcia celu. |
| `rename()` | `EACCES`, `EBUSY`, `EEXIST` lub `ENOTEMPTY` dla niepustego celu-katalogu, `EILSEQ` dla nieprzenośnej nazwy celu, `EINVAL` dla katalogu przenoszonego do własnego potomka lub końcowego `.`/`..`, `EIO`, `EISDIR`, `ELOOP`, `EMLINK`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOSPC`, `ENOTDIR`, `EROFS`, `EXDEV` | `EPERM`/`EACCES` dla sticky directory są XSI; przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` jest `may fail` | [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza `ENOTDIR` i brak zmian źródła/celu; [`test_rename_directory_transactional.py`](../tests/integration/test_rename_directory_transactional.py) sprawdza `ENOENT`, `ENOTEMPTY`, `ENOTDIR`, `EISDIR` i `EINVAL`/`EBUSY`; [`test_rename_root_conflict.sh`](../tests/integration/test_rename_root_conflict.sh) sprawdza `EBUSY`/`EXDEV`. Uprawnienia, `EILSEQ`, `EIO`, limity i `ENOSPC` wymagają dalszego mapowania. |
| `link()` | `EACCES`, `EEXIST`, `EILSEQ` dla nieprzenośnej nazwy celu, `ELOOP`, `EMLINK`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOSPC`, `ENOTDIR`, `EPERM` dla katalogu jako źródła, `EROFS`, `EXDEV` | Przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` jest `may fail`; reguły dostępu do źródła zależą od implementacji | [`test_hardlink.sh`](../tests/integration/test_hardlink.sh) sprawdza `EEXIST`, `ENOENT` i `EPERM`; [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza `ENOTDIR` dla celu pod plikiem. Pozostałe warunki wymagają osobnych regresji albo potwierdzenia obsługi przez VFS. |
| `symlink()` / `readlink()` | `symlink()`: `EACCES`, `EEXIST`, `EILSEQ` dla nieprzenośnej nazwy linku, `EIO`, `ELOOP`, `ENAMETOOLONG` dla komponentu ponad `NAME_MAX`, `ENOENT`, `ENOSPC`, `ENOTDIR`, `EROFS`. `readlink()`: `EACCES`, `EINVAL` dla obiektu niebędącego symlinkiem, `EIO`, `ELOOP`, `ENAMETOOLONG`, `ENOENT`, `ENOTDIR` | Przekroczenie `SYMLOOP_MAX` lub `PATH_MAX` jest `may fail`; `symlink()` może zwrócić `ENAMETOOLONG`, gdy treść celu przekracza `SYMLINK_MAX` | [`test_symlink.sh`](../tests/integration/test_symlink.sh) sprawdza `EINVAL`, `ENOENT`, `ELOOP`, zachowanie dangling linków, `O_CREAT|O_EXCL` oraz usuwanie samego linku; [`test_directories.sh`](../tests/integration/test_directories.sh) sprawdza `ENOTDIR` dla rodzica będącego plikiem. Pozostałe warunki wymagają osobnych regresji albo potwierdzenia wcześniejszej obsługi przez Linux VFS. |

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
- [Linux FUSE API](https://docs.kernel.org/filesystems/fuse/fuse.html) oraz
  callbacki w [`rust_fuse/src/fs.rs`](../rust_fuse/src/fs.rs).

Tabela celowo nie traktuje każdego errno wymienionego przez POSIX jako
wymagania wobec FOD: część dotyczy socketów, STREAMS, urządzeń terminalowych
lub opcji nieobjętych interfejsem FOD. Kolejny etap P12 ma rozszerzyć mapę na
metadane, blokady i pojemność oraz przypisać błędy do Linux VFS, libfuse lub
callbacku FOD.
