# Wymagania FUSE i kernela Linux dla FOD

## Cel

Ten dokument opisuje wymagania i zalecane ustawienia FUSE/kernela Linux dla
FOD, ze szczegolnym uwzglednieniem maksymalnego rozmiaru requestu I/O.

Dokument rozroznia:

1. wymagania zgodnosci - potrzebne do poprawnej pracy FOD,
2. bezpieczne ustawienia domyslne FOD,
3. tuning wydajnosciowy hosta - zalecany dla duzego sequential I/O, ale nie
   wymagany do poprawnosci,
4. ustawienia globalne kernela, ktorych FOD celowo nie zmienia samodzielnie.

Wnioski w tym dokumencie sa oparte na pomiarach FOD 3.3.12/3.3.13 wykonanych
25 sierpnia 2026 na Linuxie z `PAGE_SIZE=4096`.

## 1. Wymagania podstawowe

FOD wymaga dzialajacego FUSE3 i kernela Linux pozwalajacego zamontowac
filesystem przez `fuser`.

Podczas startu FOD raportuje m.in.:

```text
kernel_protocol=...
negotiated_protocol=...
available_capabilities=...
```

Dla duzych requestow szczegolnie istotna jest capability:

```text
MAX_PAGES
```

Nie jest ona twardym wymaganiem poprawnosci FOD. Na testowanym kernelu byla
dostepna i umozliwiala negocjowanie wiekszych requestow. Brak `MAX_PAGES` nie
oznacza automatycznie, ze FOD nie moze dzialac, ale ogranicza mozliwosci
tuningu duzego I/O i powinien byc traktowany jako ograniczenie wydajnosciowe
hosta.

## 2. `fs.fuse.max_pages_limit`

Linux ogranicza maksymalna liczbe stron pamieci, ktore moga wejsc do jednego
requestu FUSE.

Sprawdzenie:

```bash
cat /proc/sys/fs/fuse/max_pages_limit
sysctl fs.fuse.max_pages_limit
getconf PAGE_SIZE
```

Przy typowym:

```text
PAGE_SIZE = 4096
fs.fuse.max_pages_limit = 256
```

teoretyczny limit danych na pojedynczy request wynosi:

```text
4096 * 256 = 1048576 bytes = 1 MiB
```

FOD 3.3.13 loguje:

```text
kernel_page_size_bytes=...
kernel_max_pages_limit=...
kernel_max_request_bytes=...
estimated_request_ceiling_bytes=...
```

Dzieki temu wartosc ustawiona przez FOD nie jest mylona z rzeczywistym
limitem wynikajacym z kernela.

## 3. Domyslna wartosc FOD 3.3.13

Od FOD 3.3.13 domyslna wartosc:

```text
FOD_FUSE_MAX_WRITE_BYTES=1MiB
```

zastepuje poprzednie `512KiB`.

Domyslny `FOD_FUSE_MAX_READAHEAD_BYTES` pozostaje:

```text
512KiB
```

Na standardowym hostowym:

```text
fs.fuse.max_pages_limit=256
PAGE_SIZE=4096
```

1 MiB jest bezpiecznym domyslnym sufitem FOD, poniewaz odpowiada maksymalnemu
requestowi, jaki taki kernel moze przyjac przez `MAX_PAGES`.

Nie jest wymagane zmienianie sysctl, aby korzystac z nowego domyslnego
ustawienia FOD.

## 4. Wynik pomiaru dla domyslnego hosta

Powtarzany lokalny A/B:

```text
QNAP=0
plik=1GiB
fio block size=1MiB
noatime + direct_io
cache/read-ahead/prefetch FOD=0
3 przebiegi na profil
```

dal mediany:

| Profil | max_pages | FOD max_write | READ | WRITE | read callbacks |
|---|---:|---:|---:|---:|---:|
| FOD 3.3.12 baseline | 256 | 512KiB | 283 MiB/s | 60.8 MiB/s | 3072 |
| FOD 3.3.13 default | 256 | 1MiB | 322 MiB/s | 61.3 MiB/s | 2048 |

Zmiana tylko po stronie FOD daje:

```text
READ: +13.8%
read callbacks: -33.3%
WRITE: bez istotnej regresji
```

