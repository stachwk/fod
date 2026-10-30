# Linux i FUSE: wymagania oraz stan walidacji FOD

Stan dokumentu: 2026-10-30, FOD 3.4.380, commit `68d23b7`.

Ten dokument opisuje aktualny kontrakt uruchomieniowy Linux/FUSE, jego
walidację oraz ograniczenia. Nie jest deklaracją zgodności POSIX. Status
zgodności widocznej przez mount opisuje [audyt POSIX](POSIX_CONFORMANCE_AUDIT.md),
a zestawienie granic POSIX, Linux VFS i FUSE znajduje się w
[kontraktach kompatybilności](compatibility-contracts.md).

## Wymagania startowe

Kod `rust_fuse/src/platform_requirements.rs` wymusza przed połączeniem z bazą
i próbą montowania wersje:

- Linux 4.18.0 lub nowszy;
- libfuse3 3.0.0 lub nowszy;

Do samego mounta host musi ponadto udostępniać `/dev/fuse` i zgodne narzędzie
montujące, zwykle `fusermount3`; brak któregoś z tych elementów zakończy
uruchomienie na etapie tworzenia sesji/mounta.

Nieparsowalna albo starsza wersja jądra oraz za stara libfuse kończą start
błędem. To jest deklarowany próg runtime i kontrola w kodzie. Pełny mount na
minimalnej parze Linux 4.18/libfuse3 3.0 nie jest obecnie potwierdzony testem
end-to-end; zweryfikowany zestaw testowy niżej jest nowszy. Nie należy
interpretować progu preflight jako macierzy przetestowanych dystrybucji.

Pakiet musi też dostarczać SONAME/ABI libfuse zgodny z binarium. Numer wersji
sprawdzany przez FOD nie zastępuje kontroli zależności ELF.

## Warstwa protokołu FUSE

FOD przypina `fuser 0.18.0` z backendem `libfuse3`. Maksymalny protokół
obsługiwany przez warstwę userspace wynosi 7.40. Podczas inicjalizacji kernel i
userspace negocjują wspólną wersję oraz capability; runtime zapisuje do logu
wersję kernela, wynegocjowaną wersję, dostępne, żądane, włączone i niedostępne
capability.

FOD prosi m.in. o `POSIX_LOCKS`, `FLOCK_LOCKS` i `ATOMIC_O_TRUNC`. Zapisujący
mount wymaga wynegocjowanego `ATOMIC_O_TRUNC`; bez niego start jest odrzucany,
bo bezpieczeństwo własności zapisu opiera się na atomowym zachowaniu
`O_TRUNC`. Obsługa ACL żąda `POSIX_ACL` tylko wtedy, gdy ACL są włączone.
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

FUSE jest granicą kernel-userspace, a nie osobnym standardem semantyki
filesystemu. Część syscalli kończy się w Linux VFS przed wysłaniem requestu do
FOD. Sukces syscalla potwierdza zachowanie całego stosu, ale nie dowodzi, że
callback FOD został wywołany. Dokładne przykłady i przypisanie errno są w
[macierzy errno](POSIX_ERRNO_MATRIX.md).

| Stan w FOD | Obszary |
| --- | --- |
| Implementowane i objęte testami | init, lookup/forget/getattr, readdir/readlink/statfs, xattr, access, poll, open/read/write/release, POSIX/flock locks, flush/fsync, setattr, namespace, copy_file_range, regular `mknod`/FIFO, symlink/link; tworzenie węzłów urządzeń nie jest wspierane |
| Implementowane, lecz dowody nie pokrywają całego kontraktu | ioctl, bmap, wybrane tryby fallocate, callback lseek dla SEEK_DATA/SEEK_HOLE |
| Brak jawnej implementacji FOD | destroy, batch_forget, readdirplus, fsyncdir; katalogi używają domyślnego zachowania warstwy fuser zamiast własnych callbacków opendir/releasedir |
| Brak callbacku w używanym wysokopoziomowym API fuser | natywne SYNCFS, TMPFILE i STATX |

Zwykłe `SEEK_SET`, `SEEK_CUR` i `SEEK_END` obsługuje kernel na podstawie
atrybutów inode; FOD implementuje callback dla sparse `SEEK_DATA` i
`SEEK_HOLE`. Brak callbacku lub capability trzeba oceniać per operacja, a nie
na podstawie samej wersji protokołu. Szczegółowy spis oraz follow-upy są w
[kontraktach kompatybilności](compatibility-contracts.md).

## Co zostało zweryfikowane

Ostatnie zapisane testy montowanego runtime, przywołane w audycie i
`TODO.md`, używały Linux 6.17 i libfuse 3.17.4. W tej konfiguracji testy
obejmowały m.in. suite mount, operacje POSIX, locki, FIFO, sparse seek,
równoległe zadania, negocjację capability i rejection nieobsługiwanych opcji.
Test jednostkowy preflight sprawdza granicę wersji 4.18.0 oraz odrzucenie
starszego/nieparsowalnego jądra i libfuse poniżej 3.0.0.

Nie ma obecnie udokumentowanej macierzy end-to-end dla minimalnych wersji
Linux/libfuse ani wielu dystrybucji. Zamknięcie tej luki wymaga uruchomienia
zamontowanego zestawu na najstarszej wspieranej parze albo podniesienia
deklarowanego minimum do najstarszej pary faktycznie zweryfikowanej.

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
- [Linux v6.17: nagłówek protokołu FUSE i negocjacja wersji](https://github.com/torvalds/linux/blob/v6.17/include/uapi/linux/fuse.h)
- Implementacja preflight: `rust_fuse/src/platform_requirements.rs`.
- Negocjacja i diagnostyka: `rust_fuse/src/compatibility.rs`.
- Callbacki i wymóg `ATOMIC_O_TRUNC`: `rust_fuse/src/fs.rs`.
- Przegląd luk i plan weryfikacji: [`plans/CURRENT.md`](plans/CURRENT.md).
