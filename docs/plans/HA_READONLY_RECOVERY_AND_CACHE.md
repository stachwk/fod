# HA Read-Only Recovery and Cache Performance Plan

Status (2026-10-26): the opt-in replica-only read path, mount gate, endpoint
role monitoring and three-node promotion/reparenting acceptance test are
implemented. The degraded read rejects overflowing request ranges before
querying PostgreSQL. In-process operator transition and source switching remain
unimplemented. The initial five-run cache pilot is complete; controlled
multi-host performance coverage remains open. The TX03J cross-mount freshness
defect remains an independent release blocker.

This is the detailed HA/cache workstream of the
[current implementation queue](CURRENT.md). The
[contract review](../FOD_DEGRADED_READONLY_CONTRACT_REVIEW.md) explains its
semantics; [CURRENT_STATE.md](../CURRENT_STATE.md) describes implemented behavior.

## Purpose

This plan covers two related follow-ups:

1. Keep FOD data readable from a PostgreSQL standby when the writable primary
   cannot be reached, then restore normal operation safely when authoritative
   PostgreSQL service returns.
2. Measure the effect of FOD and FUSE caches on primary and replica throughput,
   and compare repeatable results with historical FOD measurements.

The first goal is availability of a read-only view. It does not permit writes,
metadata updates, lease mutations, or independent ordering decisions on a
replica. PostgreSQL must remain the single logical authority for filesystem
correctness, as required by FOD_ARCHITECTURE_INVARIANTS.md.

## Current evidence and constraints

- A primary write followed by WAL replay was visible on the physical replica.
  A separate 256 MiB run measured 43.51 MiB/s primary write, 82.74 MiB/s
  primary read, and 96.06 MiB/s replica read with FOD caches disabled and
  direct I/O enabled. SHA-256 matched, and a replica write was rejected.
- Historical baseline: the first isolated HA diagnostic stopped the primary,
  after which a replica-only mount failed because the then-current startup path
  still registered sessions and open-file leases through writable authority.
  The later opt-in degraded mount below removes that lease dependency under an
  explicitly weaker read-only contract.
- Existing cross-mount diagnostics reproduced stale reads after another mount
  completed fsync. Strict reads passed the diagnostic; disabling the userspace
  read cache, setting metadata TTL to zero, or enabling FUSE direct I/O alone
  did not. The TX03J cross-mount freshness defect remains a separate release
  blocker and must not be hidden by a new HA fallback mode.
- The recent throughput run was a single local-machine sample with the host on
  battery and the CPU governor set to powersave. Treat it as diagnostic data,
  not as a hardware performance baseline.
- Normal read-only mounts still require primary-backed lease authority.
  Explicit degraded mode is a separate weaker contract: it disables leases
  and mutations, reads each request from one standby statement snapshot, and
  may return `ESTALE` after remote unlink/reclaim. It must not claim normal
  multi-host open/unlink semantics.

## Workstream A: degraded read-only access

The conditional-lease proposal remains under design review. See
[Degraded read-only contract review](../FOD_DEGRADED_READONLY_CONTRACT_REVIEW.md)
for source evidence, a candidate weaker read contract, return-to-primary
barriers, alternatives and deterministic experiments. Missing writable lease
authority blocks the current normal-mount contract; it does not by itself
prove that an explicitly degraded view is impossible. A recovery-time reread
alone does not protect reads during an outage.

The current implementation is enabled with
`FOD_DEGRADED_READONLY_ENABLED=1` and requires role-aware endpoint routing,
`FOD_ROLE=replica`, and a validated physical replica endpoint. It checks that
startup is in PostgreSQL recovery, disables leases and locking, bypasses read
caches, and uses the generation-aware single-statement read. Background role
monitoring observes configured endpoints and reports role changes. It does not
switch sources; recovery still requires an operator-controlled remount.

### Proposed behavior

