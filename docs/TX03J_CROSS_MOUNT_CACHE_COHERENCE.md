# TX03J — spojnosc odczytow miedzy montowaniami FOD

## Punkt wyjscia (2026-10-16)

TX03I.3 potwierdzil poprawne przejecie write ownership (stary token 3205,
nowy 3206), odrzucenie stale renew/release i zgodnosc danych w montowaniu B
oraz w nowym montowaniu C. Wczesniej uzyte montowanie A odczytalo poprzednie
dane (`stale_read_on_a=1`). To jest osobny problem spojnosci odczytow,
nie dowod niepoprawnego fencing.

## Wymagany kontrakt

PostgreSQL pozostaje autorytatywnym stanem zawartosci i ownership.
Po zakonczonym `fsync` na montowaniu B nowe otwarcie pliku w A ma
odczytac zatwierdzone dane zgodnie z przyjetym modelem spojnosci.
Stare uchwyty i semantyka niezatwierdzonych zapisow sa testowane oddzielnie.
FOD nie powinien obiecywac natychmiastowej koherencji dla juz
zbuforowanych stron jadra bez mechanizmu ich uniewazniania.

## Etapy

1. **TX03J.1 — diagnoza**: powtorzyc A -> B na dwoch montowaniach,
   rozdzielic natychmiastowy i opozniony odczyt A oraz swiezy mount C.
   Porownac zwykle FUSE, tryb `FOD_FOPEN_DIRECT_IO=1` oraz odczyt
   przy wylaczonym wewnetrznym read cache (`FOD_READ_CACHE_BLOCKS=0`).
   Nie zmieniac zachowania produkcyjnego przed uzyskaniem wynikow.
2. **TX03J.2 — implementacja**: zidentyfikowac warstwe odpowiedzialna
   (kernel page cache vs read block cache vs metadata cache) i przygotowac
   bezpieczny tryb koherencji z wersjonowaniem w PostgreSQL lub
   odpowiednia walidacja/inwalidacja przy otwarciu/odczycie.
   Rozwazyc kompromis wydajnosciowy. Nie stosowac globalnego
   `direct_io` bez decyzji o zgodnosci i kosztach.
3. **TX03J.3 — testy**: macierz dwoch mountow, mount C, ponowne open,
   dotychczasowy fd, fsync i rozne rozmiary pliku; potwierdzic brak
   regresji TX03G / TX03I.1 / TX03I.2 / TX03I.3 oraz pomiar wydajnosci.
4. **TX03J.4 — multi-host**: powtorzyc po obu stronach oddzielnych hostow,
   sprawdzic TTL, recovery po rozlaczeniu oraz obciazenie PostgreSQL.

## Kryterium zamkniecia

Nie zamykamy TX03J po samym poprawnym fencing. Nowe odczyty z A musza
zwracac dane B zgodnie z jawnie udokumentowanym kontraktem i testy
regresyjne musza przechodzic. Wynik diagnostyczny z niespojnym A
pozostaje FAIL/OPEN nawet gdy testy ownership sa PASS.

## Decyzja architektoniczna (zatwierdzona)

Wszystkie klienty FOD — rowniez na roznych hostach — musza obserwowac
autorytatywny stan PostgreSQL, niezaleznie od `direct_io=0/1`. Nie
wystarczy poprawny fencing zapisu. Aktualnosc tresci i metadanych jest
osobnym kontraktem.

Docelowa implementacja:

1. Dwie niezalezne monotoniczne generacje: `content_generation` dla
   zmian danych i `namespace_generation` (oraz wersjonowanie metadanych)
   dla zmian wpisow. `file_id` moze pozostac bez zmiany podczas zapisu.
2. Zmiana generacji w tej samej transakcji PostgreSQL co modyfikacja.
   Kazdy odczyt sprawdza wersje zgodnie z zadeklarowanym kontraktem
   synchronizacji i nie uzywa danych z niewlasciwej generacji.
3. Powiadomienia `LISTEN/NOTIFY` do szybkiego lokalnego uniewazniania,
   ale bez polegania wylacznie na dostarczeniu powiadomienia.
4. Inwalidacja cache Rust (w tym mapowania blokow, ostatnich zapisow
   i metadanych) oraz Linux/FUSE (data, attr, entry). Jest obowiazkowa
   rowniez przy `direct_io=0`.
5. Gdy walidacja generacji nie dziala, scisly tryb nie moze podawac
   nieweryfikowanego cache jako aktualnych danych.
6. Osobne testy: nowy `open`, ponowny `read` ze starego uchwytu,
   wspolbieznosc zapisu, przerwanie `LISTEN`, awaria i reconnect,
   unlink/recreate, rename i hardlink; testy wydajnosci po zmianach.

### Potwierdzone wyniki TX03J.1

Szesc konfiguracji (domyslna, direct I/O, brak read cache, kombinacja,
TTL metadanych zero oraz wszystkie przelaczniki jednoczesnie)
odczytalo stare dane w A, nawet po 2 sekundach. B i swiezy C
odczytaly zatwierdzone dane. Jest to FAIL spojnosc odczytu;
`TX03J_DIAGNOSTIC=PASS` oznacza jedynie wykonanie pomiaru.

### Bramy wdrozenia

- **J2a**: instrumentacja metadanych `file_id/data_object_id` i mapy
  blokow, ustalenie zrodla stale A bez maskowania bledu.
- **J2b**: atomowe generacje PostgreSQL + walidacja przed odczytem
  i inwalidacja cache procesu oraz jadra FUSE.
- **J2c**: kanal powiadomien i uzgadnianie generacji po reconnect;
  powiadomienia nie stanowia zrodla prawdy.
- **J3/J4**: przejscie testow wielomontazowych, wielohostowych,
  crash recovery oraz testow wydajnosci. Stare A musi widziec nowe
  dane zgodnie z kontraktem; brak PASS dla samego poprawnego B/C.

Nie wlaczamy globalnie direct I/O jako obejscia.
