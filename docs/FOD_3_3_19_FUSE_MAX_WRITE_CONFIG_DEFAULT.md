# FOD 3.3.19 - standard FUSE max_write ceiling 1 MiB

## Cel

FOD 3.3.19 zamyka niespojnosc miedzy kodem a standardowa konfiguracja.

Kod Rust od FOD 3.3.13 uzywa:

```text
DEFAULT_FUSE_MAX_WRITE_BYTES = 1 MiB
DEFAULT_FUSE_MAX_READAHEAD_BYTES = 512 KiB
```

ale do FOD 3.3.18 oba standardowe pliki:

- `fod_config.ini`;
- `fod_config.example.ini`;

jawnie ustawialy `fuse_max_write_bytes=512KiB`. Poniewaz konfiguracja runtime
jest przekazywana do `FOD_FUSE_MAX_WRITE_BYTES`, ten historyczny wpis
przeslanial poprawny domyslny limit kodu.

## Zmiana

FOD 3.3.19 ustawia w obu standardowych konfiguracjach:

```ini
fuse_max_write_bytes = 1MiB
fuse_max_readahead_bytes = 512KiB
```

Zmiana dotyczy wylacznie maksymalnego sufitu requestu FUSE.

Nie zmienia:

- formatu storage ani `block_size=4096`;
- `persist_buffer_chunk_blocks=128`, czyli bazowego chunka 512 KiB;
- `write_flush_threshold_bytes=64MiB`;
- `read_ahead_blocks`;
- `sequential_read_ahead_blocks`;
- `direct_io_read_prefetch_blocks`;
- `fuse_max_readahead_bytes=512KiB`;
- globalnego `fs.fuse.max_pages_limit`.

1 MiB nie wymusza operacji 1 MiB. Pozwala kernelowi/FUSE laczyc duze requesty
do 1 MiB tam, gdzie workload i kernel na to pozwalaja.

## Podstawa pomiarowa

FOD 3.3.13 zwalidowal na standardowym `max_pages_limit=256` i stronie 4096 B
zmiane samego sufitu 512 KiB -> 1 MiB dla sekwencyjnego 1 MiB I/O:

| max_write | read MiB/s | write MiB/s | read callbacks |
| --- | ---: | ---: | ---: |
| 512 KiB | 283 | 60.8 | 3072 |
| 1 MiB | 322 | 61.3 | 2048 |

Dalo to okolo +13.8% READ i -33.3% callbackow bez istotnej regresji WRITE.

Koncowa walidacja FOD 3.3.18 potwierdzila dodatkowo stabilna sciezke
read-after-write dla `randrw50:1m` przy limicie 1 MiB:

```text
READ median  = 44.487 MiB/s
WRITE median = 47.090 MiB/s
```

oraz 256 blokow po 4096 B na jedno `fod_fetch_block_range`, czyli pelny zakres
1 MiB.

## Walidacja 3.3.19

Status: zakonczona.

Zmiana przeszla regresyjny test obu standardowych konfiguracji, test
domyslnego limitu Rust, `cargo check --workspace --locked` oraz pelny
`QNAP=0 make test-all`.

Walidacja objela:

1. test obu standardowych plikow przez `fod-config runtime-config`;
2. test Rust `defaults_use_1m_write_and_512k_readahead`;
3. `cargo check --workspace --locked`;
4. `QNAP=0 make test-all`.

Regresyjny test konfiguracji musi wymagac:

```text
fuse_max_write_bytes = 1MiB
fuse_max_readahead_bytes = 512KiB
```

dla `fod_config.ini` i `fod_config.example.ini`.

Pelny gate korzysta z repozytoryjnego `fod_config.ini`, dlatego mount smoke
sprawdza rowniez rzeczywista sciezke `fod-bootstrap -> runtime env -> FUSE`.

## Kryterium zamkniecia

FOD 3.3.19 jest gotowy do push, jezeli:

- oba standardowe INI rozwiazuja `fuse_max_write_bytes` do `1MiB`;
- `fuse_max_readahead_bytes` pozostaje `512KiB`;
- pelny gate przechodzi;
- nie ma zmian w storage batching, PostgreSQL ani kernel sysctl;
- po commicie `git diff HEAD~1..HEAD` pokazuje tylko zamierzony zakres.
