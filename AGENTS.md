# FOD agent rules

These rules apply to repository work performed by coding agents and automated
assistants.

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
