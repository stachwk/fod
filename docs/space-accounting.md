# FOD space accounting

FOD exposes two different accounting views because file data is stored in PostgreSQL and one data object may be referenced by more than one file.

## Per-file values

`stat(2)` and tools built on it use these fields:

- `st_size` is the logical file length in bytes. This is the value shown by `ls -l` and by `du --apparent-size`.
- `st_blocks` is expressed in POSIX 512-byte units, regardless of the FOD storage block size. Normal `du` multiplies this value by 512.

FOD reports per-file `st_blocks` from the logical file length, rounded up to 512 bytes. This is deliberate: a shared PostgreSQL data object has no unambiguous exclusive physical owner, so assigning its complete physical payload to every referencing file would double-count it. Normal `du` therefore describes per-file logical allocation rather than the exclusive PostgreSQL footprint.

Empty regular files report zero blocks. Directory entries retain a minimal non-zero block count.

## Filesystem-wide values

`df` reads FUSE `statfs` values. FOD calculates used filesystem bytes from the payload actually stored in `data_blocks` and `data_extents`. Missing rows for sparse or zero-only logical ranges do not consume stored payload bytes. Each stored row is counted once, including when one data object is referenced by more than one filesystem entry.

There is no universal equality or ordering between `df` and the sum reported by `du`:

- sparse or zero-only logical ranges can increase `st_size` and per-file logical allocation without requiring equivalent stored payload;
- one stored data object may be referenced by more than one filesystem entry without duplicating its payload;
- padded storage blocks or extents can make stored payload usage larger than the logical allocation of very small or partial files.

A difference between `ls`, `du`, and `df` is therefore not by itself corruption. The values must be interpreted according to the logical per-file and stored filesystem-wide contracts above. The term `stored payload` does not imply content deduplication; it means the bytes physically present in FOD's PostgreSQL payload tables.

## Diagnostic capture

Run the repository helper against a mounted FOD filesystem:

```bash
scripts/fod-space-accounting.sh /path/to/fod/mount
```

The report includes byte-precise `df`, allocated and apparent `du`, and per-file `stat` values. When a PostgreSQL connection is available through standard `PGHOST`, `PGPORT`, `PGDATABASE`, `PGUSER`, and `PGPASSWORD` variables, it also reports logical file totals and stored block/extent payload totals.
