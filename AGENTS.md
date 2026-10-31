# FOD agent rules

These rules apply to repository work performed by coding agents and automated
assistants.

## Language

Write to the user in Polish. Write notes added to repository documentation in
Polish or English.

## Local project memory

Use the local MemPalace installation at `~/.venv/bin/mempalace` when working
in this repository. Before substantive work, read the FOD context with
`~/.venv/bin/mempalace wake-up --wing fod` and search for relevant decisions,
commands or prior findings with `~/.venv/bin/mempalace search --wing fod
--results 5 "<topic>"`. Treat memory results as supporting context and verify
current behavior against the repository's code, tests and maintained
documentation.

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

## SQL column type sizing

When creating or changing SQL schemas, choose column types and constraints to
match the field's actual domain, supported range and expected data size. Avoid
both undersized types that reject valid FOD data and oversized numeric types
when a narrower type fully represents the contract. For text, use explicit
length limits only when they come from a documented filesystem, protocol or
format constraint; distinguish character limits from byte limits and enforce
the correct one in PostgreSQL. Do not use `VARCHAR(n)` as a storage
optimization over PostgreSQL `TEXT`: both store the same values the same way.
Review indexed text fields for PostgreSQL index-entry limits as well. Keep
fresh-install schemas, upgrade migrations and runtime-created SQL structures
consistent, and never rewrite an already published migration to change its
effect; add a new migration instead. See `docs/SQL_SCHEMA_LIMIT_AUDIT.md`.

## PostgreSQL page-size policy

FOD's default and target PostgreSQL image is built with compile-time
`BLCKSZ=32KiB` using `docker/postgres-blocksize/Dockerfile` and
`scripts/publish_postgres_fod_32k.sh`. The standard `BLCKSZ=8KiB` build remains
available for compatibility and comparison; it is not the deployment default.
When reviewing page-size-dependent limits, indexes or SQL storage behavior,
verify the selected image and build argument, the compiled
`pg_config --configure` value, and runtime `SHOW block_size`. Evaluate both 32KiB and 8KiB
where the 8KiB compatibility variant is in scope; do not report an 8KiB limit
as the default FOD deployment limit.

The 32KiB build compiles PostgreSQL source in the builder stage with
`--with-blocksize=32`, then verifies the configured `pg_config` and a fresh
`initdb` cluster's `block_size` before publishing. For in-container checks, use
the custom binaries under `/opt/postgresql-custom/bin` (or a non-login shell):
a login shell may reset `PATH` to `/usr/local/bin` and inspect the upstream
8KiB binaries instead of the running FOD build. Verify the live server with
`SHOW block_size` using the configured PostgreSQL role; an image tag or label
alone is not evidence of the running server's page size.

## Supported build architectures

Build and package FOD only for 64-bit and wider userspace architectures. Do not
create, compile, package, publish or add CI targets for 32-bit FOD binaries.
Keep build instructions, release artifacts and platform support documentation
consistent with this boundary. POSIX findings that apply only to a 32-bit ABI
must be recorded as outside FOD's supported platform matrix; verify applicable
overflow behavior on supported 64-bit and wider targets instead.

## Linux errno centralization

`rust_fuse/src/errno.rs` is the single source of truth for Linux errno values
and mappings exposed by the Rust FUSE frontend. Add new errno definitions and
domain-error translations there, and have filesystem callbacks refer to those
symbols instead of embedding raw `libc::E...` values or duplicating mappings.
When modifying a callback that still uses an inline errno, move that use to
`errno.rs` as part of the same change. Keep `docs/ERRNO_REFERENCE.md` aligned
with the supported names, POSIX scope and FOD behavior.

## Problem tracking

Record every newly discovered unresolved defect, POSIX gap, failed regression
or environment limitation that affects correctness or verification in
`TODO.md` before moving on. Include the observed symptom or reproduction, the
available evidence and root-cause confidence, the impact, and the next
diagnostic or fix step; link the relevant code, tests and detailed review.
Keep the item there until evidence supports resolution. `TODO.md` preserves
findings and decisions; when work becomes active or its priority changes, also
update the ordered queue in `docs/plans/CURRENT.md` rather than treating the
archive as the active queue.

## Selective refactoring

Refactor only clear duplication, unnecessary complexity or mixed responsibilities
when the change has a concrete benefit. File size alone does not justify splitting
a module. Keep cohesive code together and avoid extra abstractions or wrappers
without a demonstrated need. Stop the refactoring pass when the obvious,
worthwhile candidates in the reviewed area have been addressed.

## Linux and libfuse runtime floor

Keep the minimum supported mount runtime explicit and fail closed before a
mount is attempted. FOD requires Linux 6.17.0 or newer and libfuse3 3.18.2 or
newer. Changes to the FUSE frontend, its dependencies or required negotiated
capabilities must re-evaluate this floor, update the startup preflight and
`docs/FUSE_REQUIREMENTS.md`, and add or update boundary tests. Do not silently
continue on an older or unparseable runtime.

## Commit review

After every completed, verified logical change, create a commit on `main`
without waiting for a separate user request. Keep coordinated edits in one
coherent commit. Apply `docs/versioning.md`, review the staged changes before
committing, and inspect the complete parent-to-commit diff afterward.

When the user explicitly requests a commit and push, perform both on `main`
without asking again. Follow `docs/versioning.md`, review the staged changes,
then after committing inspect the complete `git diff HEAD~1..HEAD` before
pushing. Verify that `main` and `origin/main` agree after the push.

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
