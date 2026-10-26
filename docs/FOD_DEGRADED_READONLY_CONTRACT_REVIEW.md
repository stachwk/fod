# Degraded read-only contract review

Status (2026-10-25): design analysis with phase-one implementation in the
working tree and passing two-node and three-node Docker acceptance tests.
Automatic recovery/source switching and any change to the architecture
invariants remain unimplemented and unapproved.

## Assessment

Conditional use of primary-backed leases is worth investigating. It cannot
preserve ordinary open/unlink guarantees while the authority is unreachable,
but it can support an explicitly weaker read-only availability contract. The
standby remains a source of replayed data; it does not acquire authority over
leases, locks, namespace mutations, or conflicting operations on other hosts.

The earlier conclusion that leases make *all* replica-only reading impossible
was too broad. They prevent offering the existing filesystem contract without
a writable authority. A separate contract can allow reads to fail when an
object disappears, provided each successful read is consistent and failure
cannot be mistaken for valid file contents. The initial read-only path now has
a separate opt-in implementation, but this does not prove that the complete
recovery design below is implemented.

Refreshing files when primary connectivity returns is necessary, but does not
protect reads while the primary is unreachable. A network partition can leave
the primary accepting writes from other hosts. WAL already received or obtained
through another path can also continue advancing the standby.

## Source evidence

- `rust_fuse/src/startup.rs::mount_fuse` requires a client session and open-lease
  heartbeat on read-only mounts. This deliberately enforces the current
  contract and must remain the default behavior.
- `rust_hotpath/src/pg.rs::register_file_open_lease` checks the live session and
  linked file under a PostgreSQL advisory transaction lock. `purge_primary_file`
  takes the corresponding lock and checks active open leases before reclamation.
  This is the shared protection that a disconnected reader cannot renew.
- `rust_fuse/src/fs.rs::file_read_metadata_for_handle` can keep metadata for the
  lifetime of a read-only direct-I/O handle. Metadata and block fetching are
  separate calls in the normal read path.
- In `FodFuse::read`, a missing target block is legitimately returned as zero
  bytes for a sparse region. After a separate metadata read, replayed deletion
  can make a later block query empty too. A new degraded path must distinguish
  a missing file from a sparse region rather than assuming that skipping lease
  registration is sufficient. This is a source-derived race scenario, not a
  reproduced runtime failure or an explanation of the separate TX03J defect.
- `fetch_block_range_with_size_shared` and the strict uncached read path already
  combine file size/existence and payload in one SQL statement. The separate
  `fetch_block_range_with_generation_shared` API also returns size,
  `content_generation`, and blocks from one materialized statement snapshot.
  That generated API is opt-in and is not used by the ordinary `FodFuse::read`
  path. None of these APIs implements degraded epochs, endpoint identity checks,
  handle invalidation, or the full recovery protocol.

## Candidate contract

The current phase-one opt-in is
`FOD_DEGRADED_READONLY_ENABLED=1`. It requires role-aware endpoint routing
(`FOD_PG_ENDPOINT_ROUTING_ENABLED=1`), a replica endpoint configured in
`FOD_PG_REPLICA_HOSTS`, and `FOD_ROLE=replica`. FOD checks that routing selected
an observed physical replica and that startup sees PostgreSQL in recovery. The
mount remains read-only even when the caller omitted `--readonly`. Normal
read-only mounts keep their existing client/open-lease requirement.

This phase does not probe for role changes or switch data sources. Once the
primary returns or the mounted standby is promoted, this mount continues in
its startup mode until an operator-controlled unmount/remount. Dynamic
role-aware transition is a required follow-up; do not treat the current
implementation as automatic HA failover.

Entry is explicitly opt-in through a dedicated degraded-read-only mount
configuration. Ordinary mounts retain the current authoritative lease
requirements. The initial implementation must not silently convert an active
writable mount after an arbitrary connection error. A connection failure is
local evidence only: other hosts may still reach and write through primary, so
the degraded view must tolerate newer commits reaching the standby while it is
in use.

