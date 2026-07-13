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

There is no universal equality or ordering between `df` and the sum reported by `du`:

- shared or deduplicated data objects can make filesystem-wide payload usage smaller than the summed per-file logical allocation;
- padded storage blocks or extents can make stored payload usage larger than the logical allocation of very small or partial files;
- sparse logical ranges can increase `st_size` without requiring an equivalent stored payload.

A difference between `ls`, `du`, and `df` is therefore not by itself corruption. The values must be interpreted according to the logical per-file and unique filesystem-wide contracts above.

## Diagnostic capture

Run the repository helper against a mounted FOD filesystem:

```bash
scripts/fod-space-accounting.sh /path/to/fod/mount
```

The report includes byte-precise `df`, allocated and apparent `du`, and per-file `stat` values. When a PostgreSQL connection is available through standard `PGHOST`, `PGPORT`, `PGDATABASE`, `PGUSER`, and `PGPASSWORD` variables, it also reports logical file totals and unique block/extent payload totals.
