# Primary authority audit

Document role: topic review/specification supporting the
[current implementation queue](plans/CURRENT.md). Implemented capabilities are
summarized in [CURRENT_STATE.md](CURRENT_STATE.md); see the
[documentation map](README.md#correctness-reviews-and-evidence) for related reviews.

Date: 2026-10-25. Updated: 2026-10-31. Reviewed base: FOD 3.4.401
(`fcff376`). Scope: Rust FUSE runtime and its PostgreSQL repository. The
historical unlink data-loss case below is repaired and its mounted regression
was verified on 2026-10-26. Later increments addressed specific stale or
replica-routed mutation inputs, authorization checks and inode/path bindings;
the current evidence is summarized in the routing matrix and follow-up sections
below. This remains a targeted review, not an exhaustive proof that every
mutating callback and repository transaction has been audited.

## Result

Mutation SQL and authoritative session/ownership operations use the primary
connection targets in a role-routed writable mount. The original unlink bug,
where a lagging replica could drive link/open/unlink/purge decisions, is fixed
by one primary transaction. Follow-up reviews moved specific write and copy
inputs (sizes and payload bytes), pathname and inode identity checks, parent
permission/ACL decisions, timestamp authorization and open-handle metadata to
primary-backed or transaction-locked operations. `access()` loads request inode
attributes from primary before applying root's `X_OK` execute-bit rule when FOD
handles permission checks; paused-replica regressions cover that case and
kernel `default_permissions` attribute refresh. The source review remains open:
these findings do not establish that every mutating callback revalidates every
decision inside the transaction that commits its change. Keep general writable
replica-read routing disabled until that caller-by-caller review and the
separate cross-host cache-coherence work are complete. The current bounded
evidence and known exclusions are listed in the matrix and sections below.

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

Deferred unlink cleanup after the last handle closes also uses
`DbRepo::authoritative_view` for the unlinked-state check, active-lease check
and final purge. The previous read-routed first check could observe a lagging
replica and leave an unlinked inode behind permanently. The mounted regression
pauses replica replay between unlink and final close and checks that primary
purges the inode. The test stack verifies that both PostgreSQL services use
the FOD PostgreSQL 16.15 build (`--with-blocksize=32`, `SHOW block_size =
32768`). On 2026-10-31, both PostgreSQL checks passed, but mount startup stopped
before assertions because the host has libfuse 3.17 and requires 3.18.2 or
newer; the regression remains pending on a supported host.

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

On 2026-10-30, the two-mount `test_unlink_primary_authority_docker.py` suite
also paused replica replay and performed `mkdir`, regular-file `create`,
`mknod` (FIFO), `symlink` and `link` from the replica-routed mount. For each
operation, the primary contained exactly one committed entry while the replica
still contained none, and the syscall returned metadata with the correct inode
type. The existing paused-replica cross-type matrix continued to require
`EEXIST` for all 40 conflicting pairs. The suite passed against an isolated
PostgreSQL primary/replica stack; results are in
`artifacts/integration/unlink-primary-authority-20261030T104143Z/results.json`.

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
replay paused. A primary ownership change prevents the replica's previous owner
from using `chmod()`/`chown()` on files, hardlinks, directories and symlinks.
The same run passed the existing permission/sticky/chown compatibility test and
all earlier namespace/unlink/rmdir regressions:
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
mutations remained outside this increment. Later changes added transactional
parent ACL checks for creation and namespace removal/movement.

## Confirmed routing and enforcement

| Operation family | Authority path | What was established |
| --- | --- | --- |
| Payload `write`, flush, fsync, truncate | `with_cached_connection` -> primary targets | Persistence transaction checks the file lease, live session and fencing token under a PostgreSQL advisory lock; the unchanged-slice write optimization also reads comparison bytes from primary |
| `fallocate` hole punching, clone/copy adoption | Primary persistence APIs with a write fence | Hole-punch fencing regression passed; adoption also validates its destination fence in the primary transaction |
| Open/client sessions, ownership and heartbeat | `with_control_connection` -> the same primary targets; normal read-only mounts use a writable control repo | Remote open/unlink/recreate test passed with distinct inode generations and old-handle writes isolated from the replacement |
| `rename` / replace | Primary transaction with destination/file locks and source/target revalidation | Source inspection confirms transaction guards; test details are recorded below |
| `lookup`/`opendir` and namespace parent identity | Current parent inode is verified against primary metadata before resolving children or opening a directory snapshot | `lookup` gets the directory ID from the same primary row that validates the parent inode, queries the child by `(directory_id, name)`, and loads attributes by the returned entry ID; parent ACL checks use ID-bound attributes. `opendir` checks the requested inode against primary before snapshot creation. The `..` entry's inode is now derived from the opened directory's PostgreSQL parent ID and fetched by ID from primary, rather than the local path cache. The handle retains its PostgreSQL directory ID, used for initial/rewind snapshots, names-mode child metadata and atime writes, so path reuse cannot redirect an old handle. Snapshot reads remain routed by the configured read repository. `unlink`/`rmdir` also compare the resolved entry's parent ID. Two-mount stale-inode `opendir`, open-handle remove/recreate and rewind regressions pass in `test_stale_inode_directory.py` and `test_unlink_primary_authority_docker.py`; the standalone stale-inode regression passed on 2026-10-30. `test_readdir_snapshot_multimount.py` evicts the local parent path before nested `opendir()` and verifies `..` inode from `getdents64()` on FOD 3.4.458. |
| `create`, mkdir, mknod, link, symlink | Shared destination advisory lock, cross-table vacancy check and parent write/search authorization in the primary insertion transaction | All 40 root/nested cross-type conflicts, concurrent creators and directory/FIFO ID separation passed; parent authorization and inode identity are checked against primary metadata |
| `rmdir` | Primary namespace/row locks, sticky-bit check and delete in one transaction | Lagged replica, stale binding, concurrent child commit and remote open-unlinked writer regressions passed |
| Sticky bit on unlink/rmdir/rename | Parent/victim owner row locks inside the primary mutation transaction | Concurrent parent permission commit and nine stale-replica cases passed |
| chmod/chown | Locked primary inode, seed check and authorization in the same update transaction | Concurrent chown and stale-replica owner rejection passed; partial owner fields preserved |
| Access checks, timestamps, symlink lookup, open-handle stat/setattr, xattrs, mutating ioctl, lock identity | Primary update/persistence APIs; read-only callbacks reject mutations | FUSE access/open checks resolve current inode owner, group, mode and file identity from primary; parent search/ directory-read checks remain active alongside kernel `default_permissions`; `getattr`, `setattr` and file-handle ioctl attributes use current metadata from primary and remain bound to the handle's file ID; `link` verifies its inode against primary metadata before selecting the source file ID; `read`/`write` enforce descriptor modes and unknown handles fail closed instead of resolving by path; `flush`/`fsync` reject unknown handles; lock resource keys, write-ownership parent IDs, atime target IDs, symlink targets and `getlk`/`setlk` type checks resolve on primary; xattrs check namespace permissions; clone ioctls and `copy_file_range` validate source/destination access modes |
| Explicit degraded replica-only mount | Read-only FUSE mount with mutation guards | It deliberately does not acquire/renew primary-backed leases or grant write ownership |

`DbRepoLanes` gives the write repository primary connection targets and a
separate optional replica read pool. `with_cached_connection` always uses the
main pool, and `with_control_connection` uses a control lane of that same
authority. Calling a method a read operation does not make it safe for replica
routing if its answer selects a later mutation.

Create/open rollback cleanup follows the same rule. It now asks the primary
transaction to promote a current hardlink first; when none is promoted, the
primary purge transaction rechecks hardlinks and open leases while holding the
file-lifecycle lock. A replica-routed link count no longer selects between
these mutations.

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

### POSIX default ACL inheritance and create authorization

Source review found that the former `FodFuse::copy_default_acl_to_child`
inherited ACLs after inode creation, through separate reads and generic xattr
writes whose failures were ignored. FOD 3.4.73 replaces that path for regular
files, directories and special files: the create transaction locks the parent
directory row, reads its default ACL on the primary, filters the inherited
access ACL against the requested mode, writes child mode and ACL xattrs, and
commits them with the new inode. Directories also inherit the unmodified default
ACL. Root-level creates resolve the durable `/` row and inherit its default ACL
as well. A malformed inherited ACL or any SQL failure aborts inode creation.
The same create transaction now checks the caller's write+search permission on
the parent using its access ACL (or mode bits when no ACL exists), while holding
the parent row lock. This applies to regular files, directories, special files,
symlinks and hardlinks; root-level creates check the stored logical root row.
ACL and mode changes therefore serialize against creation authorization.

`cargo check --workspace --offline` and rustfmt validation passed. The mounted
`make test-acl-mount-option` regression now passes against this implementation:
it sets and reads a POSIX access ACL, checks the resulting mode and access
decisions, validates default ACL inheritance and creation-mode filtering through
a nested directory, and confirms denied access. FOD 3.4.435 fixes umask handling
for ACL-enabled creates: the mount negotiates `FUSE_DONT_MASK` alongside
`FUSE_POSIX_ACL`, so Linux preserves the requested mode and passes umask to FOD.
Under a default ACL, FOD filters that requested mode; without one it applies
umask in the same PostgreSQL transaction that reads the parent. The regression
exercises capability negotiation, named-user/group-class entries, the
ignored-umask case, child directories retaining the unfiltered default ACL,
mode-filtered nested files, and umask application to files, directories and
FIFOs when a parent has no default ACL.
`make test-xattr` and `make test-ioctl` also pass on the mounted filesystem;
the xattr test uses its documented compatibility fallback for namespaces that
the host denies. `FIONREAD` is exercised through FUSE. `FICLONE` returns the
documented unsupported error on the current FUSE stack, so this test does not
exercise a FOD clone implementation. Linux VFS dispatches `FICLONE` and
`FICLONERANGE` through `file_operations.remap_file_range`; Linux v6.17's FUSE
file operations provide `copy_file_range` but no `remap_file_range`. The
unreachable FOD clone branches have been removed. See the [Linux FUSE file
operations](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/fs/fuse/file.c?h=v6.17)
and [VFS ioctl dispatch](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/fs/ioctl.c?h=v6.17).
This does not validate SELinux MAC enforcement or every ioctl/xattr namespace.
`unlink`, `rmdir` and both parents of
`rename` now perform the write+search check through the parent access ACL (or
mode bits when no ACL exists) inside the namespace mutation transaction. The
parent row lock serializes each check with ACL and mode changes, including for
the durable logical root. The mounted `test_acl_mount_option.py` regression now
uses an access ACL that leaves mode bits at `0777` but grants the named `nobody`
user only search permission. It verifies exact `EACCES` for regular-file,
directory, FIFO/`mknod`, symlink and hardlink creation, `unlink()`, `rmdir()`,
rename from a denied source parent, and rename into a denied destination
parent, while confirming that source and destination namespace entries remain
unchanged. The test passed on 2026-10-30. This verifies end-to-end mounted ACL
authorization; it is not a data-loss reproduction.

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
The old file size used to plan a truncate boundary is also mutation input.
`prepare_truncate_boundary_blocks()` now reads it through
`file_size_authoritative()`. The mounted regression pauses standby replay after
the file grows on primary, then extends it through the replica-routed mount and
checks both the visible bytes and the stored primary block. It passed on
2026-10-30 in `test_unlink_primary_authority_docker.py` (artifact:
`artifacts/integration/unlink-primary-authority-20261030T184010Z/`).
Write and copy planning also reads current file sizes from the write authority.
Copy source blocks and destination blocks used for deduplication are likewise
read from primary. A replica's stale size or payload could otherwise make a
buffered write/copy persist an older, shorter extent or copy stale bytes.
The non-adoption `copy_file_range` path now uses the same authoritative slice
reader; its two-mount regression pauses replay, changes the source on primary,
copies a partial range through the replica-routed mount, and checks the
destination block directly on primary. It passed on 2026-10-30.

The write callback's unchanged-slice optimization is also mutation input: if
the requested bytes equal a lagged replica's old contents, skipping persistence
would silently lose a real overwrite. It now loads the compared slice through
`assemble_file_slice_authoritative()`. The two-mount integration regression
pauses replica replay, commits a same-length replacement through the primary,
then writes the old replica bytes through the replica-routed mount and checks
the persisted `data_blocks` bytes directly on primary. The check passed on
2026-10-30 as part of `test_unlink_primary_authority_docker.py`.

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
  read/modify/write input, write/copy planning sizes and copy data inputs now
  come from the primary, but continue auditing mutation preconditions and
  inputs for primary validation in their commit transactions. The historical
  unlink failure establishes why read routing affects mutation correctness.
- Chmod/chown and sticky-bit authorization use locked primary rows as described
  above. Transactional parent write/search checks cover regular-file,
  directory, special-file, symlink, and hardlink creation, plus file and
  symlink unlink, `rmdir`, and rename. ACL-enabled mounts use the parent access
  ACL for namespace removal and movement, with the same checks and row locks
  inside each PostgreSQL transaction. POSIX ACL writes and non-`user.*` generic
  xattr writes/removal check inode ownership in the transaction; `user.*`
  writes use locked ACL/mode write permission, and `trusted.*` requires uid 0.
  Generic xattr `CREATE`/`REPLACE` flags are serialized with the value update.
  `user.*` reads check ACL/mode read access, while non-root `listxattr` filters
  `trusted.*` names; denied `user.*`/`trusted.*` reads return `ENODATA`.
  Default ACL inheritance now shares the child creation transaction (mounted
  access-ACL and inheritance checks passed). Root `setattr`
  authorization uses the locked inode metadata paths. Path truncate now has an
  ACL/mode check and repeats authorization under the file row lock in its
  content transaction. Timestamp `setattr` authorization and field-preserving
  updates also share one primary transaction.
  Standard Linux FUSE does not route `FICLONE` or `FICLONERANGE` VFS remap
  ioctls to the userspace ioctl callback on the tested v6.17 kernel.
  `FICLONE` therefore returned `EOPNOTSUPP` before userspace; the mounted test
  verifies this unsupported result and that the destination stays unchanged.
  FOD has no clone ioctl implementation on this backend. Mounted `FIONREAD`
  remains validated.
  FUSE `access` and `open` also evaluate owner/group/mode from primary and
  reject a stale inode-to-path mapping before deciding access.
  Shared database credentials do not replace these decisions.
- An explicit telemetry/control DSN is now compared with the data source's
  PostgreSQL cluster system identifier and database OID before the mount
  registers client sessions or open-file leases. A mismatch or inability to
  read either identity fails mount startup, keeping lifecycle state in the same
  logical database authority as filesystem data.

### Rename preflight and transaction authority

Source review on FOD 3.4.401 confirms that `rename()`'s source and destination
lookups in `rust_fuse/src/fs.rs` only prepare candidate entry IDs and
operation-specific errno checks. The callback first maps both parent inode
numbers to their current primary-resolved directory IDs. The PostgreSQL
transaction in `rename_namespace_replace_with_ownership_guard_internal()` then
acquires sorted transaction-scoped advisory locks for both destination names
and the source/target file resources, checks active destination and file leases,
and resolves both names again using the parent IDs and names. It compares the
resolved kind, entry ID, and backing file ID with the candidates before any
deletion or rename. Parent ACL/mode checks, sticky-directory checks, and the
directory-cycle query also run in that transaction. If another transaction
changes a binding after preflight, the operation returns a conflict/source
change outcome without applying the stale candidate.

This source review closes the specific concern that a preflight rename lookup
could independently authorize a mutation against an old binding. It is not a
full interleaving proof for namespace mutation and does not close the broader
mutation-precondition audit; deterministic mounted regressions remain the
evidence for runtime behavior.

FOD 3.4.437 also fixed replacement of an open regular-file target. The rename
transaction holds the target's PostgreSQL inode-lifecycle advisory lock; the
shared removal helper now checks active `file_open_leases` before deleting the
row. If a lease exists, it marks the target unlinked under a private name and
keeps its inode/data until the last handle release purges it. This is essential
when the descriptor is open on another mount, whose process-local handle table
cannot protect the target. `test_rename_directory_transactional.py` verifies
that the old descriptor keeps its bytes and inode with `st_nlink == 0`, while
the replacement path becomes visible on both mounts. The test passed on Linux
6.17.0-41-generic/libfuse3 3.18.2/PostgreSQL 16.15 against a disposable DB.

### Xattr callbacks and stale inode-to-path mappings

Source review on 2026-10-30 found that `setxattr`, `getxattr`, `listxattr`, and
`removexattr` selected xattr owners by resolving the locally cached path without
checking that it still named the FUSE inode in the request. If another mount
removed and recreated that path, the stale inode could read or mutate the new
object's xattrs. `ParsedAttrs` now carries the xattr owner kind and database ID
from the resolved metadata; all four callbacks resolve the path against the
primary and reject an inode/path mismatch before addressing xattrs by that
verified owner ID. For a still-open file whose path was unlinked or
replaced, they bind to the open file ID instead. Hardlinks resolve to their
underlying file ID, and the root directory retains owner ID 0. The xattr
mutation transaction then locks that owner row as before. `user.*` reads
evaluate read permission using the same verified attributes and fetch the
access ACL by owner ID, including the open-unlinked case, rather than trying to
re-resolve a vanished pathname.
The mounted regression now exercises both cases across two mounts with ACLs
enabled: an open-unlinked file remains bound to its original file ID, and a
cached path inode whose name was remotely replaced returns `ENOENT` on
`setxattr` without changing the replacement object's xattr. The regression
passed against an isolated PostgreSQL 17.11 instance on 2026-10-30. While
making the stale-path case deterministic, it also exposed that the documented
FUSE cache timeout environment variables were incorrectly passed as mount
options. FOD now places entry and attribute TTLs in FUSE replies and can return
cacheable negative lookup replies; all three configured timeout variables are
covered by the mounted regression. This verifies the described cases, not
every xattr namespace or every interleaving.

### Directory open after remote path replacement

Source review on 2026-10-30 found that `opendir()` obtained attributes through
the replica-routed repository and did not confirm that its cached path still
resolved to the requested directory inode on primary. A remote remove/recreate
could therefore let a stale FUSE directory inode open and snapshot the
replacement directory. `opendir()` now validates the inode/path pair through
the primary-backed `authoritative_entry_attrs_for_ino()` before creating the
configured read snapshot. The snapshot remains read-routed after this identity
check. A follow-up review found that refreshing an already-open stream by path
could also attach a replacement directory on rewind. FOD now keeps the
PostgreSQL directory ID on the handle and uses it for snapshots, names-mode
child lookups and atime updates. A mounted two-mount regression covers stale
`opendir()` and remove/recreate during an open stream/rewind. It verifies a
replacement cannot reuse the old FUSE-created directory inode, the existing
handle remains bound to its PostgreSQL directory ID through rewind, and a fresh
mount sees the replacement.
FUSE-created object inode seeds combine the PostgreSQL-assigned mount session
ID with a mount-local monotonic counter and are persisted on each object.
The regression passed against an isolated PostgreSQL 17.11 instance.

### `readlink()` after remote symlink replacement

Source review found that `readlink()` resolved the cached inode path on primary
but checked only that the current object at that path was a symlink. If another
mount removed and recreated the same name, the stale inode could therefore
receive the replacement symlink's target. The callback now obtains
primary-resolved attributes through `authoritative_entry_attrs_for_ino()`,
requires the requested inode to still identify a symlink, and fetches its target
by the verified PostgreSQL symlink ID. Automatic atime updates use that same ID
instead of resolving the path a second time.

`test_stale_inode_readlink.py` keeps mount B's entry cache alive while mount A
replaces the symlink. The old inode must return `ENOENT`; after remount, the
replacement returns its exact target. The two-mount regression passed on FOD
3.4.367 against a fresh schema-32 database inside a private mount namespace.
The public test target is `make test-stale-inode-readlink`. This closes this
specific stale-symlink path; other mutation preconditions remain under review.

### POSIX locks after unlink and path replacement

Source review found that `getlk()` and `setlk()` ignored the FUSE file handle
and selected a lock resource by resolving the current pathname. If another
mount unlinked an open file and created a replacement under that name, a lock
request through the old descriptor could target the replacement's PostgreSQL
`file_id`. Both callbacks now validate the inode against the open handle and
use that handle's authoritative file ID for the lock key.

`file_locks_remain_bound_to_open_inode_after_path_replacement` opens the
original through one mount, replaces its pathname through another, locks the
old descriptor and verifies `F_GETLK` on the replacement reports `F_UNLCK`.
It passed on FOD 3.4.368 against a fresh schema-32 database inside a private
mount namespace.

### Whole-object copy source-write serialization

`adopt_source_data_object_optional_fence()` modifies the destination inode to
share the source's data object. Before this fix, the transaction serialized
only against destination writers. A source block writer could make its
copy-on-write decision under `fod:write:file:<source-id>` while adoption
incremented the shared object's reference count without taking that lock. This
left an interleaving where the source writer could update the newly shared
object, changing the destination after the copy.

FOD 3.4.397 takes both source and destination write advisory locks in sorted
resource-key order inside the adoption transaction, then re-reads object metadata
and validates the destination fencing lease before mutating. Stable ordering
avoids reciprocal-copy deadlocks; the lock is transaction-scoped and serializes
with ordinary fenced writes and legacy writes that reject active leases.
`tests/integration/test_copy_file_range_source_fencing.py` provides a
two-mount regression: it holds the source write key, requires the copy to block
on that exact PostgreSQL backend lock, releases it, then writes the source and
checks that the destination retains the original object and bytes. The test is
registered as `make test-copy-file-range-source-fencing` and in
`make test-integration`.

The C9 regression passed on FOD 3.4.397 in `release-lto` against disposable
PostgreSQL 16.15 on Linux 6.17.0-41-generic/libfuse3 3.17.4. Existing C7
destination-ownership and C8 stale-fencing regressions passed against the same
database afterward. The disposable database was removed. No pre-fix
byte-corruption reproduction was recorded; the source-level race mechanism is
supported by the transactions and the regression confirms the corrected
serialization and copy-on-write isolation.

### `create()` reply after concurrent path replacement

FUSE `create()` committed the new row and bound its file handle to the returned
`file_id`, then looked up the path again to construct the `CREATE` reply. A
second mount could unlink the new name and create a replacement before that
lookup. The callback then returned the replacement's inode attributes together
with the original file handle. `create()` now loads reply attributes by the
handle's `file_id`, including when that inode has been unlinked in the race.

`test_create_write_ownership.py` uses an integration-only barrier after the
create transaction commits and before the FUSE reply. Mount B unlinks and
recreates the name while mount A is paused. With the old path-based reply, the
test failed deterministically because both handles reported the replacement
inode. With the ID-bound reply, the original handle and replacement have
different inode numbers and independent payloads. The full test passed on FOD
3.4.451 in `release-lto` against a disposable PostgreSQL 16.15 database on
Linux 6.17.0-41-generic/libfuse3 3.18.3; the database was dropped afterward.

### Other namespace-create replies after concurrent path replacement

The same post-commit path lookup existed in `mkdir`, `mknod`, `symlink` and
`link`. The first three callbacks now load reply attributes by the entry ID
returned from their committed PostgreSQL transaction. `link` loads attributes
by its committed source file ID, so its reply always represents the inode that
the hardlink references. If another mount deletes the new entry before the
reply, the ID lookup finds no row and FOD returns `EIO`; it cannot return
attributes belonging to a replacement at the same path.

The ID-based metadata parser now receives root identity explicitly. An initial
regression exposed that using an empty path as a sentinel normalized it to `/`
and incorrectly assigned the root inode to new directory entries. The hardlink
case also exposed that registering a second name overwrote the local reverse
inode-to-path mapping; registration now preserves the existing path for shared
inodes, and removing that path promotes another locally registered alias.

`test_create_write_ownership.py` pauses each callback after commit, replaces the
path from a second mount, and checks that the original callback does not return
the replacement's identity. Its hardlink case checks that the returned alias
uses the source inode even after the other mount replaces the alias. The full
test passed on FOD 3.4.452 in `release-lto` against disposable PostgreSQL 16.15
with `BLCKSZ=32KiB`, Linux 6.17.0-41-generic and libfuse3 3.18.3; the temporary
database was dropped after the run.

### `open()` after concurrent unlink and path replacement

`open()` first resolved the requested inode and checked its access, then
resolved the path again to obtain a backing `file_id`. A second mount could
unlink that inode and create a replacement between those steps. The callback
could then attach a handle and open lease for the replacement while the FUSE
request still named the old inode. `open()` now takes the backing ID from the
already validated attributes. PostgreSQL lease registration serializes with
unlink/purge and returns `ENOENT` when that exact file ID has been removed.

`test_create_write_ownership.py` pauses `open()` after inode validation, replaces
the name from a second mount, then resumes the request. It verifies `ENOENT`
and unchanged replacement bytes. The full test passed on FOD 3.4.453 in
`release-lto`, Linux 6.17.0-41-generic, libfuse3 3.18.3 and PostgreSQL 16.15
with `BLCKSZ=32KiB`; the temporary test database was dropped afterward.

### `poll()` for an open file after remote unlink

The `poll()` callback received a file handle but looked up attributes through
the inode's former pathname. When another mount unlinked that name, polling the
still-open descriptor returned `POLLERR`, even though `pread()` continued to
read the retained inode. The callback now resolves attributes by the handle's
`file_id`. Regular files report readable at EOF as well, where a read returns
zero bytes.

`test_poll.py` reproduced `POLLERR` (mask `8`) on the old implementation. Its
mounted checks now cover empty-file EOF readiness, the open-unlinked descriptor
and the read-only mount; the full test passed on FOD 3.4.454 with PostgreSQL
16.15 `BLCKSZ=32KiB` and libfuse3 3.18.3.

### Parent inode and directory ID resolution

`parent_entry_id_for_inode()` fetched attributes, checked their FUSE inode,
then issued a second `resolve_path()` to obtain the directory ID. A second
mount could replace the path between those queries; the second query would
return the replacement's ID even though the request referred to the old parent
inode. The helper now uses the directory ID carried by the same primary-resolved
attributes row that passed the inode check. `lookup()` had a related race: after
validating a parent it resolved the child by a reconstructed path, which could
select a child in a replacement directory. It now queries the directory entry
by the verified PostgreSQL directory ID and name, then loads attributes using
the returned entry kind and ID. Parent ACL checks use the verified attributes.
Later PostgreSQL mutation transactions still validate and serialize against
that exact directory ID.

`test_create_write_ownership.py` pauses `mkdir()` and `lookup()` after parent
validation, removes and recreates the parent through another mount, and adds a
child only to the replacement before resuming. Both stale requests return
`ENOENT`; the mkdir case leaves the replacement empty. The full test passed on
FOD 3.4.455 in `release-lto`, PostgreSQL 16.15
`BLCKSZ=32KiB`, Linux 6.17.0-41-generic and libfuse3 3.18.3.

### `unlink` journal identity after path replacement

After the unlink transaction committed, the callback looked up `file_id` again
through the old pathname for its journal event. Another mount could create a
replacement first, causing the old unlink event to point at the new file. The
callback now uses the committed `FileUnlinkOutcome` and the prevalidated parent
ID. It records the file ID only when the original row remains (hardlink
promotion/removal or deferred unlink); a purged file is recorded with
`id_file=NULL`.

`test_create_write_ownership.py` pauses the callback between the unlink commit
and journal insertion, creates a replacement through another mount, and
verifies exactly one journal row records the unlink with no replacement file
ID. The full test passed on FOD 3.4.456, PostgreSQL 16.15 `BLCKSZ=32KiB`, Linux
6.17.0-41-generic and libfuse3 3.18.3.

### `rename` source inode after metadata read failure

The callback used a path-based attribute read only to obtain the source FUSE
inode for local path-cache movement. It discarded read errors and substituted
`ROOT_INO`. If the namespace transaction then succeeded, the cache could lose
the source inode mapping and map the destination path to root. The callback now
fetches attributes by the entry ID returned by the authoritative source
resolution and returns the read error before starting the rename transaction.

`test_create_write_ownership.py` injects an attribute-read failure for one
source path, then requires `EIO`, unchanged source/destination bindings and an
unchanged source inode. The full mounted `test_create_write_ownership.py`
passed on FOD 3.4.457, PostgreSQL 16.15 `BLCKSZ=32KiB`, Linux
6.17.0-41-generic and libfuse3 3.18.3; its disposable database was dropped.

### `link()` source namespace binding

The callback resolved a source path against primary and checked that its
attributes matched the FUSE inode, but then passed only the backing file or
symlink ID to the mutation. Between those steps, another mount could remove or
replace the source name while another hardlink kept the backing row alive.
The transaction could therefore create a new alias from a source binding that
no longer existed.

The FUSE path now passes the resolved source kind, entry ID, parent ID and name
to PostgreSQL. The transaction takes source and target namespace advisory
locks in sorted order (and the file lifecycle lock for regular files), then
requires that exact binding still references the expected backing inode before
inserting the destination alias. This shares lock ordering with rename and
unlink. `hardlink_creation_rejects_a_removed_source_namespace_binding` uses a
temporary schema-38 database and verifies both stale regular-hardlink and
symlink-hardlink source rows are rejected without changing link counts; valid
current bindings are accepted. It passed against PostgreSQL 16.15 after
checking `--with-blocksize=32` and live `block_size=32768`.
The full 72-test `pg_query` suite also passed on that fresh schema-38 database.

The mounted cross-host removal/replacement race has not yet been run. The
client builder compiled the callback in `release-lto` against the repository's
pinned libfuse 3.18.3. The local host reports libfuse 3.14.0, below FOD's
minimum 3.18.2, so mounted race coverage still needs a capable test host.

### Strict single-row PostgreSQL result decoding

The former `fetch_first_row_texts()` helper could turn a SQL NULL into an empty
string and silently ignore rows after the first. Replace it with
`fetch_optional_single_row_texts()`: absence is represented explicitly, while
NULL fields, zero-column results and duplicate rows are errors. All single-row
callers now use the strict helper, including parent permission/ACL checks,
file metadata, link counts, xattr authorization and unlink promotion. The
first-column helper now requires exactly one result column and rejects NULL
values; callers use it for directory entry names and orphan-purge file IDs.
`optional_row_decoder_rejects_null_and_multiple_rows` checks valid and absent
results plus NULL, duplicate-row, zero-column and invalid first-column shapes;
it passed against PostgreSQL 16.15 with `--with-blocksize=32` and live
`SHOW block_size=32768`. FOD 3.4.478 also makes the general multirow
`fetch_rows_text()` decoder reject NULL fields and zero-column results while
preserving valid empty strings. Its nullable consumers use explicit
`COALESCE`; schema-backed metadata is non-NULL, and test cleanup filters the
nullable data-object ID. `postgres_row_decoders_reject_null_and_malformed_shapes`
passed on the same verified 32 KiB PostgreSQL build. FOD 3.4.479 extends exact
column-count checks to specialized xattr, file-size and binary block readers,
including zero-row results. The configured admin database is schema 31, so the
first test attempt correctly stopped before assertions. A disposable schema-38
admin database was initialized, all 72 `pg_query` tests passed serially on the
verified PostgreSQL 16.15 `BLCKSZ=32KiB` build, and the disposable database was
dropped afterward. FOD 3.4.480 extends exact-column validation to the remaining
optional metadata readers and attribute blobs; its row-decoder regression now
checks mismatched shapes even when a query returns zero rows. That regression
and all 72 `pg_query` tests passed against the verified PostgreSQL 16.15
`BLCKSZ=32KiB` server.
FOD 3.4.481 moves all remaining production optional-row readers to the exact
column-count decoder, covering parent permission/setgid/ACL metadata, inode
authorization, xattr owner and existence queries, file purge state, and
symlink promotion. The generic wrapper is now test-only. The focused decoder
regression and all 72 `pg_query` tests passed against an isolated PostgreSQL
16.15 database after confirming `--with-blocksize=32` and runtime
`SHOW block_size=32768`; the temporary database was dropped.
FOD 3.4.482 replaces the manual lock-range lease decoder with an exact
five-column multirow decoder that rejects SQL NULLs. The query explicitly
coalesces nullable `range_end`, preserving the empty-field encoding for an
open-ended lock. The row-decoder regression and all 10 `lock_manager` tests
passed on the verified 32 KiB PostgreSQL build; the disposable database was
dropped.
FOD 3.4.483 consolidates six namespace insert `RETURNING` decoders. They now
require one row, one non-NULL column, and a positive ID, rather than accepting
the first cell from a malformed or duplicate result. The decoder regression
and all 72 `pg_query` tests passed on the verified PostgreSQL 16.15
`BLCKSZ=32KiB` build using an isolated schema-38 database.
FOD 3.4.484 applies the same exact single-positive-ID contract to the three
staged index upserts that return a source, scan-run or import-plan ID. A pure
unit regression checks empty, duplicate, multicolumn, zero and malformed
results.
FOD 3.4.485 makes post-conflict namespace confirmation queries reject
duplicate rows and enforce exact column counts; the prior `LIMIT 1` behavior
could hide duplicate bindings. Startup probes now also require exactly one
row with the expected number of columns. The decoder regression and all 72
`pg_query` tests passed against a disposable PostgreSQL 16.15 database after
checking `--with-blocksize=32` and live `SHOW block_size=32768`; the database
was dropped after the run.
FOD 3.4.486 extends exact row cardinality and shape checks to the locked inode
and ACL lookups in transactional chmod and timestamp authorization. Duplicate
or malformed authorization metadata now aborts the transaction instead of
being interpreted from its first row. All 72 `pg_query` tests passed on an
isolated current-schema database after confirming PostgreSQL 16.15's
`--with-blocksize=32` build and live `SHOW block_size=32768`; the temporary
database was dropped.
FOD 3.4.487 removes the arbitrary first-row selection from directory-entry
metadata lookup. It now reports an error when multiple namespace kinds occupy
the same parent/name; the duplicate-symlink regression covers this resolver.
All 72 `pg_query` tests passed against a disposable current-schema PostgreSQL
16.15 database after confirming `--with-blocksize=32` and live
`SHOW block_size=32768`; the database was dropped afterward.
FOD 3.4.488 changes three indexer readers from silently skipping short SQL
rows to rejecting any result row whose width differs from the query projection.
This prevents malformed hash-candidate, duplicate-set and import-plan results
from appearing complete.
FOD 3.4.489 extends exact projection-width checks across indexer source,
catalogue, snapshot, hash, import-plan and cleanup decoders. Aggregate and
single-object queries now require exactly one row and the expected column
count; missing and duplicate rows are reported instead of silently selecting
the first. All 35 `fod-indexer` unit tests passed, alongside `cargo check`,
formatting and `make test-version`.
FOD 3.4.490 makes the directory-entry metadata decoder require exactly five
columns and makes `rename()` namespace resolution reject unknown kinds,
non-positive IDs, extra/multiple rows, and malformed nonempty file IDs. The
previous `parse::<u64>().ok()` silently converted a corrupt `file_id` into
`None`, potentially confusing malformed metadata with a directory or symlink.
Both pure decoder regressions pass. The full hotpath library suite reached 90
passing tests, but four tests that connect to the configured shared database
failed its existing schema-31/version-38 guard; no shared schema was changed.
These unit tests do not establish PostgreSQL runtime or 32 KiB integration
coverage for this change. The host does not have
`/opt/postgresql-custom/bin/pg_config`; its `/usr/bin/pg_config` is Ubuntu
PostgreSQL 17.10 and does not report `--with-blocksize=32`. No live
`SHOW block_size` check was made, so this host's stock build is not evidence
for the FOD 32 KiB target.
FOD 3.4.491 makes the post-unique-violation `create` confirmation queries
reject malformed or non-positive PostgreSQL IDs instead of treating them as
an absent/mismatched object. This covers files, special files, directories,
symlinks and both hardlink forms; a valid row whose metadata differs still
returns “not confirmed” and preserves the original uniqueness error. The pure
positive-ID decoder test passes. PostgreSQL integration remains unverified in
this environment for the schema and 32 KiB build reasons recorded above.
FOD 3.4.492 requires the hardlink-promotion decoder to receive exactly the
three projected columns, and requires endpoint-role probes to return exactly
one row and one column. The endpoint result-shape regression was added, but
the FUSE crate test could not compile here: installed libfuse3 is 3.14.0 while
the project requires at least 3.18.2. No live PostgreSQL integration was run.
FOD 3.4.493 requires the file-copy metadata query to return exactly its two
projected columns. Expired-session pruning now decodes `DELETE ... RETURNING`
through the exact one-column, non-NULL row decoder instead of checking only
whether the result contained rows. The PostgreSQL row-decoder regression,
extended to require exactly three hardlink-result columns, passed against the
configured PostgreSQL 16.15 FOD container after confirming
`pg_config --configure` includes `--with-blocksize=32` and runtime
`SHOW block_size=32768`. That test uses SELECT-only decoder fixtures and does
not change the shared schema-31 database. Operation-level PostgreSQL tests for
copy metadata and expired-session pruning were then added in FOD 3.4.494 and
passed against isolated schema-38 databases on the same verified PostgreSQL
16.15 `BLCKSZ=32KiB` server. The adoption regression injects a negative stored
file size and verifies that no destination metadata changes; the session test
confirms expired-row removal, active-row preservation and both return values.
Temporary databases are created/dropped through the `postgres` database, so
the fixtures do not depend on the shared FOD database meeting the current
schema floor. The schema-31 shared database was left unchanged.
FOD 3.4.495 routes PostgreSQL text booleans through one strict decoder. It
rejects unknown values in lease-presence, write-fencing, advisory-lock,
namespace-conflict, rename-cycle, schema-readiness/upgrade-admission, and
idempotency results instead of mapping them to `false`. This closes a
fail-open path where malformed active-lease state could allow deferred unlink
to purge an inode or allow a write to proceed without a lease. The pure decoder
test and hotpath compile pass; these checks require no database, so no claim of
new PostgreSQL integration coverage is made.
FOD 3.4.496 extends strict PostgreSQL boolean decoding to the indexer. In
particular, cleanup now aborts if advisory-lock, open-handle, or write-lease
results are malformed; it can no longer interpret an unknown value as absence
of an active lease and proceed with deletion. Catalog snapshot schema checks
also reject unexpected result cardinality. All 36 indexer tests pass without
connecting to PostgreSQL.
FOD 3.4.497 hardens `mkfs` scalar-boolean queries to require one non-NULL
result cell and a recognized boolean value; this includes schema-existence and
upgrade checks plus the schema-upgrade advisory lock. FUSE runtime-override
table discovery now fails on malformed boolean output instead of treating the
table as absent. The hotpath parser test and `mkfs` unit targets pass. The
content-generation integration tests stopped at their dedicated-database-name
guard (`foddbname`) without connecting. The FUSE crate could not be compiled
because the installed libfuse3 is 3.14.0 and the project requires 3.18.2 or
newer; no compilation claim is made for that crate.
FOD 3.4.498 factors the `mkfs` boolean-cell validation so its pure regression
also rejects zero/multiple rows, extra columns, SQL NULL, a missing value
pointer and malformed text. This verifies the whole scalar-result contract
without relying on the live PostgreSQL endpoint.
FOD 3.4.499 applies the shared positive-ID decoder to both PostgreSQL
file-to-data-object metadata readers. A zero or negative backing-object ID now
fails as a database result error rather than entering later copy/CRC or
reference-count logic; the existing pure positive-ID regression and hotpath
crate check pass.
FOD 3.4.500 extends the same positive-ID validation to newly inserted,
copied and request-token-resolved data objects, including file and special-file
creation, and to the data-object ID read during rename replacement. A malformed
zero ID can no longer be propagated as a valid backing object by these paths.
FOD 3.4.501 routes the remaining file purge and rename-replacement open-lease
booleans through the strict PostgreSQL boolean decoder. Unknown values now fail
the transaction instead of being interpreted as “no hardlink” or “no open
lease”.