| Property | Normal mount | Candidate degraded view |
| --- | --- | --- |
| Authority | Validated writable PostgreSQL authority | No new authority; read replayed PostgreSQL state only |
| Lease handling | Register and renew primary-backed sessions/open leases | Do not create or renew leases, do not claim existing leases remain valid |
| Namespace/payload mutation | Existing transactional behavior | Reject with `EROFS`, including atime and metadata DML |
| Shared file locking | Existing PostgreSQL lease contract | Reject unsupported lock requests; do not simulate shared locks locally |
| Freshness | Normal authoritative freshness contract | Explicitly stale relative to primary; size, content generation, and returned blocks share one statement snapshot per FUSE read |
| Open/unlink | Authoritative open leases protect reclamation | No retention guarantee; disappearance invalidates the handle with `ESTALE` |
| Missing data | Existing sparse-file rules | Zero-fill only a hole established in a snapshot where the file exists |
| Return to primary | Existing operation | Quiesce, validate authority, discard degraded caches, invalidate old handles before admitting normal operations |

Replicated lease rows may be inspected for diagnostics, but must not authorize
an operation or prove liveness. Replay delay, expiry, and reader isolation make
them unsuitable as a substitute for a primary transaction. Local handle epochs
only reject operations within this process; they must never delay reclamation
or establish cross-host ownership.

Access checks also use potentially stale replica metadata. Permission changes
on an unreachable primary cannot be enforced immediately. The opt-in contract
must disclose this and let deployments requiring immediate revocation disable
degraded reads.

## Consistent reads during the outage

For the minimum candidate, every degraded read obtains file existence, size,
`content_generation`, the current data object, and requested blocks through
`fetch_block_range_with_generation_shared` or an equivalent single SQL
statement snapshot. Prefer this existing generated API over a new multi-query
transaction. If a future read requires multiple statements, use one read-only
repeatable-read transaction on one pinned connection; routing or pool checkout
must not split it across connections or replicas.

An absent object is `ESTALE` for an existing handle. A genuine end of file is
determined from the size in that same snapshot. Query cancellation, connection
loss, or failed validation returns an I/O error; it must not reuse old cached
metadata or manufacture a successful zero-filled response. A sparse hole is
valid only when its owning file/data object exists in the same snapshot.

The scope is one FUSE read request. A large application `read`, `cp`, or hashing
operation may issue many such requests and observe several committed versions
if another host changes the file. A stable full-file snapshot requires an
additional design; it must not be implied by the word "snapshot" in status or
documentation. Standby query cancellation may terminate even a pinned snapshot.

Initially bypass userspace content/handle-metadata caches and kernel data
caching in the degraded path, and validate the actual FUSE/mmap behavior. Direct
I/O alone is not proof that every cached access is intercepted. Lookup, inode,
directory, negative-entry, and attribute caches also need an explicit lifetime
and invalidation policy. Optimizing these caches can follow correctness tests.

## Transition and recovery protocol

1. Start only when the operator selected degraded-read-only mode and FOD
   validates a configured standby in recovery against trusted cluster and FOD
   filesystem identity. Do not infer cluster-wide primary failure from this
   client's inability to connect. Record the standby replay LSN; primary lag is
   unknown while primary cannot be reached.
2. Mount with mutations, atime DML, client-session/open-lease writes, and
   lease-backed lock operations disabled. Do not treat replicated lease rows as
   authority. Reject write opens, write syscalls, namespace/metadata changes,
   and unsupported locks with documented errors. Existing ordinary mounts
   continue to require writable lease authority.
3. Use uncached reads for degraded FUSE requests. Resolve file existence, size,
   `content_generation`, current data object, and blocks in one SQL statement
   snapshot. An absent file/object for an existing handle is `ESTALE`; only
   synthesize sparse zeros when that same snapshot proves the file/object
   exists. Query failure or cancellation returns `EIO`, never stale cached
   bytes.
4. Initially support degraded mode only for a separately started read-only
   mount. Do not carry dirty buffers or existing writable handles into it. A
   future live transition must stop mutation admission and settle each
   in-flight write as committed, failed, or outcome unknown; uncertain writes
   block transition and are never sent to the standby.
5. Probe the configured primary in the background. When reachable, enter
   `rejoining`, stop new reads, drain in-flight FUSE/SQL work, and prevent old
   replies/cache fills from publishing. Validate cluster identity, role,
   timeline ancestry, and deployment fencing/promotion evidence. Reachability,
   role, or system identifier alone is insufficient. Without trusted fencing
   evidence, do not restore writable mode automatically.
6. After validation, advance the local source epoch, discard userspace caches,
   invalidate kernel data/attribute/directory/negative-entry caches, and mark
   every old degraded handle `ESTALE`. Enable new opens only after authoritative
   client/open leases can be registered. New handles read through the primary
   until normal freshness rules permit another source. Never combine bytes from
   different source epochs in one response.
