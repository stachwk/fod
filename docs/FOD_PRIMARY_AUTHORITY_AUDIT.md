# Primary authority audit

Document role: topic review/specification supporting the
[current implementation queue](plans/CURRENT.md). Implemented capabilities are
summarized in [CURRENT_STATE.md](CURRENT_STATE.md); see the
[documentation map](README.md#correctness-reviews-and-evidence) for related reviews.

Date: 2026-10-25. Scope: the current working tree, Rust FUSE runtime and its
PostgreSQL repository. This is a correctness audit; the data-loss defect below
has not been fixed by this audit.

## Result

Mutation SQL and authoritative session/ownership operations use the primary
connection targets in a role-routed writable mount. However, this is not enough
to claim that all mutation decisions are controlled by the primary. Some FUSE
mutation preconditions use replica-routed reads. A reproducible `unlink` case
loses a surviving hardlink when the replica is behind another mount's commit.

## P1: replica-based unlink decision can delete a surviving hardlink

The failure was reproduced with two independent FOD mounts, PostgreSQL 16, the
`release-lto` runtime, direct I/O, metadata TTL zero and FOD read cache disabled.
Both mounts use the same writable primary; only mount B enables
`FOD_PG_REPLICA_READ_ROUTING_ENABLED=1`. No promotion or split brain is involved.

Sequence:

1. Mount A creates and fsyncs `authority-unlink-original`, then closes it.
2. Wait for the physical replica to replay the seed and mount B startup state.
3. Pause replica replay in this isolated test stack.
4. Mount A creates `authority-unlink-alias` as a hardlink. The primary has one
   hardlink row; the paused replica has none.
5. Mount B unlinks the original pathname. The call reports success.
6. Query the primary directly. Both the original file and the surviving alias
   are gone; the visible alias count is zero.

The primary-only control repeated the same paused-replay scenario with replica
read routing disabled for B. It preserved the alias: the hardlink entry was
promoted to the main `files` row and the visible alias count was one. A moved
entry in `files` is a valid surviving hardlink, so counting only rows in
`hardlinks` would be an incorrect test oracle.

The source mechanism is explicit:

- `rust_fuse/src/fs.rs::unlink` branches on `repo.count_file_links(file_id)`:
  the last-link branch calls `defer_open_primary_unlink`, while the other
  branch promotes a hardlink.
- `rust_hotpath/src/pg.rs::count_file_links` uses `with_read_connection`, which
  can execute on the configured replica.
- `defer_primary_file_unlink` takes PostgreSQL advisory locks and marks the
  primary file unlinked, but does not re-evaluate the hardlink count there.
- `purge_primary_file` checks authoritative open leases under the file-open
  advisory lock. It does not protect a surviving hardlink by re-evaluating link
  count before deleting the file. The schema declares `hardlinks.id_file`
  with `ON DELETE CASCADE`, so the alias disappears with that row.
- The replica WAL barrier is local to a repository/mount. Passing its recorded
  LSN does not establish visibility of another mount's later committed link.

The failure was reproduced twice. The final failing artifact is
`artifacts/integration/primary-authority-audit-20261025T221954Z/results.json`;
the mount logs and executable `reproduce.py` diagnostic are in the same
directory. Its final assertion exits with a failure when the surviving alias
is lost. The earlier valid failure is recorded under `20261025T221839Z`.
The primary-only control is
`artifacts/integration/primary-authority-audit-20261025T221928Z/results.json`.
The first diagnostic's alias-count oracle was corrected to include both
`files` and `hardlinks`; use the control above rather than the earlier
`20261025T221806Z` diagnostic.

The required repair is an authoritative unlink transaction that re-resolves
the expected namespace entry, serializes link/open/unlink/purge decisions with
all relevant competing operations, rechecks hardlinks and open leases on the
primary, and chooses promotion/deferred deletion/purge there. Merely changing
the initial link-count read to primary still leaves a check-then-act race.

Until that repair is validated, keep replica read routing disabled for writable
mounts (`FOD_PG_REPLICA_READ_ROUTING_ENABLED=0`). This control is not proof that
every primary-only concurrent namespace race is already resolved.

## Confirmed routing and enforcement

| Operation family | Authority path | What was established |
| --- | --- | --- |
| Payload `write`, flush, fsync, truncate | `with_cached_connection` -> primary targets | Persistence transaction checks the file lease, live session and fencing token under a PostgreSQL advisory lock |
| `fallocate` hole punching, clone/copy adoption | Primary persistence APIs with a write fence | Hole-punch fencing regression passed; adoption also validates its destination fence in the primary transaction |
| Open/client sessions, ownership and heartbeat | `with_control_connection` -> the same primary targets; normal read-only mounts use a writable control repo | Remote open/unlink/recreate test passed with distinct inode generations and old-handle writes isolated from the replacement |
| `rename` / replace | Primary transaction with destination/file locks and source/target revalidation | Source inspection confirms transaction guards; test details are recorded below |
| `create`, mkdir, mknod, link, symlink, rmdir | Primary SQL through `with_cached_connection` | Routing is confirmed; this is not blanket proof of namespace conflict semantics |
| chmod/chown, timestamps, xattrs, mutating ioctl | Primary update/persistence APIs; read-only callbacks reject mutations | Routing is confirmed; authorization and cached precondition checks require separate scrutiny |
| Explicit degraded replica-only mount | Read-only FUSE mount with mutation guards | It deliberately does not acquire/renew primary-backed leases or grant write ownership |

`DbRepoLanes` gives the write repository primary connection targets and a
separate optional replica read pool. `with_cached_connection` always uses the
main pool, and `with_control_connection` uses a control lane of that same
authority. Calling a method a read operation does not make it safe for replica
routing if its answer selects a later mutation.

Legacy single-DSN repositories use the `Any` target requirement rather than
role-aware target validation. A PostgreSQL physical standby rejects ordinary
filesystem DML, but the legacy path does not itself establish that an arbitrary
writable DSN is the intended unique cluster authority.

## Runtime regressions

The following existing regressions passed in the isolated primary/replica
stack with both FOD mounts routed to the primary for authoritative operations:

- `test_stale_writer_fencing.py`: a primary-revoked stale writer cannot commit
  over a new owner; the stale fsync was rejected with `EBUSY`.
- `test_fallocate_stale_writer_fencing.py`: stale in-range and beyond-EOF hole
  punching were rejected, and the winning writer's payload was preserved.
- `test_unlink_remote_open_writer_multimount.py`: remote unlink/recreate kept
  distinct inode generations, old-handle writes and replacement data isolated,
  and PostgreSQL open-lease authority visible between mounts.

Logs are in
`artifacts/integration/primary-authority-audit-20261025T221631Z/`.

The rename test initially expected `(destination leases, file leases) = (0, 1)`
after opening the primary pathname for writing in its hardlink-source case.
`create_writable_handle_with_ownership` intentionally acquires both leases for
that pathname, so the correct assertion is `(1, 1)`. The test assertion was
corrected; conflict, rollback and payload assertions were retained. This
assertion failure did not establish a rename implementation defect.
After that correction, `test_rename_write_ownership.py` passed all eight
protected scenarios, including hardlink conflicts, replacement rollback and
same-destination races. It used a `release-lto` binary built with
`integration-test-hooks`; the log is
`artifacts/integration/primary-authority-audit-20261025T221954Z/test_rename_write_ownership.py.log`.

## Remaining source-level review items

- Mutation path resolution, hardlink counts, file sizes and attributes have
  replica-routed APIs that are also used by writable callbacks. Audit each
  mutation's preconditions and partial-block read/modify/write input for
  primary validation in its commit transaction. The unlink case above proves
  this is a real correctness boundary, not just a read-performance concern.
- `rmdir` reads directory contents before its separate primary `DELETE`.
  Foreign keys protect referenced children, but correct `ENOTEMPTY` mapping
  and concurrent namespace ordering need a dedicated test.
- Namespace uniqueness indexes in `base_schema.sql` are per table (`files`,
  `directories`, `hardlinks`, `symlinks`). They do not alone establish one
  cross-type `(parent, name)` conflict boundary. The audited create methods
  need explicit concurrent cross-type coverage.
- chmod/chown and sticky-bit checks use FUSE-side attributes before a separate
  primary update. The shared database credentials do not make those user-level
  authorization decisions transactional. Stale permissions and concurrent
  owner/mode changes need targeted validation.
- An explicit telemetry/control DSN is validated as writable but is not
  compared with the data source's cluster/filesystem identity before normal
  read-only mount leases are registered. Configuration and identity validation
  must keep those leases in the same logical filesystem authority.

These items are source-review findings and test gaps; do not describe them as
additional reproduced data-loss cases.
