# Kody błędów FOD

Ten dokument jest krótką mapą znaczeń `errno` używanych przez FOD i powiązanych
testów. Opis oznacza ogólne znaczenie kodu; dokładny warunek jego zwrócenia
zależy od wywołania systemowego. Błąd może powstać w jądrze Linux podczas
rozwiązywania ścieżki, w FUSE albo w backendzie PostgreSQL. Dlatego sama
obecność kodu w tabeli nie oznacza, że każdy przypadek jest obsługiwany przez
callback FOD.

## Kody widoczne w operacjach plikowych

| Kod | Ogólne znaczenie | Przykładowy kontekst FOD | Dowód / zakres |
| --- | --- | --- | --- |
| `EACCES` | Brak prawa dostępu lub przejścia przez katalog | Odczyt/zapis albo wyszukiwanie ścieżki bez wymaganych praw | [`test_path_search_permissions.py`](../tests/integration/test_path_search_permissions.py), [`test_acl_mount_option.py`](../tests/integration/test_acl_mount_option.py), [`test_df.sh`](../tests/integration/test_df.sh) — wybrane przypadki |
| `EBADF` | Nieprawidłowy deskryptor lub niedozwolony tryb deskryptora | `read`/`write`, blokady, `ftruncate`, `fstatvfs` | [`test_lseek.py`](../tests/integration/test_lseek.py), [`test_truncate_shrink_block_boundary.py`](../tests/integration/test_truncate_shrink_block_boundary.py), [`lock_backend_smoke.rs`](../rust_fuse/tests/lock_backend_smoke.rs), [`test_df.sh`](../tests/integration/test_df.sh) |
| `EBUSY` | Zasób jest zajęty lub konfliktuje z aktywną operacją | FOD odrzuca konkurencyjne przejęcie wyłącznej własności zapisu albo konflikt rename | [`test_create_write_ownership.py`](../tests/integration/test_create_write_ownership.py), [`test_rename_write_ownership.py`](../tests/integration/test_rename_write_ownership.py). To zachowanie własności FOD; nie jest ogólną regułą POSIX dla każdej konkurencyjnej operacji. |
| `EEXIST` | Wpis o takiej nazwie już istnieje | `O_CREAT|O_EXCL`, `mkdir`, cel symlinka | [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_symlink.sh`](../tests/integration/test_symlink.sh) |
| `EINVAL` | Argument, zakres lub kombinacja opcji jest nieprawidłowa | Błędny rozmiar truncation lub nieobsługiwany wariant ioctl | [`test_truncate_shrink_block_boundary.py`](../tests/integration/test_truncate_shrink_block_boundary.py), [`test_ioctl.py`](../tests/integration/test_ioctl.py) |
| `EIO` | Błąd wejścia/wyjścia albo błąd backendu przedstawiony jako I/O | Błąd PostgreSQL/zapisu lub wstrzyknięta awaria transakcji | [`test_rename_write_ownership.py`](../tests/integration/test_rename_write_ownership.py), [`test_tx03h1_invalid_session.py`](../tests/integration/test_tx03h1_invalid_session.py). Pokrycie dotyczy wybranych scenariuszy awarii. |
| `EISDIR` | Operacja plikowa została skierowana na katalog | `unlink` lub `truncate` katalogu | [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_truncate_shrink_block_boundary.py`](../tests/integration/test_truncate_shrink_block_boundary.py) |
| `ELOOP` | Rozwiązywanie ścieżki przekroczyło limit dowiązań symbolicznych | Cykl symlinków | [`test_symlink.sh`](../tests/integration/test_symlink.sh), [`test_df.sh`](../tests/integration/test_df.sh) |
| `ENAMETOOLONG` | Nazwa komponentu lub ścieżka przekracza limit | Zbyt długi komponent ścieżki | [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py), [`test_df.sh`](../tests/integration/test_df.sh) |
| `ENODATA` | Żądany atrybut rozszerzony nie istnieje | `getxattr` dla brakującego xattr | [`test_xattr.py`](../tests/integration/test_xattr.py). Na Linuxie `ENOATTR`, gdy występuje jako nazwa przenośna, jest aliasem `ENODATA`. |
| `ENOENT` | Ścieżka, wpis lub obiekt nie istnieje | Brak pliku/katalogu, dangling symlink, nieistniejący cel albo nieaktualny uchwyt | [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_symlink.sh`](../tests/integration/test_symlink.sh), [`test_rename_directory_transactional.py`](../tests/integration/test_rename_directory_transactional.py) |
| `ENOSPC` | Brak dostępnej pojemności | Limit pojemności/quota FOD odrzuca zapis | [`test_two_mount_quota.py`](../tests/integration/test_two_mount_quota.py), [`test_df.sh`](../tests/integration/test_df.sh) |
| `ENOSYS` | Interfejs nie ma implementacji | Callback lub opcjonalna capability nie jest dostępna | [`test_post_731_capability_fallbacks.py`](../tests/integration/test_post_731_capability_fallbacks.py). Kernel może też zakończyć operację przed wysłaniem żądania FUSE. |
| `ENOTDIR` | Element ścieżki, który musi być katalogiem, jest innym typem | Przejście przez zwykły plik jako katalog | [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_path_trailing_slash.py`](../tests/integration/test_path_trailing_slash.py), [`test_df.sh`](../tests/integration/test_df.sh) |
| `ENOTEMPTY` | Katalog nie jest pusty | `rmdir` katalogu z wpisami | [`test_directories.sh`](../tests/integration/test_directories.sh), [`test_unlink_primary_authority_docker.py`](../tests/integration/test_unlink_primary_authority_docker.py) |
| `ENOTSUP` / `EOPNOTSUPP` | Żądana funkcja nie jest obsługiwana | Nieobsługiwany ioctl, fallocate lub rozszerzenie FUSE | [`test_ioctl.py`](../tests/integration/test_ioctl.py), [`test_fallocate_contract.py`](../tests/integration/test_fallocate_contract.py). Na Linuxie te nazwy mają tę samą wartość errno. |
| `ENOTTY` | Deskryptor/obiekt nie obsługuje danego ioctl | Nieznane polecenie ioctl | [`test_ioctl.py`](../tests/integration/test_ioctl.py) |
| `ENXIO` | Brak urządzenia lub dostępnego końca komunikacji | FIFO otwierane do zapisu bez czytelnika; `SEEK_DATA`/`SEEK_HOLE` poza dostępnym zakresem danych | [`test_mknod.py`](../tests/integration/test_mknod.py), [`test_lseek_sparse_runtime.py`](../tests/integration/test_lseek_sparse_runtime.py) |
| `EPERM` | Operacja jest zabroniona dla bieżącego użytkownika | Niedozwolona zmiana właściciela/atrybutu albo operacja na katalogu | [`test_acl_mount_option.py`](../tests/integration/test_acl_mount_option.py), [`test_unlink_primary_authority_docker.py`](../tests/integration/test_unlink_primary_authority_docker.py) |
| `EROFS` | System plików jest tylko do odczytu | Próba mutacji na montowaniu read-only | [`test_mount_suite.py`](../tests/integration/test_mount_suite.py) |
| `EWOULDBLOCK` / `EAGAIN` | Operacja nie może teraz postępować bez oczekiwania | Nieblokująca kolidująca blokada zakresu | [`lock_backend_smoke.rs`](../rust_fuse/tests/lock_backend_smoke.rs). Na Linuxie `EWOULDBLOCK` i `EAGAIN` są aliasami. |
| `EXDEV` | Operacja wymagałaby przekroczenia granicy systemu plików | Próba przeniesienia wpisu do katalogu głównego montowania FOD lub z niego | [`test_rename_root_conflict.sh`](../tests/integration/test_rename_root_conflict.sh) |