7. If draining, invalidation, lease registration, identity, or fencing checks
   fail, keep old handles stale and the mount unavailable/read-only. Use
   controlled unmount/remount until the live transition is proven by tests.

### Required dynamic behavior for an already mounted filesystem

The target behavior is for a mounted FOD process to monitor its configured
PostgreSQL endpoints and detect role/topology changes without requiring a
remount. Detection alone must never grant authority or change a read-only mount
to writable. When a promoted slave is detected, FOD stays read-only and
reports the candidate authority. An operator explicitly requests the
transition to normal mode. FOD then verifies deployment HA evidence that the
old primary is fenced, the promoted node is the unique writer for the same FOD
filesystem/cluster, and its timeline is valid. A PostgreSQL role probe,
successful connection, or system identifier by itself is insufficient.

The live state machine must support these transitions:

| Current mount state | Observed event | Required behavior |
| --- | --- | --- |
| Normal, primary-backed | Authority connection fails but no cluster-wide failure is established | Stop or fail operations that require the authority; do not infer global outage or serve potentially stale data automatically |
| Normal, primary-backed | HA integration certifies old primary fenced and a configured standby promoted | Quiesce operations, drain/resolve in-flight writes, validate identity/timeline, advance source epoch, replace routing/pools and resume only after leases are registered on the new primary |
| Degraded read-only on slave1 | slave1 is promoted and detected | Stay read-only, report the candidate authority, and wait for an explicit operator transition request |
| Degraded read-only on slave1 | Operator requests transition and HA integration certifies slave1 as the unique fenced primary | Stop reads, drain in-flight queries, invalidate old handles and caches, refresh endpoint routing, register fresh leases, then admit new normal opens; old degraded handles return `ESTALE` |
| Any state | Two endpoints report writable, fencing proof is absent/stale, or identity/timeline validation fails | Fail closed: do not enable writes; invalidate or reject reads as appropriate and report the reason |
| Normal on recovered primary | A configured replica catches up and passes role/identity/replay checks | Optionally return payload reads to that replica under the normal lag/fallback rules without changing lease authority |

Every transition needs a monotonically increasing process-local source epoch to
prevent an old asynchronous read or cache fill from publishing into the new
state. The epoch is only a local stale-work guard; PostgreSQL and the HA
controller remain the authorities for shared mutation order and fencing. The
mount must expose current state, authority, observed roles, transition errors,
and source epoch through diagnostics. If a transition cannot drain uncertain
writes, invalidate kernel/FUSE state, or safely re-establish leases, it must
remain unavailable and require operator recovery rather than silently
continuing.

The dynamic-transition acceptance suite must keep one mount process running
through: primary outage to degraded read-only; slave1 promotion; slave2
reparenting; detection and reporting of the candidate authority while writes
remain disabled; an explicit operator transition request; HA fencing proof
validation; live transition of slave1 from degraded reader to writable
authority; fresh-open success and old-handle `ESTALE`; and recovered replica
catch-up. It must also inject stale/missing fencing proof,
two writable endpoints, wrong cluster identity, divergent timeline, a delayed
read crossing the transition, and an in-flight write with unknown outcome.
Those negative cases must show no write admission and no data/cache publication
from the old source epoch. Exercise the same behavior from independent FOD
mounts on the master/slave hosts.

The degraded mount now has basic background role monitoring: it probes its
configured endpoints, logs role changes, and reports a promoted configured
replica as a candidate while keeping the mount read-only. This does not change
the mount's routing or mode. The in-process operator transition command,
transition barriers, and HA-controller proof interface remain unimplemented;
use controlled unmount/remount after promotion.

The three-node integration test now keeps a degraded mount on slave1 alive
through slave1 promotion. It verifies the role-change log, confirms the mount
stays mounted, and confirms a write remains rejected. The test then performs a
controlled remount to continue validating slave1 authority and slave2 replay.
The passing run recorded the role-monitor evidence in
`artifacts/integration/ha-chain-20261025T221058Z/fod-role-monitor.log` and the
topology result in the adjacent `topology.txt`. It also completed the full
promotion/reparenting/split-brain checks with 23 replica reads, 111 lag
fallbacks and zero PostgreSQL operation failures. The separate two-node
degraded read-only regression passed with matching SHA-256 and rejected
replica writes; its 16 MiB smoke artifact is
`artifacts/perf/4a4a651/lt7300-role-monitor-regression-primary-replica-64k-pattern-20261025T221145Z`.

