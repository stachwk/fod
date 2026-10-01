# FOD agent rules

These rules apply to repository work performed by coding agents and automated
assistants.

## PostgreSQL-authoritative multi-host correctness

FOD may be mounted and used concurrently from multiple hosts. PostgreSQL is the
single logical authority for every decision that affects shared filesystem
correctness.

Namespace mutations, open/unlink lifecycle state, write ownership, leases,
fencing, inode-generation decisions, rename/create conflict resolution and
other cross-host ordering decisions must be represented and serialized in
PostgreSQL transactions, constraints, leases or advisory locks. Process-local
mutexes, maps and caches may optimize one mount, but they must never be the
source of truth for correctness visible to another mount.

HA is compatible with this rule when the PostgreSQL topology still exposes one
logical write order. A physical primary may fail over, or a multi-primary
technology may be used, only if conflicting filesystem mutations retain one
globally authoritative serial order. Eventual conflict reconciliation is not a
valid replacement for filesystem operation ordering.

See `docs/FOD_ARCHITECTURE_INVARIANTS.md`.

## Commit review

After every commit, compare the new commit with its parent using
`git diff HEAD~1..HEAD`, `git show`, or an equivalent exact commit comparison.
Inspect the complete changed-file set for accidental changes, missing files,
regressions and inconsistencies with the stated goal before treating the commit
as complete.

## Ambiguous / Heisenbug diagnosis

Do not close an unexplained defect by changing timing, adding retries, adding
logging, or otherwise making the symptom disappear without identifying a
credible mechanism.

If source-level inspection, deterministic regression attempts and normal
runtime/log instrumentation do not establish one clear cause — especially for
timing-sensitive, optimization-sensitive, intermittent or Heisenbug-like
failures — ASM-level analysis is mandatory.

Use the same build profile, compiler/toolchain, feature set and relevant flags
as the failing build. Inspect the generated executable/shared-object machine
code and ELF metadata (for example with `objdump`/`llvm-objdump`,
`readelf` and symbol/disassembly tools), then compare the relevant
instructions, inlining/optimization decisions, branches, memory accesses and
call boundaries with the source-level expectation.

Record what the ASM/ELF evidence confirms or rules out. Only then select the
fix, unless an earlier layer has already produced a deterministic and
sufficiently supported root cause.