Use an explicit mount state machine rather than changing the mount's read-only
flag opportunistically:

| State | PostgreSQL condition | Allowed FOD behavior |
| --- | --- | --- |
| Primary mode | A validated writable primary is reachable | Existing read/write operation and lease behavior |
| Degraded replica mode | Primary is unavailable; a configured physical replica is reachable and in recovery | Reads only from the replica's replayed snapshot; reject every mutation |
| Rejoining | A writable authority returns, but identity and replay/commit position are not yet validated | Keep the mount read-only; probe and validate without admitting writes |
| Primary mode restored | The returning endpoint is validated as the unique writable authority and rejoin checks pass | Resume normal behavior through a controlled transition |
| Unavailable | No eligible PostgreSQL endpoint or identity is uncertain | Return I/O errors; do not serve unvalidated cached data as current |

The implemented mode is an explicit, operator-visible read-only mount with
background endpoint role monitoring and no live rejoin. A connection error is local to this
mount and does not prove that primary is unavailable to all hosts. FOD must
not promote a standby. Returning to writable operation requires the
deployment's unique-primary fencing evidence plus the recovery gate in
`FOD_DEGRADED_READONLY_CONTRACT_REVIEW.md`; until live rejoin is implemented,
remain read-only or use operator-controlled remount.

### Contract to implement and validate

1. **Lease and open-file semantics:** The opt-in degraded view does not create
   or renew client_sessions/file_open_leases. It does not claim open/unlink
   retention. If another host unlinks and reclaims a file, the degraded handle
   becomes ESTALE. This exception applies only to the explicitly selected
   read-only contract; process-local state never replaces shared authority.
2. **Stale snapshot contract:** Degraded reads expose the replica state visible
   to each PostgreSQL statement and may lag acknowledged writes. State and
   replay position must be visible to operators; never present this view as
   strongly current. One FUSE read is internally consistent, while a multi-read
   application operation can observe different committed versions.
3. **Failover identity and fencing:** Validate cluster identity, the writable
   role, timeline ancestry when relevant, and deployment fencing evidence
   before returning to writable primary mode. A successful connection or role
   check alone does not reject split brain. The deployment must provide the
   fencing evidence; without it, FOD remains read-only or requires an operator
   controlled remount.
4. **Rejoin ordering:** Enter a read-blocked rejoining state; drain old I/O,
   validate the unique fenced authority and timeline, advance source epoch,
   invalidate FOD/kernel caches, and make old degraded handles return ESTALE.
   New opens require authoritative leases and read from primary until normal
   replica replay barriers are met.
5. **Live transition mechanics:** Start with a separately mounted read-only
   view. Do not live-convert a writable mount in the first implementation.
   Rejoining can switch that view only after proving the I/O barrier and cache
   invalidation; otherwise require controlled unmount/remount. Reject or
   explicitly invalidate mmap mappings.
6. **Locking:** Reject lock operations that need shared state while degraded;
   do not substitute process-local locks. Return to primary-backed locking only
   after the recovery barrier and new authoritative session are established.
7. **Read freshness:** Use the generated single-statement read for every
   degraded FUSE request and bypass local caches in the first implementation.
   On recovery, block I/O, drain work, advance the source epoch and invalidate
   kernel and FOD caches before serving from primary. TX03J remains an
   independent release requirement for ordinary mounts.
8. **Dynamic role monitoring and transition:** A running degraded mount now
   probes configured endpoint roles and reports role changes. Detection keeps
   it read-only; controlled unmount/remount is still required. The planned
   in-process operator transition must require trusted HA fencing,
   cluster/filesystem identity and timeline checks. It must quiesce and drain
   I/O, reject stale handles, invalidate old-source caches and re-establish
   leases before new normal operations. If proof or invalidation is
   unavailable, fail closed. Detailed transitions and acceptance cases are in
   `FOD_DEGRADED_READONLY_CONTRACT_REVIEW.md`.

