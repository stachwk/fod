# FOD 3.2.90 - domyslny limit zapisu FUSE 512 KiB

## Decyzja

Domyslna wartosc `fuse_max_write_bytes` wraca z `256KiB` do `512KiB`.

Domyslna wartosc `fuse_max_readahead_bytes` pozostaje `512KiB`.

Jawny override `fuse_max_write_bytes=256KiB` oraz
`FOD_FUSE_MAX_WRITE_BYTES=256KiB` pozostaja obslugiwane.

## Kontekst

FOD 3.2.89 ustawil 256 KiB jako konserwatywny default, poniewaz w obserwowanym
srodowisku 256 KiB dochodzilo jako pojedynczy callback FUSE, a 512 KiB bylo
technicznie dzielone na dwa callbacki.

FOD 3.2.87 usunal patologiczny persistence per techniczny fragment.
Dlatego 512 KiB jest poprawne semantycznie i nie powoduje juz eksplozji
operacji PostgreSQL.

## Benchmark porownawczy

Test primary-write -> WAL replay -> primary stopped -> replica restart ->
replica-read, 5 powtorzen na wariant, plik 256 MiB.

Pierwszy benchmark, w ktorym razem z limitem zmienial sie `fio bs`, pokazal:

- 256 KiB: WRITE mediana 50.60 MiB/s, READ mediana 36.50 MiB/s,
- 512 KiB: WRITE mediana 54.00 MiB/s, READ mediana 39.80 MiB/s.

Poniewaz zmienialy sie dwa parametry naraz, wykonano test izolowany.

W tescie izolowanym `fio bs=512k` bylo stale, a zmienial sie tylko
`FOD_FUSE_MAX_WRITE_BYTES`:

- max_write 256 KiB: WRITE mediana 51.40 MiB/s, READ mediana 36.80 MiB/s,
- max_write 512 KiB: WRITE mediana 51.40 MiB/s, READ mediana 39.40 MiB/s,
- roznica mediany WRITE: 0.00%,
- roznica mediany READ: +7.07% dla wariantu 512 KiB.

Wynik READ jest obserwacja benchmarkowa, a nie dowodem, ze `max_write`
bezposrednio steruje read throughput.

## Persistence

Dla obu wariantow we wszystkich przebiegach:

- `persist_operation_count=4`,
- `persist_input_rows_total=65536`,
- `persist_input_rows_max=16384`.

Oznacza to brak regresji split-write i zachowanie naturalnych flushow 64 MiB.

## Niezmienione parametry

- `block_size=4096`,
- `persist_buffer_chunk_blocks=128`,
- `read_ahead_blocks=4`,
- `sequential_read_ahead_blocks=8`,
- bazowy `write_flush_threshold_bytes=64MiB`.

## Kryteria regresji

Walidacja FOD 3.2.90 musi potwierdzic:

1. `DEFAULT_FUSE_MAX_WRITE_BYTES == 512 * 1024`,
2. `DEFAULT_FUSE_MAX_READAHEAD_BYTES == 512 * 1024`,
3. oba bazowe pliki INI maja write `512KiB` i readahead `512KiB`,
4. `256KiB` nadal moze byc uzyte jako jawny override,
5. konfiguracja i mount suite przechodza,
6. wersja workspace jest spojna z `fod_version.txt`.

## Referencyjny baseline wydajnosci

Pelny sweep sekwencyjnego I/O 4K -> 1M dla FOD 3.2.90 jest zapisany w:

`docs/FOD_3_2_90_IO_BASELINE_4K_1M.md`

Ten sweep jest punktem odniesienia dla przyszlych wersji i nie stanowi
rekomendacji zmiany aktualnej konfiguracji.
