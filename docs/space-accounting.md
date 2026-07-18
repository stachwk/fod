# FOD space accounting

FOD exposes two different accounting views because file data is stored in PostgreSQL and one data object may be referenced by more than one file.

## Per-file values

`stat(2)` and tools built on it use these fields:

- `st_size` is the logical file length in bytes. This is the value shown by `ls -l` and by `du --apparent-size`.
- `st_blocks` is expressed in POSIX 512-byte units, regardless of the FOD storage block size. Normal `du` multiplies this value by 512.

FOD reports per-file `st_blocks` from the payload rows referenced by the file's data object, rounded up to 512 bytes. Sparse and zero-only ranges without payload rows do not contribute blocks. Multiple independent files referencing one shared data object each expose that object's allocation, while hardlinks retain one inode and are normally counted once by `du`.

Empty regular files report zero blocks. Directory entries retain a minimal non-zero block count.

## Filesystem-wide values

`df` reads FUSE `statfs` values. FOD calculates used filesystem bytes from persisted payload rows: block-row count multiplied by the configured block size plus extent `used_bytes`. Missing rows for sparse or zero-only logical ranges do not consume persisted payload bytes. Each row is counted once, including when one data object is referenced by more than one filesystem entry.

When `pg_visible_path` is configured, free blocks are additionally capped by the host filesystem's `f_bavail` value. An error while reading PostgreSQL accounting data or the configured visible path is returned as an I/O error instead of being presented as an empty filesystem.

`max_fs_size_bytes` is both the reported `statfs` capacity ceiling and a transactional PostgreSQL payload quota. The canonical limit is the `config.max_fs_size_bytes` row created by `mkfs.fod`; a value of zero disables the quota. Each refreshed `statfs` snapshot reads this canonical row together with persisted usage and active reservations, so a running mount does not retain a stale startup limit.

Every transaction that creates or replaces block or extent payload takes the same PostgreSQL transaction-scoped advisory lock, calculates the post-write persisted payload total, and commits only when that total does not exceed the limit. This serializes quota decisions across independent FOD mounts using the same database. Exceeding the limit rolls back the complete payload transaction and is returned through FUSE as `ENOSPC`.

Copy operations that must create payload reserve their rounded target capacity in PostgreSQL before reading or modifying the destination. Active reservations participate in the same quota total and temporarily reduce the free blocks reported by `statfs`, so concurrent copy jobs cannot all claim the same free capacity. Immediately before persistence, the payload transaction rechecks the reserved amount against current usage and competing reservations, then renews the reservation for another hour. This lets long-running copies continue when capacity is still available without weakening admission safety after an expired reservation has been claimed by another job. The reservation is released after the copy succeeds or fails; stale reservations expire after one hour as crash recovery. Exact whole-object adoption does not reserve capacity because it reuses an existing data object without creating payload rows.

There is no universal equality or ordering between `df` and the sum reported by `du`:

- sparse or zero-only logical ranges can increase `st_size` without requiring equivalent persisted payload bytes;
- one stored data object may be referenced by more than one filesystem entry without duplicating its payload;
- padded storage blocks or extents can make stored payload usage larger than the logical allocation of very small or partial files.

A difference between `ls`, `du`, and `df` is therefore not by itself corruption. Persisted payload bytes are application-level accounting and are not the physical PostgreSQL footprint: TOAST compression, tuple metadata, indexes, dead tuples, and relation free space can make the on-disk relation size different.

## Diagnostic capture

Run the repository helper against a mounted FOD filesystem:

```bash
scripts/fod-space-accounting.sh /path/to/fod/mount
```

The report includes byte-precise `df`, allocated and apparent `du`, and per-file `stat` values. When a PostgreSQL connection is available through standard `PGHOST`, `PGPORT`, `PGDATABASE`, `PGUSER`, and `PGPASSWORD` variables, it also reports payload-column totals and the physical size of the two payload relations through `pg_total_relation_size()`.
