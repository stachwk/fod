# TX03J — plan implementacji koherencji cache między montowaniami

**Status:** PROPOSED / NOT IMPLEMENTED  
**Data audytu:** 2026-11-01  
**Punkt odniesienia:** FOD 3.4.529, commit `3a616fe`, PostgreSQL schema 39.  
**Właściciel priorytetów i statusu:** [CURRENT.md](CURRENT.md). Ten dokument jest planem wykonawczym TX03J, nie konkurencyjną kolejką prac.

Dokument uzupełnia [kontrakt TX03J](../TX03J_CROSS_MOUNT_CACHE_COHERENCE.md) i [audyt generacji](../tx03j-generation-audit.md). Jest planem, nie dowodem poprawności ani deklaracją gotowości produkcyjnej.

## Cel i granica poprawności

PostgreSQL **primary** jest źródłem prawdy dla treści, tożsamości `file_id`, generacji, fencing, kolejności zmian i aktywnych sesji. Po **udanym** `fsync()` na montowaniu B odczyt rozpoczęty później na dowolnym innym aktywnym montowaniu A/C musi widzieć zatwierdzoną treść B także z włączonym cache jądra, dla `direct_io=0` i `direct_io=1`. Nie wolno inwalidować obcych `file_id`, usuwać niezapisanych buforów ani uznawać za sukces utraconych ACK. Przed zamknięciem etapu trzeba określić semantyk odczytów trwających równolegle z barierą oraz mmap; nie rozszerzać gwarancji ponad to, co można potwierdzić.

**Znane podstawy (source-reviewed):** migracje 0026/0027, jawny `content_generation += 1` w głównych transakcyjnych writerach, atomowy statement snapshot `fetch_block_range_with_generation_shared`, lokalny `FileCacheGenerationGate` i poprawka resetowania cache w FOD 3.4.529. **Braki:** generation-aware ordinary cached read, primary routing generacyjnego API, trwały outbox/replay, runtime FUSE notifier, ACK barrier, testy wyścigów i macierz hostów. Nie edytować historycznych migracji.

## Zasady realizacji w Codex

- Dzielić pracę na **małe, niezależnie testowalne zmiany**, preferując pojedynczą funkcję lub wąski moduł. Najpierw test regresyjny, potem poprawka. Każde nowe zachowanie za flagą i fail-closed w trybie strict.
- Wszystkie transakcje modyfikujące dane, generację oraz trwały event muszą commitować lub wycofywać się **razem**; PostgreSQL primary rozstrzyga zapis i kolejność.
- Nie opierać bezpieczeństwa na `LISTEN/NOTIFY`, TTL, globalnym flush, czasie zegara hosta, `direct_io` ani samym `pg_is_in_recovery()=false`.
- Po każdym commicie `git diff HEAD~1..HEAD --check`, `git show --stat HEAD`, przegląd zmienionych plików i ewentualnych regresji. Każdy commit zwiększa patch version według [versioning.md](../versioning.md).
- Nie modyfikować produkcyjnej bazy ani credentials. Testy zapisujące tylko na odizolowanej bazie i mountach. Brak dostępu do bazy: `BLOCKED_ENV`, nie `FAIL` kodu.

## Bramy implementacyjne

