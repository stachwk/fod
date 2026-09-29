# FOD 3.4.31 transactional rename/replace ownership

Status: completed 2026-09-29.

This record archives the C2 implementation sequence that closed the
same-destination temporary-file plus `rename`/replace ownership and atomicity
gap after FOD 3.4.30 closed direct writable `create`/copy races.

## Completed boundary

FOD 3.4.31 provides one PostgreSQL-authoritative protected namespace mutation
for file, hardlink, symlink and directory rename/replace. The completed contract
is:

- deterministic, non-blocking namespace advisory-lock ordering;
- re-resolution of source and destination under the protected transaction;
- active source-destination, target-destination and target-file ownership
  conflicts fail promptly with `EBUSY`;
- hardlink aliases cannot bypass target-file write ownership;
- target removal and source move are one PostgreSQL transaction;
- any injected failure rolls the complete replacement back;
- losing same-destination races preserve the loser source and winner-only
  destination payload;
- actively write-owned source paths are not lease-migrated; rename rejects them
  with `EBUSY` until the writer closes;
- existing POSIX namespace errors remain distinct from ownership conflicts;
- no rename-specific persistent lease table or storage-format change was added.

After all namespace kinds were transactional, the legacy FUSE fallback that
first removed a target and then called a separate direct rename helper was
removed. The four unused `DbRepo::rename_*_entry()` methods and their four FFI
wrappers were then removed as dead API.

## Baseline

Before C2, the diagnostic baseline on FOD 3.4.30 reproduced both unsafe paths:

```text
BASELINE_UNSAFE rename-active-destination-writer replace_result=SUCCESS elapsed_ms=11.133 lease_before=1|1 lease_after=1|0 final_payload=replacement-from-mount-b
BASELINE_UNSAFE rename-active-source-writer rename_result=SUCCESS elapsed_ms=8.119 old_name_lease=1|0 new_name_lease=0|1
OK rename-write-ownership-baseline known_gap_reproduced=2
```

The destination could be replaced while another mount held active write
ownership, and a live source lease could remain keyed to the old pathname after
rename. Those behaviors defined the C2 failure model.

## Implementation sequence

### Archived C2 plan and acceptance sequence

C1 closes the direct writable `create`/copy race, but the current mounted
`rename()` path still has two related correctness gaps:

- it resolves and removes an existing destination before checking PostgreSQL
  destination/file ownership, so temporary-file plus rename/replace can bypass
  the C1 first-writer-wins contract;
- target removal and source rename are separate repository calls. A failure
  after target removal can therefore leave a partially applied replacement
  instead of rolling the namespace mutation back atomically.

The current write-ownership primitive is intentionally narrower: one
destination plus an optional file identity are acquired in one transaction.
That is sufficient for writable `create`, but rename/replace needs an operation
that coordinates the old destination, the new destination and, when replacing
a regular file or hardlink, the existing target file identity.

A further source-side detail is explicit in C2: open writable handles store
their destination lease as `parent_id + name`. The current cache move updates
the handle path after rename but does not migrate that lease. C2 therefore does
not attempt lease migration for an actively write-owned source. If the source
pathname has live write ownership, rename fails promptly with `EBUSY`; the
ordinary temporary-file workflow remains supported because the temporary file
is closed before the final rename.

### C2.1 — Baseline and contract

The pre-C2 diagnostic `test_rename_write_ownership_baseline.py` captured the
known unsafe behavior before runtime changes. Its expectations were then
inverted into the blocking `make test-rename-write-ownership` gate once the
regular-file C2 slice landed.

The baseline was reproduced on 2026-09-28 against FOD 3.4.30:

```text
BASELINE_UNSAFE rename-active-destination-writer replace_result=SUCCESS elapsed_ms=11.133 lease_before=1|1 lease_after=1|0 final_payload=replacement-from-mount-b
BASELINE_UNSAFE rename-active-source-writer rename_result=SUCCESS elapsed_ms=8.119 old_name_lease=1|0 new_name_lease=0|1
OK rename-write-ownership-baseline known_gap_reproduced=2
```

This proves both bypasses. It also shows why C2 must not rename a live source
lease in place: after namespace mutation, the destination lease remains keyed
to the old pathname while the file lease follows the file identity under the
new pathname.

