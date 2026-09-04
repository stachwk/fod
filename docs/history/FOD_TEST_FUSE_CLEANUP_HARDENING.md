# FOD test FUSE cleanup hardening

Status: test-harness correction. This change does not alter the FOD storage format, PostgreSQL schema, FUSE runtime implementation, Rust toolchain policy, or production build profile.

## Problem

A failed or recently completed Rust FUSE test can occasionally leave a temporary mount such as:

```text
/tmp/fod-rust-fuse-<pid>-<suffix>-<test>/mount
```

The local database restore guard correctly refuses destructive restore while such a FOD mount remains active. This previously required a manual `fusermount3 -u` before rerunning `QNAP=0 make test-all`.

## Correction

`scripts/fod-test-fuse-cleanup.sh` only considers FUSE mountpoints matching:

```text
/tmp/fod-rust-fuse-*/mount
```

For each matching test mount it:

1. retries normal unmount with `fusermount3`, `fusermount`, and `umount` when available;
2. falls back to lazy unmount if the mount remains present;
3. waits for the mountpoint to disappear;
4. removes the temporary test workspace only after unmount;
5. fails if the test mount still exists.

It never selects arbitrary FOD mounts or non-test paths.

`GNUmakefile` runs the cleanup before `test-db-restore-local`, includes a no-root policy regression in `test-all`, and performs a final cleanup check after `test-all` and `test-all-full`.

## Validation

The lightweight policy test uses fake `findmnt`, `mountpoint`, and `fusermount3` commands. It verifies that normal unmount is attempted first, lazy unmount is used as fallback, the simulated stale mount is removed, and an unrelated `/tmp/not-fod/mount` entry is never touched.

Recommended local validation:

```bash
make test-fuse-test-cleanup-policy
make test-fuse-test-cleanup
QNAP=0 make test-all
findmnt -rn -o TARGET,FSTYPE | grep '/tmp/fod-rust-fuse-' || true
```
