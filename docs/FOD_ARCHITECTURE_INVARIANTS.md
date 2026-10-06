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
