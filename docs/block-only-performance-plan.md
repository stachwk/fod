# FOD block-only performance plan

## Status

This document is the current performance follow-up after the extent PoC was
retired, legacy extent data was migrated, and the extent runtime/schema paths
were removed in FOD 3.2.71-3.2.73.

The active storage architecture is intentionally simple and block-only:

```text
FUSE write/read
    -> block-oriented write/read state
    -> COPY BINARY staging / set-based block merge
    -> canonical data_blocks
    -> PostgreSQL
```

Do not reintroduce a second payload representation unless a later measured
workload proves that the single `data_blocks` model is insufficient.

## Measured baseline after extent removal

The 2026-08-16 block-only profiles on FOD 3.2.73 changed the next optimization
priority.

### 4 MiB sequential fio

- write: `3220 KiB/s`;
- read: `54.1 MiB/s`;
- `repo_persist_blocks_us=111374`;
- COPY stage: about `54.785 ms`;
- child lookup: about `44.950 ms`;
- `data_blocks` merge: about `37.693 ms`;
- hardlink count: 28 calls / about `1.178 ms` total.

### 128 MiB large-file multiblock (`4M x 32`)

- throughput: `46.06 MiB/s`;
- `repo_persist_blocks_us=2320189`;
- `prepare_persist_rows_from_block_plan_us=19666`;
- file-attribute allocation query with `COUNT(*)` over `data_blocks`:
  1031 calls / about `3570.578 ms` total;
- `data_blocks` merge: about `1237.777 ms`;
- COPY stage: about `1062.855 ms`;
- hardlink count: 1032 calls / about `17.737 ms` total.

The repeated hardlink-count query is therefore not the dominant cost on the
current block-only profiles. The immediate bottleneck is the file-attribute
allocation calculation that repeatedly counts `data_blocks` for the same data
object. On the 128 MiB profile it cost more than COPY plus the block merge
combined.

## 2026-08-16 FOD 3.2.74 update

The call path was:

```text
FUSE read()
    -> entry_attrs_for_ino()
    -> lookup_path()
    -> attrs_for_path()
    -> DbRepo::fetch_path_attrs_blob()
    -> FetchPathAttrsBlobFile SQL
    -> COUNT(data_blocks) for FileAttr.blocks
```

FOD 3.2.74 keeps `FileAttr.blocks` semantics unchanged for real `lookup` and
`getattr`, but moves `read()` to a narrow by-`file_id` metadata query:

```sql
SELECT size, access_date, modification_date, change_date
FROM files
WHERE id_file = $1
```

This avoids the allocated-block COUNT during normal reads while preserving
cross-mount size visibility through PostgreSQL. A pure handle-local size cache
was not used because it would need a stronger invalidation contract for
truncate, writes from another mount, copy-on-write, remount, and replay.

Validation on the FOD 3.2.74 working tree based on commit `5855293`:

| workload | result | allocation-query result |
| --- | --- | --- |
| 4 MiB sequential fio, `FOD_PROFILE_IO=1` | write `3287 KiB/s`, read `90.9 MiB/s`, callbacks `20 / 1024` | old `COUNT(data_blocks)` attr query `7` calls / `1.938 ms`; new metadata query `20` calls / `2.464 ms` |
| 128 MiB sequential fio, `FOD_PROFILE_IO=1` | write `3779 KiB/s`, read `481 MiB/s`, callbacks `516 / 32768` | old `COUNT(data_blocks)` attr query `8` calls / `24.136 ms`; new metadata query `516` calls / `14.523 ms` |
| 128 MiB large-file multiblock, `4M x 32` | `46.97 MiB/s`, callbacks `1024 / 128` | old `COUNT(data_blocks)` attr query `5` calls / `12.109 ms`; new metadata query `1024` calls / `27.215 ms` |

Compared with the 3.2.73 128 MiB large-file profile, the repeated
`COUNT(data_blocks)` path dropped from 1031 calls / about `3570.578 ms` to 5
calls / about `12.109 ms`. The large-file wall throughput stayed essentially
flat (`46.06` to `46.97 MiB/s`), so the next measured bottleneck remains the
write-side COPY plus `data_blocks` merge. The normal 128 MiB fio read improved
from the earlier `109 MiB/s` profile to `481 MiB/s` in this run.

## Corrected optimization order

### Phase 1 — identify and pin the attr/allocation call path

Before changing storage metadata or schema:

1. Locate the exact `FileAttr.blocks` / allocated-payload query and every caller.
2. Record why it is executed roughly 1031 times during the 128 MiB workload.
3. Separate required FUSE attribute refreshes from duplicate repository work.
4. Add focused observability or a regression counter so the number of allocation
   queries is measurable independently of total PostgreSQL statement time.
5. Confirm the current semantic contract for:
   - POSIX `st_blocks` units of 512 bytes;
   - sparse zero blocks, which have no canonical `data_blocks` row;
   - shared `data_object_id` ownership;
   - truncate/shrink/grow;
   - full-object adoption / `copy_file_range`;
   - remount durability;
   - independent mounts using the same PostgreSQL database.