| Brama | Priorytet | Zakres kodu / SQL | Kryterium PASS |
| --- | --- | --- | --- |
| J0 — preflight i regresja cache | P0 | `rust_fuse/src/read_cache.rs`, testy FUSE, test DB | deterministyczny disable → remote write → enable → first read; brak powrotu starego bloku, brak wpływu na drugi plik |
| J1 — primary snapshot | P0 | `rust_hotpath/src/pg/file_reads.rs` | `size + generation + data_object_id + blocks` z **jednego** snapshotu na primary; replica celowo opóźniona nie może udawać źródła strict |
| J2 — ordinary read + gate | P0 | `rust_fuse/src/fs.rs`, `read_cache.rs` | cache keyed/validated przez `file_id + content_generation + local_epoch`; opóźniony fill generacji N nie publikuje po N+1; dirty buffers nietknięte |
| J3 — durable change ledger | P0 | nowa migracja **następna po bieżącym schemacie** (dla 39: kandydat `0040`), `rust_mkfs`, `rust_hotpath` | event atomowy z bumpem, rollback bez eventu, bezpieczna kolejność replay, duplikaty i restart nie gubią zmian |
| J4 — runtime reconcile + targeted FUSE invalidation | P0 | mount startup/notifier, listener worker, `fs.rs` | event invaliduje tylko dany `file_id`, kernel inode/page/attr oraz właściwy entry dla namespace; reconnect odtwarza brakujące zmiany **przed** czytaniem |
| J5 — membership, ACK i `fsync` | P0 | istniejące `client_sessions` / lease fencing, `fsync` | ustalona lista uczestników, ACK po udowodnionej invalidacji i rozliczeniu in-flight reads; brak ACK → jawny błąd, nigdy fałszywy sukces |
| J6 — namespace | P1 | parent/name versioning + entry notifications | rename, unlink/recreate i hardlink nie aliasują starego path/inode; odrębna generacja namespace |
| J7 — rollout / HA / performance | P0 | testy A/B/C oraz osobnych hostów | pełna macierz PASS, reconnect i paused-replica PASS, bezpieczeństwo dirty buffers, kontrolowany narzut i brak global flush |

### J0. Kontrola środowiska i pierwsze regresje

1. Potwierdzić aktualny HEAD i schemat. Badany HEAD to 3.4.529, runtime wymaga **schema 39**, nie historycznej 26. Sprawdzić port primary (`55441` w ostatnim środowisku `lt7300`), poświadczenia z bezpiecznego źródła i `fod.config` (`block_size`, `max_fs_size_bytes`).
2. Test `content_generation.rs` ma izolowany schema i wymaga osobnej bazy z `test` w nazwie; pełny `pg_query` wymaga **zainicjalizowanej**, aktualnej bazy FOD. Nie wykorzystywać działającego deploymentu jako fixture destructive.
3. Odtworzyć sekwencję: zapełnij cache A → `FOD_READ_CACHE_BLOCKS=0` → commit innego mountu B → przywróć limit → pierwszy read A. Powtórzyć dla recent-write cache, direct I/O i in-flight fills. Sprawdzić, że plik C pozostaje nienaruszony.
4. Testy bez PostgreSQL można uruchamiać niezależnie. Zapisuj dokładne `PASS/FAIL/BLOCKED_ENV`, wersje kernela i libfuse.

### J1. Authoritative generation snapshot

Dodać lub wydzielić wariant `fetch_block_range_with_generation_authoritative_shared` korzystający z istniejącego mechanizmu `ConnectionLane::Write`. Zachować jedno-statementowy snapshot, nie rozdzielać SELECT metadata i SELECT data na dwa niepowiązane obrazy MVCC. Zmienić minimalną liczbę call-site'ów: opt-in strict/generation path najpierw; legacy bez regresji. Testować: missing file, EOF, pusty plik, COW, truncate, paused WAL replay i committed write z B. Dopuszczalne przyszłe optymalizacje replica reads wyłącznie po dowodzie bezpiecznego replay LSN / read-fence.

### J2. Bezpieczna publikacja i normalny read

Wpiąć wynik generacyjny w `FileCacheGenerationGate`; obejmować weryfikacją **cache hit i cache fill**, również po przełączeniu limitu. Publikacja i kontrola epoki muszą być zsynchronizowane względem inwalidacji per-file. Starsza generacja jest odrzucana, retry musi być ograniczone, z fallback do authoritative uncached read lub kontrolowanego błędu. Żaden globalny mutex nie może być trzymany przez operację SQL lub kernel notification. Testy z barierami deterministycznie ustawiają kolejność: fill N → commit N+1 → invalidate → finish fill N. Drugi plik pozostaje w cache. Wciąż **nie** deklarować strong cross-mount w kernel page-cache: ta gwarancja wymaga J4/J5.

