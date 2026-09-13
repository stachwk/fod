# FOD P4 compatibility diagnostics aggregation

Status: completed 2026-09-13.

This document archives the P4 execution sequence that followed P3 external
unmount validation.

## Goal

Provide one trustworthy machine-readable diagnostic path across PostgreSQL,
libpq, FOD schema/storage format and negotiated FUSE runtime state without
parsing human log text and without guessing missing compatibility facts.

## P4.1 — FOD 3.4.24

`fod-rust-mkfs status --json` became the versioned source for:

- libpq runtime version,
- PostgreSQL server version and client/server major relation,
- PostgreSQL runtime requirements and observed settings,
- FOD schema readiness/migration state,
- persisted storage-format settings such as `block_size`.

Its JSON schema version is independent from the database schema version.

## P4.2 — FOD 3.4.25

Negotiated FUSE compatibility became part of shared monitor telemetry.

`SharedMonitorSessionStats` moved to schema version 2 and added optional
`fuse_compatibility`. The publisher may emit `null` before `Filesystem::init`
finishes. Later samples contain the observed fuser version, kernel/userspace
protocols, requested/enabled/unsupported capabilities, effective write and
readahead limits and kernel-derived request ceiling.

Older shared-monitor schema-version-1 payloads remain readable through serde
defaults.

## P4.3 — FOD 3.4.26

`fod-monitor report --json` moved to schema version 2 and aggregated:

- raw `mkfs_status`,
- raw cluster/session telemetry,
- independent `mkfs_status_error`,
- independent `cluster_error`,
- local process/system diagnostics.

A failure of one compatibility source does not destroy the whole report.

## P4.4 — FOD 3.4.27

`fod-monitor report --json` moved to schema version 3 and added
`compatibility_summary`.

The summary is deliberately coverage-oriented rather than verdict-oriented:

- `coverage=complete|partial|unavailable` describes whether source data is
  present, not whether the environment passes compatibility;
- PostgreSQL summary fields remain optional when their source fields are
  absent;
- FUSE summary reports session coverage and observed versions/protocols rather
  than inferring unsupported values;
- missing mkfs, missing cluster, and both-sources-missing paths remain explicit.

Raw versioned source payloads stay in the report and remain the authoritative
detail.

## Stable schema versions after P4

```text
fod-rust-mkfs status --json     schema 1
fod-monitor cluster --json      schema 1
SharedMonitorSessionStats       schema 2
fod-monitor report --json       schema 3
FOD database schema             24
```

## Guardrails retained

- Do not parse FUSE human log lines to derive compatibility state.
- Do not turn unavailable diagnostics into guessed PASS/FAIL.
- Do not hide source failures behind an overall compatibility label.
- Do not introduce a separate compatibility subsystem while the existing
  versioned sources remain sufficient.
