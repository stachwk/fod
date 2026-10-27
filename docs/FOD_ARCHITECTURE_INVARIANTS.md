# FOD architecture invariants

This document records architectural rules that must remain true as FOD evolves.

## Guiding principle: PostgreSQL is the shared source of truth

FOD is a multi-host filesystem. Multiple FUSE processes can operate on the same
filesystem state from different machines at the same time. No individual FUSE
process can therefore decide shared correctness from process-local state alone.

PostgreSQL is the single logical authority for all state and ordering that can
affect another host.

The rule is:

> A filesystem decision that can conflict across hosts must be decided by the
> PostgreSQL authority, not by local process memory.

This includes at least:

- namespace visibility and mutation: create, unlink, link, rename and replace;
- open-file lifecycle state that affects deletion or reclamation;
- write ownership, destination ownership, leases and fencing tokens;
- conflict detection and operation ordering;
- inode-generation decisions when multiple generations can coexist;
- deferred deletion and orphan reclamation;
- any safety decision whose result must be identical on every mount.

## Local state is an optimization, not authority

FUSE-side maps, mutexes, lookup-reference counters, file-handle tables, caches,
buffers and telemetry remain useful. They can avoid unnecessary database work
and protect memory owned by one process.

They must not be required to make a cross-host operation correct.

A restart, crash, second mount or operation from another host must not bypass a
safety property merely because that host does not share another process's
memory.

## Transaction and lock rule

Operations that participate in the same correctness boundary must use a common
PostgreSQL serialization mechanism. Depending on the operation this can be a
transaction, uniqueness/foreign-key constraint, row lock, lease/fencing record
or PostgreSQL advisory lock.

Check-then-act sequences that can race across hosts are invalid unless the
check and mutation are protected by the same database serialization boundary.

Examples:

```text
open("/x")  <-> unlink("/x")
create("/x") <-> create("/x")
create("/x") <-> recreate while an older /x is open-unlinked
rename("/a", "/b") <-> create/unlink/rename of /a or /b
writer ownership <-> another writer on another host
```

## Clock requirements for leases and failover

PostgreSQL remains the authority for lease creation and expiry checks; a FOD
client's local clock must not decide whether a shared lease is valid. The
PostgreSQL nodes that can become primary still need synchronized system clocks:
after failover, expiry decisions use the new primary's clock, so clock skew can
make leases expire early or late. FOD hosts also need synchronized clocks for
timestamps and operations carrying an explicit client time.

Keep NTP synchronization active and monitored on FOD and PostgreSQL hosts. The
repository currently has no enforced numerical maximum clock offset; establish
one only after validating the shortest supported lease and failover behavior.
See [PostgreSQL and host time requirements](POSTGRESQL_REQUIREMENTS.md#3-synchronizacja-czasu-hostów).

## PostgreSQL HA topology

The requirement is a single **logical** write authority, not necessarily one
physical PostgreSQL server forever.

A primary/standby cluster with failover satisfies the model when committed
filesystem mutations retain one authoritative order after failover.

A multi-primary PostgreSQL architecture is acceptable only if it provides a
single globally authoritative order for conflicting FOD transactions. A design
that accepts conflicting writes independently and reconciles them later does
not satisfy filesystem semantics.

Read replicas may be used only where replica lag cannot affect correctness.
Operations participating in mutation ordering, ownership, leases, open/unlink
state or conflict decisions must observe authoritative write state.

## Review rule

When adding or changing a filesystem operation, ask:

1. Can two hosts execute conflicting versions of this operation?
2. What PostgreSQL object or lock serializes them?
3. Can a local cache be stale without changing the correct result?
4. Does crash/failover leave enough authoritative state for another host to
   continue safely?
5. If an inode/object remains alive after namespace removal, can a recreated
   namespace entry be distinguished globally from the older generation?

If any answer depends on process-local memory for cross-host correctness, the
design is incomplete.

## Current open-unlink application

Schema v25 applies this principle to open-unlink handling:

- `files.unlinked` is authoritative namespace state;
- `file_open_leases` records cross-host open-handle liveness;
- PostgreSQL advisory locks serialize open/unlink/purge boundaries;
- deferred reclamation waits for authoritative lease state;
- recreated files must use a distinct inode generation while an older
  open-unlinked generation remains alive;
- read-only mounts also require access to a writable PostgreSQL authority for
  `client_sessions` and `file_open_leases`; a replica-only mount must fail
  closed rather than operate without centrally visible open-handle state.

The FUSE process can cache and mirror this state, but PostgreSQL remains the
authority.

## Cross-mount cache coherence is mandatory (TX03J)

PostgreSQL is also the authority for **the freshness of shared content and
metadata**, not just locks, namespace changes and write ownership.

After a write has passed its documented commit/durability boundary, clients
opening and reading the file on other mounts must not indefinitely serve an
older version. Correctness must not depend on `FOD_FOPEN_DIRECT_IO` or a
particular local cache configuration. This applies to all clients, including
processes on different machines.

- Keep separate, monotonic generations for content and namespace/metadata
  changes; do not confuse a stable `file_id` with a content generation.
- Increment the relevant generation transactionally with the authoritative
  PostgreSQL mutation, including replacement of a `data_object_id`.
- Use commit-ordered notifications only as a low-latency *hint*; a listener
  may disconnect or miss notifications. A durable generation check or
  equivalent reconciliation mechanism must make missed events harmless.
- Invalidate all affected process-local snapshots, block caches and FUSE
  kernel cache entries (inode data, inode metadata and directory entry where
  relevant). Kernel invalidation must work when `direct_io=0`.
- Do not treat lease/fencing validity as proof that a reader's cached data
  is current. Lease/fencing and cache coherency are independent contracts.
- Fail closed on a coherence-validation error in strict mode: stale cached
  content may not silently become a successful read.
- Document exact synchronization semantics for already-open handles, new
  opens, concurrent writes and reads racing a commit. The intended strong
  contract (read after completed write) needs ordered validation on the read
  path; asynchronous notifications alone are insufficient.
- Preserve PostgreSQL primary authority in any HA topology; replica lag
  cannot make a stale generation appear authoritative.

### Known unresolved defect

On 2026-10-16, TX03J diagnostics reproduced stale reads in an existing mount
after another mount's `fsync`, despite `direct_io=1`, read cache disabled,
metadata TTL set to zero, and a two-second wait. The new writer and a fresh
mount saw the correct data. TX03J remains OPEN until the production read path
is corrected, tests enforce coherency, and performance/regression checks pass.

### Known mutation-authority violation

The 2026-10-25 primary authority audit reproduced loss of a surviving hardlink:
a writable mount used a lagged replica's link count to select the primary-file
unlink/purge path. Mutation SQL ran on the primary, but its deciding read was
not authoritative. The primary-only control preserved the alias. This remains
an open release blocker for replica read routing on writable mounts; keep
`FOD_PG_REPLICA_READ_ROUTING_ENABLED=0` until the unlink transaction and related
mutation preconditions are corrected and validated. See
`docs/FOD_PRIMARY_AUTHORITY_AUDIT.md` for the mechanism, evidence and remaining
review items.