### J3. Trwały dziennik: decyzje, których nie wolno pominąć

Projektować zdarzenia `{event_order, file_id, content_generation, event_kind, data_object_id?}` i osobne namespace events. Zmiana zawartości, generacja i outbox w tej samej transakcji. Możliwy trigger AFTER UPDATE na `files.content_generation` zmniejsza ryzyko pominięcia eventu przez nowy writer, lecz wymaga testów migracji i wpływu na throughput.

**Krytyczny warunek kolejności:** `BIGSERIAL`/sequence ID są alokowane **przed COMMIT**. Transakcja T1 może dostać ID=10, T2 ID=11 i commitować pierwsza; `WHERE event_id > 11` po T2 może bezpowrotnie pominąć później zatwierdzone T1. Nie wdrażać naiwnego cursor-by-serial.

Propozycja na pierwszą implementację: licznik kolejności aktualizowany transakcyjnie pod blokadą **wiersza kontrolnego dla filesystemu** podczas publikacji eventu. Wymusza to kolejność zatwierdzeń zgodną z numerami zdarzeń; rollback wycofuje przydział. Jest potencjalnym bottleneckiem i wymaga benchmarku pod równoległym zapisem. Alternatywą jest protokół oparty na log-decodowaniu/commit LSN z osobnym zatwierdzonym watermarkiem albo reconciliacja ze snapshotu autorytatywnych generacji. **Wybór i dowód bezpieczeństwa replay są obowiązkowym ADR/gate przed migracją**; same timestamps i pg_notify nie spełniają warunku.

Zdarzenia muszą przetrwać unlink, dopóki każdy uprawniony odbiorca może wymagać replay. Opracować retention/GC na podstawie bezpiecznego watermarku sesji, bez bezwarunkowego `ON DELETE CASCADE` do `files`. Zbadać skutki DDL i kosztów na obciążeniu. Przeprowadzić test `T1 allocates → T2 commits → replay → T1 commits`.

### J4. Reconciliation i kernel FUSE

Mount musi rejestrować sesję z unikalną epoką/właścicielem; po (re)start/reconnect uzgadniać brakujące zdarzenia z primary **zanim** dopuści odczyty obsługiwane z cache. `LISTEN/NOTIFY` jest tylko wake-up. Inwalidacja oddziela: read-block cache, recent-write **read snapshot** (nie dirty writer), metadata, readahead, inode page/attr i entry name. Kontrola `generation` odrzuca duplikaty i starsze zdarzenia. Lokalny epoch aktualizować przed notifierem; opóźniona odpowiedź zdarzenia N nie może cofnąć N+1.

Sprawdzić dostępność rzeczywistego fuser `Notifier` w produkcyjnym mount startup; dotychczas był tylko hook testowy. Osobno ustalić mapę `file_id → inode` dla otwartych i unlinkowanych plików, hardlink aliases oraz zakresów invalidacji. Gdy notifier/PG jest niedostępny, tryb strict **nie może** podawać starych stron z page cache jako świeżych; bezpiecznie odłączyć/odmówić lub zastosować udowodniony fallback. Nie czyścić całego filesystemu.

### J5. Bariera `fsync()` i sesje

Zdefiniować rejestr aktywnych mountów na primary, identity/epoch sesji, membership snapshot w granicy eventów i zasady fencing przy wygasłym lease. ACK jest trwały i idempotentny: odnosi się do konkretnej sesji/epoki oraz wysokiego watermarku, nie tylko do odebranego NOTIFY.

**Warunek ACK musi być zweryfikowany doświadczalnie:** sam sukces wywołania userspace `Notifier` nie musi dowodzić, że strony jądra zostały usunięte i że równoległy odczyt nie odtworzy starych danych. Opracować kolejność gate → drain/fence in-flight readers → kernel invalidation → potwierdzenie osiągalnego poziomu wykonania. Jeżeli libfuse/kernel nie zapewnia wymaganej synchronicznej gwarancji, nie raportować sukcesu strong-contract; zmienić protokół lub wyraźnie zawęzić kontrakt dopiero po osobnej decyzji.