### Implementation phases

1. **Read-path correctness (implemented; unit tests pass):** Wire the existing
   `fetch_block_range_with_generation_shared` API into degraded FUSE reads.
   Return `ESTALE` when an established handle's file disappears; only return
   sparse zeros when file identity, size, generation, data object and blocks
   come from the same statement snapshot. Fail closed on cancellation or SQL
   errors. Bypass caches for this first mode.
2. **Explicit mount gate (implemented for the first phase):** Add opt-in
   configuration for a dedicated degraded read-only mount. Require
   role-aware endpoint routing and an observed physical standby in recovery.
   Disable mutations, atime DML, session/open-lease writes, lock operations,
   FOD read caches, and metadata/statfs TTLs. The gate does not validate a
   trusted cluster/filesystem identity or deployment fencing evidence; those
   remain prerequisites before automatic rejoin or write admission.
3. **State and probes:** Add states `degraded`, `rejoining`, `primary_read_only`,
   `unavailable` and visible counters/status. Probe primary reachability,
   endpoint role, cluster identity, timeline/fencing evidence, replay LSN and
   transition reasons. A local connection failure alone must not imply global
   primary failure.
4. **Rejoin barrier:** Stop new reads; drain requests and asynchronous cache
   fills; validate the unique fenced authority; advance source epoch; invalidate
   FOD and kernel data/metadata/namespace caches; mark old handles `ESTALE`;
   allow new opens only after primary-backed leases work. Keep reads read-only
   until all checks pass. Require controlled remount if cache/mmap invalidation
   cannot be proven.
5. **Deterministic fault tests:** Implement the scenarios in the contract
   review, including unlink/reclaim, generation changes, query cancellation,
   dirty/uncertain writes, recovery races, identity mismatch, split brain and
   primary flapping. Prove no replica DML and no successful stale cache reply.
6. **Architecture and release review:** Review the narrow degraded-read
   exception against FOD_ARCHITECTURE_INVARIANTS.md; run TX03J coherence tests
   independently; only then perform HA acceptance and performance runs.

### HA test matrix

- Primary healthy: normal mount, writes, leases, cross-mount visibility.
- Primary unreachable, replica healthy: read existing files; verify create,
  write, truncate, rename, unlink, metadata changes, atime DML, lease writes,
  and unsupported locking are rejected or handled according to the documented
  contract.
- Replica behind: expose only replayed snapshot; report lag; do not imply that
  acknowledged-but-unreplayed data exists.
- Replica unreachable: fail reads that cannot be satisfied under the selected
  freshness contract; never promote or write to it.
- Primary returns without promotion: validate identity and transition
  read-only to normal mode only after the rejoin barrier.
- Standby promoted by external HA: validate cluster identity, timeline/fencing,
  and unique writable role before allowing writes.
- Three-node promotion/reparenting sequence: old master fails, slave1 is
  promoted and fenced as the sole master, slave2 is reconfigured to follow
  slave1, and FOD clients on slave2 resume using slave1 as PostgreSQL authority.
- Split brain or conflicting writable endpoints: fail closed; no write routing.
- Mount restart, FOD process crash, network partition, and repeated primary
  flapping in each state.
- Open handles across each transition, including unlink/recreate attempts from
  another host, to confirm the documented lease and inode-generation rules.
- Assert no SQLSTATE 25006, no writes to a physical replica, no unreported
  cache-generation regression, and no mutation accepted in degraded mode.

The Docker primary-write/replica-read script now has a `degraded` mode. A
32 MiB run with 64 KiB fio blocks completed primary write/read, stopped the
primary, mounted the physical standby in degraded mode, verified matching
SHA-256, and confirmed that a write is rejected. It also confirmed zero
replica operation failures. This validates the initial read-only phase only;
it does not exercise unlink/reclaim races, query cancellation, cluster
identity/fencing, or return to primary. The mount remains on the standby and
requires operator-controlled unmount/remount when primary service returns.

