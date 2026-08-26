# FOD - przyszle kierunki i wnioski

Stan: 2026-08-26.

Ten dokument zbiera propozycje do dalszej analizy i implementacji. Nie sa to
jeszcze zatwierdzone zmiany runtime ani gwarancje funkcjonalne.

## 1. Wewnetrzny autotuning na podstawie obciazenia per sesja

Pomysl: FOD moze stroic limity i zachowanie runtime dynamicznie na podstawie
rzeczywistego obciazenia obserwowanego dla aktywnej sesji, zamiast polegac
wylacznie na statycznych presetach.

Kandydaci do obserwacji per sesja:

- liczba aktywnych read/write taskow;
- liczba i czas transakcji PostgreSQL;
- sredni i maksymalny rozmiar requestow FUSE;
- liczba operacji DB na callback i na task;
- opoznienia read/write/flush;
- wykorzystanie cache, prefetch i write buffer;
- wielkosc payloadu aktualnie dopuszczonego do PostgreSQL;
- kolejki i czas oczekiwania na limity/admission gates;
- obciazenie CPU i pamieci procesu FOD;
- ewentualnie lag repliki dla sesji korzystajacej z read routing.

Mozliwy mechanizm:

1. zbierac telemetryke w ruchomym oknie czasu per sesja;
2. klasyfikowac sesje jako np. metadata-heavy, random-small-I/O,
   sequential-read, sequential-write albo mixed;
3. korygowac tylko bezpieczne limity runtime w zadanych granicach;
4. stosowac histereze i minimalny czas utrzymania ustawienia, aby uniknac
   oscylacji;
5. zawsze zachowywac globalne limity bezpieczenstwa procesu i PostgreSQL;
6. logowac kazda automatyczna zmiane wraz z powodem i metrykami wejscia;
7. umozliwic tryb tylko-obserwacja, ktory wylicza zalecenia bez zmiany
   parametrow.

Pierwszy etap powinien byc telemetryczny: policzyc, jakie parametry faktycznie
koreluja z poprawa throughput/latency i dopiero potem wlaczac automatyczne
sterowanie.

## 2. Szybsze buildy przez /dev/shm i kontrolowane czyszczenie target

Obserwacja: wszystkie artefakty Cargo trafiaja obecnie do `./target`, ktory po
wielu buildach i konfiguracjach zajmuje duzo miejsca. Dla lokalnych buildow i
testow mozna rozwazyc opcjonalne kierowanie `CARGO_TARGET_DIR` do tmpfs, np.
`/dev/shm`, aby ograniczyc I/O na dysku i przyspieszyc kompilacje.

Proponowany wariant:

```text
CARGO_TARGET_DIR=/dev/shm/fod-target
```

albo katalog per repo/uzytkownik, aby uniknac kolizji.

Warunki i ograniczenia:

- mechanizm ma byc opcjonalny i lokalny; nie zmieniac globalnie zachowania
  Cargo bez pomiaru;
- przed buildem sprawdzac dostepna pojemnosc `/dev/shm` i wymagany zapas RAM;
- nie uzywac tmpfs, jezeli mogloby to wywolac presje pamieci lub OOM;
- artefakty w `/dev/shm` sa nietrwale po restarcie i nie moga byc traktowane
  jako cache wymagany do poprawnosci;
- jezeli potrzebne sa finalne binaria/paczki, kopiowac je do trwalego katalogu
  dopiero po udanym buildzie;
- nie czyscic katalogu uzywanego przez aktywny build.

Osobny problem to rozrost zwyklego `./target`. Warto dodac kontrolowana polityke
sprzatania, np. na podstawie wieku i/lub maksymalnego rozmiaru. Czyszczenie ma
byc wykonywane tylko wtedy, gdy nie trwa build/test i powinno raportowac ile
miejsca odzyskano. Nalezy porownac proste `cargo clean` z bardziej selektywnym
usuwaniem starych artefaktow, aby nie tracic calego wartosciowego cache przy
kazdym sprzataniu.

Przed wdrozeniem nalezy wykonac A/B:

- `./target` na zwyklym filesystemie;
- `CARGO_TARGET_DIR` w `/dev/shm`;
- cold build i warm incremental build;
- zuzycie czasu, CPU, RAM, I/O i zajete miejsce.

## 3. FOD jako filesystem dla systemu operacyjnego: recovery i analiza wlaman

Potencjalny kierunek zastosowania: FOD moze byc przydatny jako filesystem dla
systemu operacyjnego lub wybranych jego warstw, jezeli historia zmian i recovery
pozwalaja odtworzyc stan sprzed awarii albo kompromitacji.

Mozliwe korzysci:

- szybki rollback zmian plikow systemowych do znanego dobrego punktu;
- porownanie stanu przed i po incydencie;
- odtworzenie kolejnosci zmian plikow;
- latwiejsza analiza powlamaniowa, jezeli FOD zachowuje wersje, czas, sesje i
  informacje o pochodzeniu zmian;
- mozliwosc uruchomienia odzyskanego systemu ze stanu sprzed incydentu, bez
  niszczenia materialu do analizy.

### Model separacji recovery

Zakladany model jest silniejszy niz zwykly filesystem z lokalnymi snapshotami:

- chroniony system operacyjny i PostgreSQL dzialaja na osobnych hostach;
- skompromitowany host systemowy nie ma dostepu administracyjnego do hosta
  PostgreSQL;
- napastnik moze poznac dane uwierzytelniajace FOD do bazy i moze uzyskac
  szerokie prawa logiczne do danych FOD, ale nie powinien otrzymywac dostepu do
  systemu operacyjnego hosta PostgreSQL;
- konto FOD nie powinno byc PostgreSQL `SUPERUSER`; powinno miec tylko prawa
  wymagane do pracy FOD i zarzadzania jego schematem/danymi;
- archiwum WAL i backupy PostgreSQL sa przechowywane poza zasiegiem
  kompromitowanego hosta, najlepiej na kolejnym odseparowanym hoście lub w
  repozytorium backupowym z osobnymi uprawnieniami;
- skompromitowany system nie ma uprawnien do kasowania ani nadpisywania
  historycznych backupow i WAL.

W takim modelu napastnik moze zmienic lub nawet zniszczyc biezacy logiczny stan
FOD widoczny przez swoje uprawnienia bazodanowe, ale nie moze zmienic
historycznego stanu utrwalonego w niedostepnych dla niego backupach i archiwum
WAL.

Po incydencie mozna wiec odtworzyc osobna instancje PostgreSQL z backupu i WAL,
wybierajac punkt czasu sprzed kompromitacji albo analizujac kolejne punkty czasu
w okresie incydentu. Odtwarzanie powinno odbywac sie na nowym/czystym hoście,
bez uzywania skompromitowanego systemu jako zaufanego elementu procesu recovery.

To daje dwa niezalezne zrodla wartosci:

1. operacyjne recovery - przywrocenie FOD i systemu do znanego dobrego stanu;
2. forensic timeline - odtworzenie kopii PostgreSQL dla wybranych LSN/czasow i
   porownanie zmian wykonanych podczas wlamania.

### Konsekwencje dla projektu

Dla tego zastosowania nalezy zaprojektowac i przetestowac:

- Point-in-Time Recovery PostgreSQL jako podstawowy mechanizm odtworzenia;
- retencje backupow i WAL wystarczajaca do cofniecia sie przed moment
  wykrycia incydentu;
- osobne credentiale i ACL dla runtime FOD, hosta PostgreSQL oraz systemu
  backupowego;
- zakaz uzywania przez FOD konta PostgreSQL `SUPERUSER`;
- mozliwosc montowania/analizy odzyskanej kopii FOD bez zapisu do materialu
  zrodlowego;
- eksport historii zmian do analizy forensic;
- rejestrowanie `session_id`, hosta, user/process i czasu zmiany tam, gdzie jest
  to wiarygodnie dostepne;
- test scenariusza: kompromitacja hosta systemowego -> zlosliwe zmiany FOD ->
  utrata biezacej bazy -> odtworzenie z backupu + WAL na osobnym hoście;
- test wyboru kilku punktow PITR przed, w trakcie i po incydencie oraz
  porownania zmian filesystemu.

Dodatkowa kryptograficzna integralnosc historii albo immutable/WORM backup moze
jeszcze wzmocnic model, ale nie jest warunkiem podstawowym, jezeli backup/WAL
sa juz fizycznie i uprawnieniowo odseparowane od kompromitowanego hosta i od
konta runtime FOD.

Wariant docelowy powinien pozwalac na dwa niezalezne rezultaty po incydencie:

1. szybkie odtworzenie dzialajacego systemu do bezpiecznego punktu;
2. zachowanie i odtwarzanie historycznego okresu kompromitacji do pozniejszej
   analizy.

## Kolejnosc dalszych prac

Proponowana kolejnosc:

1. autotuning najpierw w trybie obserwacyjnym bez automatycznej zmiany limitow;
2. A/B buildow `target` vs `/dev/shm` i projekt bezpiecznej polityki cleanup;
3. osobny threat model oraz PoC recovery/forensic z PostgreSQL PITR przed
   deklarowaniem FOD jako filesystemu dla calego systemu operacyjnego.