Capture/retain the following two-mount cases while changing runtime code:

- mount A holds an active writer on the destination while mount B closes a
  temporary source and renames it over that destination;
- two mounts concurrently rename different closed temporary files to the same
  initially absent destination;
- the same race with an already existing destination;
- rename of a source pathname while that pathname has active write ownership;
- replacement of a target file whose file identity is write-owned through a
  different hardlink pathname.

The selected contract is fail-fast first-writer-wins:

- any advisory-lock contention or live conflicting destination/file ownership
  returns `EBUSY` without waiting;
- no destination payload or namespace row is removed before all ownership and
  namespace preconditions pass;
- a losing rename leaves both its source and the destination unchanged;
- existing POSIX error behavior such as `ENOENT`, `EISDIR`, `ENOTEMPTY`,
  `EINVAL` and root `EXDEV` remains separate from ownership conflicts.

### C2.2 — Transactional repository primitive

FOD 3.4.31 starts with the regular-file slice: file sources whose destination
is absent or another primary file use one guarded PostgreSQL transaction.
Hardlink/symlink/directory rename paths remain on the legacy path until the
later C2 slices close them.

The first 3.4.31 mounted gate passed on 2026-09-28:

```text
OK rename-active-destination-writer errno=16 elapsed_ms=12.793 destination_unchanged=1 source_preserved=1
OK rename-active-source-writer errno=16 elapsed_ms=3.170 source_preserved=1 destination_absent=1
OK rename-write-ownership protected_cases=2
```

The corresponding format, workspace check and rename/root-conflict regression
were also green.

The extended C2.4 gate then passed on 2026-09-28:

```text
OK rename-hardlink-target-writer errno=16 elapsed_ms=7.066 target_unchanged=1 source_preserved=1
OK rename-same-destination-race scenario=absent winner=A loser=B loser_errno=16 winner_payload_only=1 loser_source_preserved=1 ownership_leaks=0
OK rename-same-destination-race scenario=existing winner=A loser=B loser_errno=16 winner_payload_only=1 loser_source_preserved=1 ownership_leaks=0
OK rename-write-ownership protected_cases=5
```

Production-hook isolation, integration-hook presence, create ownership and
rename/root-conflict regressions were all green.

The final regular-file atomicy gate also passed on 2026-09-28:

```text
OK rename-fault-rollback errno=5 target_restored=1 source_restored=1 ownership_leaks=0
OK rename-write-ownership protected_cases=6
```

Together with green format/workspace/diff checks, this closes the regular
`file -> absent/file` C2 slice.

The hardlink extension then passed on 2026-09-28:

```text
OK rename-hardlink-target-writer errno=16 target_unchanged=1 source_preserved=1
OK rename-hardlink-source-writer errno=16 source_preserved=1 destination_absent=1
OK rename-hardlink-file-like hardlink_source_move=1 hardlink_target_replace=1 same_file_noop=1
OK rename-write-ownership protected_cases=7
```

Format, workspace check and diff check were also green. This closes the
`file/hardlink -> absent/file/hardlink` C2 slice.

The symlink extension then passed on 2026-09-28:

```text
OK rename-symlink-target-writer errno=16 target_unchanged=1 source_preserved=1
OK rename-symlink-namespace source_move=1 file_over_symlink=1 symlink_over_file=1 symlink_over_symlink=1
OK rename-symlink-transactional protected_cases=4
```

The existing file/hardlink ownership gate, symlink/readlink smoke and
rename/root-conflict regression were also green. This closes the
`file/hardlink/symlink -> absent/file/hardlink/symlink` C2 namespace slice.
Directory rename/replace is the remaining legacy rename path.

Add one PostgreSQL-authoritative rename/replace primitive rather than chaining
the existing single-resource ownership calls.

The operation should:

1. derive resource keys for the old and new namespace destinations and use
   non-blocking PostgreSQL advisory transaction locks; known keys are acquired
   in deterministic order;
2. re-resolve source and destination after the destination locks are held;
3. when the destination resolves to a file identity, include/check its file
   ownership so a writer through another hardlink alias also fences replace;