### Required three-node HA scenario: master → slave1 → slave2

This scenario is implemented by
`tests/integration/test_pg_three_node_failover.sh` and
`docker-compose.ha-chain.yml`. It tests PostgreSQL promotion/reparenting and
FOD authority/routing separately; FOD must not promote a standby or infer
that a failed connection proves the old master is fenced.

Initial topology:

| Node | Initial PostgreSQL role | Initial upstream |
| --- | --- | --- |
| master | writable primary | — |
| slave1 | physical standby | master |
| slave2 | physical standby | master |

Required sequence and assertions:

1. Write a uniquely identifiable marker file through FOD on the original
   master. Record its content hash, the primary flush LSN, PostgreSQL system
   identifier and timeline. Wait until both standbys have replayed at least
   that LSN; verify the marker hash through each.
2. Fail the original master and have the external HA controller **fence** it
   from writes before promotion. A network timeout or stopped client process
   is not fencing evidence. Record the failed node's last known WAL position.
3. Promote slave1 through the HA controller. Verify it is writable, has the
   expected system identifier and a descendant timeline, and is the only
   writable endpoint. FOD must reject promotion if these conditions are not
   established by trusted deployment evidence.
4. Reconfigure slave2 to follow slave1. Verify `pg_is_in_recovery()` is true
   on slave2, its upstream is slave1, and replay has advanced beyond the
   promotion marker/required LSN. Confirm slave2 remains read-only and accepts
   no FOD lease, namespace, or payload writes.
5. Refresh FOD's configured primary/replica endpoints and perform the supported
   controlled rejoin of the slave2 host's mount. Until live source switching
   and fencing validation are implemented, this means unmount/remount; do not
   expect an existing degraded mount to switch automatically. Confirm
   authoritative sessions/open leases are registered on slave1, then create
   and read a second marker through FOD on the slave2 host. The write must be
   ordered by slave1; the read must come from the configured safe source and
   return the expected hash after the replay barrier.
6. Verify no FOD mutation was sent to slave2, no lease was accepted from the
   old master, and the original master cannot rejoin as writable. Rebuild or
   reconfigure it as a standby following slave1, wait for replay, and repeat
   the hash and role checks. A divergent timeline or unfenced old primary must
   make the test fail closed.

Capture every role transition, fencing action, system identifier, timeline,
upstream endpoint, promotion/replay LSN, FOD endpoint selection, lease
authority, operation errors, and file hashes in the test artifact. The
acceptance result is failure if two writable primaries are reachable, slave2
routes authoritative lease/mutation operations to itself, any write is
accepted by a physical standby, or a client resumes on an unvalidated source.
The test passed on 2026-10-25 using PostgreSQL 16 Docker containers. It
verified both initial standby roles, replay of the initial marker on both
slaves, old-master container stop, promotion of slave1 to timeline 2, unchanged
system identifier, slave2 reparenting to slave1, FOD writes through slave1,
replica replay pause/resume, correct reads during lag through the primary
fallback, 23 replica reads, 117 lag fallbacks and zero PostgreSQL operation
failures, replay on slave2, and matching post-promotion marker SHA-256
(`02802d348f72e5e10f49b8ffe5c72d58352af93982ed369ed962476ea26d711d`). The
test then restarted the old master to create two simultaneously writable
endpoints and confirmed that FOD rejected the mount. The latest run artifact
is `artifacts/integration/ha-chain-20261025T092127Z/topology.txt`; mount
routing and lane counters are in `fod-reparented-mount.log` in the same
directory, and the split-brain rejection is in
`fod-split-brain-rejection.log`.

