# Build reproducibility and packaging quality gates

## Purpose

FOD treats production ELF/SO generation and native package construction as two
separate quality domains.

A changed DEB/RPM container hash is not by itself evidence of nondeterministic
compiled code. Packaging can add timestamps, distribution metadata, dependency
metadata and other host/tool inputs after compilation.

## Gate A — production ELF reproducibility

`test-release-elf-reproducibility` is a blocking release gate.

It performs two full `release-lto` builds in two independent
`CARGO_TARGET_DIR` trees and requires byte-for-byte equality for the production
ELF/SO artifacts. It intentionally enters the canonical internal `package-artifacts`
target through `make/fod-internal-entry.mk` (the public alias is
`package-build-artifacts`). Two consecutive normal runtime builds in one target
tree are not sufficient because Make build stamps and Cargo caches can turn the
second invocation into reuse rather than a new link.

When an ELF differs, investigate the binary first with `sha256sum`, `cmp`,
`readelf`, `objdump` and `diffoscope` where available. Inspect generated
assembly for critical functions when source-level review does not explain the
difference.

## Gate B — package payload integrity

`test-package-payload-integrity` is a blocking release gate.

It uses a separate clean Cargo target and package root, builds the native
DEB/RPM, extracts it, and requires every shipped ELF/SO byte sequence to match
the `release-lto` artifact that entered packaging.

## Aggregate release gate

`test-release-quality-gate` runs Gate A followed by Gate B. These expensive
gates remain outside the ordinary `test-all` developer loop.

## Gate C — complete package reproducibility

Full byte-for-byte DEB/RPM reproducibility is a separate strict/manual gate
until packaging explicitly normalizes at least `SOURCE_DATE_EPOCH`, staging
mtimes, packaging-tool versions, distribution-specific RPM macros and
dependency-generation environment.

Until then, a different whole-package hash must not be diagnosed as a Rust/ELF
reproducibility failure without first comparing the extracted ELF/SO payload.

## Permanent rule

Never use a cached or stamped second invocation as proof of reproducibility.
Use independent output trees or equivalent clean build environments.

Always separate deterministic compiler/linker output from packaging metadata,
and require package payload ELF/SO files to be exactly the binaries accepted by
the production ELF gate.