FOD cannot make an application reread bytes it has already copied into memory.
"Read again after recovery" therefore means rejecting old handles and requiring
a fresh open/read (or restarting the application/remounting), not undoing earlier
reads. Existing mmap mappings need explicit handling; if they cannot be revoked
reliably, automatic live recovery must remain unsupported for those mounts.

## Alternatives to compare

| Approach | Benefit | Cost or limitation |
| --- | --- | --- |
| Explicit degraded view, consistent read requests, invalidatable handles | Keeps WAL replay running; no primary writes required for reads | Weaker open/unlink and whole-file stability; requires new read and transition gates |
| Long-lived read-only transaction snapshot | Can retain a logical view across requests on the same pinned connection | Connection/pool redesign, recovery conflicts and cancellation; not a durable snapshot across reconnects |
| Operator-managed paused replay or isolated snapshot clone | Can present an unchanging view while the snapshot remains valid | Affects recovery/replica availability, storage and WAL management; promotion/restart invalidates assumptions |
| New verified primary supplied by deployment HA | Retains the normal authoritative lease contract | Requires external promotion/fencing and an available writable authority |

Do not pause replay automatically from FOD. Pausing is a PostgreSQL operational
action affecting all consumers of that standby. Feedback against vacuum is not
a replacement for application leases and does not prevent logical file deletion.

## Required deterministic experiments before implementation approval

| Scenario | Required observation |
| --- | --- |
| Primary process stopped, standby available | New degraded reads succeed without any session, lease, atime or telemetry DML |
| Only the reader's primary connection partitioned | Other hosts continue normally; degraded reader never assumes primary is globally down |
| Primary unlinks/reclaims after degraded open | Old handle fails with `ESTALE`; no fabricated zeros or switch to a recreated pathname |
| Replay advances between metadata and block lookup | Combined read snapshot prevents missing-object/sparse-hole confusion |
| Concurrent truncate, sparse hole creation and data-object replacement | Existence, size, object and bytes agree within each read request |
| Recovery during a delayed read/cache fill | Old completion cannot publish bytes or cache entries after the transition barrier |
| Cached kernel read, directory lookup and mmap across recovery | No successful reuse of invalidated data; unsupported mappings force the documented remount path |
| Dirty writable handles or uncertain commit at outage | No silent discard, replay to standby, or false success; unresolved writes remain distinguishable |
| Expired/replicated session or open-lease rows | Rows never grant degraded ownership or block authoritative reclamation |
| Returned primary on wrong cluster or divergent timeline | Rejoin is rejected despite endpoint reachability |
| Standby query canceled, connection lost or replica switched | Read fails without cache fallback; snapshot identity is not silently changed |
| Primary flaps or mount crashes/restarts | Old epochs/handles are not revived; startup validates identity again |

### Three-node promotion and reparenting test