This is a local functional test, not proof of production fencing: stopping the
isolated master container is the simulated fence, and the FOD client runs on
the test host with slave1/slave2 endpoints rather than inside the slave2
container. It does not test network partitions, real HA-controller fencing,
automatic reconfiguration of deployed FOD mounts, or live source switching.
Those remain release requirements before claiming automatic recovery support.
The test now also keeps a degraded mount on slave1 running through promotion.
It verifies role-change detection, read-only state retention and write
rejection in the same process. It then unmounts and starts a new FOD mount for
the post-promotion write/replay checks. The still-planned in-process operator
transition and HA fencing evidence interface are not validated by this test.

## Workstream B: cache and historical performance comparison

### Benchmark objective

Measure separately the effect of FOD userspace read cache, FOD read-ahead,
FUSE/kernel page cache, PostgreSQL shared buffers, and replica replay. A single
"cache on/off" label is insufficient because these layers have different
lifetimes and cold/warm behavior.

### Controlled matrix

Run each case on primary and physical replica where the operation is
supported:

| Dimension | Cases |
| --- | --- |
| FOD cache | disabled; configured production/default cache; optional tuned size |
| FOD read-ahead | disabled; production/default; tuned value as a separate case |
| FUSE direct I/O | disabled for normal kernel-cache behavior; enabled as a diagnostic control |
| Access pattern | sequential read; random read; sequential write; small-file metadata/open-read |
| fio block size | 4 KiB, 64 KiB, 256 KiB, 512 KiB, 1 MiB |
| Dataset size | at least 2x PostgreSQL shared_buffers for a cold-ish DB-buffer case, plus a representative working set |
| Warmth | first pass; repeated warm pass; restart FOD process; restart PostgreSQL container where safe |
| Repetitions | minimum five measured runs per case, with an unmeasured warm-up where appropriate |

Do not drop the host's global kernel page cache in routine runs. Label runs
precisely as cold-ish or warm and state which caches were reset. Use the same
payload, dataset size, storage block size, fio options, durability settings,
host, PostgreSQL settings, and binary revision for each A/B pair. Randomize or
alternate case order to reduce thermal and battery drift.

### Data to record

- Throughput in MiB/s, IOPS, and mean/p50/p95/p99 latency from fio JSON.
- FOD callback counts, read-cache hits/misses/evictions, readahead activity,
  bytes fetched per callback, and PostgreSQL operation/connection counts.
- For replica reads: primary flush LSN, replica replay LSN and lag at start and
  end, and whether primary fallback was possible.
- Exact FOD cache and direct-I/O settings; FUSE/kernel cache policy; dataset
  and block sizes; binary Git revision; Rust profile; kernel and PostgreSQL
  versions/configuration.
- Host model/storage, AC/battery state, CPU governor/frequency policy,
  temperature/throttling indicators where available, and run timestamps.
- Hash or byte-for-byte verification for every read result; explicit rejection
  check for attempted writes to a physical replica.

### Historical comparison method

1. Inventory historical result TSV/JSON files under artifacts/perf and the
   committed performance-baselines.md tables. Record the exact revision,
   hardware, block size, cache policy, dataset, and test method for each row.
2. Compare only like-for-like cases. Do not compare a warm cached run with an
   uncached run or combine measurements from different machines into one
   regression percentage.
3. For each case report median and spread across repetitions, absolute delta,
   and percentage delta from the closest comparable historical result. Mark
   non-comparable data explicitly.
4. Establish a new baseline only after stable AC-powered runs on the intended
   reference host. Set regression thresholds after measuring run-to-run noise;
   do not invent a threshold from a single observation.
5. Preserve raw fio JSON, environment/configuration snapshots, and a concise
   result TSV/Markdown summary in a revision- and host-specific artifact
   directory. Update performance-baselines.md only after review.

### Performance acceptance criteria

- All read cases return bytes matching the written payload.
- Replica read cases report replay position and do not silently fall back when
  the scenario claims replica-only measurement.
- No replica DML, SQLSTATE 25006, or unexpected database operation failures.
- Cached and uncached results are both reported; the cache benefit or cost is
  reproducible across at least five runs and larger than observed run noise.