## Kody pozostające do sprawdzenia

| Kod | Znaczenie w tym audycie | Status |
| --- | --- | --- |
| `EINTR` | Wywołanie przerwane sygnałem, m.in. oczekujące `F_SETLKW` albo `statvfs()` | Brak ukierunkowanej regresji; śledzone w P10/P11 audytu POSIX. |
| `EIO` | Ogólny błąd I/O jest testowany w awariach backendu, ale zwrot `EIO` konkretnie przez `statvfs()` nie jest sprawdzony | Pozostaje do sprawdzenia w P11. |
| `EOVERFLOW` | Co najmniej jedno pole wyniku `statvfs()` nie mieści się w typie wyniku | Brak testu; pozostaje do sprawdzenia w P11, szczególnie na architekturach 32-bitowych. |

## Błędy transportu i sesji

Te kody występują w testach utraty połączenia i obsługi niepewnego wyniku
transakcji. Zwykle opisują przyczynę po stronie libpq/socketu; FOD może
przetłumaczyć je na `EIO` dla operacji plikowej zamiast zwracać je aplikacji.

| Kod | Ogólne znaczenie | Kontekst / dowód |
| --- | --- | --- |
| `ECONNABORTED` | Połączenie zostało przerwane przed zakończeniem | Klasyfikacja błędów granicy commit: [`test_tx03i2_commit_boundary.py`](../tests/integration/test_tx03i2_commit_boundary.py) |
| `ECONNRESET` | Zdalna strona zresetowała połączenie | Klasyfikacja błędów granicy commit: [`test_tx03i2_commit_boundary.py`](../tests/integration/test_tx03i2_commit_boundary.py) |
| `ENOTCONN` | Połączenie/socket nie jest już aktywne | Testy awarii klienta PostgreSQL i utraty mount-session; np. [`test_tx03i2_commit_boundary.py`](../tests/integration/test_tx03i2_commit_boundary.py) |

## Zakres

To zestawienie nie jest deklaracją pełnej zgodności POSIX i nie wylicza każdego
`errno`, które Linux może zwrócić. Kody są mapą ważnych FOD boundary i testów,
a nie kompletnym katalogiem wszystkich możliwych błędów systemowych. Statusy
oraz brakujące przypadki są śledzone w
[audycie POSIX](POSIX_CONFORMANCE_AUDIT.md), zwłaszcza w P10-P12.