4. prune expired ownership rows and reject every live conflicting source
   destination, target destination or target-file lease with `EBUSY`;
5. perform target removal/hardlink promotion and source rename in the same
   PostgreSQL transaction, so any error rolls the complete replacement back;
6. keep the operation replay-safe according to the existing repository
   transaction policy and map backend uncertainty to `EIO`, never to a
   partially successful rename.

No persistent rename-specific lease table is selected. Transaction-scoped
advisory locks serialize the namespace mutation, while the existing persistent
destination/file lease rows are the authority for active writers. No schema or
storage-format change is expected.

### C2.3 — FUSE integration and cache state

The FUSE callback keeps permission, sticky-bit, directory-cycle and root
validation at the mounted boundary, but delegates the final protected namespace
mutation to the transactional repository primitive.

After a successful transaction:

- invalidate old/new path and statfs metadata coherently;
- move source cached path state only after commit;
- do not drop source or target handle state before the repository transaction
  succeeds;
- do not migrate a live source write lease in C2; active source ownership is an
  `EBUSY` condition.

The existing `test_rename_root_conflict.sh` behavior remains a mandatory
regression.

### C2.4 — Deterministic acceptance

Extend the existing `integration-test-hooks` feature with a rename barrier so
the concurrency windows are deterministic without shipping test hooks in the
production binary.

Acceptance requires:

- active destination writer vs rename/replace -> prompt `EBUSY`, unchanged
  destination payload and unchanged temporary source;
- two simultaneous closed-temp renames to one absent destination -> exactly
  one winner, exactly one `EBUSY` loser and winner-only final content;
- the same race over an existing destination -> one atomic winner, no transient
  mixed/partial replacement and loser source preserved;
- target file write-owned through another hardlink pathname -> `EBUSY`;
- actively write-owned source pathname -> `EBUSY`, then successful rename
  after the writer closes;
- injected failure between logical target removal and source move proves
  PostgreSQL rollback restores the original target and source;
- cross-parent file rename and the existing file/directory/root conflict tests
  remain green;
- zero leaked destination/file ownership rows after every case;
- normal production `fod-rust-fuse` contains no rename test-hook marker.

Only after these gates pass should C2 change from selected to completed.


## Final validation

The final 2026-09-29 mounted regression sequence passed with:

```text
OK rename-active-destination-writer errno=16 destination_unchanged=1 source_preserved=1
OK rename-active-source-writer errno=16 source_preserved=1 destination_absent=1
OK rename-hardlink-target-writer errno=16 target_unchanged=1 source_preserved=1
OK rename-hardlink-source-writer errno=16 source_preserved=1 destination_absent=1
OK rename-hardlink-file-like hardlink_source_move=1 hardlink_target_replace=1 same_file_noop=1
OK rename-fault-rollback errno=5 target_restored=1 source_restored=1 ownership_leaks=0
OK rename-same-path-noop payload_preserved=1 ownership_leaks=0
OK rename-write-ownership protected_cases=8
OK rename-symlink-transactional protected_cases=4
OK rename-directory-transactional protected_cases=5
OK rename/root-conflict
```

The same-destination race was validated both for an initially absent
destination and for replacement of an existing destination: exactly one rename
won, the loser returned `EBUSY`, the loser source remained present and no
ownership rows leaked.

The directory gate covered move to an absent destination, replacement of an
empty target directory, `ENOTEMPTY`, file/directory type errors, descendant
rejection with `EINVAL`, and injected rollback. Symlink source/target
combinations were also transactional.

Final quality checks were green:

- pinned `cargo fmt --all -- --check`;
- `cargo check --workspace --locked`;
- `git diff --check`;
- `cargo test --locked -p fod-rust-hotpath --lib`: 82 passed, 0 failed.

The standalone mutating `rust_hotpath/tests/write_ownership.rs` integration
test intentionally refuses the normal `foddbname` database and requires a
dedicated database whose name contains `test`; that guard is independent of
C2 and was not bypassed for closure.

## Result

C2 is complete. FOD 3.4.31 no longer has a mounted legacy rename/replace path
for file, hardlink, symlink or directory namespace entries. Future work should
reopen this area only for a concrete correctness regression or newly measured
behavioral gap.
