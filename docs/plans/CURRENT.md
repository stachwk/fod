# FOD current implementation plan

Status: 2026-09-07.

This file is the compact maintained implementation plan. It contains only work
that is still current enough to direct the next change. Completed execution
plans are retained under [`../history/`](../history/) and are not allowed to
compete with this file for current priority.

For current implemented behavior use [`../CURRENT_STATE.md`](../CURRENT_STATE.md).
For long-term direction use [`../../ROADMAP.md`](../../ROADMAP.md). `TODO.md`
remains a mixed follow-up/archive record and may contain historical sections
whose wording predates later architectural decisions.

## P1 — replica read-path efficiency

The next measured performance target is the PostgreSQL-backed replica read path.
Preserve the strict read-only correctness boundary while reducing avoidable work
behind one FUSE read callback.

Current measurement boundary:

- treat 512 KiB as the effective measured FUSE read-callback ceiling until a
  newer negotiation/profile proves otherwise;
- keep the FOD logical storage block at 4 KiB;
- do not infer write sizing from read sizing — read and write tuning remain
  separate;
- preserve primary-unreachable replica-read validation, zero PostgreSQL write
  attempts on the read-only path, role validation and WAL/replay-LSN safety.

FOD 3.4.12 makes the existing profile data directly comparable per callback.
`scripts/perf/extract_primary_replica_profile.sh` compact output now includes:

- FUSE `read_block_map`, repository fetch, assembly and reply timing;
- prepared `fod_fetch_block_range` calls, rows, payload bytes, failures and SQL
  time;
- matching PostgreSQL result-decode time/bytes;
- PostgreSQL operations per admitted read callback;
- fetch calls/bytes/time per callback and rows/bytes per range fetch;
- the remaining operation count after subtracting range-fetch calls.

The extractor uses existing `FOD_PROFILE_IO=1` boundary telemetry; 3.4.12 does
not add runtime instrumentation or change the read path.

Next measurement step:

1. run the strict physical-replica matrix for 4 KiB, 64 KiB and 512 KiB reads;
2. extract the matrix artifact with:

   ```bash
   scripts/perf/extract_primary_replica_profile.sh <matrix-artifact-dir>
   ```

3. compare the `replica-read` rows, especially `pg_operations_per_callback`,
   `fetch_calls_per_callback`, `fetch_bytes_per_call`,
   `read_block_map_us_per_callback`, `repo_fetch_block_range_us_per_callback`,
   `pg_fetch_us_per_callback`, `pg_decode_us_per_callback` and
   `non_fetch_operation_count`;
4. identify one avoidable round trip, duplicate lookup or transport/query-shape
   cost from that evidence;
5. make one narrow runtime change;
6. rerun the same matrix and preserve the strict read-only/WAL gates.

A focused run is:

```bash
FIO_BLOCK_SIZES='4k 64k 512k' \
FIO_FILE_SIZE=128M \
make test-fio-primary-write-replica-read-matrix
```

Do not change the storage format, quota model, write request default or unrelated
routing policy in the same optimization commit.

## P2 — remaining multi-endpoint hardening

The base role-aware routing design is already implemented: startup endpoint role
selection, runtime primary failover for HA/proxy entrypoints representing one
authoritative PostgreSQL cluster, WAL-gated replica reads, replica scoring,
promotion validation and process-local generation fencing all have delivered
runtime support.

Remaining work must be phrased as a specific hardening gap rather than reopening
the original broad multi-endpoint design. In particular:

- never present independent writable PostgreSQL primaries as one safe
  multi-primary filesystem;
- keep authoritative writes/control/lease work on a verified writable primary;
- keep replica reads behind role and replay-consistency checks;
- treat external/cross-process fencing or fairness as separate work only when
  its ownership and acceptance tests are explicit.

## Delivery rule

Each implementation commit uses the next sequential FOD version, updates the
relevant current documentation/tests, stays on `main`, and uses:

```text
FOD X.Y.Z: <English description>
```

After every commit compare it with its parent using `git diff HEAD~1..HEAD` or
`git show` and inspect the complete change for accidental files, missing updates,
regressions and scope drift.

## Historical plans

The former 2026-08-26 action plan and the completed block/write/replay plans are
kept under [`../history/`](../history/) as implementation evidence. They explain
how earlier states were reached but no longer define the next task.
