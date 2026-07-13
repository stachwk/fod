# FOD space accounting

FOD exposes two different accounting views because file data is stored in PostgreSQL and one data object may be referenced by more than one file.

## Per-file values

`stat(2)` and tools built on it use these fields:

- `st_size` is the logical file length in bytes. This is the value shown by `ls -l` and by `du --apparent-size`.
- `st_blocks` is expressed in POSIX 512-byte units, regardless of the FOD storage block size. Normal `du` multiplies this value by 512.

FOD reports per-file `st_blocks` from the logical file length, rounded up to 512 bytes. This is deliberate: a shared PostgreSQL data object has no unambiguous exclusive physical owner, so assigning its complete physical payload to every referencing file would double-count it. Normal `du` therefore describes per-file logical allocation rather than the exclusive PostgreSQL footprint.

Empty regular files report zero blocks. Directory entries retain a minimal non-zero block count.

## Filesystem-wide values

`df` reads FUSE `statfs` values. FOD calculates used filesystem bytes from the payload stored in `data_blocks` and `data_extents`. Each stored row is counted once, even when its data object is referenced by multiple files.

Consequently, the following relationship is expected and is not by itself corruption:

```text
physical payload reported by df <= summed per-file logical allocation reported by du
```

A large difference may be caused by shared or deduplicated data objects, sparse logical ranges, or files whose logical length is larger than their stored payload.

## Diagnostic capture

Run the repository helper against a mounted FOD filesystem:

```bash
scripts/fod-space-accounting.sh /path/to/fod/mount
```

The report includes byte-precise `df`, allocated and apparent `du`, and per-file `stat` values. When a PostgreSQL connection is available through standard `PGHOST`, `PGPORT`, `PGDATABASE`, `PGUSER`, and `PGPASSWORD` variables, it also reports logical file totals and unique block/extent payload totals.