Dlatego `1MiB` jest przyjete jako zalecany i domyslny limit FOD 3.3.13.

## 5. Zalecany tuning hosta dla duzego sequential I/O

Dla hostow przeznaczonych do duzych sekwencyjnych transferow mozna rozwazyc:

```text
fs.fuse.max_pages_limit=512
FOD_FUSE_MAX_WRITE_BYTES=2MiB
```

Przy stronie 4096 B daje to teoretycznie:

```text
4096 * 512 = 2097152 bytes = 2 MiB
```

W powtarzanym A/B ten profil dal:

```text
READ median = 374 MiB/s
WRITE median = 61.3 MiB/s
read callbacks = 1024
```

Wzgledem baseline 512KiB/256 stron:

```text
READ: +32.2%
read callbacks: -66.7%
fused SQL total_us: ok. -26.7%
WRITE: bez istotnej regresji
```

To jest obecnie sugerowany profil wydajnosciowy dla hosta FOD, jezeli
priorytetem jest duzy sequential read.

## 6. Zmiana sysctl

`fs.fuse.max_pages_limit` jest parametrem globalnym hosta i dotyczy wszystkich
filesystemow FUSE na tym kernelu.

FOD celowo NIE wykonuje:

```bash
sysctl -w fs.fuse.max_pages_limit=...
```

automatycznie.

Administrator powinien podjac te decyzje jawnie.

Zmiana tymczasowa:

```bash
sudo sysctl -w fs.fuse.max_pages_limit=512
```

Kontrola:

```bash
sysctl fs.fuse.max_pages_limit
```

Przywracanie typowej wartosci testowanego hosta:

```bash
sudo sysctl -w fs.fuse.max_pages_limit=256
```

Jesli tuning ma byc trwaly, mozna utworzyc np.:

```text
/etc/sysctl.d/90-fod-fuse.conf
```

z zawartoscia:

```conf
fs.fuse.max_pages_limit = 512
```

i zaladowac ustawienia zgodnie ze standardowa procedura administracyjna
dystrybucji, np.:

```bash
sudo sysctl --system
```

Przed zastosowaniem ustawienia na wspoldzielonym hoście trzeba uwzglednic inne
filesystemy FUSE dzialajace na tej samej maszynie.

## 7. Dlaczego samo `FOD_FUSE_MAX_WRITE_BYTES=2MiB` nie wystarcza

Test pokazal:

```text
fs.fuse.max_pages_limit=256
FOD_FUSE_MAX_WRITE_BYTES=2MiB
effective_max_write=2097152
```

ale rzeczywisty odczyt 1 MiB nadal byl dzielony przez kernel na:

```text
16 B
1048560 B
```

czyli praktycznie jeden limit 1 MiB plus fragment wynikajacy z wyrownania.

Po zmianie:

```text
fs.fuse.max_pages_limit=512
FOD_FUSE_MAX_WRITE_BYTES=2MiB
```

ten sam fio 1 MiB generowal:

```text
1048576 B
```

w jednym callbacku.

Oznacza to, ze `effective_max_write` zwracane przez setter biblioteki nie jest
samodzielnie wystarczajaca informacja o realnym limicie requestu kernela.
Dlatego FOD 3.3.13 raportuje dodatkowo kernelowy limit stron i bajtow.

## 8. Wartosc `512` jako zalecenie, nie wymaganie

`fs.fuse.max_pages_limit=512` jest obecnie zaleceniem wydajnosciowym opartym na
powtarzanym benchmarku FOD.

Nie jest to wymaganie poprawnosci.

Nie nalezy automatycznie ustawiac wartosci wiekszych niz:

```text
512
```

ani `FOD_FUSE_MAX_WRITE_BYTES` wiekszego niz:

```text
2MiB
```

bez osobnych testow na docelowym kernelu, pamieci, filesystemach FUSE i
obciazeniu.

Obecne benchmarki potwierdzaja korzysc do 512 stron/2 MiB. Nie ma jeszcze
danych uzasadniajacych wyzsze wartosci jako domyslne lub zalecane.

## 9. Wplyw na pamiec i wspolbieznosc

Wiekszy request FUSE oznacza potencjalnie wiekszy pojedynczy bufor i wiecej
danych przetwarzanych przez jeden callback.

