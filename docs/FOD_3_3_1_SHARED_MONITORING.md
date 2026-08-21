# FOD 3.3.1 - wspolna telemetria `fod-monitor`

## Cel

FOD 3.3.1 zmienia `fod-monitor` z narzedzia przede wszystkim lokalnego w punkt
obserwacji calej instalacji FOD korzystajacej z tej samej bazy PostgreSQL.

Lokalne dane z `/proc` nadal sluza do diagnozy konkretnego hosta. Wspolnym
zrodlem prawdy dla statystyk miedzyhostowych jest PostgreSQL.

## Model

Kazda aktywna writable sesja ma rekord w `fod.client_sessions` i maksymalnie
jeden rekord w `fod.monitor_session_stats`. Rekord statystyk zawiera normalne
kolumny identity/czasu (`session_id`, `fod_version`, `sample_seq`, `sampled_at`)
oraz wersjonowany `payload_json` typu JSONB.

Payload ma `schema_version` i sekcje `read`, `write`, `copy`, `database`,
`persistence` oraz `timings`. Dzieki temu kolejne liczniki nie wymagaja
rozbudowy tabeli o dziesiatki nowych kolumn.

## Dokladnosc i koszt

Liczniki ruchu sa skumulowane od startu procesu i reprezentuja callbacki oraz
bajty faktycznie zaobserwowane na granicy FUSE. FOD nie wykonuje dodatkowego
UPDATE przy kazdym callbacku. Migawka jest publikowana domyslnie co 5 s, wiec
baza reprezentuje stan dokladny do ostatniej udanej publikacji. Normalne
zamkniecie publikuje probke koncowa; po twardym crashu moze zginac jedynie
koncowka od ostatniej probki. Predkosci w `top` sa liczone z czasu probek z
dokladnoscia mikrosekundowa, a nie z czasu odswiezania ekranu monitora.
Payload zapisuje tez faktyczny `publish_interval_millis`. Prog stale telemetry
wynosi `max(15 s, 3 * publish_interval)`, wiec wolniejsze poprawne probkowanie
nie jest falszywie raportowane jako stale.

Interwal:

```bash
FOD_MONITOR_PUBLISH_INTERVAL_MS=5000
```

Zakres: 500-60000 ms. Publikacja telemetrii odswieza rowniez liveness rekordu
`client_sessions` do FOD 3.3.3. Od FOD 3.3.4 liveness sesji odnawia osobny
heartbeat sesji, a publikacja telemetrii zapisuje tylko `monitor_session_stats`.
Interwal heartbeat'u sesji jest taki sam jak interwal publikacji monitora, a TTL
centralnej sesji wynosi `max(30 s, 3 * publish_interval)`. TTL sesji jest
niezalezny od `lock_lease_ttl`, wiec heartbeat lockow PostgreSQL nie moze
skrocic sesji przy rzadszym probkowaniu.

Telemetria nie zmienia semantyki lock managera: `lock_heartbeat_interval=0`
nadal wylacza heartbeat lockow, a `lock_backend=memory` nie uruchamia heartbeat
lockow PostgreSQL. Od FOD 3.3.3 sprzatanie wygaslych `client_sessions` jest
osobnym maintenance i dziala niezaleznie od backendu lockow. Od FOD 3.3.4 lock
heartbeat odnawia tylko lock leases/owner state, a session maintenance tylko
sprzata. Prune sesji usuwa tez historyczne wygasle lock rows z `session_id=0`.
Sam UPSERT telemetrii jest widoczny w kolejnych licznikach operacji bazy jako
koszt monitoringu.

Tabela jest ograniczona do jednego rekordu na `session_id` i jest czyszczona
kaskadowo razem z sesja. 3.3.1 celowo nie tworzy nieograniczonej historii.


Od FOD 3.3.3 identity hosta preferuje niepuste `HOSTNAME`, ale gdy zmienna nie jest eksportowana, FOD pobiera nazwe hosta z systemowego `gethostname()`. Wartosc `unknown` jest dopiero ostatnim fallbackiem.

`FOD_SESSION_MAINTENANCE_INTERVAL_MS` steruje okresem sprzatania wygaslych `client_sessions`: domyslnie 5000 ms, zakres 500-60000 ms. Ten maintenance nie odnawia lock leases i nie zmienia semantyki `lock_heartbeat_interval`.

## Widoki

- `fod-monitor cluster` - tylko wspolny widok PostgreSQL.
- `fod-monitor status` - wspolny klaster, potem lokalny host.
- `fod-monitor top` - klaster + lokalny host; `READ_BPS`, `WRITE_BPS` i
  `COPY_BPS` sa liczone z roznic kolejnych centralnych probek.
- `fod-monitor report` - dodatkowo szczegoly per sesja: DB, persistence,
  failovery i timingi FUSE.

Monitor korzysta ze standardowej sekcji `[database]`. Opcjonalny osobny DSN:

```bash
FOD_MONITOR_DSN="host=... dbname=... user=... password=..." fod-monitor cluster
```

DSN i haslo nie sa wypisywane. Jezeli monitor odczytuje fizyczna replike,
`source_role=replica` jawnie sygnalizuje, ze widok moze byc opozniony o replay WAL.
Obliczenia wieku sesji, lease i probek sa wykonywane jawnie w UTC niezaleznie
od `TimeZone` sesji PostgreSQL uzywanej przez `fod-monitor`.

## PostgreSQL

Wymagane i sprawdzane ustawienia PostgreSQL sa opisane w
[`POSTGRESQL_REQUIREMENTS.md`](POSTGRESQL_REQUIREMENTS.md).

## Ograniczenie 3.3.1

Publikacja dotyczy writable mountow, bo wymaga DML na primary PostgreSQL.
Mount dzialajacy wylacznie na fizycznej read-only replice nie publikuje jeszcze
centralnej telemetrii. Kolejny etap moze dodac osobny telemetry/control endpoint
do primary dla takich mountow.
