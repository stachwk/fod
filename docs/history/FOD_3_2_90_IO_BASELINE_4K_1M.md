# FOD 3.2.90 - referencyjny baseline I/O 4 KiB -> 1 MiB

## Cel

Ten dokument zapisuje stan wydajnosci FOD 3.2.90 jako punkt odniesienia
dla przyszlych wersji.

Wyniki NIE sa podstawa do zmiany aktualnej konfiguracji i NIE oznaczaja,
ze rozmiar z najwyzsza przepustowoscia powinien zostac ustawiony jako nowy
domyslny parametr. Ich celem jest pokazanie, na jakich wartosciach obecna
wersja FOD operuje przy kontrolowanym sekwencyjnym workloadzie.

Przy przyszlych wersjach nalezy powtorzyc ten sam scenariusz i porownac
wyniki punkt po punkcie: 4K z 4K, 8K z 8K, ... 1M z 1M.

## Identyfikacja wersji

- FOD: `3.2.90`,
- commit bazowy: `c3b675487061e50feac990ada03553ba3dc2d5f1`,
- commit subject: `FOD 3.2.90: restore 512 KiB default FUSE writes`.

## Metodologia

Scenariusz: `primary write -> unmount -> WAL replay -> stop primary ->
restart replica -> fresh replica read`.

Warunki:

- rozmiar pliku: 256 MiB,
- `fio ioengine=sync`,
- `iodepth=1`,
- jeden job,
- sekwencyjny write i sekwencyjny read,
- testowane `fio bs`: 4K, 8K, 16K, 32K, 64K, 128K, 256K, 512K, 1M,
- 3 przebiegi na kazdy rozmiar,
- kolejnosc sweepow: rosnaco, malejaco, rosnaco,
- write wykonywany na PostgreSQL primary,
- read wykonywany po zatrzymaniu primary na swiezo uruchomionej replica,
- host kernel page cache nie jest globalnie czyszczony,
- FOD read cache i wewnetrzny read-ahead scenariusza sa wylaczone,
- `FOD_FOPEN_DIRECT_IO=1`,
- `FOD_FUSE_WRITEBACK_CACHE=0`.

## Negocjacja FUSE obserwowana we wszystkich przebiegach

- `requested_max_write=524288`,
- `effective_max_write=524288`,
- `requested_max_readahead=524288`,
- `effective_max_readahead=131072`.

Czyli userspace FOD prosil o 512 KiB dla write i readahead, kernel przyjal
512 KiB dla write oraz 128 KiB jako efektywny limit FUSE readahead.

## Wyniki referencyjne

| fio bs | WRITE med [MiB/s] | WRITE mean | WRITE min | WRITE max | READ med [MiB/s] | READ mean | READ min | READ max |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 4K | 36.10 | 35.93 | 34.80 | 36.90 | 8.35 | 8.29 | 8.16 | 8.36 |
| 8K | 39.90 | 40.03 | 38.80 | 41.40 | 12.50 | 12.33 | 12.00 | 12.50 |
| 16K | 45.10 | 45.17 | 45.00 | 45.40 | 17.10 | 17.10 | 16.20 | 18.00 |
| 32K | 48.50 | 48.53 | 47.30 | 49.80 | 22.00 | 21.87 | 21.10 | 22.50 |
| 64K | 50.30 | 49.97 | 47.90 | 51.70 | 29.20 | 29.13 | 28.90 | 29.30 |
| 128K | 52.90 | 53.30 | 52.30 | 54.70 | 32.60 | 33.07 | 32.20 | 34.40 |
| 256K | 52.60 | 53.00 | 51.90 | 54.50 | 37.40 | 37.77 | 37.30 | 38.60 |
| 512K | 52.50 | 52.97 | 52.40 | 54.00 | 38.70 | 38.77 | 38.70 | 38.90 |
| 1M | 53.50 | 53.80 | 53.10 | 54.80 | 39.30 | 39.43 | 39.30 | 39.70 |

## Persistence - stan referencyjny

Dla KAZDEGO rozmiaru I/O i dla wszystkich przebiegow obserwowano:

- `persist_operation_count=4`,
- `persist_input_rows_total=65536`,
- `persist_input_rows_max=16384`.

W sweepie walidowane byly rowniez:

- `persist_copy_stage_count=4`,
- `persist_data_blocks_merge_count=4`.

To jest wazna czesc baseline: przyszla wersja nie powinna poprawiac
przepustowosci kosztem niezamierzonego zwielokrotnienia persistence.

## Charakterystyka FOD 3.2.90

Ten rozdzial opisuje ksztalt obserwowanej krzywej, bez rekomendacji
konfiguracyjnej.

- WRITE rosnie wyraznie od 4K do okolic 128K.
- WRITE od 128K do 1M pozostaje w zblizonym zakresie ok. 52-54 MiB/s.
- READ rosnie wraz z `fio bs` w calym badanym zakresie.
- READ dla 4K wynosi ok. 8.35 MiB/s mediany.
- READ dla 256K wynosi ok. 37.40 MiB/s mediany.
- READ dla 512K wynosi ok. 38.70 MiB/s mediany.
- READ dla 1M wynosi ok. 39.30 MiB/s mediany.
- `fio bs=1M` jest wieksze od `effective_max_write=512KiB`, a mimo tego
  persistence pozostaje na referencyjnym poziomie 4 operacji dla 256 MiB.

## Jak porownywac przyszle wersje

Dla nowej wersji FOD nalezy zachowac identyczna metodologie i raportowac
co najmniej:

1. mediane WRITE dla kazdego `fio bs`,
2. mediane READ dla kazdego `fio bs`,
3. min/max lub rozrzut z powtorzen,
4. requested/effective FUSE max_write i max_readahead,
5. `persist_operation_count`,
6. `persist_input_rows_total`,
7. `persist_input_rows_max`,
8. zgodnosc WAL primary/replica i zatrzymanie primary przed read.

Porownanie powinno wskazywac regresje/poprawe osobno dla kazdego punktu.
Nie nalezy zmieniac konfiguracji tylko dlatego, ze jeden punkt sweepu ma
najwyzsza wartosc przepustowosci.

## Artefakty pomiaru

Oryginalny lokalny zestaw wynikow:

`artifacts/perf/sweep-3.2.90-4k-to-1m/20260819T232052/`

Pliki podsumowania utworzone przez benchmark:

- `results.csv`,
- `summary.txt`.
