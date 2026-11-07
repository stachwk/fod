# FOD PR #30 — QNAP PostgreSQL Primary + dwa demony FUSE na lt7300

## Topologia i ograniczenia

- QNAP: istniejacy PostgreSQL **Primary**, dostepny w sieci (domyslnie
  ustawienie Makefile: `QNAP_PG_HOST=192.168.1.11`, port 5432).
- lt7300: dwa niezalezne procesy/montowania FOD A i B, ten sam endpoint
  PostgreSQL, ta sama odizolowana baza `*_test`.
- Wariant sprawdza zdalne round-trip do PostgreSQL, transakcyjne ownership,
  fencing, handoff, spojnosc dwoch widokow i wydajnosc po sieci.
- **Nie** jest to test dwoch niezaleznych jader Linux: A i B dziela jedno
  jadro lt7300. PostgreSQL na QNAP pozostaje osobnym hostem.

Targety sa na galęzi `test/pr30-real-two-host-ownership`, utworzonej
z aktualnego HEAD PR #30. Nie dodaja nowej uslugi Docker i nie zmieniaja
konfiguracji istniejacego PostgreSQL.

## Bezpieczenstwo

Nie testowac na `foddbname` ani na innej bazie produkcyjnej.
Uzyj **dedykowanej** `fod_pr30_qnap_test`; target wymaga koncowki
`_test`. Skrypt przeprowadza wstepna kontrole endpointu i stanu
`mkfs.fod status`, a nastepnie uruchamia istniejace testy integracyjne.

Target `test-pr30-qnap-remote-ownership`:
- **nie** wywoluje `docker compose up/down`, `make init`, `reset`,
  `mkfs init` ani `mkfs upgrade`;
- przed startem odrzuca brak `QNAP=1`, nazwe bazy bez `_test`,
  niespojne endpointy QNAP i endpoint oznaczony jako standby;
- wymaga `FOD ready: yes` w istniejacej bazie;
- w fazie aktywnej uruchamia juz istniejacy test C7, ktory tworzy
  i usuwa tylko swoje unikalne pliki oraz testuje ownership dwoch daemonow.

Target `test-pr30-qnap-remote-ownership-full` dodatkowo:
- uruchamia test C8, ktory celowo odwoluje lease dla **wlasnego pliku
  testowego**; to test destruktywny wzgledem lease, tylko dla bazy `*_test`;
- uruchamia `test_dual_host_same_file_cp_race.py` (dwa procesy FUSE
  na tym samym hoscie i rywalizacja o 64 MiB pliku).

Testy nadal modyfikuja dane w **dedykowanej bazie testowej**. Nie
uruchamiaj ich w bazie zawierajacej inne wazne dane.

## Krok 1 — przygotowanie komputera i odczyt ustawien

```bash
cd ~/git/fod
git fetch origin
git switch test/pr30-real-two-host-ownership
git pull --ff-only

source scripts/fod-local-libfuse-env.sh

# Przypisania QNAP_* w Makefile wybieraja zdalny serwer.
# Jesli QNAP uzywa innego adresu, przekaz QNAP_PG_HOST=... w make.
# Nie zapisuj hasla do logow i nie wklejaj tu jego wartosci.
```

Nie uruchamiaj `make qnap-up`, `make reset`, `make init`,
`make docker-deploy-install` w ramach tego testu.

## Krok 2 — baza testowa (jednorazowe przygotowanie)

Wymagane: odrebna baza PostgreSQL o nazwie `fod_pr30_qnap_test`,
utworzona na QNAP **przez operatora po weryfikacji endpointu**.
Nalezy na niej wykonać `mkfs.fod init` lub `upgrade` tylko wtedy,
gdy jest to nowa/odziedziczona baza testowa. Nie uzywac
`init-qnap` ze wskazaniem na produkcyjna baze.

Nowy target **nie tworzy ani nie aktualizuje schematu**. Jesli baza
nie istnieje lub nie jest gotowa, konczy prace przed zapisami testowymi.

