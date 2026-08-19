# FOD 3.2.89 - domyslny limit zapisu FUSE 256 KiB

## Decyzja

Domyslna wartosc `fuse_max_write_bytes` zmienia sie z `512KiB` na `256KiB`.

Domyslna wartosc `fuse_max_readahead_bytes` pozostaje `512KiB`.

Zmiana dotyczy zarowno:

- fallbacku w `rust_fuse/src/compatibility.rs`,
- lokalnego `fod_config.ini`,
- publicznego `fod_config.example.ini`.

Jawny override `fuse_max_write_bytes=512KiB` oraz
`FOD_FUSE_MAX_WRITE_BYTES=512KiB` pozostaja obslugiwane.

## Uzasadnienie

Pomiary primary-write -> replica-read pokazaly, ze 512 KiB moze byc dzielone
przez kernel/FUSE na dwa callbacki, podczas gdy 256 KiB w obserwowanym
srodowisku dochodzi jako pojedynczy callback.

FOD 3.2.87 usunal koszt persistence per techniczny fragment, wiec 512 KiB
jest poprawne semantycznie i moze byc nadal uzywane. Dla ustawienia domyslnego
wybieramy jednak 256 KiB jako bardziej przewidywalna granice callbacku.

Odczytu nie ograniczamy razem z zapisem. `fuse_max_readahead_bytes` pozostaje
512 KiB, poniewaz jest niezaleznym limitem negocjacji read-ahead.

## Niezmienione parametry

- `block_size=4096`,
- `persist_buffer_chunk_blocks=128`,
- `read_ahead_blocks=4`,
- `sequential_read_ahead_blocks=8`,
- `write_flush_threshold_bytes=64MiB` w konfiguracji bazowej.

## Kryteria regresji

Walidacja musi potwierdzic:

1. `DEFAULT_FUSE_MAX_WRITE_BYTES == 256 * 1024`,
2. `DEFAULT_FUSE_MAX_READAHEAD_BYTES == 512 * 1024`,
3. oba bazowe pliki INI maja write `256KiB` i readahead `512KiB`,
4. konfiguracja i mount suite przechodza,
5. wersja workspace jest spójna z `fod_version.txt`.
