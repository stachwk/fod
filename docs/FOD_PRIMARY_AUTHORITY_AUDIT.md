# Primary authority audit

Document role: topic review/specification supporting the
[current implementation queue](plans/CURRENT.md). Implemented capabilities are
summarized in [CURRENT_STATE.md](CURRENT_STATE.md); see the
[documentation map](README.md#correctness-reviews-and-evidence) for related reviews.

Date: 2026-10-25. Updated: 2026-10-27. Scope: the current working tree, Rust FUSE runtime and its
PostgreSQL repository. The historical unlink data-loss case below is repaired
and its mounted regression was verified on 2026-10-26. Other mutation families
still require the reviews listed at the end.

## Result

Mutation SQL and authoritative session/ownership operations use the primary
connection targets in a role-routed writable mount. Unlink now revalidates the
namespace and decides link/open/unlink/purge outcomes in one primary transaction.
The full mounted unlink regression passes with replica replay paused. Other
FUSE mutation preconditions still use replica-routed or cached reads, so this
does not close the remaining authority review.

## Historical P1: replica-based unlink decision deleted a surviving hardlink

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

The source mechanism in the failing build was:

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

The repair requires an authoritative unlink transaction that re-resolves
the expected namespace entry, serializes link/open/unlink/purge decisions with
all relevant competing operations, rechecks hardlinks and open leases on the
primary, and chooses promotion/deferred deletion/purge there. Merely changing
the initial link-count read to primary still leaves a check-then-act race.

## Current unlink implementation and validation

`FodFuse::unlink` uses `DbRepo::authoritative_view` for initial resolution and
permission inputs. `DbRepo::unlink_file_entry` then acquires namespace and file
lifecycle advisory locks in one primary transaction, revalidates the expected
binding, and chooses hardlink promotion, deferred removal or purge there.
`purge_primary_file_on_conn` rejects surviving hardlinks and active open leases.

On 2026-10-26, `test_unlink_primary_authority_docker.py` passed on source commit
`9ccf9e4` (FOD 3.4.48), using `release-lto` and `integration-test-hooks`, two
independent FUSE mounts and an isolated PostgreSQL primary/replica stack:

- a lagged replica preserved both root and nested hardlink aliases;
- a remote writer heartbeat and write survived primary-name removal/promotion;
- removing an alias preserved another pathname's destination ownership;
- an open unlinked writer remained usable and last close purged the old inode;
- ordered link/unlink contention preserved the alias when link won, while
  unlink-first made the later link fail with `ENOENT` instead of `EIO`.

Complete results and mount logs are under
`artifacts/integration/unlink-primary-authority-20261026T192523Z/`.
This closes the historical unlink data-loss case. Other mutation families below
remain open. Keep replica read routing disabled for general writable mounts
(`FOD_PG_REPLICA_READ_ROUTING_ENABLED=0`) until their preconditions are audited;
the unlink result alone does not establish their correctness.

## Current rmdir namespace transaction and validation

FOD 3.4.50 makes `rmdir` resolve its initial binding and sticky-bit inputs on
primary. `DbRepo::unlink_directory_entry` serializes the namespace key and
rechecks the full `(directory_id, parent_id, name)` binding in one primary
transaction. It checks children, detaches unlinked open files, then locks the
directory row and checks children again before deletion. The final row lock
conflicts with FK key-share locks used by child insertion, so emptiness cannot
race past a child transaction. Permission checks outside this transaction
remain part of the permission review below.

An open, unlinked file is not a visible namespace child. Its parent FK is set
to NULL in the removal transaction, preserving file identity, payload and
open/write leases; a remote open writer remains usable after parent removal.
The detach occurs before locking the directory because content updates also
check the file's directory FK. Locking directory then file inverted the
writer's file-then-directory order and produced a PostgreSQL deadlock during
recursive removal. The mounted regression modifies an open file while another
mount runs `rm -rf`; it verifies successful tree removal, post-removal access
through the open handle, and inode purge after the last close.

Before the repair, the new mounted test reproduced `EIO` instead of
`ENOTEMPTY` when a child existed on primary but not on the paused replica:
`artifacts/integration/unlink-primary-authority-20261026T193316Z/`.
After the repair, the expanded primary/replica test passed all unlink scenarios
plus authoritative non-empty checks, stale-directory rejection and parent
removal with a remote open unlinked writer:
`artifacts/integration/unlink-primary-authority-20261026T193731Z/`.
Both runs used `release-lto` and `integration-test-hooks`.

The full `pg_query` suite passed 29/29 tests. New backend regressions cover
stale/renamed bindings and a child transaction held uncommitted: removal waits
at the directory row lock, then sees the committed child and returns `NotEmpty`.
The recursive upload/removal integration scenario passed with the deadlock
lock-order fix in FOD 3.4.54:
`artifacts/integration/unlink-primary-authority-20261027T071251Z/`.

## Current cross-type namespace protection and validation

FOD 3.4.51 makes regular-file, special-file, directory, symlink and hardlink
creation take the same PostgreSQL destination advisory lock as rename and
file unlink. Each creator checks all four namespace tables on primary inside its
insertion transaction. An occupied destination returns `EEXIST` through FUSE.
The transaction locks an existing parent with `FOR KEY SHARE`, serializing
creation with directory removal; an absent parent returns `ENOENT`.
Writable create also rechecks the destination and backing hardlink on an
`authoritative_view` after acquiring destination ownership.

Existing bounded replay confirmation is preserved. Regular-file confirmation
now excludes special files and unlinked generations, and special-file
confirmation excludes unlinked generations. This prevents identical mode,
owner and seed metadata from confirming a FIFO as a regular-file creation.
This does not establish exactly-once semantics for every same-type request.

The mounted matrix exposed a separate deterministic type-decoding defect:
`attrs_for_path_from_repo` queried `special_files` for directory IDs, although
file and directory IDs come from independent sequences. When a directory ID
matched a FIFO file ID, FUSE returned FIFO attributes for a committed directory
and Linux rejected the mkdir reply with `EIO`. Special-file decoding now applies
only to files and hardlink backing files. The mounted regression requires an
actual directory/special-file numeric ID collision and verifies directory type.

The full `pg_query` suite passed 32/32 tests. New cases cover all 40 ordered
cross-type pairs at root and under a directory, missing parents, FIFO replay
misidentification and two independent creators observed waiting on the same
PostgreSQL advisory lock; exactly one creator commits after releasing it.
The complete two-mount `release-lto` / `integration-test-hooks` regression passed
all 40 paused-replica pairs plus the unlink/rmdir scenarios above:
`artifacts/integration/unlink-primary-authority-20261026T200044Z/`.
Permission authorization, other mutation inputs and cross-host cache coherence
remain separate unfinished work. General writable replica read routing stays
disabled pending those reviews.

## Current chmod/chown authorization and validation

FOD 3.4.52 routes `setattr` metadata resolution and its reply through primary.
The mode/owner portion uses `change_inode_permissions`: it locks the expected
inode row with `FOR UPDATE`, validates its seed, reads current UID/GID/mode,
authorizes the caller and applies all requested mode/owner fields in the same
transaction. Hardlinks use their backing file inode. Missing/stale targets
return `ENOENT`, rejected ownership or group changes return `EPERM`.

Unspecified UID/GID fields retain the locked database values, rather than the
requester's identity. A rejected combined chmod/chown leaves both unchanged.
File chown clears privilege bits; directory chown preserves them. Non-root
chmod cannot set SGID for a group outside the caller's group set. Existing
low-level metadata APIs remain available for internal operations; this is not
an authorization boundary for arbitrary direct SQL clients.

The full backend suite passed 34/34 tests, including a concurrent chown held
uncommitted while chmod waits at the row lock. After the ownership change
commits, the previous owner's chmod is denied and the original mode survives.
Other cases cover expected-seed rejection, partial owner updates, files,
directories and symlinks, and rejected combined updates without partial writes.
FUSE unit tests passed 69/69.

The two-mount `release-lto` / `integration-test-hooks` suite passed with replica
replay paused: a primary ownership change prevents the replica's previous owner
from chmod/chown of files, hardlinks, directories and symlinks. The same run
passed the existing permission/sticky/chown compatibility test and all earlier
namespace/unlink/rmdir regressions:
`artifacts/integration/unlink-primary-authority-20261026T200613Z/`.
That compatibility test alone did not establish transactional sticky-bit or
ACL ordering. FOD 3.4.53 adds sticky-bit transactions and regressions below.
Parent access checks, ACL/mode atomicity and truncate/timestamp authorization
still need their own transactional coverage. This change does not make every
`setattr` field part of one transaction.

## Current transactional sticky-bit checks

FOD 3.4.53 moves sticky-bit authorization from pre-transaction FUSE attribute
reads into the primary transactions for file/hardlink unlink, symlink unlink,
rmdir and rename. Each guarded operation holds a `FOR SHARE` lock on its
stored parent directory while checking its current mode and owner. If sticky
mode requires checking the victim owner, that inode owner is also read under
a row lock held through commit. Hardlinks use the backing file's owner rather
than the alias row's UID. Root UID, directory owner and victim owner retain
their sticky-bit exemptions. Denials return `EPERM` and roll back the mutation.

Rename checks both the removed source and any replaced target after binding
revalidation, before changing either entry. FUSE rename resolution, backing
hardlink lookup and mutation parent-ID lookup now use primary. Symlink unlink
additionally locks and validates its full expected `(id, parent, name)` binding
before deletion, instead of deleting a previously resolved ID alone.

The full backend suite passed 37/37 tests. New regressions cover all removal
kinds, all source and target kinds for rename, permitted root/owner exemptions,
wrong symlink bindings, and five mutations blocked by an uncommitted parent
mode/owner change. The test observes the actual PostgreSQL lock wait; after
the permission transaction commits, each mutation rejects the former owner.

The complete two-mount `release-lto` / `integration-test-hooks` regression
passed nine sticky-bit cases with replica replay paused, alongside earlier
40 cross-type cases, chmod/chown, unlink, rmdir and permission compatibility:
`artifacts/integration/unlink-primary-authority-20261027T065500Z/`.
FUSE unit tests passed 69/69; version checks passed 10/10.

This closes the sticky-bit check/act gap for stored parents in those mutation
transactions.

FOD 3.4.56 adds a primary transaction check for `mkdir` under stored parents;
FOD 3.4.57 extends it to regular-file creation (`create`/`O_CREAT` and regular
`mknod`). When ACL support is disabled, both paths lock the parent row while
checking the requester's write+search mode bits and supplementary groups, so a
concurrent `chmod` is ordered with creation. Backend regressions verify denial
without a created entry and supplementary-group access. The logical root still
has synthesized mode/owner attributes; ACL-enabled creation and other namespace
mutations remain outside this increment.

## Confirmed routing and enforcement

| Operation family | Authority path | What was established |
| --- | --- | --- |
| Payload `write`, flush, fsync, truncate | `with_cached_connection` -> primary targets | Persistence transaction checks the file lease, live session and fencing token under a PostgreSQL advisory lock |
| `fallocate` hole punching, clone/copy adoption | Primary persistence APIs with a write fence | Hole-punch fencing regression passed; adoption also validates its destination fence in the primary transaction |
| Open/client sessions, ownership and heartbeat | `with_control_connection` -> the same primary targets; normal read-only mounts use a writable control repo | Remote open/unlink/recreate test passed with distinct inode generations and old-handle writes isolated from the replacement |
| `rename` / replace | Primary transaction with destination/file locks and source/target revalidation | Source inspection confirms transaction guards; test details are recorded below |
| `create`, mkdir, mknod, link, symlink | Shared destination advisory lock and cross-table vacancy check in the primary insertion transaction | All 40 root/nested cross-type conflicts, concurrent creators and directory/FIFO ID separation passed; permission checks remain open |
| `rmdir` | Primary namespace/row locks, sticky-bit check and delete in one transaction | Lagged replica, stale binding, concurrent child commit and remote open-unlinked writer regressions passed |
| Sticky bit on unlink/rmdir/rename | Parent/victim owner row locks inside the primary mutation transaction | Concurrent parent permission commit and nine stale-replica cases passed |
| chmod/chown | Locked primary inode, seed check and authorization in the same update transaction | Concurrent chown and stale-replica owner rejection passed; partial owner fields preserved |
| Timestamps, xattrs, mutating ioctl | Primary update/persistence APIs; read-only callbacks reject mutations | Authorization, ACL/mode atomicity and other precondition checks still require scrutiny |
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

## Reproduced partial directory-path lookup

Status: repaired in FOD 3.4.45 on 2026-10-26.

During the namespace lookup module extraction on 2026-10-26, an isolated
PostgreSQL database initialized from the current schema reproduced a separate
lookup defect: after creating `/existing`, `DbRepo::get_dir_id("/existing/missing")`
returned the ID of `/existing`, rather than `None`. This observation does not
depend on replica lag or concurrent mutations.

The mechanism was in `PreparedStatement::GetDirId` in `rust_hotpath/src/pg.rs`:
the recursive `walk` stopped at the first missing component, but its final
`ORDER BY ord DESC LIMIT 1` returned the deepest successful prefix without
requiring the final input component to match. The query now accepts only a
row whose ordinal matches the final input component.

The public `get_dir_id` API retains `None` for the logical root, whose children
have SQL NULL parents. The namespace methods in
`rust_hotpath/src/pg/namespace_lookup.rs` now explicitly distinguish root,
resolved directory and missing directory before choosing a query. A missing
directory cannot select an ancestor's children or fall back to root entries.
ID/mode/attribute/xattr lookups return no entry, directory metadata listing
returns an empty vector and the optional directory-name blob returns `None`.
Connection routing and PostgreSQL mutation transactions are unchanged.

The regression `directory_lookup_rejects_missing_path_components` in
`rust_hotpath/tests/pg_query.rs` is enabled and passes for missing final and
intermediate components, valid nested directories, relative directory names,
trailing slashes and the root sentinel. The additional
`missing_namespace_parents_never_alias_existing_entries` regression passes
for path resolution, ID/mode queries, directory listings, attributes and
xattrs. Existing namespace-identity and xattr tests also pass.

Initial validation used a temporary database initialized by the current `fod-mkfs`;
it was removed after testing. `cargo check --workspace --offline` passed.
The full `pg_query` suite passed 24/25 tests, with none ignored. The remaining
`unlinked_open_inode_rejects_new_hardlinks_and_purge_until_last_close` test
failed with `unexpected PostgreSQL result status`. It also failed with the
same error in an isolated checkout of unchanged commit `5008010`, against a
separate freshly initialized database.

The follow-up diagnosis on 2026-10-26 localized that failure to
`register_client_session`, before the open/unlink assertions. Fresh `mkfs`
creates `client_sessions` without its runtime `request_token` column; the
session registration probe queries that column. The generic result decoder
masked the PostgreSQL failure as `unexpected PostgreSQL result status`.
Writable FUSE startup calls `ensure_client_session_schema` before registering
a session, but this test omitted that initialization.

The test now performs the same schema initialization and checks that the open
lease exists before unlink and is released before final purge. An isolated
fresh-database run confirmed zero matching `request_token` columns before the
test and one afterward; the focused regression passed. The full `pg_query`
suite subsequently passed 27/27 tests with no skipped or ignored cases.
This closes the test-fixture failure. The later full mounted regression is
recorded in the current unlink validation section above; other mutation
families still require the remaining authority review.

## Remaining source-level review items

### POSIX default ACL inheritance

Source review found that the former `FodFuse::copy_default_acl_to_child`
inherited ACLs after inode creation, through separate reads and generic xattr
writes whose failures were ignored. FOD 3.4.73 replaces that path for regular
files, directories and special files: the create transaction locks the parent
directory row, reads its default ACL on the primary, filters the inherited
access ACL against the requested mode, writes child mode and ACL xattrs, and
commits them with the new inode. Directories also inherit the unmodified default
ACL. A malformed inherited ACL or any SQL failure aborts inode creation.

`cargo check --workspace --offline` and rustfmt validation passed. The mounted
ACL integration scenario has not yet been rerun against this implementation;
keep that behavioral validation open. This was a source-level finding, not a
reproduced data-loss case. Parent ACL permission evaluation during create still
uses the kernel/FUSE path and requires separate review.

### Logical root metadata

The base schema has a durable `/` row in `directories` (`id_parent IS NULL`,
`name='/'`), and `resolve_path("/")` returns that row. `FodFuse::root_attr` now
reads its metadata through the authoritative PostgreSQL view, keeps the kernel
inode fixed at `ROOT_INO`, and propagates root child-count errors instead of
silently reporting zero. This makes stored mode, uid/gid and atime/mtime visible
to `getattr` across mounts. Root-level namespace operations still use a null
parent id. Migration 0028 adds PostgreSQL triggers that update the parent
directory mtime/ctime in the same transaction as namespace entry creation,
deletion, rename and movement. A null namespace parent maps to the durable root
row, so root directory changes are included.

The database record already exists; root reads and namespace timestamp
ordering are now handled through PostgreSQL, and root `setattr` mutations use
the locked inode authorization paths described below. No data-loss
reproduction is claimed.

### Truncate and timestamp authorization

Path-based `setattr(size=...)` previously acquired write ownership and persisted
the new size without checking the requester's write access. It now checks the
resolved file's primary ACL/mode before acquiring ownership. A request
carrying `fh` now also requires a live handle for the same file opened with
write access; otherwise it returns `EBADF`. Truncating the logical root now
returns `EISDIR`. Path truncation repeats the write-access check in the content
persistence transaction while holding the file row lock. When POSIX ACL support
is enabled, the transaction checks the ACL; otherwise it checks mode bits. ACL
updates lock the same row, so whichever transaction obtains the lock first
determines the operation order. Truncation through `fh` uses the validated
writable handle's open-time access, consistent with descriptor semantics.

Partial writes and truncate-boundary updates now load their read-modify-write
input blocks from the write authority. Ordinary block reads may use a replica;
using those results as mutation input could overwrite newer bytes after a
replica lagged behind a committed writer. Existing write ownership and fencing
serialize competing writers, while the authoritative read supplies the newest
committed block before local edits are persisted.

Timestamp `setattr` now resolves the inode seed and applies POSIX time-setting
authorization and timestamp writes in one primary transaction under the inode
row lock. Owners and uid 0 may set specific times; other users may request only
`NOW`/omitted fields and must have write permission through the access ACL or
mode bits. ACL updates lock the same inode row, which serializes authorization
against concurrent ACL changes. Only requested atime/mtime fields are updated;
symlink times are no longer ignored. Direct low-level repository touch helpers
remain available for internal callers and do not carry a request identity.

- Mutation path resolution, hardlink counts, file sizes and attributes have
  replica-routed APIs that are also used by writable callbacks. Partial-block
  read/modify/write input now comes from the primary, but continue auditing
  mutation preconditions and inputs for primary validation in their commit
  transactions. The historical unlink failure establishes why read routing
  affects mutation correctness.
- Chmod/chown and sticky-bit authorization use locked primary rows as described
  above. Transactional parent write/search checks cover regular-file,
  directory, special-file, symlink, and hardlink creation, plus file and
  symlink unlink, `rmdir`, and rename. POSIX ACL xattr writes now check inode
  ownership under the write transaction; default ACL inheritance now shares the
  child creation transaction (mounted validation pending). Root `setattr`
  authorization uses the locked inode metadata paths. Path truncate now has an
  ACL/mode check and repeats authorization under the file row lock in its
  content transaction. Timestamp `setattr` authorization and field-preserving
  updates also share one primary transaction.
  Shared database credentials do not replace these decisions.
- An explicit telemetry/control DSN is validated as writable but is not
  compared with the data source's cluster/filesystem identity before normal
  read-only mount leases are registered. Configuration and identity validation
  must keep those leases in the same logical filesystem authority.

These items are source-review findings and test gaps; do not describe them as
additional reproduced data-loss cases.
