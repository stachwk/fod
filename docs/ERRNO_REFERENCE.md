# Kody błędów Linuksa i POSIX w FOD

To jest kanoniczne zestawienie nazw `errno` używanych w testach FOD oraz
wymienionych w audycie POSIX, z interpretacją Linuksa i zakresem POSIX.1-2024.
Kod errno opisuje niepowodzenie wywołania; sama obecność nazwy w POSIX nie
oznacza, że POSIX wymaga zwrócenia tego błędu przez FOD w każdym przypadku.
Konkretne wymagania zależą od interfejsu, warunków wejściowych i kolejności
sprawdzeń.
Kanoniczne stałe Linux errno oraz mapowanie błędów domenowych FOD są zebrane w
[`rust_fuse/src/errno.rs`](../rust_fuse/src/errno.rs). Callbacki używają tych
stałych przy zwracaniu błędów konkretnych operacji.

## Kody operacji plikowych

| Kod Linuksa | Status nazwy w POSIX | Typowe znaczenie i kontekst | Kontekst FOD i dowód |
| --- | --- | --- | --- |
| `EACCES` | POSIX | Brak praw do operacji albo przeszukiwania komponentu ścieżki | Kontrola praw i ACL: [`test_path_search_permissions.py`](../tests/integration/test_path_search_permissions.py), [`test_acl_mount_option.py`](../tests/integration/test_acl_mount_option.py); path search w [`test_df.sh`](../tests/integration/test_df.sh) |
| `EBADF` | POSIX | Deskryptor jest nieprawidłowy albo otwarty w trybie niezgodnym z operacją | `read`/`write`, blokady, `ftruncate`, `fstatvfs`: [`test_lseek.py`](../tests/integration/test_lseek.py), [`test_truncate_shrink_block_boundary.py`](../tests/integration/test_truncate_shrink_block_boundary.py), [`lock_backend_smoke.rs`](../rust_fuse/tests/lock_backend_smoke.rs), [`test_df.sh`](../tests/integration/test_df.sh) |
| `EBUSY` | POSIX | Zasób jest zajęty | FOD używa go przy konflikcie wyłącznej własności zapisu i rename. To specyficzna polityka FOD, a nie ogólny wymóg POSIX dla konkurencyjnych zapisów: [`test_create_write_ownership.py`](../tests/integration/test_create_write_ownership.py), [`test_rename_write_ownership.py`](../tests/integration/test_rename_write_ownership.py). |
| `EDEADLK` | POSIX | Żądana blokada pliku doprowadziłaby do zakleszczenia | `F_SETLKW` rejestruje konflikty w PostgreSQL i zwraca ten błąd, gdy cykl obejmuje żądanie z innego procesu lub mounta. `lock_backend_smoke.rs::blocking_setlkw_reports_cross_mount_deadlock` sprawdza dwa procesy na dwóch mountach i wzajemnie blokowane zakresy; zakres audytu P10: [`POSIX_ERRNO_MATRIX.md`](POSIX_ERRNO_MATRIX.md). |
| `EEXIST` | POSIX | Wpis już istnieje | `O_CREAT|O_EXCL`, `mkdir` i utworzenie symlinka: [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_symlink.sh`](../tests/integration/test_symlink.sh) |
| `EFBIG` | POSIX | Zapis przekroczył limit rozmiaru pliku lub offsetu | [`test_lseek.py`](../tests/integration/test_lseek.py) sprawdza `RLIMIT_FSIZE`: zapis częściowy, `SIGXFSZ` oraz `EFBIG` dla dalszych `write()`/`pwrite()`; test obejmuje też `truncate()`/`ftruncate()` ponad limit i potwierdza `EFBIG` oraz niezmieniony plik. Szczegóły: [`POSIX_ERRNO_MATRIX.md`](POSIX_ERRNO_MATRIX.md). |
| `EINVAL` | POSIX | Argument lub kombinacja opcji jest nieprawidłowa | Zakres `truncate` i ioctl: [`test_truncate_shrink_block_boundary.py`](../tests/integration/test_truncate_shrink_block_boundary.py), [`test_ioctl.py`](../tests/integration/test_ioctl.py) |
| `EIO` | POSIX | Błąd wejścia/wyjścia | Wybrane błędy PostgreSQL i wstrzyknięta awaria transakcji: [`test_rename_write_ownership.py`](../tests/integration/test_rename_write_ownership.py), [`test_tx03h1_invalid_session.py`](../tests/integration/test_tx03h1_invalid_session.py). `EIO` konkretnie z `statvfs()` pozostaje nieprzetestowane. |
| `EILSEQ` | POSIX | Sekwencja bajtów nie tworzy poprawnej nazwy w bieżącym locale; `open(O_CREAT)` może wymagać go dla nieprzenośnej nazwy, której implementacja nie potrafi utworzyć | FOD zwraca `EILSEQ` dla nie-UTF-8 komponentów, których PostgreSQL `text` nie może przechować: [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py) |
| `EISDIR` | POSIX tylko w określonych interfejsach (np. `open`/`truncate`); Linux zwraca go także dla `unlink` katalogu, gdzie POSIX wskazuje `EPERM`, jeśli implementacja odmawia usunięcia | Operacja, która nie obsługuje katalogu, trafiła na katalog | Linuxowy kontrakt FOD: `unlink` katalogu zwraca `EISDIR` ([`test_directories.sh`](../tests/integration/test_directories.sh)); `truncate` katalogu także zwraca `EISDIR` ([`test_truncate_shrink_block_boundary.py`](../tests/integration/test_truncate_shrink_block_boundary.py)). Różnica względem errno wskazanego przez POSIX Issue 8 jest świadoma i zapisana w [`TODO.md`](../TODO.md). |
| `ELOOP` | POSIX | Rozwiązywanie ścieżki napotkało zbyt wiele symlinków | Cykl symlinków: [`test_symlink.sh`](../tests/integration/test_symlink.sh), [`test_df.sh`](../tests/integration/test_df.sh) |
| `EINTR` | POSIX | Wywołanie przerwane sygnałem | Montowany test `F_SETLKW` sprawdza powrót `EINTR` po `SIGUSR1` i brak późniejszego przyznania blokady; przerwanie `statvfs()` pozostaje do sprawdzenia w P11 [`audytu POSIX`](POSIX_CONFORMANCE_AUDIT.md). |
| `EMFILE` | POSIX | Proces osiągnął limit otwartych deskryptorów | Brak dedykowanej regresji limitu deskryptorów dla `open()`; przypadek ujęty w [`POSIX_ERRNO_MATRIX.md`](POSIX_ERRNO_MATRIX.md). |
| `EMLINK` | POSIX | Osiągnięto limit dowiązań do pliku lub katalogu | Brak dedykowanego testu limitu dowiązań; wymagania `link()`/`mkdir()`/`rename()` są ujęte w [`POSIX_ERRNO_MATRIX.md`](POSIX_ERRNO_MATRIX.md). |
| `ENAMETOOLONG` | POSIX | Komponent lub ścieżka przekracza limit | Zbyt długi komponent: [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py), [`test_df.sh`](../tests/integration/test_df.sh) |
| `ENFILE` | POSIX | System osiągnął limit otwartych plików | Brak dedykowanej regresji systemowego limitu dla `open()`; przypadek ujęty w [`POSIX_ERRNO_MATRIX.md`](POSIX_ERRNO_MATRIX.md). |
| `ENODATA` | POSIX symbol; znaczenie w POSIX dotyczy STREAMS | Na Linuksie brak żądanego atrybutu rozszerzonego | Xattr są rozszerzeniem Linux/FUSE, nie bazowym interfejsem POSIX: [`test_xattr.py`](../tests/integration/test_xattr.py). |
| `ENOENT` | POSIX | Ścieżka, wpis lub wymagany obiekt nie istnieje | Brak pliku/katalogu, dangling symlink lub brakujący cel: [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_symlink.sh`](../tests/integration/test_symlink.sh), [`test_rename_directory_transactional.py`](../tests/integration/test_rename_directory_transactional.py) |
| `ENOSPC` | POSIX | Brak dostępnej pojemności | Limit pojemności/quota FOD odrzuca zapis: [`test_two_mount_quota.py`](../tests/integration/test_two_mount_quota.py), [`test_df.sh`](../tests/integration/test_df.sh) |
| `ENOSYS` | POSIX | Funkcja nie jest zaimplementowana | Callback/capability nie jest dostępna; kernel może też zakończyć operację przed żądaniem FUSE: [`test_post_731_capability_fallbacks.py`](../tests/integration/test_post_731_capability_fallbacks.py) |
| `ENOTDIR` | POSIX | Komponent ścieżki, który ma być katalogiem, jest innego typu | Przejście przez zwykły plik: [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py), [`test_df.sh`](../tests/integration/test_df.sh) |
| `ENOTEMPTY` | POSIX | Katalog nie jest pusty | `rmdir` katalogu z wpisami: [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_unlink_primary_authority_docker.py`](../tests/integration/test_unlink_primary_authority_docker.py) |
| `ENOTSUP` | POSIX symbol; konkretne warunki zależą od interfejsu/opcji | Żądana funkcja nie jest obsługiwana | FOD ioctl/fallocate/FUSE: [`test_ioctl.py`](../tests/integration/test_ioctl.py), [`test_fallocate_contract.py`](../tests/integration/test_fallocate_contract.py). Na Linuksie ma tę samą wartość co `EOPNOTSUPP`. |
| `ENOTTY` | POSIX | Nieodpowiednia operacja ioctl dla obiektu/deskryptora | Nieznane polecenie ioctl: [`test_ioctl.py`](../tests/integration/test_ioctl.py) |
| `ENXIO` | POSIX | Brak urządzenia/adresu albo żądanie poza możliwościami obiektu | FIFO otwierane do zapisu bez czytelnika oraz sparse `lseek`: [`test_mknod.py`](../tests/integration/test_mknod.py), [`test_lseek_sparse_runtime.py`](../tests/integration/test_lseek_sparse_runtime.py) |
| `ENOLCK` | POSIX | Brak dostępnych zasobów do utworzenia blokady | POSIX wymienia ten błąd, gdy system nie może przydzielić zasobów blokad. Brak dedykowanej regresji limitu, P10/P12 audytu: [`POSIX_ERRNO_MATRIX.md`](POSIX_ERRNO_MATRIX.md). |
| `EOVERFLOW` | POSIX | Wynik nie mieści się w typie danych | Obowiązuje m.in. dla określonych wyników `statvfs()`; brak testu, P11 audytu POSIX. |
| `EPERM` | POSIX | Operacja jest zabroniona z powodu uprawnień lub ograniczeń obiektu | ACL/właściciel i operacje na katalogach: [`test_acl_mount_option.py`](../tests/integration/test_acl_mount_option.py), [`test_unlink_primary_authority_docker.py`](../tests/integration/test_unlink_primary_authority_docker.py) |
| `EPIPE` | POSIX | Zapis do FIFO bez czytelników; proces może otrzymać `SIGPIPE` | `test_mknod.py` sprawdza `EPIPE` po zamknięciu ostatniego czytelnika FIFO. |
| `ERANGE` | POSIX | Wynik jest za duży dla przekazanego bufora | Callbacki Linux xattr FOD zwracają `ERANGE`, gdy bufor `getxattr()` lub `listxattr()` jest za mały; montowana regresja libc sprawdza oba przypadki w [`test_xattr.py`](../tests/integration/test_xattr.py), a obsługa jest w [`fs.rs`](../rust_fuse/src/fs.rs). |
| `EROFS` | POSIX | System plików jest tylko do odczytu | Zmiana trybu, właściciela/grupy i znaczników czasu na replice tylko do odczytu ma asercje dokładnego `EROFS`: [`test_mount_suite.py`](../tests/integration/test_mount_suite.py) |
| `ESPIPE` | POSIX | Operacja pozycyjna lub seek dotyczy obiektu nieprzeszukiwalnego | `test_mknod.py` sprawdza `ESPIPE` dla `pread()` i `pwrite()` na FIFO. |
| `ESTALE` | POSIX | Uchwyt do pliku jest nieaktualny | Degraded read-only zwraca `ESTALE`, gdy otwarty plik znika ze snapshotu standby; montowany test trzywęzłowy potwierdza ten przypadek w [`test_pg_three_node_failover.sh`](../tests/integration/test_pg_three_node_failover.sh), a mapowanie jest w [`fs.rs`](../rust_fuse/src/fs.rs). |
| `EAGAIN` / `EWOULDBLOCK` | POSIX; wartości mogą być aliasami | Operacja nie może teraz postępować bez oczekiwania | Nieblokująca kolidująca blokada zakresu pliku regularnego oraz odczyt pustego FIFO z otwartym pisarzem: [`lock_backend_smoke.rs`](../rust_fuse/tests/lock_backend_smoke.rs), [`test_mknod.py`](../tests/integration/test_mknod.py). Blokady rekordowe FIFO są opcjonalne w POSIX, a Linux FUSE obsługuje je lokalnie bez gwarancji między mountami; szczegóły w [`TODO.md`](../TODO.md). Na Linuksie `EAGAIN` i `EWOULDBLOCK` są aliasami. |
| `ETXTBSY` | POSIX (`may fail` dla niektórych operacji) | Próba modyfikacji pliku używanego jako wykonywalny tekst | FOD nie ma dedykowanego testu; POSIX dopuszcza m.in. ten błąd dla `unlink()` w warunkach opisanych w [`POSIX_ERRNO_MATRIX.md`](POSIX_ERRNO_MATRIX.md). |
| `EXDEV` | POSIX | Operacja wymaga przekroczenia granicy systemu plików | FOD sprawdza rename do/z katalogu głównego montowania: [`test_rename_root_conflict.sh`](../tests/integration/test_rename_root_conflict.sh) |

## Kody POSIX socketów używane w diagnostyce backendu

Nazwy poniżej są standaryzowane przez POSIX dla operacji socketowych. W tych
testach opisują błąd transportu PostgreSQL/libpq, a nie kontrakt errno
operacji plikowej FOD. Warstwa FOD może zamienić błąd transportu na `EIO` dla
wywołującego.

| Kod | Znaczenie | Kontekst / dowód |
| --- | --- | --- |
| `ECONNABORTED` | Połączenie zostało przerwane | Klasyfikacja granicy commit: [`test_tx03i2_commit_boundary.py`](../tests/integration/test_tx03i2_commit_boundary.py) |
| `ECONNRESET` | Połączenie zostało zresetowane przez drugą stronę | Klasyfikacja granicy commit: [`test_tx03i2_commit_boundary.py`](../tests/integration/test_tx03i2_commit_boundary.py) |
| `ENOTCONN` | Socket nie jest połączony | Testy awarii PostgreSQL i utraty mount-session: [`test_tx03i2_commit_boundary.py`](../tests/integration/test_tx03i2_commit_boundary.py) |

## Nazwy i aliasy spoza bazowego katalogu Linuksa

- `ENOATTR` nie jest przenośną nazwą POSIX ani linuksowym kodem używanym przez
  FOD. Niektóre systemy używają jej jako odpowiednika `ENODATA`; testy
  przenośne mogą sprawdzać obie nazwy.
- `EOPNOTSUPP` jest nazwą POSIX dla operacji nieobsługiwanej przez socket.
  Linux utożsamia ją liczbowo z `ENOTSUP` i używa szerzej, m.in. dla
  nieobsługiwanych operacji systemu plików. To linuksowa praktyka, a nie
  rozszerzenie kontraktu POSIX dla FOD.
- Linux utożsamia `EAGAIN` i `EWOULDBLOCK`; POSIX dopuszcza takie aliasowanie.

## Zakres i dalszy audyt

Tabela opisuje nazwy i przykłady, nie jest obietnicą pełnej zgodności POSIX.
Pełne wymagane błędy trzeba sprawdzić osobno dla każdego interfejsu, w tym ich
warunki, priorytet oraz przypadki obsługiwane wcześniej przez VFS. Zbiorczy
audyt i brakujące przypadki są śledzone w
[POSIX_CONFORMANCE_AUDIT.md](POSIX_CONFORMANCE_AUDIT.md), szczególnie P10-P12;
robocza mapa wymaganych i warunkowych błędów dla operacji plikowych i namespace
znajduje się w [POSIX_ERRNO_MATRIX.md](POSIX_ERRNO_MATRIX.md).

Źródła: [POSIX `<errno.h>`](https://pubs.opengroup.org/onlinepubs/9799919799/basedefs/errno.h.html),
[lista symboli POSIX errno](https://linuxman7.org/linux/man-pages/man0/errno.h.0p.html),
[znaczenia errno w Linuksie](https://man7.org/linux/man-pages/man3/errno.3.html).