The automated test is `tests/integration/test_pg_three_node_failover.sh`, using
`docker-compose.ha-chain.yml`. Initially both slaves follow master. It writes
a marker through FOD, records its flush LSN/system identifier/timeline, and
waits for both slaves to replay it. It stops the isolated master container
(the test's simulated fence), promotes slave1, and asserts slave1 is writable,
retains the system identifier, and advances to a descendant timeline.
It reconfigures slave2 to follow slave1 and checks that slave2 remains in
recovery and reconnects to slave1.

Refresh FOD endpoints and perform a controlled unmount/remount. Automatic
source switching is not implemented, so an existing degraded mount must not
be expected to discover slave1. The test remounts a FOD client on the test host
with slave1 as writable authority and slave2 as replica, creates a second
marker, waits for its replay on slave2, and verifies its SHA-256 through FOD.
It records promotion/replay LSNs, roles, upstream, system identifier,
timelines, and content hash. The 2026-10-25 run passed and recorded marker hash
`02802d348f72e5e10f49b8ffe5c72d58352af93982ed369ed962476ea26d711d` in
`artifacts/integration/ha-chain-20261025T092127Z/topology.txt`. The test pauses
slave2 replay, checks that FOD returns a new marker through primary fallback
while the replica is behind, resumes replay, and checks the same marker
through slave2. That run recorded 23 replica reads, 117 lag fallbacks, and zero
PostgreSQL operation failures. It also restarted the old master to simulate
two writable endpoints and verified that FOD refused to mount with a clear
split-brain error. Routing counters and the rejection log are stored beside
the topology artifact.

This local Docker test stops the old master container as its simulated fence;
the split-brain case verifies FOD rejects two reachable writable endpoints,
but does not prove external HA-controller fencing under network partitions.
The FOD client also runs on the test host rather than inside the slave2
container. Network partition testing, real fencing, automatic endpoint
reconfiguration, old-master rejoin as a standby, and live source switching
remain untested requirements. Do not infer production HA safety or automatic
recovery from this test alone.

These remain required experiments except for the narrower initial acceptance
run documented below. Keep TX03J cross-mount coherence as an independent
release requirement. Benchmark the new read path only after the selected
contract passes deterministic correctness experiments.

## Initial implementation status

The opt-in first phase is implemented as a dedicated mount selected with
`FOD_DEGRADED_READONLY_ENABLED=1`. Startup requires role-aware routing to an
observed physical standby in recovery. Reads use one generation-aware
PostgreSQL statement snapshot, bypass FOD caches and TTLs, and return
`ESTALE` if an already opened file no longer exists. Writes and lock operations
are rejected; degraded mounts do not register or renew client/open leases.
`fod-bootstrap` preserves the opt-in flag when it applies runtime settings.

The Docker acceptance script passed with a 32 MiB patterned file and 64 KiB
fio blocks: primary write/read succeeded, the primary was stopped, replica
read returned the same SHA-256, and a replica write was rejected. This does
not establish safety for unlink/reclaim races, SQL cancellation, recovery
transitions, fencing, or identity mismatches. The mount does not probe for a
returning primary or switch sources. Use controlled unmount/remount after
recovery; automatic rejoin remains unimplemented and must not be inferred from
this acceptance result.

## Decisions and deployment prerequisite

The selected design is explicit opt-in, a dedicated read-only mount as the
first implementation, one consistent PostgreSQL statement per FUSE read, no
lease-based protection or local replacement locks, `ESTALE` for old degraded
handles after recovery, cache invalidation and fresh authoritative leases
before new opens, and fail-closed behavior when fencing evidence is absent. It
does not promise stable contents across a multi-request copy or immediate
permission revocation while primary is unreachable. Existing mmap mappings
must be rejected or proven invalidatable; otherwise require unmount/remount.

The deployment must identify how FOD proves that a returning primary is the
unique fenced authority, including promotion to a new timeline. If the
deployment cannot supply that proof, the mount can remain read-only or require
operator-controlled unmount/remount; it must not accept writes based only on a
successful connection.

The next work is deterministic testing of the remaining selected semantics:
unlink/reclaim and generation races, SQL cancellation, replica DML absence,
kernel cache/mmap behavior, and fencing/identity requirements. Then design and
implement the rejoin barrier as a separate change. Any architecture-invariant
exception must stay narrowly scoped to degraded reads and must not make local
state authoritative for lease, namespace, or mutation decisions.

## Follow-up test run (2026-10-25)

The strict TX03J cross-mount coherence scenario passed against an isolated
`fod_test` database with PostgreSQL snapshot checks and a held reader. It passed
both with the default cache policy and with FUSE direct I/O enabled, FOD read
cache disabled, and metadata TTL set to zero. In both cases the held reader
and mounts A, B, and C returned the current committed content; PostgreSQL
mapping checks confirmed the same file ID with a new data object and changed
block hash after the write. The temporary database Compose project and volume
were removed after the tests.

A one-repetition 64 KiB cache smoke comparison also passed on 2026-10-25. Both
cache-off and 4,096-block cache-on reads produced matching primary/replica
SHA-256 values and zero PostgreSQL operation failures. Throughput was 74.1/78.0
MiB/s (primary/replica) cache-off and 46.8/60.6 MiB/s cache-on. This small
single-run sample is only a wiring/correctness check; it is not a performance
conclusion or a replacement for the five-run matrix and historical comparison.
Artifacts are in
`artifacts/perf/4a4a651/lt7300-read-cache-matrix-20261025T093113Z/summary.tsv`.

## PostgreSQL references

- [Hot Standby](https://www.postgresql.org/docs/17/hot-standby.html): read-only
  queries, statement/transaction snapshots, and recovery conflicts that may
  cancel queries or disconnect sessions.
- [Recovery Control Functions](https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-RECOVERY-CONTROL):
  replay pause/resume, pause-state validation and operational constraints.
  Implementation must check the deployed PostgreSQL version before using
  version-specific functions; these links do not raise FOD's minimum version.