Przy wielu rownoleglych operacjach wzrost limitu moze zwiekszac chwilowe
zuzycie pamieci.

Dlatego tuning nalezy oceniac lacznie z:

```text
liczba watkow FUSE
liczba rownoleglych read/write
FOD payload in-flight budget
PostgreSQL pool
dostepna pamiec hosta
```

Nie nalezy traktowac maksymalnego requestu jako niezaleznego parametru.

## 10. Sugerowane profile

### Profil standardowy FOD 3.3.13

Bez zmiany sysctl:

```text
fs.fuse.max_pages_limit=256   # typowa wartosc badanego hosta
FOD_FUSE_MAX_WRITE_BYTES=1MiB
FOD_FUSE_MAX_READAHEAD_BYTES=512KiB
```

To jest domyslny profil FOD 3.3.13.

### Profil large sequential I/O

Jawny tuning administratora:

```text
fs.fuse.max_pages_limit=512
FOD_FUSE_MAX_WRITE_BYTES=2MiB
FOD_FUSE_MAX_READAHEAD_BYTES=512KiB
```

Stosowac po benchmarku na docelowym hoscie.

### Profil zgodnosci / diagnostyki

Jesli trzeba odtworzyc historyczne zachowanie 3.3.12:

```text
FOD_FUSE_MAX_WRITE_BYTES=512KiB
```

bez potrzeby zmiany sysctl.

## 11. Szybka kontrola hosta

```bash
echo "PAGE_SIZE=$(getconf PAGE_SIZE)"
echo "FUSE_MAX_PAGES=$(cat /proc/sys/fs/fuse/max_pages_limit)"
echo "FUSE_MAX_BYTES=$(( $(getconf PAGE_SIZE) * $(cat /proc/sys/fs/fuse/max_pages_limit) ))"
```

Dla standardowego profilu badanego hosta oczekiwano:

```text
PAGE_SIZE=4096
FUSE_MAX_PAGES=256
FUSE_MAX_BYTES=1048576
```

Dla profilu large sequential I/O:

```text
PAGE_SIZE=4096
FUSE_MAX_PAGES=512
FUSE_MAX_BYTES=2097152
```

## 12. Kontrola w logu FOD

Po starcie FOD nalezy sprawdzic linie:

```text
FOD FUSE compatibility:
FOD FUSE negotiated:
```

Dla FOD 3.3.13 druga linia raportuje m.in.:

```text
requested_max_write=...
effective_max_write=...
requested_max_readahead=...
effective_max_readahead=...
kernel_page_size_bytes=...
kernel_max_pages_limit=...
kernel_max_request_bytes=...
estimated_request_ceiling_bytes=...
```

Jesli skonfigurowany request jest wiekszy niz limit kernela, FOD raportuje
ostrzezenie zamiast udawac, ze caly skonfigurowany rozmiar jest osiagalny.

## 13. Testy po zmianie

Po zmianie ustawien FUSE lub sysctl nalezy wykonac co najmniej:

```bash
cd ~/git/fod

QNAP=0 make test-mount-suite
QNAP=0 make test-all
```

Dla pomiaru wydajnosci nalezy dodatkowo powtorzyc benchmark na docelowym
rozmiarze pliku i block size.

## 14. Podsumowanie

Najwazniejszy kontrakt FUSE dla FOD 3.3.13:

```text
FOD domyslnie:
  FOD_FUSE_MAX_WRITE_BYTES = 1MiB
  FOD_FUSE_MAX_READAHEAD_BYTES = 512KiB

typowy host:
  PAGE_SIZE = 4096
  fs.fuse.max_pages_limit = 256
  kernel request ceiling ~= 1MiB

zalecany tuning dla large sequential I/O:
  fs.fuse.max_pages_limit = 512
  FOD_FUSE_MAX_WRITE_BYTES = 2MiB
  kernel request ceiling ~= 2MiB

zasady:
  FOD nie zmienia globalnego sysctl automatycznie
  512 stron / 2MiB jest zaleceniem wydajnosciowym, nie wymogiem poprawnosci
  wartosci wyzsze wymagaja osobnych benchmarkow
```

Szczegolowe wyniki benchmarkow znajduja sie w `BENCHMARKS.md`.
