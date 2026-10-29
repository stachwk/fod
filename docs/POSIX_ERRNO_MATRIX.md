# Mapa błędów POSIX dla operacji plikowych

To jest roboczy, ograniczony wycinek audytu P12, a nie deklaracja zgodności.
Rozróżnia błędy, dla których opis POSIX mówi `shall fail`, od błędów
`may fail`. Wymienione błędy dotyczą tylko warunków wejściowych i typów
obiektów objętych daną pozycją. Zachowanie Linux VFS może zakończyć operację
przed wywołaniem callbacku FUSE; testy zamontowane w FOD potwierdzają wynik
wywołania systemowego, ale same nie dowodzą, która warstwa zwróciła errno.

Odnośniki do Issue 8 są punktem odniesienia dla dalszej weryfikacji. Bieżące
zestawienie oparto na pozycjach POSIX Programmer's Manual oznaczonych `3p`;
zgodność każdego warunku z wydaniem Issue 8 oraz granica VFS/FUSE pozostają do
potwierdzenia przed zamknięciem P12.

| Interfejs | POSIX: wymagane błędy w zakresie FOD | POSIX: warunkowe / `may fail` | Dowody FOD i otwarte sprawdzenia |
| --- | --- | --- | --- |
| `open()` | `EACCES`, `EEXIST`, `EINTR`, `EISDIR`, `ELOOP`, `EMFILE`, `ENAMETOOLONG`, `ENFILE`, `ENOENT`, `ENOSPC`, `ENOTDIR`, `ENXIO` (nieblokujący zapis do FIFO bez czytelnika), `EOVERFLOW`, `EROFS` | `EAGAIN` dla zablokowanego PTY; `EINVAL` dla nieobsługiwanej opcji; `ELOOP` przy przekroczeniu `SYMLOOP_MAX`; `ENAMETOOLONG` dla `PATH_MAX`; `EOPNOTSUPP` dla socketu; `ETXTBSY` dla wykonywanego pliku | `O_EXCL`, tryby deskryptora i błędy ścieżek mają regresje w [`test_mount_suite.py`](../tests/integration/test_mount_suite.py), [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_symlink.sh`](../tests/integration/test_symlink.sh), [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py), [`test_df.sh`](../tests/integration/test_df.sh) i [`test_mknod.py`](../tests/integration/test_mknod.py). Limit deskryptorów, `EINTR`, `EOVERFLOW` i błędne flagi nie mają jeszcze dedykowanych testów FOD. |
| `read()` / `pread()` | `EBADF`; `EINTR` gdy sygnał przerwie odczyt przed przekazaniem danych; `EISDIR` dla katalogu, którego implementacja nie pozwala czytać jako pliku; `EOVERFLOW` dla pozycji poza limitem offsetu; `EINVAL` dla ujemnego offsetu `pread()`; `EAGAIN` dla nieblokującego pustego FIFO, gdy zapisujący pozostaje otwarty | `EIO` w warunkach terminala albo błędu fizycznego; `ENXIO` dla nieistniejącego urządzenia; błędy STREAMS/socket dotyczą zakresu spoza FOD | [`test_lseek.py`](../tests/integration/test_lseek.py) sprawdza EOF, offsety i niezgodny tryb deskryptora; [`test_mknod.py`](../tests/integration/test_mknod.py) sprawdza FIFO. Sygnałowe przerwanie, `EOVERFLOW`, ujemny offset `pread()` i błąd odczytu `EIO` wymagają osobnych testów albo udokumentowania obsługi przez Linux VFS. |
| `write()` / `pwrite()` | `EBADF`, `EFBIG` (limit rozmiaru pliku lub offsetu), `EINTR` gdy nie zapisano danych, `ENOSPC`; `EINVAL` dla ujemnego offsetu `pwrite()`; `EPIPE` dla FIFO bez czytelników; `EAGAIN` dla nieblokującego FIFO, gdy zapis opóźniłby wątek | `EIO` w warunkach terminala albo błędu fizycznego; błędy STREAMS/socket/urządzeń są poza zakresem FOD | [`test_lseek.py`](../tests/integration/test_lseek.py) sprawdza tryby deskryptora, zapis pozycyjny i rozszerzanie pliku; [`test_two_mount_quota.py`](../tests/integration/test_two_mount_quota.py) i [`test_df.sh`](../tests/integration/test_df.sh) sprawdzają `ENOSPC`; [`test_mknod.py`](../tests/integration/test_mknod.py) sprawdza zapis do FIFO. Sygnałowe przerwanie, limity `off_t`, `EPIPE` i nieblokujące `EAGAIN` wymagają dedykowanych regresji albo udokumentowania obsługi przez Linux VFS. |

## Źródła i granice

- POSIX [`open()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/open.html)
  i [POSIX Programmer's Manual `open(3p)`](https://man7.org/linux/man-pages/man3/open.3p.html).
- POSIX [`read()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/read.html)
  i [POSIX Programmer's Manual `read(3p)`](https://man7.org/linux/man-pages/man3/read.3p.html).
- POSIX [`write()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/write.html)
  i [POSIX Programmer's Manual `write(3p)`](https://man7.org/linux/man-pages/man3/write.3p.html).
- [Linux FUSE API](https://docs.kernel.org/filesystems/fuse/fuse.html) oraz
  callbacki w [`rust_fuse/src/fs.rs`](../rust_fuse/src/fs.rs).

Tabela celowo nie traktuje każdego errno wymienionego przez POSIX jako
wymagania wobec FOD: część dotyczy socketów, STREAMS, urządzeń terminalowych
lub opcji nieobjętych interfejsem FOD. Kolejny etap P12 ma rozszerzyć mapę na
namespace, metadane, blokady i pojemność oraz przypisać błędy do Linux VFS,
libfuse lub callbacku FOD.
