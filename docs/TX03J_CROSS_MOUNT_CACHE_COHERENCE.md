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