`fsync()` może zwrócić sukces dopiero po COMMIT oraz ACK wymaganych aktywnych sesji. Timeout/przerwany peer zwraca błąd; PostgreSQL commit może już być trwały — konieczna idempotencja retry i jednoznaczne logi. Sesja uznana za martwą musi być **rzeczywiście fenced i nie może dalej obsługiwać cache**, zanim zostanie pominięta w barierze. Osobne testy stale session, reconnect session epoch i nowego mountu.

### J6. Metadata/namespace coherence

`unlink` bez zmiany treści prawidłowo nie podnosi `content_generation`. Wprowadzić osobny protokół `namespace_generation` i targeted `FUSE_NOTIFY_INVAL_ENTRY` obejmujący dokładne parent+name, rename (oba parenty), hardlink i unlink/recreate. Tożsamości `file_id` nie zastępować ścieżką.

### J7. Macierz zamknięcia i rollout

| Scenariusz | Minimalny dowód |
| --- | --- |
| A ma stary cache, B zapisuje i wykonuje `fsync`, C jest świeży | A/B/C odczytuje nowe bajty po sukcesie B, także A ze starym FD |
| `direct_io=0/1`, cache on/off/on, TTL zero/nonzero | ten sam wynik, bez globalnego flush |
| fill N blokowany, commit N+1, delayed completion N | N nie jest publikowane |
| paused replica | strict-read nie akceptuje starego snapshotu |
| utracony NOTIFY, reconnect, restart w środku replay | replay dociera do aktualnego watermarku, bez luki |
| dwóch writerów i odwrotna kolejność commitów | monotonia generation i kompletność eventów |
| dirty bufory A + remote event B | pending writes A nienaruszone, ownership/fencing zachowany |
| unlink/recreate, rename, hardlink | brak aliasowania inode/entry |
| mount bez ACK / heartbeat timeout | `fsync` nie zwraca fałszywego sukcesu; fencing sesji przed wykluczeniem |
| wielomontowaniowy stres ≥100 operacji, osobne hosty | brak starego cache, deadlock, stale resurrection, brak global invalidation |

Najpierw J0–J2 na jednym hoście i bezpiecznej bazie, następnie J3–J5 z wyłączonym domyślnie mechanizmem, potem J6/J7 na wielu hostach. Mierzyć obciążenie PostgreSQL, latencję read, `fsync` ACK p50/p95/p99, liczbę eventów/s i GC ledger. **Nie włączać domyślnie** przed kompletem PASS.

## Walidacja minimalna i raport Codex

```bash
cd ~/git/fod
git status --short --branch
git rev-list --left-right --count HEAD...origin/main
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --locked -p fod-rust-fuse read_cache::tests
cargo test --locked -p fod-rust-hotpath --lib
git diff --check
```

Testy PostgreSQL tylko po sprawdzeniu odizolowanej bazy, aktualnego schematu i hasła:

```bash
# Wymaga bezpiecznie przygotowanej bazy i FOD_PG_*; nie uruchamiac na production.
cargo test --locked -p fod-rust-mkfs --test content_generation -- --nocapture
cargo test --locked -p fod-rust-hotpath --test pg_query generated_block_snapshot_matches_committed_payload -- --exact --nocapture
```

Nie mieszać wyników testów przeprowadzonych 2026-10-23 z nieuruchomionymi testami na HEAD 3.4.529. Każda brama raportuje: commit, ścieżki, testy i wersję środowiska, dokładny PASS/FAIL/BLOCKED_ENV, brakujące dowody oraz decyzję GO/NO_GO. Wersję schematu, CLI i numery migracji odświeżać na początku każdego etapu; po 39 numer 0040 jest **warunkowy**.