Do not optimize by changing the meaning of `st_blocks` or by counting logical
file size instead of actually allocated canonical payload blocks.

### Phase 2 — eliminate repeated COUNT round trips

The primary objective is to remove `COUNT(data_blocks)` from the repeated
`FileAttr` hot path, not merely make each COUNT slightly faster.

Evaluate solutions in this order:

1. **Reuse already-fetched metadata** if the allocation value can be carried in
   the existing file/data-object metadata result without a second database
   round trip.
2. **Cache allocation by `data_object_id`** only if invalidation remains correct
   for writes, truncate, object adoption/swap, unlink/GC, remount, and changes
   performed by another mount. Prefer the existing bounded metadata-cache
   discipline rather than adding an unrelated cache.
3. **Persist an allocation counter on `data_objects`** if runtime caching cannot
   provide a correct cross-process contract. Maintain it transactionally with
   block persistence/truncate/object-swap operations and update it once per
   logical payload mutation, never once per individual block row.
4. Consider an index/query-plan change only after the number of repeated calls
   has been reduced. An index is not the first answer to 1031 semantically
   repeated queries for the same object.

Avoid PostgreSQL per-row triggers on the `data_blocks` COPY/merge path unless a
measurement proves they are cheaper than transaction-level accounting. They can
turn an attribute-read optimization into a write-amplification regression.

### Phase 3 — correctness gates for the chosen solution

Any implementation must preserve:

- sparse-file `st_blocks` behavior;
- exact allocation after overwrite and truncate;
- correct allocation for shared data objects without filesystem-wide double
  counting;
- full-object adoption / copy-on-write semantics;
- rollback and ambiguous-commit replay behavior;
- two-mount consistency within the documented cache/refresh contract;
- remount durability;
- existing quota and `statfs` semantics.

Add or extend focused tests for at least:

1. dense file allocation;
2. sparse zero ranges;
3. overwrite of an allocated block with zero and zero with non-zero;
4. truncate across a block boundary;
5. shared data object followed by copy-on-write divergence;
6. full-object adoption;
7. remount;
8. two independent mounts observing an allocation change.

### Phase 4 — performance acceptance gate

Re-run the same profiles used to select the bottleneck.

Required measurements:

```text
4 MiB sequential fio, FOD_PROFILE_IO=1
128 MiB large-file multiblock, 4M x 32
PostgreSQL top statements / call counts
repo_persist_blocks_us
COPY stage time
data_blocks merge time
FileAttr allocation-query count and total time
```

Acceptance criteria:

- the repeated `COUNT(data_blocks)` statement is absent from the normal
  `FileAttr` hot path or reduced from about 1031 calls to a small number tied to
  real payload mutations/cache misses rather than FUSE callbacks;
- the 128 MiB attr/allocation SQL time drops materially from the measured
  `~3570 ms` baseline;
- 128 MiB throughput improves without hiding work in another SQL statement;
- 4 MiB sequential performance does not regress materially;
- `repo_persist_blocks_us`, COPY, and merge times are recorded separately so a
  faster metadata path is not confused with a payload-persistence change;
- all correctness gates above pass.

Do not set a new default or introduce schema state from a single noisy sample.
Use repeated measurements when the result is close to the noise floor.

### Phase 5 — next measured bottleneck only

After the attr/allocation path is fixed, profile again before choosing the next
optimization.

Current candidates, in likely order, are:

1. COPY BINARY staging and `data_blocks` set-based merge, which together cost
   about `2.30 s` in the current 128 MiB profile;
2. metadata lookup/cache behavior if it becomes dominant after allocation-query
   removal;
3. hardlink nlink batching only if a metadata-heavy workload shows the repeated
   hardlink count becoming material again.

Do not optimize hardlink counting merely because its call count is high: the
current 128 MiB measurement shows only about `17.7 ms` total, far below the
allocation COUNT, COPY, and merge costs.

## Delivery boundary

The next production-code change should use the next sequential FOD version and
stay narrowly scoped to the measured file-attribute allocation bottleneck.
Recommended delivery shape:

1. locate/instrument the allocation path and add regression coverage;
2. implement the smallest correct call-count reduction;
3. run targeted correctness tests;
4. run the required hot-path/fio profile gates;
5. update README/documentation and TODO with measured before/after results;
6. commit on `main` using the standard `FOD <version>: <English description>`
   format;
7. after committing, compare the new commit with its parent using
   `git diff HEAD~1..HEAD` or `git show` and inspect accidental changes,
   regressions, missing files, and consistency with this plan before push.

## Non-goals

- Do not restore extents or another alternate payload format.
- Do not change the 4 KiB logical block size in this phase.
- Do not add a PostgreSQL index as a substitute for eliminating duplicate
  round trips.
- Do not optimize hardlink counting before a profile makes it material.
- Do not mix primary/replica routing, fencing, or unrelated PostgreSQL tuning
  into the allocation-query optimization.
- Do not weaken sparse, shared-object, replay, quota, or remount semantics for
  throughput.
