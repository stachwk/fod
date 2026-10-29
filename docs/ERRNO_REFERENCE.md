# Kody błędów Linuksa i POSIX w FOD

To jest kanoniczne zestawienie nazw `errno` używanych w testach FOD, z
interpretacją Linuksa i zakresem POSIX.1-2024. Kod errno opisuje niepowodzenie
wywołania; sam fakt, że jego nazwa występuje w POSIX, nie oznacza, że POSIX
wymaga zwrócenia go przez FOD w każdym wymienionym tu przypadku. Konkretne
wymagania zależą od interfejsu, warunków wejściowych i kolejności sprawdzeń.
Mapowanie błędów domenowych FOD na linuksowe kody errno jest zebrane w
[`rust_fuse/src/errno.rs`](../rust_fuse/src/errno.rs); kody dla błędów
konkretnych operacji są zwracane bezpośrednio przy ich obsłudze.

## Kody operacji plikowych

| Kod Linuksa | Status nazwy w POSIX | Typowe znaczenie i kontekst | Kontekst FOD i dowód |
| --- | --- | --- | --- |
| `EACCES` | POSIX | Brak praw do operacji albo przeszukiwania komponentu ścieżki | Kontrola praw i ACL: [`test_path_search_permissions.py`](../tests/integration/test_path_search_permissions.py), [`test_acl_mount_option.py`](../tests/integration/test_acl_mount_option.py); path search w [`test_df.sh`](../tests/integration/test_df.sh) |
| `EBADF` | POSIX | Deskryptor jest nieprawidłowy albo otwarty w trybie niezgodnym z operacją | `read`/`write`, blokady, `ftruncate`, `fstatvfs`: [`test_lseek.py`](../tests/integration/test_lseek.py), [`test_truncate_shrink_block_boundary.py`](../tests/integration/test_truncate_shrink_block_boundary.py), [`lock_backend_smoke.rs`](../rust_fuse/tests/lock_backend_smoke.rs), [`test_df.sh`](../tests/integration/test_df.sh) |
| `EBUSY` | POSIX | Zasób jest zajęty | FOD używa go przy konflikcie wyłącznej własności zapisu i rename. To specyficzna polityka FOD, a nie ogólny wymóg POSIX dla konkurencyjnych zapisów: [`test_create_write_ownership.py`](../tests/integration/test_create_write_ownership.py), [`test_rename_write_ownership.py`](../tests/integration/test_rename_write_ownership.py). |
| `EEXIST` | POSIX | Wpis już istnieje | `O_CREAT|O_EXCL`, `mkdir` i utworzenie symlinka: [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_symlink.sh`](../tests/integration/test_symlink.sh) |
| `EINVAL` | POSIX | Argument lub kombinacja opcji jest nieprawidłowa | Zakres `truncate` i ioctl: [`test_truncate_shrink_block_boundary.py`](../tests/integration/test_truncate_shrink_block_boundary.py), [`test_ioctl.py`](../tests/integration/test_ioctl.py) |
| `EIO` | POSIX | Błąd wejścia/wyjścia | Wybrane błędy PostgreSQL i wstrzyknięta awaria transakcji: [`test_rename_write_ownership.py`](../tests/integration/test_rename_write_ownership.py), [`test_tx03h1_invalid_session.py`](../tests/integration/test_tx03h1_invalid_session.py). `EIO` konkretnie z `statvfs()` pozostaje nieprzetestowane. |
| `EISDIR` | POSIX | Operacja plikowa została skierowana na katalog | `unlink` lub `truncate` katalogu: [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_truncate_shrink_block_boundary.py`](../tests/integration/test_truncate_shrink_block_boundary.py) |
| `ELOOP` | POSIX | Rozwiązywanie ścieżki napotkało zbyt wiele symlinków | Cykl symlinków: [`test_symlink.sh`](../tests/integration/test_symlink.sh), [`test_df.sh`](../tests/integration/test_df.sh) |
| `EINTR` | POSIX | Wywołanie przerwane sygnałem | Wymagania zależą od interfejsu; przerwanie `F_SETLKW` i `statvfs()` pozostaje do sprawdzenia w P10/P11 audytu POSIX. |
| `ENAMETOOLONG` | POSIX | Komponent lub ścieżka przekracza limit | Zbyt długi komponent: [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py), [`test_df.sh`](../tests/integration/test_df.sh) |
| `ENODATA` | POSIX symbol; znaczenie w POSIX dotyczy STREAMS | Na Linuksie brak żądanego atrybutu rozszerzonego | Xattr są rozszerzeniem Linux/FUSE, nie bazowym interfejsem POSIX: [`test_xattr.py`](../tests/integration/test_xattr.py). |
| `ENOENT` | POSIX | Ścieżka, wpis lub wymagany obiekt nie istnieje | Brak pliku/katalogu, dangling symlink lub brakujący cel: [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_symlink.sh`](../tests/integration/test_symlink.sh), [`test_rename_directory_transactional.py`](../tests/integration/test_rename_directory_transactional.py) |
| `ENOSPC` | POSIX | Brak dostępnej pojemności | Limit pojemności/quota FOD odrzuca zapis: [`test_two_mount_quota.py`](../tests/integration/test_two_mount_quota.py), [`test_df.sh`](../tests/integration/test_df.sh) |
| `ENOSYS` | POSIX | Funkcja nie jest zaimplementowana | Callback/capability nie jest dostępna; kernel może też zakończyć operację przed żądaniem FUSE: [`test_post_731_capability_fallbacks.py`](../tests/integration/test_post_731_capability_fallbacks.py) |
| `ENOTDIR` | POSIX | Komponent ścieżki, który ma być katalogiem, jest innego typu | Przejście przez zwykły plik: [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py), [`test_df.sh`](../tests/integration/test_df.sh) |
| `ENOTEMPTY` | POSIX | Katalog nie jest pusty | `rmdir` katalogu z wpisami: [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_unlink_primary_authority_docker.py`](../tests/integration/test_unlink_primary_authority_docker.py) |
| `ENOTSUP` | POSIX symbol; konkretne warunki zależą od interfejsu/opcji | Żądana funkcja nie jest obsługiwana | FOD ioctl/fallocate/FUSE: [`test_ioctl.py`](../tests/integration/test_ioctl.py), [`test_fallocate_contract.py`](../tests/integration/test_fallocate_contract.py). Na Linuksie ma tę samą wartość co `EOPNOTSUPP`. |
| `ENOTTY` | POSIX | Nieodpowiednia operacja ioctl dla obiektu/deskryptora | Nieznane polecenie ioctl: [`test_ioctl.py`](../tests/integration/test_ioctl.py) |
| `ENXIO` | POSIX | Brak urządzenia/adresu albo żądanie poza możliwościami obiektu | FIFO otwierane do zapisu bez czytelnika oraz sparse `lseek`: [`test_mknod.py`](../tests/integration/test_mknod.py), [`test_lseek_sparse_runtime.py`](../tests/integration/test_lseek_sparse_runtime.py) |
| `EOVERFLOW` | POSIX | Wynik nie mieści się w typie danych | Obowiązuje m.in. dla określonych wyników `statvfs()`; brak testu, P11 audytu POSIX. |
| `EPERM` | POSIX | Operacja jest zabroniona z powodu uprawnień lub ograniczeń obiektu | ACL/właściciel i operacje na katalogach: [`test_acl_mount_option.py`](../tests/integration/test_acl_mount_option.py), [`test_unlink_primary_authority_docker.py`](../tests/integration/test_unlink_primary_authority_docker.py) |
| `EROFS` | POSIX | System plików jest tylko do odczytu | Próby mutacji w montowaniu read-only: [`test_mount_suite.py`](../tests/integration/test_mount_suite.py) |
| `EAGAIN` / `EWOULDBLOCK` | POSIX; wartości mogą być aliasami | Operacja nie może teraz postępować bez oczekiwania | Nieblokująca kolidująca blokada zakresu: [`lock_backend_smoke.rs`](../rust_fuse/tests/lock_backend_smoke.rs). Na Linuksie są aliasami. |
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
[POSIX_CONFORMANCE_AUDIT.md](POSIX_CONFORMANCE_AUDIT.md), szczególnie P10-P12.

Źródła: [POSIX `<errno.h>`](https://pubs.opengroup.org/onlinepubs/9799919799/basedefs/errno.h.html),
[lista symboli POSIX errno](https://linuxman7.org/linux/man-pages/man0/errno.h.0p.html),
[znaczenia errno w Linuksie](https://man7.org/linux/man-pages/man3/errno.3.html).