## Krok 3 — test podstawowy

```bash
cd ~/git/fod
make QNAP=1 \
  QNAP_PG_DBNAME=fod_pr30_qnap_test \
  FOD_RUNTIME_PROFILE=dev \
  test-pr30-qnap-remote-ownership
```

**Oczekiwanie:** `PR30 QNAP preflight PASS` oraz C7 PASS (w tym `EBUSY`,
poprawny fencing i dane). W razie niepowodzenia preflight:
sprawdz aktualny endpoint, istnienie bazy i wynik `mkfs.fod status`
wyłącznie dla dedykowanej bazy testowej.

Uwaga: odziedziczone zmienne `FOD_PG_*` z poprzednich testow moga
wplywac na proces kompilacji lub mkfs; sam target wymusza endpoint
QNAP dla zapytania weryfikacyjnego i uruchamianych testow.
W razie watpliwosci sprawdz niejawne ustawienia bez wyswietlania hasel.

## Krok 4 — test rozszerzony (dopiero po C7 PASS)

```bash
make QNAP=1 \
  QNAP_PG_DBNAME=fod_pr30_qnap_test \
  FOD_RUNTIME_PROFILE=dev \
  test-pr30-qnap-remote-ownership-full
```

Target rozszerzony najpierw **ponownie** wykonuje C7, potem C8
(stary token po przejeciu lease), a na koncu test 64 MiB CP race.
Na zdalnym QNAP test 64 MiB moze przekroczyc 120 s i wtedy potrzebna
bedzie diagnostyka opóznien, nie usuniecie fencing.

## Kryteria PASS

- PostgreSQL odpowiada przez siec z lt7300 jako Primary.
- Oba daemony FOD korzystaja z tej samej `fod_pr30_qnap_test`.
- B nie otrzymuje prawa zapisu, dopoki A ma aktywne ownership.
- Po zwolnieniu ownership operacje B koncza sie sukcesem, bez utraty
  danych, a stare tokeny sa odrzucane.
- Testy nie dotykaja produkcyjnego `foddbname`.
- Osobno zapisac RTT do PostgreSQL, czasy odczytow/zapisow i wyniki
  z obciazeniem 1–16 blokow `no-op`.

## Status

2026-11-07 testy wykonano z `lt7300` do dedykowanej bazy
`fod_pr30_qnap_test` na QNAP `192.168.1.11:5432`. Przed uruchomieniem
potwierdzono PostgreSQL 16.15 Primary, `pg_config --configure` z
`--with-blocksize=32`, runtime `SHOW block_size=32768` i schemat FOD 39.
`fod-rust-mkfs status` (FOD 3.4.536) zwrocil `Latest schema shape: yes`,
`FOD ready: yes` i brak pending migrations. Testy nie wykonywaly `init`,
`upgrade` ani resetu schematu.

Target `test-pr30-qnap-remote-ownership-full` zakonczyl sie kodem 0:

- C7 PASS: dwa mounty; przegrywajacy zapis otrzymal `EBUSY` (errno 16),
  poprawne dane, `bytes=65536`.
- C8 PASS: fencing token zmienil sie `11->12`; stary uchwyt otrzymal
  `EIO` (errno 5), wlasciciel z nowym tokenem wykonal zapis.
- 64 MiB same-file copy race PASS: A wygral, B otrzymal `EBUSY`, oba
  mounty widzialy hash zwycieskiej kopii, `ownership_leaks=0`.

To byly dwa niezalezne procesy FUSE na jednym hoście klienckim `lt7300`,
nie test dwoch hostow Linux. Nie sprawdzono osobnego RTT ani macierzy
obciazenia `no-op` 1–16 blokow; pozostaja poza tym przebiegiem. Testowano
wylacznie `fod_pr30_qnap_test`; produkcyjna `foddbname` nie byla celem
polaczen ani testow.
