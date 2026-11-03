# PR #30 — walidacja ownership na dwoch rzeczywistych hostach

**Status:** scenariusz przygotowany; brak wyniku z dwoch hostow. Nie traktowac wynikow
`mount_smoke` (dwa mounty na jednym hoście) jako zamiennika tego testu.

## Cel i izolacja

Zweryfikowac, ze dwie rozne maszyny z odrebnymi jadrami Linux i procesami FOD,
ale **ta sama izolowana baza testowa PostgreSQL** i tym samym filesystemem FOD:

1. widza te same, zatwierdzone bajty;
2. nie otrzymuja rownoczesnie prawa do zapisu tego samego pliku;
3. poprawnie przekazuja ownership po `close()` i asynchronicznym `FUSE_RELEASE`;
4. odczytuja z obu hostow identyczny wynik `A+B`.

Test nie sprawdza automatycznie, do jakiej bazy podpieto **procesy montujace**.
Nalezy to porownac w ich konfiguracjach i logach **przed uruchomieniem**.
Nie uruchamiac na bazie `foddbname` ani na montowaniu produkcyjnym.

Test korzysta tylko z jednego nowego pliku
`fod-pr30-real-two-host-<TEST_ID>.bin`. Nie wykonuje `mkfs`,
`ALTER`, `DROP`, bezposrednich mutacji SQL, restartu uslug ani
administracyjnego wymuszenia wygasniecia dzierzaw.

## Wymagania

- Dwa **odrebne hosty**, oznaczone jako A oraz B. Dwa mounty na tym samym hoscie
  nie wystarczaja do zaliczenia walidacji rzeczywiscie wielohostowej.
- Na kazdym hoscie aktywne montowanie FOD, bazujace na tym samym serwerze
  PostgreSQL (wlasciwy primary) i tej samej bazie **testowej**.
- To samo wydanie FOD i zgodny schemat bazy na obu hostach. Kod
  produkcyjny powinien odpowiadac HEAD PR #30 `6cf77b9`.
- Python 3, `mountpoint`, `findmnt`, dostep do testowego montowania i
  uprawnienia do utworzenia pliku. Test ma tylko zaleznosci standardowe Pythona.
- Zmienne `FOD_MOUNT` (lokalna sciezka do aktywnego mountu) i `TEST_ID`
  (identyczny na obu hostach, np. `pr30-20261103-01`).
- Sprawdzic `hostname` i `findmnt`; wyniki musza pochodzic z dwoch
  roznych hostow i pokazywac FUSE. Do dowodu zapisywac dane bez hasel.

Na **obu** hostach:

```bash
cd ~/git/fod
git fetch origin
git switch test/pr30-real-two-host-ownership
git pull --ff-only

# Ustaw na kazdym hoscie rzeczywista lokalna sciezke do aktywnego FOD.
# export FOD_MOUNT="/twoja/sciezka/do/mountu/fod"
export TEST_ID="pr30-20261103-01"

hostname
git rev-parse HEAD
mountpoint -q "$FOD_MOUNT" && findmnt -T "$FOD_MOUNT" -o TARGET,FSTYPE,SOURCE
```

Dodatkowo porownac uzywana baze i endpoint PostgreSQL w konfiguracji obu
uruchomionych mountow. Samo ustawienie `FOD_PG_DBNAME` w terminalu
**nie przestawia juz dzialajacego mountu**.

## Sekwencja krok po kroku

**1. Host A — utworz plik i utrzymuj otwarty writer.** Polecenie nie
konczy sie, dopoki nie nacisniesz Enter w tym terminalu:

```bash
python3 tests/integration/test_real_two_host_ownership.py \
  --mount "$FOD_MOUNT" --test-id "$TEST_ID" --verbose 1 a-hold
```

Zapisz `A_HOLD_READY` (identyfikator pliku, rozmiar, SHA256, host). Nie
naciskaj jeszcze Enter.

**2. Host B — odczyt, odmowa przejecia ownership.** Przy aktywnym A:

```bash
python3 tests/integration/test_real_two_host_ownership.py \
  --mount "$FOD_MOUNT" --test-id "$TEST_ID" --verbose 1 b-deny
```

Warunek PASS: `B_DENY_PASS errno=16` i niezmienione 64 KiB danych.
Inne `errno`, mozliwosc zapisu lub niespojny payload sa FAIL.

**3. Host A — nacisnij Enter** w terminalu `a-hold`. Zapisz
`A_RELEASED`.

**4. Host B — przejecie ownership i append 8 KiB:**

```bash
python3 tests/integration/test_real_two_host_ownership.py \
  --mount "$FOD_MOUNT" --test-id "$TEST_ID" --timeout 5 --verbose 1 b-takeover
```

Warunek PASS: `B_TAKEOVER_PASS`, rozmiar 73728 B, hash dokladnie
`A+B`. Skrypt ponawia jedynie przejsciowe `EBUSY`, nie dluzej
niz `--timeout` sekund; nie omija fencing.

**5. Host A — sprawdz konwergencje po modyfikacji B:**

```bash
python3 tests/integration/test_real_two_host_ownership.py \
  --mount "$FOD_MOUNT" --test-id "$TEST_ID" --timeout 5 --verbose 1 a-verify
```

Warunek PASS: `A_VERIFY_PASS`. FAIL moze wskazac stale dane w
cache starego mountu, nawet jesli PostgreSQL i odczyt na B sa poprawne.
Zapisz wtedy wynik; **nie czysc danych przed diagnostyka**.

**6. Host B — sprzatanie po sukcesie wszystkich krokow:**

```bash
python3 tests/integration/test_real_two_host_ownership.py \
  --mount "$FOD_MOUNT" --test-id "$TEST_ID" cleanup
```

Skrypt usuwa wyłącznie konkretny plik testowy i tylko po sprawdzeniu
jego zawartosci.

## Kryteria decyzji

PASS tylko jesli:

- `hostname` wskazuje dwa rozne hosty oraz oba mounty prowadza do
  tej samej izolowanej bazy PostgreSQL.
- `A_HOLD_READY`, `B_DENY_PASS`, `A_RELEASED`, `B_TAKEOVER_PASS`,
  `A_VERIFY_PASS` wystepuja w tej kolejnosci.
- B nie moze przejac ownership, dopoki A trzyma fd.
- Po zwolnieniu ownership B zapisuje, fsyncuje i odczytuje pelne `A+B`.
- A konwerguje do tych samych danych. Brak zbieznosci to osobna usterka
  spójnosci cache, a nie powod do oslabienia PostgreSQL fencing.

Nie uruchamiac `cleanup` przy FAIL, dopoki nie sa zachowane logi obu
klientow i dane diagnostyczne PostgreSQL.

## Dodatkowe scenariusze przed merge

Ten scenariusz obejmuje **normalny handoff**. Oddzielnie do wykonania:

- przetrwanie awarii klienta A oraz wygasniecie sesji/lease bez naruszania
  aktualnego tokenu fencing B;
- odrzucenie starego writera A po przejeciu przez B (bez SQL podmieniajacego
  wlasciciela recznie);
- kontrola `file_open_leases` i `file_write_leases` po testach;
- pomiar opóznienia 1–16 porownan blokowych przy zdalnym primary oraz kosztu
  identycznego zapisu; uwzglednic cache kernel/FUSE, aby nie mylic pomiaru
  wywolania `write()` z liczba zapytan PostgreSQL.

PR #30 pozostaje niescalony do czasu walidacji tych ryzyk.
