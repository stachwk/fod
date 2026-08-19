# FOD 3.2.85 — 512 KiB FUSE I/O negotiation plan

## Goal

Make 512 KiB the default negotiated size for large FUSE write and readahead
operations without changing the FOD on-disk/storage block format and without
turning small/random reads into forced 512 KiB reads.

## Invariants

The following behavior stays unchanged in 3.2.85:

- schema/storage `block_size` remains normally 4096 bytes (4 KiB);
- `persist_buffer_chunk_blocks = 128` remains the base write batch, which is
  512 KiB at a 4 KiB block size;
- `read_ahead_blocks = 4` remains unchanged;
- `sequential_read_ahead_blocks = 8` remains unchanged;
- `read_block_map -> repo_fetch_block_range` is not changed in this version;
- small direct/random I/O is not artificially expanded to 512 KiB.

## 3.2.85 implementation

1. Add startup-only configuration keys:
   - `fuse_max_write_bytes = 512KiB`;
   - `fuse_max_readahead_bytes = 512KiB`.
2. Preserve `FOD_*` environment override precedence with:
   - `FOD_FUSE_MAX_WRITE_BYTES`;
   - `FOD_FUSE_MAX_READAHEAD_BYTES`.
3. Validate both values as positive byte sizes that fit in `u32`.
4. During `Filesystem::init()`, request the configured values through
   `KernelConfig::set_max_write()` and `KernelConfig::set_max_readahead()`.
5. If `fuser` or the kernel returns a smaller supported value, retry/accept the
   returned value rather than rejecting the mount. A kernel readahead limit of
   zero is accepted as effective zero.
6. Replace the old `max_write=unavailable max_readahead=unavailable`
   compatibility diagnostics with requested/effective values.
7. Add regression tests for size parsing and fallback selection.

## Expected startup diagnostics

Normal 512 KiB negotiation should include:

```text
FOD FUSE negotiated: requested_max_write=524288 effective_max_write=524288 requested_max_readahead=524288 effective_max_readahead=524288 ...
```

If the kernel offers less readahead, the log must show the requested value and
the lower effective value explicitly.

## Validation before performance conclusions

Functional validation for 3.2.85:

```bash
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
make test-version
```

Mount validation should confirm the negotiated line in the FOD startup log and
verify that the filesystem still mounts when the kernel limits readahead below
512 KiB.

## Validation result and 3.2.86 cleanup

The first 3.2.85 validation run confirmed that the workspace compiles and the
new FUSE compatibility unit tests pass. `cargo test --workspace --locked`
reached the PostgreSQL-backed integration benchmarks, which failed because no
PostgreSQL server was listening on `127.0.0.1:5432`; this is an environment
precondition failure, not a FUSE negotiation assertion failure.

The same run also showed that the 3.2.85 commit was not `rustfmt` clean. FOD
3.2.86 is therefore a behavior-neutral cleanup that applies the exact
`cargo fmt` output and keeps the 512 KiB negotiation logic unchanged.
PostgreSQL-backed integration tests must be repeated with the expected test
database available.

## Performance validation after 3.2.85

Repeat the same workload with application request sizes:

- 4 KiB;
- 64 KiB;
- 512 KiB.

For every run record at least:

- requested/effective FUSE write and readahead limits;
- throughput and latency;
- FUSE callback request-size distribution;
- PostgreSQL query/transaction counts;
- PostgreSQL payload bytes and persist batch sizes.

Only after these measurements should the next optimization consider combining
block fetches inside a large callback, including a possible
`read_block_map -> repo_fetch_block_range` change. That optimization is outside
3.2.85.

## Aktualizacja FOD 3.2.87 - split write buffering

Benchmark primary-write -> replica-read wykazal, ze w trybie `direct_io`
logiczny write 512 KiB moze zostac technicznie podzielony na niewyrownane
callbacki FUSE. FOD nie interpretuje juz samej granicy takiego callbacku jako
powodu do natychmiastowego persistence przy pojedynczym `fh`.

Szczegoly, dowod pomiarowy i kryteria regresji opisuje
`docs/FOD_3_2_87_FUSE_SPLIT_WRITE_BUFFERING.md`.
