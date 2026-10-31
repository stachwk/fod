# Linux i FUSE: wymagania oraz stan walidacji FOD

Stan dokumentu: 2026-10-31, przegląd kodu do FOD 3.4.429. Najnowsze
przywołane testy funkcjonalne pochodzą z wersji wskazanych przy konkretnych
przypadkach; nie oznacza to pełnego testu stosu dla FOD 3.4.429. Zmiana w
3.4.415 podniosła wymagane minimum jądra do Linux 6.17.0; nie dostarczyła
nowego zamontowanego przebiegu POSIX.

Ten dokument opisuje aktualny kontrakt uruchomieniowy Linux/FUSE, jego
walidację oraz ograniczenia. Nie jest deklaracją zgodności POSIX. Status
zgodności widocznej przez mount opisuje [audyt POSIX](POSIX_CONFORMANCE_AUDIT.md),
a zestawienie granic POSIX, Linux VFS i FUSE znajduje się w
[kontraktach kompatybilności](compatibility-contracts.md).

## Wymagania startowe

Kod `rust_fuse/src/platform_requirements.rs` wymusza przed połączeniem z bazą
i próbą montowania wersje:

- Linux 6.17.0 lub nowszy;
- libfuse3 3.18.2 lub nowszy;

Do samego mounta host musi ponadto udostępniać `/dev/fuse` i zgodne narzędzie
montujące, zwykle `fusermount3`; brak któregoś z tych elementów zakończy
uruchomienie na etapie tworzenia sesji/mounta.

Nieparsowalna albo starsza wersja jądra oraz za stara libfuse kończą start
błędem. FOD sprawdza pełną wersję `fuse_pkgversion()`, ponieważ `fuse_version()`
zwraca tylko major/minor i nie odróżnia poprawek w ramach 3.18. Testy montowane
działają na Linux `6.17.0-41-generic` z libfuse3 3.18.2 i 3.18.3. Na obu
wersjach przeszły testy montowania, odmontowania, zapisu i ponownego odczytu;
na 3.18.2 przeszły także uprawnienia, `truncate()`/`O_TRUNC` i killpriv.
Testowano izolowane buildy upstream, bez podmiany systemowej biblioteki.
Nie należy interpretować tych przebiegów jako macierzy dystrybucji.

