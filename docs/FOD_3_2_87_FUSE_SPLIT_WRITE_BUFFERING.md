# FOD 3.2.87 - buforowanie podzielonych zapisow FUSE

## Problem

Pomiar primary-write -> replica-read wykazal, ze wariant fio 512 KiB byl
nietypowo wolny na zapisie mimo poprawnego negocjowania duzego requestu FUSE.

Dla pliku 7.5 MiB i `FOD_FOPEN_DIRECT_IO=1` jeden logiczny zapis fio 512 KiB
byl dostarczany do FOD jako dwa callbacki:

- 523728 B,
- 560 B.

Suma wynosi 524288 B, ale oba callbacki sa niewyrownane do 4 KiB.
Dotychczas `partial_block_visibility_write` traktowal kazdy taki callback jako
powod do natychmiastowego flushu. W efekcie 15 logicznych zapisow fio dawalo
30 callbackow FUSE i 30 operacji persistence.

Kontrola 256/320/384 KiB pozostawala wyrownana i wykonywala jeden zbiorczy
persistence przy koncowym `fsync`.

## Zmiana

Granica callbacku FUSE nie jest traktowana jako granica logicznego `write(2)`.
Przy jednym otwartym `fh` czesciowy callback pozostaje w `WriteState` i moze
zostac uzupelniony przez kolejny callback tego samego logicznego zapisu.

Auto-flush nadal wystepuje, gdy:

- `write_flush_threshold_bytes` zostal osiagniety,
- ten sam plik ma wiecej niz jeden aktywny `fh`.

Semantyka wielu uchwytow pozostaje zabezpieczona przez istniejace mechanizmy:

- `write()` drenuje i scala pending sibling states przed aktualnym zapisem,
- `read()` publikuje pending sibling states przed odczytem,
- `fsync`, `flush` i `release` publikuja dirty state.

Dodano log debug `op=write flush_decision`, ktory pokazuje:
`buffered_bytes`, `shared_open_handles`,
`partial_block_visibility_write` i wynik `should_flush`.

## Test regresji

Test jednostkowy wymaga, aby:

- pojedynczy `fh` + czesciowy callback nie wymuszal visibility flush,
- dwa lub wiecej `fh` nadal wymuszaly visibility flush.

Po zmianie benchmark diagnostyczny 512 KiB powinien zachowac techniczne
rozbicie callbackow, ale wykonac jeden zbiorczy persistence zamiast persistence
per callback.

## Zasada pomiaru

Wynik wydajnosci odczytu jest uznawany za miarodajny tylko w scenariuszu
primary-write -> WAL replay -> zatrzymanie primary -> restart PostgreSQL
replica -> replica-read, zgodnie z `zasady_sprawdzen.md`.

Nie obnizamy `fuse_max_write_bytes=512KiB`, `block_size=4096`,
`read_ahead_blocks=4` ani `sequential_read_ahead_blocks=8` na podstawie tej
regresji.


## Walidacja 64 MiB

Po poprawce wykonano benchmark primary-write -> replica-read dla 64 MiB.

| parametr | 256 KiB | 512 KiB |
|---|---:|---:|
| WRITE | 46.5 MiB/s | 61.4 MiB/s |
| READ replica | 37.7 MiB/s | 36.8 MiB/s |
| FOD write callbacks | 256 | 256 |
| persist_operation_count | 1 | 1 |
| persist_input_rows_total | 16384 | 16384 |
| repo_persist_blocks_us | 869746 | 855128 |
| update_write_buffer_us | 245074 | 53658 |

Dla obu wariantow primary_flush_lsn i replica_replay_lsn wyniosly
`0/3650B18`. Regresja persistence per techniczny callback 512 KiB
nie wystapila. Limit 512 KiB pozostaje bez zmian.

Przed push nalezy dodatkowo pokryc kontrakt wielu fh bez jawnego flush/fsync.
