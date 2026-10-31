# Hard links to symlink inodes

Status: design for a Linux compatibility gap; implementation remains pending.

Linux `link()` can add a directory entry for the symlink inode itself. POSIX
leaves the behavior when the source names a symlink implementation-defined.
FOD currently rejects this source with `ENOENT`. A local probe on Linux
6.17.0-41-generic created two names with the same `lstat().st_ino`, symlink
type, and `st_nlink == 2`.

## Required model

Keep one canonical `symlinks` row as the inode object and add a
`symlink_hardlinks` table for additional directory entries. An alias row points
to the canonical `id_symlink`; it does not copy the target, metadata, inode
seed, or extended-attribute owner. The canonical row remains the inode's stable
identity and one of its names. A lookup result must distinguish the directory
entry identity from the underlying symlink inode identity instead of
overloading one `entry_id` for both.

The alias table needs a `BIGSERIAL` entry ID, the canonical symlink ID, nullable
parent directory ID for root entries, and a name constrained to at most 255
bytes. Add root and nested unique name indexes and a parent lookup index. The
schema belongs in a new migration and `migrations/base_schema.sql`; do not
rewrite a published migration. Preserve the project's 32 KiB PostgreSQL build
when validating index limits and migrations.

## Transaction and inode invariants

- PostgreSQL remains authoritative for link, unlink, rename, replacement,
  parent authorization, and cross-host conflict order. Lock the canonical
  symlink inode and involved parent/name keys in one transaction.
- A symlink inode has exactly one canonical `symlinks` row and zero or more
  alias rows. All names resolve to that inode's `inode_seed`, target, metadata,
  and xattr owner.
- `st_nlink` is one plus the number of alias rows. Aliases increase directory
  entry counts but do not create another inode for `statfs` accounting.
- Removing a noncanonical alias deletes only that alias. Removing the
  canonical name while aliases remain promotes one alias by moving its parent
  and name into the canonical row and deleting the promoted alias, in the same
  transaction. Removing the final name deletes the inode row and its owned
  metadata according to existing symlink cleanup rules.
- Renaming an alias changes that alias row. Renaming the canonical name moves
  the canonical row. When rename replaces a canonical symlink name, promote a
  surviving alias before removing the old inode; when it replaces an alias,
  remove only that directory entry. Same-inode rename remains a no-op.
- Creating a link to a symlink links the inode itself; it must not follow the
  target. Preserve existing directory-source rejection and destination errno
  precedence.

## Integration points

Update primary-backed path resolution, direct symlink ID lookup, child lookup,
directory metadata and name snapshots, inode-to-path registration/invalidation,
`getattr`, `readlink`, xattr ownership, `link`, `unlink`, `rename`, directory
emptiness, and stat/link-count accounting. Keep the request's entry identity
through namespace mutation while using the canonical symlink ID for inode data.
Audit every SQL query that unions `files`, `hardlinks`, `directories`, and
`symlinks`; aliases must appear exactly once in lookup and readdir results.

## Acceptance coverage

- Mounted `ln -P` (and `os.link(..., follow_symlinks=False)`) succeeds for a
  symlink and creates the same inode with `nlink == 2`; `readlink()` through
  both names returns the same target.
- Changing shared symlink metadata or xattrs through one name is visible via
  the other name; aliases retain one inode and one xattr owner.
- Cover unlink and rename of both canonical and alias names, canonical-name
  promotion, destination replacement, and final-link removal.
- Use two mounts on the same PostgreSQL primary to exercise link/unlink/rename
  races and stale inode requests. Validate namespace order and cleanup from
  PostgreSQL, not process-local maps.
- Verify root and nested readdir results, directory emptiness, and
  `statfs` inode counts. Test exact Linux errno results for missing sources,
  existing destinations, read-only mounts, and permission-denied parents.
- Before each mounted run, verify the selected PostgreSQL build with its custom
  `pg_config --configure` and live `SHOW block_size`; the default target is
  PostgreSQL 16.15 `--with-blocksize=32` / `32768` bytes.