Minimum 3.18.2 uwzględnia upstreamowy [advisory
GHSA-qxv7-xrc2-qmfx](https://github.com/libfuse/libfuse/security/advisories/GHSA-qxv7-xrc2-qmfx):
io_uring use-after-free dotyczy 3.18.0–3.18.1 i został naprawiony w 3.18.2.
FOD używa standardowego `/dev/fuse` transportu; mimo to próg nie dopuszcza
wersji biblioteki z tym błędem. Aktualnym sprawdzonym wydaniem jest 3.18.3.

Linux 6.16 i starsze wydania są poza deklarowanym zakresem i preflight odrzuca
je przed połączeniem z bazą danych i uruchomieniem mounta. Minimalna
deklarowana wersja to Linux 6.17.0. Test graniczny potwierdza, że wersja 4.18.0
jest poprawnie parsowana (także z sufiksem dystrybucji), ale odrzucana jako za
stara. Wartość 4.18 w historycznym raporcie strojenia FUSE opisuje wyłącznie
ówczesne wymaganie i nie jest aktualną deklaracją. Parser akceptuje numeryczne pola
major/minor/patch oraz typowe sufiksy dystrybucji po ostatnim polu numerycznym
(`-` lub `+`); odrzuca też sufiks wstawiony przed polem patch. Odrzuca
uszkodzone pola takie jak `6.17malformed.0` zamiast interpretować ich prefiks
numeryczny jako prawidłową wersję.

Pakiet musi też dostarczać SONAME/ABI libfuse zgodny z binarium. Numer wersji
sprawdzany przez FOD nie zastępuje kontroli zależności ELF.

## Warstwa protokołu FUSE

FOD przypina `fuser 0.18.0` z backendem `libfuse3`. Maksymalny protokół
obsługiwany przez warstwę userspace wynosi 7.40. Podczas inicjalizacji kernel i
userspace negocjują wspólną wersję oraz capability; runtime zapisuje do logu
wersję kernela, wynegocjowaną wersję, dostępne, żądane, włączone i niedostępne
capability.

FOD prosi m.in. o `POSIX_LOCKS`, `FLOCK_LOCKS`, `ATOMIC_O_TRUNC`,
`HANDLE_KILLPRIV_V2` i `SETXATTR_EXT`. Mount z ACL dodatkowo wymaga
`POSIX_ACL` i `DONT_MASK`: Linux stosuje umask do trybu tworzonego obiektu,
chyba że userspace jawnie wynegocjuje `FUSE_DONT_MASK`. FOD stosuje wtedy
umask w transakcji tworzenia, po sprawdzeniu domyślnego ACL rodzica. Zapisujący
mount wymaga wynegocjowanych
`ATOMIC_O_TRUNC`, `HANDLE_KILLPRIV_V2` i `SETXATTR_EXT`; bez nich start jest
odrzucany. `HANDLE_KILLPRIV_V2` przekazuje FOD informację, że Linux usuwa
uprawnienia uprzywilejowane wskutek zapisu, truncate, chown lub `O_TRUNC`.
FOD stosuje flagi do set-ID i czyści pozostałe `security.capability` w
autorytatywnej transakcji PostgreSQL zapisu. Linux może wcześniej wysłać
wewnętrzne `REMOVEXATTR security.capability` przed zapisem. Ponieważ ten krok
VFS może przyjść z UID zapisującego bez flagi identyfikującej przyczynę, FOD
dopuszcza wyłącznie usunięcie tego konkretnego atrybutu bez kontroli właściciela;
jawna nieuprzywilejowana próba `removexattr()` jest odrzucana przez Linux przed
FUSE. W takim przebiegu usunięcie xattr jest osobną transakcją, poprzedzającą
transakcję danych; obie są serializowane przez PostgreSQL. `SETXATTR_EXT`
dostarcza flagę czyszczenia SGID przy zmianie access ACL. Obsługa ACL żąda
`POSIX_ACL` i `DONT_MASK` tylko jeśli ACL są włączone.
`MAX_PAGES` wpływa na maksymalny rozmiar requestu, ale nie jest wymagane do
poprawności podstawowego mounta.

Rejestrowana linia diagnostyczna zaczyna się od:

```text
FOD FUSE compatibility:
FOD FUSE negotiated:
```

Udostępnia m.in. `kernel_protocol`, `negotiated_protocol`, capability oraz
limity `requested/effective_max_write`, `requested/effective_max_readahead`,
`kernel_max_pages_limit` i `kernel_max_request_bytes`. Obecne wartości
domyślne to `fuse_max_write_bytes=1MiB` i `fuse_max_readahead_bytes=512KiB`;
rzeczywisty request może ograniczyć kernel.

## Zakres callbacków

FUSE jest protokołem kernel-userspace, a nie standardem semantyki filesystemu.
libfuse jest biblioteką referencyjną userspace; FOD korzysta z Rust API `fuser`
0.18.0 przez backend libfuse3. Każda z tych warstw może ograniczać zachowanie.
Część syscalli kończy się w Linux VFS przed wysłaniem requestu do FOD. Sukces
syscalla potwierdza zachowanie całego stosu, ale nie dowodzi, że callback FOD
został wywołany. Dokładne przykłady i przypisanie errno są w
[macierzy errno](POSIX_ERRNO_MATRIX.md).

| Stan w FOD | Obszary |
| --- | --- |
| Implementowane i objęte testami wybranych zachowań | init, lookup/forget/getattr/setattr, opendir/readdir/releasedir, readlink/statfs (w tym `EINTR` dla przerwanego `statvfs()`), xattr, access, poll, open/create/read/write/release, POSIX/flock locks, flush/fsync, namespace, copy_file_range, regular `mknod`/FIFO, symlink/link; tworzenie węzłów urządzeń nie jest wspierane |
| Implementowane, lecz dowody nie pokrywają całego kontraktu | ioctl, bmap, wybrane tryby fallocate, callback lseek dla SEEK_DATA/SEEK_HOLE; obecność callbacku nie oznacza pełnej zgodności POSIX ani kompletnego pokrycia błędów |
| Brak jawnej implementacji FOD | destroy, batch_forget, readdirplus, fsyncdir |
| Brak callbacku w używanym wysokopoziomowym API fuser | natywne SYNCFS, TMPFILE i STATX |

`opendir()` sprawdza typ katalogu i prawo odczytu oraz tworzy stan uchwytu;
`releasedir()` zwalnia ten stan. Zwykłe `SEEK_SET`, `SEEK_CUR` i `SEEK_END`
obsługuje kernel, a callback FOD `lseek` jest używany dla
`SEEK_DATA`/`SEEK_HOLE`. POSIX.1-2024 określa te dwa ostatnie whence jako
opcjonalne zachowanie; wcześniejsze wydania traktowały je jako rozszerzenie
Linux/BSD. Zgodność samego syscalla nie oznacza wywołania callbacku.
Nieobsługiwane callbacki i capability oceniamy per operacja, nie na podstawie
samego numeru protokołu.

## Otwarte prace w warstwie FUSE

- dodać test błędów `fsync`, gdy będzie dostępny wąski mechanizm wstrzykiwania
  błędów PostgreSQL w teście montowanym;
- zmierzyć retencję cache inode/ścieżek i rozważyć `forget`/`batch_forget` po
  zdefiniowaniu cyklu życia referencji lookup;
- podłączyć wybrane tryby `fallocate` do obsługi resize i sparse oraz odrzucać
  nieobsługiwane kombinacje;
- zmierzyć korzyści `readdirplus` przed jego włączeniem;
- powtarzać inwentaryzację callbacków i capability po aktualizacji `fuser`.

## Co zostało zweryfikowane

Ostatnie zapisane testy montowanego runtime, przywołane w audycie i
`TODO.md`, używały Linux 6.17 i libfuse 3.17.4. Na FOD 3.4.385 test
primary/replica potwierdza też odmowę `open()` po cofnięciu prawa przejścia
katalogu nadrzędnego, gdy kernelowe `default_permissions` widzi nieaktualne
atrybuty repliki. Ten sam test potwierdza, że `access(X_OK)` po zdalnym cofnięciu
bitu wykonywania widzi nowy tryb: mount read-only ma skonfigurowany dodatni TTL
3 s, replay repliki jest wstrzymany, a FOD odpowiada zerowym TTL atrybutów dla
`default_permissions` i pobiera `getattr` z autorytatywnego źródła. W tej
konfiguracji testy
obejmowały m.in. suite mount, operacje POSIX, locki, FIFO, sparse seek,
równoległe zadania, negocjację capability i rejection nieobsługiwanych opcji.
Mounted regresja `statvfs()` blokuje zapytanie na tabeli PostgreSQL, wysyła
przechwycony `SIGUSR1` i potwierdza `EINTR`; Linux dokumentuje, że FUSE
userspace może odpowiedzieć `EINTR` na pierwotne żądanie po `FUSE_INTERRUPT`
([opis zachowania](https://github.com/torvalds/linux/blob/v6.17/Documentation/filesystems/fuse/fuse.rst)).
Test jednostkowy preflight sprawdza granicę wersji 6.17.0 i 3.18.2, odrzucenie
starszego/nieparsowalnego jądra i libfuse, w tym 3.18.1 oraz nieparsowalnych
lub niespójnych metadanych wersji. Dla kernela sprawdza również zachowanie
dystrybucyjnych sufiksów, odrzucenie wadliwych pól numerycznych i sufiksu
wstawionego przed polem patch.
Montowane regresje `test-symlink.sh` i `test_xattr.py` sprawdzają także, że
niepoprawny UTF-8 w celu symlinka i nazwie xattr kończy się `EILSEQ` bez
aliasowania poprawnego wejścia z U+FFFD.

Pełna macierz end-to-end dla wielu wersji Linux/libfuse i dystrybucji pozostaje
poza pokryciem. Host systemowy nadal ma libfuse 3.17.4, ale testowe buildy
upstream 3.18.2 i 3.18.3 zainstalowano izolowanie pod `/var/tmp`; zamontowane
testy uruchomiono z tymi bibliotekami i helperami. Minimum runtime/build
ustawiono na najstarszą wersję objętą poprawką bezpieczeństwa i sprawdzoną
montowanym testem.

## Rozmiar requestu i strojenie

Domyślne limity FOD nie zmieniają globalnego `fs.fuse.max_pages_limit`.
Można sprawdzić hosta poleceniami:

```bash
uname -r
cat /proc/sys/fs/fuse/max_pages_limit
getconf PAGE_SIZE
```

`MAX_PAGES` oraz limit stron kernela mogą ograniczać request poniżej wartości
skonfigurowanej w FOD. Zmiana sysctl dotyczy wszystkich filesystemów FUSE na
hoście i pozostaje decyzją administratora. Historyczne pomiary profili
512KiB/1MiB/2MiB oraz ich wyników znajdują się w
[archiwalnym raporcie FOD 3.3](history/FUSE_REQUEST_SIZING_FOD_3_3_12_TO_3_3_19.md)
i [BENCHMARKS.md](../BENCHMARKS.md); nie są one macierzą zgodności ani
gwarancją wydajności na aktualnym kernelu.

## Źródła

- [Linux kernel: FUSE technical documentation](https://docs.kernel.org/filesystems/fuse/)
- [libfuse: userspace reference implementation and API documentation](https://github.com/libfuse/libfuse)
- [libfuse releases](https://github.com/libfuse/libfuse/releases), including tested 3.18.2 and 3.18.3
- [Linux v6.17: nagłówek protokołu FUSE i negocjacja wersji](https://github.com/torvalds/linux/blob/v6.17/include/uapi/linux/fuse.h)
- Implementacja preflight: `rust_fuse/src/platform_requirements.rs`.
- Negocjacja i diagnostyka: `rust_fuse/src/compatibility.rs`.
- Callbacki i wymóg `ATOMIC_O_TRUNC`: `rust_fuse/src/fs.rs`.
- Przegląd luk i plan weryfikacji: [`plans/CURRENT.md`](plans/CURRENT.md).