- Results include callback and database-work counts so throughput changes can
  be attributed to FOD behavior rather than host page cache alone.
- No cross-mount freshness regression; TX03J correctness tests pass before a
  cache setting is recommended for production.

### Initial cache-matrix pilot (2026-10-24)

The five-run pilot is recorded in
`artifacts/perf/4a4a651/lt7300-read-cache-matrix-20261023T220443Z/summary.tsv`.
It covers 64 KiB and 512 KiB sequential reads, five repetitions per cache
state, a 128 MiB pattern file, and three passes over the file in each measured
read. FOD userspace cache was either disabled or set to 4,096 storage blocks
(128 MiB at the measured 32 KiB storage block size). Read-ahead and metadata
TTL were disabled, FUSE direct I/O was enabled, and the primary remained online
as lease/namespace authority while FOD routed payload reads to the physical
replica. Each case restarted the replica PostgreSQL container before reading;
the host kernel page cache was not dropped.

| fio block | FOD cache | Primary median | Replica median | Replica range |
| --- | --- | ---: | ---: | ---: |
| 64 KiB | off | 112.5 MiB/s | 121.6 MiB/s | 59.5–141.4 MiB/s |
| 64 KiB | 4,096 blocks | 123.4 MiB/s | 110.3 MiB/s | 54.7–169.6 MiB/s |
| 512 KiB | off | 254.3 MiB/s | 234.7 MiB/s | 214.2–300.9 MiB/s |
| 512 KiB | 4,096 blocks | 401.7 MiB/s | 429.5 MiB/s | 223.1–474.7 MiB/s |

These five-run medians remain pilot data, not release baselines. At 64 KiB the
cached replica median was about 9% lower than uncached and the ranges overlap
widely, so this case is inconclusive. At 512 KiB the cached replica median was
about 83% higher; however, the cached primary median includes one pronounced
low outlier (130.5 MiB/s), and the replica range still overlaps the uncached
range. Do not treat this as a stable regression threshold or production
recommendation. Every measured primary and replica read passed SHA-256
validation, the replica read counter was nonzero, and no PostgreSQL operation
failures were observed. The routed runs had a small number of primary
fallbacks under pool pressure; the summary records those counts.

FOD boundary counters show a much clearer cache effect on block fetching than
the total-throughput samples: median read_block_map time fell from 10.89 s to
2.05 s at 64 KiB and from 9.30 s to 0.60 s at 512 KiB. At the same time,
replica-read operation counts stayed close to FUSE read callback counts. These
isolation runs set metadata TTL to zero; the primary-role mount therefore
refreshes per-handle metadata before checking cached content on each callback.
The block cache avoids most block-range fetch work, while metadata validation
continues to contribute database work. The production-policy check below uses
a one-second TTL and produced only 33–36 replica reads for the whole file.
Preserve this distinction in future performance analysis; it does not justify
weakening TX03J freshness checks.

#### Historical comparison

The closest historical records are the single-run 2026-09-07 results at
`artifacts/perf/8b656f6/lt7300-docker-matrix-64k-pattern-primary-replica-64k-pattern-20260907T230654Z/result.tsv`
and
`artifacts/perf/8b656f6/lt7300-docker-matrix-512k-pattern-primary-replica-512k-pattern-20260907T230718Z/result.tsv`.
They used a 128 MiB pattern file, the same 32 KiB storage block size and
production read policy (4,096 cache blocks, read-ahead 4, sequential read-ahead
8, direct-I/O prefetch 512, small-file threshold 8, metadata TTL 1 second,
FUSE direct I/O enabled). Historical replica throughput was 292.2 MiB/s at
64 KiB and 314.5 MiB/s at 512 KiB; primary throughput was 303.3 and
323.2 MiB/s, respectively.

