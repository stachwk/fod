# FOD 3.2.88 - test widocznosci dirty write miedzy wieloma fh

## Cel

FOD 3.2.87 przestal wykonywac persistence tylko dlatego, ze pojedynczy
callback FUSE jest czesciowy lub niewyrownany. To usuwa regresje 512 KiB,
ale wymaga jawnego testu semantyki wielu uchwytow.

Dotychczasowy test wielu otwarc wykonywal `flush()` przed dalszym uzyciem,
wiec nie sprawdzal dirty state pozostajacego tylko w `WriteState`.

## Testy

Dodano dwa przypadki do `tests/integration/test_mount_suite.py`.

Pierwszy test zapisuje dane pierwszym fh bez `fsync` i `flush`, otwiera drugi
fh, a nastepnie przed odczytem sprawdza `fstat`, `stat` i `SEEK_END`.
Drugi fh musi widziec aktualny logiczny rozmiar i dane.

Drugi test zapisuje `AA` pierwszym fh, otwiera drugi fh bez publikowania
pierwszego stanu i zapisuje `BB` od offsetu 2. Wynik musi byc `AABB`.

## Wynik test-first

Pierwsze uruchomienie bez zmiany runtime wykazalo realna luke:

- `test_multi_handle_dirty_write_visibility`: drugi `fstat` zwrocil rozmiar
  `0` zamiast `42`,
- `test_multi_handle_dirty_partial_write_merge`: drugi `fstat` zwrocil
  rozmiar `0` zamiast `2`.

Oznacza to, ze dane byly obecne tylko w `WriteState` pierwszego fh, natomiast
nowy fh nadal obserwowal metadane zapisane w PostgreSQL.

## Poprawka runtime

`open()` po ustaleniu `file_id`, sprawdzeniu ACL i trybu read-only sprawdza,
czy plik ma juz aktywny uchwyt. Tylko dla drugiego lub kolejnego fh wywoluje:

`flush_pending_write_states_for_file_except(file_id, u64::MAX)`.

W module FUSE wartosc `u64::MAX` jest juz stosowana jako brak wyjatku, czyli
publikacja wszystkich pending states danego pliku.

Dzieki temu:

- drugi `open` staje sie jawna granica widocznosci miedzy fh,
- `fstat`, `stat` i `SEEK_END` nowego fh widza aktualny rozmiar,
- pierwszy fh nadal buforuje zapis do threshold/fsync/flush/release,
- techniczne fragmenty 512 KiB na pojedynczym fh nadal nie wymuszaja
  persistence per callback,
- zapis drugiego fh startuje z opublikowanego stanu pierwszego fh.

## Kryterium regresji

Oba testy multi-handle musza przejsc, a diagnostyczny benchmark
primary-write -> replica-read dla 512 KiB musi nadal raportowac
`persist_operation_count=1`.
