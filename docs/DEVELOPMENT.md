# FOD development guide

This document is the task-oriented entry point for changing FOD itself. It
links the current development contracts without duplicating the detailed test,
performance or architecture documents.

## Repository/toolchain baseline

The repository pins the development/build toolchain in `rust-toolchain.toml`:

```text
Rust 1.98.0
components: clippy, rustfmt
```

The Cargo workspace declares `rust-version = "1.85"` as the package minimum.
These are different concepts: the repository-pinned production/development
toolchain is newer than the workspace minimum version.

Production-style optimized builds use the `release-lto` profile where the
relevant packaging/deployment paths require it.

## Active runtime crates

| Crate | Responsibility |
| --- | --- |
| `rust_fuse` | FUSE frontend and filesystem callbacks |
| `rust_runtime` | PostgreSQL runtime, configuration and shared services |
| `rust_hotpath` | storage/read/write hot paths |
| `rust_mkfs` | schema/bootstrap/configuration tooling |
| `rust_monitor` | runtime/cluster observability |
| `rust_indexer` | external-source indexing/import tooling |
| `rust_libfod` | shared library/API surface |

The current architecture summary is in [`CURRENT_STATE.md`](CURRENT_STATE.md).

## Versioning and commits

`../fod_version.txt` is the authoritative FOD release version. Every commit
increments the patch component unless a deliberate major/minor decision has
been made.

A versioned commit keeps aligned:

- `fod_version.txt`,
- `[workspace.package].version` in `Cargo.toml`,
- local FOD package versions in `Cargo.lock`,
- exact release references where documentation/deployment policy requires
  them,
- commit subject: `FOD X.Y.Z: <english description>`.

The complete rule is in [`versioning.md`](versioning.md).

After committing, compare the new commit with its parent and inspect the whole
change for accidental files, missing updates, regressions and scope drift.

## Documentation maintenance

Classify documentation before editing or adding it:

- **current behavior** -> update `CURRENT_STATE.md` or the relevant task guide
  linked from [`README.md`](README.md),
- **durable contract/decision** -> update the relevant requirements, contract or
  ADR document,
- **release-specific measurement or implementation evidence** -> store it under
  [`history/`](history/) and add it to [`HISTORY.md`](HISTORY.md),
- **active/future work** -> keep the maintained next-step plan in
  [`plans/CURRENT.md`](plans/CURRENT.md), longer-term ideas in
  [`plans/`](plans/) / `../ROADMAP.md`, and supporting follow-ups in
  `../TODO.md`,
- **completed plan/project** -> move it to [`history/`](history/) and index it in
  [`HISTORY.md`](HISTORY.md),
- **execution journal** -> keep it out of current task guides; `../commands.md`
  and `../conclusions.md` are retained only as chronological work evidence.

Do not edit historical evidence merely to make an old result look current. If a
new default or lifecycle rule replaces it, update the canonical task document
and preserve the old evidence as recorded. New canonical guides should be
linked from `docs/README.md`; new historical evidence should be linked from
`HISTORY.md`; maintained plans should be linked from `plans/README.md`.

The source-of-truth order and documentation classes are defined at the top of
[`README.md`](README.md).

## Local validation

The repository intentionally has no active GitHub Actions workflow. Local
Make/Cargo gates are the validation path.

Main regression gates:

```bash
make test-all
make test-all-full
```

Release/policy checks commonly used for versioned deployment changes:

```bash
make test-cargo-lock-integrity
make test-version
make test-docker-deploy-policy
make test-docker-fod-install-policy
make test-docker-deploy-systemd-policy
make test-docker-fod-client-policy
```

Detailed test profiles and ordering rules are documented in
[`../zasady_sprawdzen.md`](../zasady_sprawdzen.md).

## Runtime configuration changes

Before adding a new `FOD_*` variable or INI key, read
[`runtime-configuration.md`](runtime-configuration.md) and the generated
[`runtime-env-ini-audit.md`](runtime-env-ini-audit.md).

Keep the precedence contract explicit:

```text
[fod] base -> selected profile -> FOD_* environment -> owned bootstrap CLI options
```

Do not silently turn diagnostic/test environment variables into persistent
configuration.

## Schema and storage-format changes

Fresh schema state and numbered migrations live under `../migrations/`.
Storage-format changes require an explicit compatibility/versioning decision;
do not infer format compatibility from the FOD application version alone.

Relevant design contracts:

- [`adr/storage-format-versioning.md`](adr/storage-format-versioning.md),
- [`adr/storage-object-segment-manifest.md`](adr/storage-object-segment-manifest.md),
- [`compatibility-contracts.md`](compatibility-contracts.md),
- [`space-accounting.md`](space-accounting.md).

## Performance changes

Do not merge a performance optimization based only on intuition or a single
throughput number. Capture before/after evidence from the workload that owns the
cost.

The profiling workflow is in [`performance.md`](performance.md), while accepted
benchmark baselines are summarized in [`../BENCHMARKS.md`](../BENCHMARKS.md)
and [`performance-baselines.md`](performance-baselines.md).

The maintained next optimization target belongs in
[`plans/CURRENT.md`](plans/CURRENT.md). Completed performance plans and dated
experiments belong under [`history/`](history/) as evidence; they are not a
source of current defaults. See [`HISTORY.md`](HISTORY.md).

## Indexer changes

`fod-indexer` is the shared indexing core. Source-specific behavior belongs at
the adapter boundary; the common scan/hash/duplicate/plan/materialize/cleanup
flow remains shared.

Read:

- [`fod-indexer.md`](fod-indexer.md),
- [`fod-indexer-read-api.md`](fod-indexer-read-api.md).

## Make targets and repository structure

Follow [`MAKE_TARGET_NAMING.md`](MAKE_TARGET_NAMING.md) when adding Make
interfaces. Prefer existing task groups and explicit policy targets over
parallel ad-hoc command names.

Keep code changes within the active Rust/runtime surface. Historical documents
and benchmark artifacts should not be edited merely to make old text look
current; add or update canonical task documentation instead.

## Planning and future work

Current direction and implementation sequence are tracked separately from
current behavior:

- [`plans/CURRENT.md`](plans/CURRENT.md),
- [`../ROADMAP.md`](../ROADMAP.md),
- [`plans/FOD_FUTURE_IDEAS.md`](plans/FOD_FUTURE_IDEAS.md),
- [`../TODO.md`](../TODO.md).

Plans describe intended work. `CURRENT_STATE.md`, current configuration and the
runtime/tests describe what exists now.