The table compares the current five-run cache matrix medians with those
historical single samples. Percentage deltas are calculated against the
historical throughput:

| fio block | Current cache | Primary median | Primary delta | Replica median | Replica delta |
| --- | --- | ---: | ---: | ---: | ---: |
| 64 KiB | off | 112.5 MiB/s | -62.9% | 121.6 MiB/s | -58.4% |
| 64 KiB | 4,096 blocks | 124.3 MiB/s | -59.0% | 110.3 MiB/s | -62.2% |
| 512 KiB | off | 254.3 MiB/s | -21.3% | 234.7 MiB/s | -25.4% |
| 512 KiB | 4,096 blocks | 401.7 MiB/s | +24.3% | 429.5 MiB/s | +36.6% |

This is not a like-for-like regression measurement. The current matrix disabled
read-ahead and set metadata TTL to zero to isolate the userspace cache, used
three measured read loops, and left the primary online as lease authority while
routing payload reads to the physical replica. Historical runs used the
production cache policy, one read loop, and stopped the primary before the
replica read. Current 64 KiB and 512 KiB measurements also have substantial
run-to-run spread, while history has only one sample. Treat the deltas as
diagnostic context, not as evidence of a code regression or improvement.

For a closer policy match, a separate one-run current check used the same file
size, payload, block sizes, FOD cache/read-ahead settings and direct-I/O setting
as the historical records. It measured replica reads at 272.3 MiB/s at 64 KiB
and 306.2 MiB/s at 512 KiB: -6.8% and -2.6% against the historical samples.
Primary reads measured 268.9 and 280.7 MiB/s. The current check still routed
reads through the replica while leaving the primary online, and its CPU energy
preference was `balance_performance` versus historical `performance`; it is
also a single sample. These differences preclude a firm regression conclusion.

The first 4 KiB pilot was stopped after the callback-heavy path took more than
two minutes for one measured read phase. Its partial records remain in
`artifacts/perf/4a4a651/lt7300-read-cache-matrix-20261023T215527Z/summary.tsv`.
The matrix runner defaults to 64 KiB and 512 KiB; 4 KiB remains available as
an explicit stress case with `FIO_BLOCK_SIZES=4k`.

## Suggested execution order

1. Close or explicitly track the TX03J stale-read defect and keep its release
   gate independent of degraded mode.
2. Review and approve the degraded read-only contract and any proposed narrow
   exception to FOD_ARCHITECTURE_INVARIANTS.md.
3. Specify and implement the HA-controller fencing-proof interface plus
   cluster/filesystem identity and timeline validation. PostgreSQL role probes
   alone must never grant write authority.
4. Implement the in-process operator transition with request draining, source
   epoch fencing, cache/kernel invalidation, old-handle `ESTALE`, and fresh
   primary-backed leases. Keep controlled remount as the fallback.
5. Add deterministic tests for wrong identity/timeline, missing or stale
   fencing proof, two writable endpoints, delayed reads, uncertain writes,
   mmap, cancellation and repeated failover before Docker acceptance.
6. Run the cache matrix on stable AC power, first on the reference local
   primary/replica stack and then on the intended multi-host deployment.
7. Compare only like-for-like results with historical baselines and publish
   environment details and measurements.

## Risks and explicit non-goals

- Degraded reads can be stale relative to acknowledged primary writes that
  have not reached the replica. The mode is not a substitute for synchronous
  replication or a promoted, fenced primary.
- Reusing process-local leases or locks to bypass missing PostgreSQL authority
  would violate FOD's cross-host correctness rules.
- Live read-only-to-read/write transition may be unsafe for existing file
  handles and kernel caches. A controlled remount is acceptable for the first
  safe implementation.
- Performance results from one host do not establish multi-host network
  performance. The reference local test and remote deployment test must be
  reported separately.
- This plan does not authorize writes to a replica, independent multi-primary
  operation, or conflict reconciliation after the fact.
